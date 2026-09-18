//! Видео в ленте чата: после ⏵ кадр и панель управления остаются в карточке.
//!
//! Зачем тест: плеер syngui (`video_player_view`) метит холст классом
//! `.ffmpeg-canvas`, а стиль под этот класс писался для полноэкранного
//! просмотрщика — 1080×620. В карточке 420×236 холст вырастал до 620 px в
//! высоту, `Contain` центрировал кадр по этой высоте, и видео уезжало в
//! нижний угол сцены, а полоса ⏵/перемотки уходила под обрезку целиком.
//!
//! Видео — `tests/fixtures/silent_160x120.mp4` (`testsrc` 160×120, 1 с, без
//! звуковой дорожки: плеер стартует сразу, и тест не должен пищать в колонки).
//! Нужен `TestHarness`: `cargo test --features testing --test chat_inline_video_layout`.

#![cfg(feature = "testing")]

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::Point;
use syngui::prelude::*;
use syngui::testing::{click_at, TestHarness};

use synthos::pages::syn_chat::media_inline;
use synthos::syn_chat::attach::blobs;
use synthos::syn_chat::state::{AttachmentKind, MsgAttachment};

fn video() -> MsgAttachment {
    MsgAttachment {
        sha256: "inline-video-layout-test".into(),
        mime: "video/mp4".into(),
        original_name: "node9.mp4".into(),
        width: 160,
        height: 120,
        size_bytes: 6_499,
        kind: AttachmentKind::Video,
        ext: "mp4".into(),
        duration_ms: 1_000,
        model_ext: String::new(),
        ui_ext: String::new(),
        has_thumb: false,
        share_path: false,
    }
}

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    h.rebuild();
    h.apply_styles(engine);
    h.layout_loose(800.0, 900.0);
}

fn one(h: &TestHarness, class: &str) -> syngui::core::Rect {
    let ids = h.find_by_class(class);
    assert_eq!(ids.len(), 1, "ровно один элемент .{class}");
    h.element_bounds(ids[0])
}

#[test]
fn playing_video_stays_inside_card() {
    let home = std::env::temp_dir().join(format!("synthos-inline-video-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    let a = video();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/silent_160x120.mp4");
    let blob = blobs::source_path(&a);
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::copy(&fixture, &blob).expect("фикстура silent_160x120.mp4");

    let mut h = TestHarness::new(media_inline::media_card(&a, || {}));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);

    // Постер с ⏵: клик по нему запускает плеер на том же месте.
    let stage = one(&h, "chat-media-stage");
    let poster_bottom = stage.origin.y + stage.size.height;
    h.send_events(&click_at(Point::new(
        stage.origin.x + stage.size.width / 2.0,
        stage.origin.y + stage.size.height / 2.0,
    )));
    settle(&mut h, &engine);

    let card = one(&h, "chat-media-card");
    let player = one(&h, "ffmpeg-player");
    let canvas = one(&h, "ffmpeg-canvas");
    let controls = one(&h, "ffmpeg-controls");
    let actions = one(&h, "chat-media-actions");

    let bottom = |r: syngui::core::Rect| r.origin.y + r.size.height;
    let right = |r: syngui::core::Rect| r.origin.x + r.size.width;

    // Плеер целиком над строкой действий: ничего не уходит под обрезку.
    assert!(
        bottom(player) <= actions.origin.y + 0.5,
        "плеер {player:?} залез под строку действий {actions:?}"
    );
    for (name, r) in [("холст", canvas), ("панель управления", controls)] {
        assert!(
            r.origin.x >= card.origin.x - 0.5 && right(r) <= right(card) + 0.5,
            "{name} {r:?} шире карточки {card:?}"
        );
        assert!(
            r.origin.y >= player.origin.y - 0.5 && bottom(r) <= bottom(player) + 0.5,
            "{name} {r:?} вне плеера {player:?}"
        );
    }
    assert!(controls.origin.y >= bottom(canvas) - 0.5, "полоса управления под кадром");

    // Кадр занимает место постера: та же высота сцены, во всю ширину карточки.
    assert!(
        (bottom(canvas) - poster_bottom).abs() < 1.0,
        "холст {canvas:?} должен кончаться там же, где постер ({poster_bottom})"
    );
    assert!(
        canvas.size.width >= card.size.width - 4.0,
        "холст {canvas:?} во всю ширину карточки {card:?}"
    );

    // Просмотрщик на весь экран — тот же плеер, но холст свой, 1080×620:
    // сужение селектора до `.media-viewer-video` его не должно задеть.
    let player = syngui::video::VideoPlayer::open(blob.to_str().unwrap()).expect("open");
    let mut h = TestHarness::new(Box::new(
        DecoratedBox::new()
            .class("media-viewer-video")
            .child(syngui::widgets::visual::video_player_view(Arc::new(Mutex::new(player)))),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    h.rebuild();
    h.apply_styles(&engine);
    h.layout_loose(1400.0, 900.0);
    let canvas = one(&h, "ffmpeg-canvas");
    assert_eq!(
        (canvas.size.width, canvas.size.height),
        (1080.0, 620.0),
        "холст просмотрщика"
    );

    let _ = std::fs::remove_dir_all(&home);
}
