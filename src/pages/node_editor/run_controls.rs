//! Floating Run / Pause / Stop pill — кладётся в верхнюю overlay-строку
//! canvas-Stack'а (см. `mod.rs::overlay_row`).
//!
//! По нажатию Run строим зависимостно-ориентированную очередь запусков
//! (`RunQueue`) над on_run-нодами графа: каждая нода ожидает, пока все её
//! upstream-`on_run`-предки завершатся (`busy_signal=false`), и только
//! затем стартует. Pass-through-ноды без `on_run` (TextView, AudioRecorder,
//! AudioPlayer и т. п.) проходятся как «прозрачные» — они публикуют
//! значения реактивно через `evaluate_graph` и не требуют явного старта.
//!
//! Для линейной цепочки ASR → TextView → OmniVoice это даёт строго
//! последовательный запуск (OmniVoice ждёт ASR), для разветвлённого графа
//! — параллельный запуск независимых ветвей.
//!
//! Pulse-анимация активной кнопки реализована в MSS через `@keyframes`
//! на классе `.ne-run-btn--active`.
//!
//! **Где живёт очередь.** `RunQueue` лежит в статике [`run_queue`], а не в
//! локальной переменной `view()`, и watcher-effect ставится один раз на
//! старте приложения ([`install_run_watcher`]). Причина: страница нодового
//! редактора пересобирается целиком при переключении вкладки роутера
//! (`RouterView::build_children`) и при смене вкладки графа (`Reactive` в
//! `mod.rs::canvas_area`). Пока очередь и effect жили внутри `view()`,
//! любой уход со страницы во время прогона уничтожал sequencer: worker
//! досчитывал (в логе оставалось «воркер: готово»), но финиш ноды никто
//! уже не замечал, следующая нода не стартовала, а свежепостроенный
//! watcher видел «никто не занят при run_state=Running» и молча гасил
//! pill. Именно так терялись 16-минутные прогоны H3.
//!
//! Логирование прогона идёт в target `node-editor.run` на уровне INFO:
//! нажатия Run/Pause/Stop, построение очереди, старт и финиш каждой
//! ноды с длительностью, разблокировка преемников и итог прогона.
//! Этого достаточно, чтобы по логу восстановить, где встал граф, без
//! правки самих нод — хуки `on_run`/`busy_signal` инструментируются
//! здесь централизованно.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::Instant;

use syngui::core::sync::Mutex;
use tracing::{info, warn};
use syngui::prelude::*;
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row, ToolButton};

use crate::context::AppCtx;
use crate::icons::{MI_PAUSE, MI_PLAY_ARROW, MI_STOP};

use super::eval;
use super::registry;
use super::state::NodeEditorCtx;
use super::tabs::{EditorWorkspace, RunState};
use super::types::{Connection, NodeId, NodeInstance};

/// Чем закончился прогон — для внешнего наблюдателя ([`RunOutcome`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// Очередь опустела сама: все on_run-ноды отработали.
    Completed,
    /// Прогон остановлен (Stop в pill или [`cancel_active_run`]).
    Stopped,
    /// Новый Run заместил очередь до завершения этого прогона.
    Superseded,
}

/// Итог работы одной on_run-ноды в прогоне.
#[derive(Debug, Clone)]
pub struct NodeRunReport {
    pub id: u64,
    pub title: &'static str,
    pub elapsed_ms: u64,
    /// Ошибка ноды на момент её финиша (`NodeRuntime::run_error_signal`).
    pub error: Option<String>,
}

/// Итог прогона, отправляемый в oneshot-канал `notify` (если внешний
/// инициатор — агент Syn-чата — его передал в [`start_run`]).
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub end: RunEnd,
    pub total_ms: u64,
    /// Отчёты завершившихся нод в порядке финиша. При `Stopped`/`Superseded`
    /// ноды, не успевшие финишировать, сюда не попадают.
    pub nodes: Vec<NodeRunReport>,
}

impl RunOutcome {
    /// Ошибки прогона одним списком (нода → текст).
    pub fn errors(&self) -> Vec<(&'static str, &str)> {
        self.nodes
            .iter()
            .filter_map(|r| r.error.as_deref().map(|e| (r.title, e)))
            .collect()
    }
}

