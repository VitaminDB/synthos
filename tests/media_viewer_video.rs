//! Видео в полноэкранном просмотрщике вложений: плеер занимает всё тело
//! карточки, пустого подвала нет, F разворачивает на всё окно, Esc сначала
//! сворачивает обратно и только потом закрывает просмотр.
//!
//! Зачем тест: на скриншоте 18.09 плеер стоял холстом 1080×620 у левого края
//! сцены, справа и снизу — пустые полосы, под ними пустой подвал. Переход во
//! весь экран пересобирает карточку — плеер при этом должен остаться тем же
//! (та же пауза, та же позиция), а не открыться заново.
//!
//! Видео — `tests/fixtures/silent_160x120_20s.mp4` (без звука, 20 с).
//! `cargo test --features testing --test media_viewer_video`.

#![cfg(feature = "testing")]

use syngui::core::Rect;
use syngui::input::Key;
use syngui::prelude::*;
use syngui::testing::{press_key, TestHarness};

use synthos::pages::syn_chat::media_viewer;
use synthos::syn_chat::attach::blobs;
use synthos::syn_chat::state::{AttachmentKind, MsgAttachment, ViewerState};
use synthos::syn_chat::SynChatCtx;

const W: f32 = 1600.0;
const H: f32 = 1000.0;

fn video() -> MsgAttachment {
    MsgAttachment {
        sha256: "media-viewer-video-test".into(),
        mime: "video/mp4".into(),
        original_name: "node9.mp4".into(),
        width: 160,
        height: 120,
        size_bytes: 7_844,
        kind: AttachmentKind::Video,
        ext: "mp4".into(),
        duration_ms: 20_000,
        model_ext: String::new(),
        ui_ext: String::new(),
        has_thumb: false,
        share_path: false,
    }
}

/// Кадр приложения: пересборки, стили, раскладка, стек оверлеев (Portal
/// модальный — клавиши идут через него).
fn frames(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    for _ in 0..3 {
        h.frame(Some(engine), W, H);
    }
}

fn one(h: &TestHarness, class: &str) -> Rect {
    let ids = h.find_by_class(class);
    assert_eq!(ids.len(), 1, "ровно один элемент .{class}");
    h.element_bounds(ids[0])
}

fn bottom(r: Rect) -> f32 {
    r.origin.y + r.size.height
}

fn same(a: Rect, b: Rect) -> bool {
    (a.origin.x - b.origin.x).abs() < 1.0
        && (a.origin.y - b.origin.y).abs() < 1.0
        && (a.size.width - b.size.width).abs() < 1.0
        && (a.size.height - b.size.height).abs() < 1.0
}

#[test]
fn video_fills_card_and_goes_fullscreen() {
    let home = std::env::temp_dir().join(format!("synthos-viewer-video-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    let a = video();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/silent_160x120_20s.mp4");
    let blob = blobs::source_path(&a);
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::copy(&fixture, &blob).expect("фикстура silent_160x120_20s.mp4");

    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.set(Some(ViewerState {
        items: vec![a],
        index: 0,
    }));

    let mut h = TestHarness::new(Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(media_viewer::view()),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    frames(&mut h, &engine);

    // Карточка: шапка, под ней плеер до самого низа; подвала нет.
    let card = one(&h, "media-viewer");
    let header = one(&h, "media-viewer-header");
    let stage = one(&h, "media-viewer-video-stage");
    assert!(
        h.find_by_class("media-viewer-footer").is_empty(),
        "пустой подвал у видео"
    );
    assert!(
        (stage.origin.y - bottom(header)).abs() < 1.0,
        "сцена сразу под шапкой: {stage:?}"
    );
    assert!(
        (bottom(stage) - bottom(card)).abs() < 1.5,
        "сцена до низа карточки: {stage:?} / {card:?}"
    );
    assert!(
        (stage.size.width - card.size.width).abs() < 2.5,
        "сцена во всю ширину: {stage:?} / {card:?}"
    );
    assert!(same(one(&h, "vp-canvas"), stage), "кадр во всю сцену");
    assert!(
        (bottom(one(&h, "vp-controls")) - bottom(stage)).abs() < 1.0,
        "панель у низа сцены"
    );

    // Пауза пробелом, затем F — во весь экран: только плеер, на всё окно,
    // и это тот же плеер — он по-прежнему на паузе (новый стартовал бы сам).
    h.send_events(&press_key(Key::Space));
    frames(&mut h, &engine);
    assert_eq!(
        h.find_by_class("vp-big-play").len(),
        1,
        "пробел ставит паузу"
    );
    h.send_events(&press_key(Key::F));
    frames(&mut h, &engine);
    assert_eq!(
        h.find_by_class("media-viewer-full").len(),
        1,
        "F — во весь экран"
    );
    assert!(
        h.find_by_class("media-viewer-header").is_empty(),
        "без шапки"
    );
    assert!(
        same(
            one(&h, "vp-canvas"),
            Rect::new(Point::new(0.0, 0.0), Size::new(W, H))
        ),
        "кадр на всё окно"
    );
    assert_eq!(
        h.find_by_class("vp-big-play").len(),
        1,
        "плеер тот же: всё ещё на паузе"
    );

    // Esc — сначала выход из полноэкранного, просмотр открыт.
    h.send_events(&press_key(Key::Escape));
    frames(&mut h, &engine);
    assert!(
        h.find_by_class("media-viewer-full").is_empty(),
        "Esc сворачивает"
    );
    assert_eq!(h.find_by_class("media-viewer-header").len(), 1);
    assert!(
        ctx.viewer.get_untracked().is_some(),
        "первый Esc не закрывает просмотр"
    );

    // Второй Esc — закрыть.
    h.send_events(&press_key(Key::Escape));
    frames(&mut h, &engine);
    assert!(
        ctx.viewer.get_untracked().is_none(),
        "второй Esc закрывает просмотр"
    );

    let _ = std::fs::remove_dir_all(&home);
}
