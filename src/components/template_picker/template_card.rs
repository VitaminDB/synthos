//! Карточка одного шаблона в сетке окна выбора.
//!
//! Слои:
//! - DecoratedBox `.ne-template-card` (— hover/selected через MSS)
//! - GestureDetector — двойной клик = открыть в новой вкладке (или
//!   активировать существующую через `EditorWorkspace::open_template`,
//!   которая заодно закрывает окно выбора).
//! - ContextMenu — Open / Rename / Duplicate / Delete (custom-only).
//!
//! Inline-rename: двойной клик по тексту имени переключает Text →
//! TextField в [`super::editable_label`].

use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{DecoratedBox, GestureDetector, Padding, Reactive, Row};

use crate::context::AppCtx;
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::template_preview;
use crate::templates::{self, Template, TemplateKind};

use super::editable_label::{editable_label, EditableLabelState};

pub fn view(t: Template) -> impl Widget {
    // Per-card editing-сигнал — изолирован между карточками.
    let edit_state = EditableLabelState::new(&t.name);
    // Confirm-delete dialog state — Dialog внутри уже Portal-овский overlay,
    // рендерится как сосед карточки. Карточка сама делает только
    // `delete_open.set(true)`.
    let delete_open = use_signal(false);
    let delete_open_for_dialog = delete_open;

    let preview = DecoratedBox::new()
        .child(template_preview::view(&t))
        .class("ne-template-card-preview");

    let kind_chip_text = match t.kind {
        TemplateKind::Full => "Full",
        TemplateKind::Subgraph => "Subgraph",
    };
    let kind_class = if t.kind == TemplateKind::Full {
        "ne-template-chip ne-template-chip--full"
    } else {
        "ne-template-chip ne-template-chip--subgraph"
    };

    let has_badge = t.builtin;

    let title_widget = editable_label(edit_state.clone(), t.id.clone(), t.builtin);

    let title_row_factory = {
        let kind_chip_text = kind_chip_text.to_string();
        let kind_class = kind_class.to_string();
        move || -> Vec<Box<dyn Widget>> {
            let mut row = Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Start);
            row = row.child(Text::new(&kind_chip_text).class(kind_class.clone()));
            if has_badge {
                row = row.child(Text::new("Builtin").class("ne-template-badge"));
            }
            vec![Box::new(row)]
        }
    };

    let body = mgui! {
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                preview,
                title_widget,
                Reactive::new(title_row_factory),
                description_text(&t.description),
            ]
    };

    let card_class = "ne-template-card";

    let inner = DecoratedBox::new()
        .child(Padding::all(10.0).child(body))
        .class(card_class);

    // ContextMenu: Open / Rename / Duplicate / Delete. Builtin позволяет
    // только Open/Duplicate — Rename/Delete блокируем filter'ом ниже.
    let menu_items = if t.builtin {
        vec![
            MenuItem::new("open", "Открыть в новой вкладке"),
            MenuItem::new("duplicate", "Дублировать в Свои"),
        ]
    } else {
        vec![
            MenuItem::new("open", "Открыть в новой вкладке"),
            MenuItem::new("rename", "Переименовать"),
            MenuItem::new("duplicate", "Дублировать"),
            MenuItem::separator(),
            MenuItem::new("delete", "Удалить"),
        ]
    };

    let t_for_menu = t.clone();
    let edit_state_for_menu = edit_state.clone();
    let menu = ContextMenu::new()
        .child(
            GestureDetector::new()
                .on_double_click({
                    let t_for_open = t.clone();
                    move || {
                        let ws = use_context::<EditorWorkspace>();
                        ws.open_template(&t_for_open);
                    }
                })
                .child(inner),
        )
        .items(menu_items)
        .on_select(move |action| {
            let app = use_context::<AppCtx>();
            let ws = use_context::<EditorWorkspace>();
            match action {
                "open" => {
                    ws.open_template(&t_for_menu);
                }
                "rename" => {
                    edit_state_for_menu.editing.set(true);
                }
                "duplicate" => match templates::duplicate_to_custom(&t_for_menu) {
                    Ok(c) => {
                        app.notifications.success(format!("Создан: {}", c.name));
                        super::bump_revision();
                    }
                    Err(e) => {
                        app.notifications.error(format!("Не удалось дублировать: {e}"));
                    }
                },
                "delete" => {
                    delete_open_for_dialog.set(true);
                }
                _ => {}
            }
        });

    // Confirm-dialog рендерим как сосед карточки — Dialog внутри уже
    // Portal-овский overlay, перекрывает всё окно при is_open=true.
    let dialog = super::delete_dialog(delete_open, t.name.clone(), t.id.clone());

    mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            menu,
            dialog,
        ]
    }
}

fn description_text(desc: &str) -> impl Widget {
    if desc.is_empty() {
        // Возвращаем пустой DecoratedBox чтобы Column не падал на None.
        DecoratedBox::new()
            .child(Text::new("").class("ne-template-card-desc-empty"))
            .class("ne-template-card-desc-host")
    } else {
        DecoratedBox::new()
            .child(Text::new(desc).class("ne-template-card-desc"))
            .class("ne-template-card-desc-host")
    }
}

#[allow(dead_code)]
fn _unused_color_imp() -> Color {
    Color::new(0.0, 0.0, 0.0, 0.0)
}
