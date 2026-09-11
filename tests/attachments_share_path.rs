//! Галочка «передать модели путь к файлу» в полосе вложений: появляется
//! только при вложениях, выключена по умолчанию, по клику взводит
//! `attach_share_paths`, переживает пересборку полосы (добавили ещё файл),
//! сбрасывается вместе с черновиком и не вылезает за край узкого окна чата.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::{click_at, TestHarness};

use synthos::pages::syn_chat::attachments;
use synthos::syn_chat::state::{AttachmentKind, MsgAttachment};
use synthos::syn_chat::SynChatCtx;

/// Плавающее окно чата по умолчанию (560) минус отступы панели ввода.
const NARROW: f32 = 480.0;

fn doc(sha: char, name: &str) -> MsgAttachment {
    MsgAttachment {
        sha256: sha.to_string().repeat(64),
        mime: "text/markdown".into(),
        original_name: name.into(),
        width: 0,
        height: 0,
        size_bytes: 2048,
        kind: AttachmentKind::Document,
        ext: "md".into(),
        duration_ms: 0,
        model_ext: String::new(),
        ui_ext: String::new(),
        has_thumb: false,
        share_path: false,
    }
}

fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(NARROW, 400.0);
}

#[test]
fn share_path_checkbox_toggles_and_resets_with_draft() {
    let home = std::env::temp_dir().join(format!("synthos-share-path-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));

    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();

    let mut harness = TestHarness::new(Box::new(attachments::strip()));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);
    assert!(
        harness.find_by_class("attachments-share-path").is_empty(),
        "без вложений галочки нет"
    );

    ctx.pending_attachments.set(vec![doc('a', "notes.md")]);
    settle(&mut harness, &engine);
    let boxes = harness.find_by_class("attachments-share-path");
    assert_eq!(boxes.len(), 1, "одна галочка под полосой");
    let b = harness.element_bounds(boxes[0]);
    assert!(b.size.width > 20.0 && b.size.height > 0.0, "галочка без подписи: {b:?}");
    assert!(
        b.origin.x + b.size.width <= NARROW + 0.5,
        "галочка вылезла за окно шириной {NARROW}: {b:?}"
    );
    assert!(!ctx.attach_share_paths.get_untracked(), "по умолчанию выключена");

    harness.send_events(&click_at(Point::new(
        b.origin.x + 10.0,
        b.origin.y + b.size.height / 2.0,
    )));
    assert!(ctx.attach_share_paths.get_untracked(), "клик взводит флаг");

    // Второй файл пересобирает полосу — отметка не теряется.
    ctx.pending_attachments.update(|l| l.push(doc('b', "plan.md")));
    settle(&mut harness, &engine);
    assert_eq!(harness.find_by_class("attachments-share-path").len(), 1);
    assert!(ctx.attach_share_paths.get_untracked());

    // Отправка и смена чата сбрасывают черновик вместе с галочкой.
    ctx.clear_draft_attachments();
    settle(&mut harness, &engine);
    assert!(!ctx.attach_share_paths.get_untracked(), "галочка сброшена с черновиком");
    assert!(harness.find_by_class("attachments-share-path").is_empty());
}
