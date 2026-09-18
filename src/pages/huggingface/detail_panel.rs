//! Детали выбранной модели: центр страницы — шапка репозитория + README
//! ([`view`]), правая панель — список файлов с загрузками ([`files_view`]).
//!
//! Подписаны на `selected_model`, `model_details`, `readme_*`, `downloads`.

use syngui::mgui;
use syngui::mss::{MssColor, StyleValue};
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::input::Checkbox;
use syngui::widgets::visual::MarkdownView;

use crate::context::AppCtx;
use crate::icons::{
    MI_AUTO_AWESOME, MI_CHECK, MI_CLOSE, MI_CLOUD_DOWNLOAD, MI_CODE, MI_DATA_OBJECT,
    MI_DESCRIPTION, MI_DOWNLOAD, MI_GRID_VIEW, MI_HOURGLASS_TOP, MI_IMAGE,
    MI_INSERT_DRIVE_FILE, MI_MEMORY, MI_MOVIE, MI_PAUSE, MI_PLAY_ARROW, MI_REPORT, MI_STOP,
    MI_VERIFIED_USER, MI_VIEW_LIST,
};

use super::download;
use super::progress::{human_bytes, human_speed};
use super::state::{
    DlStatus, DownloadState, FilesViewMode, HfModelDetails, HfSibling, HuggingFaceCtx,
    VerifyStatus,
};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("hf-detail-panel").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            let Some(repo_id) = ctx.selected_model.get() else {
                return vec![Box::new(placeholder_view())];
            };
            vec![Box::new(detail_body(repo_id))]
        },
    ))
}

fn placeholder_view() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("hf-detail-placeholder") => [
                Text::new(tr!("hf.detail.placeholder.title")).class("hf-detail-placeholder-title"),
                Text::new(tr!("hf.detail.placeholder.hint"))
                    .class("hf-detail-placeholder-hint"),
            ]
    })
}

fn detail_body(repo_id: String) -> impl Widget {
    mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("hf-detail-header").child(
                    detail_header_row(repo_id)
                ),
                DecoratedBox::new().class("hf-detail-content grow").child(readme_tab()),
            ]
    }
}

/// Правая панель: файлы репозитория и их загрузки.
pub fn files_view() -> impl Widget {
    DecoratedBox::new().class("hf-detail-panel hf-files-panel").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            let Some(repo_id) = ctx.selected_model.get() else {
                return vec![Box::new(files_placeholder())];
            };
            vec![Box::new(
                DecoratedBox::new().class("hf-detail-content").child(files_tab(repo_id)),
            )]
        },
    ))
}

fn files_placeholder() -> impl Widget {
    Center::new().child(
        Text::new(tr!("hf.detail.files.placeholder")).class("hf-detail-placeholder-hint"),
    )
}

/// Заголовок detail-панели: аватар автора + repo_id (заголовок) + downloads/likes.
fn detail_header_row(repo_id: String) -> impl Widget {
    let author = repo_id.split('/').next().unwrap_or("").to_string();
    let avatar = avatar_widget(&author);
    mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                avatar,
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Text::new(repo_id.clone()).class("hf-detail-title"),
                        Reactive::new(|| -> Vec<Box<dyn Widget>> {
                            let ctx = use_context::<HuggingFaceCtx>();
                            let d = ctx.model_details.get();
                            let line = match d {
                                Some(d) => tr!(
                                    "hf.detail.stats_line",
                                    downloads = human_count(d.downloads),
                                    likes = human_count(d.likes),
                                    date = d.last_modified
                                        .as_deref()
                                        .map(|s| s.chars().take(10).collect::<String>())
                                        .unwrap_or_else(|| "—".to_string())
                                ),
                                None => tr!("hf.detail.loading_ellipsis"),
                            };
                            vec![Box::new(Text::new(line).class("hf-detail-subtitle"))]
                        }),
                    ],
            ]
    }
}

/// Letter-avatar 32×32 для detail-header'а. Цвет детерминирован хэшем имени.
/// HF не отдаёт публичные avatar-URL без auth (401), поэтому рисуем сами.
fn avatar_widget(author: &str) -> impl Widget {
    let author = author.to_string();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if author.is_empty() {
            return vec![Box::new(
                DecoratedBox::new().class("hf-detail-avatar-fallback").child(
                    Icon::new(MI_AUTO_AWESOME).class("hf-detail-avatar-icon"),
                ),
            )];
        }
        let (letter, bg) = letter_and_color(&author);
        let badge = DecoratedBox::new()
            .class("hf-detail-avatar-letter")
            .style("background-color", StyleValue::Color(bg))
            .child(Center::new().child(
                Text::new(letter).class("hf-detail-avatar-letter-text"),
            ));
        vec![Box::new(badge)]
    })
}

