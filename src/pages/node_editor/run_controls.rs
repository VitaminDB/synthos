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
//! Логирование прогона идёт в target `node-editor.run` на уровне INFO:
//! нажатия Run/Pause/Stop, построение очереди, старт и финиш каждой
//! ноды с длительностью, разблокировка преемников и итог прогона.
//! Этого достаточно, чтобы по логу восстановить, где встал граф, без
//! правки самих нод — хуки `on_run`/`busy_signal` инструментируются
//! здесь централизованно.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use syngui::core::sync::Mutex;
use tracing::info;
use syngui::prelude::*;
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row, ToolButton};

use crate::context::AppCtx;
use crate::icons::{MI_PAUSE, MI_PLAY_ARROW, MI_STOP};

use super::eval;
use super::registry;
use super::state::NodeEditorCtx;
use super::tabs::{EditorWorkspace, RunState};
use super::types::{Connection, NodeId, NodeInstance};

/// Sequencer-стейт одного «Run»-прохода. Живёт в локальном
/// `Arc<Mutex<Option<RunQueue>>>` внутри `view()`. `None` — между runs.
struct RunQueue {
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
}

impl RunQueue {
    fn is_done(&self) -> bool {
        self.active.is_empty() && self.remaining.values().all(|c| *c == 0)
    }
}

