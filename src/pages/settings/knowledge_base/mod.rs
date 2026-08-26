//! Settings → «Базы знаний» — управление RAG-коллекциями.
//!
//! Двухпанельная компоновка повторяет `audio_models`:
//! - Центральная панель (`view()` ниже) — редактор активной коллекции
//!   или empty state, если ни одной не выбрано.
//! - Правая панель (`collections_panel::view()`) — список коллекций +
//!   кнопка «+ Новая коллекция». Подключена в `pages/settings/mod.rs::right_panel`.
//!
//! Все операции реактивные: state хранится в `AppCtx.kb` (см. `kb::ctx`),
//! изменения через `kb::loader` / `kb::runner` асинхронно публикуют
//! обратные сигналы (snackbar / ingest_progress).

pub mod collections_panel;
pub mod url_dialog;

use std::path::PathBuf;

use syngui::mgui;
use syngui::prelude::*;
use rfd::AsyncFileDialog;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::collection::CollectionMeta;
use crate::kb::ingest::source::DocSource;
use crate::kb::{loader, runner};

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("settings-page kb-page")
        .child(move || {
            let ctx = use_context::<AppCtx>();
            let active = ctx.kb.active_collection_id.get();
            let registry = ctx.kb.registry.get();
            let meta = active
                .as_ref()
                .and_then(|id| registry.get(id).cloned());
            let child: Box<dyn Widget> = match meta {
                Some(m) => Box::new(editor(m)),
                None => Box::new(empty_state()),
            };
            Stack::new().fit(StackFit::Expand).children(vec![child])
        })
}

fn empty_state() -> impl Widget {
    mgui! {
        Center::new() => [
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("models-empty-bubble") => [
                    Center::new().child(Icon::new(MI_MENU_BOOK).class("models-empty-icon")),
                ],
                Text::new(tr!("settings.knowledge_base.empty.title")).class("models-empty-title"),
                Padding::symmetric(32.0, 0.0).child(
                    Text::new(tr!("settings.knowledge_base.empty.text")).class("models-empty-text"),
                ),
            ]
        ]
    }
}

fn editor(meta: CollectionMeta) -> impl Widget {
    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    header_card(&meta),
                    embedder_card(),
                    sources_card(meta.id.clone()),
                    progress_card(),
                    documents_card(meta.id.clone()),
                ]
            ]
        ]
    }
}

fn header_card(meta: &CollectionMeta) -> impl Widget {
    let id = meta.id.clone();
    let name_initial = meta.name.clone();
    let docs = meta.document_count;
    let chunks = meta.chunk_count;
    let dim = meta.embedding_dim;
    let model = meta.embedding_model.clone();
    mgui! {
        DecoratedBox::new().class("models-card models-card-header") => [
            Padding::all(20.0) => [
                Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        DecoratedBox::new().class("models-header-icon-wrap") => [
                            Center::new().child(Icon::new(MI_MENU_BOOK).class("models-header-icon")),
                        ],
                        DecoratedBox::new().class("grow") => [
                            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                Text::new(tr!("settings.knowledge_base.name")).class("models-field-label"),
                                TextField::with_text(name_initial)
                                    .placeholder(tr!("settings.knowledge_base.name.placeholder"))
                                    .on_change(move |s| {
                                        let name = s.to_string();
                                        let id = id.clone();
                                        let ctx = use_context::<AppCtx>();
                                        ctx.kb.registry.update(move |reg| {
                                            // Pre-compute db_path до mutable-borrow,
                                            // чтобы не схлестнуться с iter_mut.
                                            let db_path = reg.kb_dir.join(format!("{}.sqlite", id));
                                            if let Some(m) = reg.items.iter_mut().find(|m| m.id == id) {
                                                m.name = name.clone();
                                                if let Ok(mut store) = crate::kb::store::Store::open(&db_path) {
                                                    let _ = store.upsert_collection(m);
                                                }
                                            }
                                            reg.items.sort_by(|a, b| a.name.cmp(&b.name));
                                        });
                                    }),
                            ]
                        ]
                    ],
                    Text::new(tr!(
                        "settings.knowledge_base.meta_line",
                        model = model, dim = dim, docs = docs, chunks = chunks
                    )).class("kb-meta-line"),
                ]
            ]
        ]
    }
}