fn letter_and_color(name: &str) -> (String, MssColor) {
    let first = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('?');
    const PALETTE: &[MssColor] = &[
        MssColor::rgb(0x3B, 0x82, 0xF6),
        MssColor::rgb(0x10, 0xB9, 0x81),
        MssColor::rgb(0xF5, 0x9E, 0x0B),
        MssColor::rgb(0xEF, 0x44, 0x44),
        MssColor::rgb(0x8B, 0x5C, 0xF6),
        MssColor::rgb(0xEC, 0x48, 0x99),
        MssColor::rgb(0x14, 0xB8, 0xA6),
        MssColor::rgb(0xF9, 0x73, 0x16),
        MssColor::rgb(0x84, 0xCC, 0x16),
        MssColor::rgb(0x06, 0xB6, 0xD4),
        MssColor::rgb(0xA8, 0x55, 0xF7),
        MssColor::rgb(0x64, 0x74, 0x8B),
    ];
    let h: u32 = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    (first.to_string(), PALETTE[(h as usize) % PALETTE.len()])
}

fn human_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn readme_tab() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        if ctx.readme_loading.get() {
            return vec![Box::new(
                Center::new().child(Text::new(tr!("hf.detail.readme.loading")).class("hf-readme-loading")),
            )];
        }
        let txt = ctx.readme_text.get();
        if txt.trim().is_empty() {
            return vec![Box::new(
                Center::new().child(Text::new(tr!("hf.detail.readme.missing")).class("hf-readme-empty")),
            )];
        }
        // Класс `hf-md` ставим на сам MarkdownView: его apply_computed_style
        // читает `--md-*` только из своего собственного ComputedStyle, custom
        // properties от родителя не каскадятся вниз (syngui quirk).
        let md = MarkdownView::new(txt)
            .with_copy_code(true)
            .with_syntax_highlight(true)
            .class("hf-md");
        let scroll = ScrollView::new()
            .vertical()
            .class("hf-readme-scroll")
            .child(DecoratedBox::new().class("hf-readme").child(md));
        vec![Box::new(scroll)]
    })
}

fn files_tab(repo_id: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let details = ctx.model_details.get();
        let downloads = ctx.downloads.get();
        let rid = repo_id.clone();

        let Some(d) = details else {
            return vec![Box::new(
                Center::new().child(Text::new(tr!("hf.detail.files.loading")).class("hf-files-loading")),
            )];
        };
        if d.siblings.is_empty() {
            return vec![Box::new(
                Center::new()
                    .child(Text::new(tr!("hf.detail.files.empty")).class("hf-files-empty")),
            )];
        }
        let toolbar = files_toolbar(rid.clone(), d.siblings.clone());
        let rows = d.siblings.iter().map(|sibling| {
            let key = format!("{}/{}", rid, sibling.rfilename);
            (sibling.clone(), downloads.get(&key).cloned())
        });
        let scroll: Box<dyn Widget> = match ctx.files_view_mode.get() {
            FilesViewMode::List => {
                let mut col = Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch);
                for (sibling, dl) in rows {
                    col = col.child(file_row(rid.clone(), sibling, dl));
                }
                Box::new(files_scroll(col))
            }
            // Плитки фиксированной ширины с переносом: число колонок следует
            // за шириной панели, которую пользователь двигает разделителем.
            FilesViewMode::Icons => Box::new(files_scroll(
                Flex::new()
                    .direction(FlexDirection::Row)
                    .wrap()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(rows.map(|(sibling, dl)| {
                        Box::new(file_tile(rid.clone(), sibling, dl)) as Box<dyn Widget>
                    })),
            )),
        };
        let body = mgui! {
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    toolbar,
                    DecoratedBox::new().class("grow").child(Stack::new().fit(StackFit::Expand).children(vec![scroll])),
                ]
        };
        vec![Box::new(body)]
    })
}

fn files_scroll<M>(content: impl syngui::widgets::containers::IntoWidget<M>) -> impl Widget {
    ScrollView::new().vertical().class("hf-files-scroll").child(content)
}

