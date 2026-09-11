//! Секундомеры выполнения: per-node (сколько работала конкретная нода) и
//! глобальный (сколько длился весь прогон графа по кнопке Run).
//!
//! [`Stopwatch`] — тройка сигналов вокруг монотонной точки старта:
//! - `started` — `Some(Instant)` пока идёт отсчёт, `None` когда стоит;
//! - `live_ms` — «живое» показание, его двигает тик-элемент (см.
//!   [`ticker`]) с частотой [`TICK_INTERVAL`];
//! - `last_ms` — длительность последнего завершённого измерения, остаётся
//!   на карточке после финиша.
//!
//! Тик сделан на `Element::animate` (через существующий
//! [`super::controls::ProgressTicker`]), а не на фоновом потоке: `animate`
//! вызывается движком на main-thread, поэтому секундомер читает свои
//! сигналы напрямую (`RwSignal::get*` вне main-thread паникует), а возврат
//! `true` из `tick` заставляет syngui запросить следующий кадр — цикл
//! держится сам, пока идёт отсчёт, и гаснет вместе с ним.
//!
//! Кто дёргает start/finish:
//! - ноды — эффект в [`super::state::NodeEditorCtx::new`], подписанный на
//!   `busy_signal` каждой ноды (ловит и глобальный Run, и per-node Play);
//! - глобальный прогон — [`super::run_controls`] на Run / Stop / опустошение
//!   очереди.

use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row};

use crate::icons::MI_TIMER;

use super::controls::{node_progress_animator, ProgressTicker};

/// Шаг обновления «живого» показания. 100 мс = ровно один разряд десятых
/// долей секунды в [`fmt_elapsed`]: чаще — лишние пересборки Reactive,
/// реже — заметные скачки цифры.
const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Секундомер одного измерения. `Copy` — как и остальные сигнальные
/// структуры редактора, кладётся в `NodeInstance` / `EditorWorkspace`
/// по значению.
#[derive(Clone, Copy)]
pub struct Stopwatch {
    /// Момент старта текущего измерения; `None` — секундомер стоит.
    started: RwSignal<Option<Instant>>,
    /// Показание идущего отсчёта, мс. Двигается [`ticker`]'ом.
    live_ms: RwSignal<u64>,
    /// Длительность последнего завершённого измерения, мс.
    last_ms: RwSignal<Option<u64>>,
}

impl Stopwatch {
    pub fn new() -> Self {
        Self {
            started: use_signal(None::<Instant>),
            live_ms: use_signal(0_u64),
            last_ms: use_signal(None::<u64>),
        }
    }

    /// Начать отсчёт с нуля. Предыдущий результат (`last_ms`) держится до
    /// первого тика — карточка не мигает пустотой на старте.
    pub fn start(&self) {
        self.live_ms.set(0);
        self.started.set(Some(Instant::now()));
    }

    /// Остановить отсчёт и зафиксировать длительность. No-op если
    /// секундомер уже стоит.
    pub fn finish(&self) {
        let Some(t0) = self.started.get_untracked() else {
            return;
        };
        let ms = t0.elapsed().as_millis() as u64;
        self.started.set(None);
        self.live_ms.set(ms);
        self.last_ms.set(Some(ms));
    }

    /// Сбросить в исходное состояние — и отсчёт, и прошлый результат.
    pub fn reset(&self) {
        self.started.set(None);
        self.live_ms.set(0);
        self.last_ms.set(None);
    }

    /// Идёт ли отсчёт (подписывает enclosing-effect на изменение).
    pub fn is_running(&self) -> bool {
        self.started.get().is_some()
    }

    /// То же без подписки — для эффектов, которые сами же и стартуют
    /// секундомер (иначе получили бы самоперезапуск).
    pub fn is_running_untracked(&self) -> bool {
        self.started.get_untracked().is_some()
    }

    /// Подтянуть «живое» показание под текущий момент. Вызывается
    /// [`ticker`]'ом на main-thread.
    pub fn tick(&self) {
        if let Some(t0) = self.started.get_untracked() {
            self.live_ms.set(t0.elapsed().as_millis() as u64);
        }
    }