/// Почему прогон не стартовал. UI мапит в notification, агент — в текст
/// ошибки инструмента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRunError {
    /// В графе нет ни одной enabled-ноды с `on_run`.
    NoRunnableNodes,
    /// on_run-ноды есть, но все ждут предков — в графе цикл.
    CycleInGraph,
}

/// Sequencer-стейт одного «Run»-прохода. Живёт в статике [`run_queue`],
/// переживая пересборку страницы. `None` — между runs.
struct RunQueue {
    /// Граф, по которому идёт прогон. Зафиксирован в момент нажатия Run:
    /// переключение вкладки графа или страницы больше не уводит watcher
    /// на чужой `NodeEditorCtx`.
    ctx: NodeEditorCtx,
    /// Сколько ещё on_run-предков должно завершиться, прежде чем ноду
    /// разрешено стартовать. При `0` нода готова к fire.
    remaining: HashMap<NodeId, usize>,
    /// Для каждой on_run-ноды — список её on_run-преемников (ближайших
    /// по графу, через пассивные звенья). При завершении ноды
    /// декрементим `remaining` у этих преемников.
    downstream: HashMap<NodeId, Vec<NodeId>>,
    /// Ноды, у которых уже вызван `on_run` и которые ждут флипа
    /// `busy_signal → false`.
    active: HashSet<NodeId>,
    /// Момент вызова `on_run` каждой активной ноды — только для замера
    /// длительности в логе (`node-editor.run`).
    started_at: HashMap<NodeId, Instant>,
    /// Момент нажатия Run — для итоговой строки прогона.
    run_started_at: Instant,
    /// Канал итога для внешнего инициатора прогона (агент Syn-чата).
    /// None у прогонов, запущенных кнопкой Run.
    notify: Option<tokio::sync::oneshot::Sender<RunOutcome>>,
    /// Отчёты финишировавших нод — копятся по ходу прогона.
    reports: Vec<NodeRunReport>,
}

impl RunQueue {
    fn is_done(&self) -> bool {
        self.active.is_empty() && self.remaining.values().all(|c| *c == 0)
    }
}

/// Отправить итог прогона внешнему инициатору (если он есть). Забирает
/// очередь по значению: вызывается ровно в трёх местах, где она
/// выкорчёвывается из статики — завершение, Stop/отмена, замещение новым Run.
fn send_outcome(mut q: RunQueue, end: RunEnd) {
    if let Some(tx) = q.notify.take() {
        let _ = tx.send(RunOutcome {
            end,
            total_ms: q.run_started_at.elapsed().as_millis() as u64,
            nodes: std::mem::take(&mut q.reports),
        });
    }
}

/// Текущая ошибка ноды (`NodeRuntime::run_error_signal`). try_lock — чтобы
/// main thread не встал, если воркер ноды ещё держит runtime.
fn node_error(ctx: &NodeEditorCtx, id: NodeId) -> Option<String> {
    let nodes = ctx.nodes.get_untracked();
    let n = nodes.iter().find(|n| n.id == id)?;
    let rt = n.runtime.try_lock().ok()?;
    rt.run_error_signal().and_then(|s| s.get_untracked())
}

/// Глобальный слот очереди. Статика, а не поле `EditorWorkspace`, потому
/// что `EditorWorkspace` — `Copy`-структура из одних `RwSignal`, а очередь
/// хранит не-реактивное состояние (`Instant`, множества id).
fn run_queue() -> &'static Mutex<Option<RunQueue>> {
    static QUEUE: OnceLock<Mutex<Option<RunQueue>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(None))
}

