//! Правая панель: детали выбранной модели — README или список файлов.
//!
//! Подписана на `selected_model`, `model_details`, `readme_*`, `detail_tab`,
//! `downloads`.

use syngui::mgui;
use syngui::mss::{MssColor, StyleValue};
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::input::{Checkbox, SpinBox, Toggle};
use syngui::widgets::navigation::{Tab, TabBar};
use syngui::widgets::visual::MarkdownView;

use crate::context::AppCtx;
use crate::icons::{
    MI_CHECK, MI_CLOSE, MI_CLOUD_DOWNLOAD, MI_DOWNLOAD, MI_HOURGLASS_TOP, MI_PAUSE, MI_PLAY_ARROW,
    MI_REPORT, MI_AUTO_AWESOME, MI_SPEED, MI_STOP, MI_TUNE, MI_VERIFIED_USER,
};

use super::download;
use super::state::{
    DlStatus, DownloadState, HfModelDetails, HfSibling, HuggingFaceCtx, TAB_FILES, TAB_README,
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
    let ctx_tab = use_context::<HuggingFaceCtx>().detail_tab;
    let tab_bar = TabBar::new()
        .tab(Tab::new("README", TAB_README, &ctx_tab))
        .tab(Tab::new(tr!("hf.detail.tab.files"), TAB_FILES, &ctx_tab));

    let rid_for_header = repo_id.clone();
    mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("hf-detail-header").child(
                    detail_header_row(rid_for_header)
                ),
                DecoratedBox::new().class("hf-detail-tabs").child(tab_bar),
                DecoratedBox::new().class("hf-detail-content grow").child(
                    tab_content(repo_id)
                ),
            ]
    }
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

fn tab_content(repo_id: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let tab = ctx.detail_tab.get();
        if tab == TAB_README {
            vec![Box::new(readme_tab())]
        } else {
            vec![Box::new(files_tab(repo_id.clone()))]
        }
    })
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
        let mut col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for sibling in d.siblings.iter() {
            let key = format!("{}/{}", rid, sibling.rfilename);
            let dl = downloads.get(&key).cloned();
            col = col.child(file_row(rid.clone(), sibling.clone(), dl));
        }
        let scroll = ScrollView::new().vertical().class("hf-files-scroll").child(col);
        let body = mgui! {
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    toolbar,
                    DecoratedBox::new().class("grow").child(scroll),
                ]
        };
        vec![Box::new(body)]
    })
}

/// Тулбар над списком файлов: total-size, прогресс-статус, SpinBox concurrent,
/// кнопка «Скачать всё». Реактивно реагирует на изменения `downloads`.
fn files_toolbar(repo_id: String, siblings: Vec<HfSibling>) -> impl Widget {
    let total_bytes: u64 = siblings.iter().filter_map(|s| s.size).sum();
    let total_text = if total_bytes > 0 {
        tr!("hf.detail.toolbar.total", size = human_bytes(total_bytes))
    } else {
        tr!("hf.detail.toolbar.total", size = "—")
    };
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

    let rid_for_status = repo_id.clone();
    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let downloads = ctx.downloads.get();
        let prefix = format!("{}/", rid_for_status);
        let mut done = 0u32;
        let mut active = 0u32;
        let mut pending = 0u32;
        let mut error = 0u32;
        let mut paused = 0u32;
        let mut total_speed = 0.0_f64;
        for (k, d) in downloads.iter() {
            if !k.starts_with(&prefix) {
                continue;
            }
            match &d.status {
                DlStatus::Done => done += 1,
                DlStatus::Active => {
                    active += 1;
                    total_speed += d.speed_bps;
                }
                DlStatus::Pending => pending += 1,
                DlStatus::Error(_) => error += 1,
                // Paused и Stopped считаем вместе — оба «на паузе, есть .part».
                DlStatus::Paused | DlStatus::Stopped => paused += 1,
            }
        }
        let speed_tail = if active > 0 && total_speed > 1.0 {
            format!(" • {}", human_speed(total_speed))
        } else {
            String::new()
        };
        let mut msg = tr!(
            "hf.detail.status.summary",
            done = done, total = n_files, active = active, pending = pending
        );
        if paused > 0 {
            msg.push_str(&tr!("hf.detail.status.paused_suffix", n = paused));
        }
        if error > 0 {
            msg.push_str(&tr!("hf.detail.status.error_suffix", n = error));
        }
        msg.push_str(&speed_tail);
        vec![Box::new(
            Text::new(msg).class("hf-toolbar-stat-text"),
        )]
    });

    // Тумблер фильтра форматов: «Скачать всё» пропускает onnx/openvino/fp32/bin.
    // Reactive, чтобы переключение из другого места (Settings) отражалось здесь.
    let skip_toggle = Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let on = ctx.skip_unwanted_formats.get();
        let toggle = Toggle::with_state(on).on_change(move |v| {
            let ctx = use_context::<HuggingFaceCtx>();
            ctx.skip_unwanted_formats.set(v);
        });
        let row = mgui! {
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(tr!("hf.detail.skip_formats_toggle")).class("hf-toolbar-stat-text"),
                    toggle,
                ]
        };
        vec![Box::new(row)]
    });

    let spin_widget = Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let current = ctx.concurrent_limit.get();
        let spin = SpinBox::new()
            .value(current as f64)
            .range(1.0, 8.0)
            .step(1.0)
            .decimal_places(0)
            .width(72.0)
            .on_change(move |v| {
                let ctx = use_context::<HuggingFaceCtx>();
                let v = v.round().clamp(1.0, 8.0) as u32;
                if ctx.concurrent_limit.get_untracked() != v {
                    ctx.concurrent_limit.set(v);
                    // Если лимит увеличили — попробовать слить очередь.
                    let app = use_context::<AppCtx>();
                    download::try_drain_queue(ctx, app.notifications.clone());
                }
            })
            .class("hf-toolbar-spin");
        vec![Box::new(spin)]
    });

    let speed_limit_widget = Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let current = ctx.speed_limit_mbps.get();
        let spin = SpinBox::new()
            .value(current as f64)
            .range(0.0, 2000.0)
            .step(1.0)
            .decimal_places(0)
            .width(88.0)
            .on_change(move |v| {
                let ctx = use_context::<HuggingFaceCtx>();
                let v = v.round().clamp(0.0, 2000.0) as u32;
                if ctx.speed_limit_mbps.get_untracked() != v {
                    ctx.speed_limit_mbps.set(v);
                }
            })
            .class("hf-toolbar-spin");
        vec![Box::new(spin)]
    });

    let global_pause_btn = Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let paused = ctx.global_paused.get();
        let btn: Box<dyn Widget> = if paused {
            Box::new(
                Button::new("")
                    .leading_icon(MI_PLAY_ARROW)
                    .on_click(move || {
                        let ctx = use_context::<HuggingFaceCtx>();
                        let app = use_context::<AppCtx>();
                        download::resume_all(ctx, app.notifications.clone());
                    })
                    .class("hf-toolbar-pause-all paused"),
            )
        } else {
            Box::new(
                Button::new("")
                    .leading_icon(MI_PAUSE)
                    .on_click(move || {
                        let ctx = use_context::<HuggingFaceCtx>();
                        download::pause_all(ctx);
                    })
                    .class("hf-toolbar-pause-all"),
            )
        };
        vec![btn]
    });

    DecoratedBox::new().class("hf-files-toolbar").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                select_all_checkbox,
                download_selected_btn,
                download_all_btn,
                global_pause_btn,
                Text::new(total_text).class("hf-toolbar-stat-text"),
                status_text,
                skip_toggle,
                DecoratedBox::new().class("grow"),
                Icon::new(MI_SPEED).class("hf-toolbar-icon"),
                speed_limit_widget,
                Icon::new(MI_TUNE).class("hf-toolbar-icon"),
                spin_widget,
            ]
    })
}

