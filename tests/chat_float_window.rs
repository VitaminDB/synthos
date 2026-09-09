//! Отрыв чата в плавающее окно (`pages::syn_chat::float_window`).
//!
//! Зачем тест: лента и ввод должны существовать в одном экземпляре —
//! пока чат оторван, центральная колонка страницы обязана показывать
//! плейсхолдер, а не вторую ленту; кнопка «Показать окно» появляется
//! только у свёрнутого окна; закрытое окно не держит тело. Плюс
//! арифметика `detach_at`: окно от дропа не должно вылезать за хост.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::pages::syn_chat::{chat_pane, float_window};
use synthos::syn_chat::state::CHAT_WINDOW_DEFAULT_SIZE;
use synthos::syn_chat::SynChatCtx;

fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(1400.0, 900.0);
}

/// Конструктор контекста читает `~/.config/synthos` — уводим HOME во
/// временный каталог, чтобы тест не трогал настройки пользователя.
fn isolated_ctx(tag: &str) -> SynChatCtx {
    let home = std::env::temp_dir().join(format!("synthos-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    provide_context(SynChatCtx::new());
    use_context::<SynChatCtx>()
}

#[test]
fn detached_page_shows_placeholder_instead_of_feed() {
    let ctx = isolated_ctx("float-placeholder");
    ctx.chat_detached.set(true);

    let mut harness = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(chat_pane::view()),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);

    assert_eq!(harness.find_by_class("chat-detached-placeholder").len(), 1, "плейсхолдер");
    assert!(harness.find_by_class("message-area").is_empty(), "ленты на странице быть не должно");
    assert!(harness.find_by_class("input-panel").is_empty(), "поля ввода на странице быть не должно");

    // Окно развёрнуто — одна кнопка «Вернуть»; свёрнуто — ещё и
    // «Показать окно».
    assert_eq!(harness.find_by_class("code-editor-dialog-btn-primary").len(), 1);
    assert!(harness.find_by_class("code-editor-dialog-btn-secondary").is_empty());
    ctx.chat_window_minimized.set(true);
    settle(&mut harness, &engine);
    assert_eq!(harness.find_by_class("code-editor-dialog-btn-secondary").len(), 1, "«Показать окно»");
}

#[test]
fn closed_window_has_no_body() {
    let ctx = isolated_ctx("float-closed");
    ctx.chat_detached.set(false);

    let mut harness = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(float_window::window()),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);

    assert_eq!(harness.find_by_class("chat-float-window-empty").len(), 1, "пустышка вместо тела");
    assert!(harness.find_by_class("message-area").is_empty(), "лента в закрытом окне не строится");
}

/// Окно берёт тень и рамку из MSS (`FloatingWindow` раньше рисовал
/// захардкоженную тень): двухслойная тень, вторая — `--glass-shadow` темы,
/// радиус 14px. Ловит и парсер: `var()` внутри списка теней должен
/// раскрываться.
#[test]
fn window_style_has_layered_shadow_and_rounder_corners() {
    // Закрытое окно: стиль применяется и к нему, а живая лента требует AppCtx.
    let ctx = isolated_ctx("float-style");
    ctx.chat_detached.set(false);

    let mut harness = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(float_window::window()),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);

    let win = harness.find_by_class("chat-float-window")[0];
    let mss = harness.element_mss(win).expect("mss окна");
    let shadows = mss.box_shadow.as_ref().expect("box-shadow из стиля");
    assert_eq!(shadows.as_slice().len(), 2, "ближняя тень + тень темы");
    assert!(shadows.as_slice().iter().all(|s| !s.inset && s.blur_radius > 0.0));
    assert_eq!(mss.border_radius_uniform(100.0, 0.0), 14.0, "радиус углов окна");
    assert_eq!(mss.border_width_or(0.0), 1.0, "рамка 1px");
}

#[test]
fn detach_at_puts_title_under_cursor_and_keeps_window_inside_host() {
    let ctx = isolated_ctx("float-detach-at");
    let size = CHAT_WINDOW_DEFAULT_SIZE;
    let host = Size::new(1400.0, 900.0);

    // Дроп в верхней части: заголовок под курсором, окно центрировано по x
    // (окно 680px высотой — ниже y≈220 его уже прижимало бы к низу хоста).
    float_window::detach_at(Point::new(700.0, 100.0), host);
    assert!(ctx.chat_detached.get_untracked());
    assert!(!ctx.chat_window_minimized.get_untracked());
    let p = ctx.chat_window_pos.get_untracked();
    assert!((p.x - (700.0 - size.width / 2.0)).abs() < 0.5, "x: {p:?}");
    assert!((p.y - (100.0 - 18.0)).abs() < 0.5, "y: {p:?}");

    // Дроп у правого нижнего края: окно прижимается к границам хоста.
    float_window::detach_at(Point::new(1390.0, 890.0), host);
    let p = ctx.chat_window_pos.get_untracked();
    assert!((p.x - (host.width - size.width)).abs() < 0.5, "x у края: {p:?}");
    assert!((p.y - (host.height - size.height)).abs() < 0.5, "y у края: {p:?}");

    // Дроп у левого верхнего угла — не уходит в отрицательные координаты.
    float_window::detach_at(Point::new(5.0, 5.0), host);
    let p = ctx.chat_window_pos.get_untracked();
    assert_eq!((p.x, p.y), (0.0, 0.0), "{p:?}");
}
