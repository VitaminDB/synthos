//! Сколько контекста осталось под результат инструмента.
//!
//! Знает это agent-loop: у него на руках длина промпта в токенах и ёмкость
//! хода ([`RingPlan`](crate::syn_chat::session) — окно модели, ограниченное
//! свободной VRAM). Перед исполнением вызовов он кладёт сюда остаток и
//! токенизатор загруженной модели. Дальше два пути:
//!
//! - инструмент, который сам решает, сколько отдать (`notes read` пачкой),
//!   спрашивает [`grant`] и режет ответ по живому бюджету на границе
//!   страницы — и пишет модели, сколько осталось;
//! - выхлоп остальных (`bash`, `web`, …) укладывает в окно цикл —
//!   [`fit_for_prompt`]: голова и хвост целиком, середина заменяется
//!   пометкой с номерами пропущенных строк. Мера — токены по токенизатору
//!   модели, а не символы: 16 КБ русской таблицы — это ~5,5k токенов, и при
//!   свободном окне в 100k резать их незачем.
//!
//! Статического клипа истории (было 8000 символов с одного вызова) больше
//! нет: он не знал, сколько места в окне на самом деле, резал середину у
//! каждого куска файла и подписью «уточните команду» отправлял модель
//! перечитывать вырезанное — по кругу, пока пользователь не нажмёт «Стоп».
//! Ленте это ничего не стоит: в неё уходит уже уложенная копия, так что
//! история, пересобранная из ленты на следующем сообщении, совпадает с
//! промптом хода байт в байт, и префикс-KV цел.

use std::sync::{Arc, Mutex};

use crate::syn_chat::model_registry::LoadedSynModel;

/// Счётчик токенов — обычно токенизатор загруженной модели.
pub type Counter = Arc<dyn Fn(&str) -> usize + Send + Sync>;

/// Счётчик по загруженной модели. Ошибка кодирования не должна ронять ход:
/// где токенизатор не смог, сойдёт оценка по символам.
///
/// Держит только токенизатор, не модель: счётчик живёт в бюджете хода и в
/// `MediaCaps` весь agent-loop, и клон `Arc<LoadedSynModel>` не давал
/// `pipelines run` с `free_vram` освободить веса — ACE-Step и LTX грузились
/// рядом с чат-LLM и падали в OOM.
pub fn model_counter(model: &Arc<LoadedSynModel>) -> Counter {
    let tokenizer = Arc::clone(&model.tokenizer);
    Arc::new(move |s: &str| tokenizer.encode(s).map(|ids| ids.len()).unwrap_or_else(|_| estimate(s)))
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

/// Сколько получает выхлоп инструмента, когда цикла нет и остатка окна
/// взять неоткуда (вызов вне agent-loop, юнит-тесты): столько же, сколько
/// `notes` вне цикла.
pub const FALLBACK_RESULT_TOKENS: usize = 8_000;

/// Запас под пометку о вырезанной середине — она тоже уходит в промпт.
const CUT_NOTE_TOKENS: usize = 96;

/// Что вырезано из выхлопа: для пометки модели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// Пропущенные строки вывода, `первая..=последняя`, нумерация с 1.
    pub omitted: (usize, usize),
    /// Всего строк в выводе.
    pub lines: usize,
    /// Цена целого вывода в токенах.
    pub tokens: usize,
    /// Сколько токенов на вывод выдано.
    pub allowed: usize,
    /// Сколько токенов окна осталось (0 — не измерено).
    pub left: usize,
    /// Бюджет посчитан по живому окну, а не по [`FALLBACK_RESULT_TOKENS`].
    pub measured: bool,
}

/// Уложить выхлоп инструмента в окно: если он дороже половины остатка —
/// оставить голову (⅔) и хвост (⅓) по границам строк, середину заменить
/// пометкой `note`. `cap` — потолок без живого окна.
///
/// Хвост важен не меньше головы: у команд там `--- stderr ---`, у длинных
/// таблиц — итоги. Доля берётся от цены целого и ужимается, пока результат
/// вместе с пометкой не влезет: считать токены по каждой строке дорого,
/// а весь текст — несколько миллисекунд.
pub fn fit(text: &str, cap: usize, note: impl FnMut(&Cut) -> String) -> String {
    let g = grant(usize::MAX);
    let allowed = if g.measured { g.tokens } else { cap };
    fit_with(text, allowed, g.left, g.measured, &count, note)
}

