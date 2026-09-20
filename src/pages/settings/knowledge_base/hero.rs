//! Шапка коллекции: имя, счётчики, «в чате», удаление.

use syngui::mgui;
use syngui::prelude::*;

use crate::context::AppCtx;
use crate::icons::*;

pub(super) fn view(collection_id: String) -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let name = ctx
        .kb
        .registry
        .get_untracked()
        .get(&collection_id)
        .map(|m| m.name.clone())
        .unwrap_or_default();

    let id_name = collection_id.clone();
    // Имя пишется в БД по Enter / уходу фокуса, а не на каждую букву.
    let name_field = TextField::with_text(name)
        .placeholder(tr!("settings.knowledge_base.name.placeholder"))
        .submit_on_focus_lost(true)
        .on_submit(move |s| rename(&id_name, s))
        .class("kb-hero-name");

    Box::new(mgui! {
        DecoratedBox::new().class("settings-card kb-hero") => [
            Padding::symmetric(24.0, 20.0) => [
                Row::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    DecoratedBox::new().class("kb-hero-badge") => [
                        Center::new().child(Icon::new(MI_MENU_BOOK).class("kb-hero-badge-icon")),
                    ],
                    DecoratedBox::new().class("grow kb-min0") => [
                        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            name_field,
                            stats(collection_id.clone()),
                        ]
                    ],
                    actions(collection_id),
                ]
            ]
        ]
    })
}

/// Счётчики под именем — свой Reactive: они меняются после каждой индексации.
fn stats(collection_id: String) -> impl Widget {
    Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let registry = ctx.kb.registry.get();
        let Some(meta) = registry.get(&collection_id) else {
            return Vec::new();
        };
        let chips: Vec<Box<dyn Widget>> = vec![
            stat_chip(
                MI_DESCRIPTION,
                tr!("settings.knowledge_base.stat.documents"),
                meta.document_count.to_string(),
            ),
            stat_chip(
                MI_LAYERS,
                tr!("settings.knowledge_base.stat.chunks"),
                meta.chunk_count.to_string(),
            ),
            stat_chip(
                MI_HUB,
                tr!("settings.knowledge_base.stat.model"),
                format!("{} · {}", meta.embedding_model, meta.embedding_dim),
            ),
        ];
        vec![Box::new(Flex::row().wrap().gap(8.0).children(chips)) as Box<dyn Widget>]
    })
}

fn stat_chip(icon: &str, label: String, value: String) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new().class("kb-stat-chip").child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon.to_string()).class("kb-stat-chip-icon"))
                .child(Text::new(label).class("kb-stat-chip-label"))
                .child(Text::new(value).class("kb-stat-chip-value")),
        ),
    )
}

/// Справа: «использовать в чате» и удаление с подтверждением на месте.
fn actions(collection_id: String) -> impl Widget {
    let confirm = use_signal(false);
    Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let id = collection_id.clone();
        if confirm.get() {
            let id_del = id.clone();
            return vec![Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(tr!("settings.knowledge_base.delete.confirm")).class("kb-confirm-text"))
                    .child(
                        Button::new(tr!("settings.knowledge_base.delete.yes"))
                            .on_click(move || delete(&id_del))
                            .class("kb-btn kb-btn-danger"),
                    )
                    .child(
                        Button::new(tr!("app.cancel"))
                            .on_click(move || confirm.set(false))
                            .class("kb-btn"),
                    ),
            ) as Box<dyn Widget>];
        }

        let in_chat = ctx.kb.active_in_chat_ids.get().iter().any(|x| x == &id);
        let id_toggle = id.clone();
        let chat_class = if in_chat { "kb-chat-pill on" } else { "kb-chat-pill" };
        let chat_label = if in_chat {
            tr!("settings.knowledge_base.in_chat.on")
        } else {
            tr!("settings.knowledge_base.in_chat.off")
        };
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Button::new(chat_label)
                        .leading_icon(if in_chat { MI_CHECK } else { MI_CHAT })
                        .on_click(move || toggle_in_chat(&id_toggle))
                        .class(chat_class),
                )
                .child(
                    ToolButton::new(MI_DELETE)
                        .tooltip(tr!("settings.knowledge_base.delete.tooltip"))
                        .on_click(move || confirm.set(true))
                        .class("kb-icon-btn danger"),
                ),
        ) as Box<dyn Widget>]
    })
}

fn rename(id: &str, raw: &str) {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return;
    }
    let id = id.to_string();
    use_context::<AppCtx>().kb.registry.update(move |reg| {
        let db_path = reg.kb_dir.join(format!("{id}.sqlite"));
        let Some(meta) = reg.items.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if meta.name == name {
            return;
        }
        meta.name = name;
        match crate::kb::store::Store::open(&db_path) {
            Ok(mut store) => {
                if let Err(e) = store.upsert_collection(meta) {
                    log::error!("kb rename: {e}");
                }
            }
            Err(e) => log::error!("kb rename: {e}"),
        }
        reg.items.sort_by(|a, b| a.name.cmp(&b.name));
    });
}

pub(super) fn toggle_in_chat(id: &str) {
    let id = id.to_string();
    use_context::<AppCtx>().kb.active_in_chat_ids.update(move |list| {
        match list.iter().position(|x| x == &id) {
            Some(pos) => {
                list.remove(pos);
            }
            None => list.push(id),
        }
    });
}

fn delete(id: &str) {
    let app = use_context::<AppCtx>();
    // Идёт индексация в эту коллекцию — сначала остановить её.
    if app
        .kb
        .ingest_progress
        .get_untracked()
        .is_some_and(|p| p.collection_id == id)
    {
        app.notifications.warning(tr!("settings.knowledge_base.delete.busy"));
        return;
    }
    let id = id.to_string();
    let failed = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let failed_in = failed.clone();
    let id_reg = id.clone();
    app.kb.registry.update(move |reg| {
        if let Err(e) = reg.delete(&id_reg) {
            *failed_in.lock().unwrap() = Some(e.to_string());
        }
    });
    if let Some(e) = failed.lock().unwrap().take() {
        app.notifications.error(tr!("settings.knowledge_base.delete.failed", error = e));
        return;
    }
    app.kb.active_in_chat_ids.update(|list| list.retain(|x| x != &id));
    // Открыть соседнюю коллекцию, а не пустую страницу.
    let next = app.kb.registry.get_untracked().items.first().map(|m| m.id.clone());
    app.kb.active_collection_id.set(next);
}
