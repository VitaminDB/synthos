//! Нижняя панель загрузок — на всю ширину страницы, под трёхпанельным
//! каркасом.
//!
//! ```text
//! .hf-dock
//!   summary  [˄] Загрузки  ████████░░░ 23%          [3 файла × 4 потока · без лимита] [⏸ Пауза]
//!                106 ГБ из 464 ГБ · 30 МБ/с · осталось 3 ч · 67/280 · активных 4 · очередь 10
//!   body (если развёрнута)
//!     очередь по всем репозиториям        │ настройки с подписями
//! ```
//!
//! Зачем отдельная панель: сводка и настройки раньше жили в тулбаре панели
//! файлов — одна строка из двенадцати элементов, два `SpinBox` без подписей, а
//! чтобы увидеть статус, приходилось сужать соседние панели. Здесь сводка видна
//! при любой раскладке и для всех репозиториев сразу, а у каждой настройки есть
//! название и пояснение.

use syngui::mgui;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::containers::IntoWidget;
use syngui::widgets::input::{SpinBox, Toggle};

use crate::config;
use crate::context::AppCtx;
use crate::icons::{
    MI_CLOUD_DOWNLOAD, MI_EXPAND_LESS, MI_EXPAND_MORE, MI_FOLDER_OPEN, MI_PAUSE, MI_PLAY_ARROW,
    MI_TUNE,
};

use super::detail_panel::{icon_actions, progress_bar, status_indicator};
use super::download;
use super::progress::{self, Totals};
use super::state::{DlStatus, DownloadState, HuggingFaceCtx};

/// Сколько ожидающих файлов показывать в очереди: «Скачать всё» на большом
/// репозитории даёт сотни Pending, а панель перестраивается на каждый тик
/// прогресса. Остаток — строкой «ещё N в очереди».
const MAX_PENDING_ROWS: usize = 40;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("hf-dock").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                summary_bar(),
                Reactive::new(|| -> Vec<Box<dyn Widget>> {
                    let ctx = use_context::<HuggingFaceCtx>();
                    if !ctx.dock_expanded.get() {
                        return vec![Box::new(DecoratedBox::new().class("hf-dock-body-empty"))];
                    }
                    vec![Box::new(body())]
                }),
            ]
    })
}

// ───────────────────────────── Сводка ─────────────────────────────

fn summary_bar() -> impl Widget {
    DecoratedBox::new().class("hf-dock-summary").child(mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                expand_button(),
                Icon::new(MI_CLOUD_DOWNLOAD).class("hf-dock-title-icon"),
                Text::new(tr!("hf.dock.title")).max_lines(1).class("hf-dock-title"),
                DecoratedBox::new().class("grow").child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
                    let ctx = use_context::<HuggingFaceCtx>();
                    let t = progress::totals(&ctx.downloads.get());
                    vec![Box::new(summary_progress(&t, ctx.global_paused.get()))]
                })),
                settings_chip(),
                pause_all_button(),
            ]
    })
}

fn expand_button() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let open = ctx.dock_expanded.get();
        let (icon, tip) = if open {
            (MI_EXPAND_MORE, tr!("hf.dock.collapse"))
        } else {
            (MI_EXPAND_LESS, tr!("hf.dock.expand"))
        };
        vec![Box::new(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || {
                    let ctx = use_context::<HuggingFaceCtx>();
                    ctx.dock_expanded.set(!ctx.dock_expanded.get_untracked());
                })
                .class("hf-icon-btn"),
        )]
    })
}

