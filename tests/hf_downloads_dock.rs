//! Страница HuggingFace: нижняя панель загрузок, вид файлов «список / значки»
//! и чип прогресса в шапке окна.
//!
//! Зачем тест: всё это — раскладка, которую сборка не проверяет. Тулбар панели
//! файлов раньше обрезался по правому краю узкой панели; плитки держатся на
//! `Flex::wrap` и фиксированной ширине из MSS — пропади она, плитки лягут в
//! один столбец или уедут за край; полосы прогресса рисуются инлайн-шириной в
//! процентах, и молча нулевая ширина выглядела бы как «загрузка стоит».
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::path::PathBuf;

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::components::titlebar;
use synthos::config::AppConfig;
use synthos::pages::huggingface::state::{
    DlStatus, DownloadState, FilesViewMode, HfModelDetails, HfSibling, VerifyStatus,
};
use synthos::pages::huggingface::{detail_panel, dock, progress, HuggingFaceCtx};

const REPO: &str = "org/model";
const MB: u64 = 1024 * 1024;

fn settle(h: &mut TestHarness, engine: &syngui::mss::StyleEngine, w: f32, height: f32) {
    syngui::signal::drain_and_run_effects();
    h.rebuild();
    h.apply_styles(engine);
    h.layout(w, height);
}

fn entry(name: &str, done: u64, total: u64, status: DlStatus, speed: f64) -> (String, DownloadState) {
    (
        format!("{REPO}/{name}"),
        DownloadState {
            repo_id: REPO.to_string(),
            filename: name.to_string(),
            bytes_done: done,
            total,
            status,
            dest_path: PathBuf::new(),
            segments: Vec::new(),
            speed_bps: speed,
            speed_sample_at: None,
            speed_sample_bytes: 0,
            retry_count: 0,
            last_meta_flush_at: None,
            expected_sha256: None,
            verify: VerifyStatus::Unknown,
        },
    )
}

fn files() -> Vec<HfSibling> {
    (0..12)
        .map(|i| HfSibling {
            rfilename: format!("FL2VA/audio_vae/shard-{i:02}.safetensors"),
            size: Some(100 * MB),
            lfs: None,
        })
        .collect()
}

/// Контекст с репозиторием из 12 файлов: один скачан, один качается (на 25 %),
/// один в очереди.
fn ctx_with_downloads() -> HuggingFaceCtx {
    syngui::signal::allow_signal_reads_on_this_thread();
    provide_context(HuggingFaceCtx::new(&AppConfig::default()));
    let ctx = use_context::<HuggingFaceCtx>();
    let siblings = files();
    ctx.downloads.update(|m| {
        m.extend([
            entry(&siblings[0].rfilename, 100 * MB, 100 * MB, DlStatus::Done, 0.0),
            entry(&siblings[1].rfilename, 25 * MB, 100 * MB, DlStatus::Active, (10 * MB) as f64),
            entry(&siblings[2].rfilename, 0, 100 * MB, DlStatus::Pending, 0.0),
        ]);
    });
    ctx.model_details.set(Some(HfModelDetails {
        id: REPO.to_string(),
        siblings,
        tags: Vec::new(),
        last_modified: None,
        downloads: 0,
        likes: 0,
        author: None,
        pipeline_tag: None,
    }));
    ctx.selected_model.set(Some(REPO.to_string()));
    ctx
}

fn right_edge(h: &TestHarness, id: syngui::widget::ElementId) -> f32 {
    let b = h.element_bounds(id);
    b.origin.x + b.size.width
}

