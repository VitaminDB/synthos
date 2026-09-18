//! Видео в ленте чата: после ⏵ кадр — на месте постера, панель управления
//! (компактный плеер `components::video_player`) — поверх кадра, в карточке;
//! группы панели не наезжают друг на друга, «во весь экран» открывает
//! просмотрщик и ставит карточку на паузу.
//!
//! Зачем тест: прежний плеер syngui (`video_player_view`) метил холст
//! `.ffmpeg-canvas`, а стиль под этот класс писался для полноэкранного
//! просмотрщика — 1080×620: в карточке 420×236 видео уезжало в нижний угол,
//! а полоса ⏵/перемотки уходила под обрезку. Теперь в карточке тот же плеер,
//! что в просмотрщике, в компактном режиме — и в 420 px должны влезть время,
//! пять кнопок транспорта и «во весь экран».
//!
//! Видео — `tests/fixtures/silent_160x120_20s.mp4` (`testsrc` 160×120, 20 с,
//! без звуковой дорожки: плеер стартует сразу, и тест не должен пищать в
//! колонки). `cargo test --features testing --test chat_inline_video_layout`.

#![cfg(feature = "testing")]

use syngui::core::Point;
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
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/silent_160x120_20s.mp4");
    let blob = blobs::source_path(&a);
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::copy(&fixture, &blob).expect("фикстура silent_160x120_20s.mp4");

    let opened = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let opened_in = opened.clone();
    let mut h = TestHarness::new(media_inline::media_card(&a, move || {
        opened_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
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
    let canvas = one(&h, "vp-canvas");
    let controls = one(&h, "vp-controls-sm");
    let actions = one(&h, "chat-media-actions");

    let bottom = |r: syngui::core::Rect| r.origin.y + r.size.height;
    let right = |r: syngui::core::Rect| r.origin.x + r.size.width;

    // Кадр — на месте постера: та же высота, во всю ширину карточки.
    assert!(
        (bottom(canvas) - poster_bottom).abs() < 1.0,
        "холст {canvas:?} должен кончаться там же, где постер ({poster_bottom})"
    );
    assert!(
        (canvas.size.height - 236.0).abs() < 1.0,
        "холст {canvas:?} высотой с постер"
    );
    assert!(
        canvas.size.width >= card.size.width - 4.0,
        "холст {canvas:?} во всю ширину карточки {card:?}"
    );
    assert!(
        bottom(canvas) <= actions.origin.y + 0.5,
        "кадр над строкой действий"
    );

    // Панель поверх кадра у его нижнего края, в пределах карточки.
    assert!(
        (bottom(controls) - bottom(canvas)).abs() < 1.0 && controls.origin.y > canvas.origin.y,
        "панель {controls:?} у низа кадра {canvas:?}"
    );
    assert!(
        controls.origin.x >= card.origin.x - 0.5 && right(controls) <= right(card) + 0.5,
        "панель {controls:?} шире карточки {card:?}"
    );

    // Время, транспорт (−1 ⟲10 ⏯ 10⟳ +1) и «во весь экран» не наезжают друг
    // на друга, ⏯ по центру.
    let time = one(&h, "vp-time-sm");
    let play = one(&h, "vp-play-sm");
    let mut btns: Vec<syngui::core::Rect> = h
        .find_by_class("vp-btn-sm")
        .into_iter()
        .map(|id| h.element_bounds(id))
        .collect();
    btns.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    assert_eq!(
        btns.len(),
        5,
        "−1, ⟲10, 10⟳, +1 и «во весь экран»: {btns:?}"
    );
    let mid = canvas.origin.x + canvas.size.width / 2.0;
    assert!(
        (play.origin.x + play.size.width / 2.0 - mid).abs() < 2.0,
        "⏯ по центру: {play:?}"
    );
    assert!(
        right(time) <= btns[0].origin.x,
        "время {time:?} наезжает на −1 {:?}",
        btns[0]
    );
    assert!(
        right(btns[3]) <= btns[4].origin.x,
        "+1 наезжает на «во весь экран»"
    );
    assert!(
        right(btns[4]) <= right(card) + 0.5,
        "«во весь экран» за краем карточки"
    );

    // «Во весь экран» — открыть просмотрщик, карточку поставить на паузу.
    let full = btns[4];
    h.send_events(&click_at(Point::new(
        full.origin.x + full.size.width / 2.0,
        full.origin.y + full.size.height / 2.0,
    )));
    // Паузу, поставленную мимо кнопок плеера, подхватывает его поток-тикер
    // (шаг 50 мс) через очередь главного потока — в тесте её разбираем сами.
    std::thread::sleep(std::time::Duration::from_millis(200));
    syngui::async_runtime::drain_main_thread_callbacks();
    settle(&mut h, &engine);
    assert_eq!(
        opened.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "просмотрщик открыт"
    );
    assert_eq!(
        h.find_by_class("vp-big-play-sm").len(),
        1,
        "карточка на паузе — большая ⏵ на кадре"
    );

    let _ = std::fs::remove_dir_all(&home);
}
