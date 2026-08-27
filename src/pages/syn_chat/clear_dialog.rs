//! Portal-диалог подтверждения очистки ленты активного чата.
//!
//! Источник — [`SynChatCtx::pending_clear`]: кнопка «Очистить» в шапке чата
//! только взводит сигнал, а `session::clear_chat` вызывается отсюда, после
//! подтверждения. Чат при этом остаётся (название, параметры, плитка в
//! рейле) — уходят только сообщения; это способ вести всю переписку в одном
//! чате, не плодя новые.
//!
//! Смонтирован в shell'е (`lib.rs::build_app`) рядом с `archive_dialog`;
//! разметка и классы — те же.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;

use crate::icons::{MI_CLEAR_ALL, MI_CLOSE};
use crate::syn_chat::{session, SynChatCtx};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let want = ctx.pending_clear.get();
        if is_open.get_untracked() != want {
            is_open.set(want);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(|| {
            use_context::<SynChatCtx>().pending_clear.set(false);
        })
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynChatCtx>();
            if !ctx.pending_clear.get() {
                return vec![Box::new(
                    DecoratedBox::new().class("code-editor-dialog-empty"),
                )];
            }
            let active = ctx.active_chat_id.get_untracked();
            let title = active
                .as_ref()
                .and_then(|id| {
                    ctx.chats
                        .get_untracked()
                        .into_iter()
                        .find(|m| &m.id == id)
                        .map(|m| m.title)
                })
                .unwrap_or_default();
            let count = ctx.messages.get_untracked().len();
            vec![Box::new(confirm_card(title, count))]
        }))
}

fn confirm_card(title: String, count: usize) -> impl Widget {
    let confirm = || {
        let ctx = use_context::<SynChatCtx>();
        ctx.pending_clear.set(false);
        session::clear_chat();
    };
    let cancel = || {
        use_context::<SynChatCtx>().pending_clear.set(false);
    };
    let shown_title = if title.trim().is_empty() {
        tr!("chat.archive_dialog.untitled")
    } else {
        title
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("chat.clear_dialog.title")).class("code-editor-dialog-title"),
                    Text::new(tr!("chat.clear_dialog.hint", title = shown_title, n = count))
                        .class("code-editor-dialog-hint"),
                    Text::new(tr!("chat.clear_dialog.detail"))
                        .class("code-editor-dialog-path"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("chat.clear_dialog.confirm"))
                                .leading_icon(MI_CLEAR_ALL)
                                .on_click(confirm)
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}
