//! Чипы Donate / GitHub в шапке окна (`components::titlebar::link_chip`).
//!
//! Зачем тест: чипы живут внутри `WindowDragRegion`, который по нажатию
//! начинает перетаскивать окно. Перестань чип брать нажатие сам — щелчок
//! тащил бы окно вместо открытия ссылки, и в сборке этого не видно. Цвет
//! подписи и логотипа при наведении держится на классе `titlebar-chip--hover`
//! от `on_hover_change`: `:hover` у потомка считается по его границам, а не
//! по чипу, и такая ошибка тоже тихая.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use syngui::input::Event;
use syngui::prelude::*;
use syngui::testing::*;
use syngui::widgets::overlay::WindowDragRegion;

use synthos::components::titlebar::{self, ChipIcon};
use synthos::icons::MI_FAVORITE;

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    syngui::signal::drain_and_run_effects();
    h.rebuild();
    h.apply_styles(engine);
    h.layout(900.0, 32.0);
}

fn center(h: &TestHarness, id: ElementId) -> Point {
    let b = h.element_bounds(id);
    Point::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0)
}

#[test]
fn chips_open_links_without_dragging_window_and_highlight_on_hover() {
    syngui::signal::allow_signal_reads_on_this_thread();
    assert!(titlebar::GITHUB_URL.starts_with("https://github.com/"), "{}", titlebar::GITHUB_URL);
    assert!(titlebar::DONATE_URL.starts_with("https://paypal.me/"), "{}", titlebar::DONATE_URL);

    let github = Arc::new(AtomicUsize::new(0));
    let donate = Arc::new(AtomicUsize::new(0));
    let (g, d) = (github.clone(), donate.clone());
    let row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(titlebar::link_chip(ChipIcon::Glyph(MI_FAVORITE), "donate", "Donate".into(), "tip".into(), move || {
            d.fetch_add(1, Ordering::SeqCst);
        }))
        .child(titlebar::link_chip(ChipIcon::GithubMark, "github", "GitHub".into(), "tip".into(), move || {
            g.fetch_add(1, Ordering::SeqCst);
        }))
        .child(DecoratedBox::new().class("grow"));
    let mut h = TestHarness::new(Box::new(
        WindowDragRegion::new().child(DecoratedBox::new().class("titlebar").child(row)),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);

    // ── Вид: два чипа без рамки, у GitHub — логотип ───────────────────
    let chips = h.find_by_class("titlebar-chip");
    assert_eq!(chips.len(), 2, "два чипа");
    for &chip in &chips {
        let size = h.element_bounds(chip).size;
        assert_eq!(size.height, 20.0, "высота чипа");
        let border = h.element_mss(chip).and_then(|m| m.border_width);
        assert!(border.is_none_or(|w| w == 0.0), "у чипа рамка: {border:?}");
        assert!(size.width > 30.0, "чип схлопнут: {size:?}");
    }
    let logo = h.find_by_class("titlebar-chip-logo");
    assert_eq!(logo.len(), 1, "логотип GitHub");
    let tint_idle = h.element_mss(logo[0]).and_then(|m| m.color_tint);
    assert!(tint_idle.is_some(), "логотип без color-tint — был бы белым");

    // ── Щелчок по чипу — ссылка, а не перетаскивание окна ─────────────
    let github_chip = chips[1];
    h.send_events(&click_at(center(&h, github_chip)));
    assert_eq!(github.load(Ordering::SeqCst), 1, "щелчок по GitHub");
    assert_eq!(donate.load(Ordering::SeqCst), 0, "Donate не нажимали");
    assert!(!h.tree.window_drag_request, "щелчок по чипу потащил окно");

    // Щелчок выше пилюли, но в полосе титлбара — тоже чип (зона на всю высоту).
    let b = h.element_bounds(chips[0]);
    h.send_events(&click_at(Point::new(b.origin.x + b.size.width / 2.0, 2.0)));
    assert_eq!(donate.load(Ordering::SeqCst), 1, "щелчок над пилюлей Donate");
    assert!(!h.tree.window_drag_request, "щелчок над пилюлей потащил окно");

    // Пустое место титлбара по-прежнему тащит окно.
    h.send_events(&click_at(Point::new(800.0, 16.0)));
    assert!(h.tree.window_drag_request, "пустая полоса перестала тащить окно");
    h.tree.window_drag_request = false;

    // ── Наведение: класс на чипе, логотип темнеет, уход — обратно ─────
    h.send_event(&Event::MouseMove(center(&h, github_chip)));
    settle(&mut h, &engine);
    assert_eq!(h.find_by_class("titlebar-chip--hover").len(), 1, "подсветка наведённого чипа");
    assert_eq!(h.find_by_class("titlebar-chip-github").len(), 1);
    let hovered = h.find_by_class("titlebar-chip--hover")[0];
    assert_eq!(hovered, h.find_by_class("titlebar-chip-github")[0], "подсвечен не тот чип");
    let logo = h.find_by_class("titlebar-chip-logo")[0];
    let tint_hover = h.element_mss(logo).and_then(|m| m.color_tint);
    assert!(tint_hover.is_some() && tint_hover != tint_idle, "логотип не сменил цвет: {tint_idle:?} → {tint_hover:?}");

    h.send_event(&Event::MouseMove(Point::new(800.0, 16.0)));
    settle(&mut h, &engine);
    assert!(h.find_by_class("titlebar-chip--hover").is_empty(), "подсветка осталась после ухода");
    let logo = h.find_by_class("titlebar-chip-logo")[0];
    assert_eq!(h.element_mss(logo).and_then(|m| m.color_tint), tint_idle, "цвет логотипа не вернулся");
}

/// Чипы стоят у правого края шапки — рядом с кнопками окна, а заголовок не
/// сдвигается: при центрированном он ровно посередине, чипы внутри правой
/// распорки.
#[test]
fn chips_sit_at_the_right_edge_and_keep_the_title_centered() {
    syngui::signal::allow_signal_reads_on_this_thread();
    const W: f32 = 1000.0;
    for centered in [true, false] {
        let row = Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(titlebar::middle(centered, 12.0));
        let mut h = TestHarness::new(Box::new(DecoratedBox::new().class("titlebar").child(row)));
        let engine = h.apply_mss(synthos::styles::styles());
        syngui::signal::drain_and_run_effects();
        h.rebuild();
        h.apply_styles(&engine);
        h.layout(W, 32.0);

        let heading = h.element_bounds(h.find_by_class("titlebar-heading")[0]);
        let chips = h.find_by_class("titlebar-chip");
        assert_eq!(chips.len(), 2);
        let first = h.element_bounds(chips[0]);
        let last = h.element_bounds(chips[1]);
        // У `.titlebar` 16 px слева; справа — 8 px до кнопок окна и отступ чипа
        // внутри детектора.
        let right_gap = W - (last.origin.x + last.size.width);
        assert!((0.0..=12.0).contains(&right_gap), "centered={centered}: чипы не у правого края, зазор {right_gap}");
        assert!(first.origin.x > heading.origin.x + heading.size.width, "centered={centered}: чипы левее заголовка");
        if centered {
            let mid = heading.origin.x + heading.size.width / 2.0;
            let content_mid = 16.0 + (W - 16.0) / 2.0;
            assert!((mid - content_mid).abs() <= 1.0, "заголовок сдвинут: центр {mid}, ждали {content_mid}");
        } else {
            assert!(heading.origin.x <= 16.0 + 12.0 + 1.0, "заголовок не слева: {}", heading.origin.x);
        }
    }
}
