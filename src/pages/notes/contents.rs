//! Левая панель «Содержимое»: дерево страниц проекта.
//!
//! Строки — плоский DFS по дереву с учётом раскрытых узлов; шеврон
//! сворачивает, клик по строке активирует страницу, клик по иконке открывает
//! панель выбора иконки. Перетаскивание строки на другую: верхняя/нижняя
//! кромка — поставить перед/после, середина — вложить внутрь. Контекстное
//! меню — создать вложенную, переименовать (inline), дублировать, удалить.

use syngui::input::CursorIcon;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{Draggable, DropArea, GestureDetector};

use crate::components::panel_header;
use crate::icons::*;
use crate::rail;

use super::icon_picker;
use super::project::TreeRow;
use super::state::{NotesCtx, TAB_BLOCKS, TAB_PAGES};

pub const DRAG_TYPE_PAGE: &str = "notes-page";
/// Высота строки — фиксирована в MSS (`.notes-tree-row`), от неё считаются
/// зоны «перед/внутрь/после» при дропе.
const ROW_H: f32 = 28.0;
const EDGE: f32 = 7.0;

pub fn header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    // Две вкладки в узкой панели не помещаются подписями — показываем
    // название только активной, а переключение отдаём иконке соседней.
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let blocks = ctx.left_tab.get() == TAB_BLOCKS;
        let (icon, title) = if blocks {
            (MI_LAYERS, tr!("notes.blocks.title"))
        } else {
            (MI_LIST_ALT, tr!("notes.tree.title"))
        };
        let (other_icon, other_title) = if blocks {
            (MI_LIST_ALT, tr!("notes.tree.title"))
        } else {
            (MI_LAYERS, tr!("notes.blocks.title"))
        };
        let mut row = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(panel_header::side_title(icon, title)))
            .child(panel_header::action_button(other_icon, other_title, move || {
                ctx.left_tab.set(if blocks { TAB_PAGES } else { TAB_BLOCKS });
            }));
        // Создание страницы и граф — про дерево страниц; на вкладке блоков
        // они только съедали бы ширину.
        if !blocks {
            row = row
                .child(panel_header::action_button(
                    MI_NOTE_ADD,
                    tr!("notes.tree.new_page"),
                    move || {
                        ctx.create_page(None, &tr!("notes.untitled"));
                        ctx.open_tile();
                        rail::navigate("notes");
                    },
                ))
                .child(panel_header::action_button(MI_TODAY, tr!("notes.journal.today"), move || {
                    let id = ctx.journal_page(crate::agent::time::local_today_days());
                    ctx.activate(&id);
                    ctx.open_tile();
                    rail::navigate("notes");
                }))
                .child(panel_header::action_button(MI_HUB, tr!("notes.graph.title"), move || {
                    ctx.show_graph.set(true);
                    rail::navigate("notes");
                }));
        }
        vec![Box::new(row)]
    })
}

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        match ctx.left_tab.get() {
            TAB_BLOCKS => vec![Box::new(super::blocks::body())],
            _ => vec![Box::new(pages())],
        }
    })
}

/// Дерево страниц проекта.
fn pages() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let tree = ctx.tree.get();
        let expanded = ctx.expanded.get();
        let active = if ctx.show_graph.get() { None } else { ctx.active.get() };
        let renaming = ctx.renaming.get();

        let mut col = Column::new()
            .gap(1.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-tree");
        let rows = tree.flatten(&expanded);
        if rows.is_empty() {
            col = col.child(
                DecoratedBox::new()
                    .class("notes-tree-empty")
                    .child(Text::new(tr!("notes.tree.empty")).class("notes-tree-empty-text")),
            );
        }
        for r in rows {
            let is_active = active.as_deref() == Some(r.id.as_str());
            let is_expanded = expanded.contains(&r.id);
            let is_renaming = renaming.as_deref() == Some(r.id.as_str());
            col = col.child(Stack::new().children(vec![row(ctx, r, is_active, is_expanded, is_renaming)]));
        }
        // Хвост: дроп сюда — в конец корня.
        col = col.child(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_PAGE.to_string()])
                .on_drop(move |data| {
                    ctx.move_page(&data.payload, None, None);
                })
                .child(DecoratedBox::new().class("notes-tree-tail")),
        );
        vec![Box::new(col)]
    });
    // Правый клик по пустому месту панели — новая страница в корне;
    // на строке первым сработает её собственное меню (события идут
    // от внутреннего элемента наружу).
    ContextMenu::new()
        .items(vec![MenuItem::new("root", tr!("notes.tree.new_root")).icon(MI_NOTE_ADD)])
        .on_select(move |action| {
            if action == "root" {
                ctx.create_page(None, &tr!("notes.untitled"));
            }
        })
        .child(ScrollView::new().vertical().class("notes-tree-scroll").child(list))
}