fn embedder_card() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("models-card") => [
            Padding::all(20.0) => [
                Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("settings.knowledge_base.embedder")).class("models-section-title"),
                    Reactive::new(move || {
                        let ctx = use_context::<AppCtx>();
                        let loaded = ctx.kb.get_embedder().is_some();
                        let label: Box<dyn Widget> = Box::new(
                            Text::new(if loaded {
                                tr!("settings.knowledge_base.embedder.loaded")
                            } else {
                                tr!("settings.knowledge_base.embedder.not_loaded")
                            })
                                .class("kb-embedder-status"),
                        );
                        let action: Box<dyn Widget> = if loaded {
                            Box::new(
                                Button::new(tr!("settings.knowledge_base.embedder.unload"))
                                    .class("kb-embedder-btn")
                                    .on_click(|| {
                                        let app = use_context::<AppCtx>();
                                        loader::unload(&app.kb, &app.notifications);
                                    }),
                            )
                        } else {
                            Box::new(
                                Button::new(tr!("settings.knowledge_base.embedder.load"))
                                    .class("kb-embedder-btn primary")
                                    .on_click(|| {
                                        let app = use_context::<AppCtx>();
                                        let cfg = crate::config::AppConfig::load().kb;
                                        loader::ensure_loaded(
                                            app.kb.clone(),
                                            app.notifications.clone(),
                                            cfg.clone(),
                                        );
                                        // Параллельно подтягиваем реранкер, если
                                        // включён в config и путь существует — это
                                        // одна и та же UI-команда «подготовить KB».
                                        loader::ensure_reranker_loaded(
                                            app.kb.clone(),
                                            app.notifications.clone(),
                                            cfg,
                                        );
                                    }),
                            )
                        };
                        vec![Box::new(
                            Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(vec![label, Box::new(DecoratedBox::new().class("grow")), action])
                        ) as Box<dyn Widget>]
                    }),
                    Text::new(tr!(
                        "settings.knowledge_base.embedder.path_hint",
                        path = crate::config::AppConfig::load().kb.embedder_model_path,
                    )).class("kb-embedder-hint"),
                ]
            ]
        ]
    }
}

fn sources_card(collection_id: String) -> impl Widget {
    let cid_file = collection_id.clone();
    let cid_folder = collection_id.clone();
    let cid_url = collection_id.clone();
    mgui! {
        DecoratedBox::new().class("models-card") => [
            Padding::all(20.0) => [
                Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("settings.knowledge_base.sources")).class("models-section-title"),
                    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Button::new(tr!("settings.knowledge_base.sources.file"))
                            .class("kb-source-btn")
                            .on_click(move || pick_files_async(cid_file.clone())),
                        Button::new(tr!("settings.knowledge_base.sources.folder"))
                            .class("kb-source-btn")
                            .on_click(move || pick_folder_async(cid_folder.clone())),
                        Button::new(tr!("settings.knowledge_base.sources.url"))
                            .class("kb-source-btn")
                            .on_click(move || prompt_url_and_ingest(cid_url.clone())),
                    ],
                    Text::new(tr!("settings.knowledge_base.sources.hint")).class("kb-embedder-hint"),
                ]
            ]
        ]
    }
}