/// То же, что [`fit`], но с явным потолком `allowed` и счётчиком токенов —
/// для укладки вне слота бюджета: документ-вложение меряется долей окна
/// модели ещё при сборке промпта (`attach::prompt`), когда остаток хода
/// не известен. `left`/`measured` уходят в [`Cut`] как есть.
pub fn fit_with(
    text: &str,
    allowed: usize,
    left: usize,
    measured: bool,
    count: &dyn Fn(&str) -> usize,
    mut note: impl FnMut(&Cut) -> String,
) -> String {
    let tokens = count(text);
    if tokens <= allowed {
        return text.to_string();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let total_chars = chars.len();
    let lines = text.split_inclusive('\n').count();
    let per_token = total_chars as f64 / tokens.max(1) as f64;
    // Первый заход — по цене целого; дальше ужимаем по факту.
    let mut budget = allowed.saturating_sub(CUT_NOTE_TOKENS).max(64);
    let mut best: Option<String> = None;
    for _ in 0..6 {
        let head_chars = ((budget * 2 / 3) as f64 * per_token) as usize;
        let tail_chars = ((budget / 3) as f64 * per_token) as usize;
        if head_chars + tail_chars >= total_chars {
            // Бюджет почти равен целому — резать нечего, но и целиком не
            // влезает: ужимаем и пробуем снова.
            budget = budget * 4 / 5;
            continue;
        }
        // Голова кончается на последнем переводе строки до границы, хвост
        // начинается с первого после — так строки не рвутся посередине.
        let head_end = {
            let byte = chars[head_chars.min(total_chars - 1)].0;
            text[..byte].rfind('\n').map(|p| p + 1).unwrap_or(byte)
        };
        let tail_start = {
            let byte = chars[total_chars - tail_chars.max(1)].0;
            text[byte..].find('\n').map(|p| byte + p + 1).unwrap_or(byte)
        };
        if tail_start <= head_end {
            budget = budget * 4 / 5;
            continue;
        }
        let head = &text[..head_end];
        let tail = &text[tail_start..];
        let head_lines = head.matches('\n').count();
        let tail_lines = tail.split_inclusive('\n').count();
        let cut = Cut {
            omitted: (head_lines + 1, lines.saturating_sub(tail_lines).max(head_lines + 1)),
            lines,
            tokens,
            allowed,
            left,
            measured,
        };
        let out = format!("{head}{}{tail}", note(&cut));
        let cost = count(&out);
        if cost <= allowed {
            return out;
        }
        best = Some(out);
        // Ужимаем пропорционально перебору, с запасом.
        budget = (budget * allowed / cost.max(1)) * 9 / 10;
    }
    // Шесть заходов не уложились (неровная токенизация): отдаём последнее
    // приближение — оно всё равно кратно меньше целого.
    best.unwrap_or_else(|| text.to_string())
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

    fn chars3(s: &str) -> usize {
        s.chars().count() / 3
    }

    fn table(rows: usize) -> String {
        (1..=rows).map(|i| format!("| {i:04} | строка таблицы {i} |\n")).collect()
    }

    /// Что влезает в окно — уходит как есть, без пометок.
    #[test]
    fn fit_keeps_output_that_fits() {
        let _serial = serial();
        let _guard = arm(200_000, Arc::new(chars3));
        let text = table(300);
        assert_eq!(fit(&text, 100, |_| unreachable!()), text, "живое окно, а не потолок");
        drop(_guard);
        assert_eq!(fit(&text, FALLBACK_RESULT_TOKENS, |_| unreachable!()), text);
    }

    /// Не влезает — голова и хвост по строкам, пометка знает номера
    /// вырезанных строк, а результат укладывается в грант.
    #[test]
    fn fit_cuts_the_middle_by_lines_within_the_grant() {
        let _serial = serial();
        let _guard = arm(4_000, Arc::new(chars3));
        let text = table(1000);
        let mut seen: Option<Cut> = None;
        let out = fit(&text, FALLBACK_RESULT_TOKENS, |c| {
            seen = Some(*c);
            format!("…[пропущены строки {}–{}]…\n", c.omitted.0, c.omitted.1)
        });
        let cut = seen.expect("середина вырезана");
        assert!(cut.measured);
        assert_eq!(cut.allowed, 2_000, "половина остатка");
        assert_eq!(cut.lines, 1000);
        assert!(out.starts_with("| 0001 |"), "голова цела");
        assert!(out.ends_with("| 1000 | строка таблицы 1000 |\n"), "хвост цел");
        assert!(count(&out) <= 2_000, "{} токенов при гранте 2000", count(&out));
        // Пометка стоит ровно на границе строк, и номера сходятся с тем,
        // что осталось по обе стороны.
        let head_lines = out.split("…[").next().unwrap().matches('\n').count();
        assert_eq!(cut.omitted.0, head_lines + 1);
        let tail_lines = out.split("]…\n").nth(1).unwrap().matches('\n').count();
        assert_eq!(cut.omitted.1, 1000 - tail_lines);
        assert!(cut.omitted.0 < cut.omitted.1);
        // Голова примерно вдвое длиннее хвоста.
        assert!(head_lines > tail_lines && head_lines < tail_lines * 3, "{head_lines} vs {tail_lines}");
    }

    /// Вне цикла работает потолок и оценка по символам.
    #[test]
    fn fit_falls_back_to_the_cap_without_a_loop() {
        let _serial = serial();
        let text = table(1000);
        let out = fit(&text, 1_000, |c| {
            assert!(!c.measured);
            assert_eq!(c.allowed, 1_000);
            "…\n".to_string()
        });
        assert!(estimate(&out) <= 1_000);
        assert!(out.contains("…\n"));
    }

    /// Одна гигантская строка без переводов тоже режется — по символам.
    #[test]
    fn fit_handles_a_single_huge_line() {
        let _serial = serial();
        let _guard = arm(2_000, Arc::new(chars3));
        let text = "я".repeat(30_000);
        let out = fit(&text, FALLBACK_RESULT_TOKENS, |_| "|…|".to_string());
        assert!(out.contains("|…|"));
        assert!(count(&out) <= 1_000, "{}", count(&out));
        assert!(out.starts_with('я') && out.ends_with('я'));
        // Совсем тесное окно всё равно отдаёт минимум ([`MIN_GRANT_TOKENS`]).
        drop(_guard);
        let _guard = arm(100, Arc::new(chars3));
        let out = fit(&text, FALLBACK_RESULT_TOKENS, |_| "|…|".to_string());
        assert!(count(&out) <= MIN_GRANT_TOKENS && count(&out) > 300, "{}", count(&out));
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

