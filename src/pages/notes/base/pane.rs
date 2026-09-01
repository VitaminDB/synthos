//! Панель базы: переключатель представлений + активное представление.
//! Работает и страницей (плитка рейла), и внутри embed-врезки (T9).

use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::icons::*;

use super::model::ViewKind;
use super::{table, BaseHandle};

pub fn view(handle: BaseHandle) -> impl Widget {
    let switcher_handle = handle.clone();
    Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(switcher(switcher_handle))
        .child(DecoratedBox::new().class("grow").child(body(handle)))
}

fn switcher(handle: BaseHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let selected = handle.view_sel.get();
        let views: Vec<(usize, String, ViewKind)> = handle
            .lock()
            .views
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.name.clone(), v.kind))
            .collect();
        let mut row = Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("notes-base-switcher");
        for (i, name, kind) in views {
            let icon = match kind {
                ViewKind::Table => MI_GRID_ON,
                ViewKind::Kanban => MI_VIEW_SIDEBAR,
                ViewKind::Gantt => MI_HORIZONTAL_RULE,
            };
            let class = if i == selected {
                "notes-base-tab selected"
            } else {
                "notes-base-tab"
            };
            let h = handle.clone();
            row = row.child(
                GestureDetector::new()
                    .cursor(syngui::input::CursorIcon::Pointer)
                    .on_click(move || h.view_sel.set(i))
                    .child(
                        DecoratedBox::new().class(class).child(
                            Row::new()
                                .gap(6.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Icon::new(icon).class("notes-base-tab-icon"))
                                .child(Text::new(name).class("notes-base-tab-label")),
                        ),
                    ),
            );
        }
        vec![Box::new(row)]
    })
}

fn body(handle: BaseHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let idx = handle.view_sel.get();
        let kind = handle
            .lock()
            .views
            .get(idx)
            .map(|v| v.kind)
            .unwrap_or(ViewKind::Table);
        let content: Box<dyn Widget> = match kind {
            ViewKind::Table => Box::new(table::view(handle.clone())),
            ViewKind::Kanban => Box::new(soon(MI_VIEW_SIDEBAR, tr!("notes.base.soon.kanban"))),
            ViewKind::Gantt => Box::new(soon(MI_HORIZONTAL_RULE, tr!("notes.base.soon.gantt"))),
        };
        vec![content]
    })
}

fn soon(icon: &'static str, text: String) -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("notes-empty-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
