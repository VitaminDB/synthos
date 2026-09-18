//! Плеер приложения (`components::video_player`): кадр во всю сцену,
//! панель у нижнего края поверх кадра, перемотка во всю ширину, ⏯ по центру,
//! ±1 с по краям транспорта; пауза пробелом, кликом по кадру и большой ⏵;
//! дорожка ползунка по центру его кружка.
//!
//! Зачем тест: прежний просмотрщик ставил плеер syngui холстом 1080×620 в
//! сцену 1248×680 — кадр жался к левому краю, справа и снизу оставались
//! пустые полосы, а ползунок перемотки без ширины не было видно вовсе.
//!
//! Видео — `tests/fixtures/silent_160x120_20s.mp4` (`testsrc` 20 с без
//! звука): плеер стартует сразу, не пищит в колонки и не доходит до конца,
//! пока идёт тест. Нужен `TestHarness`:
//! `cargo test --features testing --test video_player_layout`.

#![cfg(feature = "testing")]

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::{Point, Rect};
use syngui::input::Key;
use syngui::render::DrawCommand;
use syngui::testing::{click_at, press_key, TestHarness};
use syngui::video::VideoPlayer;

use synthos::components::video_player::VideoPlayerView;

const W: f32 = 1200.0;
const H: f32 = 680.0;

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    h.rebuild();
    h.apply_styles(engine);
    h.layout(W, H);
}

fn one(h: &TestHarness, class: &str) -> Rect {
    let ids = h.find_by_class(class);
    assert_eq!(ids.len(), 1, "ровно один элемент .{class}");
    h.element_bounds(ids[0])
}