fn file_row(repo_id: String, sibling: HfSibling, state: Option<DownloadState>) -> impl Widget {
    let filename = sibling.rfilename.clone();
    let size_text = sibling
        .size
        .map(human_bytes)
        .unwrap_or_else(|| "—".to_string());

    // expected_sha256 — нужен и для авто-verify после Done, и как hint в
    // verify_file для ручной кнопки. Pre-extract здесь чтобы не дёргать
    // sibling.lfs из замыкания.
    let expected_sha256 = sibling.lfs.as_ref().and_then(|l| {
        let raw = l.sha256.trim();
        if raw.is_empty() {
            None
        } else {
            Some(super::download::strip_sha256_prefix(raw).to_string())
        }
    });

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
    status: Option<DlStatus>,
) -> impl Widget {
    let key = format!("{}/{}", repo_id, filename);
    let base = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("hf-file-actions");
    match status {
        // Не качали / пусто → одна кнопка «Скачать».
        None => base.child(start_btn(repo_id, filename, expected_sha256)),
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
            .child(start_btn(repo_id, filename, expected_sha256))
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

fn start_btn(repo_id: String, filename: String, expected: Option<String>) -> Button {
    Button::new(tr!("hf.file.download"))
        .leading_icon(MI_DOWNLOAD)
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            let app = use_context::<AppCtx>();
            download::start_download(
                ctx,
                app.notifications.clone(),
                repo_id.clone(),
                filename.clone(),
                expected.clone(),
            );
        })
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

fn human_speed(bps: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bps >= GB {
        format!("{:.2} {}", bps / GB, tr!("hf.unit.gb_per_s"))
    } else if bps >= MB {
        format!("{:.1} {}", bps / MB, tr!("hf.unit.mb_per_s"))
    } else if bps >= KB {
        format!("{:.0} {}", bps / KB, tr!("hf.unit.kb_per_s"))
    } else {
        format!("{:.0} {}", bps, tr!("hf.unit.b_per_s"))
    }
}

/// Иконка статуса справа: пусто / pending / done / error.
/// Для Active + retry_count > 0 показываем «попытка N/3» рядом с пустым
/// слотом, давая пользователю понять, что мы не повисли — идёт backoff.
fn status_indicator(state: Option<&DownloadState>) -> impl Widget {
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
fn progress_bar(state: Option<&DownloadState>) -> impl Widget {
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

fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = n as f64;
    if f >= GB {
        format!("{:.2} {}", f / GB, tr!("hf.unit.gb"))
    } else if f >= MB {
        format!("{:.1} {}", f / MB, tr!("hf.unit.mb"))
    } else if f >= KB {
        format!("{:.1} {}", f / KB, tr!("hf.unit.kb"))
    } else {
        format!("{} {}", n, tr!("hf.unit.b"))
    }
}
// silence unused import (HfModelDetails reachable via `Reactive` closure types)
#[allow(dead_code)]
fn _ref_details(_: HfModelDetails) {}
