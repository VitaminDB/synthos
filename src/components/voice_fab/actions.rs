//! Реактивный footer-row окна распознавания.
//!
//! Состояния:
//! - `is_recording=true`           → [Pause] [Stop]
//! - `transcribing=true`           → [Hourglass disabled]
//! - `awaiting_actions=true`       → [Copy] [Paste] [Restart]
//! - иначе (после Pause)           → [Mic «Записать»] [Stop «Завершить»]
//!
//! Закрытие панели — клик по backdrop'у Portal'a (`Portal::on_close`) и ESC.
//! Кнопки «Закрыть» в actions_row нет — UI минималистичный.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;

use crate::agent;
use crate::context::{AppCtx, FocusTarget};
use crate::icons::{
    MI_AUTORENEW, MI_CONTENT_COPY, MI_CONTENT_PASTE, MI_HOURGLASS_TOP, MI_MIC, MI_PAUSE, MI_STOP,
};

/// Реактивный actions_row. mgui! автоматически оборачивает Fn-замыкание в Reactive.
pub fn view() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let recording = app.audio.is_recording.get();
        let transcribing = app.audio.transcribing.get();
        let awaiting = app.voice.awaiting_actions.get();

        let inner = if transcribing {
            mgui! {
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Center) => [
                        ToolButton::new(MI_HOURGLASS_TOP)
                            .tooltip("Распознавание…")
                            .on_click(|| {})
                            .class("voice-action-busy")
                    ]
            }
        } else if recording {
            mgui! {
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Center) => [
                        action_button(MI_PAUSE, "Пауза", "voice-action-pause", on_pause),
                        action_button(MI_STOP,  "Стоп", "voice-action-stop",  on_stop)
                    ]
            }
        } else if awaiting {
            mgui! {
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Center) => [
                        action_button(MI_CONTENT_COPY,  "Копировать",  "voice-action-copy",   on_copy),
                        action_button(MI_CONTENT_PASTE, "Вставить",    "voice-action-paste",  on_paste),
                        action_button(MI_AUTORENEW,     "Заново",      "voice-action-restart", on_restart)
                    ]
            }
        } else {
            mgui! {
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Center) => [
                        action_button(MI_MIC,  "Записать", "voice-action-resume", on_resume),
                        action_button(MI_STOP, "Завершить", "voice-action-stop", on_finish_after_pause)
                    ]
            }
        };

        DecoratedBox::new().class("voice-actions-row").child(inner)
    }
}

