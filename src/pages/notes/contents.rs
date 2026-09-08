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

use std::collections::HashSet;

use super::icon_picker;
use super::project::{ProjectTree, TreeRow};
use super::state::{NotesCtx, TreeDropHint, TAB_BLOCKS, TAB_PAGES};

pub const DRAG_TYPE_PAGE: &str = "notes-page";
/// Линия вставки над каждой строкой (и одна под последней): 2px всегда,
/// красится только под курсором — строки не прыгают. Зазора между
/// строками нет: точка в зазоре не попадала ни в одну цель дропа.
const LINE_H: f32 = 2.0;
/// Доля высоты строки сверху и снизу, где дроп значит «перед/после»
/// (середина — «внутрь»). Раньше кромки были по 7px из 28 и без индикации —
/// попасть в них вслепую было почти невозможно, дерево «не переставлялось».
const EDGE_FRAC: f32 = 0.3;

/// Зона строки под курсором.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Zone {
    Before,
    Into,
    After,
}

/// Зона по локальной координате внутри цели дропа (линия + тело строки).
pub(super) fn zone_at(y: f32, height: f32) -> Zone {
    let body = (height - LINE_H).max(1.0);
    let y = y - LINE_H;
    if y < body * EDGE_FRAC {
        Zone::Before
    } else if y > body * (1.0 - EDGE_FRAC) {
        Zone::After
    } else {
        Zone::Into
    }
}

/// Куда переносить `src` при дропе в зону `zone` строки `target`:
/// `(родитель, индекс)` для `NotesCtx::move_page`. `None` — переносить
/// нечего: на себя, в собственное поддерево или на то же место.
/// Нижняя кромка раскрытой страницы с детьми — первым ребёнком: линия
/// там рисуется над первым ребёнком, туда и падает.
pub(super) fn drop_plan(
    tree: &ProjectTree,
    expanded: &HashSet<String>,
    src: &str,
    target: &str,
    zone: Zone,
) -> Option<(Option<String>, Option<usize>)> {
    if src == target {
        return None;
    }
    let (parent, index) = match zone {
        Zone::Into => (Some(target.to_string()), None),
        Zone::After
            if expanded.contains(target)
                && tree.find(target).map(|n| !n.children.is_empty()).unwrap_or(false) =>
        {
            if tree.parent_of(src).as_deref() == Some(target) && tree.index_in_parent(src) == Some(0) {
                return None;
            }
            (Some(target.to_string()), Some(0))
        }
        Zone::Before | Zone::After => {
            let parent = tree.parent_of(target);
            let mut idx = tree.index_in_parent(target)?;
            if zone == Zone::After {
                idx += 1;
            }
            // Источник среди тех же соседей выше цели: после его изъятия
            // индекс цели сдвигается на один; совпал — это то же место.
            if tree.parent_of(src) == parent {
                if let Some(si) = tree.index_in_parent(src) {
                    if si < idx {
                        idx -= 1;
                    }
                    if si == idx {
                        return None;
                    }
                }
            }
            (parent, Some(idx))
        }
    };
    if let Some(p) = &parent {
        if tree.is_ancestor_or_self(src, p) {
            return None;
        }
    }
    Some((parent, index))
}

/// Индикатор для плана: линия над строкой-целью, над следующей видимой
/// строкой (для «после»), в хвосте — или рамка «внутрь».
fn hint_for(
    tree: &ProjectTree,
    expanded: &HashSet<String>,
    src: &str,
    target: &str,
    next_visible: Option<&str>,
    zone: Zone,
) -> Option<TreeDropHint> {
    drop_plan(tree, expanded, src, target, zone)?;
    Some(match zone {
        Zone::Into => TreeDropHint::Into(target.to_string()),
        Zone::Before => TreeDropHint::Line(target.to_string()),
        Zone::After => next_visible.map(|n| TreeDropHint::Line(n.to_string())).unwrap_or(TreeDropHint::Tail),
    })
}

