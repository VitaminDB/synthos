//! Виджет интеллект-карты внутри врезки: тулбар + холст в `PanZoomViewport`
//! под обёрткой `ContextMenu` (правый клик по узлу — меню действий).

use std::sync::Arc;

use syngui::containers::PanZoomViewport;
use syngui::prelude::*;
use syngui::widgets::overlay::ContextMenu;
use syngui::widgets::{Dropdown, DropdownItem, ToolButton};

use crate::icons::*;

use super::canvas::MindmapCanvas;
use super::model::Direction;
use super::{MindmapHandle, ZOOM_MAX, ZOOM_MIN};

/// Окружение карты: открыть страницу по ссылке узла, превратить карту в
/// список на её странице.
#[derive(Clone)]
pub struct MapEnv {
    pub open_page: super::OpenPage,
    pub to_list: Arc<dyn Fn(&str) + Send + Sync>,
}

pub fn view(env: MapEnv, id: String, handle: MindmapHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let editing = handle.editing.get();
        let selected = handle.selected.get();
        let fit = handle.fit_request.get();
        vec![build(&env, &id, handle.clone(), editing, selected, fit)]
    })
}

fn build(
    env: &MapEnv,
    id: &str,
    handle: MindmapHandle,
    editing: Option<String>,
    selected: Option<String>,
    fit: u64,
) -> Box<dyn Widget> {
    let canvas = MindmapCanvas { handle: handle.clone(), open_page: env.open_page.clone(), editing, selected: selected.clone(), fit }
        .class("notes-mindmap-canvas");
    let viewport = PanZoomViewport::new()
        .pan(handle.pan)
        .zoom(handle.zoom)
        .zoom_range(ZOOM_MIN, ZOOM_MAX)
        .grid(false)
        .child(canvas)
        .class("notes-mindmap-viewport");
    let h_menu = handle.clone();
    let env_menu = env.clone();
    let map_id = id.to_string();
    let menu = ContextMenu::new()
        .items(menu_items(&handle, selected.as_deref()))
        .on_select(move |action| menu_action(&h_menu, &env_menu, &map_id, action))
        .child(viewport);
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(toolbar(&handle, selected.as_deref()))
            .child(DecoratedBox::new().class("grow").child(menu)),
    )
}

pub fn direction_label(d: Direction) -> String {
    syngui::i18n::tr(&format!("notes.mindmap.dir.{}", d.key()))
}

fn toolbar(handle: &MindmapHandle, selected: Option<&str>) -> impl Widget {
    let h_child = handle.clone();
    let h_sibling = handle.clone();
    let h_delete = handle.clone();
    let h_fit = handle.clone();
    let h_reset = handle.clone();
    let h_dir = handle.clone();
    let sel_child = selected.map(str::to_string);
    let sel_sibling = selected.map(str::to_string);
    let sel_delete = selected.map(str::to_string);
    let root = handle.root_id();
    let direction = handle.layout().direction;
    Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-mindmap-toolbar")
        .child(
            ToolButton::new(MI_ADD)
                .text(tr!("notes.mindmap.add_child"))
                .tooltip(tr!("notes.mindmap.add_child_hint"))
                .on_click(move || {
                    h_child.add_child(sel_child.as_deref(), "");
                }),
        )
        .child(
            ToolButton::new(MI_SUBDIRECTORY_ARROW_RIGHT)
                .tooltip(tr!("notes.mindmap.add_sibling_hint"))
                .on_click(move || {
                    if let Some(s) = sel_sibling.as_deref() {
                        h_sibling.add_sibling(s, "");
                    }
                }),
        )
        .child(
            ToolButton::new(MI_DELETE)
                .tooltip(tr!("notes.mindmap.delete_hint"))
                .on_click(move || {
                    if let Some(s) = sel_delete.as_deref().filter(|s| *s != root) {
                        h_delete.delete(s);
                    }
                }),
        )
        .child(DecoratedBox::new().class("grow"))
        .child(
            Dropdown::new()
                .width(132.0)
                .items(Direction::ALL.iter().map(|d| DropdownItem::new(d.key(), direction_label(*d))).collect())
                .selected(direction.key())
                .on_change(move |v| {
                    if let Some(d) = Direction::parse(v) {
                        h_dir.set_direction(d);
                    }
                })
                .class("notes-mindmap-direction"),
        )
        .child(ToolButton::new(MI_FIT_SCREEN).tooltip(tr!("notes.mindmap.fit")).on_click(move || h_fit.fit()))
        .child(
            ToolButton::new(MI_AUTORENEW)
                .tooltip(tr!("notes.mindmap.reset_layout"))
                .on_click(move || h_reset.reset_offsets()),
        )
}

fn menu_items(handle: &MindmapHandle, selected: Option<&str>) -> Vec<MenuItem> {
    let has = selected.is_some();
    let is_root = selected.is_some_and(|s| s == handle.root_id());
    let collapsed = selected.and_then(|s| handle.lock().node(s).map(|n| n.collapsed)).unwrap_or(false);
    vec![
        MenuItem::new("child", tr!("notes.mindmap.add_child")).icon(MI_ADD),
        MenuItem::new("sibling", tr!("notes.mindmap.add_sibling")).icon(MI_SUBDIRECTORY_ARROW_RIGHT).disabled(!has || is_root),
        MenuItem::new("edit", tr!("notes.mindmap.edit")).icon(MI_EDIT).disabled(!has),
        MenuItem::new(
            "collapse",
            if collapsed { tr!("notes.mindmap.expand") } else { tr!("notes.mindmap.collapse") },
        )
        .icon(if collapsed { MI_EXPAND_MORE } else { MI_EXPAND_LESS })
        .disabled(!has),
        MenuItem::new("reset_offset", tr!("notes.mindmap.reset_offset")).icon(MI_AUTORENEW).disabled(!has),
        MenuItem::separator(),
        MenuItem::new("delete", tr!("notes.mindmap.delete")).icon(MI_DELETE).disabled(!has || is_root),
        MenuItem::separator(),
        MenuItem::new("fit", tr!("notes.mindmap.fit")).icon(MI_FIT_SCREEN),
        MenuItem::new("reset_layout", tr!("notes.mindmap.reset_layout")).icon(MI_AUTORENEW),
        MenuItem::new("to_list", tr!("notes.mindmap.to_list")).icon(MI_FORMAT_LIST_BULLETED),
    ]
}

fn menu_action(handle: &MindmapHandle, env: &MapEnv, map_id: &str, action: &str) {
    let selected = handle.selected.get_untracked();
    match action {
        "child" => {
            handle.add_child(selected.as_deref(), "");
        }
        "sibling" => {
            if let Some(s) = selected.as_deref() {
                handle.add_sibling(s, "");
            }
        }
        "edit" => {
            if let Some(s) = selected.as_deref() {
                handle.start_editing(s);
            }
        }
        "collapse" => {
            if let Some(s) = selected.as_deref() {
                handle.toggle_collapsed(s);
            }
        }
        "reset_offset" => {
            if let Some(s) = selected.as_deref() {
                handle.reset_offset(s);
            }
        }
        "delete" => {
            if let Some(s) = selected.as_deref().filter(|s| *s != handle.root_id()) {
                handle.delete(s);
            }
        }
        "fit" => handle.fit(),
        "reset_layout" => handle.reset_offsets(),
        "to_list" => (env.to_list)(map_id),
        _ => {}
    }
}
