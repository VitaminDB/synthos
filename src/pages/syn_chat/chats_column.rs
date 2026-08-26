//! Левая колонка Syn-чатов — visual parity с `components::chats_column`,
//! но с CRUD-callback'ами syn_chat::registry.

use std::sync::Arc;

use syngui::mgui;
use syngui::prelude::*;

use crate::components::chat_item;
use crate::icons::*;
use crate::syn_chat::{registry, SynChatCtx};

pub fn view() -> impl Widget {
    let content = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header_row())
        .child(filter_row())
        .child(group_header_reactive())
        .child(chats_list_reactive());

    let scroll = ScrollView::new().vertical().child(content);

    DecoratedBox::new().class("chats-column").child(scroll)
}

fn header_row() -> impl Widget {
    DecoratedBox::new().class("chats-header-row").child(mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(tr!("chat.chats_column.title")).class("chats-column-title"),
                ToolButton::new(MI_ADD)
                    .tooltip(tr!("chat.chats_column.new_chat"))
                    .on_click(|| { registry::create_new(); })
                    .class("chats-header-add"),
            ]
    })
}

fn filter_row() -> impl Widget {
    DecoratedBox::new().class("chats-filter-wrap").child(mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("filter-chip").child(mgui! {
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_FILTER_LIST).class("filter-chip-icon"),
                    Text::new(tr!("chat.chats_column.filter")).class("filter-chip-text"),
                    DecoratedBox::new().class("filter-chip-dot"),
                ]
            }),
        ]
    })
}

fn group_header_reactive() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let count = ctx.chats.get().len();
        let label = tr!("chat.chats_column.group_label");
        let text = if count == 0 {
            label
        } else {
            format!("{label} ({count})")
        };
        DecoratedBox::new().class("chats-group-wrap").child(mgui! {
            Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(text).class("chats-group-label"),
                Icon::new(MI_EXPAND_MORE).class("chats-group-chev"),
            ]
        })
    }
}

fn chats_list_reactive() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let chats = ctx.chats.get();
        let active = ctx.active_chat_id.get();

        let on_select: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|id: &str| registry::select(id));
        // Корзина не удаляет сразу: удаление необратимо, поэтому она лишь
        // взводит `pending_delete`, а `registry::delete` зовёт диалог
        // подтверждения (`delete_dialog`).
        let on_delete: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|id: &str| {
            let ctx = use_context::<SynChatCtx>();
            let meta = ctx.chats.get_untracked().into_iter().find(|m| m.id == id);
            ctx.pending_delete.set(meta);
        });

        let inner: Column = if chats.is_empty() {
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(empty_hint())
        } else {
            let mut items: Vec<Box<dyn Widget>> = Vec::with_capacity(chats.len());
            for meta in chats.iter() {
                let selected = active.as_deref() == Some(&meta.id);
                items.push(chat_item::row_generic(
                    meta,
                    selected,
                    on_select.clone(),
                    on_delete.clone(),
                ));
            }
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(items)
        };

        DecoratedBox::new().class("chats-list-wrap").child(inner)
    }
}

fn empty_hint() -> impl Widget {
    DecoratedBox::new()
        .class("chats-empty-hint")
        .child(Text::new(tr!("chat.chats_column.empty_hint")).class("chats-empty-hint-text"))
}
