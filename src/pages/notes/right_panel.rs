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
use syngui::widgets::input::document_editor::{BlockProps, DocOp, ShapeKind, TableOp};
use syngui::widgets::{Dropdown, DropdownItem, GestureDetector, ToolButton};

use crate::icons::*;

use super::icon_picker;
use super::blocks;
use super::doc_menu;
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
        let _ = ctx.doc_epoch.get();
        // Сначала блок под кареткой: правишь текст — панель про него.
        let block: Option<Box<dyn Widget>> = ctx.active_page().and_then(|page| {
            let _ = page.handle.revision().get();
            let selected = page.handle.selected().get()?;
            let props = page.handle.block_props(selected)?;
            Some(Box::new(block_props(ctx, props)) as Box<dyn Widget>)
        });
        let mut col = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(page_props(ctx, id));
        if let Some(block) = block {
            col = col.child(Stack::new().children(vec![block]));
        }
        vec![Box::new(ScrollView::new().vertical().child(col))]
    })
}

/// Свойства текущего блока: тип, стиль и операции над ним.
fn block_props(ctx: NotesCtx, props: BlockProps) -> impl Widget {
    let id = props.id;
    let attrs = props.attrs.clone();
    let set = move |key: &'static str, value: Option<String>| {
        ctx.doc_op(DocOp::SetAttr { block: id, key: key.to_string(), value });
    };

    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(
            Text::new(tr!("notes.props.block", kind = blocks::kind_label(props.kind, props.level)))
                .class("notes-links-section"),
        );

    // Тип блока задаёт умолчания; всё ниже — переопределения поверх темы.
    // У фигуры и картинки текста нет — кегль, начертание, цвет и
    // выравнивание им ни к чему; их настройки идут секциями ниже.
    if props.shape.is_none() && props.kind != "media" {
        col = col
            .child(field_row(tr!("notes.props.color"), swatches(COLOR_PRESETS, attrs.get("color"), move |v| set("color", v))))
            .child(field_row(tr!("notes.props.bg"), swatches(BG_PRESETS, attrs.get("bg"), move |v| set("bg", v))))
            .child(field_row(
                tr!("notes.props.size"),
                SpinBox::new()
                    .range(0.0, 160.0)
                    .step(1.0)
                    .width(96.0)
                    .value(attrs.get("size").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0))
                    .on_change(move |v| set("size", (v >= 6.0).then(|| format!("{}", v as i64))))
                    .class("notes-props-field"),
            ))
            .child(field_row(
                tr!("notes.props.align"),
                segmented(
                    &[
                        (MI_FORMAT_ALIGN_LEFT, "left"),
                        (MI_FORMAT_ALIGN_CENTER, "center"),
                        (MI_FORMAT_ALIGN_RIGHT, "right"),
                    ],
                    attrs.get("align").unwrap_or("left"),
                    move |v| set("align", (v != "left").then(|| v.to_string())),
                ),
            ))
            .child(field_row(
                tr!("notes.props.weight"),
                segmented(
                    &[(MI_ARTICLE, "normal"), (MI_FORMAT_BOLD, "bold")],
                    attrs.get("weight").unwrap_or("normal"),
                    move |v| set("weight", (v != "normal").then(|| v.to_string())),
                ),
            ));
    }

    // Примитив: вид, заливка, обводка, пунктир, скругление, прозрачность.
    if let Some(shape) = props.shape {
        col = col.child(shape_props(ctx, id, shape, &attrs, set));
    }
    // Размеры — у того, у кого высота своя (фигура, картинка): текст
    // растёт по контенту, задавать ему высоту нечего.
    if props.shape.is_some_and(|s| !s.is_line()) || props.kind == "media" {
        col = col.child(size_props(&attrs, set));
    }

    // Таблица: строки и колонки — иначе её вообще нельзя дорастить.
    if let Some((rows, cols)) = props.table {
        col = col
            .child(
                Text::new(tr!("notes.props.table", rows = rows, cols = cols))
                    .class("notes-links-section"),
            )
            .child(
                Row::new()
                    .gap(4.0)
                    .child(table_button(ctx, id, MI_TABLE_ROWS, tr!("notes.props.table.add_row"), TableOp::AddRow))
                    .child(table_button(ctx, id, MI_VIEW_COLUMN, tr!("notes.props.table.add_col"), TableOp::AddColumn))
                    .child(table_button(ctx, id, MI_DELETE, tr!("notes.props.table.del_row"), TableOp::DeleteRow))
                    .child(table_button(ctx, id, MI_DELETE, tr!("notes.props.table.del_col"), TableOp::DeleteColumn)),
            );
    }
    col
}

