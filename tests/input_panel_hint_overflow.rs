//! Строка ошибки в панели ввода не имеет права выталкивать кнопки за край.
//!
//! Зачем тест: `measure_row` меряет не-flex детей с `max_width = INFINITY`
//! (`syngui/src/widget/tree/layout/measure/helpers.rs`). Левая группа
//! тулбара (скрепка + hint) не была flex-элементом, поэтому длинный текст
//! вида «Остановлено: инструмент `bash` вызван с теми же аргументами…»
//! мерился в одну бесконечную строку и уезжал вправо вместе с правой
//! группой: «Продолжить», regen и «Отправить» оказывались за границей
//! панели, и нажать их было нельзя — ровно в тот момент, когда кнопка
//! «Продолжить» и нужна.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

/// Текст, на котором баг воспроизводился: сообщение guard'а повторов
/// плюс хвост `chat.session.error.stopped_suffix`.
const LONG_ERROR: &str = "Ошибка: Остановлено: инструмент `bash` вызван с теми \
же аргументами 3-й раз подряд — агент ходит по кругу. Генерация остановлена, \
история цела. Уточните задачу или нажмите «Продолжить».";

/// Повторяет нижний ряд `input_panel::panel_body`: слева скрепка + hint,
/// справа — счётчик токенов, «Продолжить», regen и «Отправить».
fn toolbar(width: f32, hint: &str) -> TestHarness {
    let left = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("input-toolbar-left")
        .child(
            DecoratedBox::new()
                .class("input-attach-wrap")
                .style("width", 32.0_f32)
                .style("height", 32.0_f32),
        )
        .child(
            DecoratedBox::new()
                .class("input-hint error")
                .child(Text::new(hint).max_lines(3).class("input-hint-text")),
        );
    let right = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("input-regen-wrap")
                .style("width", 32.0_f32)
                .style("height", 32.0_f32),
        )
        .child(
            DecoratedBox::new()
                .class("input-send-wrap")
                .style("width", 40.0_f32)
                .style("height", 40.0_f32),
        );
    let row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .child(left)
        .child(right);
    let mut harness = TestHarness::new(Box::new(
        DecoratedBox::new().class("input-panel").child(row),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    harness.apply_styles(&engine);
    harness.layout(width, 80.0);
    harness
}

/// Правый край элемента с данным классом.
fn right_edge(harness: &TestHarness, class: &str) -> f32 {
    let ids = harness.find_by_class(class);
    assert!(!ids.is_empty(), "не нашли элемент .{class}");
    let b = harness.element_bounds(ids[0]);
    b.origin.x + b.size.width
}

#[test]
fn send_button_stays_inside_the_panel_on_a_long_error() {
    let width = 900.0;
    let harness = toolbar(width, LONG_ERROR);
    let send = right_edge(&harness, "input-send-wrap");
    assert!(
        send <= width,
        "кнопка «Отправить» уехала за панель: правый край {send}px при ширине {width}px"
    );
    let regen = right_edge(&harness, "input-regen-wrap");
    assert!(
        regen <= width,
        "кнопка «Продолжить»/regen уехала за панель: правый край {regen}px при ширине {width}px"
    );
}

#[test]
fn hint_text_wraps_instead_of_growing_sideways() {
    let width = 900.0;
    let harness = toolbar(width, LONG_ERROR);
    let text_w = harness
        .find_by_type_name("Text")
        .into_iter()
        .map(|id| harness.element_bounds(id).size.width)
        .fold(0.0_f32, f32::max);
    assert!(
        text_w > 0.0 && text_w <= width,
        "текст подсказки шире панели: {text_w}px при ширине {width}px"
    );
}

#[test]
fn buttons_stay_put_on_a_narrow_panel() {
    // Узкая панель — тот же контракт: кнопки внутри, текст ужимается.
    let width = 420.0;
    let harness = toolbar(width, LONG_ERROR);
    let send = right_edge(&harness, "input-send-wrap");
    assert!(
        send <= width,
        "на узкой панели «Отправить» вылезла: правый край {send}px при ширине {width}px"
    );
}
