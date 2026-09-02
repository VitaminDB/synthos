//! Правая панель «Заметок»: TabBar «Свойства | Связи».
//!
//! Вставка блоков ушла в контекстное меню документа и slash-меню.
//! «Свойства» — выделенная карточка канваса (цвет/удаление) либо активная
//! страница (название, иконка, путь в дереве, раскладка); «Связи» —
//! мини-граф, обратные и исходящие ссылки.
//!
//! Раскладка страницы: «Поток» (колонка Notion) или «Свободная» — блоки
//! ставятся мышью в любое место, с привязкой к шагу (по умолчанию 5 px) и
//! фон-сеткой (нет / точки / линии / крест). Настройки живут в дереве
//! (`PageNode.layout`), координаты блоков — в markdown страницы.

use syngui::prelude::*;
use syngui::widgets::input::{SpinBox, Toggle};
use syngui::widgets::navigation::{Tab, TabBar};
use syngui::widgets::{Dropdown, DropdownItem, GestureDetector};

use crate::icons::*;

use super::icon_picker;
use super::project::{PageGrid, PageLayout};
use super::state::{LiveObject, NotesCtx, TAB_LINKS, TAB_PROPS};

pub fn header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let tab = ctx.right_tab;
    let tabbar = TabBar::new()
        .tab(Tab::new(tr!("notes.right.tab.props"), TAB_PROPS, &tab).icon(MI_TUNE))
        .tab(Tab::new(tr!("notes.right.tab.links"), TAB_LINKS, &tab).icon(MI_HUB))
        .class("right-panel-tabbar-inner");
    DecoratedBox::new().class("right-panel-tabbar").child(tabbar)
}

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let content: Box<dyn Widget> = match ctx.right_tab.get() {
            TAB_PROPS => Box::new(props_tab(ctx)),
            _ => Box::new(links_tab(ctx)),
        };
        vec![content]
    })
}

/// Вкладка «Связи»: обратные и исходящие ссылки активной страницы.
fn links_tab(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(active) = ctx.active.get() else {
            return vec![Box::new(placeholder(MI_HUB, tr!("notes.right.links.empty")))];
        };
        let index = ctx.index.get();
        let backlinks = index.backlinks_of(&active);
        let outgoing = index.outgoing_of(&active);
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-links-list");
        col = col.child(
            DecoratedBox::new()
                .class("notes-mini-graph")
                .child(super::graph::mini(ctx, active.clone())),
        );
        if backlinks.is_empty() && outgoing.is_empty() {
            col = col.child(Text::new(tr!("notes.right.links.none")).class("notes-empty-hint"));
            return vec![Box::new(ScrollView::new().vertical().child(col))];
        }
        if !backlinks.is_empty() {
            col = col.child(Text::new(tr!("notes.links.backlinks")).class("notes-links-section"));
            for id in backlinks {
                let title = index.title_of(&id);
                col = col.child(Stack::new().children(vec![link_row(ctx, id, title)]));
            }
        }
        if !outgoing.is_empty() {
            col = col.child(Text::new(tr!("notes.links.outgoing")).class("notes-links-section"));
            for id in outgoing {
                let title = index.title_of(&id);
                col = col.child(Stack::new().children(vec![link_row(ctx, id, title)]));
            }
        }
        vec![Box::new(ScrollView::new().vertical().child(col))]
    })
}

fn link_row(ctx: NotesCtx, id: String, title: String) -> Box<dyn Widget> {
    let icon = ctx
        .tree
        .get_untracked()
        .icon_of(&id)
        .unwrap_or_else(|| MI_DESCRIPTION.to_string());
    let row = DecoratedBox::new().class("notes-link-row").child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon_picker::render_icon(&icon, "notes-link-icon"))
            .child(Text::new(title).max_lines(1).class("notes-link-label")),
    );
    Box::new(
        GestureDetector::new()
            .cursor(syngui::input::CursorIcon::Pointer)
            .on_click(move || ctx.activate(&id))
            .child(row),
    )
}

/// Инспектор «Свойства»: карточка канваса либо активная страница.
fn props_tab(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Выделенная карточка любого живого канваса.
        for o in ctx.objects.get().iter() {
            if let LiveObject::Canvas { handle, .. } = o {
                if let Some(node_id) = handle.selected.get() {
                    return vec![Box::new(canvas_card_props(handle.clone(), node_id))];
                }
            }
        }
        if ctx.show_graph.get() {
            return vec![Box::new(placeholder(MI_HUB, tr!("notes.right.props.empty")))];
        }
        let Some(id) = ctx.active.get() else {
            return vec![Box::new(placeholder(MI_TUNE, tr!("notes.right.props.empty")))];
        };
        let _ = ctx.tree.get();
        vec![Box::new(page_props(ctx, id))]
    })
}

/// Свойства страницы: название, иконка и путь в дереве.
fn page_props(ctx: NotesCtx, id: String) -> impl Widget {
    let tree = ctx.tree.get_untracked();
    let title = tree.title_of(&id).unwrap_or_default();
    let icon = tree.icon_of(&id).unwrap_or_else(|| MI_DESCRIPTION.to_string());
    let path = tree
        .path_of(&id)
        .into_iter()
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" / ");
    let id_rename = id.clone();
    let id_icon = id.clone();
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.name")).class("notes-links-section"))
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    GestureDetector::new()
                        .cursor(syngui::input::CursorIcon::Pointer)
                        .on_click_with_bounds(move |_, bounds| icon_picker::open_for(ctx, &id_icon, bounds))
                        .child(
                            DecoratedBox::new()
                                .class("notes-props-icon-box")
                                .child(Center::new().child(icon_picker::render_icon(&icon, "notes-props-icon"))),
                        ),
                )
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .text(title)
                            .submit_on_focus_lost(true)
                            .on_submit(move |v: &str| ctx.rename_page(&id_rename, v))
                            .class("notes-props-name"),
                    ),
                ),
        )
        .child(Text::new(tr!("notes.props.path")).class("notes-links-section"))
        .child(Text::new(path).max_lines(3).class("notes-props-path"))
        .child(layout_props(ctx, id))
}

