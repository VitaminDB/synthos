//! Левая панель страницы SynExplorer:
//! - заголовок «Закладки» + кнопка `+`
//! - вертикальный ListView закладок
//! - ниже — список `.syn` файлов из выбранной папки (карточки)
//!
//! Все списки реактивные: при изменении `bookmarks`, `selected_folder` или
//! `folder_entries` UI перестраивается через Reactive-обёртки.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::{ContextMenu, MenuItem};

use crate::icons::{
    MI_BOOKMARK_ADD, MI_DELETE, MI_FOLDER, MI_FOLDER_OPEN, MI_FOLDER_ZIP,
};

use super::actions;
use super::bookmarks;
use super::state::{SynExplorerCtx, SynFileEntry};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("syn-explorer-left-panel").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                bookmarks_section(),
                folder_section(),
            ]
    })
}

/// Верхняя секция: заголовок «Закладки» + ListView закладок.
fn bookmarks_section() -> impl Widget {
    mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                bookmarks_header(),
                Reactive::new(|| -> Vec<Box<dyn Widget>> {
                    let ctx = use_context::<SynExplorerCtx>();
                    let bookmarks = ctx.bookmarks.get();
                    let selected = ctx.selected_folder.get();
                    if bookmarks.is_empty() {
                        return vec![Box::new(
                            DecoratedBox::new()
                                .class("syn-bookmarks-empty")
                                .child(Text::new("Нет закладок. Нажмите «+» чтобы добавить папку с `.syn`.")
                                    .class("syn-empty-hint")),
                        )];
                    }
                    let mut col = Column::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch);
                    for (idx, path) in bookmarks.iter().enumerate() {
                        let path = path.clone();
                        let is_selected = selected.as_ref() == Some(&path);
                        col = col.child(move || bookmark_item(idx, path.clone(), is_selected));
                    }
                    vec![Box::new(DecoratedBox::new()
                        .class("syn-bookmarks-list")
                        .child(col))]
                }),
            ]
    }
}

fn bookmarks_header() -> impl Widget {
    DecoratedBox::new()
        .class("syn-bookmark-section-header")
        .child(mgui! {
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new("Закладки").class("syn-section-title"),
                    DecoratedBox::new().class("syn-spacer grow"),
                    Tooltip::new(
                        ToolButton::new(MI_BOOKMARK_ADD)
                            .on_click(|| {
                                let ctx = use_context::<SynExplorerCtx>();
                                actions::pick_and_add_bookmark(ctx);
                            })
                            .class("syn-icon-btn"),
                        "Добавить папку в закладки",
                    ),
                ]
        })
}

fn bookmark_item(
    idx: usize,
    path: std::path::PathBuf,
    selected: bool,
) -> impl Widget {
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let tooltip = path.display().to_string();
    let class = if selected {
        "syn-bookmark-item selected"
    } else {
        "syn-bookmark-item"
    };
    let path_for_click = path.clone();
    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_FOLDER).class("syn-bookmark-icon"),
                Text::new(label.clone()).max_lines(1).class("syn-bookmark-label grow"),
            ]
    };
    let tile = DecoratedBox::new().class(class).child(row);
    let clickable = GestureDetector::new()
        .child(tile)
        .on_click(move || {
            let ctx = use_context::<SynExplorerCtx>();
            bookmarks::select_folder(ctx, path_for_click.clone());
        });
    let _ = tooltip; // tooltip пока не используется (Material Tooltip требует ToolButton-обёртки)

    ContextMenu::new()
        .items(vec![MenuItem::new("remove", "Удалить закладку").icon(MI_DELETE)])
        .on_select(move |action| {
            if action == "remove" {
                let ctx = use_context::<SynExplorerCtx>();
                bookmarks::remove_bookmark(ctx, idx);
            }
        })
        .child(clickable)
}

/// Нижняя секция: список `.syn` из выбранной папки. Если папка не выбрана —
/// hint.
fn folder_section() -> impl Widget {
    DecoratedBox::new().class("syn-folder-section grow").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                folder_header(),
                folder_body(),
            ]
    })
}

fn folder_header() -> impl Widget {
    DecoratedBox::new()
        .class("syn-bookmark-section-header")
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let selected = ctx.selected_folder.get();
            let title = match selected.as_ref() {
                Some(p) => p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string()),
                None => "Пакеты".to_string(),
            };
            let count = ctx.folder_entries.get().len();
            let count_label = if count == 0 {
                String::new()
            } else {
                format!("{count}")
            };
            vec![Box::new(mgui! {
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Text::new(title).max_lines(1).class("syn-section-title"),
                        DecoratedBox::new().class("syn-spacer grow"),
                        Text::new(count_label).class("syn-section-count"),
                    ]
            })]
        }))
}

fn folder_body() -> impl Widget {
    DecoratedBox::new().class("syn-folder-body grow").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let folder = ctx.selected_folder.get();
            let entries = ctx.folder_entries.get();
            let active_path = ctx
                .active_bundle
                .get()
                .map(|b| b.path.get_untracked());
            if folder.is_none() {
                return vec![Box::new(no_folder_placeholder())];
            }
            if entries.is_empty() {
                return vec![Box::new(empty_folder_placeholder())];
            }
            let mut col = Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch);
            for entry in entries.iter() {
                let entry = entry.clone();
                let is_active = active_path
                    .as_ref()
                    .map(|p| p == &entry.path)
                    .unwrap_or(false);
                col = col.child(move || bundle_card(entry.clone(), is_active));
            }
            vec![Box::new(col)]
        },
    ))
}

fn bundle_card(entry: SynFileEntry, is_active: bool) -> impl Widget {
    let class = if is_active {
        "syn-bundle-card selected"
    } else {
        "syn-bundle-card"
    };
    let size_label = entry
        .size
        .map(humanize_bytes)
        .unwrap_or_else(|| "—".to_string());
    let path = entry.path.clone();
    let path_str = path.display().to_string();

    let row = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_FOLDER_ZIP).class("syn-bundle-card-icon"),
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("grow") => [
                        Text::new(entry.display_name.clone())
                            .max_lines(1)
                            .class("syn-bundle-card-name"),
                        Text::new(size_label)
                            .class("syn-bundle-card-meta"),
                    ],
            ]
    };
    let _ = path_str;

    let tile = DecoratedBox::new().class(class).child(row);
    GestureDetector::new()
        .child(tile)
        .on_click(move || {
            let ctx = use_context::<SynExplorerCtx>();
            actions::open_bundle(ctx, path.clone());
        })
}

fn no_folder_placeholder() -> impl Widget {
    DecoratedBox::new().class("syn-folder-placeholder").child(mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_FOLDER_OPEN).class("syn-placeholder-icon-mini"),
                Text::new("Выберите закладку, чтобы увидеть `.syn` пакеты.")
                    .class("syn-empty-hint"),
            ]
    })
}

fn empty_folder_placeholder() -> impl Widget {
    DecoratedBox::new().class("syn-folder-placeholder").child(
        Text::new("В этой папке нет `.syn` файлов.").class("syn-empty-hint"),
    )
}

/// Удобочитаемый размер.
fn humanize_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.0} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}
