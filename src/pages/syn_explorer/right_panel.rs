//! Правая панель — TreeView содержимого открытого пакета.
//!
//! Аналог `code_editor::file_tree::body()`, но источник узлов — не файловая
//! система, а cdir пакета (`Bundle::list_dir_shallow`). Дерево строится
//! целиком при open/reload в `tree_build::build_dir_tree` — папки внутри
//! `.syn` обычно небольшие (десятки/сотни записей), так что lazy-expand
//! не критичен; вся реактивность — replace tree_nodes на новый Vec.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::{SelectionMode, TreeView};

use crate::icons::{MI_DEPLOYED_CODE, MI_DOWNLOAD};

use super::actions;
use super::state::{SynExplorerCtx, TabKind};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("syn-explorer-right-panel").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(),
                body(),
            ]
    })
}

fn header() -> impl Widget {
    DecoratedBox::new()
        .class("syn-bookmark-section-header")
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let count = ctx
                .active_bundle
                .get()
                .map(|b| {
                    let files = b.files.get();
                    files.iter().filter(|f| f.alive).count()
                })
                .unwrap_or(0);
            let count_label = if count == 0 {
                String::new()
            } else {
                format!("{count}")
            };
            vec![Box::new(mgui! {
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Text::new(MI_DEPLOYED_CODE).class("syn-section-title-icon"),
                        Text::new(tr!("explorer.right.title")).class("syn-section-title"),
                        DecoratedBox::new().class("syn-spacer grow"),
                        Text::new(count_label).class("syn-section-count"),
                        Tooltip::new(
                            ToolButton::new(MI_DOWNLOAD)
                                .on_click(|| {
                                    let ctx = use_context::<SynExplorerCtx>();
                                    actions::pick_and_extract_file(ctx);
                                })
                                .class("syn-icon-btn"),
                            tr!("explorer.right.extract_tooltip"),
                        ),
                    ]
            })]
        }))
}

fn body() -> impl Widget {
    DecoratedBox::new().class("syn-explorer-right-body grow").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let Some(active) = ctx.active_bundle.get() else {
                return vec![Box::new(empty_placeholder())];
            };
            // Реактивные подписки на содержимое:
            let _ = active.reload_gen.get();
            let nodes = active.dir_tree.get();
            let selected = active
                .selected_path
                .get()
                .map(|s| vec![s])
                .unwrap_or_default();

            if nodes.is_empty() {
                return vec![Box::new(empty_placeholder())];
            }

            let tree = TreeView::new(nodes)
                .selection_mode(SelectionMode::Single)
                .selected(selected)
                .show_lines(true)
                .on_select(move |id| {
                    let ctx = use_context::<SynExplorerCtx>();
                    let Some(active) = ctx.active_untracked() else {
                        return;
                    };
                    // Если выбран leaf-файл (alive файлы), переключаемся в
                    // таб Preview. Папки (id содержит вложенные сегменты, но
                    // суть — branch) тоже сохраняем как selected_path для
                    // подсветки.
                    active.selected_path.set(Some(id.to_string()));
                    // Heuristic: TreeNode branch строится с id = full prefix
                    // (например "vocab"), leaf — это полный путь файла внутри
                    // пакета. Если такой путь есть в files — это файл.
                    let is_file = active
                        .files
                        .get_untracked()
                        .iter()
                        .any(|f| f.alive && f.name == id);
                    if is_file && ctx.current_tab.get_untracked() != TabKind::Preview {
                        ctx.current_tab.set(TabKind::Preview);
                    }
                })
                .class("syn-explorer-tree");

            vec![Box::new(tree)]
        },
    ))
}

fn empty_placeholder() -> impl Widget {
    Center::new().child(
        Text::new(tr!("explorer.right.empty_hint"))
            .class("syn-empty-hint"),
    )
}