/// Раскладка страницы: режим, фон-сетка и привязка.
fn layout_props(ctx: NotesCtx, id: String) -> impl Widget {
    let layout = ctx.page_layout(&id);

    let free_id = id.clone();
    let free_row = switch_row(tr!("notes.props.layout.free"), layout.free, move |on| {
        ctx.set_page_layout(&free_id, PageLayout { free: on, ..ctx.page_layout(&free_id) });
    });

    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.layout")).class("notes-links-section"))
        .child(free_row);

    if !layout.free {
        return col.child(Text::new(tr!("notes.props.layout.hint")).class("notes-props-hint"));
    }

    let grid_id = id.clone();
    let grid = Dropdown::new()
        .items(
            PageGrid::ALL
                .iter()
                .map(|g| DropdownItem::new(grid_value(*g), grid_label(*g)))
                .collect(),
        )
        .selected(grid_value(layout.grid))
        .on_change(move |v| {
            let grid = PageGrid::ALL
                .iter()
                .copied()
                .find(|g| grid_value(*g) == v)
                .unwrap_or_default();
            ctx.set_page_layout(&grid_id, PageLayout { grid, ..ctx.page_layout(&grid_id) });
        });

    let step_id = id.clone();
    let grid_step = SpinBox::new()
        .range(2.0, 200.0)
        .step(5.0)
        .value(layout.grid_step as f64)
        .on_change(move |v| {
            ctx.set_page_layout(
                &step_id,
                PageLayout { grid_step: v as f32, ..ctx.page_layout(&step_id) },
            );
        });

    let snap_id = id.clone();
    let snap_row = switch_row(tr!("notes.props.layout.snap"), layout.snap, move |on| {
        ctx.set_page_layout(&snap_id, PageLayout { snap: on, ..ctx.page_layout(&snap_id) });
    });

    let snap_step_id = id;
    let snap_step = SpinBox::new()
        .range(1.0, 100.0)
        .step(1.0)
        .value(layout.snap_step as f64)
        .on_change(move |v| {
            ctx.set_page_layout(
                &snap_step_id,
                PageLayout { snap_step: v as f32, ..ctx.page_layout(&snap_step_id) },
            );
        });

    col = col
        .child(field_row(tr!("notes.props.layout.grid"), grid))
        .child(field_row(tr!("notes.props.layout.grid_step"), grid_step))
        .child(snap_row)
        .child(field_row(tr!("notes.props.layout.snap_step"), snap_step))
        .child(Text::new(tr!("notes.props.layout.free_hint")).class("notes-props-hint"));
    col
}

fn grid_value(grid: PageGrid) -> String {
    match grid {
        PageGrid::None => "none",
        PageGrid::Dots => "dots",
        PageGrid::Lines => "lines",
        PageGrid::Cross => "cross",
    }
    .to_string()
}

fn grid_label(grid: PageGrid) -> String {
    match grid {
        PageGrid::None => tr!("notes.props.grid.none"),
        PageGrid::Dots => tr!("notes.props.grid.dots"),
        PageGrid::Lines => tr!("notes.props.grid.lines"),
        PageGrid::Cross => tr!("notes.props.grid.cross"),
    }
}

/// Строка «подпись — переключатель».
fn switch_row(
    label: String,
    on: bool,
    change: impl Fn(bool) + Send + Sync + 'static,
) -> impl Widget {
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .class("notes-props-row")
        .child(Text::new(label).class("notes-props-row-label"))
        .child(Toggle::with_state(on).on_change(move |v| change(v)))
}

/// Строка «подпись — поле».
fn field_row(label: String, field: impl Widget + 'static) -> impl Widget {
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .class("notes-props-row")
        .child(Text::new(label).class("notes-props-row-label"))
        .child(field)
}

/// Свойства выделенной карточки канваса: цвет и удаление.
fn canvas_card_props(handle: super::canvas::CanvasHandle, node_id: String) -> impl Widget {
    const PRESETS: &[&str] = &["", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#8B95A6"];
    let mut swatches = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for preset in PRESETS {
        let h = handle.clone();
        let id = node_id.clone();
        let color = preset.to_string();
        let mut dot = DecoratedBox::new().class("notes-props-swatch");
        if color.is_empty() {
            dot = dot.class("notes-props-swatch empty");
        } else {
            dot = dot.style("background-color", syngui::core::Color::from_hex(&color));
        }
        swatches = swatches.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || h.set_node_color(&id, &color))
                .child(dot),
        );
    }
    let h_del = handle.clone();
    let id_del = node_id.clone();
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.card_color")).class("notes-links-section"))
        .child(swatches)
        .child(
            Button::new(tr!("notes.canvas.delete"))
                .on_click(move || h_del.delete_node(&id_del))
                .class("notes-props-delete"),
        )
}

fn placeholder(icon: &'static str, text: String) -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("notes-empty-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