/// Тулбар над списком файлов: выбор, «Скачать выбранные / всё», сводка по
/// репозиторию и переключатель вида. Пауза, лимиты и фильтр форматов — в
/// нижней панели загрузок ([`super::dock`]): они общие для всех репозиториев.
///
/// `Flex` с переносом: панель узкая и двигается разделителем — в одну строку
/// элементы не помещались и обрезались по правому краю.
fn files_toolbar(repo_id: String, siblings: Vec<HfSibling>) -> impl Widget {
    let total_bytes: u64 = siblings.iter().filter_map(|s| s.size).sum();
    let n_files = siblings.len();

    let siblings_for_dl = siblings.clone();
    let rid_for_dl = repo_id.clone();
    let download_all_btn = Button::new(tr!("hf.detail.download_all"))
        .leading_icon(MI_CLOUD_DOWNLOAD)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            let app = use_context::<AppCtx>();
            download::download_all_for_repo(
                ctx,
                app.notifications.clone(),
                rid_for_dl.clone(),
                siblings_for_dl.clone(),
            );
        })
        .class("hf-toolbar-dl-all-btn");

    // Все ключи репозитория — для «Выбрать всё» и подсчёта выбранных.
    let all_keys: Vec<String> = siblings
        .iter()
        .map(|s| format!("{}/{}", repo_id, s.rfilename))
        .collect();

    let select_all_checkbox = {
        let all_keys = all_keys.clone();
        Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            let sel = ctx.selected_files.get();
            let all_checked = !all_keys.is_empty() && all_keys.iter().all(|k| sel.contains(k));
            let keys_c = all_keys.clone();
            let cb = Checkbox::checked(all_checked)
                .label(tr!("hf.detail.select_all"))
                .on_change(move |v| {
                    let ctx = use_context::<HuggingFaceCtx>();
                    ctx.selected_files.update(|s| {
                        for k in &keys_c {
                            if v {
                                s.insert(k.clone());
                            } else {
                                s.remove(k);
                            }
                        }
                    });
                })
                .class("hf-toolbar-select-all");
            vec![Box::new(cb)]
        })
    };

    let download_selected_btn = {
        let siblings = siblings.clone();
        let repo_id = repo_id.clone();
        Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            let prefix = format!("{}/", repo_id);
            let n = ctx
                .selected_files
                .get()
                .iter()
                .filter(|k| k.starts_with(&prefix))
                .count();
            let siblings_c = siblings.clone();
            let rid_c = repo_id.clone();
            let btn = Button::new(tr!("hf.detail.download_selected", n = n))
                .leading_icon(MI_DOWNLOAD)
                .disabled(n == 0)
                .on_click(move || {
                    let ctx = use_context::<HuggingFaceCtx>();
                    let app = use_context::<AppCtx>();
                    let sel = ctx.selected_files.get_untracked();
                    download::download_selected(
                        ctx,
                        app.notifications.clone(),
                        rid_c.clone(),
                        siblings_c.clone(),
                        sel,
                    );
                })
                .class("hf-toolbar-dl-sel-btn");
            vec![Box::new(btn)]
        })
    };

    // «464 ГБ · скачано 67 из 280» — только по этому репозиторию; скорость,
    // очередь и остаток времени показывает нижняя панель.
    let rid_for_status = repo_id.clone();
    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let prefix = format!("{}/", rid_for_status);
        let done = ctx
            .downloads
            .get()
            .iter()
            .filter(|(k, d)| k.starts_with(&prefix) && matches!(d.status, DlStatus::Done))
            .count();
        let size = if total_bytes > 0 { human_bytes(total_bytes) } else { "—".to_string() };
        vec![Box::new(
            Text::new(tr!("hf.detail.toolbar.summary", size = size, done = done, total = n_files))
                .max_lines(1)
                .class("hf-toolbar-stat-text"),
        )]
    });

    DecoratedBox::new().class("hf-files-toolbar").child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                DecoratedBox::new().class("grow").child(
                    Flex::new()
                        .direction(FlexDirection::Row)
                        .wrap()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(select_all_checkbox)
                        .child(download_selected_btn)
                        .child(download_all_btn)
                        .child(status_text),
                ),
            )
            .child(view_mode_switch()),
    )
}

/// Переключатель «список / значки» у правого края тулбара.
fn view_mode_switch() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let mode = ctx.files_view_mode.get();
        let button = |icon: &'static str, tip: String, target: FilesViewMode| {
            ToolButton::new(icon)
                .tooltip(tip)
                .active(mode == target)
                .on_click(move || use_context::<HuggingFaceCtx>().files_view_mode.set(target))
                .class("hf-icon-btn hf-view-mode-btn")
        };
        vec![Box::new(
            DecoratedBox::new().class("hf-view-mode").child(
                Row::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(button(MI_VIEW_LIST, tr!("hf.detail.view.list"), FilesViewMode::List))
                    .child(button(MI_GRID_VIEW, tr!("hf.detail.view.icons"), FilesViewMode::Icons)),
            ),
        )]
    })
}

