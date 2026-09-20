//! Картинка в просмотрщике вложений: область просмотра занимает всё тело
//! карточки, картинка стоит по центру, окно разворачивается и меняет размер,
//! колесо над сценой доходит до области просмотра сквозь слои с кнопками.
//!
//! Зачем тест: на скриншоте 20.09 картинка стояла у левого края сцены
//! фиксированного размера 1100×680 (стили `.media-viewer-panzoom`), полос
//! прокрутки при увеличении не было, окно не разворачивалось и не тянулось.
//!
//! Стора картинок в харнессе нет — область просмотра считает вид по размеру
//! из метаданных вложения, этого проверкам хватает.
//! `cargo test --features testing --test media_viewer_image`.

#![cfg(feature = "testing")]

use syngui::core::Rect;
use syngui::input::{Event, Key};
use syngui::prelude::*;
use syngui::render::DrawCommand;
use syngui::testing::{press_key, TestHarness};

use synthos::pages::syn_chat::media_viewer;
use synthos::syn_chat::state::{AttachmentKind, MsgAttachment, ViewerState};
use synthos::syn_chat::SynChatCtx;

const W: f32 = 1600.0;
const H: f32 = 1000.0;

fn image(sha: &str, width: u32, height: u32) -> MsgAttachment {
    MsgAttachment {
        sha256: sha.into(),
        mime: "image/png".into(),
        original_name: format!("{sha}.png"),
        width,
        height,
        size_bytes: 1_100_000,
        kind: AttachmentKind::Image,
        ext: "png".into(),
        duration_ms: 0,
        model_ext: String::new(),
        ui_ext: String::new(),
        has_thumb: false,
        share_path: false,
    }
}

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

fn right(r: Rect) -> f32 {
    r.origin.x + r.size.width
}

fn bottom(r: Rect) -> f32 {
    r.origin.y + r.size.height
}

fn same(a: Rect, b: Rect) -> bool {
    (a.origin.x - b.origin.x).abs() < 1.5
        && (a.origin.y - b.origin.y).abs() < 1.5
        && (a.size.width - b.size.width).abs() < 1.5
        && (a.size.height - b.size.height).abs() < 1.5
}

fn viewport(h: &TestHarness) -> Rect {
    let ids = h.find_by_type_name("ImageViewport");
    assert_eq!(ids.len(), 1, "одна область просмотра");
    h.element_bounds(ids[0])
}

fn zoom_text(h: &mut TestHarness) -> String {
    h.paint()
        .commands()
        .into_iter()
        .find_map(|cmd| match cmd {
            DrawCommand::Text { text, .. } if text.ends_with('%') => Some(text.to_string()),
            _ => None,
        })
        .expect("подпись масштаба на панели")
}

