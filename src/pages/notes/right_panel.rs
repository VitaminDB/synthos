//! Правая панель «Заметок»: TabBar «Свойства | Связи».
//!
//! Вставка блоков ушла в контекстное меню документа и slash-меню.
//! «Свойства» — активная страница (название, иконка, путь в дереве,
//! раскладка) и текущий блок (стиль, фигура, размер); «Связи» — мини-граф,
//! обратные и исходящие ссылки.
//!
//! Раскладка страницы: «Поток» (колонка Notion) или «Свободная» — блоки
//! ставятся мышью в любое место, с привязкой к шагу (по умолчанию 5 px) и
//! фон-сеткой (нет / точки / линии / крест). Настройки живут в дереве
//! (`PageNode.layout`), координаты блоков — в markdown страницы.

use syngui::prelude::*;
use syngui::widgets::input::{SpinBox, Toggle};
use syngui::widgets::navigation::{Tab, TabBar};
use syngui::widgets::input::document_editor::{BlockProps, DocOp, DocumentEditor, ShapeKind, TableOp};
use syngui::widgets::{ColorPicker, ColorValue, Dropdown, DropdownItem, GestureDetector, ToolButton};

use crate::icons::*;

use super::icon_picker;
use super::blocks;
use super::doc_menu;
use super::kanban;
use super::kanban::model::{MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH};
use super::kanban::KanbanHandle;
use super::calendar::model::{CalView, EventStyle, CalendarStyle};
use super::calendar::CalendarHandle;
use super::mindmap::model::{MindNode, MindmapDoc};
use super::mindmap::MindmapHandle;
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

/// Инспектор «Свойства»: активная страница и текущий блок.
fn props_tab(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
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

    // Доска и диаграмма — объекты со своей высотой: у них нет текста, зато
    // есть размер.
    let sized_object = props.embed.as_deref().is_some_and(super::embeds::is_sized_object);
    let kind_label = match props.embed.as_deref() {
        Some(t) if t.starts_with("kanban:") => tr!("notes.block.kanban"),
        Some(t) if t.starts_with("gantt:") => tr!("notes.block.gantt"),
        Some(t) if t.starts_with("mindmap:") => tr!("notes.block.mindmap"),
        Some(t) if t.starts_with("calendar:") => tr!("notes.block.calendar"),
        _ => blocks::kind_label(props.kind, props.level),
    };
    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.block", kind = kind_label)).class("notes-links-section"));

    // Тип блока задаёт умолчания; всё ниже — переопределения поверх темы.
    // У фигуры, картинки и объекта текста нет — кегль, начертание, цвет и
    // выравнивание им ни к чему; их настройки идут секциями ниже.
    if props.shape.is_none() && props.kind != "media" && !sized_object {
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
    // Размеры — у того, у кого высота своя (фигура, картинка, доска,
    // диаграмма): текст растёт по контенту, задавать ему высоту нечего.
    if props.shape.is_some_and(|s| !s.is_line()) || props.kind == "media" || sized_object {
        col = col.child(size_props(&attrs, set));
    }
    // Доска: колонки и внешний вид живут здесь, а не на самой доске.
    if let Some(oid) = props.embed.as_deref().and_then(|t| t.strip_prefix("kanban:")) {
        if let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", oid.trim()) {
            col = col.child(kanban_props(handle));
        }
    }
    // Интеллект-карта: выбранный узел и оформление карты.
    if let Some(oid) = props.embed.as_deref().and_then(|t| t.strip_prefix("mindmap:")) {
        if let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", oid.trim()) {
            col = col.child(mindmap_props(ctx, handle));
        }
    }
    // Календарь: событие, вид, стиль, календари проекта.
    if let Some(oid) = props.embed.as_deref().and_then(|t| t.strip_prefix("calendar:")) {
        if let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", oid.trim()) {
            col = col.child(calendar_props(ctx, handle));
        }
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
        // Видов десять — в строку рядом с подписью они не влезают, поэтому
        // подпись отдельной строкой, а сетка иконок под ней.
        .child(Text::new(tr!("notes.props.shape.kind")).class("notes-props-row-label"))
        .child(shape_picker(ctx, id, shape));
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
    let mut grid = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start);
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (i, kind) in ShapeKind::ALL.into_iter().enumerate() {
        if i > 0 && i % 5 == 0 {
            grid = grid.child(std::mem::replace(
                &mut row,
                Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center),
            ));
        }
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
    grid.child(row)
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