/// Построить очередь зависимостей: для каждой on_run-ноды посчитать
/// число «ближайших on_run-предков» в DAG (через пассивные ноды).
///
/// BFS от каждой on_run-ноды `a` через `out_edges`: пассивные звенья
/// прозрачно пробрасываются, on_run-ноды-потомки добавляются в
/// `downstream[a]` и инкрементируют `remaining`.
fn build_queue(ctx: NodeEditorCtx, nodes: &[NodeInstance], conns: &[Connection]) -> RunQueue {
    let on_run_set: HashSet<NodeId> = nodes
        .iter()
        .filter(|n| n.enabled.get_untracked() && registry::meta(n.kind).on_run.is_some())
        .map(|n| n.id)
        .collect();

    let mut out_edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in conns {
        out_edges.entry(c.from_node).or_default().push(c.to_node);
    }

    let mut remaining: HashMap<NodeId, usize> =
        on_run_set.iter().map(|id| (*id, 0_usize)).collect();
    let mut downstream: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for &a in &on_run_set {
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut q: VecDeque<NodeId> = VecDeque::new();
        if let Some(succ) = out_edges.get(&a) {
            for &m in succ {
                q.push_back(m);
            }
        }
        let mut downs: Vec<NodeId> = Vec::new();
        while let Some(x) = q.pop_front() {
            if x == a || !visited.insert(x) {
                continue;
            }
            if on_run_set.contains(&x) {
                downs.push(x);
                // Цепочка обрывается на ближайшем on_run-потомке: дальше
                // искать предков `a` сквозь `x` смысла нет — это работа
                // отдельного BFS из `x`.
            } else if let Some(succ) = out_edges.get(&x) {
                for &m in succ {
                    q.push_back(m);
                }
            }
        }
        if !downs.is_empty() {
            for d in &downs {
                *remaining.entry(*d).or_insert(0) += 1;
            }
            downstream.insert(a, downs);
        }
    }

    RunQueue {
        ctx,
        remaining,
        downstream,
        active: HashSet::new(),
        started_at: HashMap::new(),
        run_started_at: Instant::now(),
        notify: None,
        reports: Vec::new(),
    }
}

/// Synchronously пересчитать `ctx.values`. Реактивный effect в `state.rs`
/// обновляет их асинхронно — а sequencer'у нужны fresh-выходы прямо
/// сейчас, до того как `on_run` прочитает `values.get_untracked()`.
fn refresh_values(ctx: &NodeEditorCtx) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let map = eval::evaluate_graph(&nodes, &conns, false);
    ctx.values.set(map);
}

/// Вызвать `on_run` ноды и поставить её в `active`. Hook сам выставит
/// `running.set(true)` — это и сделает её busy для watcher'а.
fn fire(ctx: &NodeEditorCtx, id: NodeId, q: &mut RunQueue) {
    let nodes = ctx.nodes.get_untracked();
    let Some(node) = nodes.iter().find(|n| n.id == id) else {
        info!(target: RUN_LOG, node = id.0, "старт пропущен: ноды нет в графе");
        return;
    };
    let meta = registry::meta(node.kind);
    let Some(hook) = meta.on_run else {
        info!(
            target: RUN_LOG,
            node = id.0,
            title = meta.title,
            "старт пропущен: у ноды нет on_run"
        );
        return;
    };
    info!(target: RUN_LOG, node = id.0, title = meta.title, "нода: старт");
    q.active.insert(id);
    q.started_at.insert(id, Instant::now());
    hook(node, ctx);
}

/// Target логов прогона. Отдельная константа, чтобы `RUST_LOG` и
/// файловый EnvFilter могли адресовать его одной директивой.
const RUN_LOG: &str = "node-editor.run";

/// Человекочитаемое имя ноды для логов — `title` из реестра.
fn node_title(ctx: &NodeEditorCtx, id: NodeId) -> &'static str {
    ctx.nodes
        .get_untracked()
        .iter()
        .find(|n| n.id == id)
        .map(|n| registry::meta(n.kind).title)
        .unwrap_or("?")
}

/// Собрать busy-снимок enabled-нод. Используется и watcher'ом для
/// определения «just_finished», и legacy-веткой (per-node Play).
fn collect_busy(nodes: &[NodeInstance]) -> HashMap<NodeId, bool> {
    nodes
        .iter()
        .filter(|n| n.enabled.get_untracked())
        .filter_map(|n| {
            let meta = registry::meta(n.kind);
            meta.busy_signal
                .and_then(|f| f(n))
                .map(|sig| (n.id, sig.get()))
        })
        .collect()
}