/// Построить очередь зависимостей: для каждой on_run-ноды посчитать
/// число «ближайших on_run-предков» в DAG (через пассивные ноды).
///
/// BFS от каждой on_run-ноды `a` через `out_edges`: пассивные звенья
/// прозрачно пробрасываются, on_run-ноды-потомки добавляются в
/// `downstream[a]` и инкрементируют `remaining`.
fn build_queue(nodes: &[NodeInstance], conns: &[Connection]) -> RunQueue {
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
        remaining,
        downstream,
        active: HashSet::new(),
        started_at: HashMap::new(),
        run_started_at: Instant::now(),
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

/// Сборка pill — фиксированный размер в MSS (`min-width` / `height`).
///
/// `editor_ctx` передаётся явно потому что `run_controls` живёт в
/// `overlay_row`, который сидит ВЫШЕ `world_layer` (`provide_context(ctx)`
/// делается там). Без явной передачи `use_context::<NodeEditorCtx>()`
/// упал бы по «Context not provided» — что и было.
pub fn view(editor_ctx: NodeEditorCtx) -> impl Widget {
    let ws_init = use_context::<EditorWorkspace>();

    // Sequencer-стейт текущего run'а. None между запусками; Some(...) на
    // время Running. Захватывается и run/stop-кнопками, и watcher-эффектом.
    let queue_cell: Arc<Mutex<Option<RunQueue>>> = Arc::new(Mutex::new(None));

    // Watcher: подписан на все busy_signal'ы. Когда нода из `active`
    // флипается в idle — декрементим её преемников, fire'им готовых,
    // при опустошении очереди возвращаем pill в Stopped.
    {
        let queue_cell = queue_cell.clone();
        create_effect(move || {
            let nodes = editor_ctx.nodes.get();
            let busy = collect_busy(&nodes);

            let mut guard = match queue_cell.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let Some(q) = guard.as_mut() else {
                // Run не активен — оставляем legacy auto-Stop для per-node
                // Play / других внешних запусков (на будущее).
                if !busy.values().any(|b| *b)
                    && ws_init.run_state.get_untracked() == RunState::Running
                {
                    ws_init.run_timer.finish();
                    ws_init.run_state.set(RunState::Stopped);
                }
                return;
            };

            let just_finished: Vec<NodeId> = q
                .active
                .iter()
                .copied()
                .filter(|id| !busy.get(id).copied().unwrap_or(true))
                .collect();
            if just_finished.is_empty() {
                return;
            }
            let finished_now = just_finished.len();
            for id in &just_finished {
                let elapsed_ms = q
                    .started_at
                    .remove(id)
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                info!(
                    target: RUN_LOG,
                    node = id.0,
                    title = node_title(&editor_ctx, *id),
                    elapsed_ms,
                    "нода: финиш"
                );
            }

            // Свежий evaluate, чтобы output_text / output_buf завершившихся
            // нод попали в values до того, как downstream `current_input_*`
            // их прочитает.
            refresh_values(&editor_ctx);

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
                                    title = node_title(&editor_ctx, d),
                                    after = id.0,
                                    "нода разблокирована предком"
                                );
                                fire(&editor_ctx, d, q);
                            } else {
                                info!(
                                    target: RUN_LOG,
                                    node = d.0,
                                    title = node_title(&editor_ctx, d),
                                    waiting_on = *c,
                                    "нода ждёт остальных предков"
                                );
                            }
                        }
                    }
                }
                q.remaining.remove(id);
            }
            // Счётчик «сделано/всего» в Run-pill. Считаем здесь, а не по
            // busy-снимку: снимок не различает «ещё не стартовала» и
            // «уже отработала».
            let done_before = ws_init.run_done.get_untracked();
            ws_init.run_done.set(done_before + finished_now);

            if q.is_done() {
                info!(
                    target: RUN_LOG,
                    total_ms = q.run_started_at.elapsed().as_millis() as u64,
                    done = ws_init.run_done.get_untracked(),
                    "run: очередь пуста, прогон завершён"
                );
                *guard = None;
                drop(guard);
                ws_init.run_timer.finish();
                ws_init.run_state.set(RunState::Stopped);
            } else {
                info!(
                    target: RUN_LOG,
                    active = q.active.len(),
                    pending = q.remaining.len(),
                    "run: очередь продвинулась"
                );
            }
        });
    }

    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ws = use_context::<EditorWorkspace>();
        let app = use_context::<AppCtx>();
        let state = ws.run_state.get();

        let app_run = app.clone();
        let run_queue_cell = queue_cell.clone();
        let run_btn = ToolButton::new(MI_PLAY_ARROW)
            .tooltip("Run")
            .on_click(move || {
                let nodes = editor_ctx.nodes.get_untracked();
                let conns = editor_ctx.connections.get_untracked();
                let mut q = build_queue(&nodes, &conns);
                info!(
                    target: RUN_LOG,
                    nodes = nodes.len(),
                    connections = conns.len(),
                    on_run = q.remaining.len(),
                    "run: нажат Run, очередь построена"
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
                    if let Ok(mut g) = run_queue_cell.lock() {
                        *g = None;
                    }
                    if stuck {
                        info!(target: RUN_LOG, "run: отменён — цикл в графе");
                        app_run.notifications.info(
                            "Граф содержит цикл — нет нод, готовых к запуску",
                        );
                    } else {
                        info!(target: RUN_LOG, "run: отменён — нет on_run-нод");
                        app_run.notifications.info("Нет нод с явным запуском");
                    }
                    return;
                }

                // Глобальный секундомер: отсчёт от нажатия Run до опустошения
                // очереди. `run_total` — все on_run-ноды прогона, включая те,
                // что ещё ждут своих upstream-предков.
                ws.run_total.set(q.remaining.len());
                ws.run_done.set(0);
                ws.run_timer.start();
                q.run_started_at = Instant::now();

                if let Ok(mut g) = run_queue_cell.lock() {
                    *g = Some(q);
                }
                ws.run_state.set(RunState::Running);
                app_run.notifications.info(format!("Запущено нод: {started}"));
            })
            .class(button_class("ne-run-btn ne-run-btn--play", state, RunState::Running));

        let app_pause = app.clone();
        let pause_btn = ToolButton::new(MI_PAUSE)
            .tooltip("Pause")
            .on_click(move || {
                info!(target: RUN_LOG, "run: нажат Pause");
                ws.run_state.set(RunState::Paused);
                app_pause.notifications.info("Paused");
            })
            .class(button_class("ne-run-btn ne-run-btn--pause", state, RunState::Paused));

        let app_stop = app.clone();
        let stop_queue_cell = queue_cell.clone();
        let stop_btn = ToolButton::new(MI_STOP)
            .tooltip("Stop")
            .on_click(move || {
                // Очищаем sequencer-state. Уже запущенные worker'ы
                // доработают (нет cooperative cancel), но новых fire'ов
                // больше не произойдёт.
                if let Ok(mut g) = stop_queue_cell.lock() {
                    if let Some(q) = g.as_ref() {
                        info!(
                            target: RUN_LOG,
                            active = q.active.len(),
                            pending = q.remaining.len(),
                            "run: нажат Stop, очередь сброшена (активные worker'ы дорабатывают)"
                        );
                    } else {
                        info!(target: RUN_LOG, "run: нажат Stop вне прогона");
                    }
                    *g = None;
                }
                // Фиксируем то, что успело натикать: уже запущенные worker'ы
                // доработают, но прогон как таковой закончился здесь.
                ws.run_timer.finish();
                ws.run_state.set(RunState::Stopped);
                app_stop.notifications.info("Stopped");
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