fn expected_sha256(sibling: &HfSibling) -> Option<String> {
    sibling.lfs.as_ref().and_then(|l| {
        let raw = l.sha256.trim();
        (!raw.is_empty()).then(|| super::download::strip_sha256_prefix(raw).to_string())
    })
}

/// Иконка типа файла для плитки — по расширению.
fn file_icon(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "safetensors" | "gguf" | "bin" | "pt" | "pth" | "ckpt" | "onnx" | "h5" | "msgpack"
        | "syn" | "npz" => MI_MEMORY,
        "json" | "yaml" | "yml" | "toml" | "jsonl" | "xml" => MI_DATA_OBJECT,
        "py" | "sh" | "js" | "ts" | "rs" | "cpp" | "c" | "ipynb" | "jinja" => MI_CODE,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp" => MI_IMAGE,
        "mp4" | "mov" | "mkv" | "webm" | "wav" | "mp3" | "flac" | "ogg" => MI_MOVIE,
        "md" | "txt" | "pdf" | "license" | "rst" => MI_DESCRIPTION,
        _ => MI_INSERT_DRIVE_FILE,
    }
}

/// Плитка файла в режиме «значки»: иконка типа, имя (без каталога — он строкой
/// ниже), размер, прогресс и действия иконками. Состояния и действия те же,
/// что у строки списка.
fn file_tile(repo_id: String, sibling: HfSibling, state: Option<DownloadState>) -> impl Widget {
    let filename = sibling.rfilename.clone();
    let (dir, base) = match filename.rsplit_once('/') {
        Some((dir, base)) => (format!("{dir}/"), base.to_string()),
        None => (String::new(), filename.clone()),
    };
    let size_text = sibling.size.map(human_bytes).unwrap_or_else(|| "—".to_string());
    let key = format!("{}/{}", repo_id, filename);
    let status = state.as_ref().map(|s| s.status.clone());
    let class = match &status {
        Some(DlStatus::Done) => "hf-file-tile done",
        Some(DlStatus::Active) => "hf-file-tile active",
        Some(DlStatus::Error(_)) => "hf-file-tile error",
        _ => "hf-file-tile",
    };
    let caption = match state.as_ref() {
        Some(s) if matches!(s.status, DlStatus::Active) && s.total > 0 => {
            let pct = (s.bytes_done as f64 / s.total as f64 * 100.0).floor() as u32;
            if s.speed_bps > 1.0 {
                format!("{pct}% · {}", human_speed(s.speed_bps))
            } else {
                format!("{pct}%")
            }
        }
        _ => size_text,
    };
    let actions = icon_actions(
        repo_id.clone(),
        filename.clone(),
        expected_sha256(&sibling),
        sibling.size,
        status,
    );

    // Без общего Tooltip на плитке: внутри свои подсказки у кнопок действий,
    // а каталог файла показан строкой под именем.
    DecoratedBox::new().class(class).child(mgui! {
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        file_checkbox(key),
                        DecoratedBox::new().class("grow"),
                        verify_badge(state.as_ref()),
                        status_indicator(state.as_ref()),
                    ],
                Center::new().child(Icon::new(file_icon(&filename)).class("hf-file-tile-icon")),
                Text::new(base).max_lines(2).class("hf-file-tile-name"),
                Text::new(dir).max_lines(1).class("hf-file-tile-dir"),
                Text::new(caption).max_lines(1).class("hf-file-tile-size"),
                progress_bar(state.as_ref()),
                Center::new().child(actions),
            ]
    })
}

