//! Плитки в панели группы рейла — столбиком и с подписями.
//!
//! Зачем тест: в попапе `Text` подписи получал всю ширину ограничения
//! (`max-width` из MSS не срабатывал), центрированный текст уезжал за
//! обрезку узкой панели — плитки были без подписей.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::core::{Point, Rect, Size};
use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widgets::containers::Stack;
use syngui::widgets::{PopupAnchor, PopupPanel};

fn tile(label: &str) -> impl Widget {
    Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(ToolButton::new("\u{e2c7}").class("nav-rail-item in-flyout"))
        .child(Text::new(label.to_string()).max_lines(1).class("nav-rail-session-label in-flyout"))
}

#[test]
fn flyout_tiles_have_labels() {
    let open = use_signal(false);
    let anchor = use_signal(Rect::new(Point::new(0.0, 200.0), Size::new(72.0, 60.0)));
    let col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(tile("synthos"))
        .child(tile("syngui"));
    let widget = Stack::new().child(
        PopupPanel::new()
            .is_open(open)
            .anchor_rect(anchor)
            .anchor(PopupAnchor::EndCenter)
            .min_width(80.0)
            .max_width(80.0)
            .reveal(AnimationAxis::Width)
            .class("nav-rail-flyout")
            .child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("SYNTHOS").max_lines(1).class("nav-rail-flyout-title"))
                    .child(col),
            ),
    );
    let mut h = TestHarness::new(Box::new(widget));
    let engine = h.apply_mss(synthos::styles::styles());
    h.apply_styles(&engine);
    h.layout(1000.0, 800.0);
    open.set(true);
    h.layout(1000.0, 800.0);
    for _ in 0..60 {
        h.animate(std::time::Duration::from_millis(16));
        h.layout(1000.0, 800.0);
    }
    // Панель стоит справа от рейла: x ∈ [72, 152].
    let (lo, hi) = (72.0, 152.0);
    let btns: Vec<_> = h.find_by_type_name("ToolButton").into_iter().map(|id| h.element_bounds(id)).collect();
    let texts: Vec<_> = h.find_by_type_name("Text").into_iter().map(|id| h.element_bounds(id)).collect();
    // Столбиком: плитки одна под другой.
    assert_eq!(btns.len(), 2);
    assert!((btns[0].origin.x - btns[1].origin.x).abs() < 0.5, "{btns:?}");
    assert!(btns[1].origin.y > btns[0].origin.y + 40.0, "{btns:?}");
    // Подписи (и заголовок) — внутри панели, а не за её обрезкой.
    for r in texts.iter().chain(btns.iter()) {
        assert!(r.size.width > 10.0 && r.origin.x >= lo - 0.5 && r.origin.x + r.size.width <= hi + 0.5, "{r:?}");
    }
    // Подпись под своей кнопкой.
    for (b, t) in btns.iter().zip(texts.iter().skip(1)) {
        assert!(t.origin.y >= b.origin.y + b.size.height, "{b:?} {t:?}");
    }
}