/// Поставить watcher прогона. Вызывается один раз на старте приложения
/// (`lib.rs`) — вне element-scope, поэтому `cleanup_element` при пересборке
/// страниц его не гасит.
///
/// Подписки эффекта пересобираются на каждом запуске: `run_state` — чтобы
/// нажатие Run разбудило watcher, когда очередь пуста и подписок на
/// busy-сигналы ещё нет; `nodes` + `busy_signal` каждой enabled-ноды —
/// чтобы флип «busy → idle» дошёл до sequencer'а.
pub fn install_run_watcher() {
    create_effect(move || {
        let ws = use_context::<EditorWorkspace>();
        let _ = ws.run_state.get();

        let mut guard = match run_queue().lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let Some(q) = guard.as_mut() else {
            // Run не активен — legacy auto-Stop для per-node Play / других
            // внешних запусков (на будущее). Слушаем активную вкладку.
            let ctx = match ws.active_ctx() {
                Some(c) => c,
                None => return,
            };
            let nodes = ctx.nodes.get();
            let busy = collect_busy(&nodes);
            if !busy.values().any(|b| *b) && ws.run_state.get_untracked() == RunState::Running {
                info!(
                    target: RUN_LOG,
                    "run: авто-стоп — очереди нет, ни одна нода не занята"
                );
                ws.run_timer.finish();
                ws.run_state.set(RunState::Stopped);
            }
            return;
        };

        if advance(&ws, q) {
            let q = match guard.take() {
                Some(q) => q,
                None => return,
            };
            drop(guard);
            info!(
                target: RUN_LOG,
                total_ms = q.run_started_at.elapsed().as_millis() as u64,
                done = ws.run_done.get_untracked(),
                "run: очередь пуста, прогон завершён"
            );
            send_outcome(q, RunEnd::Completed);
            ws.run_timer.finish();
            ws.run_state.set(RunState::Stopped);
        }
    });
}

/// Продвинуть очередь до устойчивого состояния: обработать завершившиеся
/// активные ноды, разблокировать и запустить преемников — и повторить,
/// потому что hook может отработать синхронно, не тронув ни одного
/// отслеживаемого сигнала (типовой случай — sampler без входа: `error.set`,
/// `return`, `running` так и не выставлен). Без повторного прохода такая
/// нода зависала бы в `active` навсегда: watcher-effect без флипа сигнала
/// не перезапустится. Возвращает `is_done()`.
///
/// Вызывается из watcher-effect (там `.get()` внутри `collect_busy` заодно
/// оформляет подписки) и из [`start_run`] сразу после старта корней.
fn advance(ws: &EditorWorkspace, q: &mut RunQueue) -> bool {
    let ctx = q.ctx;
    let mut finished_total = 0usize;
    loop {
        let nodes = ctx.nodes.get();
        let busy = collect_busy(&nodes);

        let just_finished: Vec<NodeId> = q
            .active
            .iter()
            .copied()
            .filter(|id| match busy.get(id) {
                Some(is_busy) => !*is_busy,
                // Ноду удалили или выключили посреди прогона — её
                // busy-сигнала в снимке больше нет. Считаем завершённой:
                // иначе очередь ждала бы её вечно.
                None => {
                    warn!(
                        target: RUN_LOG,
                        node = id.0,
                        "нода пропала из busy-снимка во время прогона — считаем завершённой"
                    );
                    true
                }
            })
            .collect();
        if just_finished.is_empty() {
            break;
        }
        finished_total += just_finished.len();
        for id in &just_finished {
            let elapsed_ms = q
                .started_at
                .remove(id)
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            let error = node_error(&ctx, *id);
            if let Some(err) = &error {
                warn!(
                    target: RUN_LOG,
                    node = id.0,
                    title = node_title(&ctx, *id),
                    error = err.as_str(),
                    "нода: финиш с ошибкой"
                );
            } else {
                info!(
                    target: RUN_LOG,
                    node = id.0,
                    title = node_title(&ctx, *id),
                    elapsed_ms,
                    "нода: финиш"
                );
            }
            q.reports.push(NodeRunReport {
                id: id.0,
                title: node_title(&ctx, *id),
                elapsed_ms,
                error,
            });
        }

        // Свежий evaluate, чтобы output_text / output_buf завершившихся
        // нод попали в values до того, как downstream `current_input_*`
        // их прочитает.
        refresh_values(&ctx);

        for id in &just_finished {
            q.active.remove(id);
            if let Some(succ_list) = q.downstream.get(id).cloned() {
                for d in succ_list {
                    if let Some(c) = q.remaining.get_mut(&d) {
                        *c = c.saturating_sub(1);
                        if *c == 0 && !q.active.contains(&d) {
                            info!(
                                target: RUN_LOG,
                                node = d.0,
                                title = node_title(&ctx, d),
                                after = id.0,
                                "нода разблокирована предком"
                            );
                            fire(&ctx, d, q);
                        } else {
                            info!(
                                target: RUN_LOG,
                                node = d.0,
                                title = node_title(&ctx, d),
                                waiting_on = *c,
                                "нода ждёт остальных предков"
                            );
                        }
                    }
                }
            }
            q.remaining.remove(id);
        }
    }

    if finished_total > 0 {
        // Счётчик «сделано/всего» в Run-pill. Считаем здесь, а не по
        // busy-снимку: снимок не различает «ещё не стартовала» и
        // «уже отработала».
        let done_before = ws.run_done.get_untracked();
        ws.run_done.set(done_before + finished_total);
        if !q.is_done() {
            info!(
                target: RUN_LOG,
                active = q.active.len(),
                pending = q.remaining.len(),
                "run: очередь продвинулась"
            );
        }
    }
    q.is_done()
}