#[test]
fn dock_summarises_all_downloads_and_expands_into_queue_and_settings() {
    let ctx = ctx_with_downloads();
    let mut h = TestHarness::new(Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow"))
            .child(dock::view()),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine, 1400.0, 800.0);

    // ── Свёрнута: одна строка на всю ширину, прижата к низу ───────────
    let dock_id = h.find_by_class("hf-dock")[0];
    let b = h.element_bounds(dock_id);
    assert_eq!(b.size.width, 1400.0, "панель на всю ширину окна");
    assert!((b.origin.y + b.size.height - 800.0).abs() < 0.5, "панель прижата к низу: {b:?}");
    assert!(b.size.height < 70.0, "свёрнутая панель — одна строка: {}", b.size.height);
    assert!(h.find_by_class("hf-dock-row").is_empty(), "очередь свёрнута");

    // 125 из 300 МБ → заливка ≈ 41,7 % полосы.
    let rail = h.element_bounds(h.find_by_class("hf-dock-rail")[0]).size.width;
    let fill = h.element_bounds(h.find_by_class("hf-dock-rail-fill")[0]).size.width;
    assert!(rail > 300.0, "полоса прогресса занимает свободное место: {rail}");
    assert!((fill / rail - 125.0 / 300.0).abs() < 0.01, "заливка {fill} из {rail}");

    let t = progress::totals(&ctx.downloads.get_untracked());
    assert_eq!(t.percent_text(), "41%");
    assert_eq!(t.eta_secs(), Some(18), "175 МБ при 10 МБ/с");

    // ── Развёрнута: очередь без скачанных + три подписанных SpinBox ───
    ctx.dock_expanded.set(true);
    settle(&mut h, &engine, 1400.0, 800.0);
    assert_eq!(h.find_by_class("hf-dock-row").len(), 2, "качается + в очереди, без Done");
    assert_eq!(h.find_by_class("hf-dock-spin").len(), 3, "файлов / потоков / лимит скорости");
    assert_eq!(
        h.find_by_class("hf-dock-setting-label").len(),
        5,
        "у каждой настройки есть название"
    );
    let rows = h.find_by_class("hf-dock-row");
    let first = h.element_bounds(rows[0]);
    let second = h.element_bounds(rows[1]);
    assert!(first.origin.y < second.origin.y, "активная загрузка выше ожидающей");
    let settings = h.element_bounds(h.find_by_class("hf-dock-settings")[0]);
    assert!(right_edge(&h, rows[0]) <= settings.origin.x + 0.5, "очередь не заходит под настройки");
    let b = h.element_bounds(dock_id);
    assert!((b.origin.y + b.size.height - 800.0).abs() < 0.5, "развёрнутая панель тоже у низа");

    // ── Всё докачано — сводка пустеет ─────────────────────────────────
    ctx.downloads.update(|m| {
        for d in m.values_mut() {
            d.status = DlStatus::Done;
            d.bytes_done = d.total;
        }
    });
    settle(&mut h, &engine, 1400.0, 800.0);
    assert!(h.find_by_class("hf-dock-rail").is_empty(), "полосы нет, когда качать нечего");
    assert!(h.find_by_class("hf-dock-row").is_empty());
}

#[test]
fn queue_order_is_active_then_attention_then_fifo() {
    let map: std::collections::HashMap<_, _> = [
        entry("c", 0, 10, DlStatus::Pending, 0.0),
        entry("b", 0, 10, DlStatus::Pending, 0.0),
        entry("p", 5, 10, DlStatus::Paused, 0.0),
        entry("a", 5, 10, DlStatus::Active, 1.0),
        entry("d", 10, 10, DlStatus::Done, 0.0),
    ]
    .into_iter()
    .collect();
    let fifo = vec![format!("{REPO}/b"), format!("{REPO}/c")];
    let (rows, hidden) = dock::queue_rows(&map, &fifo);
    let names: Vec<&str> = rows.iter().map(|d| d.filename.as_str()).collect();
    assert_eq!(names, ["a", "p", "b", "c"]);
    assert_eq!(hidden, 0);
}