/// Полоса общего прогресса и строка цифр под ней.
fn summary_progress(t: &Totals, paused: bool) -> impl Widget {
    if !t.has_unfinished() {
        return Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Text::new(tr!("hf.dock.idle")).max_lines(1).class("hf-dock-stats"));
    }
    let percent = t.ratio().unwrap_or(0.0) * 100.0;
    let rail_class = if paused || !t.in_flight() {
        "hf-dock-rail hf-dock-rail--idle"
    } else {
        "hf-dock-rail"
    };
    let rail = DecoratedBox::new().class(rail_class).child(
        DecoratedBox::new()
            .class("hf-dock-rail-fill")
            .style("width", StyleValue::percent(percent)),
    );
    Column::new()
        .gap(5.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(rail))
                .child(Text::new(t.percent_text()).max_lines(1).class("hf-dock-percent")),
        )
        .child(Text::new(stats_line(t)).max_lines(1).class("hf-dock-stats"))
}

/// «106 ГБ из 464 ГБ · 30 МБ/с · осталось 3 ч 15 мин · 67/280 · активных 4 · …».
pub fn stats_line(t: &Totals) -> String {
    let mut parts: Vec<String> = Vec::new();
    if t.total_bytes > 0 {
        parts.push(tr!(
            "hf.dock.bytes_of",
            done = progress::human_bytes(t.done_bytes),
            total = progress::human_bytes(t.total_bytes)
        ));
    }
    if t.active > 0 && t.speed_bps > 1.0 {
        parts.push(progress::human_speed(t.speed_bps));
        if let Some(eta) = t.eta_secs() {
            parts.push(tr!("hf.dock.eta", time = progress::human_eta(eta)));
        }
    }
    parts.push(tr!("hf.dock.files", done = t.files_done, total = t.files_total));
    if t.active > 0 {
        parts.push(tr!("hf.dock.active", n = t.active));
    }
    if t.pending > 0 {
        parts.push(tr!("hf.dock.pending", n = t.pending));
    }
    if t.paused > 0 {
        parts.push(tr!("hf.dock.paused", n = t.paused));
    }
    if t.errors > 0 {
        parts.push(tr!("hf.dock.errors", n = t.errors));
    }
    parts.join(" · ")
}

/// Текущие настройки одной строкой; щелчок разворачивает панель, где их
/// можно поменять. В свёрнутом виде сразу видно, чем ограничена загрузка.
fn settings_chip() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let limit = ctx.speed_limit_mbps.get();
        let speed = if limit == 0 {
            tr!("hf.dock.chip.no_limit")
        } else {
            tr!("hf.dock.chip.limit", n = limit)
        };
        let label = tr!(
            "hf.dock.chip",
            files = ctx.concurrent_limit.get(),
            segments = ctx.segments_per_file.get(),
            speed = speed
        );
        vec![Box::new(
            Tooltip::new(
                Button::new(label)
                    .leading_icon(MI_TUNE)
                    .on_click(|| {
                        let ctx = use_context::<HuggingFaceCtx>();
                        ctx.dock_expanded.set(!ctx.dock_expanded.get_untracked());
                    })
                    .class("hf-dock-chip"),
                tr!("hf.dock.chip.tooltip"),
            ),
        )]
    })
}

fn pause_all_button() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let paused = ctx.global_paused.get();
        let btn = if paused {
            Button::new(tr!("hf.dock.resume_all"))
                .leading_icon(MI_PLAY_ARROW)
                .on_click(|| {
                    let ctx = use_context::<HuggingFaceCtx>();
                    let app = use_context::<AppCtx>();
                    download::resume_all(ctx, app.notifications.clone());
                })
                .class("hf-dock-pause-all paused")
        } else {
            Button::new(tr!("hf.dock.pause_all"))
                .leading_icon(MI_PAUSE)
                .on_click(|| download::pause_all(use_context::<HuggingFaceCtx>()))
                .class("hf-dock-pause-all")
        };
        vec![Box::new(btn)]
    })
}

// ───────────────────────────── Тело ─────────────────────────────

fn body() -> impl Widget {
    DecoratedBox::new().class("hf-dock-body").child(mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("hf-dock-queue grow").child(queue()),
                DecoratedBox::new().class("hf-dock-settings").child(settings()),
            ]
    })
}

