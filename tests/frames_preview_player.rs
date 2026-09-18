//! Превью кадров из памяти в нодах (LTX/H3 VAE Decode, сохранение H3,
//! видеоплеер на выходе пайплайна): общий плеер приложения в компактном
//! режиме поверх `FramesSource`.
//!
//! Зачем тест: раньше превью было голым `FramesView` (клик — play/pause, без
//! перемотки и времени). Теперь там панель плеера: в 360×202 должны влезть
//! время и пять кнопок транспорта без наложений, пробел и ⏵ запускают часы
//! кадров, ±1 с двигает позицию, а ▶ ноды находит тот же источник
//! (`FramesSource::find`), что показывает кадр.
//!
//! Второй тест — нода «Видеоплеер» с кадрами на входе: плеер на весь холст
//! 640×480, заглушка без файла, ▶ ноды (`on_run`) — пауза/старт того же
//! источника.
//!
//! Кадры синтетические (48 × 32×18 при 24 fps = 2 с), звука нет.
//! `cargo test --features testing --test frames_preview_player`.

#![cfg(feature = "testing")]

use std::sync::Arc;
use std::time::Duration;

use syngui::core::{Point, Rect};
use syngui::input::Key;
use syngui::testing::{click_at, press_key, TestHarness};
use syngui::video::VideoFrame;

use synthos::components::video_player::{frames_preview, FramesSource, MediaSource};

fn frames() -> Arc<Vec<Arc<VideoFrame>>> {
    let (w, h) = (32u32, 18u32);
    Arc::new(
        (0..48)
            .map(|i| {
                let shade = (i * 5) as u8;
                let rgba: Vec<u8> = (0..w * h).flat_map(|_| [shade, 64, 200, 255]).collect();
                Arc::new(VideoFrame {
                    width: w,
                    height: h,
                    rgba: rgba.into(),
                    pts_sec: i as f64 / 24.0,
                    seek_generation: 0,
                    surface: None,
                })
            })
            .collect(),
    )
}

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    h.rebuild();
    h.apply_styles(engine);
    h.layout_loose(800.0, 600.0);
}

fn one(h: &TestHarness, class: &str) -> Rect {
    let ids = h.find_by_class(class);
    assert_eq!(ids.len(), 1, "ровно один элемент .{class}");
    h.element_bounds(ids[0])
}

fn right(r: Rect) -> f32 {
    r.origin.x + r.size.width
}

fn center(r: Rect) -> Point {
    Point::new(
        r.origin.x + r.size.width / 2.0,
        r.origin.y + r.size.height / 2.0,
    )
}

#[test]
fn node_preview_fits_and_plays() {
    let frames = frames();
    let mut h = TestHarness::new(frames_preview(&frames, 24.0, None));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);

    let src = FramesSource::find(&frames).expect("источник кадров зарегистрирован");
    assert!(
        Arc::ptr_eq(&src, &FramesSource::shared(&frames, 24.0, None)),
        "пересборка тела ноды получает тот же источник"
    );
    assert!((src.duration() - 2.0).abs() < 1e-6);
    assert!(src.is_paused(), "превью ждёт ⏵");

    // Рамка ноды 360×202, кадр в неё, панель у низа.
    let host = one(&h, "vp-node-preview");
    assert_eq!((host.size.width, host.size.height), (360.0, 202.0));
    let canvas = one(&h, "vp-canvas");
    assert!(
        (canvas.size.width - 360.0).abs() < 0.5 && (canvas.size.height - 202.0).abs() < 0.5,
        "кадр во всю рамку: {canvas:?}"
    );
    let controls = one(&h, "vp-controls-sm");
    assert!(
        (controls.origin.y + controls.size.height - (host.origin.y + 202.0)).abs() < 1.0,
        "панель у низа: {controls:?}"
    );

    // Время и транспорт (−1 ⟲10 ⏯ 10⟳ +1) не наезжают; ⏯ по центру.
    let time = one(&h, "vp-time-sm");
    let play = one(&h, "vp-play-sm");
    let mut btns: Vec<Rect> = h
        .find_by_class("vp-btn-sm")
        .into_iter()
        .map(|id| h.element_bounds(id))
        .collect();
    btns.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    assert_eq!(
        btns.len(),
        4,
        "без звука и «во весь экран» — 4 кнопки: {btns:?}"
    );
    assert!(
        right(time) <= btns[0].origin.x,
        "время {time:?} наезжает на −1 {:?}",
        btns[0]
    );
    assert!(right(btns[3]) <= right(host), "+1 за краем рамки");
    assert!(
        (center(play).x - center(host).x).abs() < 2.0,
        "⏯ по центру: {play:?}"
    );

    // Большая ⏵ — старт часов кадров.
    let big = one(&h, "vp-big-play-sm");
    h.send_events(&click_at(center(big)));
    settle(&mut h, &engine);
    assert!(!src.is_paused(), "⏵ запускает");
    assert!(h.find_by_class("vp-big-play-sm").is_empty());
    std::thread::sleep(Duration::from_millis(120));
    assert!(src.position() > 0.08, "часы идут: {}", src.position());

    // Пробел — пауза; +1 с на паузе.
    h.send_events(&press_key(Key::Space));
    settle(&mut h, &engine);
    assert!(src.is_paused(), "пробел ставит паузу");
    let before = src.position();
    h.send_events(&click_at(center(btns[3])));
    settle(&mut h, &engine);
    // Кнопка шага берёт позицию из сигнала плеера; часы кадров в тесте не
    // тикают (нет цикла кадров), поэтому сигнал отстаёт от источника — от
    // него и считаем.
    let after = src.position();
    assert!(
        after > before + 0.5 || (after - 1.0).abs() < 0.05,
        "+1 с: {before} → {after}"
    );
    assert!(src.is_paused(), "шаг не снимает паузу");
}

