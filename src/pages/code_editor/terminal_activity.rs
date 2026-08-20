//! Фоновый семплер занятости терминалов для бейджей нав-рейла.
//!
//! Раз в секунду фоновый поток постит main-thread callback, который проходит
//! по всем code-сессиям и пишет в `TerminalsState.busy_count` число «занятых»
//! терминалов. Терминал занят, если **вывод обновлялся** в последние
//! [`ACTIVITY_WINDOW`] секунд: `TerminalSession::revision()` (bump на каждый
//! обработанный чанк PTY-вывода) изменился недавно. Окно, а не «с прошлого
//! тика», — чтобы команды с редким выводом (компиляция) не мигали
//! зелёный↔красный между строками.
//!
//! Наличие foreground-процесса (tcgetpgrp ≠ shell) сознательно НЕ считается
//! занятостью: интерактивные TUI (Claude Code, vim, htop в паузе) висят
//! foreground'ом всё время жизни, даже когда просто ждут ввода — бейдж
//! горел бы зелёным на простаивающем терминале. «Что-то происходит» ⇔
//! «текст меняется».
//!
//! Сигналы thread-local, поэтому вся работа с ними — строго внутри
//! `run_on_main_thread`; `RwSignal::set` с равным значением не будит
//! подписчиков, так что тик без изменения состояния не провоцирует rebuild
//! нав-рейла.
//!
//! Поток живёт до конца процесса (1 wakeup/с — дешевле, чем городить
//! stop-флаг: сессии редактора существуют всё время жизни приложения).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;

use super::state::{CodeEditorCtx, SessionId};

/// Интервал опроса. 1 Гц достаточно: бейдж — индикатор «идёт ли работа»,
/// а не осциллограф; сам опрос — чтение одного атомика на таб.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Сколько держать статус «занят» после последнего изменения вывода.
/// 3 с сглаживают паузы между строками у «медленных» команд, но красный
/// загорается достаточно быстро после того, как терминал затих.
const ACTIVITY_WINDOW: Duration = Duration::from_secs(3);

/// Последнее наблюдение по каждому табу: (revision, момент её изменения).
/// Ключ — (session.id, tab.id). static, потому что тик — короткий
/// main-thread callback и состояние между тиками держать больше негде
/// (closure пересоздаётся). Карта пересобирается целиком на каждом тике —
/// записи закрытых табов не накапливаются.
type TabKey = (SessionId, u32);
fn revision_registry() -> &'static Mutex<HashMap<TabKey, (u64, Instant)>> {
    static R: OnceLock<Mutex<HashMap<TabKey, (u64, Instant)>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Стартовать семплер. Вызывается один раз из `run_desktop` /
/// `android_main` сразу после `provide_context(CodeEditorCtx)`.
pub fn start_sampler(ctx: CodeEditorCtx) {
    thread::Builder::new()
        .name("synthos-term-activity".into())
        .spawn(move || loop {
            run_on_main_thread(move || sample_tick(ctx));
            thread::sleep(SAMPLE_INTERVAL);
        })
        .expect("не удалось создать поток synthos-term-activity");
}

/// Один тик на main-thread: пересчитать busy_count каждой сессии.
fn sample_tick(ctx: CodeEditorCtx) {
    let Ok(mut registry) = revision_registry().lock() else {
        return;
    };
    let now = Instant::now();
    let mut seen: HashMap<TabKey, (u64, Instant)> = HashMap::new();

    for session in ctx.sessions.get_untracked() {
        let tabs = session.terminals.tabs.get_untracked();
        let busy = tabs
            .iter()
            .filter(|t| {
                let key = (session.id, t.id);
                let rev = t.session.revision();
                // Первое наблюдение таба: считаем простаивающим (last_change
                // отодвинут на окно назад), пока вывод реально не изменится.
                let (prev_rev, mut last_change) = registry
                    .get(&key)
                    .copied()
                    .unwrap_or((rev, now - ACTIVITY_WINDOW));
                if rev != prev_rev {
                    last_change = now;
                }
                seen.insert(key, (rev, last_change));
                now.duration_since(last_change) < ACTIVITY_WINDOW
            })
            .count();
        session.terminals.busy_count.set(busy);
    }

    *registry = seen;
}