/// Колонки доски (название, цвет, ширина, удаление, добавление) и её
/// внешний вид. Перестраивается по `structure_rev` доски.
fn kanban_props(handle: KanbanHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let selected = handle.selected.get();
        let doc = handle.lock().clone();
        let mut col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-props");
        // Выбранная карточка: приоритет, срок, метки — дубль полей под
        // самой карточкой.
        if let Some(card) = selected.as_deref().and_then(|id| doc.card(id)).cloned() {
            let title = if card.title.trim().is_empty() { tr!("notes.kanban.untitled") } else { card.title.clone() };
            let h_del = handle.clone();
            let id_del = card.id.clone();
            col = col
                .child(Text::new(tr!("notes.kanban.card")).class("notes-links-section"))
                .child(Text::new(title).max_lines(2).class("notes-props-card-title"))
                .child(field_row(
                    tr!("notes.kanban.priority"),
                    kanban::view::priority_control(&handle, &card, 140.0),
                ))
                .child(field_row(tr!("notes.kanban.due"), kanban::view::due_control(&handle, &card, 140.0)))
                .child(Text::new(tr!("notes.kanban.tags")).class("notes-props-row-label"))
                .child(kanban::view::tags_control(&handle, &card, 200.0))
                .child(
                    GestureDetector::new()
                        .cursor(syngui::input::CursorIcon::Pointer)
                        .on_click(move || h_del.delete_card(&id_del))
                        .child(
                            DecoratedBox::new().class("notes-kanban-tail").child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(MI_DELETE).class("notes-insert-icon"))
                                    .child(Text::new(tr!("notes.kanban.delete_card")).class("notes-insert-label")),
                            ),
                        ),
                );
        }
        col = col.child(Text::new(tr!("notes.props.kanban.columns")).class("notes-links-section"));
        for column in &doc.columns {
            let h_color = handle.clone();
            let id_color = column.id.clone();
            let h_name = handle.clone();
            let id_name = column.id.clone();
            let h_w = handle.clone();
            let id_w = column.id.clone();
            let h_del = handle.clone();
            let id_del = column.id.clone();
            let mut dot = DecoratedBox::new().class("notes-props-swatch");
            if column.color.is_empty() {
                dot = dot.class("notes-props-swatch empty");
            } else {
                dot = dot.style("background-color", syngui::core::Color::from_hex(&column.color));
            }
            // Две строки на колонку: в одну название и спинбокс ширины не
            // помещались — цифры переносились.
            col = col
                .child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("notes-props-row")
                        .child(
                            GestureDetector::new()
                                .cursor(syngui::input::CursorIcon::Pointer)
                                .on_click(move || h_color.cycle_column_color(&id_color))
                                .child(dot),
                        )
                        .child(
                            DecoratedBox::new().class("grow").child(
                                TextField::new()
                                    .text(column.name.clone())
                                    .submit_on_focus_lost(true)
                                    .on_submit(move |v: &str| h_name.rename_column(&id_name, v))
                                    .class("notes-props-name"),
                            ),
                        )
                        .child(
                            ToolButton::new(MI_CLOSE)
                                .tooltip(tr!("notes.kanban.delete_column"))
                                .on_click(move || h_del.delete_column(&id_del)),
                        ),
                )
                .child(field_row(
                    tr!("notes.props.size_box.width"),
                    SpinBox::new()
                        .range(MIN_COLUMN_WIDTH as f64, MAX_COLUMN_WIDTH as f64)
                        .step(10.0)
                        .width(96.0)
                        .value(doc.column_width(column) as f64)
                        .on_change(move |v| h_w.set_column_width(&id_w, Some(v as f32)))
                        .class("notes-props-field"),
                ));
        }
        let h_add = handle.clone();
        col = col.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || {
                    h_add.add_column(&tr!("notes.kanban.new_column"));
                })
                .child(
                    DecoratedBox::new().class("notes-kanban-tail").child(
                        Row::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(MI_ADD).class("notes-insert-icon"))
                            .child(Text::new(tr!("notes.kanban.add_column")).class("notes-insert-label")),
                    ),
                ),
        );

        // Внешний вид: общая ширина колонок, фоны, счётчики.
        let style = doc.style.clone();
        let h_cw = handle.clone();
        let h_lane = handle.clone();
        let h_card = handle.clone();
        let h_counts = handle.clone();
        col = col
            .child(Text::new(tr!("notes.props.kanban.style")).class("notes-links-section"))
            .child(field_row(
                tr!("notes.props.kanban.column_width"),
                SpinBox::new()
                    .range(MIN_COLUMN_WIDTH as f64, MAX_COLUMN_WIDTH as f64)
                    .step(10.0)
                    .width(96.0)
                    .value(style.column_width as f64)
                    .on_change(move |v| h_cw.set_style(|s| s.column_width = v as f32))
                    .class("notes-props-field"),
            ))
            .child(field_row(
                tr!("notes.props.kanban.lane_bg"),
                swatches(TINT_PRESETS, Some(style.lane_bg.as_str()), move |v| {
                    h_lane.set_style(|s| s.lane_bg = v.unwrap_or_default())
                }),
            ))
            .child(field_row(
                tr!("notes.props.kanban.card_bg"),
                swatches(TINT_PRESETS, Some(style.card_bg.as_str()), move |v| {
                    h_card.set_style(|s| s.card_bg = v.unwrap_or_default())
                }),
            ))
            .child(switch_row(tr!("notes.props.kanban.counts"), style.show_counts, move |on| {
                h_counts.set_style(|s| s.show_counts = on)
            }));
        vec![Box::new(col)]
    })
}

