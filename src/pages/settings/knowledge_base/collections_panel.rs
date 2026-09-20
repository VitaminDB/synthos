//! Правая колонка вкладки «Базы знаний» — список коллекций + «+».

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::CollectionMeta;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("skills-panel kb-panel").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("grow").child(
                Padding::symmetric(4.0, 4.0).child(list_reactive()),
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
                    .tooltip(tr!("settings.knowledge_base.panel.create"))
                    .on_click(create_collection)
                    .class("skills-panel-add-btn"),
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

        if registry.items.is_empty() {
            return vec![Box::new(empty_state()) as Box<dyn Widget>];
        }
        let rows: Vec<Box<dyn Widget>> = registry
            .items
            .iter()
            .map(|meta| {
                collection_row(
                    meta,
                    active.as_deref() == Some(meta.id.as_str()),
                    active_in_chat.iter().any(|x| x == &meta.id),
                )
            })
            .collect();
        vec![Box::new(
            ScrollView::new().vertical().child(
                Column::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(rows),
            ),
        ) as Box<dyn Widget>]
    })
}

fn empty_state() -> impl Widget {
    Center::new().child(mgui! {
        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("skill-empty-bubble") => [
                Center::new().child(Icon::new(MI_MENU_BOOK).class("skill-empty-icon")),
            ],
            Text::new(tr!("settings.knowledge_base.panel.empty.title")).class("skill-empty-title"),
            Padding::symmetric(20.0, 0.0).child(
                Text::new(tr!("settings.knowledge_base.panel.empty")).class("skill-empty-text"),
            ),
        ]
    })
}

/// Строка коллекции — зеркало строки скила: иконка, имя, счётчики; справа —
/// переключатель «в чате». Коллекция, подключённая к чату, видна по залитой
/// иконке, а не по строчке текста.
fn collection_row(meta: &CollectionMeta, selected: bool, in_chat: bool) -> Box<dyn Widget> {
    let class = if selected { "skill-list-row selected" } else { "skill-list-row" };
    let icon_wrap = if in_chat { "skill-list-icon-wrap kb-coll-in-chat" } else { "skill-list-icon-wrap" };
    let icon_class = if in_chat { "skill-list-icon kb-coll-in-chat-icon" } else { "skill-list-icon" };
    let subtitle = tr!(
        "settings.knowledge_base.panel.subtitle",
        docs = meta.document_count,
        chunks = meta.chunk_count
    );
    let id_select = meta.id.clone();
    let id_toggle = meta.id.clone();
    let chat_tip = if in_chat {
        tr!("settings.knowledge_base.panel.remove_from_chat")
    } else {
        tr!("settings.knowledge_base.panel.use_in_chat")
    };

    // Клик по строке открывает коллекцию. Кнопка «в чате» лежит внутри
    // детектора, но событие забирает первой (глубокая цель) — строку она
    // не открывает; это закреплено тестом `knowledge_base_page`.
    let content = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class(icon_wrap)
                .child(Center::new().child(Icon::new(MI_MENU_BOOK).class(icon_class))),
        )
        .child(
            DecoratedBox::new().class("grow kb-min0").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(meta.name.clone()).max_lines(1).class("skill-list-title"))
                    .child(Text::new(subtitle).max_lines(1).class("skill-list-subtitle")),
            ),
        )
        .child(
            ToolButton::new(if in_chat { MI_CHECK } else { MI_CHAT })
                .tooltip(chat_tip)
                .active(in_chat)
                .on_click(move || super::hero::toggle_in_chat(&id_toggle))
                .class(if in_chat { "skill-list-action kb-coll-chat on" } else { "skill-list-action kb-coll-chat" }),
        );

    Box::new(Padding::symmetric(4.0, 3.0).child(
        DecoratedBox::new().class(class).child(
            GestureDetector::new()
                .on_click(move || {
                    use_context::<AppCtx>().kb.active_collection_id.set(Some(id_select.clone()));
                })
                .child(Padding::symmetric(12.0, 10.0).child(content)),
        ),
    ))
}

pub fn create_collection() {
    let app = use_context::<AppCtx>();
    let cfg = crate::config::AppConfig::load().kb;
    // Имя модели в мете коллекции — по найденному файлу (без расширения).
    let model_basename = app
        .kb
        .model_paths
        .get_untracked()
        .embedder
        .and_then(|f| f.path.file_stem().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| crate::kb::models::ModelKind::Embedder.stem().to_string());
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