/// Запустить прогон графа `editor_ctx`. Общий вход кнопки Run и агентского
/// инструмента `pipelines`. Только main thread: внутри — сигналы и context.
///
/// `notify` — oneshot, в который уйдёт [`RunOutcome`], когда прогон
/// завершится, будет остановлен или замещён новым Run.
///
/// Возвращает число нод, запущенных сразу (корни DAG).
pub fn start_run(
    editor_ctx: NodeEditorCtx,
    notify: Option<tokio::sync::oneshot::Sender<RunOutcome>>,
    // `std::result` явно: prelude syngui затеняет Result своим алиасом.
) -> std::result::Result<usize, StartRunError> {
    let ws = use_context::<EditorWorkspace>();

    // Очередь одна на приложение (pill в EditorWorkspace тоже один).
    // Перезапуск поверх живого прогона легален — уже работающие воркеры
    // досчитают и будут учтены новой очередью, — но прежний инициатор
    // обязан узнать, что его прогон замещён (иначе агент ждал бы вечно).
    if let Ok(mut g) = run_queue().lock() {
        if let Some(prev) = g.take() {
            warn!(
                target: RUN_LOG,
                active = prev.active.len(),
                pending = prev.remaining.len(),
                "run: Run поверх незавершённого прогона — очередь пересобирается"
            );
            send_outcome(prev, RunEnd::Superseded);
        }
    }

    let nodes = editor_ctx.nodes.get_untracked();
    let conns = editor_ctx.connections.get_untracked();
    let mut q = build_queue(editor_ctx, &nodes, &conns);
    q.notify = notify;
    info!(
        target: RUN_LOG,
        nodes = nodes.len(),
        connections = conns.len(),
        on_run = q.remaining.len(),
        "run: очередь построена"
    );

    // Свежий evaluate перед стартом корней — чтобы свежий
    // AudioRecorder.last_result уже сидел в values, а не
    // «застрял» в предыдущем evaluate.
    refresh_values(&editor_ctx);

    // Корни — все on_run-ноды с remaining == 0.
    let roots: Vec<NodeId> = q
        .remaining
        .iter()
        .filter(|(_, c)| **c == 0)
        .map(|(id, _)| *id)
        .collect();
    info!(target: RUN_LOG, roots = roots.len(), "run: старт корней");
    for id in roots {
        fire(&editor_ctx, id, &mut q);
    }
    let started = q.active.len();
    let stuck = q.remaining.values().any(|c| *c > 0) && q.active.is_empty();

    if started == 0 {
        if stuck {
            info!(target: RUN_LOG, "run: отменён — цикл в графе");
            return Err(StartRunError::CycleInGraph);
        }
        info!(target: RUN_LOG, "run: отменён — нет on_run-нод");
        return Err(StartRunError::NoRunnableNodes);
    }

    // Глобальный секундомер: отсчёт от нажатия Run до опустошения
    // очереди. `run_total` — все on_run-ноды прогона, включая те,
    // что ещё ждут своих upstream-предков.
    ws.run_total.set(q.remaining.len());
    ws.run_done.set(0);
    ws.run_timer.start();
    q.run_started_at = Instant::now();

    // Дренаж корней, завершившихся синхронно прямо в fire (hook мог
    // выставить error и выйти, не тронув running) — иначе очередь легла бы
    // в статику уже зависшей.
    if advance(&ws, &mut q) {
        info!(
            target: RUN_LOG,
            total_ms = q.run_started_at.elapsed().as_millis() as u64,
            "run: все ноды завершились синхронно на старте"
        );
        send_outcome(q, RunEnd::Completed);
        ws.run_timer.finish();
        ws.run_state.set(RunState::Stopped);
        return Ok(started);
    }

    if let Ok(mut g) = run_queue().lock() {
        *g = Some(q);
    }
    ws.run_state.set(RunState::Running);
    Ok(started)
}