/// Ряд цветовых кружков; первый — «как в теме» (свойство снимается).
fn swatches(
    presets: &'static [&'static str],
    current: Option<&str>,
    on_pick: impl Fn(Option<String>) + Send + Sync + Clone + 'static,
) -> impl Widget {
    let current = current.unwrap_or("").to_string();
    let mut row = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for preset in presets {
        let on_pick = on_pick.clone();
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
    on_pick: impl Fn(&str) + Send + Sync + Clone + 'static,
) -> impl Widget {
    let current = current.to_string();
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (icon, value) in items {
        let selected = current == *value;
        let v = *value;
        let on_pick = on_pick.clone();
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

/// Кнопка точного цвета: попап `ColorPicker` рядом со свотчами.
fn color_picker(current: Option<&str>, fallback: (u8, u8, u8), on_pick: impl Fn(String) + Send + Sync + 'static) -> impl Widget {
    let initial = current
        .map(|h| ColorValue::from_color(syngui::core::Color::from_hex(h)))
        .unwrap_or_else(|| ColorValue::new(fallback.0, fallback.1, fallback.2));
    ColorPicker::new().color(initial).width(96.0).on_change(move |c| on_pick(c.to_hex()))
}

/// Свойства интеллект-карты: выбранный узел и оформление карты.
fn mindmap_props(ctx: NotesCtx, handle: MindmapHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let selected = handle.selected.get();
        let doc = handle.lock().clone();
        let mut col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-props");
        if let Some(node) = selected.as_deref().and_then(|id| doc.node(id)).cloned() {
            col = col.child(mindmap_node_props(ctx, &handle, &doc, node));
        }
        col = col.child(mindmap_map_props(&handle, &doc));
        vec![Box::new(col)]
    })
}

fn mindmap_node_props(ctx: NotesCtx, handle: &MindmapHandle, doc: &MindmapDoc, node: MindNode) -> impl Widget {
    use super::mindmap::model::NodeShape;
    let id = node.id.clone();
    let is_root = node.parent.is_none();

    let h_text = handle.clone();
    let id_text = id.clone();
    let text = TextField::with_text(node.text.clone())
        .submit_on_focus_lost(true)
        .on_submit(move |t| h_text.set_text(&id_text, t))
        .class("notes-props-field");

    let (editor, source) = handle.note_editor(&id);
    let note = DecoratedBox::new()
        .class("notes-mindmap-note")
        .child(
            DocumentEditor::new()
                .markdown((*source).clone())
                .handle(&editor)
                .plain(true)
                .placeholder(tr!("notes.props.mindmap.note"))
                .class("notes-mindmap-note-editor"),
        );

    let h_color = handle.clone();
    let id_color = id.clone();
    let h_pick = handle.clone();
    let id_pick = id.clone();
    let color_row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(swatches(COLOR_PRESETS, (!node.color.is_empty()).then_some(node.color.as_str()), move |c| {
            h_color.set_color(&id_color, c)
        }))
        .child(color_picker((!node.color.is_empty()).then_some(node.color.as_str()), (79, 140, 255), move |hex| {
            h_pick.set_color(&id_pick, Some(hex))
        }));

    let h_shape = handle.clone();
    let id_shape = id.clone();
    let shape = Dropdown::new()
        .width(132.0)
        .items(
            NodeShape::ALL
                .iter()
                .map(|s| DropdownItem::new(s.key(), syngui::i18n::tr(&format!("notes.mindmap.shape.{}", s.key()))))
                .collect(),
        )
        .selected(node.shape.key())
        .on_change(move |v| {
            if let Some(s) = NodeShape::parse(v) {
                h_shape.set_shape(&id_shape, s);
            }
        })
        .class("notes-props-field");

    let h_icon = handle.clone();
    let id_icon = id.clone();
    let icon = TextField::with_text(node.icon.clone())
        .width(80.0)
        .submit_on_focus_lost(true)
        .on_submit(move |t| h_icon.set_icon(&id_icon, t))
        .class("notes-props-field");

    let h_link = handle.clone();
    let id_link = id.clone();
    let mut pages = vec![DropdownItem::new("", tr!("notes.props.mindmap.no_link"))];
    for n in ctx.tree.get_untracked().all() {
        pages.push(DropdownItem::new(n.id.clone(), n.title.clone()));
    }
    let link = Dropdown::new()
        .width(160.0)
        .items(pages)
        .selected(node.link.clone().unwrap_or_default())
        .on_change(move |v| h_link.set_link(&id_link, (!v.is_empty()).then(|| v.to_string())))
        .class("notes-props-field");

    let h_collapse = handle.clone();
    let id_collapse = id.clone();
    let collapsed = switch_row(tr!("notes.props.mindmap.collapsed"), node.collapsed, move |on| {
        h_collapse.set_collapsed(&id_collapse, on)
    });

    let h_reset = handle.clone();
    let id_reset = id.clone();
    let h_del = handle.clone();
    let id_del = id.clone();
    let mut actions = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            ToolButton::new(MI_AUTORENEW)
                .text(tr!("notes.mindmap.reset_offset"))
                .on_click(move || h_reset.reset_offset(&id_reset)),
        );
    if !is_root {
        actions = actions.child(
            ToolButton::new(MI_DELETE).text(tr!("notes.mindmap.delete")).on_click(move || {
                h_del.delete(&id_del);
            }),
        );
    }

    let _ = doc;
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.mindmap.node")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.mindmap.text"), text))
        .child(note)
        .child(field_row(tr!("notes.props.color"), color_row))
        .child(field_row(tr!("notes.props.mindmap.shape"), shape))
        .child(field_row(tr!("notes.props.mindmap.icon"), icon))
        .child(field_row(tr!("notes.props.mindmap.link"), link))
        .child(collapsed)
        .child(actions)
}