/// Действия над файлом иконками (с подсказкой) — для плиток и для очереди в
/// нижней панели, где подписи кнопок не помещаются. Набор по статусу тот же,
/// что у [`action_cluster`]. `size` — размер из API: с ним файл сразу входит в
/// сводный прогресс, не дожидаясь своей очереди.
pub(super) fn icon_actions(
    repo_id: String,
    filename: String,
    expected: Option<String>,
    size: Option<u64>,
    status: Option<DlStatus>,
) -> Row {
    let key = format!("{}/{}", repo_id, filename);
    let start = {
        let (rid, fname, exp) = (repo_id.clone(), filename.clone(), expected.clone());
        move |tip: String| {
            ToolButton::new(MI_DOWNLOAD)
                .tooltip(tip)
                .on_click(move || start_one(rid.clone(), fname.clone(), exp.clone(), size))
                .class("hf-icon-btn primary")
        }
    };
    let pause = {
        let key = key.clone();
        ToolButton::new(MI_PAUSE)
            .tooltip(tr!("hf.file.pause"))
            .on_click(move || download::pause_download(use_context::<HuggingFaceCtx>(), key.clone()))
            .class("hf-icon-btn")
    };
    let stop = {
        let key = key.clone();
        ToolButton::new(MI_STOP)
            .tooltip(tr!("hf.file.stop"))
            .on_click(move || download::stop_download(use_context::<HuggingFaceCtx>(), key.clone()))
            .class("hf-icon-btn")
    };
    let cancel = {
        let key = key.clone();
        ToolButton::new(MI_CLOSE)
            .tooltip(tr!("app.cancel"))
            .on_click(move || download::cancel_download(use_context::<HuggingFaceCtx>(), key.clone()))
            .class("hf-icon-btn danger")
    };
    let resume = {
        let key = key.clone();
        move |icon: &'static str, tip: String| {
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || {
                    let ctx = use_context::<HuggingFaceCtx>();
                    let app = use_context::<AppCtx>();
                    download::resume_download(ctx, app.notifications.clone(), key.clone());
                })
                .class("hf-icon-btn primary")
        }
    };
    let base = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("hf-file-actions");
    match status {
        None => base.child(start(tr!("hf.file.download"))),
        Some(DlStatus::Done) => base,
        Some(DlStatus::Error(_)) => base.child(start(tr!("hf.file.retry"))).child(cancel),
        Some(DlStatus::Pending) => base.child(pause).child(cancel),
        Some(DlStatus::Active) => base.child(pause).child(stop).child(cancel),
        Some(DlStatus::Paused) => base.child(resume(MI_PLAY_ARROW, tr!("hf.file.resume"))).child(cancel),
        Some(DlStatus::Stopped) => base.child(resume(MI_DOWNLOAD, tr!("hf.file.download"))).child(cancel),
    }
}

/// «Скачать» для одного файла + размер из API в сводный прогресс.
fn start_one(repo_id: String, filename: String, expected: Option<String>, size: Option<u64>) {
    let ctx = use_context::<HuggingFaceCtx>();
    let app = use_context::<AppCtx>();
    download::start_download(ctx, app.notifications.clone(), repo_id.clone(), filename.clone(), expected);
    let sibling = HfSibling { rfilename: filename, size, lfs: None };
    download::seed_totals(ctx, &repo_id, std::slice::from_ref(&sibling));
}

fn file_row(repo_id: String, sibling: HfSibling, state: Option<DownloadState>) -> impl Widget {
    let filename = sibling.rfilename.clone();
    let size_text = sibling
        .size
        .map(human_bytes)
        .unwrap_or_else(|| "—".to_string());

    // Нужен и для авто-verify после Done, и как hint ручной кнопке «SHA256».
    let expected_sha256 = expected_sha256(&sibling);

    let dl_status_widget = status_indicator(state.as_ref());
    let progress_widget = progress_bar(state.as_ref());
    let speed_widget = speed_indicator(state.as_ref());
    let verify_widget = verify_badge(state.as_ref());

    let key = format!("{}/{}", repo_id, filename);
    let key_for_verify = key.clone();
    let verify_btn_enabled = state
        .as_ref()
        .map(|s| matches!(s.status, DlStatus::Done) && !matches!(s.verify, VerifyStatus::Computing))
        .unwrap_or(false);
    let verify_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let inner: Box<dyn Widget> = if verify_btn_enabled {
            let key_clone = key_for_verify.clone();
            Box::new(
                Button::new("SHA256")
                    .leading_icon(MI_VERIFIED_USER)
                    .on_click(move || {
                        let ctx = use_context::<HuggingFaceCtx>();
                        let app = use_context::<AppCtx>();
                        super::verify::verify_file(
                            ctx,
                            app.notifications.clone(),
                            key_clone.clone(),
                        );
                    })
                    .class("hf-file-verify-btn"),
            )
        } else {
            Box::new(DecoratedBox::new().class("hf-file-verify-empty"))
        };
        vec![inner]
    });

    let actions = action_cluster(
        repo_id.clone(),
        filename.clone(),
        expected_sha256.clone(),
        sibling.size,
        state.as_ref().map(|s| s.status.clone()),
    );
    let checkbox = file_checkbox(key.clone());

    DecoratedBox::new().class("hf-file-row").child(mgui! {
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        checkbox,
                        Text::new(filename).class("hf-file-name"),
                        DecoratedBox::new().class("hf-file-spacer grow"),
                        speed_widget,
                        Text::new(size_text).class("hf-file-size"),
                        verify_widget,
                        dl_status_widget,
                        verify_btn,
                        actions,
                    ],
                progress_widget,
            ]
    })
}

