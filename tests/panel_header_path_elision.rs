//! Подзаголовок шапки панели обязан оставаться внутри своей колонки.
//!
//! Зачем тест: `measure_row` меряет не-flex детей с `max_width = INFINITY`
//! (`syngui/src/widget/tree/layout/measure/helpers.rs`). Пока колонка
//! «заголовок + подзаголовок» внутри `panel_header::identity` не была
//! flex-элементом, Text считал, что места сколько угодно: `max_lines(1)`
//! и `Elide::Middle` не срабатывали, и длинный путь вида
//! `/home/master/Projects/2027/synthos` рисовался поверх соседней панели.
//! Глазами это ловилось только на узкой панели.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use synthos::components::panel_header;
use synthos::components::workspace_frame::expand;
use syngui::prelude::*;
use syngui::testing::TestHarness;

/// Повторяет обвязку из `workspace_frame::column` + `code_editor::left_header`:
/// шапка колонки — Row, внутри `.grow`-бокс с идентичностью и кнопка справа.
fn header(width: f32, subtitle: &str) -> TestHarness {
    let identity = panel_header::identity_text(
        "\u{e2c7}",
        "synthos".to_string(),
        subtitle.to_string(),
    );
    let row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(expand(identity)))
        .child(DecoratedBox::new().style("width", 32.0_f32).style("height", 32.0_f32));
    let mut harness = TestHarness::new(Box::new(
        DecoratedBox::new().class("panel-header").child(row),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    harness.apply_styles(&engine);
    harness.layout(width, 52.0);
    harness
}

fn widest_text(harness: &TestHarness) -> f32 {
    harness
        .find_by_type_name("Text")
        .into_iter()
        .map(|id| harness.element_bounds(id).size.width)
        .fold(0.0_f32, f32::max)
}

#[test]
fn subtitle_stays_inside_a_narrow_panel() {
    // Панель уже, чем путь: 240px против ~33 символов подзаголовка.
    let harness = header(240.0, "/home/master/Projects/2027/synthos");
    let text_w = widest_text(&harness);
    assert!(
        text_w <= 240.0,
        "подзаголовок вылез за панель: {text_w}px при ширине панели 240px"
    );
}

#[test]
fn subtitle_still_fits_when_panel_is_wide() {
    // На широкой панели ограничение не должно ничего ломать: текст
    // занимает свою натуральную ширину, а не всю панель.
    let harness = header(900.0, "/home/master/Projects/2027/synthos");
    let text_w = widest_text(&harness);
    assert!(
        text_w > 0.0 && text_w < 900.0,
        "ожидали натуральную ширину текста, получили {text_w}px"
    );
}