fn mindmap_map_props(handle: &MindmapHandle, doc: &MindmapDoc) -> impl Widget {
    use super::mindmap::model::{palette_by_key, Curve, Direction, PALETTES};
    let layout = doc.layout.clone();
    let style = doc.style.clone();

    let h = handle.clone();
    let direction = Dropdown::new()
        .width(132.0)
        .items(Direction::ALL.iter().map(|d| DropdownItem::new(d.key(), super::mindmap::view::direction_label(*d))).collect())
        .selected(layout.direction.key())
        .on_change(move |v| {
            if let Some(d) = Direction::parse(v) {
                h.set_direction(d);
            }
        })
        .class("notes-props-field");

    let h = handle.clone();
    let curve = Dropdown::new()
        .width(132.0)
        .items(
            Curve::ALL
                .iter()
                .map(|c| DropdownItem::new(c.key(), syngui::i18n::tr(&format!("notes.mindmap.curve.{}", c.key()))))
                .collect(),
        )
        .selected(layout.curve.key())
        .on_change(move |v| {
            if let Some(c) = Curve::parse(v) {
                h.set_curve(c);
            }
        })
        .class("notes-props-field");

    let spin = |value: f32, lo: f64, hi: f64, step: f64, on: Box<dyn Fn(f32) + Send + Sync>| {
        SpinBox::new()
            .range(lo, hi)
            .step(step)
            .width(96.0)
            .value(value as f64)
            .on_change(move |v| on(v as f32))
            .class("notes-props-field")
    };
    let h = handle.clone();
    let h_gap = spin(layout.h_gap, 8.0, 400.0, 4.0, Box::new(move |v| h.set_layout(|l| l.h_gap = v)));
    let h = handle.clone();
    let v_gap = spin(layout.v_gap, 0.0, 200.0, 2.0, Box::new(move |v| h.set_layout(|l| l.v_gap = v)));

    // Палитра: пресет либо «своя», плюс её цвета для наглядности.
    let current_palette = PALETTES
        .iter()
        .find(|(_, p)| p.iter().map(|s| s.to_string()).collect::<Vec<_>>() == style.palette)
        .map(|(k, _)| *k)
        .unwrap_or("custom");
    let mut palette_items: Vec<DropdownItem> = PALETTES
        .iter()
        .map(|(k, _)| DropdownItem::new(*k, syngui::i18n::tr(&format!("notes.mindmap.palette.{k}"))))
        .collect();
    if current_palette == "custom" {
        palette_items.push(DropdownItem::new("custom", "…"));
    }
    let h = handle.clone();
    let palette = Dropdown::new()
        .width(132.0)
        .items(palette_items)
        .selected(current_palette)
        .on_change(move |v| {
            if let Some(p) = palette_by_key(v) {
                let colors: Vec<String> = p.iter().map(|s| s.to_string()).collect();
                h.set_style(move |s| s.palette = colors);
            }
        })
        .class("notes-props-field");
    let mut palette_row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for c in &style.palette {
        palette_row = palette_row.child(
            DecoratedBox::new().class("notes-props-swatch").style("background-color", syngui::core::Color::from_hex(c)),
        );
    }

    let tint_row = |current: String, on: Box<dyn Fn(Option<String>) + Send + Sync>| {
        let on = std::sync::Arc::new(on);
        let on_sw = on.clone();
        let on_pk = on.clone();
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(swatches(TINT_PRESETS, (!current.is_empty()).then_some(current.as_str()), move |c| on_sw(c)))
            .child(color_picker((!current.is_empty()).then_some(current.as_str()), (36, 49, 73), move |hex| on_pk(Some(hex))))
    };
    let color_row = |current: String, on: Box<dyn Fn(Option<String>) + Send + Sync>| {
        let on = std::sync::Arc::new(on);
        let on_sw = on.clone();
        let on_pk = on.clone();
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(swatches(COLOR_PRESETS, (!current.is_empty()).then_some(current.as_str()), move |c| on_sw(c)))
            .child(color_picker((!current.is_empty()).then_some(current.as_str()), (79, 140, 255), move |hex| on_pk(Some(hex))))
    };

    let h = handle.clone();
    let node_fill = tint_row(style.node_fill.clone(), Box::new(move |c| h.set_style(|s| s.node_fill = c.unwrap_or_default())));
    let h = handle.clone();
    let node_stroke = color_row(style.node_stroke.clone(), Box::new(move |c| h.set_style(|s| s.node_stroke = c.unwrap_or_default())));
    let h = handle.clone();
    let text_color = color_row(style.text_color.clone(), Box::new(move |c| h.set_style(|s| s.text_color = c.unwrap_or_default())));
    let h = handle.clone();
    let line_color = color_row(style.line_color.clone(), Box::new(move |c| h.set_style(|s| s.line_color = c.unwrap_or_default())));
    let h = handle.clone();
    let bg = tint_row(style.bg.clone(), Box::new(move |c| h.set_style(|s| s.bg = c.unwrap_or_default())));

    let h = handle.clone();
    let font_size = spin(style.font_size, 8.0, 40.0, 1.0, Box::new(move |v| h.set_style(|s| s.font_size = v)));
    let h = handle.clone();
    let weight = segmented(
        &[(MI_ARTICLE, "normal"), (MI_FORMAT_BOLD, "bold")],
        if style.weight == "bold" { "bold" } else { "normal" },
        move |v| {
            let v = v.to_string();
            h.set_style(move |s| s.weight = if v == "bold" { v } else { String::new() });
        },
    );
    let h = handle.clone();
    let radius = spin(style.radius, 0.0, 40.0, 1.0, Box::new(move |v| h.set_style(|s| s.radius = v)));
    let h = handle.clone();
    let padding = spin(style.padding, 2.0, 30.0, 1.0, Box::new(move |v| h.set_style(|s| s.padding = v)));
    let h = handle.clone();
    let line_width = SpinBox::new()
        .range(0.5, 8.0)
        .step(0.5)
        .decimal_places(1)
        .width(96.0)
        .value(style.line_width as f64)
        .on_change(move |v| h.set_style(|s| s.line_width = v as f32))
        .class("notes-props-field");
    let h = handle.clone();
    let line_dash = spin(style.line_dash, 0.0, 30.0, 1.0, Box::new(move |v| h.set_style(|s| s.line_dash = v)));
    let h = handle.clone();
    let max_w = spin(style.max_node_w, 80.0, 800.0, 20.0, Box::new(move |v| h.set_style(|s| s.max_node_w = v)));
    let h = handle.clone();
    let show_icons = switch_row(tr!("notes.props.mindmap.show_icons"), style.show_icons, move |on| {
        h.set_style(|s| s.show_icons = on)
    });
    let h = handle.clone();
    let reset = ToolButton::new(MI_AUTORENEW).text(tr!("notes.mindmap.reset_layout")).on_click(move || h.reset_offsets());

    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.mindmap.map")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.mindmap.direction"), direction))
        .child(field_row(tr!("notes.props.mindmap.curve"), curve))
        .child(field_row(tr!("notes.props.mindmap.h_gap"), h_gap))
        .child(field_row(tr!("notes.props.mindmap.v_gap"), v_gap))
        .child(field_row(tr!("notes.props.mindmap.palette"), palette))
        .child(palette_row)
        .child(field_row(tr!("notes.props.mindmap.node_fill"), node_fill))
        .child(field_row(tr!("notes.props.mindmap.node_stroke"), node_stroke))
        .child(field_row(tr!("notes.props.mindmap.text_color"), text_color))
        .child(field_row(tr!("notes.props.size"), font_size))
        .child(field_row(tr!("notes.props.weight"), weight))
        .child(field_row(tr!("notes.props.mindmap.radius"), radius))
        .child(field_row(tr!("notes.props.mindmap.padding"), padding))
        .child(field_row(tr!("notes.props.mindmap.line_color"), line_color))
        .child(field_row(tr!("notes.props.mindmap.line_width"), line_width))
        .child(field_row(tr!("notes.props.mindmap.line_dash"), line_dash))
        .child(field_row(tr!("notes.props.bg"), bg))
        .child(show_icons)
        .child(field_row(tr!("notes.props.mindmap.max_w"), max_w))
        .child(reset)
}

