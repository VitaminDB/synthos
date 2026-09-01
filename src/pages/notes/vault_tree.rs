//! Левая панель «Заметок»: дерево vault'а.
//!
//! Плоский DFS-список из [`storage::scan`] рисуется колонкой с отступами
//! по глубине; папки сворачиваются кликом, страницы открываются плитками.
//! Контекстное меню строки — удаление (файл остаётся только на диске у
//! корзины ОС нет — удаляем безвозвратно, как в файловом менеджере).

use syngui::prelude::*;
use syngui::input::CursorIcon;
use syngui::mss::StyleValue;
use syngui::widgets::GestureDetector;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;

use crate::components::panel_header;
use crate::icons::*;
use crate::rail;

use super::state::NotesCtx;
use super::storage::{VaultEntry, VaultEntryKind};

pub fn header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(
            panel_header::side_title(MI_EDIT_NOTE, tr!("notes.tree.title")),
        ))
        .child(
            panel_header::action_button(MI_NOTE_ADD, tr!("notes.tree.new_page"), move || {
                ctx.create_page(&tr!("notes.untitled"));
                rail::navigate("notes");
            }),
        )
        .child(
            panel_header::action_button(MI_GRID_ON, tr!("notes.tree.new_base"), move || {
                ctx.create_base(&tr!("notes.untitled_base"));
                rail::navigate("notes");
            }),
        )
        .child(
            panel_header::action_button(MI_ACCOUNT_TREE, tr!("notes.tree.new_canvas"), move || {
                ctx.create_canvas(&tr!("notes.untitled_canvas"));
                rail::navigate("notes");
            }),
        )
        .child(
            panel_header::action_button(MI_AUTORENEW, tr!("notes.tree.rescan"), move || {
                ctx.rescan();
            }),
        )
}

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let entries = ctx.tree.get();
        let collapsed = ctx.collapsed.get();
        let active = ctx.active.get();

        let mut col = Column::new()
            .gap(1.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-tree");
        if entries.is_empty() {
            col = col.child(
                DecoratedBox::new()
                    .class("notes-tree-empty")
                    .child(Text::new(tr!("notes.tree.empty")).class("notes-tree-empty-text")),
            );
        }
        // Строки внутри свёрнутых папок пропускаем.
        let mut skip_prefix: Option<String> = None;
        for e in entries.iter() {
            if let Some(prefix) = &skip_prefix {
                if e.rel.starts_with(prefix.as_str()) {
                    continue;
                }
                skip_prefix = None;
            }
            if e.kind == VaultEntryKind::Dir && collapsed.contains(&e.rel) {
                skip_prefix = Some(format!("{}/", e.rel));
            }
            let is_active = active.as_deref() == Some(e.rel.as_str());
            col = col.child(Stack::new().children(vec![row(ctx, e.clone(), is_active)]));
        }
        vec![Box::new(col)]
    });
    ScrollView::new().vertical().class("notes-tree-scroll").child(list)
}

fn row(ctx: NotesCtx, entry: VaultEntry, is_active: bool) -> Box<dyn Widget> {
    let icon = match entry.kind {
        VaultEntryKind::Dir => {
            if ctx.collapsed.get_untracked().contains(&entry.rel) {
                MI_FOLDER
            } else {
                MI_FOLDER_OPEN
            }
        }
        VaultEntryKind::Page => MI_DESCRIPTION,
        VaultEntryKind::Base => MI_GRID_ON,
        VaultEntryKind::Canvas => MI_ACCOUNT_TREE,
    };
    let class = if is_active { "notes-tree-row selected" } else { "notes-tree-row" };
    let indent = 8.0 + entry.depth as f32 * 14.0;
    let content = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().style("width", StyleValue::px(indent)).class("notes-tree-indent"))
        .child(Icon::new(icon).class("notes-tree-icon"))
        .child(
            Text::new(entry.name.clone())
                .max_lines(1)
                .class("notes-tree-name"),
        );
    let body = DecoratedBox::new().class(class).child(content);

    let rel_click = entry.rel.clone();
    let kind = entry.kind;
    let clickable = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            if kind == VaultEntryKind::Dir {
                ctx.collapsed.update(|set| {
                    if !set.remove(&rel_click) {
                        set.insert(rel_click.clone());
                    }
                });
            } else {
                ctx.open_path(&rel_click);
            }
        })
        .child(body);

    let rel_menu = entry.rel.clone();
    Box::new(
        ContextMenu::new()
            .items(vec![MenuItem::new("delete", tr!("app.delete")).icon(MI_CLOSE)])
            .on_select(move |action| {
                if action == "delete" {
                    ctx.delete_entry(&rel_menu);
                }
            })
            .child(clickable),
    )
}
