//! Inline-rename для имени шаблона в карточке.
//!
//! Двойной клик по тексту имени = переключение Text → TextField.
//! Enter сохраняет (через [`templates::rename`]). Builtin-шаблоны не
//! редактируются — `editing.set(true)` не вызывается.

use syngui::prelude::*;
use syngui::widgets::{DecoratedBox, GestureDetector, Reactive, TextField};

use crate::context::AppCtx;
use crate::templates;

/// Per-card state inline-rename.
#[derive(Clone, Copy)]
pub struct EditableLabelState {
    pub editing: RwSignal<bool>,
    pub display: RwSignal<String>,
}

impl EditableLabelState {
    pub fn new(initial: &str) -> Self {
        Self {
            editing: use_signal(false),
            display: use_signal(initial.to_string()),
        }
    }
}

/// Виджет — Reactive обёртка над `editing`-сигналом.
pub fn editable_label(state: EditableLabelState, template_id: String, builtin: bool) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let editing = state.editing.get();

        if !editing {
            let display = state.display.get();
            let label = Text::new(display).class("ne-template-card-name");
            let detector = if builtin {
                GestureDetector::new().child(label)
            } else {
                GestureDetector::new()
                    .on_double_click(move || {
                        state.editing.set(true);
                    })
                    .child(label)
            };
            return vec![Box::new(detector)];
        }

        let initial = state.display.get_untracked();
        let template_id_save = template_id.clone();
        let field = TextField::with_text(initial)
            .placeholder("Имя шаблона")
            .on_submit(move |new_name: &str| {
                let app = use_context::<AppCtx>();
                let trimmed = new_name.trim();
                if trimmed.is_empty() {
                    state.editing.set(false);
                    return;
                }
                match templates::rename(&template_id_save, trimmed) {
                    Ok(updated) => {
                        state.display.set(updated.name.clone());
                        app.notifications.success("Шаблон переименован");
                        super::bump_revision();
                    }
                    Err(e) => {
                        app.notifications.error(format!("Не удалось переименовать: {e}"));
                    }
                }
                state.editing.set(false);
            })
            .class("ne-template-card-name-edit");

        vec![Box::new(
            DecoratedBox::new()
                .child(field)
                .class("ne-template-card-name-edit-host"),
        )]
    })
}