/// Свойства календаря: выбранное событие, вид, стиль, календари проекта.
fn calendar_props(ctx: NotesCtx, handle: CalendarHandle) -> impl Widget {
    let store = ctx.calendar_store();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let _ = store.revision.get();
        let selected = handle.selected.get();
        let doc = handle.lock().clone();
        let data = store.lock().clone();
        let mut col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-props");
        if let Some(e) = selected.as_deref().and_then(|id| data.event(id)).cloned() {
            col = col.child(calendar_event_props(&store, &handle, &data, e));
        }
        col = col.child(calendar_view_props(&handle, &doc, &data));
        col = col.child(calendar_style_props(&handle, &doc.style));
        col = col.child(calendar_list_props(&store, &data));
        vec![Box::new(col)]
    })
}

fn calendar_event_props(
    store: &super::calendar::CalendarStoreHandle,
    handle: &CalendarHandle,
    data: &super::calendar::model::CalendarStore,
    e: super::calendar::model::CalEvent,
) -> impl Widget {
    use syngui::widgets::input::{DatePicker, Date};
    let id = e.id.clone();
    let s = store.clone();
    let id_t = id.clone();
    let title = TextField::with_text(e.title.clone())
        .submit_on_focus_lost(true)
        .on_submit(move |t| {
            let t = t.to_string();
            s.update_event(&id_t, move |e| e.title = t);
        })
        .class("notes-props-field");
    let s = store.clone();
    let id_d = id.clone();
    let date = DatePicker::new()
        .width(140.0)
        .selected(
            super::gantt::calendar::parse_days(&e.date)
                .map(|d| {
                    let (y, m, dd) = super::gantt::calendar::civil_from_days(d);
                    Date::new(y as i32, m, dd)
                })
                .unwrap_or_else(Date::today),
        )
        .on_change(move |d| {
            if let Some(d) = d {
                let day = super::gantt::calendar::days_from_civil(d.year as i64, d.month, d.day);
                s.move_event(&id_d, day, None);
            }
        });
    let s = store.clone();
    let id_done = id.clone();
    let done = switch_row(tr!("notes.calendar.event.done"), e.done, move |on| {
        s.update_event(&id_done, move |e| e.done = on);
    });
    let s = store.clone();
    let id_c = id.clone();
    let calendar = Dropdown::new()
        .width(160.0)
        .items(data.calendars.iter().map(|c| DropdownItem::new(c.id.clone(), c.name.clone())).collect())
        .selected(e.calendar.clone())
        .on_change(move |v| {
            let v = v.to_string();
            s.update_event(&id_c, move |e| e.calendar = v);
        })
        .class("notes-props-field");
    let s = store.clone();
    let id_col = id.clone();
    let color = swatches(COLOR_PRESETS, (!e.color.is_empty()).then_some(e.color.as_str()), move |c| {
        let c = c.unwrap_or_default();
        s.update_event(&id_col, move |e| e.color = c);
    });
    let h_open = handle.clone();
    let e_open = e.clone();
    let s_del = store.clone();
    let h_del = handle.clone();
    let id_del = id.clone();
    let actions = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(ToolButton::new(MI_EDIT).text(tr!("app.edit")).on_click(move || {
            h_open.open_edit(e_open.clone(), syngui::core::Rect::new(syngui::core::Point::new(0.0, 40.0), syngui::core::Size::new(1.0, 1.0)))
        }))
        .child(ToolButton::new(MI_DELETE).text(tr!("notes.calendar.event.delete")).on_click(move || {
            s_del.remove_event(&id_del);
            h_del.select(None);
        }));
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.calendar.event")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.mindmap.text"), title))
        .child(field_row(tr!("notes.calendar.event.date"), date))
        .child(done)
        .child(field_row(tr!("notes.calendar.event.calendar"), calendar))
        .child(field_row(tr!("notes.props.color"), color))
        .child(actions)
}

