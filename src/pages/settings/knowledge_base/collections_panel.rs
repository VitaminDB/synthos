//! Правая колонка вкладки «Базы знаний» — список коллекций + «+».

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::context::AppCtx;
use crate::icons::*;

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("skills-panel models-panel kb-panel")
        .child(mgui! {
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("grow").child(
                    Padding::symmetric(8.0, 8.0).child(list_reactive()),
                ),
            ]
        })
}

/// Заголовок панели — в общей строке заголовков каркаса настроек.
pub fn header() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("skills-panel-header") => [
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("skills-panel-header-icon-wrap") => [
                    Center::new().child(Icon::new(MI_MENU_BOOK).class("skills-panel-header-icon")),
                ],
                DecoratedBox::new().class("grow").child(
                    Text::new(tr!("settings.knowledge_base.panel.title")).class("skills-panel-header-title"),
                ),
                ToolButton::new(MI_ADD)
                    .on_click(create_collection)
                    .class("models-add-btn"),
            ]
        ]
    }
}

fn list_reactive() -> impl Widget {
    Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let registry = ctx.kb.registry.get();
        let active = ctx.kb.active_collection_id.get();
        let active_in_chat = ctx.kb.active_in_chat_ids.get();

        let body: Box<dyn Widget> = if registry.items.is_empty() {
            Box::new(
                Center::new().child(
                    Padding::all(24.0).child(
                        Text::new(tr!("settings.knowledge_base.panel.empty")).class("models-empty-list"),
                    ),
                ),
            )
        } else {
            let mut items: Vec<Box<dyn Widget>> = Vec::new();
            for meta in &registry.items {
                let id = meta.id.clone();
                let id_for_select = id.clone();
                let id_for_active = id.clone();
                let is_selected = active.as_deref() == Some(&id);
                let in_chat = active_in_chat.iter().any(|x| x == &id);
                let class = if is_selected { "kb-coll-row selected" } else { "kb-coll-row" };
                let badge = if in_chat {
                    tr!("settings.knowledge_base.panel.badge.active")
                } else {
                    tr!("settings.knowledge_base.panel.badge.inactive")
                };
                let line1 = meta.name.clone();
                let line2 = tr!(
                    "settings.knowledge_base.panel.subtitle",
                    docs = meta.document_count, chunks = meta.chunk_count, badge = badge
                );
                let row: Box<dyn Widget> = Box::new(mgui! {
                    DecoratedBox::new().class(class) => [
                        GestureDetector::new()
                            .on_click(move || {
                                let ctx = use_context::<AppCtx>();
                                ctx.kb.active_collection_id.set(Some(id_for_select.clone()));
                            })
                            .child(Padding::all(12.0).child(mgui! {
                                Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                    Text::new(line1.clone()).class("kb-coll-title"),
                                    Text::new(line2.clone()).class("kb-coll-subtitle"),
                                    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                                        Button::new(
                                            if in_chat {
                                                tr!("settings.knowledge_base.panel.remove_from_chat")
                                            } else {
                                                tr!("settings.knowledge_base.panel.use_in_chat")
                                            }
                                        )
                                            .class("kb-coll-toggle")
                                            .on_click({
                                                let id = id_for_active.clone();
                                                move || toggle_in_chat(&id)
                                            }),
                                    ]
                                ]
                            })),
                    ]
                });
                items.push(row);
            }
            Box::new(
                ScrollView::new().vertical().child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(items),
                ),
            )
        };
        vec![body]
    })
}

fn create_collection() {
    let app = use_context::<AppCtx>();
    let cfg = crate::config::AppConfig::load().kb;
    let model_basename = std::path::Path::new(&cfg.embedder_model_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bge-m3")
        .to_string();
    // dim определим после загрузки эмбеддера; пока используем 1024 (BGE-M3).
    let dim = app.kb.get_embedder().map(|e| e.dim() as i32).unwrap_or(1024);
    // RwSignal::update не возвращает значение из замыкания; используем
    // shared slot, чтобы вытащить созданную мету наружу.
    let slot: std::sync::Arc<std::sync::Mutex<Option<crate::kb::CollectionMeta>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot2 = slot.clone();
    // `tr!` подписывается на сигнал языка — переводим до `update`, а не внутри.
    let default_name = tr!("settings.knowledge_base.panel.new_name");
    app.kb.registry.update(move |reg| {
        match reg.create(
            default_name,
            model_basename,
            dim,
            cfg.chunk_target_tokens as i32,
            cfg.chunk_overlap_tokens as i32,
        ) {
            Ok(m) => {
                *slot2.lock().unwrap() = Some(m);
            }
            Err(e) => {
                log::error!("kb create: {e}");
            }
        }
    });
    let created = slot.lock().unwrap().take();
    match created {
        Some(meta) => {
            app.kb.active_collection_id.set(Some(meta.id.clone()));
            app.notifications.success(tr!("settings.knowledge_base.panel.created", name = meta.name));
        }
        None => {
            app.notifications.error(tr!("settings.knowledge_base.panel.create_failed"));
        }
    }
}

fn toggle_in_chat(id: &str) {
    let app = use_context::<AppCtx>();
    let id = id.to_string();
    app.kb.active_in_chat_ids.update(move |list| {
        if let Some(pos) = list.iter().position(|x| x == &id) {
            list.remove(pos);
        } else {
            list.push(id.clone());
        }
    });
}
