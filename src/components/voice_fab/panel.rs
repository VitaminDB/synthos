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
use syngui::widgets::{MultilineTextEdit, Stack, StackFit};

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
///
/// Canvas ауры рисует себя от (cx,cy)=(W/2,H/2) и потому выглядит
/// центрированным сам по себе, а вот Stack кладёт child'ов в общий origin —
/// левый верхний угол. Кнопка из-за этого садилась в угол ауры, поэтому
/// она приходит уже завёрнутой в `Center` (`central_fab::centered`), а
/// Stack растягивает обоих на полные bounds обёртки (`StackFit::Expand`).
/// Stack::child принимает IntoWidget — реактивный closure из `aura::view()`
/// оборачивается в `Reactive` автоматически.
fn aura_with_central_fab() -> impl Widget {
    DecoratedBox::new()
        .class("voice-aura-wrap")
        .clip(false)
        .child(
            Stack::new()
                .fit(StackFit::Expand)
                .clip(false)
                .child(aura::view())
                .child(central_fab::centered()),
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

/// Вертикальная компоновка секций с Refine-кнопкой между ними:
/// `raw_section / refine_button / refined_section / error`.
///
/// Раньше секции стояли рядом в Row — карточка из-за этого была широкой и
/// приплюснутой. В колонку они складываются в ту же логику «сверху что
/// распознали, снизу что вернула модель», но окно выходит вертикальным, а
/// поля получают всю ширину карточки вместо половины.
fn transcripts_view() -> impl Widget {
    // Строка ошибки postprocess'а живёт ПОД секциями, а не внутри refined:
    // она всегда смонтирована (пустой Text при отсутствии ошибки), и
    // четвёртым child'ом `refined_section` делала правую колонку выше
    // левой — с `cross_axis_alignment(Center)` заголовки и поля из-за
    // этого разъезжались по вертикали.
    DecoratedBox::new().class("voice-overlay-transcripts").child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(raw_section())
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .child(refine_button_reactive()),
            )
            .child(refined_section())
            .child(refine_error_reactive()),
    )
}

fn raw_section() -> impl Widget {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
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
    // Структура обязана совпадать с `raw_section` (заголовок + поле) —
    // иначе колонки разной высоты разъезжаются по вертикали.
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new("Отредактированный текст").class("voice-overlay-section-title"))
        .child(refined_field_reactive())
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