fn calendar_view_props(
    handle: &CalendarHandle,
    doc: &super::calendar::model::CalendarDoc,
    data: &super::calendar::model::CalendarStore,
) -> impl Widget {
    use syngui::widgets::input::{DatePicker, Date};
    let style = doc.style.clone();
    let h = handle.clone();
    let view = Dropdown::new()
        .width(132.0)
        .items(CalView::ALL.iter().map(|v| DropdownItem::new(v.key(), syngui::i18n::tr(&format!("notes.calendar.view.{}", v.key())))).collect())
        .selected(doc.view.key())
        .on_change(move |v| {
            if let Some(v) = CalView::parse(v) {
                h.set_view(v);
            }
        })
        .class("notes-props-field");
    let h = handle.clone();
    let (y, m, d) = super::gantt::calendar::civil_from_days(doc.anchor_days());
    let anchor = DatePicker::new().width(140.0).selected(Date::new(y as i32, m, d)).on_change(move |dt| {
        if let Some(dt) = dt {
            h.set_anchor(super::gantt::calendar::days_from_civil(dt.year as i64, dt.month, dt.day));
        }
    });
    let locale = super::calendar::view::locale();
    let h = handle.clone();
    let first = Dropdown::new()
        .width(132.0)
        .items([0u32, 5, 6].iter().map(|wd| DropdownItem::new(wd.to_string(), locale.weekday_short(*wd).to_string())).collect())
        .selected(style.first_weekday.to_string())
        .on_change(move |v| {
            if let Ok(wd) = v.parse::<u32>() {
                h.set_style(move |s| s.first_weekday = wd);
            }
        })
        .class("notes-props-field");
    let h = handle.clone();
    let week_numbers = switch_row(tr!("notes.props.calendar.week_numbers"), style.show_week_numbers, move |on| h.set_style(|s| s.show_week_numbers = on));
    let h1 = handle.clone();
    let h2 = handle.clone();
    let hours = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            SpinBox::new().range(0.0, 23.0).step(1.0).width(72.0).value(style.hour_from as f64).on_change(move |v| h1.set_style(|s| s.hour_from = v as u32)).class("notes-props-field"),
        )
        .child(Text::new("–").class("notes-props-hint"))
        .child(
            SpinBox::new().range(1.0, 24.0).step(1.0).width(72.0).value(style.hour_to as f64).on_change(move |v| h2.set_style(|s| s.hour_to = v as u32)).class("notes-props-field"),
        );
    let h = handle.clone();
    let slot = Dropdown::new()
        .width(96.0)
        .items([5u32, 10, 15, 20, 30, 60].iter().map(|m| DropdownItem::new(m.to_string(), m.to_string())).collect())
        .selected(style.slot_min.to_string())
        .on_change(move |v| {
            if let Ok(m) = v.parse::<u32>() {
                h.set_style(move |s| s.slot_min = m);
            }
        })
        .class("notes-props-field");
    let h = handle.clone();
    let compact = switch_row(tr!("notes.props.calendar.compact"), style.compact, move |on| h.set_style(|s| s.compact = on));
    let h = handle.clone();
    let kanban = switch_row(tr!("notes.props.calendar.show_kanban"), style.show_kanban_due, move |on| h.set_style(|s| s.show_kanban_due = on));
    let h = handle.clone();
    let gantt = switch_row(tr!("notes.props.calendar.show_gantt"), style.show_gantt, move |on| h.set_style(|s| s.show_gantt = on));

    // Фильтр календарей: чипы-переключатели; пустой фильтр = все.
    let all: Vec<String> = data.calendars.iter().map(|c| c.id.clone()).collect();
    let mut chips = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for c in &data.calendars {
        let visible = doc.calendars.is_empty() || doc.calendars.contains(&c.id);
        let h = handle.clone();
        let id = c.id.clone();
        let all = all.clone();
        let mut chip = DecoratedBox::new().class(if visible { "notes-calendar-chip selected" } else { "notes-calendar-chip" });
        if visible && !c.color.is_empty() {
            chip = chip.style("border-color", syngui::core::Color::from_hex(&c.color));
        }
        chips = chips.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || h.toggle_calendar(&id, &all))
                .child(chip.child(Text::new(c.name.clone()).max_lines(1).class("notes-calendar-chip-text"))),
        );
    }

    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.calendar.view")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.calendar.view"), view))
        .child(field_row(tr!("notes.props.calendar.anchor"), anchor))
        .child(field_row(tr!("notes.props.calendar.calendars"), chips))
        .child(field_row(tr!("notes.props.calendar.first_weekday"), first))
        .child(week_numbers)
        .child(field_row(tr!("notes.props.calendar.hours"), hours))
        .child(field_row(tr!("notes.props.calendar.slot"), slot))
        .child(compact)
        .child(kanban)
        .child(gantt)
}