/// Остановить прогон: выкорчевать очередь, отдать инициатору `Stopped`,
/// финализировать pill. `cancel_workers` — дополнительно взвести
/// cooperative-cancel флаги активных нод. Возвращает false, если прогона
/// не было. Только main thread.
fn stop_run(cancel_workers: bool) -> bool {
    let ws = use_context::<EditorWorkspace>();
    let taken = match run_queue().lock() {
        Ok(mut g) => g.take(),
        Err(_) => None,
    };
    let Some(q) = taken else {
        info!(target: RUN_LOG, "run: Stop вне прогона");
        // Фиксируем возможный legacy-Running (per-node Play).
        ws.run_timer.finish();
        ws.run_state.set(RunState::Stopped);
        return false;
    };
    if cancel_workers {
        let nodes = q.ctx.nodes.get_untracked();
        for id in &q.active {
            let Some(n) = nodes.iter().find(|n| n.id == *id) else {
                continue;
            };
            // try_lock: если воркер держит runtime, флаг не взвести — нода
            // доработает, как и ноды вовсе без cancel (ACE-Step Generate).
            if let Ok(rt) = n.runtime.try_lock() {
                if let Some(flag) = rt.run_cancel_flag() {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    info!(target: RUN_LOG, node = id.0, "нода: взведён cancel");
                }
            }
        }
    }
    info!(
        target: RUN_LOG,
        active = q.active.len(),
        pending = q.remaining.len(),
        cancel_workers,
        "run: очередь сброшена (активные worker'ы дорабатывают)"
    );
    send_outcome(q, RunEnd::Stopped);
    // Фиксируем то, что успело натикать: уже запущенные worker'ы
    // доработают, но прогон как таковой закончился здесь.
    ws.run_timer.finish();
    ws.run_state.set(RunState::Stopped);
    true
}

/// Отменить текущий прогон извне (агент Syn-чата, abort хода): сброс
/// очереди + cooperative-cancel активных нод. LTX/H3-сэмплеры прерываются
/// через `DenoiseHooks`; ноды без флага досчитывают до конца.
pub fn cancel_active_run() -> bool {
    stop_run(true)
}