/// Линия вставки: 2px, красится, пока подсказка дропа указывает на неё.
/// Своя реактивная обёртка на каждую линию — дерево не перестраивается
/// на движение курсора.
fn drop_line(ctx: NotesCtx, hint: TreeDropHint) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let on = ctx.tree_drop.get().as_ref() == Some(&hint);
        let class = if on { "notes-tree-drop-line active" } else { "notes-tree-drop-line" };
        vec![Box::new(DecoratedBox::new().class(class).style("height", StyleValue::px(LINE_H)))]
    })
}

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
            .gap(0.0)
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
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        for (i, r) in rows.into_iter().enumerate() {
            let is_active = active.as_deref() == Some(r.id.as_str());
            let is_expanded = expanded.contains(&r.id);
            let is_renaming = renaming.as_deref() == Some(r.id.as_str());
            let next = ids.get(i + 1).cloned();
            col = col.child(Stack::new().children(vec![row(ctx, r, next, is_active, is_expanded, is_renaming)]));
        }
        // Хвост: дроп сюда — в конец корня.
        col = col.child(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_PAGE.to_string()])
                .on_drag_over(move |_| {
                    if ctx.tree_drop.get_untracked() != Some(TreeDropHint::Tail) {
                        ctx.tree_drop.set(Some(TreeDropHint::Tail));
                    }
                })
                .on_drag_leave(move || {
                    if ctx.tree_drop.get_untracked() == Some(TreeDropHint::Tail) {
                        ctx.tree_drop.set(None);
                    }
                })
                .on_drop(move |data| {
                    ctx.tree_drop.set(None);
                    ctx.move_page(&data.payload, None, None);
                })
                .child(
                    Column::new()
                        .gap(0.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(drop_line(ctx, TreeDropHint::Tail))
                        .child(DecoratedBox::new().class("notes-tree-tail")),
                ),
        );
        // Курсор с переносом ушёл из дерева — индикатор гаснет; внутри
        // дерева события получают вложенные цели, внешняя молчит.
        vec![Box::new(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_PAGE.to_string()])
                .on_drag_leave(move || ctx.tree_drop.set(None))
                .on_drop(move |_| ctx.tree_drop.set(None))
                .child(col),
        )]
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