#[test]
fn ffmpeg_player_node_uses_shared_player() {
    use syngui::prelude::*;
    use synthos::pages::node_editor::nodes::ffmpeg_player;
    use synthos::pages::node_editor::registry;
    use synthos::pages::node_editor::timing::Stopwatch;
    use synthos::pages::node_editor::types::{
        LtxFrames, NodeId, NodeInstance, NodeKind, NodeRuntime, NodeStyle,
    };

    let kind = NodeKind::FfmpegPlayer;
    let node = NodeInstance {
        id: NodeId(1),
        kind,
        pos: use_signal(Point::new(0.0, 0.0)),
        bounds: use_signal(Rect::zero()),
        fields: registry::default_fields(kind),
        runtime: registry::default_runtime(kind),
        style: use_signal(NodeStyle::default()),
        enabled: use_signal(true),
        timing: Stopwatch::new(),
    };

    let mut h = TestHarness::new(ffmpeg_player::body(&node));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);
    // Ни файла, ни кадров — заглушка без плеера.
    assert!(h.find_by_class("vp").is_empty());
    assert_eq!(h.find_by_class("ffmpeg-player-canvas-empty").len(), 1);

    // Кадры на входе `frames` — плеер на весь холст ноды.
    let frames = frames();
    if let NodeRuntime::FfmpegPlayer {
        frames_in,
        preview_version,
        ..
    } = &*node.runtime.lock().unwrap()
    {
        *frames_in.lock().unwrap() = Some(Arc::new(LtxFrames {
            frames: frames.clone(),
            width: 32,
            height: 18,
            fps: 24.0,
        }));
        preview_version.set(preview_version.get_untracked().wrapping_add(1));
    } else {
        panic!("runtime видеоплеера");
    }
    settle(&mut h, &engine);
    let host = one(&h, "ffmpeg-player-canvas-host");
    let canvas = one(&h, "vp-canvas");
    assert!(
        (canvas.size.width - host.size.width).abs() < 0.5
            && (canvas.size.height - host.size.height).abs() < 0.5,
        "плеер {canvas:?} на весь холст {host:?}"
    );
    assert_eq!(
        h.find_by_class("vp-controls").len(),
        1,
        "полная панель в 640 px"
    );

    // ▶ ноды управляет тем же источником, что и кнопки на кадре.
    let src = FramesSource::find(&frames).expect("источник кадров ноды");
    assert!(src.is_paused());
    let ctx = synthos::pages::node_editor::state::NodeEditorCtx::new();
    ffmpeg_player::on_run(&node, &ctx);
    assert!(!src.is_paused(), "▶ ноды запускает");
    ffmpeg_player::on_run(&node, &ctx);
    assert!(src.is_paused(), "второй ▶ ставит паузу");
}