/// Статус активных нод текущего прогона: (заголовок, прогресс 0..1 если у
/// ноды есть сигнал). Для живой карточки прогона в чате. Только main
/// thread; прогресс читается tracked (`.get()`) — вызов из `Reactive`
/// подпишет карточку на обновления.
pub fn active_nodes_status() -> Vec<(String, Option<f32>)> {
    let Ok(g) = run_queue().lock() else {
        return Vec::new();
    };
    let Some(q) = g.as_ref() else {
        return Vec::new();
    };
    let nodes = q.ctx.nodes.get_untracked();
    let mut out: Vec<(String, Option<f32>)> = Vec::new();
    for id in &q.active {
        let Some(n) = nodes.iter().find(|n| n.id == *id) else {
            continue;
        };
        let pct = n
            .runtime
            .try_lock()
            .ok()
            .and_then(|rt| rt.run_progress_signal())
            .map(|s| s.get());
        out.push((registry::meta(n.kind).title.to_string(), pct));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Сборка pill — фиксированный размер в MSS (`min-width` / `height`).
///
/// `editor_ctx` передаётся явно потому что `run_controls` живёт в
/// `overlay_row`, который сидит ВЫШЕ `world_layer` (`provide_context(ctx)`
/// делается там). Без явной передачи `use_context::<NodeEditorCtx>()`
/// упал бы по «Context not provided» — что и было.
pub fn view(editor_ctx: NodeEditorCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ws = use_context::<EditorWorkspace>();
        let app = use_context::<AppCtx>();
        let state = ws.run_state.get();

        let app_run = app.clone();
        let run_btn = ToolButton::new(MI_PLAY_ARROW)
            .tooltip(tr!("nodes.run.start"))
            .on_click(move || {
                info!(target: RUN_LOG, "run: нажат Run");
                match start_run(editor_ctx, None) {
                    Ok(started) => {
                        app_run.notifications.info(tr!("nodes.run.started_notice", count = started));
                    }
                    Err(StartRunError::CycleInGraph) => {
                        app_run.notifications.info(tr!("nodes.run.cycle_error"));
                    }
                    Err(StartRunError::NoRunnableNodes) => {
                        app_run.notifications.info(tr!("nodes.run.no_runnable"));
                    }
                }
            })
            .class(button_class("ne-run-btn ne-run-btn--play", state, RunState::Running));

        let app_pause = app.clone();
        let pause_btn = ToolButton::new(MI_PAUSE)
            .tooltip(tr!("nodes.run.pause"))
            .on_click(move || {
                info!(target: RUN_LOG, "run: нажат Pause");
                ws.run_state.set(RunState::Paused);
                app_pause.notifications.info(tr!("nodes.run.paused_notice"));
            })
            .class(button_class("ne-run-btn ne-run-btn--pause", state, RunState::Paused));

        let app_stop = app.clone();
        let stop_btn = ToolButton::new(MI_STOP)
            .tooltip(tr!("nodes.run.stop"))
            .on_click(move || {
                info!(target: RUN_LOG, "run: нажат Stop");
                // Уже запущенные worker'ы доработают (кнопка исторически не
                // взводит cancel), но новых fire'ов больше не произойдёт.
                stop_run(false);
                app_stop.notifications.info(tr!("nodes.run.stopped_notice"));
            })
            .class(button_class("ne-run-btn ne-run-btn--stop", state, RunState::Stopped));

        // Секундомер прогона справа от кнопок, за тонким разделителем
        // (разделитель — часть бейджа, поэтому исчезает вместе с ним).
        // До первого Run бейдж пуст — pill остаётся компактным.
        let timer = super::timing::run_timer_badge(ws.run_timer, (ws.run_done, ws.run_total));

        let pill = DecoratedBox::new()
            .child(Padding::symmetric(8.0, 4.0).child(
                Row::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .children(vec![
                        Box::new(run_btn) as Box<dyn Widget>,
                        Box::new(pause_btn) as Box<dyn Widget>,
                        Box::new(stop_btn) as Box<dyn Widget>,
                        timer,
                    ]),
            ))
            .class(format!(
                "ne-run-controls ne-run-controls--{}",
                state.label().to_lowercase()
            ));
        vec![Box::new(pill)]
    })
}

fn button_class(base: &str, current: RunState, target: RunState) -> String {
    if current == target {
        format!("{base} ne-run-btn--active")
    } else {
        base.to_string()
    }
}
