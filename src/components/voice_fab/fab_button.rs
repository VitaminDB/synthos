//! Круглая FAB-кнопка в правом нижнем углу.
//!
//! Реализована через `Portal::BottomEnd { 24, 24 }` с `is_open=always_true`,
//! `modal=false`, `backdrop=false` — Portal только позиционирует виджет в
//! viewport-координатах, не давая родительскому layout'у влиять на её
//! положение. Сама кнопка — `ToolButton(MI_MIC)` с MSS-классом `.fab-voice`.
//!
//! Реактивный класс переключается между:
//! - `.fab-voice-corner` — idle (с keyframe-пульсацией shadow);
//! - `.fab-voice-corner opening` — окно распознавания открыто (scale-out + fade
//!   в пользу центральной FAB-кнопки в `.voice-overlay-card`);
//! - `.fab-voice-corner recording` — запись идёт (когда панель закрыта, такого
//!   состояния практически не бывает — мы открываем панель сразу).

use syngui::prelude::*;
use syngui::signal::use_signal;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::overlay::portal::{Portal, PortalAnchor};

use crate::context::AppCtx;
use crate::icons::MI_MIC;

pub fn view() -> impl Widget {
    // FAB всегда виден — отдельный сигнал, который никто извне не меняет.
    let always_open = use_signal(true);

    Portal::new()
        .is_open(always_open)
        .anchor(PortalAnchor::BottomEnd {
            margin_bottom: 16.0,
            margin_right: 16.0,
        })
        .modal(false)
        .backdrop(false)
        .child(reactive_fab())
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
            .tooltip("Голосовой ввод")
            .on_click(open_panel_and_record)
            .class(class)
    }
}

fn open_panel_and_record() {
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
    crate::chat::audio::voice_start();
}
