//! Центральная круглая FAB-кнопка (96×96) внутри окна распознавания.
//!
//! Визуально — крупный «брат» угловой FAB. Появляется по центру панели,
//! окружена аурой (см. `aura.rs`). Click — toggle pause/resume; во время
//! `awaiting_actions` — clickable рестарт.
//!
//! Размер и анимация (transform: scale + opacity) полностью на MSS:
//! `.fab-voice-center` / `.fab-voice-center.opening`. `panel.rs::view` ставит
//! класс `opening` через реактивное замыкание.

use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;

use crate::chat;
use crate::context::AppCtx;
use crate::icons::MI_MIC;

pub fn view() -> impl Fn() -> StyledWidget<ToolButton> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let panel_open = app.voice.panel_open.get();
        let recording = app.audio.is_recording.get();

        // Стартовое (закрытое) состояние — scale(0.6); opacity:0. Когда
        // panel_open=true, MSS-class .opening переключает на scale(1); opacity:1
        // через transition. Backdrop FAB-окна одновременно становится видимым,
        // визуально — «кнопка прилетела из угла + увеличилась».
        let class = match (panel_open, recording) {
            (true, true) => "fab-voice-center opening recording",
            (true, false) => "fab-voice-center opening",
            (false, _) => "fab-voice-center",
        };

        ToolButton::new(MI_MIC)
            .tooltip("Пауза / Продолжить")
            .on_click(toggle_central)
            .class(class)
    }
}

/// Click по центральной кнопке: если идёт запись — Pause; если на паузе —
/// Resume; если awaiting_actions — Restart (как «начать заново»).
fn toggle_central() {
    let app = use_context::<AppCtx>();
    let recording = app.audio.is_recording.get_untracked();
    let awaiting = app.voice.awaiting_actions.get_untracked();

    if recording {
        chat::audio::voice_pause();
    } else if awaiting {
        app.voice.accumulated.set(String::new());
        app.voice.last_transcript.set(String::new());
        app.voice.awaiting_actions.set(false);
        chat::audio::voice_start();
    } else {
        chat::audio::voice_resume();
    }
}
