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

use super::autosave;
use super::state::{NoteKind, NotePayload, NotesCtx, OpenNote};

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(note) = ctx.active_note() else {
            return vec![Box::new(empty_state())];
        };
        match &note.payload {
            NotePayload::Page { source, handle } => {
                let note_path = note.path.clone();
                let editor = DocumentEditor::new()
                    .markdown((**source).clone())
                    .handle(handle)
                    .links(super::links::provider(ctx))
                    .media(super::media::resolver(ctx))
                    .model_epoch(ctx.media_epoch.get())
                    .on_drop_file(move |file, token| {
                        super::media::ingest_dropped_file(
                            ctx,
                            note_path.clone(),
                            file,
                            token,
                        );
                    })
                    .class("notes-editor");
                let banner_note = note.clone();
                vec![Box::new(
                    Column::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                            if banner_note.conflict.get() {
                                vec![Box::new(conflict_banner(banner_note.clone()))]
                            } else {
                                Vec::new()
                            }
                        }))
                        .child(
                            DecoratedBox::new().class("grow").child(
                                Page::new().child(editor).class("notes-editor-page"),
                            ),
                        ),
                )]
            }
            NotePayload::Base(handle) => {
                let banner_note = note.clone();
                let base = super::base::pane::view(handle.clone());
                vec![Box::new(
                    Column::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                            if banner_note.conflict.get() {
                                vec![Box::new(conflict_banner(banner_note.clone()))]
                            } else {
                                Vec::new()
                            }
                        }))
                        .child(DecoratedBox::new().class("grow").child(base)),
                )]
            }
            NotePayload::Raw => vec![Box::new(coming_soon(note.kind))],
        }
    })
}

/// Жёлтая полоса «файл изменён снаружи»: перечитать или перезаписать.
fn conflict_banner(note: OpenNote) -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let path_reload = note.path.clone();
    let path_overwrite = note.path.clone();
    DecoratedBox::new().class("notes-conflict-banner").child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_SYNC_PROBLEM).class("notes-conflict-icon"))
            .child(
                DecoratedBox::new().class("grow").child(
                    Text::new(tr!("notes.conflict.text")).class("notes-conflict-text"),
                ),
            )
            .child(
                Button::new(tr!("notes.conflict.reload"))
                    .on_click(move || ctx.reload_from_disk(&path_reload))
                    .class("notes-conflict-btn"),
            )
            .child(
                Button::new(tr!("notes.conflict.overwrite"))
                    .on_click(move || {
                        autosave::force_save(&ctx, &path_overwrite);
                        note.conflict.set(false);
                    })
                    .class("notes-conflict-btn primary"),
            ),
    )
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
