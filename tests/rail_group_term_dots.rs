//! Плитка группы рейла показывает статусы терминалов проектов — по точке на
//! проект, а не число плиток.
//!
//! Зачем тест: точки — `DecoratedBox` без содержимого, размер им даёт только
//! MSS; полоска позиционируется по расчётной ширине и должна заканчиваться у
//! правого края кнопки, как числовой бейдж.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widgets::containers::Stack;
use synthos::components::nav_rail::group_term_dots;

#[test]
fn group_tile_shows_dot_per_project() {
    let widget = Stack::new()
        .child(ToolButton::new("\u{e2c7}").class("nav-rail-item nav-rail-group-btn"))
        .child(group_term_dots(&["busy", "idle", "mixed"]));
    let mut h = TestHarness::new(Box::new(widget));
    let engine = h.apply_mss(synthos::styles::styles());
    h.apply_styles(&engine);
    h.layout(200.0, 200.0);

    let btn = h.element_bounds(h.find_by_type_name("ToolButton")[0]);
    let boxes: Vec<_> = h
        .find_by_type_name("DecoratedBox")
        .into_iter()
        .map(|id| h.element_bounds(id))
        .collect();
    let dots: Vec<_> = boxes.iter().filter(|r| (r.size.width - 7.0).abs() < 0.5).collect();
    assert_eq!(dots.len(), 3, "{boxes:?}");
    for d in &dots {
        assert!((d.size.height - 7.0).abs() < 0.5, "{d:?}");
    }
    // Точки в ряд, слева направо, с зазором.
    assert!(dots.windows(2).all(|w| w[1].origin.x >= w[0].origin.x + 8.5), "{dots:?}");
    // Полоска заканчивается у правого края кнопки (как бейдж) и в её верхней части.
    let pill = boxes.iter().max_by(|a, b| a.size.width.total_cmp(&b.size.width)).unwrap();
    let right = pill.origin.x + pill.size.width;
    assert!((right - (btn.origin.x + 42.0)).abs() < 1.0, "{pill:?} {btn:?}");
    assert!(pill.origin.y < btn.origin.y + 12.0, "{pill:?}");
}
