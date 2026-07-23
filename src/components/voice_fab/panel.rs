//! Overlay-окно распознавания: появляется в центре поверх любой страницы.
//!
//! Открывается реактивно по `voice.panel_open`. Структура (без обёртки-карточки
//! с заголовком — UI минималистичный, закрытие через ESC и клик по backdrop'у):
//! - центральный блок «аура + крупная FAB-кнопка» (Stack);
//! - реактивный текст статуса («Слушаю…» / «Распознаю…» / «Готово»);
//! - реактивная пара текстовых полей: слева raw, справа refined, между ними
//!   крупная Refine-кнопка (см. `transcripts_view`);
//! - реактивный actions_row (Pause/Stop / Mic-Resume / Copy-Paste-Restart).

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::overlay::portal::{Portal, PortalAnchor};
use syngui::widgets::{MultilineTextEdit, Stack};

use crate::agent;
use crate::context::AppCtx;
use crate::icons::{MI_AUTORENEW, MI_HOURGLASS_TOP};

use super::actions;
use super::aura;
use super::central_fab;

pub fn view() -> impl Widget {
    let app = use_context::<AppCtx>();
    let panel_open = app.voice.panel_open;

    // Backdrop почти прозрачный — основное «затемнение» даёт frosted-glass blur
    // самой карточки (`backdrop-filter: blur(...)` в MSS).
    Portal::new()
        .is_open(panel_open)
        .anchor(PortalAnchor::Center)
        .modal(true)
        .backdrop(true)
        .backdrop_color(Color::new(0.0, 0.0, 0.0, 0.10))
        .on_close(actions::close_panel)
        .child(card())
}

/// Внутренняя карточка панели: централизованная Column через явный builder
/// (`.child(closure)`) — не через `mgui!{ Column => [...] }`, потому что в
/// массиве макрос терял реактивные closures (Fn() -> StyledWidget<T>) и они
/// не отрисовывались. Все размеры — в MSS `.voice-overlay-card`.
fn card() -> impl Widget {
    let column = Column::new()
        .gap(20.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(aura_with_central_fab())
        .child(status_text_reactive())
        .child(transcripts_view())
        .child(actions::view());

    DecoratedBox::new()
        .class("voice-overlay-card")
        .child(column)
}

/// Stack из ауры (Canvas 480×480) и центральной FAB-кнопки (96×96).
/// Оба child'а ужe центрированы относительно своих bounds: Canvas через
/// (cx,cy)=(W/2,H/2), а Stack кладёт child'ов в один и тот же origin.
/// Stack::child принимает IntoWidget — автоматически оборачивает реактивный
/// closure из `aura::view()` в `Reactive`.
fn aura_with_central_fab() -> impl Widget {
    DecoratedBox::new()
        .class("voice-aura-wrap")
        .clip(false)
        .child(
            Stack::new()
                .clip(false)
                .child(aura::view())
                .child(central_fab::view()),
        )
}

/// Реактивный статус. Приоритет проверок:
/// 1. ASR-модель грузится (после клика FAB при выгруженной модели) → «Загружаю модель…»
/// 2. Идёт транскрипция → «Распознаю…»
/// 3. Идёт запись → «Слушаю…»
/// 4. Финальный Stop отработан → «Готово — выберите действие»
/// 5. Иначе → «На паузе…» либо текст ошибки.
fn status_text_reactive() -> impl Fn() -> StyledWidget<Text> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let recording = app.audio.is_recording.get();
        let transcribing = app.audio.transcribing.get();
        let awaiting = app.voice.awaiting_actions.get();
        let asr_loading = app.audio.asr_loading.get();
        let error = app.audio.error.get();

        let label = if let Some(err) = error.as_deref() {
            err.to_string()
        } else if asr_loading {
            "Загружаю модель…".to_string()
        } else if transcribing {
            "Распознаю…".to_string()
        } else if recording {
            "Слушаю…".to_string()
        } else if awaiting {
            "Готово — выберите действие".to_string()
        } else {
            "На паузе. Нажмите «Записать», чтобы продолжить".to_string()
        };
        Text::new(label).class("voice-overlay-status")
    }
}

/// Горизонтальная компоновка двух полей со средней Refine-кнопкой:
/// `[ raw_section | refine_button | refined_section ]`.
///
/// Каждая колонка получает `.grow` (flex-grow: 1) — поля делят ширину поровну.
/// Refine-кнопка центрируется по вертикали через `cross_axis_alignment(Center)`
/// на самом Row.
fn transcripts_view() -> impl Widget {
    DecoratedBox::new().class("voice-overlay-transcripts").child(
        Row::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(raw_section())
            .child(refine_button_reactive())
            .child(refined_section()),
    )
}

fn raw_section() -> impl Widget {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(Text::new("Исходная речь").class("voice-overlay-section-title"))
        .child(raw_field_reactive())
}

fn raw_field_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        // Подписка на «поколение» поля — пересборка при внешнем обновлении.
        let _ = app.voice.raw_gen.get();
        let accumulated = app.voice.accumulated;
        let initial = accumulated.get_untracked();

        DecoratedBox::new().class("voice-overlay-text-raw").child(
            MultilineTextEdit::new()
                .text(initial)
                .placeholder("Здесь появится распознанная речь…")
                .rows(2)
                .max_rows(4)
                .soft_wrap(true)
                .auto_height(true)
                .on_change(move |s| {
                    accumulated.set(s.to_string());
                })
                .class("voice-overlay-text"),
        )
    }
}

fn refined_section() -> impl Widget {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(Text::new("Отредактированный текст").class("voice-overlay-section-title"))
        .child(refined_field_reactive())
        .child(refine_error_reactive())
}

/// Центральная Refine-кнопка между raw и refined колонками.
/// Иконка переключается на «песочные часы» во время постобработки;
/// кнопка disabled при пустом raw или активном refine.
fn refine_button_reactive() -> impl Fn() -> StyledWidget<ToolButton> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let refining = app.voice.refining.get();
        let raw_empty = app.voice.accumulated.get().trim().is_empty();
        let icon = if refining { MI_HOURGLASS_TOP } else { MI_AUTORENEW };
        let tooltip = if refining {
            "Постобработка…"
        } else {
            "Прогнать через модель"
        };
        let disabled = refining || raw_empty;

        ToolButton::new(icon)
            .tooltip(tooltip)
            .disabled(disabled)
            .on_click(trigger_refine)
            .class("voice-action-refine")
    }
}

fn refined_field_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let _ = app.voice.refined_gen.get();
        let refined = app.voice.refined;
        let initial = refined.get_untracked();

        DecoratedBox::new().class("voice-overlay-text-refined").child(
            MultilineTextEdit::new()
                .text(initial)
                .placeholder("Нажмите ⟲ чтобы пропустить через модель")
                .rows(2)
                .max_rows(4)
                .soft_wrap(true)
                .auto_height(true)
                .on_change(move |s| {
                    refined.set(s.to_string());
                })
                .class("voice-overlay-text"),
        )
    }
}

/// Реактивная строка ошибки postprocess'а — видна только при `Some(_)`.
fn refine_error_reactive() -> impl Fn() -> StyledWidget<Text> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let err = app.voice.refine_error.get();
        let label = err.unwrap_or_default();
        Text::new(label).class("voice-overlay-refine-error")
    }
}

fn trigger_refine() {
    let app = use_context::<AppCtx>();
    let raw = app.voice.accumulated.get_untracked();
    agent::voice_refine::refine_voice_text(app, raw);
}