/// Кластер кнопок действий справа в строке файла — зависит от статуса
/// загрузки. Перестраивается вместе со всей строкой (родительский `files_tab`
/// реактивен на `downloads`), поэтому достаточно снимка `status`.
fn action_cluster(
    repo_id: String,
    filename: String,
    expected_sha256: Option<String>,
    size: Option<u64>,
    status: Option<DlStatus>,
) -> impl Widget {
    let key = format!("{}/{}", repo_id, filename);
    let base = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("hf-file-actions");
    match status {
        // Не качали / пусто → одна кнопка «Скачать».
        None => base.child(start_btn(repo_id, filename, expected_sha256, size)),
        // Завершено → конвертация GGUF → .syn, если формат подходит.
        Some(DlStatus::Done) => {
            let ctx = use_context::<HuggingFaceCtx>();
            if ctx.gguf_support.get_untracked()
                && super::convert::is_gguf(&filename)
                && !super::convert::is_mmproj(&filename)
            {
                base.child(convert_btn(repo_id, filename))
            } else {
                base
            }
        }
        // Ошибка → повторить (то же start_download) + отмена (убирает запись).
        Some(DlStatus::Error(_)) => base
            .child(start_btn(repo_id, filename, expected_sha256, size))
            .child(cancel_btn(key)),
        // В очереди → пауза (снимет из очереди) + отмена.
        Some(DlStatus::Pending) => base
            .child(pause_btn(key.clone()))
            .child(cancel_btn(key)),
        // Качается → пауза + стоп + отмена.
        Some(DlStatus::Active) => base
            .child(pause_btn(key.clone()))
            .child(stop_btn(key.clone()))
            .child(cancel_btn(key)),
        // На паузе → продолжить (резюм) + отмена.
        Some(DlStatus::Paused) => base
            .child(resume_play_btn(key.clone()))
            .child(cancel_btn(key)),
        // Остановлено → скачать (резюм) + отмена.
        Some(DlStatus::Stopped) => base
            .child(resume_dl_btn(key.clone()))
            .child(cancel_btn(key)),
    }
}

/// Чекбокс выбора файла слева в строке. Reactive на `selected_files`, чтобы
/// отражать «Выбрать всё»/сброс. Тоггл правит множество выбранных ключей.
fn convert_btn(repo_id: String, filename: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let active = ctx.convert_active.get();
        let busy = active.is_some();
        let mine = active.as_deref() == Some(filename.as_str());
        let label = if mine {
            format!("→ .syn {:.0}%", ctx.convert_progress.get() * 100.0)
        } else {
            "→ .syn".to_string()
        };
        let (rid, fname) = (repo_id.clone(), filename.clone());
        let btn = Button::new(label)
            .on_click(move || {
                super::convert::start_from_context(rid.clone(), fname.clone());
            })
            .class(if busy && !mine { "hf-file-action-btn disabled" } else { "hf-file-action-btn" });
        vec![Box::new(btn)]
    })
}

fn file_checkbox(key: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let checked = ctx.selected_files.get().contains(&key);
        let key_c = key.clone();
        let cb = Checkbox::checked(checked)
            .on_change(move |v| {
                let ctx = use_context::<HuggingFaceCtx>();
                ctx.selected_files.update(|s| {
                    if v {
                        s.insert(key_c.clone());
                    } else {
                        s.remove(&key_c);
                    }
                });
            })
            .class("hf-file-checkbox");
        vec![Box::new(cb)]
    })
}

fn start_btn(repo_id: String, filename: String, expected: Option<String>, size: Option<u64>) -> Button {
    Button::new(tr!("hf.file.download"))
        .leading_icon(MI_DOWNLOAD)
        .on_click(move || start_one(repo_id.clone(), filename.clone(), expected.clone(), size))
        .class("hf-file-dl-btn")
}