/// Порядок строк очереди: что качается → что прервано или упало (требует
/// внимания) → что ждёт, в порядке FIFO.
fn status_rank(s: &DlStatus) -> u8 {
    match s {
        DlStatus::Active => 0,
        DlStatus::Error(_) => 1,
        DlStatus::Paused | DlStatus::Stopped => 2,
        DlStatus::Pending => 3,
        DlStatus::Done => 4,
    }
}

/// Незавершённые загрузки в порядке показа и число не поместившихся ожидающих.
pub fn queue_rows(
    downloads: &std::collections::HashMap<String, DownloadState>,
    fifo: &[String],
) -> (Vec<DownloadState>, usize) {
    let mut rows: Vec<&DownloadState> = downloads
        .values()
        .filter(|d| !matches!(d.status, DlStatus::Done))
        .collect();
    let fifo_pos = |d: &DownloadState| {
        let key = format!("{}/{}", d.repo_id, d.filename);
        fifo.iter().position(|k| *k == key).unwrap_or(usize::MAX)
    };
    rows.sort_by(|a, b| {
        status_rank(&a.status)
            .cmp(&status_rank(&b.status))
            .then_with(|| fifo_pos(a).cmp(&fifo_pos(b)))
            .then_with(|| (&a.repo_id, &a.filename).cmp(&(&b.repo_id, &b.filename)))
    });
    let head = rows.iter().filter(|d| !matches!(d.status, DlStatus::Pending)).count();
    let keep = head + MAX_PENDING_ROWS;
    let hidden = rows.len().saturating_sub(keep);
    (rows.into_iter().take(keep).cloned().collect(), hidden)
}

fn queue() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let (rows, hidden) = queue_rows(&ctx.downloads.get(), &ctx.download_queue.get());
        if rows.is_empty() {
            return vec![Box::new(Center::new().child(
                Text::new(tr!("hf.dock.queue.empty")).class("hf-dock-empty"),
            ))];
        }
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for d in rows {
            col = col.child(queue_row(d));
        }
        if hidden > 0 {
            col = col.child(Text::new(tr!("hf.dock.queue.more", n = hidden)).class("hf-dock-more"));
        }
        vec![Box::new(ScrollView::new().vertical().class("hf-dock-scroll").child(col))]
    })
}

fn queue_row(d: DownloadState) -> impl Widget {
    let size = if d.total > 0 {
        tr!(
            "hf.dock.bytes_of",
            done = progress::human_bytes(d.bytes_done.min(d.total)),
            total = progress::human_bytes(d.total)
        )
    } else {
        "—".to_string()
    };
    let speed = if matches!(d.status, DlStatus::Active) && d.speed_bps > 1.0 {
        progress::human_speed(d.speed_bps)
    } else {
        String::new()
    };
    let repo = d.repo_id.clone();
    let actions = icon_actions(
        d.repo_id.clone(),
        d.filename.clone(),
        d.expected_sha256.clone(),
        None,
        Some(d.status.clone()),
    );
    DecoratedBox::new().class("hf-dock-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                status_indicator(Some(&d)),
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Text::new(d.filename.clone()).max_lines(1).class("hf-dock-row-name"))
                                .child(
                                    // Щелчок по репозиторию открывает его файлы
                                    // в правой панели.
                                    Button::new(repo.clone())
                                        .on_click(move || super::actions::select_model(repo.clone()))
                                        .class("hf-dock-row-repo"),
                                )
                                .child(DecoratedBox::new().class("grow"))
                                .child(Text::new(speed).max_lines(1).class("hf-file-speed"))
                                .child(Text::new(size).max_lines(1).class("hf-dock-row-size")),
                        )
                        .child(progress_bar(Some(&d))),
                ),
                actions,
            ]
    })
}

// ───────────────────────────── Настройки ─────────────────────────────