fn action_button(
    icon: &'static str,
    tooltip: &'static str,
    class: &'static str,
    handler: fn(),
) -> StyledWidget<ToolButton> {
    ToolButton::new(icon)
        .tooltip(tooltip)
        .on_click(handler)
        .class(class)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

fn on_pause() {
    agent::audio::voice_pause();
}

fn on_stop() {
    agent::audio::voice_stop();
}

fn on_resume() {
    let app = use_context::<AppCtx>();
    app.voice.awaiting_actions.set(false);
    agent::audio::voice_resume();
}

/// «Завершить» из состояния паузы (запись не идёт, текст уже накоплен).
/// Не пытаемся stop'ать (нечего стопать) — просто переключаем UI на actions.
fn on_finish_after_pause() {
    let app = use_context::<AppCtx>();
    if !app.voice.accumulated.get_untracked().trim().is_empty() {
        app.voice.awaiting_actions.set(true);
        // Sprint 2: сохранить текущую сессию даже без новой записи —
        // если был хотя бы один Pause-чанк. Для простоты делегируем в
        // saving-функцию с пустым WAV (она проверит и пропустит).
        // НЕ делаем здесь, чтобы избежать дубль-сохранения; запись сохраняется
        // только в `voice_stop` при final_chunk=true (см. on_transcription_done).
    } else {
        // Накопленного текста нет — закрываем панель.
        close_panel();
    }
}

fn on_copy() {
    let app = use_context::<AppCtx>();
    let text = effective_text(&app);
    if text.trim().is_empty() {
        return;
    }
    syngui::clipboard::copy(&text);
}

fn on_paste() {
    let app = use_context::<AppCtx>();
    let text = effective_text(&app);
    if text.trim().is_empty() {
        return;
    }
    paste_to_target(&app, &text);
    close_panel();
}

/// Что вставить/скопировать: refined (если он непустой и сейчас не идёт
/// постобработка) с fallback на raw. Refined может быть пустым, если
/// пользователь ни разу не нажал кнопку постобработки или модель вернула
/// пустой ответ — в этом случае берём исходную стенограмму.
fn effective_text(app: &AppCtx) -> String {
    if !app.voice.refining.get_untracked() {
        let r = app.voice.refined.get_untracked();
        if !r.trim().is_empty() {
            return r;
        }
    }
    app.voice.accumulated.get_untracked()
}

fn on_restart() {
    let app = use_context::<AppCtx>();
    app.voice.accumulated.set(String::new());
    app.voice.last_transcript.set(String::new());
    app.voice.awaiting_actions.set(false);
    app.voice.refined.set(String::new());
    app.voice.refine_error.set(None);
    app.voice.raw_gen.update(|n| *n = n.wrapping_add(1));
    app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
    agent::audio::voice_start();
}

/// Закрыть панель: остановить запись если идёт, очистить state, спрятать UI.
pub fn close_panel() {
    let app = use_context::<AppCtx>();
    if app.audio.is_recording.get_untracked() {
        // Аккуратно stop — сохранит запись как final_chunk и закроет рекордер.
        agent::audio::voice_stop();
    }
    app.voice.panel_open.set(false);
    app.voice.awaiting_actions.set(false);
    app.voice.accumulated.set(String::new());
    app.voice.last_transcript.set(String::new());
    app.voice.refined.set(String::new());
    app.voice.refine_error.set(None);
    app.voice.raw_gen.update(|n| *n = n.wrapping_add(1));
    app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
    // Если ждали авто-загрузку модели — отменяем; сама загрузка не отменяется
    // (она в spawn_blocking), но запись по её завершении не стартует.
    app.voice.pending_record_start.set(false);
}

// ─────────────────────────────────────────────────────────────────────────────
// Paste routing
// ─────────────────────────────────────────────────────────────────────────────

/// Вставить распознанный текст в активный целевой виджет.
///
/// Логика (эвристика для MVP — честный focus-tracking отложен в future-work):
/// - явный `FocusTarget::Terminal` → пишем в pty stdin (если доступно);
/// - страница `code` + есть активный терминал → пишем в pty stdin (типичный
///   сценарий: пользователь на code-editor странице, voice-ввод имеет смысл
///   именно для терминала);
/// - иначе → дописываем в `chat.input` (через пробел) + bump `input_gen`,
///   чтобы MultilineTextEdit пересобрал поле с новым содержимым.
pub fn paste_to_target(app: &AppCtx, text: &str) {
    let target = app.voice.focus_target.get_untracked();
    let route = app.current_route.get_untracked();

    let prefer_terminal = matches!(target, FocusTarget::Terminal)
        || (route == "code" && !matches!(target, FocusTarget::ChatInput));

    if prefer_terminal && crate::pages::code_editor::state::write_to_active_terminal(text) {
        return;
    }
    paste_to_chat_input(app, text);
}

fn paste_to_chat_input(_app: &AppCtx, text: &str) {
    let syn = use_context::<crate::syn_chat::state::SynChatCtx>();
    let prev = syn.input.get_untracked();
    let merged = if prev.trim().is_empty() {
        text.to_string()
    } else {
        format!("{} {}", prev.trim_end(), text)
    };
    syn.input.set(merged);
    syn.input_gen.update(|n| *n = n.wrapping_add(1));
}
