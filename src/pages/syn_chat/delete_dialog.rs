//! Portal-диалог подтверждения удаления чата.
//!
//! Источник — [`SynChatCtx::pending_delete`]: корзина в строке списка
//! (`chats_column`) только взводит сигнал, а необратимое
//! `registry::delete` (файл чата + GC блобов вложений) вызывается уже
//! отсюда, после явного подтверждения.
//!
//! Разметка и классы — те же, что у `code_editor::dialogs`
//! (`code-editor-dialog-card` / `-danger` / `-title` / `-hint` / `-path` /
//! `-btn-secondary` / `-btn-danger`), чтобы подтверждение удаления
//! выглядело одинаково во всём приложении и не заводило второй набор
//! стилей.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;

use crate::icons::{MI_CLOSE, MI_DELETE};
use crate::syn_chat::{registry, SynChatCtx};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let has = ctx.pending_delete.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(|| {
            use_context::<SynChatCtx>().pending_delete.set(None);
        })
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynChatCtx>();
            let Some(meta) = ctx.pending_delete.get() else {
                return vec![Box::new(
                    DecoratedBox::new().class("code-editor-dialog-empty"),
                )];
            };
            vec![Box::new(confirm_card(meta.id, meta.title))]
        }))
}

fn confirm_card(id: String, title: String) -> impl Widget {
    let confirm = move || {
        let ctx = use_context::<SynChatCtx>();
        ctx.pending_delete.set(None);
        registry::delete(&id);
    };
    let cancel = || {
        use_context::<SynChatCtx>().pending_delete.set(None);
    };
    let shown_title = if title.trim().is_empty() {
        "Без названия".to_string()
    } else {
        title
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-dialog-danger") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new("Удалить чат?").class("code-editor-dialog-title"),
                    Text::new(format!("«{shown_title}» — действие нельзя отменить."))
                        .class("code-editor-dialog-hint"),
                    Text::new("Переписка и вложения, на которые не ссылаются другие чаты, будут удалены с диска.")
                        .class("code-editor-dialog-path"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new("Отмена")
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new("Удалить")
                                .leading_icon(MI_DELETE)
                                .on_click(confirm)
                                .class("code-editor-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}