fn row(
    ctx: NotesCtx,
    r: TreeRow,
    next_visible: Option<String>,
    is_active: bool,
    is_expanded: bool,
    is_renaming: bool,
) -> Box<dyn Widget> {
    let id = r.id.clone();
    let indent = 4.0 + r.depth as f32 * 14.0;

    // Содержимое строки собирается замыканием: бокс строки реактивен
    // (рамка «вложить внутрь» по подсказке дропа), а виджеты syngui не
    // клонируются. `Draggable`/`DropArea` снаружи живут — перенос и его
    // призрак-снимок не рвутся.
    let has_children = r.has_children;
    let icon_glyph = r.icon.clone().unwrap_or_else(|| MI_DESCRIPTION.to_string());
    let title = r.title.clone();
    let id_content = id.clone();
    let content = move || {
        let id = id_content.clone();
        // Шеврон: только у страниц с детьми.
        let chevron: Box<dyn Widget> = if has_children {
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
        let id_i = id.clone();
        let icon = GestureDetector::new()
            .cursor(CursorIcon::Pointer)
            .on_click_with_bounds(move |_, bounds| icon_picker::open_for(ctx, &id_i, bounds))
            .child(
                DecoratedBox::new()
                    .class("notes-tree-icon-box")
                    .child(Center::new().child(icon_picker::render_icon(&icon_glyph, "notes-tree-icon"))),
            );

        let label: Box<dyn Widget> = if is_renaming {
            let id_r = id.clone();
            Box::new(
                TextField::new()
                    .text(title.clone())
                    .autofocus(true)
                    .submit_on_focus_lost(true)
                    .on_submit(move |v: &str| {
                        ctx.rename_page(&id_r, v);
                        ctx.renaming.set(None);
                    })
                    .class("notes-tree-rename"),
            )
        } else {
            Box::new(Text::new(title.clone()).max_lines(1).class("notes-tree-name"))
        };

        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().style("width", StyleValue::px(indent)))
            .child(Stack::new().clip(false).children(vec![chevron]))
            .child(icon)
            .child(DecoratedBox::new().class("grow").child(Stack::new().clip(false).children(vec![label])))
    };
    let id_body = id.clone();
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let into = matches!(ctx.tree_drop.get(), Some(TreeDropHint::Into(ref h)) if *h == id_body);
        let class = match (is_active, into) {
            (_, true) => "notes-tree-row drop-into",
            (true, false) => "notes-tree-row selected",
            (false, false) => "notes-tree-row",
        };
        vec![Box::new(DecoratedBox::new().class(class).child(content()))]
    });

    let id_click = id.clone();
    let drag = Draggable::new(DRAG_TYPE_PAGE, id.clone())
        .label(r.title.clone())
        .on_click(move || {
            ctx.activate(&id_click);
            ctx.open_tile();
        })
        .child(body);

    let id_over = id.clone();
    let next_over = next_visible.clone();
    let id_leave = id.clone();
    let id_drop = id.clone();
    let drop = DropArea::new()
        .accept_types(vec![DRAG_TYPE_PAGE.to_string()])
        .on_drag_over(move |info| {
            let zone = zone_at(info.local_position.y, info.size.height);
            let tree = ctx.tree.get_untracked();
            let expanded = ctx.expanded.get_untracked();
            let hint = hint_for(&tree, &expanded, &info.data.payload, &id_over, next_over.as_deref(), zone);
            if ctx.tree_drop.get_untracked() != hint {
                ctx.tree_drop.set(hint);
            }
        })
        .on_drag_leave(move || {
            let mine = match ctx.tree_drop.get_untracked() {
                Some(TreeDropHint::Line(h)) | Some(TreeDropHint::Into(h)) => h == id_leave,
                _ => false,
            };
            if mine {
                ctx.tree_drop.set(None);
            }
        })
        .on_drop_positioned(move |info| {
            ctx.tree_drop.set(None);
            let src = info.data.payload.clone();
            let zone = zone_at(info.local_position.y, info.size.height);
            let tree = ctx.tree.get_untracked();
            let expanded = ctx.expanded.get_untracked();
            let plan = drop_plan(&tree, &expanded, &src, &id_drop, zone);
            drop(tree);
            if let Some((parent, index)) = plan {
                ctx.move_page(&src, parent.as_deref(), index);
            }
        })
        .child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(drop_line(ctx, TreeDropHint::Line(id.clone())))
                .child(drag),
        );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::notes::project::PageNode;

    /// a(a1, a2), b, c — `a` раскрыта.
    fn tree() -> (ProjectTree, HashSet<String>) {
        let node = |id: &str| {
            let mut n = PageNode::new(id.to_uppercase());
            n.id = id.to_string();
            n
        };
        let mut a = node("a");
        a.children.push(node("a1"));
        a.children.push(node("a2"));
        let t = ProjectTree { version: 1, roots: vec![a, node("b"), node("c")] };
        (t, HashSet::from(["a".to_string()]))
    }

    #[test]
    fn zones_split_row_into_before_into_after() {
        let h = LINE_H + 28.0;
        assert_eq!(zone_at(LINE_H + 1.0, h), Zone::Before);
        assert_eq!(zone_at(LINE_H + 14.0, h), Zone::Into);
        assert_eq!(zone_at(LINE_H + 27.0, h), Zone::After);
    }

    /// Перестановка среди соседей: индекс считается после изъятия
    /// источника; то же место — не перенос; в собственное поддерево — нет.
    #[test]
    fn drop_plan_reorders_siblings_and_nests() {
        let (t, exp) = tree();
        let p = |src, target, zone| drop_plan(&t, &exp, src, target, zone);
        assert_eq!(p("c", "b", Zone::Before), Some((None, Some(1))));
        assert_eq!(p("a", "b", Zone::After), Some((None, Some(1))));
        assert_eq!(p("b", "c", Zone::Before), None, "то же место");
        assert_eq!(p("c", "a", Zone::After), Some((Some("a".into()), Some(0))), "под раскрытой — первым ребёнком");
        assert_eq!(p("a1", "a", Zone::After), None, "a1 и так первый ребёнок");
        assert_eq!(p("a2", "a1", Zone::Before), Some((Some("a".into()), Some(0))));
        assert_eq!(p("c", "b", Zone::Into), Some((Some("b".into()), None)));
        assert_eq!(p("a", "a1", Zone::Into), None, "в своё поддерево");
        assert_eq!(p("a", "a1", Zone::Before), None, "в своё поддерево соседом");
        assert_eq!(p("b", "b", Zone::Into), None);
    }

    /// Индикатор: «после» рисуется линией над следующей видимой строкой,
    /// у последней — хвостовой линией.
    #[test]
    fn hint_follows_next_visible_row() {
        let (t, exp) = tree();
        assert_eq!(
            hint_for(&t, &exp, "c", "a", Some("a1"), Zone::After),
            Some(TreeDropHint::Line("a1".into()))
        );
        assert_eq!(hint_for(&t, &exp, "a", "c", None, Zone::After), Some(TreeDropHint::Tail));
        assert_eq!(hint_for(&t, &exp, "c", "b", None, Zone::Into), Some(TreeDropHint::Into("b".into())));
        assert_eq!(hint_for(&t, &exp, "b", "c", None, Zone::Before), None);
    }
}
