//! Верхняя панель действий страницы SynExplorer.
//!
//! `ToolButton::tooltip(...)` заполняет только accessibility-label (для
//! screen reader'ов), визуально hover-tooltip он не рисует. Чтобы пользователь
//! видел подсказку — каждую кнопку оборачиваем в `Tooltip::new(...)` widget.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::feedback::Tooltip;

use crate::icons::{
    MI_ADD_CIRCLE, MI_AUTORENEW, MI_CLOSE, MI_DELETE, MI_DOWNLOAD, MI_FOLDER_OPEN, MI_NOTE_ADD,
    MI_SAVE,
};

use super::actions;
use super::state::{LoadState, SynExplorerCtx};

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("syn-explorer-toolbar")
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let active = ctx.active_bundle.get();
            let dirty = active.map(|b| b.dirty.get()).unwrap_or(false);
            let has_active = active.is_some();
            let load_state = ctx.load_state.get();
            let busy = !matches!(load_state, LoadState::Idle);

            let save_class = if has_active && dirty {
                "syn-toolbar-btn primary"
            } else {
                "syn-toolbar-btn primary disabled"
            };
            let on_save_enabled = has_active && dirty && !busy;
            let on_active_enabled = has_active && !busy;

            let enabled_class = "syn-toolbar-btn";
            let disabled_class = "syn-toolbar-btn disabled";
            let plain_class = if on_active_enabled { enabled_class } else { disabled_class };
            let danger_class = if on_active_enabled {
                "syn-toolbar-btn danger"
            } else {
                disabled_class
            };

            let btn_open = ToolButton::new(MI_FOLDER_OPEN)
                .on_click(|| {
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::pick_and_open_bundle(ctx);
                })
                .class(enabled_class);
            let btn_new = ToolButton::new(MI_ADD_CIRCLE)
                .on_click(|| {
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::request_create_bundle(ctx);
                })
                .class(enabled_class);
            let btn_save = ToolButton::new(MI_SAVE)
                .on_click(move || {
                    if !on_save_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::save_active(ctx);
                })
                .class(save_class);
            let btn_reload = ToolButton::new(MI_AUTORENEW)
                .on_click(move || {
                    if !on_active_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::reload_active(ctx);
                })
                .class(plain_class);
            let btn_close = ToolButton::new(MI_CLOSE)
                .on_click(move || {
                    if !on_active_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::close_active_bundle(ctx);
                })
                .class(plain_class);
            let btn_import = ToolButton::new(MI_NOTE_ADD)
                .on_click(move || {
                    if !on_active_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::pick_and_import_file(ctx);
                })
                .class(plain_class);
            let btn_extract = ToolButton::new(MI_DOWNLOAD)
                .on_click(move || {
                    if !on_active_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::pick_and_extract_file(ctx);
                })
                .class(plain_class);
            let btn_delete = ToolButton::new(MI_DELETE)
                .on_click(move || {
                    if !on_active_enabled {
                        return;
                    }
                    let ctx = use_context::<SynExplorerCtx>();
                    actions::request_delete_selected(ctx);
                })
                .class(danger_class);

            let row = mgui! {
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Tooltip::new(btn_open, tr!("explorer.toolbar.open.tooltip")),
                        Tooltip::new(btn_new, tr!("explorer.toolbar.new.tooltip")),
                        DecoratedBox::new().class("syn-toolbar-divider"),
                        Tooltip::new(btn_save, tr!("explorer.toolbar.save.tooltip")),
                        Tooltip::new(btn_reload, tr!("explorer.toolbar.reload.tooltip")),
                        Tooltip::new(btn_close, tr!("explorer.toolbar.close.tooltip")),
                        DecoratedBox::new().class("syn-toolbar-divider"),
                        Tooltip::new(btn_import, tr!("explorer.toolbar.import.tooltip")),
                        Tooltip::new(btn_extract, tr!("explorer.toolbar.extract.tooltip")),
                        Tooltip::new(btn_delete, tr!("explorer.toolbar.delete.tooltip")),
                        DecoratedBox::new().class("syn-toolbar-spacer grow"),
                        status_label(load_state, dirty),
                    ]
            };
            vec![Box::new(row)]
        }))
}

fn status_label(state: LoadState, dirty: bool) -> impl Widget {
    let (text, cls) = match state {
        LoadState::Idle => {
            if dirty {
                (tr!("explorer.unsaved_edits"), "syn-toolbar-status dirty")
            } else {
                (String::new(), "syn-toolbar-status")
            }
        }
        LoadState::Loading => (tr!("explorer.toolbar.status.loading"), "syn-toolbar-status busy"),
        LoadState::Saving => (tr!("explorer.toolbar.status.saving"), "syn-toolbar-status busy"),
        LoadState::Creating => (tr!("explorer.toolbar.status.creating"), "syn-toolbar-status busy"),
    };
    Text::new(text).class(cls)
}