fn progress_card() -> impl Widget {
    Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let progress = ctx.kb.ingest_progress.get();
        let body: Box<dyn Widget> = match progress {
            None => Box::new(DecoratedBox::new()),
            Some(p) => {
                let pct = if p.total > 0 {
                    (p.current as f32 / p.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let stage = format!("{:?}", p.stage);
                let cur = p
                    .current_file
                    .clone()
                    .unwrap_or_else(|| tr!("settings.knowledge_base.progress.preparing"));
                Box::new(mgui! {
                    DecoratedBox::new().class("models-card kb-progress-card") => [
                        Padding::all(20.0) => [
                            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                Text::new(tr!("settings.knowledge_base.progress.title", current = p.current, total = p.total)).class("models-section-title"),
                                ProgressBar::new().value(pct).class("kb-progress-bar"),
                                Text::new(tr!("settings.knowledge_base.progress.stage", stage = stage, file = cur)).class("kb-progress-line"),
                                Button::new(tr!("settings.knowledge_base.progress.cancel"))
                                    .class("kb-source-btn")
                                    .on_click(|| {
                                        let app = use_context::<AppCtx>();
                                        runner::cancel(&app.kb);
                                    }),
                            ]
                        ]
                    ]
                })
            }
        };
        vec![body]
    })
}

fn documents_card(collection_id: String) -> impl Widget {
    Reactive::new(move || {
        let registry = {
            let app = use_context::<AppCtx>();
            app.kb.registry.get()
        };
        let store = registry.open_store(&collection_id).ok();
        let docs = store
            .as_ref()
            .and_then(|s| s.list_documents().ok())
            .unwrap_or_default();
        let header: Box<dyn Widget> = Box::new(
            Text::new(tr!("settings.knowledge_base.documents", n = docs.len()))
                .class("models-section-title"),
        );
        let mut children: Vec<Box<dyn Widget>> = vec![header];
        if docs.is_empty() {
            children.push(Box::new(
                Text::new(tr!("settings.knowledge_base.documents.empty")).class("kb-embedder-hint"),
            ));
        } else {
            for d in docs.into_iter().take(50) {
                let title = d.title.unwrap_or_else(|| d.source_path.clone());
                let kb_size = (d.bytes as f64 / 1024.0).max(0.0);
                let line = format!("{}\n{}  ({:.1} KB)", title, d.source_path, kb_size);
                children.push(Box::new(
                    DecoratedBox::new()
                        .class("kb-doc-row")
                        .child(Padding::all(10.0).child(Text::new(line).class("kb-doc-line"))),
                ));
            }
        }
        let card: Box<dyn Widget> = Box::new(
            DecoratedBox::new().class("models-card").child(
                Padding::all(20.0).child(
                    Column::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(children),
                ),
            ),
        );
        vec![card]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Source pickers
// ─────────────────────────────────────────────────────────────────────────────

fn pick_files_async(collection_id: String) {
    syngui::async_runtime::spawn(async move {
        let result = AsyncFileDialog::new()
            .add_filter(tr!("settings.knowledge_base.pick_files.filter"), &["md", "markdown", "txt", "html", "htm", "pdf"])
            .set_title(tr!("settings.knowledge_base.pick_files.title"))
            .pick_files()
            .await;
        let Some(files) = result else { return };
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
        syngui::async_runtime::run_on_main_thread(move || {
            launch(collection_id, sources);
        });
    });
}

fn pick_folder_async(collection_id: String) {
    syngui::async_runtime::spawn(async move {
        let result = AsyncFileDialog::new()
            .set_title(tr!("settings.knowledge_base.pick_folder.title"))
            .pick_folder()
            .await;
        let Some(folder) = result else { return };
        let path = PathBuf::from(folder.path());
        let sources = vec![DocSource::Folder {
            root: path,
            extra_excludes: Vec::new(),
        }];
        syngui::async_runtime::run_on_main_thread(move || {
            launch(collection_id, sources);
        });
    });
}

fn prompt_url_and_ingest(collection_id: String) {
    // Открываем Portal-диалог `url_dialog::view()` — он смонтирован
    // в `pages::settings::view()` и слушает `AppCtx.kb_url_dialog`.
    use_context::<AppCtx>().kb_url_dialog.set(Some(collection_id));
}

fn launch(collection_id: String, sources: Vec<DocSource>) {
    let app = use_context::<AppCtx>();
    let cfg = crate::config::AppConfig::load().kb;
    runner::start(
        app.kb.clone(),
        app.notifications.clone(),
        cfg,
        collection_id,
        sources,
    );
}
