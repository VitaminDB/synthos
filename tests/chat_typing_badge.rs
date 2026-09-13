//! Бейджик «модель печатает» в полосе плавающего окна чата
//! (`pages::syn_chat::float_window::window_bar`).
//!
//! Зачем тест: индикатор должен появляться со стартом хода и гаснуть с его
//! концом, и только для открытого чата — фоновый ход другого чата виден на
//! его плитке в рейле, а не в окне. Отдельный файл: контекст `SynChatCtx`
//! общий на процесс, соседние тесты с `provide_context` гонялись бы за него.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::pages::syn_chat::float_window;
use synthos::syn_chat::SynChatCtx;

fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(800.0, 600.0);
}

#[test]
fn window_bar_shows_typing_only_while_open_chat_generates() {
    // Конструктор контекста читает `~/.config/synthos` — уводим HOME во
    // временный каталог.
    let home = std::env::temp_dir().join(format!("synthos-typing-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();
    ctx.active_chat_id.set(Some("a".into()));

    let mut harness = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(float_window::window_bar()),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);
    assert!(harness.find_by_class("chat-typing-dot").is_empty(), "хода нет");
    assert_eq!(harness.find_by_class("panel-header-action").len(), 1, "кнопка «К панелям»");

    ctx.generating_chat.set(Some("b".into()));
    settle(&mut harness, &engine);
    assert!(harness.find_by_class("chat-typing-dot").is_empty(), "ход в другом чате");

    ctx.generating_chat.set(Some("a".into()));
    settle(&mut harness, &engine);
    assert_eq!(harness.find_by_class("chat-typing-dot").len(), 1, "ход в открытом чате");
    assert_eq!(harness.find_by_class("chat-float-window-typing").len(), 1, "подпись «печатает…»");

    ctx.generating_chat.set(None);
    settle(&mut harness, &engine);
    assert!(harness.find_by_class("chat-typing-dot").is_empty(), "ход закончился");
}
