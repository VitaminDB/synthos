//! Фоновый семплер занятости терминалов для бейджей нав-рейла.
//!
//! Раз в секунду фоновый поток постит main-thread callback, который проходит
//! по всем code-сессиям и пишет в `TerminalsState.busy_count` число «занятых»
//! терминалов. Терминал занят, если выполняется ЛЮБОЕ из двух условий:
//!
//! 1. **foreground-процесс** — у tty есть foreground process group, отличная
//!    от shell'а, т.е. прямо сейчас идёт команда
//!    (`TerminalSession::is_busy`, `tcgetpgrp` под капотом);
//! 2. **вывод обновляется** — `TerminalSession::revision()` (bump на каждый
//!    обработанный чанк PTY-вывода) изменился с прошлого тика. Ловит
//!    активность, которую первое условие не видит: фоновые job'ы, пишущие в
//!    терминал, `tail -f` и т.п. Если текст не обновляется и foreground'а
//!    нет — терминал простаивает.
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
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;

use super::state::{CodeEditorCtx, SessionId};

/// Интервал опроса. 1 Гц достаточно: бейдж — индикатор «идёт ли работа»,
/// а не осциллограф; сам опрос — один `tcgetpgrp` + чтение атомика на таб.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Последняя виденная `revision` каждого таба; ключ — (session.id, tab.id).
/// static, потому что тик — короткий main-thread callback и состояние между
/// тиками держать больше негде (closure пересоздаётся). Карта пересобирается
/// целиком на каждом тике — записи закрытых табов не накапливаются.
fn revision_registry() -> &'static Mutex<HashMap<(SessionId, u32), u64>> {
    static R: OnceLock<Mutex<HashMap<(SessionId, u32), u64>>> = OnceLock::new();
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
    let mut seen: HashMap<(SessionId, u32), u64> = HashMap::new();

    for session in ctx.sessions.get_untracked() {
        let tabs = session.terminals.tabs.get_untracked();
        let busy = tabs
            .iter()
            .filter(|t| {
                let key = (session.id, t.id);
                let rev = t.session.revision();
                // Первый тик после открытия таба: prev нет → «не менялось»,
                // судим только по foreground-процессу.
                let output_changed = registry.get(&key).is_some_and(|&prev| prev != rev);
                seen.insert(key, rev);
                t.session.is_busy() || output_changed
            })
            .count();
        session.terminals.busy_count.set(busy);
    }

    *registry = seen;
}