fn calendar_style_props(handle: &CalendarHandle, style: &CalendarStyle) -> impl Widget {
    let h = handle.clone();
    let preset = Dropdown::new()
        .width(132.0)
        .items(CalendarStyle::PRESETS.iter().map(|p| DropdownItem::new(*p, syngui::i18n::tr(&format!("notes.calendar.preset.{p}")))).collect())
        .selected(if style.preset.is_empty() { "theme".to_string() } else { style.preset.clone() })
        .on_change(move |v| {
            let v = v.to_string();
            h.set_style(move |s| s.apply_preset(&v));
        })
        .class("notes-props-field");
    let h = handle.clone();
    let event_style = Dropdown::new()
        .width(132.0)
        .items(EventStyle::ALL.iter().map(|e| DropdownItem::new(e.key(), syngui::i18n::tr(&format!("notes.calendar.event_style.{}", e.key())))).collect())
        .selected(style.event_style.key())
        .on_change(move |v| {
            if let Some(e) = EventStyle::parse(v) {
                h.set_style(move |s| s.event_style = e);
            }
        })
        .class("notes-props-field");
    let h = handle.clone();
    let font = SpinBox::new()
        .range(8.0, 24.0)
        .step(1.0)
        .width(96.0)
        .value(style.font_size as f64)
        .on_change(move |v| h.set_style(|s| s.font_size = v as f32))
        .class("notes-props-field");
    let color_row = |current: String, presets: &'static [&'static str], fallback: (u8, u8, u8), on: Box<dyn Fn(String) + Send + Sync>| {
        let on = std::sync::Arc::new(on);
        let on_sw = on.clone();
        let on_pk = on.clone();
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(swatches(presets, (!current.is_empty()).then_some(current.as_str()), move |c| on_sw(c.unwrap_or_default())))
            .child(color_picker((!current.is_empty()).then_some(current.as_str()), fallback, move |hex| on_pk(hex)))
    };
    let h = handle.clone();
    let today = color_row(style.today_color.clone(), COLOR_PRESETS, (79, 140, 255), Box::new(move |c| h.set_style(|s| { s.today_color = c; s.preset.clear(); })));
    let h = handle.clone();
    let header = color_row(style.header_bg.clone(), TINT_PRESETS, (36, 49, 73), Box::new(move |c| h.set_style(|s| { s.header_bg = c; s.preset.clear(); })));
    let h = handle.clone();
    let cell = color_row(style.cell_bg.clone(), TINT_PRESETS, (36, 49, 73), Box::new(move |c| h.set_style(|s| { s.cell_bg = c; s.preset.clear(); })));
    let h = handle.clone();
    let grid = color_row(style.grid_color.clone(), COLOR_PRESETS, (139, 149, 166), Box::new(move |c| h.set_style(|s| { s.grid_color = c; s.preset.clear(); })));
    let h = handle.clone();
    let weekend = color_row(style.weekend_tint.clone(), TINT_PRESETS, (36, 49, 73), Box::new(move |c| h.set_style(|s| { s.weekend_tint = c; s.preset.clear(); })));
    let h = handle.clone();
    let text = color_row(style.text_color.clone(), COLOR_PRESETS, (230, 232, 238), Box::new(move |c| h.set_style(|s| { s.text_color = c; s.preset.clear(); })));
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.calendar.style")).class("notes-links-section"))
        .child(field_row(tr!("notes.props.calendar.preset"), preset))
        .child(field_row(tr!("notes.props.calendar.event_style"), event_style))
        .child(field_row(tr!("notes.props.size"), font))
        .child(field_row(tr!("notes.props.calendar.today_color"), today))
        .child(field_row(tr!("notes.props.calendar.header_bg"), header))
        .child(field_row(tr!("notes.props.calendar.cell_bg"), cell))
        .child(field_row(tr!("notes.props.calendar.grid_color"), grid))
        .child(field_row(tr!("notes.props.calendar.weekend"), weekend))
        .child(field_row(tr!("notes.props.calendar.text_color"), text))
}

