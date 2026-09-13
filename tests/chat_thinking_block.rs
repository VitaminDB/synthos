//! Блок «Размышления» в пузыре ассистента
//! (`pages::syn_chat::message_bubble`).
//!
//! Зачем тест: над последним ответом шапка «Размышления» стояла всегда — и в
//! чате с выключенными размышлениями, где их не было вовсе. Блок должен
//! появляться только при тексте размышлений: сохранённом или хвосте идущего
//! хода. Отдельный файл: контекст `SynChatCtx` общий на процесс.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::pages::syn_chat::message_bubble;
use synthos::syn_chat::state::ChatMsg;
use synthos::syn_chat::{SynChatCtx, SynModelRegistry};

fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    syngui::signal::drain_and_run_effects();
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(800.0, 600.0);
}

fn answer(thinking: &str) -> ChatMsg {
    let mut m = ChatMsg::assistant_empty();
    m.body = "Напрямую мне доступны два инструмента.".into();
    m.thinking = thinking.into();
    m
}

fn bubble(msg: &ChatMsg, is_last: bool) -> (TestHarness, syngui::mss::StyleEngine) {
    let mut harness = TestHarness::new(Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![message_bubble::view(msg, 1, false, is_last, "full")]),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);
    (harness, engine)
}

#[test]
fn thinking_header_only_with_thinking_text() {
    let home = std::env::temp_dir().join(format!("synthos-thinking-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::signal::allow_signal_reads_on_this_thread();
    let (_theme, app) = synthos::build_context();
    synthos::i18n::install(app.general);
    provide_context(app);
    provide_context(SynChatCtx::new());
    provide_context(SynModelRegistry::new());
    let ctx = use_context::<SynChatCtx>();

    // Последний ответ без размышлений — шапки нет.
    let (mut h, engine) = bubble(&answer(""), true);
    assert!(h.find_by_class("msg-thinking").is_empty(), "размышлений не было");
    let plain_height = h.find_by_class("msg-bubble-in").first().map(|&id| h.element_bounds(id).size.height);

    // Ход идёт, размышлений ещё нет — блока нет и текст не сдвинут.
    ctx.pending.set(true);
    settle(&mut h, &engine);
    assert!(h.find_by_class("msg-thinking").is_empty(), "ход без размышлений");
    let pending_height = h.find_by_class("msg-bubble-in").first().map(|&id| h.element_bounds(id).size.height);
    assert!(plain_height.is_some_and(|v| v > 0.0), "пузырь не найден: {plain_height:?}");
    assert_eq!(pending_height, plain_height, "пустой блок размышлений занял место");

    // Пошёл хвост размышлений — блок появился.
    ctx.streaming_thinking.set("думаю".into());
    settle(&mut h, &engine);
    assert_eq!(h.find_by_class("msg-thinking").len(), 1, "хвост идущего хода");

    // Хвост без хода (чужой или устаревший) не показывается.
    ctx.pending.set(false);
    settle(&mut h, &engine);
    assert!(h.find_by_class("msg-thinking").is_empty(), "хвост без хода");
    ctx.streaming_thinking.set(String::new());

    // Сохранённые размышления видны и у последнего, и у прежнего ответа.
    let (h, _) = bubble(&answer("рассуждение"), true);
    assert_eq!(h.find_by_class("msg-thinking").len(), 1, "последний с размышлениями");
    let (h, _) = bubble(&answer("рассуждение"), false);
    assert_eq!(h.find_by_class("msg-thinking").len(), 1, "прежний с размышлениями");
    let (h, _) = bubble(&answer(""), false);
    assert!(h.find_by_class("msg-thinking").is_empty(), "прежний без размышлений");
}