fn row(ctx: NotesCtx, r: TreeRow, is_active: bool, is_expanded: bool, is_renaming: bool) -> Box<dyn Widget> {
    let id = r.id.clone();
    let indent = 4.0 + r.depth as f32 * 14.0;

    // Шеврон: только у страниц с детьми.
    let chevron: Box<dyn Widget> = if r.has_children {
        let id_c = id.clone();
        let glyph = if is_expanded { MI_EXPAND_MORE } else { MI_CHEVRON_RIGHT };
        Box::new(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || ctx.toggle_expanded(&id_c))
                .child(
                    DecoratedBox::new()
                        .class("notes-tree-chevron-box")
                        .child(Center::new().child(Icon::new(glyph).class("notes-tree-chevron"))),
                ),
        )
    } else {
        Box::new(DecoratedBox::new().class("notes-tree-chevron-box"))
    };

    // Иконка: клик — панель выбора.
    let glyph = r.icon.clone().unwrap_or_else(|| MI_DESCRIPTION.to_string());
    let id_i = id.clone();
    let icon = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click_with_bounds(move |_, bounds| icon_picker::open_for(ctx, &id_i, bounds))
        .child(
            DecoratedBox::new()
                .class("notes-tree-icon-box")
                .child(Center::new().child(icon_picker::render_icon(&glyph, "notes-tree-icon"))),
        );

    let label: Box<dyn Widget> = if is_renaming {
        let id_r = id.clone();
        Box::new(
            TextField::new()
                .text(r.title.clone())
                .autofocus(true)
                .submit_on_focus_lost(true)
                .on_submit(move |v: &str| {
                    ctx.rename_page(&id_r, v);
                    ctx.renaming.set(None);
                })
                .class("notes-tree-rename"),
        )
    } else {
        Box::new(Text::new(r.title.clone()).max_lines(1).class("notes-tree-name"))
    };

    let class = if is_active { "notes-tree-row selected" } else { "notes-tree-row" };
    let content = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().style("width", StyleValue::px(indent)))
        .child(Stack::new().clip(false).children(vec![chevron]))
        .child(icon)
        .child(DecoratedBox::new().class("grow").child(Stack::new().clip(false).children(vec![label])));
    let body = DecoratedBox::new().class(class).child(content);

    let id_click = id.clone();
    let drag = Draggable::new(DRAG_TYPE_PAGE, id.clone())
        .label(r.title.clone())
        .on_click(move || {
            ctx.activate(&id_click);
            ctx.open_tile();
        })
        .child(body);

    let id_drop = id.clone();
    let drop = DropArea::new()
        .accept_types(vec![DRAG_TYPE_PAGE.to_string()])
        .on_drop_positioned(move |info| {
            let src = info.data.payload.clone();
            if src == id_drop {
                return;
            }
            let y = info.local_position.y;
            if y > EDGE && y < ROW_H - EDGE {
                ctx.move_page(&src, Some(&id_drop), None);
                return;
            }
            let tree = ctx.tree.get_untracked();
            let parent = tree.parent_of(&id_drop);
            let mut idx = tree.index_in_parent(&id_drop).unwrap_or(0);
            if y >= ROW_H - EDGE {
                idx += 1;
            }
            // Источник среди тех же соседей выше цели: после его изъятия
            // индекс цели сдвигается на один.
            if tree.parent_of(&src) == parent {
                if let Some(si) = tree.index_in_parent(&src) {
                    if si < idx {
                        idx -= 1;
                    }
                }
            }
            drop(tree);
            ctx.move_page(&src, parent.as_deref(), Some(idx));
        })
        .child(drag);

    let id_menu = id.clone();
    Box::new(
        ContextMenu::new()
            .items(vec![
                MenuItem::new("child", tr!("notes.tree.new_child")).icon(MI_NOTE_ADD),
                MenuItem::new("rename", tr!("notes.tree.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
                MenuItem::new("duplicate", tr!("notes.tree.duplicate")).icon(MI_CONTENT_COPY),
                MenuItem::separator(),
                MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE),
            ])
            .on_select(move |action| match action {
                "child" => {
                    ctx.create_page(Some(&id_menu), &tr!("notes.untitled"));
                }
                "rename" => ctx.renaming.set(Some(id_menu.clone())),
                "duplicate" => ctx.duplicate_page(&id_menu),
                "delete" => ctx.delete_page(&id_menu),
                _ => {}
            })
            .child(drop),
    )
}
