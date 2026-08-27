//! Portal-диалог подтверждения переноса чата в архив.
//!
//! Источник — [`SynChatCtx::pending_archive`]: корзина в шапке чата и
//! «Закрыть» в контекстном меню плитки рейла только взводят сигнал, а
//! `registry::archive` вызывается уже отсюда, после подтверждения. Файл чата
//! при этом остаётся на диске — вернуть его можно из Настройки → Архив,
//! удаление насовсем живёт там же.
//!
//! Смонтирован в shell'е (`lib.rs::build_app`): плитку чата закрывают с
//! любой страницы. Разметка и классы — те же, что у `code_editor::dialogs`.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;

use crate::icons::{MI_ARCHIVE, MI_CLOSE};
use crate::syn_chat::{registry, SynChatCtx};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let has = ctx.pending_archive.get().is_some();
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
            use_context::<SynChatCtx>().pending_archive.set(None);
        })
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynChatCtx>();
            let Some(meta) = ctx.pending_archive.get() else {
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
        ctx.pending_archive.set(None);
        registry::archive(&id);
    };
    let cancel = || {
        use_context::<SynChatCtx>().pending_archive.set(None);
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
                    Text::new(tr!("chat.archive_dialog.title")).class("code-editor-dialog-title"),
                    Text::new(tr!("chat.archive_dialog.hint", title = shown_title))
                        .class("code-editor-dialog-hint"),
                    Text::new(tr!("chat.archive_dialog.detail"))
                        .class("code-editor-dialog-path"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("chat.archive_dialog.confirm"))
                                .leading_icon(MI_ARCHIVE)
                                .on_click(confirm)
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}