fn settings() -> impl Widget {
    let col = mgui! {
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("hf.dock.settings.title")).class("hf-dock-settings-title"),
                setting_row(
                    tr!("hf.dock.settings.concurrent"),
                    tr!("hf.dock.settings.concurrent.hint"),
                    concurrent_spin(),
                ),
                setting_row(
                    tr!("hf.dock.settings.segments"),
                    tr!("hf.dock.settings.segments.hint"),
                    segments_spin(),
                ),
                setting_row(
                    tr!("hf.dock.settings.speed"),
                    tr!("hf.dock.settings.speed.hint"),
                    speed_spin(),
                ),
                setting_row(
                    tr!("hf.dock.settings.skip"),
                    tr!("hf.dock.settings.skip.hint"),
                    skip_toggle(),
                ),
                folder_row(),
            ]
    };
    ScrollView::new().vertical().class("hf-dock-scroll").child(col)
}

/// Название и пояснение слева, контрол справа.
fn setting_row<M>(label: String, hint: String, control: impl IntoWidget<M>) -> impl Widget {
    Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(1.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(label).class("hf-dock-setting-label"))
                    .child(Text::new(hint).class("hf-dock-setting-hint")),
            ),
        )
        .child(control)
}

fn concurrent_spin() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let spin = SpinBox::new()
            .value(ctx.concurrent_limit.get() as f64)
            .range(1.0, 8.0)
            .step(1.0)
            .decimal_places(0)
            .width(96.0)
            .on_change(|v| {
                let ctx = use_context::<HuggingFaceCtx>();
                let v = v.round().clamp(1.0, 8.0) as u32;
                if ctx.concurrent_limit.get_untracked() != v {
                    ctx.concurrent_limit.set(v);
                    // Лимит подняли — очередь может тут же занять новые слоты.
                    let app = use_context::<AppCtx>();
                    download::try_drain_queue(ctx, app.notifications.clone());
                }
            })
            .class("hf-dock-spin");
        vec![Box::new(spin)]
    })
}

fn segments_spin() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let spin = SpinBox::new()
            .value(ctx.segments_per_file.get() as f64)
            .range(1.0, 16.0)
            .step(1.0)
            .decimal_places(0)
            .width(96.0)
            .on_change(|v| {
                let ctx = use_context::<HuggingFaceCtx>();
                let v = v.round().clamp(1.0, 16.0) as u32;
                if ctx.segments_per_file.get_untracked() != v {
                    ctx.segments_per_file.set(v);
                }
            })
            .class("hf-dock-spin");
        vec![Box::new(spin)]
    })
}

fn speed_spin() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let spin = SpinBox::new()
            .value(ctx.speed_limit_mbps.get() as f64)
            .range(0.0, 2000.0)
            .step(1.0)
            .decimal_places(0)
            .width(96.0)
            .on_change(|v| {
                let ctx = use_context::<HuggingFaceCtx>();
                let v = v.round().clamp(0.0, 2000.0) as u32;
                if ctx.speed_limit_mbps.get_untracked() != v {
                    ctx.speed_limit_mbps.set(v);
                }
            })
            .class("hf-dock-spin");
        vec![Box::new(spin)]
    })
}

fn skip_toggle() -> impl Widget {
    // Reactive: тумблер есть и в «Настройках» — правка оттуда отражается здесь.
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        vec![Box::new(Toggle::with_state(ctx.skip_unwanted_formats.get()).on_change(|v| {
            use_context::<HuggingFaceCtx>().skip_unwanted_formats.set(v);
        }))]
    })
}

fn folder_row() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let path = config::resolve_hf_cache_dir(&ctx.cache_dir.get()).display().to_string();
        vec![Box::new(setting_row(
            tr!("hf.dock.settings.folder"),
            path,
            Button::new(tr!("hf.dock.settings.folder.change"))
                .leading_icon(MI_FOLDER_OPEN)
                .on_click(super::dialogs::pick_folder)
                .class("hf-dock-chip"),
        ))]
    })
}