    /// Что показывать на карточке: идущее показание, иначе результат
    /// последнего прогона, иначе `None` (измерений ещё не было).
    /// Читает сигналы tracked — рассчитано на вызов внутри `Reactive`.
    pub fn display_ms(&self) -> Option<u64> {
        if self.started.get().is_some() {
            Some(self.live_ms.get())
        } else {
            self.last_ms.get()
        }
    }
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Человекочитаемая длительность:
/// `840` → `0.8 с`, `12_400` → `12.4 с`, `65_300` → `1:05.3`,
/// `3_723_000` → `1:02:03`.
pub fn fmt_elapsed(ms: u64) -> String {
    let secs = ms / 1000;
    let tenths = (ms % 1000) / 100;
    if secs < 60 {
        format!("{secs}.{tenths} {}", tr!("nodes.unit.seconds"))
    } else if secs < 3600 {
        format!("{}:{:02}.{}", secs / 60, secs % 60, tenths)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

/// Невидимый (0×0) элемент, который на каждом кадре подкручивает
/// секундомер. Живёт рядом с текстом, а не внутри его `Reactive` — иначе
/// пересобирался бы на каждом тике вместе с текстом и терял накопленный
/// `dt`.
pub fn ticker(sw: Stopwatch) -> Box<dyn Widget> {
    node_progress_animator(Arc::new(StopwatchTicker {
        sw,
        acc: Mutex::new(Duration::ZERO),
    }))
}

struct StopwatchTicker {
    sw: Stopwatch,
    /// Накопитель `dt` между обновлениями сигнала — движок зовёт `tick`
    /// на каждом кадре (~60 Гц), а показание меняем раз в TICK_INTERVAL.
    acc: Mutex<Duration>,
}

impl ProgressTicker for StopwatchTicker {
    fn active(&self) -> bool {
        self.sw.is_running_untracked()
    }
    fn active_tracked(&self) -> bool {
        self.sw.is_running()
    }
    fn tick(&self, dt: Duration) -> bool {
        if !self.sw.is_running_untracked() {
            return false;
        }
        let due = match self.acc.lock() {
            Ok(mut acc) => {
                *acc += dt;
                if *acc >= TICK_INTERVAL {
                    *acc = Duration::ZERO;
                    true
                } else {
                    false
                }
            }
            // Отравленный mutex — не повод ронять анимацию: тикаем каждый
            // кадр, показание останется верным.
            Err(_) => true,
        };
        if due {
            self.sw.tick();
        }
        // true пока идёт отсчёт → syngui продолжает запрашивать кадры.
        true
    }
}

/// Бейдж таймера ноды — «⏱ 12.4 с» в шапке карточки. Пока нода работает
/// цифра тикает и бейдж подсвечен зелёным, после завершения остаётся
/// приглушённый итог. До первого запуска бейджа нет вовсе — шапка не
/// занята пустым местом.
pub fn node_timer_badge(sw: Stopwatch) -> Box<dyn Widget> {
    badge(sw, "node-timer", None, false)
}

/// Бейдж глобального прогона для Run-pill. `progress` — сигналы
/// «завершено / всего нод», рисуются мелким счётчиком справа от времени.
/// Слева от бейджа рисуется вертикальный разделитель, отделяющий его от
/// кнопок Run/Pause/Stop — он часть бейджа, поэтому пропадает вместе с ним.
pub fn run_timer_badge(
    sw: Stopwatch,
    progress: (RwSignal<usize>, RwSignal<usize>),
) -> Box<dyn Widget> {
    badge(sw, "ne-run-timer", Some(progress), true)
}

/// Общая сборка бейджа. `base` даёт префикс MSS-классов
/// (`{base}`, `{base}--running`, `{base}--done`, `{base}-icon`,
/// `{base}-text`, `{base}-count`, `{base}-sep`).
fn badge(
    sw: Stopwatch,
    base: &'static str,
    progress: Option<(RwSignal<usize>, RwSignal<usize>)>,
    leading_sep: bool,
) -> Box<dyn Widget> {
    let text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let running = sw.is_running();
        let Some(ms) = sw.display_ms() else {
            return Vec::new();
        };

        let mut row = Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![
                Box::new(Text::new(MI_TIMER).class(format!("{base}-icon"))) as Box<dyn Widget>,
                Box::new(Text::new(fmt_elapsed(ms)).class(format!("{base}-text"))),
            ]);

        // Счётчик «3/7» — только пока прогон идёт: после финиша время
        // уже само по себе итог, а счётчик был бы всегда «7/7».
        if let Some((done, total)) = progress {
            let total_n = total.get();
            if running && total_n > 0 {
                row = row.child(
                    Text::new(format!("{}/{}", done.get().min(total_n), total_n))
                        .class(format!("{base}-count")),
                );
            }
        }

        let state = if running { "running" } else { "done" };
        let chip = DecoratedBox::new()
            .child(Padding::symmetric(6.0, 2.0).child(row))
            .class(format!("{base} {base}--{state}"));

        if !leading_sep {
            return vec![Box::new(chip)];
        }
        vec![Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(vec![
                    Box::new(DecoratedBox::new().class(format!("{base}-sep")))
                        as Box<dyn Widget>,
                    Box::new(chip),
                ]),
        )]
    });

    Box::new(
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![ticker(sw), Box::new(text) as Box<dyn Widget>]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_seconds_with_tenths() {
        let unit = tr!("nodes.unit.seconds");
        assert_eq!(fmt_elapsed(0), format!("0.0 {unit}"));
        assert_eq!(fmt_elapsed(840), format!("0.8 {unit}"));
        assert_eq!(fmt_elapsed(12_400), format!("12.4 {unit}"));
        assert_eq!(fmt_elapsed(59_990), format!("59.9 {unit}"));
    }

    #[test]
    fn fmt_minutes_and_hours() {
        assert_eq!(fmt_elapsed(60_000), "1:00.0");
        assert_eq!(fmt_elapsed(65_300), "1:05.3");
        assert_eq!(fmt_elapsed(3_599_900), "59:59.9");
        assert_eq!(fmt_elapsed(3_600_000), "1:00:00");
        assert_eq!(fmt_elapsed(3_723_000), "1:02:03");
    }

    #[test]
    fn start_tick_finish_cycle() {
        let sw = Stopwatch::new();
        // До первого запуска показывать нечего — бейдж скрыт.
        assert!(!sw.is_running_untracked());
        assert_eq!(sw.display_ms(), None);

        sw.start();
        assert!(sw.is_running_untracked());
        assert_eq!(sw.display_ms(), Some(0));

        std::thread::sleep(Duration::from_millis(20));
        sw.tick();
        let live = sw.display_ms().expect("идёт отсчёт");
        assert!(live >= 15, "живое показание не двигается: {live}");

        sw.finish();
        assert!(!sw.is_running_untracked());
        let last = sw.display_ms().expect("итог остаётся после финиша");
        assert!(last >= live);

        // Повторный finish — no-op, итог не затирается нулём.
        sw.finish();
        assert_eq!(sw.display_ms(), Some(last));

        sw.reset();
        assert_eq!(sw.display_ms(), None);
    }

    /// Связка «busy-сигнал ноды → её секундомер» из
    /// `state::NodeEditorCtx::install_node_timers`. Флипаем `running`
    /// у runtime'а ASR-ноды напрямую — ровно это делает её worker.
    #[test]
    fn node_stopwatch_follows_busy_signal() {
        use super::super::registry;
        use super::super::state::NodeEditorCtx;
        use super::super::types::NodeKind;
        use syngui::core::Point;

        let ctx = NodeEditorCtx::new();
        let id = ctx.add_node(NodeKind::AsrGigaam, Point::zero());
        syngui::signal::drain_and_run_effects();

        let node = ctx.find_node(id).expect("нода добавлена");
        let busy = registry::meta(node.kind)
            .busy_signal
            .and_then(|hook| hook(&node))
            .expect("у ASR-ноды есть busy-сигнал");
        assert!(!node.timing.is_running_untracked());

        busy.set(true);
        syngui::signal::drain_and_run_effects();
        assert!(node.timing.is_running_untracked(), "отсчёт не начался");

        busy.set(false);
        syngui::signal::drain_and_run_effects();
        assert!(!node.timing.is_running_untracked(), "отсчёт не остановлен");
        assert!(
            node.timing.display_ms().is_some(),
            "итог прогона не зафиксирован"
        );
    }

    #[test]
    fn ticker_stops_with_the_stopwatch() {
        let sw = Stopwatch::new();
        let t = StopwatchTicker { sw, acc: Mutex::new(Duration::ZERO) };
        // Стоящий секундомер не должен держать кадры.
        assert!(!t.tick(Duration::from_millis(16)));
        sw.start();
        assert!(t.tick(Duration::from_millis(16)));
        sw.finish();
        assert!(!t.tick(Duration::from_millis(16)));
    }

    /// Reactive в syngui меряется по детям (`measure_loose`), но в этом
    /// проекте уже был случай схлопывания Reactive в 0×0 внутри Row
    /// (см. `node_editor::mod::build_toolbar`) — бейдж проверяем на
    /// реальном layout-проходе, а не на глаз.
    ///
    /// Ручной: `TestHarness::new` вызывает `signal::init_main_thread`, а
    /// `MAIN_THREAD_ID` в syngui — глобальный `OnceLock`. Занятый им поток
    /// делает чтение сигналов паникой во всех остальных тест-потоках, так
    /// что в общем прогоне тест участвовать не может. Запуск:
    /// `cargo test --features testing badge_takes_space -- --ignored`.
    #[cfg(feature = "testing")]
    #[test]
    #[ignore = "занимает глобальный MAIN_THREAD_ID syngui — только отдельным прогоном"]
    fn badge_takes_space_only_after_first_run() {
        use syngui::testing::TestHarness;

        let sw = Stopwatch::new();
        let mut h = TestHarness::new(node_timer_badge(sw));
        h.rebuild();
        h.layout_loose(400.0, 64.0);
        assert_eq!(
            h.root_size().width,
            0.0,
            "до первого прогона бейджа быть не должно"
        );

        sw.start();
        sw.tick();
        h.rebuild();
        h.layout_loose(400.0, 64.0);
        let size = h.root_size();
        assert!(
            size.width > 0.0 && size.height > 0.0,
            "бейдж схлопнулся: {size:?}"
        );
    }

    /// Реестр анимаций syngui (с 01.09) зовёт `animate` только у элементов
    /// с заявкой `wants_animate_tick`. Секундомер стартует извне (сигнал), и
    /// его аниматор обязан попасть в реестр и двигать показание по кадрам, а
    /// в простое — не существовать и не просить кадров. Раньше таймер стоял
    /// на «0.0 с» весь прогон. Ручной — по той же причине, что и тест выше.
    #[cfg(feature = "testing")]
    #[test]
    #[ignore = "занимает глобальный MAIN_THREAD_ID syngui — только отдельным прогоном"]
    fn badge_ticks_through_animation_registry() {
        use syngui::testing::TestHarness;

        let sw = Stopwatch::new();
        let mut h = TestHarness::new(node_timer_badge(sw));
        h.rebuild();
        h.layout_loose(400.0, 64.0);
        assert!(h.find_by_type_name("ProgressAnimator").is_empty(), "аниматор без прогона");
        assert!(!h.animate(Duration::from_millis(16)), "в простое кадров быть не должно");

        sw.start();
        h.rebuild();
        h.layout_loose(400.0, 64.0);
        let ids = h.find_by_type_name("ProgressAnimator");
        assert_eq!(ids.len(), 1, "аниматор не появился на старте");
        assert!(h.is_animating(ids[0]), "аниматор не попал в реестр анимаций");
        std::thread::sleep(Duration::from_millis(150));
        for _ in 0..8 {
            assert!(h.animate(Duration::from_millis(16)), "кадр не запрошен посреди отсчёта");
        }
        assert!(sw.live_ms.get_untracked() >= 100, "показание не двигалось");

        sw.finish();
        h.rebuild();
        assert!(h.find_by_type_name("ProgressAnimator").is_empty(), "аниматор пережил финиш");
        assert!(!h.animate(Duration::from_millis(16)), "после финиша кадров быть не должно");
    }
}