fn pause_btn(key: String) -> Button {
    Button::new(tr!("hf.file.pause"))
        .leading_icon(MI_PAUSE)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            download::pause_download(ctx, key.clone());
        })
        .class("hf-file-action-btn")
}

fn stop_btn(key: String) -> Button {
    Button::new(tr!("hf.file.stop"))
        .leading_icon(MI_STOP)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            download::stop_download(ctx, key.clone());
        })
        .class("hf-file-action-btn")
}

fn cancel_btn(key: String) -> Button {
    Button::new(tr!("app.cancel"))
        .leading_icon(MI_CLOSE)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            download::cancel_download(ctx, key.clone());
        })
        .class("hf-file-action-btn cancel")
}

/// ▶ Продолжить — для Paused.
fn resume_play_btn(key: String) -> Button {
    Button::new(tr!("hf.file.resume"))
        .leading_icon(MI_PLAY_ARROW)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            let app = use_context::<AppCtx>();
            download::resume_download(ctx, app.notifications.clone(), key.clone());
        })
        .class("hf-file-dl-btn")
}

/// ⬇ Скачать (резюм) — для Stopped. Та же логика, что Продолжить, но другой
/// визуальный акцент (как обычная «Скачать»).
fn resume_dl_btn(key: String) -> Button {
    Button::new(tr!("hf.file.download"))
        .leading_icon(MI_DOWNLOAD)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            let app = use_context::<AppCtx>();
            download::resume_download(ctx, app.notifications.clone(), key.clone());
        })
        .class("hf-file-dl-btn")
}

/// Бейдж результата SHA-256-верификации справа от размера. Reactive
/// потому что состояние меняется со временем (Computing → Match/Mismatch).
fn verify_badge(state: Option<&DownloadState>) -> impl Widget {
    let snapshot = state.map(|s| s.verify.clone());
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let inner: Box<dyn Widget> = match &snapshot {
            Some(VerifyStatus::Match { has_expected: true, .. }) => Box::new(
                DecoratedBox::new()
                    .class("hf-verify-badge ok")
                    .child(Text::new("SHA256 ✓").class("hf-verify-text")),
            ),
            Some(VerifyStatus::Match { has_expected: false, sha256 }) => Box::new(
                DecoratedBox::new()
                    .class("hf-verify-badge info")
                    .child(Text::new(format!("SHA256 {}", &sha256[..8])).class("hf-verify-text")),
            ),
            Some(VerifyStatus::Mismatch { .. }) => Box::new(
                DecoratedBox::new()
                    .class("hf-verify-badge bad")
                    .child(Text::new("SHA256 ✗").class("hf-verify-text")),
            ),
            Some(VerifyStatus::Computing) => Box::new(
                DecoratedBox::new()
                    .class("hf-verify-badge computing")
                    .child(Text::new(tr!("hf.file.verifying")).class("hf-verify-text")),
            ),
            Some(VerifyStatus::Error(_)) => Box::new(
                DecoratedBox::new()
                    .class("hf-verify-badge bad")
                    .child(Text::new("SHA256 !").class("hf-verify-text")),
            ),
            _ => Box::new(DecoratedBox::new().class("hf-verify-badge-empty")),
        };
        vec![inner]
    })
}

/// «12.4 МБ/с» / «512 КБ/с» — для Active с положительной скоростью. Иначе
/// возвращает Reactive с пустым DecoratedBox (нулевая высота).
fn speed_indicator(state: Option<&DownloadState>) -> impl Widget {
    let snapshot = state.cloned();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let visible = snapshot
            .as_ref()
            .map(|s| matches!(s.status, DlStatus::Active) && s.speed_bps > 1.0)
            .unwrap_or(false);
        if !visible {
            return vec![Box::new(DecoratedBox::new().class("hf-file-speed-empty"))];
        }
        let bps = snapshot.as_ref().unwrap().speed_bps;
        vec![Box::new(
            Text::new(human_speed(bps)).class("hf-file-speed"),
        )]
    })
}

