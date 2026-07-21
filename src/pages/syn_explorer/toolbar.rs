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
                        Tooltip::new(btn_open, "Открыть .syn… — выбрать файл вручную"),
                        Tooltip::new(btn_new, "Создать новый .syn пакет из папки safetensors"),
                        DecoratedBox::new().class("syn-toolbar-divider"),
                        Tooltip::new(btn_save, "Сохранить изменения (метаданные + pending-операции)"),
                        Tooltip::new(btn_reload, "Перечитать пакет с диска (drop mmap + open снова)"),
                        Tooltip::new(btn_close, "Закрыть открытый пакет"),
                        DecoratedBox::new().class("syn-toolbar-divider"),
                        Tooltip::new(btn_import, "Импортировать файл в пакет (добавит в pending-операции)"),
                        Tooltip::new(btn_extract, "Извлечь выбранный в TreeView файл на диск"),
                        Tooltip::new(btn_delete, "Удалить выбранный файл из пакета (tombstone)"),
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
                ("Несохранённые правки", "syn-toolbar-status dirty")
            } else {
                ("", "syn-toolbar-status")
            }
        }
        LoadState::Loading => ("Загрузка…", "syn-toolbar-status busy"),
        LoadState::Saving => ("Сохраняем…", "syn-toolbar-status busy"),
        LoadState::Creating => ("Создаём…", "syn-toolbar-status busy"),
    };
    Text::new(text).class(cls)
}
