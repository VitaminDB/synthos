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
use syngui::widgets::Reactive;

use crate::agent;
use crate::context::AppCtx;
use crate::icons::MI_MIC;

/// FAB, отцентрованный в квадрате размера ауры.
///
/// `Stack` кладёт всех child'ов в один origin — то есть в левый верхний
/// угол, а не в центр: аура рисует себя от (W/2,H/2) внутри Canvas'а и
/// выглядела центрированной, а кнопка садилась в угол ауры.
///
/// Слот `.voice-fab-slot` имеет явные размеры, равные `.voice-aura-wrap`.
/// Без него центрирование пришлось бы вытягивать через
/// `StackFit::Expand`, а тот раздувает Stack до максимума входящих
/// constraint'ов — карточка растягивалась на весь экран. Явный квадрат
/// даёт `Center` определённые bounds независимо от fit'а Stack'а.
pub fn centered() -> impl Widget {
    DecoratedBox::new()
        .class("voice-fab-slot")
        .clip(false)
        .child(Center::new().child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            vec![Box::new(build())]
        })))
}

fn build() -> StyledWidget<ToolButton> {
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
        .tooltip(tr!("voice.fab.center.tooltip"))
        .on_click(toggle_central)
        .class(class)
}

/// Click по центральной кнопке: если идёт запись — Pause; если на паузе —
/// Resume; если awaiting_actions — Restart (как «начать заново»).
fn toggle_central() {
    let app = use_context::<AppCtx>();
    let recording = app.audio.is_recording.get_untracked();
    let awaiting = app.voice.awaiting_actions.get_untracked();

    if recording {
        agent::audio::voice_pause();
    } else if awaiting {
        app.voice.accumulated.set(String::new());
        app.voice.last_transcript.set(String::new());
        app.voice.awaiting_actions.set(false);
        agent::audio::voice_start();
    } else {
        agent::audio::voice_resume();
    }
}