fn calendar_list_props(store: &super::calendar::CalendarStoreHandle, data: &super::calendar::model::CalendarStore) -> impl Widget {
    let mut col = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("notes.props.calendar.list")).class("notes-links-section"));
    let removable = data.calendars.len() > 1;
    for c in &data.calendars {
        let s = store.clone();
        let id_name = c.id.clone();
        let name = TextField::with_text(c.name.clone())
            .submit_on_focus_lost(true)
            .on_submit(move |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    s.edit(|st| {
                        if let Some(cal) = st.calendars.iter_mut().find(|c| c.id == id_name) {
                            cal.name = t.clone();
                        }
                    });
                }
            })
            .class("notes-props-field");
        let s = store.clone();
        let id_color = c.id.clone();
        let color = swatches(COLOR_PRESETS, (!c.color.is_empty()).then_some(c.color.as_str()), move |v| {
            let v = v.unwrap_or_default();
            s.edit(|st| {
                if let Some(cal) = st.calendars.iter_mut().find(|c| c.id == id_color) {
                    cal.color = v.clone();
                }
            });
        });
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(name));
        if removable {
            let s = store.clone();
            let id_del = c.id.clone();
            row = row.child(ToolButton::new(MI_CLOSE).on_click(move || {
                s.remove_calendar(&id_del);
            }));
        }
        col = col.child(row).child(color);
    }
    let s = store.clone();
    col = col.child(
        ToolButton::new(MI_ADD).text(tr!("notes.props.calendar.add_calendar")).on_click(move || {
            s.add_calendar(&tr!("notes.props.calendar.new_calendar"), "");
        }),
    );
    col
}

/// Цвета текста и подложки: пусто — «как в теме».
const COLOR_PRESETS: &[&str] =
    &["", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#8B95A6"];
const BG_PRESETS: &[&str] =
    &["", "#3A2A28", "#3A3324", "#243A2E", "#243149", "#332944", "#2B2F36"];
/// Полупрозрачные оттенки для фонов доски: ложатся на любую тему, а не
/// чёрные пятна, как тёмные подложки текста.
const TINT_PRESETS: &[&str] =
    &["", "#EE5E4833", "#E8A33D33", "#4FBF7A33", "#4F8CFF33", "#C08FE833", "#8B95A633"];
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

    let snap_step_id = id.clone();
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

    // Фон страницы: пресеты подложек + точный цвет через ColorPicker.
    let bg_current = (!layout.bg.is_empty()).then(|| layout.bg.clone());
    let bg_id = id.clone();
    let bg_swatches = swatches(BG_PRESETS, bg_current.as_deref(), move |c| {
        ctx.set_page_layout(&bg_id, PageLayout { bg: c.unwrap_or_default(), ..ctx.page_layout(&bg_id) });
    });
    let picker_id = id;
    let initial = bg_current
        .as_deref()
        .map(|h| ColorValue::from_color(syngui::core::Color::from_hex(h)))
        .unwrap_or_else(|| ColorValue::new(36, 49, 73));
    let picker = ColorPicker::new()
        .color(initial)
        .width(96.0)
        .on_change(move |c| {
            ctx.set_page_layout(&picker_id, PageLayout { bg: c.to_hex(), ..ctx.page_layout(&picker_id) });
        });
    let bg_row = field_row(
        tr!("notes.props.layout.bg"),
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(bg_swatches).child(picker),
    );

    col = col
        .child(field_row(tr!("notes.props.layout.grid"), grid))
        .child(field_row(tr!("notes.props.layout.grid_step"), grid_step))
        .child(snap_row)
        .child(field_row(tr!("notes.props.layout.snap_step"), snap_step))
        .child(bg_row)
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

fn placeholder(icon: &'static str, text: String) -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("notes-empty-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