/// Иконка статуса справа: пусто / pending / done / error.
/// Для Active + retry_count > 0 показываем «попытка N/3» рядом с пустым
/// слотом, давая пользователю понять, что мы не повисли — идёт backoff.
pub(super) fn status_indicator(state: Option<&DownloadState>) -> impl Widget {
    let snapshot = state.map(|s| (s.status.clone(), s.retry_count));
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let inner: Box<dyn Widget> = match &snapshot {
            Some((DlStatus::Done, _)) => Box::new(
                DecoratedBox::new()
                    .class("hf-status-done")
                    .child(Icon::new(MI_CHECK).class("hf-status-icon")),
            ),
            Some((DlStatus::Error(_), _)) => Box::new(
                DecoratedBox::new()
                    .class("hf-status-error")
                    .child(Icon::new(MI_REPORT).class("hf-status-icon")),
            ),
            Some((DlStatus::Pending, _)) => Box::new(
                DecoratedBox::new()
                    .class("hf-status-pending")
                    .child(Icon::new(MI_HOURGLASS_TOP).class("hf-status-icon")),
            ),
            Some((DlStatus::Paused, _)) => Box::new(
                DecoratedBox::new()
                    .class("hf-status-paused")
                    .child(Icon::new(MI_PAUSE).class("hf-status-icon")),
            ),
            Some((DlStatus::Stopped, _)) => Box::new(
                DecoratedBox::new()
                    .class("hf-status-stopped")
                    .child(Icon::new(MI_STOP).class("hf-status-icon")),
            ),
            Some((DlStatus::Active, n)) if *n > 0 => Box::new(
                Text::new(tr!("hf.file.retry_attempt", n = n)).class("hf-status-retry"),
            ),
            _ => Box::new(DecoratedBox::new().class("hf-status-empty")),
        };
        vec![inner]
    })
}

/// Прогресс-бар:
/// - Pending → серая «заглушка-rail» без заполнения (показывает, что задача
///   стоит в очереди).
/// - Active + total=0 → indeterminate (полная полоса с opacity).
/// - Active + сегментов нет → один rail с заполнением `bytes_done/total`.
/// - Active + N сегментов → Row из N равных rail'ов, каждый заполняется
///   отдельно (визуально видно параллельную работу).
pub(super) fn progress_bar(state: Option<&DownloadState>) -> impl Widget {
    let snapshot = state.cloned();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(s) = &snapshot else {
            return vec![Box::new(DecoratedBox::new().class("hf-progress-empty"))];
        };
        if matches!(s.status, DlStatus::Pending) {
            return vec![Box::new(
                DecoratedBox::new().class("hf-progress hf-progress-pending"),
            )];
        }
        // Paused/Stopped рисуем «замороженный» прогресс: тот же расчёт
        // bytes_done/total, но без пульсации — видно, сколько уже скачано.
        let frozen = matches!(s.status, DlStatus::Paused | DlStatus::Stopped);
        if !matches!(s.status, DlStatus::Active) && !frozen {
            return vec![Box::new(DecoratedBox::new().class("hf-progress-empty"))];
        }
        if s.total == 0 {
            // Indeterminate-пульсация осмысленна только пока качаем; для
            // замороженных без известного total показать нечего.
            if frozen {
                return vec![Box::new(DecoratedBox::new().class("hf-progress-empty"))];
            }
            return vec![Box::new(
                DecoratedBox::new()
                    .class("hf-progress hf-progress-indeterminate")
                    .child(DecoratedBox::new().class("hf-progress-bar")),
            )];
        }
        if s.segments.len() > 1 {
            let has_unfinished = s.segments.iter().any(|seg| {
                let span = (seg.to.saturating_sub(seg.from)).saturating_add(1);
                seg.bytes_done < span
            });
            let container_class = if has_unfinished && !frozen {
                "hf-progress-segments hf-progress-segments-active"
            } else {
                "hf-progress-segments"
            };
            let mut row = Row::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class(container_class);
            for seg in s.segments.iter() {
                let span = (seg.to.saturating_sub(seg.from)).saturating_add(1).max(1) as f64;
                let ratio = (seg.bytes_done as f64 / span).clamp(0.0, 1.0);
                let percent = (ratio * 100.0) as f32;
                let fill = DecoratedBox::new()
                    .class("hf-progress-segment-fill")
                    .style("width", StyleValue::percent(percent));
                let cell = DecoratedBox::new()
                    .class("hf-progress-segment grow")
                    .child(fill);
                row = row.child(cell);
            }
            return vec![Box::new(row)];
        }
        let ratio = (s.bytes_done as f64 / s.total as f64).clamp(0.0, 1.0);
        let percent = (ratio * 100.0) as f32;
        let bar = DecoratedBox::new()
            .class("hf-progress-bar")
            .style("width", StyleValue::percent(percent));
        vec![Box::new(
            DecoratedBox::new().class("hf-progress").child(bar),
        )]
    })
}

// silence unused import (HfModelDetails reachable via `Reactive` closure types)
#[allow(dead_code)]
fn _ref_details(_: HfModelDetails) {}
