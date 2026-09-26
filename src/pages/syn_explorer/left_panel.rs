//! Левая панель страницы SynExplorer:
//! - заголовок «Закладки» + кнопка `+` ([`header`] — в строке заголовков каркаса)
//! - вертикальный ListView закладок
//! - ниже — список `.syn` файлов из выбранной папки (карточки)
//!
//! Все списки реактивные: при изменении `bookmarks`, `selected_folder` или
//! `folder_entries` UI перестраивается через Reactive-обёртки.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::trn;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::{ContextMenu, MenuItem};

use crate::icons::{
    MI_ADD_CIRCLE, MI_BOOKMARK_ADD, MI_DELETE, MI_DEPLOYED_CODE, MI_FOLDER, MI_FOLDER_OPEN,
    MI_FOLDER_ZIP,
};

use super::actions;
use super::bookmarks;
use super::state::{SourceEntry, SynExplorerCtx, SynFileEntry};

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
                Reactive::new(|| -> Vec<Box<dyn Widget>> {
                    let ctx = use_context::<SynExplorerCtx>();
                    let bookmarks = ctx.bookmarks.get();
                    let selected = ctx.selected_folder.get();
                    if bookmarks.is_empty() {
                        return vec![Box::new(
                            DecoratedBox::new()
                                .class("syn-bookmarks-empty")
                                .child(Text::new(tr!("explorer.left.bookmarks.empty"))
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

/// Заголовок панели: «Закладки» + добавить папку.
pub fn header() -> impl Widget {
    DecoratedBox::new()
        .class("syn-panel-header-row")
        .child(mgui! {
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(tr!("explorer.left.bookmarks.title")).class("syn-section-title"),
                    DecoratedBox::new().class("syn-spacer grow"),
                    Tooltip::new(
                        ToolButton::new(MI_BOOKMARK_ADD)
                            .on_click(|| {
                                let ctx = use_context::<SynExplorerCtx>();
                                actions::pick_and_add_bookmark(ctx);
                            })
                            .class("syn-icon-btn"),
                        tr!("explorer.left.bookmarks.add_tooltip"),
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
    // Полный путь — подпись обрезается до имени папки.
    let clickable = Tooltip::new(clickable, tooltip);

    ContextMenu::new()
        .items(vec![MenuItem::new("remove", tr!("explorer.left.bookmarks.remove")).icon(MI_DELETE)])
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
                None => tr!("explorer.left.folder.default_title"),
            };
            let count = ctx.folder_entries.get().len() + ctx.folder_sources.get().len();
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

/// Содержимое выбранной папки двумя группами: готовые пакеты и модели,
/// которые ещё можно упаковать. Вторая группа — главное изменение: раньше
/// папка с моделью выглядела пустой, и путь к упаковке начинался с
/// многополевой формы вместо одной кнопки.
fn folder_body() -> impl Widget {
    DecoratedBox::new().class("syn-folder-body grow").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let folder = ctx.selected_folder.get();
            let entries = ctx.folder_entries.get();
            let sources = ctx.folder_sources.get();
            let active_path = ctx
                .active_bundle
                .get()
                .map(|b| b.path.get_untracked());
            if folder.is_none() {
                return vec![Box::new(no_folder_placeholder())];
            }
            if entries.is_empty() && sources.is_empty() {
                return vec![Box::new(empty_folder_placeholder())];
            }
            let mut col = Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch);

            if !entries.is_empty() {
                let n = entries.len();
                col = col.child(move || group_header(tr!("explorer.left.group.bundles"), n));
                for entry in entries.iter() {
                    let entry = entry.clone();
                    let is_active = active_path
                        .as_ref()
                        .map(|p| p == &entry.path)
                        .unwrap_or(false);
                    col = col.child(move || bundle_card(entry.clone(), is_active));
                }
            }
            if !sources.is_empty() {
                let n = sources.len();
                col = col.child(move || group_header(tr!("explorer.left.group.packable"), n));
                for entry in sources.iter() {
                    let entry = entry.clone();
                    col = col.child(move || source_card(entry.clone()));
                }
            }
            vec![Box::new(col)]
        },
    ))
}

fn group_header(title: String, count: usize) -> impl Widget {
    DecoratedBox::new().class("syn-folder-group-header").child(mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(title).class("syn-folder-group-title"),
                DecoratedBox::new().class("syn-spacer grow"),
                Text::new(format!("{count}")).class("syn-section-count"),
            ]
    })
}

/// Карточка готового `.syn`: клик открывает пакет.
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
                        Text::new(size_label).class("syn-bundle-card-meta"),
                    ],
            ]
    };

    let tile = DecoratedBox::new().class(class).child(row);
    GestureDetector::new().child(tile).on_click(move || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::open_bundle(ctx, path.clone());
    })
}

/// Карточка нераспакованной модели: что это, сколько весит — и кнопка,
/// после которой остаётся одно подтверждение.
fn source_card(entry: SourceEntry) -> impl Widget {
    let subtitle = source_subtitle(&entry);
    let path_for_click = entry.path.clone();
    let path_for_btn = entry.path.clone();

    let pack_btn = Tooltip::new(
        ToolButton::new(MI_ADD_CIRCLE)
            .on_click(move || {
                let ctx = use_context::<SynExplorerCtx>();
                actions::pack_source(ctx, path_for_btn.clone());
            })
            .class("syn-source-card-action"),
        tr!("explorer.left.source.pack_tooltip"),
    );

    let row = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_DEPLOYED_CODE).class("syn-source-card-icon"),
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("grow") => [
                        Text::new(entry.display_name.clone())
                            .max_lines(1)
                            .class("syn-source-card-name"),
                        Text::new(subtitle).max_lines(1).class("syn-source-card-meta"),
                    ],
                pack_btn,
            ]
    };

    let tile = DecoratedBox::new().class("syn-source-card").child(row);
    // Клик по всей карточке делает то же, что кнопка: попасть в упаковку
    // должно быть проще, чем промахнуться мимо неё.
    GestureDetector::new().child(tile).on_click(move || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::pack_source(ctx, path_for_click.clone());
    })
}

/// «qwen3_5 · 12 шардов · 43.7 ГБ» — ровно то, по чему модель узнаётся.
fn source_subtitle(entry: &SourceEntry) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !entry.arch.is_empty() {
        parts.push(entry.arch.clone());
    }
    if entry.component_count > 1 {
        parts.push(trn!("explorer.left.source.components", entry.component_count));
    } else if entry.shard_count > 1 {
        parts.push(trn!("explorer.left.source.shards", entry.shard_count));
    }
    parts.push(humanize_bytes(entry.bytes));
    parts.join(" · ")
}

fn no_folder_placeholder() -> impl Widget {
    DecoratedBox::new().class("syn-folder-placeholder").child(mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_FOLDER_OPEN).class("syn-placeholder-icon-mini"),
                Text::new(tr!("explorer.left.folder.no_folder_hint"))
                    .class("syn-empty-hint"),
            ]
    })
}

fn empty_folder_placeholder() -> impl Widget {
    DecoratedBox::new().class("syn-folder-placeholder").child(
        Text::new(tr!("explorer.left.folder.empty_hint")).class("syn-empty-hint"),
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