fn bottom(r: Rect) -> f32 {
    r.origin.y + r.size.height
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

fn paused(p: &Arc<Mutex<VideoPlayer>>) -> bool {
    p.lock().map(|p| p.is_paused()).unwrap()
}

#[test]
fn player_fills_stage_and_controls_work() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/silent_160x120_20s.mp4");
    let player = Arc::new(Mutex::new(
        VideoPlayer::open(fixture.to_str().unwrap()).expect("фикстура открывается"),
    ));

    let mut h = TestHarness::new(Box::new(VideoPlayerView::file(player.clone()).build()));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine);

    // Кадр — во всю сцену.
    let canvas = one(&h, "vp-canvas");
    assert_eq!(
        (
            canvas.origin.x,
            canvas.origin.y,
            canvas.size.width,
            canvas.size.height
        ),
        (0.0, 0.0, W, H),
        "холст во всю сцену"
    );

    // Панель — поверх кадра у нижнего края, во всю ширину, не выше разумного.
    let controls = one(&h, "vp-controls");
    assert!(
        (bottom(controls) - H).abs() < 1.0,
        "панель у низа: {controls:?}"
    );
    assert!(
        (controls.size.width - W).abs() < 1.0,
        "панель во всю ширину: {controls:?}"
    );
    assert!(
        controls.size.height < 170.0,
        "панель не съедает кадр: {controls:?}"
    );

    // Перемотка — во всю ширину панели (минус её поля).
    let seek = one(&h, "vp-seek");
    assert!(
        seek.size.width >= W - 2.0 * 18.0 - 1.0,
        "ползунок перемотки во всю ширину: {seek:?}"
    );
    assert!(seek.origin.y >= controls.origin.y && bottom(seek) <= bottom(controls));

    // ⏯ строго по центру, время слева, всё — в панели. Звука у фикстуры нет —
    // нет и кнопки звука с громкостью.
    let play = one(&h, "vp-play");
    assert!(
        (center(play).x - W / 2.0).abs() < 2.0,
        "⏯ по центру: {play:?}"
    );
    assert!(play.origin.y > bottom(seek), "⏯ под ползунком");
    let time = one(&h, "vp-time");
    assert!(time.origin.x < 40.0, "время слева: {time:?}");
    for (name, r) in [("⏯", play), ("время", time)] {
        assert!(
            bottom(r) <= H + 0.5 && r.origin.y >= controls.origin.y,
            "{name} {r:?} вне панели"
        );
    }
    assert!(
        h.find_by_class("vp-volume").is_empty(),
        "без звука нет громкости"
    );

    // Транспорт слева направо: −1, ⟲10, ⏯, 10⟳, +1 — шаги по секунде по краям.
    let mut btns: Vec<Rect> = h
        .find_by_class("vp-btn")
        .into_iter()
        .map(|id| h.element_bounds(id))
        .collect();
    btns.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    assert_eq!(btns.len(), 4, "−1, ⟲10, 10⟳, +1: {btns:?}");
    assert!(right(btns[1]) <= play.origin.x && right(play) <= btns[2].origin.x);
    let steps: Vec<Rect> = h
        .find_by_class("vp-step")
        .into_iter()
        .map(|id| h.element_bounds(id))
        .collect();
    assert_eq!(steps.len(), 2, "подписи −1 и +1");
    for step in &steps {
        assert!(
            btns.iter()
                .any(|b| (b.origin.x - step.origin.x).abs() < 0.5),
            "подпись {step:?} под кнопкой"
        );
    }
    assert!(
        steps
            .iter()
            .any(|s| (s.origin.x - btns[0].origin.x).abs() < 0.5)
            && steps
                .iter()
                .any(|s| (s.origin.x - btns[3].origin.x).abs() < 0.5),
        "±1 — крайние кнопки"
    );

    // Дорожка ползунка по центру его кружка (раньше в syngui она стояла на
    // `y + 10` при любой высоте и уезжала на 3 px вниз).
    let list = h.paint();
    let seek_mid = seek.origin.y + seek.size.height / 2.0;
    let track = list
        .iter_all_commands()
        .filter_map(|c| match c {
            DrawCommand::Rect { rect, .. } => Some(*rect),
            _ => None,
        })
        .find(|r| {
            (r.size.width - (seek.size.width - 16.0)).abs() < 0.5
                && r.size.height < 6.0
                && r.origin.y >= seek.origin.y
                && r.origin.y <= bottom(seek)
        })
        .expect("дорожка перемотки в display list");
    assert!(
        (track.origin.y + track.size.height / 2.0 - seek_mid).abs() < 0.3,
        "дорожка {track:?} не по центру ползунка {seek:?}"
    );

    // Играет — большой ⏵ нет. Пробел — пауза и большой ⏵ по центру кадра.
    assert!(!paused(&player), "фикстура стартует сразу");
    assert!(h.find_by_class("vp-big-play").is_empty());
    h.send_events(&press_key(Key::Space));
    settle(&mut h, &engine);
    assert!(paused(&player), "пробел ставит паузу");
    let big = one(&h, "vp-big-play");
    let c = center(big);
    assert!(
        (c.x - W / 2.0).abs() < 2.0 && (c.y - H / 2.0).abs() < 2.0,
        "⏵ по центру: {big:?}"
    );

    // Клик по большой ⏵ — снова играет, кнопка уходит.
    h.send_events(&click_at(c));
    settle(&mut h, &engine);
    assert!(!paused(&player), "клик по ⏵ запускает");
    assert!(h.find_by_class("vp-big-play").is_empty());

    // Клик по кадру над панелью — пауза.
    h.send_events(&click_at(Point::new(W * 0.25, H * 0.3)));
    settle(&mut h, &engine);
    assert!(paused(&player), "клик по кадру ставит паузу");

    // Клик по ⏯ на панели — играет.
    let play = one(&h, "vp-play");
    h.send_events(&click_at(center(play)));
    settle(&mut h, &engine);
    assert!(!paused(&player), "⏯ запускает");

    // Пустое место панели (между временем и транспортом) — не кадр: паузы нет.
    let controls = one(&h, "vp-controls");
    h.send_events(&click_at(Point::new(W * 0.3, bottom(controls) - 20.0)));
    settle(&mut h, &engine);
    assert!(!paused(&player), "клик по фону панели не ставит паузу");

    // ±1 с: на паузе +1 ставит на секунду вперёд, −1 — обратно.
    h.send_events(&press_key(Key::Space));
    settle(&mut h, &engine);
    assert!(paused(&player));
    let plus = btns[3];
    h.send_events(&click_at(center(plus)));
    settle(&mut h, &engine);
    let pos = |p: &Arc<Mutex<VideoPlayer>>| p.lock().map(|p| p.position_sec()).unwrap();
    assert!((pos(&player) - 1.0).abs() < 0.15, "+1 с: {}", pos(&player));
    let minus = btns[0];
    h.send_events(&click_at(center(minus)));
    settle(&mut h, &engine);
    // Вторая перемотка подряд придержана троттлингом — её доводит тикер.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(pos(&player) < 0.15, "−1 с: {}", pos(&player));
}