#[test]
fn files_panel_switches_between_list_and_icon_grid() {
    let ctx = ctx_with_downloads();
    const W: f32 = 560.0;
    let mut h = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(detail_panel::files_view()),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine, W, 900.0);

    // ── Тулбар помещается в узкую панель (раньше обрезался справа) ────
    let toolbar = h.find_by_class("hf-files-toolbar")[0];
    for class in ["hf-toolbar-dl-all-btn", "hf-toolbar-dl-sel-btn", "hf-toolbar-stat-text", "hf-view-mode"] {
        let id = h.find_by_class(class)[0];
        assert!(
            right_edge(&h, id) <= right_edge(&h, toolbar) + 0.5,
            "{class} вылез за тулбар: {:?}",
            h.element_bounds(id)
        );
    }

    // ── Список: строка на файл ────────────────────────────────────────
    assert_eq!(h.find_by_class("hf-file-row").len(), 12);
    assert!(h.find_by_class("hf-file-tile").is_empty());

    // ── Значки: плитки одной ширины, в несколько колонок и рядов ──────
    ctx.files_view_mode.set(FilesViewMode::Icons);
    settle(&mut h, &engine, W, 900.0);
    assert!(h.find_by_class("hf-file-row").is_empty());
    let tiles = h.find_by_class("hf-file-tile");
    assert_eq!(tiles.len(), 12);
    let bounds: Vec<_> = tiles.iter().map(|&t| h.element_bounds(t)).collect();
    let first_row = bounds.iter().filter(|b| (b.origin.y - bounds[0].origin.y).abs() < 0.5).count();
    assert_eq!(first_row, 3, "при ширине {W} в ряд встаёт три плитки по 148 px");
    for b in &bounds {
        assert_eq!(b.size.width, 148.0, "ширина плитки из MSS");
        assert!(b.origin.x + b.size.width <= W, "плитка за краем панели: {b:?}");
    }
    // Шире панель — больше колонок: число следует за разделителем.
    settle(&mut h, &engine, 900.0, 900.0);
    let tiles = h.find_by_class("hf-file-tile");
    let y0 = h.element_bounds(tiles[0]).origin.y;
    let first_row = tiles.iter().filter(|&&t| (h.element_bounds(t).origin.y - y0).abs() < 0.5).count();
    assert_eq!(first_row, 5);

    // Прогресс качающегося файла виден и на плитке.
    assert_eq!(h.find_by_class("hf-progress-bar").len(), 1);
    assert_eq!(FilesViewMode::from_config(ctx.files_view_mode.get_untracked().as_config()), FilesViewMode::Icons);
}

#[test]
fn titlebar_shows_download_progress_before_link_chips() {
    let ctx = ctx_with_downloads();
    let mut h = TestHarness::new(Box::new(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(titlebar::middle(false, 0.0)),
        ),
    ));
    let engine = h.apply_mss(synthos::styles::styles());
    settle(&mut h, &engine, 1200.0, 32.0);

    let chip = h.find_by_class("titlebar-dl");
    assert_eq!(chip.len(), 1, "чип прогресса в шапке");
    let chip_b = h.element_bounds(chip[0]);
    assert_eq!(chip_b.size.height, 20.0, "та же высота, что у Donate/GitHub");
    let donate = h.element_bounds(h.find_by_class("titlebar-chip-donate")[0]);
    assert!(chip_b.origin.x + chip_b.size.width <= donate.origin.x, "прогресс левее Donate");
    let rail = h.element_bounds(h.find_by_class("titlebar-dl-rail")[0]).size.width;
    let fill = h.element_bounds(h.find_by_class("titlebar-dl-fill")[0]).size.width;
    assert!((fill / rail - 125.0 / 300.0).abs() < 0.02, "заливка {fill} из {rail}");

    // Качать нечего — чипа нет, Donate/GitHub на месте.
    ctx.downloads.update(|m| m.clear());
    settle(&mut h, &engine, 1200.0, 32.0);
    assert!(h.find_by_class("titlebar-dl").is_empty());
    assert_eq!(h.find_by_class("titlebar-chip").len(), 2);
}