#[test]
fn image_stage_fills_card_and_window_resizes() {
    let home = std::env::temp_dir().join(format!("synthos-viewer-image-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.set(Some(ViewerState {
        items: vec![image("viewer-image-a", 896, 1184), image("viewer-image-b", 4000, 1000)],
        index: 0,
    }));

    let mut h = TestHarness::new(Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(media_viewer::view()),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    frames(&mut h, &engine);

    // Карточка по центру окна; под шапкой — сцена до самого низа и во всю
    // ширину, область просмотра — вся сцена. Подвала у картинки нет.
    let card = one(&h, "media-viewer");
    assert!(
        (card.origin.x + card.size.width * 0.5 - W * 0.5).abs() < 1.0,
        "карточка по центру: {card:?}"
    );
    let header = one(&h, "media-viewer-header");
    let stage = one(&h, "iv-stage");
    assert!(h.find_by_class("media-viewer-footer").is_empty());
    assert!((stage.origin.y - bottom(header)).abs() < 1.0, "{stage:?}");
    assert!((bottom(stage) - bottom(card)).abs() < 1.5, "{stage:?} / {card:?}");
    assert!((stage.size.width - card.size.width).abs() < 2.5, "{stage:?} / {card:?}");
    assert!(same(viewport(&h), stage), "область просмотра во всю сцену");
    assert!(same(one(&h, "iv-backdrop"), stage), "размытый фон во всю сцену");

    // Кнопки шапки прижаты к правому краю, а не идут сразу за названием.
    let close = one(&h, "media-viewer-close");
    assert!(
        right(card) - right(close) < 24.0,
        "крестик у правого края: {close:?} / {card:?}"
    );

    // Панель — внизу по центру сцены, лента миниатюр — над ней.
    let bar = one(&h, "iv-toolbar");
    assert!(
        (bar.origin.x + bar.size.width * 0.5 - (stage.origin.x + stage.size.width * 0.5)).abs()
            < 1.5,
        "панель по центру: {bar:?}"
    );
    assert!(bottom(bar) < bottom(stage) && bottom(bar) > bottom(stage) - 40.0);
    let strip = one(&h, "iv-strip");
    assert!(bottom(strip) <= bar.origin.y, "лента над панелью");
    assert_eq!(h.find_by_class("iv-thumb").len(), 2);
    assert_eq!(h.find_by_class("iv-thumb-active").len(), 1);

    // Открывается вписанной: 896×1184 в сцену ниже 1184 — меньше 100 %.
    let fitted = zoom_text(&mut h);
    assert_ne!(fitted, "100%");

    // Колесо над серединой сцены проходит сквозь слои стрелок и панели.
    h.send_event(&Event::MouseWheel {
        delta: 120.0,
        delta_x: 0.0,
        position: Point::new(W * 0.5, stage.origin.y + stage.size.height * 0.4),
    });
    frames(&mut h, &engine);
    assert_ne!(zoom_text(&mut h), fitted, "колесо меняет масштаб");

    // «1» — пиксель в пиксель, «0» — снова вписать.
    h.send_events(&press_key(Key::Num1));
    frames(&mut h, &engine);
    assert_eq!(zoom_text(&mut h), "100%");
    h.send_events(&press_key(Key::Num0));
    frames(&mut h, &engine);
    assert_eq!(zoom_text(&mut h), fitted);

    // «M» разворачивает окно на всё окно приложения, шапка остаётся.
    h.send_events(&press_key(Key::M));
    frames(&mut h, &engine);
    let card = one(&h, "media-viewer");
    assert!(
        same(card, Rect::new(Point::new(0.0, 0.0), Size::new(W, H))),
        "развёрнуто: {card:?}"
    );
    assert_eq!(h.find_by_class("media-viewer-header").len(), 1);
    assert!(
        h.find_by_class("media-viewer-grip").is_empty(),
        "развёрнутое окно за край не тянут"
    );
    let stage = one(&h, "iv-stage");
    assert!(same(viewport(&h), stage));
    assert!((right(stage) - W).abs() < 1.5 && (bottom(stage) - H).abs() < 1.5);
    h.send_events(&press_key(Key::M));
    frames(&mut h, &engine);

    // Правый край тянем на 100 px вправо: карточка по центру, поэтому
    // ширина растёт на 200, а край оказывается под курсором.
    let card = one(&h, "media-viewer");
    let grab = Point::new(right(card) - 2.0, card.origin.y + card.size.height * 0.5);
    h.send_event(&Event::MouseMove(grab));
    h.send_event(&Event::MouseDown {
        button: syngui::input::MouseButton::Left,
        position: grab,
    });
    let to = Point::new(grab.x + 100.0, grab.y);
    h.send_event(&Event::MouseMove(Point::new(grab.x + 50.0, grab.y)));
    frames(&mut h, &engine);
    h.send_event(&Event::MouseMove(to));
    frames(&mut h, &engine);
    h.send_event(&Event::MouseUp {
        button: syngui::input::MouseButton::Left,
        position: to,
    });
    frames(&mut h, &engine);
    let wider = one(&h, "media-viewer");
    assert!(
        (wider.size.width - (card.size.width + 200.0)).abs() < 2.0,
        "было {card:?}, стало {wider:?}"
    );
    assert!((right(wider) - to.x).abs() < 3.0, "край под курсором: {wider:?}");
    assert!(same(viewport(&h), one(&h, "iv-stage")), "сцена тянется вместе с окном");

    // Стрелка вправо у вписанной картинки листает; у новой — свой масштаб.
    h.send_events(&press_key(Key::Right));
    frames(&mut h, &engine);
    assert_eq!(ctx.viewer.get_untracked().unwrap().index, 1);
    assert_ne!(zoom_text(&mut h), fitted, "4000×1000 вписывается иначе");

    // F — во весь экран: только сцена; Esc сначала выходит из него.
    h.send_events(&press_key(Key::F));
    frames(&mut h, &engine);
    assert_eq!(h.find_by_class("media-viewer-full").len(), 1);
    assert!(h.find_by_class("media-viewer-header").is_empty());
    assert!(same(viewport(&h), Rect::new(Point::new(0.0, 0.0), Size::new(W, H))));
    h.send_events(&press_key(Key::Escape));
    frames(&mut h, &engine);
    assert!(h.find_by_class("media-viewer-full").is_empty());
    assert!(ctx.viewer.get_untracked().is_some(), "первый Esc не закрывает");
    h.send_events(&press_key(Key::Escape));
    frames(&mut h, &engine);
    assert!(ctx.viewer.get_untracked().is_none());

    let _ = std::fs::remove_dir_all(&home);
}
