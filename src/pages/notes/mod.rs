//! Маршрут `notes` — режим «Заметки»: Notion/Obsidian-редактор поверх
//! vault-папки на диске (см. `docs/notes_2026.md`, план в memory).
//!
//! Компоновка — общий трёхпанельный каркас ([`workspace_frame`]):
//!
//! ```text
//! [▤ Дерево vault  ] [иконка · название · поиск]        [Вставка|Свойства|Связи ▥]
//! vault_tree::body   editor_pane::body (DocumentEditor)  right_panel::body
//! ```
//!
//! Открытые страницы — плитки нав-рейла (`RailEntry::Note`), по одной на
//! страницу, как у чатов; закрытие плитки не трогает файл. Положения
//! разделителей — `AppCtx.notes_{left,right}_split_ratio`, видимость —
//! `AppCtx.panels.notes`.

use syngui::prelude::*;

use crate::components::workspace_frame::{self, FrameSpec, Pane};
use crate::components::panel_header::{self, CenterSpec};
use crate::context::AppCtx;
use crate::icons::*;

pub mod autosave;
pub mod base;
pub mod canvas;
pub mod editor_pane;
pub mod embeds;
pub mod fs_watcher;
pub mod graph;
pub mod index;
pub mod links;
pub mod media;
pub mod right_panel;
pub mod state;
pub mod storage;
pub mod vault_tree;

pub use state::{NoteKind, NotePayload, NotesCtx, OpenNote};

pub fn view() -> impl Widget {
    let app = use_context::<AppCtx>();
    let (left_visible, right_visible) = app.panels.notes;

    let spec = FrameSpec::new(
        "notes-h-split",
        || Box::new(center_header()),
        || Box::new(workspace_frame::expand(Box::new(editor_pane::body()))),
    )
    .left(Pane::new(
        left_visible,
        app.notes_left_split_ratio,
        180.0,
        || Box::new(vault_tree::header()),
        || Box::new(vault_tree::body()),
    ))
    .right(Pane::new(
        right_visible,
        app.notes_right_split_ratio,
        220.0,
        || Box::new(right_panel::header()),
        || Box::new(right_panel::body()),
    ));
    workspace_frame::view(spec)
}

fn center_header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let (title, subtitle) = match ctx.active_note() {
            Some(n) => (n.title.clone(), n.path.clone()),
            None => (tr!("notes.title"), tr!("notes.header.subtitle")),
        };
        vec![Box::new(panel_header::center(CenterSpec::new(
            panel_header::identity_text(MI_EDIT_NOTE, title, subtitle),
        )))]
    })
}
