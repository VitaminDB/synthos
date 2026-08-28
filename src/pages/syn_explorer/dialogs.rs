//! Модальные диалоги SynExplorer: NewPackage / ConfirmDeleteFile /
//! ConfirmCloseUnsaved / RenameFile / Error.
//!
//! Один Portal в `view()` слушает `ctx.pending_dialog` и подменяет содержимое
//! по варианту. Закрытие — Esc, клик вне модала, кнопка отмены.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::TextField;

use crate::icons::{MI_CHECK, MI_CLOSE, MI_DELETE};

use super::actions;
use super::pack_dialogs;
use super::state::{DialogKind, SynExplorerCtx};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynExplorerCtx>();
        let has = ctx.pending_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        // Промах мимо карточки не должен стирать настроенный состав пакета
        // или обрывать вид на идущую упаковку. Закрыть можно кнопкой или
        // Escape — то есть осознанно.
        .close_on_outside_click(false)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let Some(kind) = ctx.pending_dialog.get() else {
                return vec![Box::new(DecoratedBox::new().class("syn-dialog-empty"))];
            };
            let card: Box<dyn Widget> = match kind {
                DialogKind::PackConfirm => Box::new(pack_dialogs::confirm_card(ctx.wizard)),
                DialogKind::PackWizard => Box::new(pack_dialogs::wizard_card(ctx.wizard)),
                DialogKind::ConfirmDeleteFile { name } => Box::new(confirm_delete_card(name)),
                DialogKind::ConfirmCloseUnsaved => Box::new(confirm_close_card()),
                DialogKind::RenameFile { old } => Box::new(rename_card(old)),
                DialogKind::Error { title, message } => Box::new(error_card(title, message)),
            };
            vec![card]
        }))
}

fn confirm_delete_card(name: String) -> impl Widget {
    let name_for_btn = name.clone();
    let confirm = move || {
        let ctx = use_context::<SynExplorerCtx>();
        if let Some(active) = ctx.active_untracked() {
            actions::confirm_delete_file(active, name_for_btn.clone());
        }
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-danger") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.dialog.confirm_delete.title")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.confirm_delete.hint", name = name))
                        .class("syn-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("app.delete"))
                                .leading_icon(MI_DELETE)
                                .on_click(confirm)
                                .class("syn-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}

fn confirm_close_card() -> impl Widget {
    let proceed = || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::force_close_active_bundle(ctx);
        ctx.close_dialog();
    };
    let reload = || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::force_reload_active(ctx);
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.unsaved_edits")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.confirm_close.hint"))
                        .class("syn-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("explorer.dialog.confirm_close.reload"))
                                .on_click(reload)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("explorer.dialog.confirm_close.discard"))
                                .leading_icon(MI_DELETE)
                                .on_click(proceed)
                                .class("syn-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}

fn rename_card(old: String) -> impl Widget {
    // RenameFile в SynExplorer не вызывается из toolbar — оставляем
    // диалог как заглушку для будущего; UI и handler есть.
    let ctx = use_context::<SynExplorerCtx>();
    ctx.rename_buffer.set(old.clone());
    let old_for_action = old.clone();
    let buffer = ctx.rename_buffer;

    let confirm = move || {
        let new_name = buffer.get_untracked();
        let ctx = use_context::<SynExplorerCtx>();
        if let Some(active) = ctx.active_untracked() {
            if !new_name.is_empty() && new_name != old_for_action {
                active.pending_ops.update(|v| {
                    v.push(super::state::PendingOp::Rename {
                        old: old_for_action.clone(),
                        new: new_name.clone(),
                    });
                });
            }
        }
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };

    mgui! {
        DecoratedBox::new().class("syn-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.dialog.rename.title")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.rename.current_name", name = old)).class("syn-dialog-hint"),
                    TextField::with_text(old.clone())
                        .placeholder(tr!("explorer.dialog.rename.placeholder"))
                        .on_change(move |s| buffer.set(s.to_string()))
                        .class("syn-dialog-input"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(confirm)
                                .class("syn-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn error_card(title: String, message: String) -> impl Widget {
    let close = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-error") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(title).class("syn-dialog-title"),
                    Text::new(message).class("syn-dialog-error-body"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(close)
                                .class("syn-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}