/// Оформление примитива. Вид меняется через «превратить в» (сохраняя
/// оформление), остальное — атрибуты блока.
fn shape_props(
    ctx: NotesCtx,
    id: syngui::widgets::input::document_editor::BlockId,
    shape: ShapeKind,
    attrs: &syngui::widgets::input::document_editor::Attrs,
    set: impl Fn(&'static str, Option<String>) + Send + Sync + Copy + 'static,
) -> impl Widget {
    let is_line = shape.is_line();
    let num = |key: &str, default: f64| -> f64 {
        attrs.get(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default)
    };
    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.shape")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.shape.kind"), shape_picker(ctx, id, shape)));
    if !is_line {
        col = col.child(field_row(
            tr!("notes.props.shape.fill"),
            swatches(SHAPE_FILL_PRESETS, attrs.get("fill"), move |v| set("fill", v)),
        ));
    }
    col = col
        .child(field_row(
            tr!("notes.props.shape.stroke"),
            swatches(COLOR_PRESETS, attrs.get("stroke"), move |v| set("stroke", v)),
        ))
        .child(field_row(
            tr!("notes.props.shape.width"),
            SpinBox::new()
                .range(0.0, 40.0)
                .step(1.0)
                .width(96.0)
                .value(num("sw", 2.0))
                .on_change(move |v| set("sw", Some(format!("{}", v as i64))))
                .class("notes-props-field"),
        ))
        .child(field_row(
            tr!("notes.props.shape.dash"),
            SpinBox::new()
                .range(0.0, 60.0)
                .step(1.0)
                .width(96.0)
                .value(num("dash", 0.0))
                .on_change(move |v| set("dash", (v >= 1.0).then(|| format!("{}", v as i64))))
                .class("notes-props-field"),
        ));
    if matches!(shape, ShapeKind::Rect) {
        col = col.child(field_row(
            tr!("notes.props.shape.radius"),
            SpinBox::new()
                .range(0.0, 200.0)
                .step(1.0)
                .width(96.0)
                .value(num("radius", 8.0))
                .on_change(move |v| set("radius", Some(format!("{}", v as i64))))
                .class("notes-props-field"),
        ));
    }
    col.child(field_row(
        tr!("notes.props.shape.opacity"),
        SpinBox::new()
            .range(5.0, 100.0)
            .step(5.0)
            .width(96.0)
            .value(num("opacity", 100.0))
            .on_change(move |v| set("opacity", (v < 100.0).then(|| format!("{}", v as i64))))
            .class("notes-props-field"),
    ))
}

/// Выбор вида примитива иконками — это «превратить в», оформление и
/// координаты при смене сохраняются.
fn shape_picker(
    ctx: NotesCtx,
    id: syngui::widgets::input::document_editor::BlockId,
    current: ShapeKind,
) -> impl Widget {
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for kind in ShapeKind::ALL {
        let selected = kind == current;
        row = row.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || {
                    // Сначала делаем фигуру текущей: панель могла показывать
                    // её, пока каретка стоит в другом блоке.
                    ctx.doc_op(DocOp::Select(id));
                    ctx.doc_op(DocOp::TurnInto(
                        syngui::widgets::input::document_editor::SlashAction::Shape(kind),
                    ));
                })
                .child(
                    DecoratedBox::new()
                        .class(if selected { "notes-props-seg selected" } else { "notes-props-seg" })
                        .child(
                            Center::new().child(
                                Icon::new(doc_menu::shape_icon(kind)).class("notes-props-seg-icon"),
                            ),
                        ),
                ),
        );
    }
    row
}

