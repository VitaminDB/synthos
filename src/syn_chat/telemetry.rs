//! Живая телеметрия вложенных агент-циклов (субагентов) для таба «Детали».
//!
//! Зачем: пока субагент крутит свой цикл, основная лента молчит — ни
//! стрима, ни tool-карточек (промежуточные вызовы субагента наружу не
//! отдаются, см. `agent::tools::subagent`). В UI это выглядит как зависший
//! чат, хотя модель работает. Панель показывает по карточке на каждый
//! запущенный цикл с тем же набором метрик, что и у основного: tps,
//! префилл, KV-ринг, VRAM.
//!
//! Поток данных однонаправленный: циклы живут в worker-потоках и
//! tokio-задачах, где signal-runtime syngui недоступен, поэтому любая
//! правка идёт через `run_on_main_thread` в `SynChatCtx.agent_runs`.
//! Обновления из стрима throttled'ятся вызывающей стороной (каждый токен
//! дергал бы redraw).

use std::sync::atomic::{AtomicU64, Ordering};

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::try_use_context;

use crate::syn_chat::state::SynChatCtx;

/// Идентификатор корневого цикла — карточка «Основной чат». Сам корень в
/// `agent_runs` не лежит (его метрики — в отдельных `SynChatCtx.last_*`),
/// но субагентам он нужен как `parent`.
pub const ROOT_RUN: u64 = 0;

/// Сколько завершённых карточек держим в панели. Дальше вытесняем самые
/// старые: цепочка из двух десятков субагентов за один ход — норма, а
/// панель должна оставаться читаемой.
const MAX_FINISHED: usize = 8;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Что за цикл. Пока субагенты — единственный вложенный вид, но поле
/// нужно уже сейчас: карточка выбирает по нему иконку и заголовок.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunKind {
    Subagent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunState {
    Running,
    Done,
    Failed,
    Aborted,
}

impl RunState {
    pub fn is_running(self) -> bool {
        matches!(self, RunState::Running)
    }
}

/// Метрики хода — тот же набор, что показывает карточка основного чата.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct RunStats {
    pub prompt_tokens: u32,
    /// Сколько токенов промпта взято из префикс-KV.
    pub reused_tokens: u32,
    pub gen_tokens: u32,
    pub prefill_ms: u32,
    pub decode_tps: f32,
    pub ring_tokens: u32,
    pub ring_bytes: u64,
    /// Потолок контекста по свободной VRAM (`RingPlan.by_mem`).
    pub ctx_budget: u32,
    pub vram_free_mb: u32,
    /// `(на карте, всего)` блоков модели — только при частичном оффлоаде:
    /// именно он превращает 32 ток/с в 20, и без этой цифры просадка в
    /// панели выглядит загадкой.
    pub blocks_resident: Option<(u32, u32)>,
}

/// Один запущенный цикл в панели.
#[derive(Clone, PartialEq, Debug)]
pub struct AgentRun {
    pub id: u64,
    /// Кто запустил: [`ROOT_RUN`] — основной чат, иначе id другого цикла.
    /// Нужен для рекурсии: карточки вложенных циклов рисуются с отступом.
    pub parent: u64,
    /// Глубина вложенности (1 — субагент основного чата).
    pub depth: u32,
    pub kind: RunKind,
    /// Задача субагента — подпись под заголовком карточки.
    pub label: String,
    pub state: RunState,
    /// Текущий ход цикла (1-based) и его лимит.
    pub turn: u32,
    pub max_turns: u32,
    /// Инструмент, который цикл исполняет прямо сейчас.
    pub tool: Option<String>,
    /// Сколько tool-вызовов цикл уже сделал.
    pub tool_calls: u32,
    pub stats: RunStats,
}

impl AgentRun {
    fn new(id: u64, parent: u64, depth: u32, kind: RunKind, label: String, max_turns: u32) -> Self {
        Self {
            id,
            parent,
            depth,
            kind,
            label,
            state: RunState::Running,
            turn: 0,
            max_turns,
            tool: None,
            tool_calls: 0,
            stats: RunStats::default(),
        }
    }
}

/// Регистрирует новый цикл и возвращает его id. Сам id выдаётся здесь же
/// (атомарный счётчик), поэтому вызывающему не нужно ждать main thread —
/// правки можно слать сразу после.
pub fn begin(parent: u64, depth: u32, kind: RunKind, label: String, max_turns: u32) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let run = AgentRun::new(id, parent, depth, kind, label, max_turns);
    with_runs(move |runs| {
        // Вытесняем старые завершённые, чтобы список не рос без предела.
        while runs.iter().filter(|r| !r.state.is_running()).count() > MAX_FINISHED {
            if let Some(pos) = runs.iter().position(|r| !r.state.is_running()) {
                runs.remove(pos);
            } else {
                break;
            }
        }
        runs.push(run);
    });
    id
}

/// Точечная правка карточки. Если карточки уже нет (вытеснена) — no-op.
pub fn patch(id: u64, f: impl FnOnce(&mut AgentRun) + Send + 'static) {
    with_runs(move |runs| {
        if let Some(run) = runs.iter_mut().find(|r| r.id == id) {
            f(run);
        }
    });
}

/// Завершает цикл: карточка остаётся в панели с финальными числами, но
/// перестаёт считаться живой (сворачивается и теряет пульсирующую точку).
pub fn finish(id: u64, state: RunState) {
    patch(id, move |run| {
        run.state = state;
        run.tool = None;
    });
}

/// Снести все карточки — вызывается на старте новой генерации основного
/// чата: числа прошлого хода к новому вопросу отношения не имеют.
pub fn reset() {
    with_runs(|runs| runs.clear());
}

fn with_runs(f: impl FnOnce(&mut Vec<AgentRun>) + Send + 'static) {
    run_on_main_thread(move || {
        // Контекст — синглтон приложения, но в тестах и на раннем старте его
        // может не быть: телеметрия не тот повод, чтобы падать.
        if let Some(ctx) = try_use_context::<SynChatCtx>() {
            ctx.agent_runs.update(f);
        }
    });
}
