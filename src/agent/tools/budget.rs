//! Сколько контекста осталось под результат инструмента.
//!
//! Клип истории (`syn_chat::session::clip_for_history`) — статическая
//! страховка: он режет копию для промпта по числу символов и обязан быть
//! воспроизводимым, иначе история, пересобранная из ленты на следующем
//! сообщении, разойдётся с промптом хода и префикс-KV обнулится. Поэтому
//! он и не может знать, сколько места в окне осталось на самом деле.
//!
//! Знает это agent-loop: у него на руках длина промпта в токенах и ёмкость
//! хода ([`RingPlan`](crate::syn_chat::session) — окно модели, ограниченное
//! свободной VRAM). Перед исполнением вызовов он кладёт сюда остаток и
//! токенизатор загруженной модели; инструмент, который сам решает, сколько
//! отдать (`notes read` пачкой), спрашивает [`grant`] и режет ответ по
//! живому бюджету, а не по константе — и пишет модели, сколько осталось.
//!
//! Динамика идёт только вниз, от потолка инструмента: что бы ни показал
//! бюджет, ответ не станет больше статической страховки, и клип истории
//! по-прежнему не срабатывает на честной пачке.

use std::sync::{Arc, Mutex};

use crate::syn_chat::model_registry::LoadedSynModel;

/// Счётчик токенов — обычно токенизатор загруженной модели.
pub type Counter = Arc<dyn Fn(&str) -> usize + Send + Sync>;

/// Счётчик по загруженной модели. Ошибка кодирования не должна ронять ход:
/// где токенизатор не смог, сойдёт оценка по символам.
pub fn model_counter(model: &Arc<LoadedSynModel>) -> Counter {
    let model = model.clone();
    Arc::new(move |s: &str| model.tokenizer.encode(s).map(|ids| ids.len()).unwrap_or_else(|_| estimate(s)))
}

/// Оценка, когда счётчика нет (юнит-тесты, вызов вне agent-loop). Русский
/// текст у BPE-словарей Qwen/Gemma идёт по ~2,5–3 символа на токен,
/// латиница и разметка — по 3,5–4; берём нижнюю границу, чтобы оценка
/// ошибалась в сторону «меньше отдать».
const CHARS_PER_TOKEN: usize = 3;

/// Сколько инструмент получает, когда в окне почти ничего не осталось.
/// Отдать ноль нельзя: ответ «ничего не влезло» бесполезен модели — пусть
/// вернёт хотя бы первую страницу и скажет, что контекст кончился.
const MIN_GRANT_TOKENS: usize = 512;

#[derive(Clone)]
struct Slot {
    /// Остаток окна в токенах на момент вызова.
    left: usize,
    counter: Option<Counter>,
}

static SLOT: Mutex<Option<Slot>> = Mutex::new(None);

