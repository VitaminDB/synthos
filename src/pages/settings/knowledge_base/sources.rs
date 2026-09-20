//! Секция «Источники»: три способа добавить документы и ход индексации.
//!
//! Пока идёт job, строки добавления заменяет блок прогресса — второй job
//! параллельно не запустить, и кнопки, которые ничего не делают, не нужны.

use std::path::PathBuf;

use rfd::AsyncFileDialog;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::ingest::pipeline::{IngestProgress, IngestStage};
use crate::kb::ingest::source::DocSource;
use crate::kb::runner;

pub(super) fn view(collection_id: String) -> Box<dyn Widget> {
    let body = Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let body: Box<dyn Widget> = match ctx.kb.ingest_progress.get() {
            Some(p) => progress(p, &collection_id),
            None => add_rows(collection_id.clone()),
        };
        vec![body]
    });
    super::section(tr!("settings.knowledge_base.sources"), None, Box::new(body))
}

fn add_rows(collection_id: String) -> Box<dyn Widget> {
    let cid_files = collection_id.clone();
    let cid_folder = collection_id.clone();
    let cid_url = collection_id;
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(add_row(
                MI_NOTE_ADD,
                tr!("settings.knowledge_base.sources.files"),
                tr!("settings.knowledge_base.sources.files.desc"),
                move || pick_files(cid_files.clone()),
            ))
            .child(add_row(
                MI_CREATE_NEW_FOLDER,
                tr!("settings.knowledge_base.sources.folder"),
                tr!("settings.knowledge_base.sources.folder.desc"),
                move || pick_folder(cid_folder.clone()),
            ))
            .child(add_row(
                MI_LANGUAGE,
                tr!("settings.knowledge_base.sources.url"),
                tr!("settings.knowledge_base.sources.url.desc"),
                move || {
                    // Portal-диалог смонтирован в `pages::settings::view()`.
                    use_context::<AppCtx>().kb_url_dialog.set(Some(cid_url.clone()));
                },
            )),
    )
}

/// Строка-кнопка: вся строка кликабельна, справа «+».
fn add_row(
    icon: &'static str,
    title: String,
    desc: String,
    on_click: impl Fn() + Send + Sync + 'static,
) -> impl Widget {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![super::row_icon(icon)])
        .child(
            DecoratedBox::new().class("grow kb-min0").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .child(Icon::new(MI_ADD).class("kb-row-action-icon"));
    DecoratedBox::new().class("settings-row kb-click-row").child(
        GestureDetector::new()
            .on_click(on_click)
            .child(Padding::symmetric(24.0, 16.0).child(inner)),
    )
}

fn progress(p: IngestProgress, open_collection: &str) -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let fraction = (p.total > 0).then(|| (p.current as f32 / p.total as f32).clamp(0.0, 1.0));
    let title = match fraction {
        Some(_) => tr!(
            "settings.knowledge_base.progress.title",
            current = p.current,
            total = p.total
        ),
        None => tr!("settings.knowledge_base.progress.title_unknown"),
    };
    // Job может идти в другую коллекцию — подписываем, в какую.
    let elsewhere = (p.collection_id != open_collection)
        .then(|| {
            ctx.kb
                .registry
                .get_untracked()
                .get(&p.collection_id)
                .map(|m| m.name.clone())
        })
        .flatten();
    let mut detail = stage_label(p.stage);
    if let Some(file) = &p.current_file {
        detail = format!("{detail} · {file}");
    }
    if let Some(name) = elsewhere {
        detail = format!("{} · {detail}", tr!("settings.knowledge_base.progress.into", name = name));
    }

    let bar = match fraction {
        Some(v) => ProgressBar::new().value(v),
        None => ProgressBar::new().indeterminate(),
    };

    Box::new(
        Padding::symmetric(24.0, 20.0).child(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(vec![super::row_icon(MI_AUTORENEW)])
                        .child(
                            DecoratedBox::new().class("grow kb-min0").child(
                                Column::new()
                                    .gap(2.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                    .child(Text::new(title).class("settings-row-title"))
                                    .child(
                                        Text::new(detail)
                                            .elide(Elide::Middle)
                                            .class("settings-row-desc"),
                                    ),
                            ),
                        )
                        .child(
                            Button::new(tr!("settings.knowledge_base.progress.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(|| runner::cancel(&use_context::<AppCtx>().kb))
                                .class("kb-btn"),
                        ),
                )
                .child(bar.class("kb-progress-bar")),
        ),
    )
}

fn stage_label(stage: IngestStage) -> String {
    match stage {
        IngestStage::Preparing => tr!("settings.knowledge_base.stage.preparing"),
        IngestStage::Discovering => tr!("settings.knowledge_base.stage.discovering"),
        IngestStage::Parsing => tr!("settings.knowledge_base.stage.parsing"),
        IngestStage::Chunking => tr!("settings.knowledge_base.stage.chunking"),
        IngestStage::Embedding => tr!("settings.knowledge_base.stage.embedding"),
        IngestStage::Storing => tr!("settings.knowledge_base.stage.storing"),
        IngestStage::Done => tr!("settings.knowledge_base.stage.done"),
        IngestStage::Cancelled => tr!("settings.knowledge_base.stage.cancelled"),
        IngestStage::Error => tr!("settings.knowledge_base.stage.error"),
    }
}

fn pick_files(collection_id: String) {
    syngui::async_runtime::spawn(async move {
        let picked = AsyncFileDialog::new()
            .add_filter(
                tr!("settings.knowledge_base.pick_files.filter"),
                &["md", "markdown", "txt", "html", "htm", "pdf"],
            )
            .add_filter(tr!("settings.knowledge_base.pick_files.filter_all"), &["*"])
            .set_title(tr!("settings.knowledge_base.pick_files.title"))
            .pick_files()
            .await;
        let Some(files) = picked else { return };
        let sources: Vec<DocSource> = files
            .into_iter()
            .map(|f| {
                let p = PathBuf::from(f.path());
                if p.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    DocSource::Pdf(p)
                } else {
                    DocSource::File(p)
                }
            })
            .collect();
        syngui::async_runtime::run_on_main_thread(move || super::launch(collection_id, sources));
    });
}

fn pick_folder(collection_id: String) {
    syngui::async_runtime::spawn(async move {
        let picked = AsyncFileDialog::new()
            .set_title(tr!("settings.knowledge_base.pick_folder.title"))
            .pick_folder()
            .await;
        let Some(folder) = picked else { return };
        let sources = vec![DocSource::Folder {
            root: PathBuf::from(folder.path()),
            extra_excludes: Vec::new(),
        }];
        syngui::async_runtime::run_on_main_thread(move || super::launch(collection_id, sources));
    });
}
