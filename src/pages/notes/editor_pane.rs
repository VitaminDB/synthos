//! Центр «Заметок»: WYSIWYG-редактор активной страницы.
//!
//! Каждая открытая страница держит свою [`DocumentEditorHandle`] — модель
//! переживает переключения плиток; сам виджет пересоздаётся Reactive'ом,
//! но по общей ручке продолжает ту же модель. Базы и канвасы пока
//! показываются заглушкой (их редакторы — этапы T5–T8).

use syngui::prelude::*;
use syngui::widgets::Page;
use syngui::widgets::input::document_editor::DocumentEditor;

use crate::icons::*;

use super::state::{NoteKind, NotesCtx};

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(note) = ctx.active_note() else {
            return vec![Box::new(empty_state())];
        };
        match note.kind {
            NoteKind::Page => {
                let editor = DocumentEditor::new()
                    .markdown((*note.source).clone())
                    .handle(&note.handle)
                    .class("notes-editor");
                vec![Box::new(
                    Page::new()
                        .child(editor)
                        .class("notes-editor-page"),
                )]
            }
            NoteKind::Base | NoteKind::Canvas => vec![Box::new(coming_soon(note.kind))],
        }
    })
}

fn empty_state() -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_EDIT_NOTE).class("notes-empty-icon"))
            .child(Text::new(tr!("notes.empty.title")).class("notes-empty-title"))
            .child(Text::new(tr!("notes.empty.hint")).class("notes-empty-hint")),
    )
}

fn coming_soon(kind: NoteKind) -> impl Widget {
    let (icon, text) = match kind {
        NoteKind::Base => (MI_GRID_ON, tr!("notes.soon.base")),
        _ => (MI_ACCOUNT_TREE, tr!("notes.soon.canvas")),
    };
    Center::new().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("notes-empty-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