fn slot() -> Option<Slot> {
    SLOT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Бюджет одного ответа, выданный инструменту.
#[derive(Debug, Clone, Copy)]
pub struct Grant {
    /// Верхняя граница ответа в токенах.
    pub tokens: usize,
    /// Сколько всего осталось в окне до конца контекста.
    pub left: usize,
    /// Бюджет посчитан по живому окну; `false` — счётчика нет, инструмент
    /// работает по своему потолку и оценке «символы / 3».
    pub measured: bool,
}

impl Grant {
    /// Ответ пришлось урезать не своим потолком, а теснотой в окне.
    pub fn is_tight(&self, cap: usize) -> bool {
        self.measured && self.tokens < cap
    }
}

/// Активный бюджет живёт, пока жив guard. Восстановление прежнего слота на
/// Drop нужно для вложенных циклов: субагент крутит свой agent-loop внутри
/// вызова родителя и по выходе обязан вернуть родительский бюджет.
pub struct Guard(Option<Slot>);

impl Drop for Guard {
    fn drop(&mut self) {
        *SLOT.lock().unwrap_or_else(|e| e.into_inner()) = self.0.take();
    }
}

/// Что остаётся ходу поверх промпта и уже сгенерированного: место на
/// следующий вызов и на сам ответ пользователю. Инструмент и так берёт лишь
/// половину остатка, так что резерв страхует хвост серии вызовов.
const TURN_RESERVE_TOKENS: usize = 2048;

/// Бюджет хода: `capacity` — честный потолок контекста (окно модели,
/// ограниченное свободной VRAM), `used` — промпт плюс то, что ход уже
/// сгенерировал. Считают одинаково основной цикл и субагент.
pub fn arm_turn(capacity: usize, used: usize, counter: Counter) -> Guard {
    arm(capacity.saturating_sub(used + TURN_RESERVE_TOKENS), counter)
}

/// Объявить остаток окна под результаты инструментов текущего хода.
pub fn arm(left: usize, counter: Counter) -> Guard {
    let mut g = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let prev = g.replace(Slot { left, counter: Some(counter) });
    Guard(prev)
}

/// Учесть результат, который уже уехал в историю: следующий вызов того же
/// хода получит окно на его размер меньше.
pub fn spend(text: &str) {
    let n = count(text);
    let mut g = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = g.as_mut() {
        s.left = s.left.saturating_sub(n);
    }
}

/// Токены строки: точно — токенизатором модели, иначе оценкой по символам.
pub fn count(s: &str) -> usize {
    match slot().and_then(|s| s.counter) {
        Some(f) => f(s),
        None => estimate(s),
    }
}

/// Оценка «символы / 3» — запасной счётчик.
pub fn estimate(s: &str) -> usize {
    s.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Сколько инструмент может отдать этим ответом. `cap` — его собственный
/// потолок; бюджет только опускает эту границу, но не поднимает.
///
/// Половина остатка, а не весь: ход почти никогда не состоит из одного
/// вызова, и результат, занявший всё окно, оставил бы агента без места на
/// следующий шаг и на сам ответ.
pub fn grant(cap: usize) -> Grant {
    match slot() {
        Some(s) => Grant {
            tokens: (s.left / 2).min(cap).max(MIN_GRANT_TOKENS),
            left: s.left,
            measured: true,
        },
        None => Grant { tokens: cap, left: 0, measured: false },
    }
}

/// Слот один на процесс: тесты, которые его трогают (здесь и у `notes`),
/// обязаны идти по очереди — иначе чужой `arm` меняет бюджет посреди
/// чужого же чтения.
#[cfg(test)]
pub fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        test_serial()
    }

    /// Без agent-loop'а инструмент работает по своему потолку и по оценке.
    #[test]
    fn without_a_loop_the_cap_stands() {
        let _serial = serial();
        let g = grant(8_000);
        assert_eq!(g.tokens, 8_000);
        assert!(!g.measured);
        assert!(!g.is_tight(8_000));
        assert_eq!(estimate("абвабвабв"), 3);
    }

    /// Тесный контекст опускает потолок; счётчик берётся из слота.
    #[test]
    fn a_tight_window_lowers_the_cap() {
        let _serial = serial();
        let _guard = arm(3_000, Arc::new(|s: &str| s.chars().count()));
        let g = grant(8_000);
        assert_eq!(g.tokens, 1_500, "половина остатка");
        assert!(g.is_tight(8_000));
        assert_eq!(count("абвг"), 4, "счётчик слота, а не оценка");

        spend("абвг");
        assert_eq!(grant(8_000).left, 2_996);
    }

    /// Совсем тесное окно всё равно даёт минимум — иначе ответ пуст.
    #[test]
    fn an_exhausted_window_still_grants_the_minimum() {
        let _serial = serial();
        let _guard = arm(10, Arc::new(|s: &str| s.chars().count()));
        assert_eq!(grant(8_000).tokens, MIN_GRANT_TOKENS);
    }

    /// Вложенный цикл (субагент) возвращает родительский бюджет.
    #[test]
    fn a_nested_loop_restores_the_outer_budget() {
        let _serial = serial();
        let outer = arm(4_000, Arc::new(|_: &str| 1));
        {
            let _inner = arm(1_000, Arc::new(|_: &str| 1));
            assert_eq!(grant(8_000).left, 1_000);
        }
        assert_eq!(grant(8_000).left, 4_000);
        drop(outer);
        assert!(!grant(8_000).measured);
    }
}
