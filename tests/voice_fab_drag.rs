//! Перетаскивание голосовой FAB-кнопки (`voice_fab::fab_button::layer` +
//! `components::drag_handle`).
//!
//! Зачем тест: слой кнопки растянут на всё окно и лежит поверх оболочки —
//! перестань он быть прозрачным для hit-test, приложение целиком перестало бы
//! реагировать на мышь, и сборка этого не покажет. Второе — развилка «щелчок
//! или перенос»: обёртка перехватывает нажатие раньше кнопки, и щелчок отдаёт
//! она сама; ошибка в пороге — и FAB либо не открывает окно распознавания,
//! либо открывает его после каждого перетаскивания.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, MouseButton};
use syngui::prelude::*;
use syngui::testing::*;
use syngui::widgets::GestureDetector;

use synthos::components::voice_fab::fab_button::{dragged_margin, layer, FAB_DEFAULT_MARGIN};

const W: f32 = 1200.0;
const H: f32 = 800.0;

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    syngui::signal::drain_and_run_effects();
    h.rebuild();
    h.apply_styles(engine);
    h.layout(W, H);
}

fn center(b: Rect) -> Point {
    Point::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0)
}

#[test]
fn margin_follows_cursor_and_stays_inside_window() {
    let bounds = Rect::new(Point::new(1140.0, 740.0), Size::new(44.0, 44.0));
    // Влево-вверх на 100×50 — отступы от правого-нижнего угла растут.
    assert_eq!(dragged_margin((16.0, 16.0), bounds, Point::new(-100.0, -50.0)), (116.0, 66.0));
    // За правый-нижний край не уходит.
    assert_eq!(dragged_margin((16.0, 16.0), bounds, Point::new(500.0, 500.0)), (4.0, 4.0));
    // За левый-верхний — тоже: не дальше расстояния кнопки до этих краёв.
    assert_eq!(
        dragged_margin((16.0, 16.0), bounds, Point::new(-5000.0, -5000.0)),
        (16.0 + 1140.0 - 4.0, 16.0 + 740.0 - 4.0)
    );
}

#[test]
fn fab_drags_live_saves_on_release_and_keeps_click() {
    syngui::signal::allow_signal_reads_on_this_thread();
    let margin = use_signal(FAB_DEFAULT_MARGIN);
    let saved = use_signal(FAB_DEFAULT_MARGIN);
    let clicks = Arc::new(AtomicUsize::new(0));
    let under = Arc::new(AtomicUsize::new(0));
    let (c, u) = (clicks.clone(), under.clone());

    // Под слоем — «страница» на всё окно: клики мимо кнопки обязаны доходить.
    let page = GestureDetector::new()
        .on_click(move || {
            u.fetch_add(1, Ordering::SeqCst);
        })
        .child(DecoratedBox::new().class("hf-page"));
    let fab = layer(
        margin,
        saved,
        move || {
            c.fetch_add(1, Ordering::SeqCst);
        },
        || Box::new(DecoratedBox::new().class("fab-voice-corner")),
    );
    let mut h = TestHarness::new(Box::new(Stack::new().clip(false).child(page).child(fab)));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);

    // ── По умолчанию — правый нижний угол, 16 px от краёв ─────────────
    let b = h.element_bounds(h.find_by_class("fab-voice-corner")[0]);
    assert_eq!((b.size.width, b.size.height), (44.0, 44.0));
    assert_eq!((b.origin.x, b.origin.y), (W - 16.0 - 44.0, H - 16.0 - 44.0), "{b:?}");

    // ── Слой на всё окно не крадёт клики у страницы ───────────────────
    h.send_events(&click_at(Point::new(300.0, 300.0)));
    assert_eq!(under.load(Ordering::SeqCst), 1, "клик мимо FAB дошёл до страницы");
    assert_eq!(clicks.load(Ordering::SeqCst), 0);

    // ── Нажатие без движения — щелчок по FAB, страница его не видит ───
    h.send_events(&click_at(center(b)));
    assert_eq!(clicks.load(Ordering::SeqCst), 1, "щелчок по FAB");
    assert_eq!(under.load(Ordering::SeqCst), 1);
    assert_eq!(margin.get_untracked(), FAB_DEFAULT_MARGIN);

    // ── Перенос: кнопка едет за курсором, в конфиг — только по отпусканию
    let start = center(b);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: start });
    h.send_event(&Event::MouseMove(Point::new(start.x - 200.0, start.y - 100.0)));
    settle(&mut h, &engine);
    assert_eq!(margin.get_untracked(), (216.0, 116.0));
    assert_eq!(saved.get_untracked(), FAB_DEFAULT_MARGIN, "пока тащим — конфиг не трогаем");
    let moved = h.element_bounds(h.find_by_class("fab-voice-corner")[0]);
    assert_eq!((moved.origin.x, moved.origin.y), (b.origin.x - 200.0, b.origin.y - 100.0));

    // Курсор уже вне кнопки (она пересобрана) — перенос продолжается от той
    // же точки нажатия, без накопления ошибки.
    h.send_event(&Event::MouseMove(Point::new(start.x - 500.0, start.y - 400.0)));
    settle(&mut h, &engine);
    assert_eq!(margin.get_untracked(), (516.0, 416.0));

    h.send_event(&Event::MouseUp {
        button: MouseButton::Left,
        position: Point::new(start.x - 500.0, start.y - 400.0),
    });
    settle(&mut h, &engine);
    assert_eq!(saved.get_untracked(), (516.0, 416.0), "позиция записана по отпусканию");
    assert_eq!(clicks.load(Ordering::SeqCst), 1, "перенос — не щелчок");

    // ── Следующий перенос стартует от новой позиции ───────────────────
    let b2 = h.element_bounds(h.find_by_class("fab-voice-corner")[0]);
    let s2 = center(b2);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: s2 });
    h.send_event(&Event::MouseMove(Point::new(s2.x + 100.0, s2.y + 100.0)));
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: Point::new(s2.x + 100.0, s2.y + 100.0) });
    settle(&mut h, &engine);
    assert_eq!(saved.get_untracked(), (416.0, 316.0));
}
