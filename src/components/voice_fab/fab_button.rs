//! Круглая FAB-кнопка: по умолчанию в правом нижнем углу, зажав мышь, её
//! можно перетащить куда угодно.
//!
//! Живёт отдельным слоем верхнего `Stack` оболочки: `Column` на всё окно,
//! прижимающий ребёнка к правому-нижнему углу, а инлайн-`padding` — отступы
//! кнопки от этого угла (`VoiceFabCtx.fab_margin`). Отступы от угла, а не
//! координаты: при смене размера окна и maximize кнопка остаётся у своего
//! края. `Column` прозрачен для hit-test — слой не крадёт клики у страницы.
//! Раньше кнопку ставил `Portal::BottomEnd`; у Portal якорь фиксирован, и она
//! перекрывала то, что страница кладёт в тот же угол (кнопку «Пауза» панели
//! загрузок HuggingFace).
//!
//! Перенос ведёт [`DragHandle`]: нажатие без движения — щелчок (открыть окно
//! распознавания), с движением — кнопка едет за курсором. Позиция пишется в
//! конфиг по отпусканию (`fab_margin_saved`), а не на каждое смещение.
//!
//! Реактивный класс самой кнопки переключается между:
//! - `.fab-voice-corner` — idle (с keyframe-пульсацией shadow);
//! - `.fab-voice-corner opening` — окно распознавания открыто (scale-out + fade
//!   в пользу центральной FAB-кнопки в `.voice-overlay-card`);
//! - `.fab-voice-corner recording` — запись идёт (когда панель закрыта, такого
//!   состояния практически не бывает — мы открываем панель сразу).

use syngui::core::{Point, Rect};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::signal::use_signal;
use syngui::widget::styled::{StyledWidget, WidgetExt};

use crate::components::drag_handle::{DragHandle, DragPhase};
use crate::context::AppCtx;
use crate::icons::MI_MIC;

/// Отступы по умолчанию: (справа, снизу).
pub const FAB_DEFAULT_MARGIN: (f32, f32) = (16.0, 16.0);
/// Ближе к краю окна кнопку не подвести.
const FAB_EDGE: f32 = 4.0;

/// Новые отступы от правого-нижнего угла: `start` — отступы на момент нажатия,
/// `bounds` — границы кнопки тогда же, `delta` — смещение курсора. Кнопка не
/// выходит за окно: влево/вверх она может уйти не дальше своего расстояния до
/// левого/верхнего края, вправо/вниз — до `FAB_EDGE`.
pub fn dragged_margin(start: (f32, f32), bounds: Rect, delta: Point) -> (f32, f32) {
    let axis = |start: f32, origin: f32, delta: f32| {
        let max = (start + origin - FAB_EDGE).max(FAB_EDGE);
        (start - delta).clamp(FAB_EDGE, max)
    };
    (axis(start.0, bounds.origin.x, delta.x), axis(start.1, bounds.origin.y, delta.y))
}

pub fn view() -> impl Widget {
    let voice = use_context::<AppCtx>().voice;
    layer(voice.fab_margin, voice.fab_margin_saved, open_panel_and_record, || {
        Box::new(Stack::new().clip(false).child(reactive_fab()))
    })
}

/// Слой с перетаскиваемой кнопкой. Отдельно от `AppCtx` — чтобы собрать в
/// тесте: `margin` меняется вживую, `saved` — по отпусканию.
pub fn layer(
    margin: RwSignal<(f32, f32)>,
    saved: RwSignal<(f32, f32)>,
    on_click: impl Fn() + Send + Sync + Clone + 'static,
    button: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
) -> impl Widget {
    // Отступы на момент нажатия. Сигнал, а не поле замыкания: слой
    // пересобирается на каждое смещение, и замыкание каждый раз новое.
    let drag_origin = use_signal(None::<(f32, f32)>);
    // `Stack` — лишь носитель реактивного замыкания; как и `Column`, прозрачен
    // для hit-test.
    Stack::new().clip(false).child(move || {
        let (right, bottom) = margin.get();
        let handle = DragHandle::new(Stack::new().clip(false).children(vec![button()]))
            .on_click(on_click.clone())
            .on_drag(move |phase| match phase {
                DragPhase::Move { bounds, delta } => {
                    let start = drag_origin.get_untracked().unwrap_or_else(|| {
                        let now = margin.get_untracked();
                        drag_origin.set(Some(now));
                        now
                    });
                    margin.set(dragged_margin(start, bounds, delta));
                }
                DragPhase::End => {
                    drag_origin.set(None);
                    saved.set(margin.get_untracked());
                }
            });
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::End)
            .child(handle)
            // Сначала `style`, потом `class`: у `Column` есть свой `class()`, и
            // в обратном порядке класс остался бы внутри `Column`, а обёртка
            // `StyledWidget` при первом же обновлении сбросила бы его пустым
            // списком — слой схлопывался до кнопки в левом верхнем углу.
            .style("padding-right", StyleValue::px(right))
            .style("padding-bottom", StyleValue::px(bottom))
            .class("fab-voice-layer")
    })
}

/// Реактивный wrapper: переключает MSS-класс по состоянию voice.panel_open
/// и audio.is_recording. Tap → открыть окно распознавания + старт записи.
fn reactive_fab() -> impl Fn() -> StyledWidget<ToolButton> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let panel_open = app.voice.panel_open.get();
        let recording = app.audio.is_recording.get();

        let class = if panel_open {
            "fab-voice-corner opening"
        } else if recording {
            "fab-voice-corner recording"
        } else {
            "fab-voice-corner"
        };

        ToolButton::new(MI_MIC)
            .tooltip(tr!("voice.fab.tooltip"))
            .on_click(open_panel_and_record)
            .class(class)
    }
}

/// Открыть окно распознавания и начать запись. Зовётся кликом по FAB и
/// командой «Голосовой ввод» из глобального поиска.
pub fn open_panel_and_record() {
    let app = use_context::<AppCtx>();
    if app.voice.panel_open.get_untracked() {
        return;
    }
    app.voice.accumulated.set(String::new());
    app.voice.last_transcript.set(String::new());
    app.voice.awaiting_actions.set(false);
    app.voice.refined.set(String::new());
    app.voice.refine_error.set(None);
    app.voice.raw_gen.update(|n| *n = n.wrapping_add(1));
    app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
    app.audio.session.error().set(None);
    app.voice.panel_open.set(true);
    // voice_start: если ASR-модель уже загружена — стартует запись сразу;
    // иначе ставит pending_record_start=true и просит загрузить модель.
    // install_voice_auto_record (effect) дёрнет start_recording при успехе.
    crate::agent::audio::voice_start();
}
