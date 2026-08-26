//! Модальные диалоги CRUD-операций над скилами: Create / Edit / Delete.
//!
//! Один [`Portal`] монтируется в `pages::settings::view()` и слушает
//! `AppCtx.skills_dialog`. Содержимое реактивно подменяется по варианту
//! [`crate::context::SkillDialogKind`] — точно тем же приёмом, что
//! используется в `pages::code_editor::dialogs`.
//!
//! ВАЖНО: `TextField::on_submit` в syngui стреляет не только на Enter, но и
//! на FocusLost (mobile-friendly, см. `syngui/.../text_field.rs:634`). В
//! формах с >1 TextField это означает: клик в соседнее поле = submit. Поэтому
//! в Create / Edit диалогах `on_submit` НЕ используется — пользователь
//! подтверждает кнопкой «Создать» / «OK».

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::TextField;

use crate::context::{AppCtx, SkillDialogKind};
use crate::icons::{MI_CHECK, MI_CLOSE, MI_DELETE};
use crate::skills;

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<AppCtx>();
        let has = ctx.skills_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<AppCtx>();
            let Some(kind) = ctx.skills_dialog.get() else {
                return vec![Box::new(DecoratedBox::new().class("skill-dialog-empty"))];
            };
            let card: Box<dyn Widget> = match kind {
                SkillDialogKind::Create => Box::new(create_card()),
                SkillDialogKind::Edit {
                    id,
                    current_name,
                    current_description,
                } => Box::new(edit_card(id, current_name, current_description)),
                SkillDialogKind::Delete { id, name } => Box::new(delete_card(id, name)),
            };
            vec![card]
        }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Create
// ─────────────────────────────────────────────────────────────────────────────

fn create_card() -> impl Widget {
    let name = use_signal(String::new());
    let description = use_signal(String::new());

    let confirm = move || {
        let n = name.get_untracked();
        let d = description.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        match skills::create(&n, &d, "") {
            Ok(skill) => {
                let ctx = use_context::<AppCtx>();
                let new_id = skill.id.clone();
                ctx.skills.update(|list| {
                    list.push(skill);
                    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                });
                ctx.skills_selected_id.set(Some(new_id));
                ctx.skills_dialog.set(None);
            }
            Err(e) => {
                tracing::warn!(error = %e, "create skill failed");
            }
        }
    };
    let cancel = || close_dialog();

    mgui! {
        DecoratedBox::new().class("skill-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("settings.skills.dialog.create.title")).class("skill-dialog-title"),
                Text::new(tr!("settings.skills.dialog.create.hint"))
                    .class("skill-dialog-hint"),
                TextField::new()
                    .placeholder(tr!("settings.skills.dialog.name.placeholder"))
                    .on_change(move |s| name.set(s.to_string())),
                TextField::new()
                    .placeholder(tr!("settings.skills.dialog.create.description.placeholder"))
                    .on_change(move |s| description.set(s.to_string())),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(cancel)
                        .class("skill-dialog-btn-secondary"),
                    Button::new(tr!("settings.skills.dialog.create.confirm"))
                        .leading_icon(MI_CHECK)
                        .on_click(confirm)
                        .class("skill-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Edit (имя + описание)
// ─────────────────────────────────────────────────────────────────────────────

fn edit_card(id: String, current_name: String, current_description: String) -> impl Widget {
    let name = use_signal(current_name.clone());
    let description = use_signal(current_description.clone());

    let confirm = {
        let id = id.clone();
        let baseline_name = current_name.clone();
        let baseline_desc = current_description.clone();
        move || {
            let new_name = name.get_untracked();
            let new_desc = description.get_untracked();
            // Без изменений — просто закрываем.
            if new_name.trim().is_empty()
                || (new_name == baseline_name && new_desc == baseline_desc)
            {
                close_dialog();
                return;
            }
            match skills::update_meta(&id, &new_name, &new_desc) {
                Ok(updated) => {
                    let ctx = use_context::<AppCtx>();
                    let old_id = id.clone();
                    let new_id = updated.id.clone();
                    ctx.skills.update(|list| {
                        if let Some(pos) = list.iter().position(|s| s.id == old_id) {
                            list[pos] = updated.clone();
                        }
                        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    });
                    if ctx.skills_selected_id.get_untracked().as_deref() == Some(old_id.as_str()) {
                        ctx.skills_selected_id.set(Some(new_id.clone()));
                    }
                    if old_id != new_id {
                        ctx.skills_active.update(|list| {
                            for k in list.iter_mut() {
                                if *k == old_id {
                                    *k = new_id.clone();
                                }
                            }
                        });
                    }
                    ctx.skills_dialog.set(None);
                }
                Err(e) => tracing::warn!(error = %e, "edit skill failed"),
            }
        }
    };
    let cancel = || close_dialog();

    mgui! {
        DecoratedBox::new().class("skill-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("settings.skills.dialog.edit.title")).class("skill-dialog-title"),
                TextField::with_text(current_name)
                    .placeholder(tr!("settings.skills.dialog.name.placeholder"))
                    .on_change(move |s| name.set(s.to_string())),
                TextField::with_text(current_description)
                    .placeholder(tr!("settings.skills.dialog.edit.description.placeholder"))
                    .on_change(move |s| description.set(s.to_string())),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(cancel)
                        .class("skill-dialog-btn-secondary"),
                    Button::new(tr!("app.ok"))
                        .leading_icon(MI_CHECK)
                        .on_click(confirm)
                        .class("skill-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Delete
// ─────────────────────────────────────────────────────────────────────────────

fn delete_card(id: String, name: String) -> impl Widget {
    let id_for_btn = id.clone();
    let confirm = move || {
        if let Err(e) = skills::delete(&id_for_btn) {
            tracing::warn!(error = %e, "delete skill failed");
            return;
        }
        let ctx = use_context::<AppCtx>();
        let id = id_for_btn.clone();
        ctx.skills.update(|list| list.retain(|s| s.id != id));
        if ctx.skills_selected_id.get_untracked().as_deref() == Some(id.as_str()) {
            ctx.skills_selected_id.set(None);
        }
        ctx.skills_active.update(|list| list.retain(|k| *k != id));
        ctx.skills_dialog.set(None);
    };
    let cancel = || close_dialog();
    let label = name.clone();

    mgui! {
        DecoratedBox::new().class("skill-dialog-card skill-dialog-danger") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("settings.skills.dialog.delete.title")).class("skill-dialog-title"),
                Text::new(tr!("settings.skills.dialog.delete.hint", name = label))
                    .class("skill-dialog-hint"),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(cancel)
                        .class("skill-dialog-btn-secondary"),
                    Button::new(tr!("app.delete"))
                        .leading_icon(MI_DELETE)
                        .on_click(confirm)
                        .class("skill-dialog-btn-danger"),
                ],
            ]
        ]
    }
}

fn close_dialog() {
    let ctx = use_context::<AppCtx>();
    ctx.skills_dialog.set(None);
}