/// Ширина и высота блока (свободная раскладка): те же значения, что
/// тянутся за кромки, но набираемые точно.
fn size_props(
    attrs: &syngui::widgets::input::document_editor::Attrs,
    set: impl Fn(&'static str, Option<String>) + Send + Sync + Copy + 'static,
) -> impl Widget {
    let num = |key: &str| -> f64 { attrs.get(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) };
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.size_box")).class("notes-links-section"))
        .child(field_row(
            tr!("notes.props.size_box.width"),
            SpinBox::new()
                .range(0.0, 4000.0)
                .step(10.0)
                .width(96.0)
                .value(num("w"))
                .on_change(move |v| set("w", (v >= 40.0).then(|| format!("{}", v as i64))))
                .class("notes-props-field"),
        ))
        .child(field_row(
            tr!("notes.props.size_box.height"),
            SpinBox::new()
                .range(0.0, 4000.0)
                .step(10.0)
                .width(96.0)
                .value(num("h"))
                .on_change(move |v| set("h", (v >= 20.0).then(|| format!("{}", v as i64))))
                .class("notes-props-field"),
        ))
}

/// Ряд цветовых кружков; первый — «как в теме» (свойство снимается).
fn swatches(
    presets: &'static [&'static str],
    current: Option<&str>,
    on_pick: impl Fn(Option<String>) + Send + Sync + Copy + 'static,
) -> impl Widget {
    let current = current.unwrap_or("").to_string();
    let mut row = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for preset in presets {
        let value = preset.to_string();
        let selected = current == value;
        let mut dot = DecoratedBox::new().class(if selected {
            "notes-props-swatch selected"
        } else {
            "notes-props-swatch"
        });
        if value.is_empty() {
            dot = dot.class(if selected { "notes-props-swatch empty selected" } else { "notes-props-swatch empty" });
        } else {
            dot = dot.style("background-color", syngui::core::Color::from_hex(&value));
        }
        row = row.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || on_pick((!value.is_empty()).then(|| value.clone())))
                .child(dot),
        );
    }
    row
}

/// Переключатель из иконок (выравнивание, начертание).
fn segmented(
    items: &'static [(&'static str, &'static str)],
    current: &str,
    on_pick: impl Fn(&str) + Send + Sync + Copy + 'static,
) -> impl Widget {
    let current = current.to_string();
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (icon, value) in items {
        let selected = current == *value;
        let v = *value;
        row = row.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || on_pick(v))
                .child(
                    DecoratedBox::new()
                        .class(if selected { "notes-props-seg selected" } else { "notes-props-seg" })
                        .child(Center::new().child(Icon::new(*icon).class("notes-props-seg-icon"))),
                ),
        );
    }
    row
}

fn table_button(
    ctx: NotesCtx,
    block: syngui::widgets::input::document_editor::BlockId,
    icon: &'static str,
    tooltip: String,
    op: TableOp,
) -> impl Widget {
    ToolButton::new(icon)
        .tooltip(tooltip)
        .on_click(move || ctx.doc_op(DocOp::Table { block, op }))
}

/// Цвета текста и подложки: пусто — «как в теме».
const COLOR_PRESETS: &[&str] =
    &["", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#8B95A6"];
const BG_PRESETS: &[&str] =
    &["", "#3A2A28", "#3A3324", "#243A2E", "#243149", "#332944", "#2B2F36"];
/// Заливка фигуры: первый кружок — «без заливки» (только контур).
const SHAPE_FILL_PRESETS: &[&str] =
    &["", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#2B2F36"];

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
        .width(132.0)
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
        })
        .class("notes-props-field");

    let step_id = id.clone();
    let grid_step = SpinBox::new()
        .range(2.0, 200.0)
        .step(5.0)
        .width(96.0)
        .value(layout.grid_step as f64)
        .on_change(move |v| {
            ctx.set_page_layout(
                &step_id,
                PageLayout { grid_step: v as f32, ..ctx.page_layout(&step_id) },
            );
        })
        .class("notes-props-field");

    let snap_id = id.clone();
    let snap_row = switch_row(tr!("notes.props.layout.snap"), layout.snap, move |on| {
        ctx.set_page_layout(&snap_id, PageLayout { snap: on, ..ctx.page_layout(&snap_id) });
    });

    let snap_step_id = id;
    let snap_step = SpinBox::new()
        .range(1.0, 100.0)
        .step(1.0)
        .width(96.0)
        .value(layout.snap_step as f64)
        .on_change(move |v| {
            ctx.set_page_layout(
                &snap_step_id,
                PageLayout { snap_step: v as f32, ..ctx.page_layout(&snap_step_id) },
            );
        })
        .class("notes-props-field");

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
