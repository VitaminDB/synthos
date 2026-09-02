//! Маршрут `notes` — режим «Заметки»: Notion-редактор поверх проекта в
//! одном `.syn`-файле (см. `docs/notes_2026.md`, план в memory).
//!
//! Компоновка — общий трёхпанельный каркас ([`workspace_frame`]):
//!
//! ```text
//! [▤ Содержимое     ] [иконка · название · путь]           [Свойства|Связи ▥]
//! contents::body      editor_pane::body (DocumentEditor)   right_panel::body
//! ```
//!
//! Проект — одна плитка нав-рейла (`RailEntry::Notes`); страницы
//! переключаются в дереве. Положения разделителей —
//! `AppCtx.notes_{left,right}_split_ratio`, видимость — `AppCtx.panels.notes`.

use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::*;

pub mod autosave;
pub mod blocks;
pub mod contents;
pub mod doc_menu;
pub mod editor_pane;
pub mod embeds;
pub mod gantt;
pub mod graph;
pub mod icon_picker;
pub mod index;
pub mod kanban;
pub mod links;
pub mod media;
pub mod project;
pub mod right_panel;
pub mod state;

pub use state::{LiveObject, LivePage, NotesCtx};

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
        || Box::new(contents::header()),
        || Box::new(contents::body()),
    ))
    .right(Pane::new(
        right_visible,
        app.notes_right_split_ratio,
        220.0,
        || Box::new(right_panel::header()),
        || Box::new(right_panel::body()),
    ));
    Stack::new()
        .clip(false)
        .child(workspace_frame::view(spec))
        .child(icon_picker::view())
}

/// Шапка центра: иконка страницы (клик — выбор иконки), название и путь в
/// дереве; для графа — его заголовок; без страницы — имя проекта.
fn center_header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let project = ctx.project_title.get();
        if ctx.show_graph.get() {
            return vec![Box::new(panel_header::center(CenterSpec::new(
                panel_header::identity_text(MI_HUB, tr!("notes.graph.title"), project),
            )))];
        }
        let tree = ctx.tree.get();
        let Some(id) = ctx.active.get().filter(|id| tree.find(id).is_some()) else {
            return vec![Box::new(panel_header::center(CenterSpec::new(
                panel_header::identity_text(MI_EDIT_NOTE, tr!("notes.title"), project),
            )))];
        };
        let title = tree.title_of(&id).unwrap_or_default();
        let icon = tree.icon_of(&id).unwrap_or_else(|| MI_DESCRIPTION.to_string());
        let mut crumbs: Vec<String> = tree.path_of(&id).into_iter().map(|(_, t)| t).collect();
        crumbs.pop();
        let subtitle = if crumbs.is_empty() { project } else { crumbs.join(" / ") };
        let id_icon = id.clone();
        let leading = GestureDetector::new()
            .cursor(CursorIcon::Pointer)
            .on_click_with_bounds(move |_, bounds| icon_picker::open_for(ctx, &id_icon, bounds))
            .child(
                DecoratedBox::new()
                    .class("panel-header-icon-bubble notes-header-icon-bubble")
                    .child(Center::new().child(icon_picker::render_icon(&icon, "panel-header-icon notes-header-icon"))),
            );
        vec![Box::new(panel_header::center(CenterSpec::new(panel_header::identity(
            leading,
            panel_header::title_text(title),
            panel_header::subtitle_text(subtitle),
        ))))]
    })
}
