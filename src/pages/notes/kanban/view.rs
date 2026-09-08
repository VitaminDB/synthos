//! Виджет канбан-доски: колонки в горизонтальной прокрутке, карточки
//! перетаскиваются внутри колонки, между колонками и **между досками**
//! (payload — `<доска>|<карточка>`, см. [`super::drop_card`]); блоки
//! страницы, взятые за ⋮⋮, приходят тем же drag'ом дерева
//! ([`DRAG_TYPE_BLOCK`]) и становятся карточками.
//!
//! Цели дропа — вся колонка: `DropArea` каждой карточки (верхняя половина
//! — «перед ней», нижняя — «после») и `DropArea` тела колонки (пустое
//! место, хвост — «в конец»); дерево отдаёт событие самой глубокой цели.
//! Место вставки показывает **плейсхолдер** — пунктирная пустая карточка
//! высотой с переносимую ([`gap`], сигнал `hover` ручки). Доска целиком
//! тоже `DropArea`: дроп мимо колонок (шапка, зазор) поглощается, а не
//! уходит на страницу.
//!
//! Карточка: бейдж приоритета и метки, заголовок, первые строки
//! содержимого без разметки, срок и прогресс чек-листа. Клик — правка
//! одним `DocumentEditor` (`## …` — заголовок); у выбранной карточки под
//! текстом ряд полей (приоритет, срок, метки, удалить) — он остаётся и
//! после закрытия правки, чтобы клик по полю не терял их.

use std::sync::atomic::{AtomicU32, Ordering};

use syngui::core::Color;
use syngui::input::{CursorIcon, DragData};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditor;
use syngui::widgets::overlay::{Draggable, DropArea, DropInfo};
use syngui::widgets::{Date, DatePicker, Dropdown, DropdownItem, GestureDetector, Image, ImageFit, MenuItem, PopupMenu, ProgressBar, SpinBox};

use crate::icons::*;

use super::super::calendar::model::fmt_hm;
use super::super::gantt::calendar::{civil_from_days, days_from_civil, parse_days, short_date, today_days};
use super::drag_strip::DragStrip;
use super::model::{parse_tags, preview_text, tag_color, CardFile, DropSpot, KanbanCard, KanbanColumn, KanbanDoc, Moment, Priority, Repeat, DAY_MIN};
pub use super::sinks::DRAG_TYPE_BLOCK;
use super::{drop_block, drop_card, BoardEnv, KanbanHandle};

pub const DRAG_TYPE_CARD: &str = "notes-kanban-card";

/// Высота плейсхолдера, когда переносят не карточку (блок страницы).
const DEFAULT_GAP_H: f32 = 44.0;
/// Высота переносимой карточки (общая: карточка может ехать на другую
/// доску, у которой своей ручки источника нет).
static CARD_DRAG_H: AtomicU32 = AtomicU32::new(0);

fn accept_types() -> Vec<String> {
    vec![DRAG_TYPE_CARD.to_string(), DRAG_TYPE_BLOCK.to_string()]
}

pub fn view(env: BoardEnv, board: String, handle: KanbanHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let editing = handle.editing.get();
        let selected = handle.selected.get();
        vec![build(&env, &board, handle.clone(), editing.as_deref(), selected.as_deref())]
    })
}

fn build(
    env: &BoardEnv,
    board: &str,
    handle: KanbanHandle,
    editing: Option<&str>,
    selected: Option<&str>,
) -> Box<dyn Widget> {
    let doc = handle.lock().clone();
    let mut lanes = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban");
    for column in &doc.columns {
        lanes = lanes.child(lane(env, board, &handle, &doc, column, editing, selected));
    }
    // Доска целиком: дроп мимо колонок поглощается, плейсхолдер снимается.
    let h_over = handle.clone();
    let h_leave = handle.clone();
    let h_drop = handle.clone();
    Box::new(
        DropArea::new()
            .accept_types(accept_types())
            .on_drag_over(move |_| h_over.clear_hover())
            .on_drag_leave(move || h_leave.clear_hover())
            .on_drop(move |_| h_drop.clear_hover())
            .child(ScrollView::new().horizontal().class("notes-kanban-scroll").child(lanes)),
    )
}

/// Дроп в место `spot`: карточка (своя или с другой доски) либо блок страницы.
fn drop_into(env: &BoardEnv, data: &DragData, board: &str, handle: &KanbanHandle, spot: &DropSpot) {
    handle.clear_hover();
    if data.drag_type == DRAG_TYPE_CARD {
        drop_card(&env.boards, &data.payload, board, handle, spot);
    } else if data.drag_type == DRAG_TYPE_BLOCK {
        drop_block(&env.take_block, &data.payload, handle, spot);
    }
}

/// Показать плейсхолдер в месте `spot` для переносимых данных.
fn hover_at(handle: &KanbanHandle, board: &str, data: &DragData, spot: Option<DropSpot>) {
    let h = if data.drag_type == DRAG_TYPE_CARD { f32::from_bits(CARD_DRAG_H.load(Ordering::Relaxed)) } else { 0.0 };
    if handle.drag_h.get_untracked() != h {
        handle.drag_h.set(h);
    }
    handle.set_hover(spot, board, &data.payload);
}

#[allow(clippy::too_many_arguments)]
fn lane(
    env: &BoardEnv,
    board: &str,
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    column: &KanbanColumn,
    editing: Option<&str>,
    selected: Option<&str>,
) -> impl Widget {
    let cards_in = doc.cards_of(&column.id);
    let col_id = column.id.clone();
    let width = doc.column_width(column);

    // Шапка: метка цвета (клик — следующий цвет), название, счётчик, «+».
    let h_color = handle.clone();
    let id_color = col_id.clone();
    let h_name = handle.clone();
    let id_name = col_id.clone();
    let h_add = handle.clone();
    let id_add = col_id.clone();
    let mut header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-kanban-lane-header")
        .child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || h_color.cycle_column_color(&id_color))
                .child(color_dot(&column.color)),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                TextField::new()
                    .text(column.name.clone())
                    .submit_on_focus_lost(true)
                    .on_submit(move |v: &str| h_name.rename_column(&id_name, v))
                    .class("notes-kanban-lane-name"),
            ),
        );
    if doc.style.show_counts {
        header = header.child(Text::new(format!("{}", cards_in.len())).class("notes-kanban-lane-count"));
    }
    header = header.child(
        ToolButton::new(MI_ADD)
            .tooltip(tr!("notes.kanban.add_card"))
            .on_click(move || {
                h_add.add_card(&id_add);
            })
            .class("notes-kanban-lane-btn"),
    );

    let mut cards = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban-cards");
    for c in cards_in {
        cards = cards.child(card_slot(
            env,
            board,
            handle,
            doc,
            &column.color,
            c,
            editing == Some(c.id.as_str()),
            selected == Some(c.id.as_str()),
            width,
        ));
    }
    // Клик по пустому месту колонки закрывает правку и снимает выбор.
    let h_blur = handle.clone();
    let cards_area = GestureDetector::new()
        .on_click(move || {
            if let Some(id) = h_blur.editing.get_untracked() {
                h_blur.finish_editing(&id);
            }
            h_blur.select(None);
        })
        .child(ScrollView::new().vertical().child(cards));

    // Хвост: полоса «+» — новая карточка.
    let h_tail_add = handle.clone();
    let id_tail_add = col_id.clone();
    let tail = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            h_tail_add.add_card(&id_tail_add);
        })
        .child(
            DecoratedBox::new()
                .class("notes-kanban-tail")
                .child(Center::new().child(Icon::new(MI_ADD).class("notes-kanban-tail-icon"))),
        );

    // Тело колонки — цель дропа «в конец»: пустое место и хвост.
    let end = DropSpot::end(&col_id);
    let h_over = handle.clone();
    let board_over = board.to_string();
    let end_over = end.clone();
    let h_drop = handle.clone();
    let board_drop = board.to_string();
    let env_drop = env.clone();
    let end_drop = end.clone();
    let body = DropArea::new()
        .accept_types(accept_types())
        .on_drag_over(move |info: DropInfo| hover_at(&h_over, &board_over, &info.data, Some(end_over.clone())))
        .on_drop(move |data| drop_into(&env_drop, &data, &board_drop, &h_drop, &end_drop))
        .child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("grow").child(cards_area))
                .child(gap(handle, end))
                .child(tail),
        )
        .class("grow");

    let mut lane_box = DecoratedBox::new()
        .class("notes-kanban-lane")
        .style("width", StyleValue::px(width));
    if !doc.style.lane_bg.is_empty() {
        lane_box = lane_box.style("background-color", Color::from_hex(&doc.style.lane_bg));
    }
    let lane_box = lane_box.child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(body),
    );

    // Правая кромка — ширина колонки (приращения, текущее — из документа).
    let h_w = handle.clone();
    let id_w = col_id;
    let strip = DragStrip::new(move |dx| {
        let w = h_w.column_width(&id_w) + dx;
        h_w.set_column_width(&id_w, Some(w));
    });

    Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(lane_box)
        .child(DecoratedBox::new().class("notes-kanban-resize").child(strip))
}

fn color_dot(color: &str) -> impl Widget {
    let mut dot = DecoratedBox::new().class("notes-kanban-dot");
    if color.is_empty() {
        dot = dot.class("notes-kanban-dot empty");
    } else {
        dot = dot.style("background-color", Color::from_hex(color));
    }
    dot
}

/// Плейсхолдер места вставки: пустая пунктирная карточка высотой с
/// переносимую, пока `hover` ручки указывает на `spot`; иначе — ничего.
/// Своя реактивная обёртка на каждый зазор — доска не перестраивается
/// на каждое движение курсора, а карточка-источник живёт (призрак
/// переноса — её живой снимок).
fn gap(handle: &KanbanHandle, spot: DropSpot) -> impl Widget {
    let h = handle.clone();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let active = h.hover.get().as_ref() == Some(&spot);
        let height = if active {
            let v = h.drag_h.get_untracked();
            if v > 0.0 { v } else { DEFAULT_GAP_H }
        } else {
            0.0
        };
        let (class, spacer) = if active { ("notes-kanban-gap active", 8.0) } else { ("notes-kanban-gap", 0.0) };
        vec![Box::new(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class(class).style("height", StyleValue::px(height)))
                .child(DecoratedBox::new().style("height", StyleValue::px(spacer))),
        )]
    })
}

/// Слот карточки: цель дропа (верхняя половина — перед ней, нижняя —
/// после), плейсхолдер «перед ней» и сама карточка.
#[allow(clippy::too_many_arguments)]
fn card_slot(
    env: &BoardEnv,
    board: &str,
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    lane_color: &str,
    c: &KanbanCard,
    editing: bool,
    selected: bool,
    lane_width: f32,
) -> impl Widget {
    let id = c.id.clone();
    let spot_of = {
        let h = handle.clone();
        let id = id.clone();
        move |info: &DropInfo| -> Option<DropSpot> {
            let upper = info.local_position.y < info.size.height / 2.0;
            let doc = h.lock();
            if upper { doc.spot_before(&id) } else { doc.spot_after(&id) }
        }
    };
    let h_over = handle.clone();
    let board_over = board.to_string();
    let spot_over = spot_of.clone();
    let h_drop = handle.clone();
    let board_drop = board.to_string();
    let env_drop = env.clone();
    DropArea::new()
        .accept_types(accept_types())
        .on_drag_over(move |info: DropInfo| {
            let spot = spot_over(&info);
            hover_at(&h_over, &board_over, &info.data, spot);
        })
        .on_drop_positioned(move |info: DropInfo| {
            if let Some(spot) = spot_of(&info) {
                drop_into(&env_drop, &info.data, &board_drop, &h_drop, &spot);
            } else {
                h_drop.clear_hover();
            }
        })
        .child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(gap(handle, DropSpot::before(&c.column, &id)))
                .child(Stack::new().children(vec![card(env, board, handle, doc, lane_color, c, editing, selected, lane_width)])),
        )
}

/// Карточка: заголовок, поля и содержимое с цветной полосой колонки;
/// перетаскивается, по клику — правка на месте.
#[allow(clippy::too_many_arguments)]
fn card(
    env: &BoardEnv,
    board: &str,
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    lane_color: &str,
    c: &KanbanCard,
    editing: bool,
    selected: bool,
    lane_width: f32,
) -> Box<dyn Widget> {
    let id = c.id.clone();
    let accent = if lane_color.is_empty() { "#8B95A6" } else { lane_color };
    let class = match (editing, selected) {
        (true, _) => "notes-kanban-card editing",
        (false, true) => "notes-kanban-card selected",
        (false, false) => "notes-kanban-card",
    };
    let mut shell = DecoratedBox::new().class(class);
    if !doc.style.card_bg.is_empty() {
        shell = shell.style("background-color", Color::from_hex(&doc.style.card_bg));
    }
    let accent_bar = DecoratedBox::new()
        .class("notes-kanban-card-accent")
        .style("background-color", Color::from_hex(accent));

    if editing {
        let (editor, source) = handle.card_editor(&id);
        let h_blur = handle.clone();
        let id_blur = id.clone();
        let body = Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DocumentEditor::new()
                    .markdown((*source).clone())
                    .handle(&editor)
                    .autofocus(true)
                    .plain(true)
                    .heading_placeholder(tr!("notes.kanban.title_hint"))
                    .placeholder(tr!("notes.kanban.body_hint"))
                    .on_focus_lost(move || h_blur.finish_editing(&id_blur))
                    .class("notes-kanban-card-editor"),
            )
            .child(card_fields(env, handle, c, lane_width));
        return Box::new(shell.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(accent_bar)
                .child(DecoratedBox::new().class("grow").child(body)),
        ));
    }

    let has_title = !c.title.trim().is_empty();
    let preview = preview_text(&c.md);
    let has_body = !preview.is_empty();
    let title = if has_title { c.title.clone() } else { tr!("notes.kanban.untitled") };
    let mut body = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if c.priority.is_some() || !c.tags.is_empty() {
        body = body.child(chips_row(c, lane_width));
    }
    if has_title || !has_body {
        body = body.child(Text::new(title.clone()).max_lines(3).class("notes-kanban-card-title"));
    }
    if has_body {
        body = body.child(Text::new(preview).max_lines(5).class("notes-kanban-card-preview"));
    }
    if !c.files.is_empty() {
        body = body.child(files_row(env, c, lane_width));
    }
    if c.due.is_some() || c.done.is_some() || c.repeat != Repeat::None || c.checklist().is_some() {
        body = body.child(footer_row(c));
    }
    if selected {
        body = body.child(card_fields(env, handle, c, lane_width));
    }
    let content = shell.child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(accent_bar)
            .child(DecoratedBox::new().class("grow").child(body)),
    );
    let h_click = handle.clone();
    let id_click = id.clone();
    let label = if has_title { title } else { c.md.lines().next().unwrap_or_default().to_string() };
    Box::new(
        Draggable::new(DRAG_TYPE_CARD, format!("{board}|{id}"))
            .label(label)
            .on_click(move || h_click.start_editing(&id_click))
            .on_drag_start(|bounds| CARD_DRAG_H.store(bounds.size.height.to_bits(), Ordering::Relaxed))
            .child(content),
    )
}

/// Вложения карточки: картинки — миниатюрами (клик — окно просмотра),
/// файлы — чипами со скрепкой (клик — системное приложение).
fn files_row(env: &BoardEnv, c: &KanbanCard, lane_width: f32) -> impl Widget {
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Start);
    let images: Vec<&CardFile> = c.files.iter().filter(|f| f.is_image()).collect();
    let files: Vec<&CardFile> = c.files.iter().filter(|f| !f.is_image()).collect();
    if !images.is_empty() {
        // Сколько миниатюр влезает в ряд; остальные — «+n».
        let per_row = ((lane_width - 56.0) / (THUMB + 4.0)).floor().max(1.0) as usize;
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for (i, f) in images.iter().enumerate() {
            if i >= per_row {
                row = row.child(Text::new(format!("+{}", images.len() - per_row)).class("notes-kanban-chip-more"));
                break;
            }
            row = row.child(thumb(env, f));
        }
        col = col.child(row);
    }
    for f in files {
        let open = env.open_file.clone();
        let file = (*f).clone();
        col = col.child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || open(&file))
                .child(
                    DecoratedBox::new().class("notes-kanban-file").child(
                        Row::new()
                            .gap(4.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(MI_ATTACH_FILE).class("notes-kanban-file-icon"))
                            .child(Text::new(f.label()).max_lines(1).class("notes-kanban-file-text")),
                    ),
                ),
        );
    }
    col
}

/// Сторона миниатюры вложения.
const THUMB: f32 = 64.0;

fn thumb(env: &BoardEnv, f: &CardFile) -> impl Widget {
    let open = env.open_file.clone();
    let file = f.clone();
    let inner: Box<dyn Widget> = match (env.asset_path)(&f.url) {
        Some(path) => Box::new(Image::new(path.display().to_string()).fit(ImageFit::Cover).class("notes-kanban-thumb-img")),
        None => Box::new(Center::new().child(Icon::new(MI_ATTACH_FILE).class("notes-kanban-file-icon"))),
    };
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || open(&file))
        .child(DecoratedBox::new().clip(true).class("notes-kanban-thumb").child(Stack::new().clip(true).children(vec![inner])))
}

/// Бейдж приоритета и метки (сколько влезает по ширине, остальное — «+n»).
fn chips_row(c: &KanbanCard, lane_width: f32) -> impl Widget {
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let mut used = 0.0;
    let avail = (lane_width - 60.0).max(80.0);
    if let Some(p) = c.priority {
        let label = tr!(&p.i18n_key());
        used += chip_width(&label);
        row = row.child(chip(&label, p.color(), "notes-kanban-chip priority"));
    }
    let mut hidden = 0;
    for tag in &c.tags {
        let w = chip_width(tag);
        if used + w > avail && used > 0.0 {
            hidden += 1;
            continue;
        }
        used += w;
        row = row.child(chip(tag, tag_color(tag), "notes-kanban-chip"));
    }
    if hidden > 0 {
        row = row.child(Text::new(format!("+{hidden}")).class("notes-kanban-chip-more"));
    }
    row
}

/// Прикидка ширины чипа: 6.5px на символ + отступы (`padding: 2px 8px`
/// в `.notes-kanban-chip`).
fn chip_width(text: &str) -> f32 {
    text.chars().count() as f32 * 6.5 + 20.0
}

fn chip(text: &str, color: &str, class: &str) -> impl Widget {
    let (bg, fg) = super::model::chip_colors(color);
    DecoratedBox::new()
        .class(class)
        .style("background-color", bg)
        .child(Text::new(text.to_string()).max_lines(1).style("color", fg).class("notes-kanban-chip-text"))
}

/// Подвал: срок (просроченный — красным; у закрытой карточки — дата
/// закрытия с галочкой), значок повтора и прогресс чек-листа.
fn footer_row(c: &KanbanCard) -> impl Widget {
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(days) = c.done.as_deref().and_then(parse_days) {
        row = row.child(
            Row::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("notes-kanban-due done")
                .child(Icon::new(MI_CHECK).class("notes-kanban-due-icon"))
                .child(Text::new(short_date(days)).class("notes-kanban-due-text")),
        );
    } else if let Some(days) = c.due.as_deref().and_then(parse_days) {
        let overdue = days < today_days();
        let class = if overdue { "notes-kanban-due overdue" } else { "notes-kanban-due" };
        row = row.child(
            Row::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class(class)
                .child(Icon::new(MI_TODAY).class("notes-kanban-due-icon"))
                .child(Text::new(short_date(days)).class("notes-kanban-due-text")),
        );
    }
    if c.repeat != Repeat::None {
        row = row.child(Icon::new(MI_AUTORENEW).class("notes-kanban-repeat-icon"));
    }
    row = row.child(DecoratedBox::new().class("grow"));
    if let Some((done, total)) = c.checklist() {
        row = row.child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(format!("{done}/{total}")).class("notes-kanban-progress-text"))
                .child(
                    DecoratedBox::new()
                        .class("notes-kanban-progress")
                        .child(ProgressBar::new().value(done as f32 / total as f32).class("notes-kanban-progress-bar")),
                ),
        );
    }
    row
}

// ─── Поля карточки (в карточке и в панели свойств) ────────────────────────

/// Выпадающий список приоритета.
pub fn priority_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    let mut items = vec![DropdownItem::new("", tr!("notes.kanban.priority.none"))];
    for p in Priority::ALL {
        items.push(DropdownItem::new(p.key(), tr!(&p.i18n_key())));
    }
    let h = handle.clone();
    let id = card.id.clone();
    Dropdown::with_items(items)
        .selected(card.priority.map(|p| p.key()).unwrap_or(""))
        .width(width)
        .on_change(move |v| h.set_priority(&id, Priority::parse(v)))
        .class("notes-kanban-field")
}

/// Поле срока с календарём.
pub fn due_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    let h = handle.clone();
    let id = card.id.clone();
    let mut picker = DatePicker::new()
        .placeholder(tr!("notes.kanban.due"))
        .width(width)
        .on_change(move |d: Option<Date>| {
            h.set_due(&id, d.map(|d| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)));
        });
    if let Some(days) = card.due.as_deref().and_then(parse_days) {
        let (y, m, d) = civil_from_days(days);
        picker = picker.selected(Date::new(y as i32, m, d));
    }
    picker.class("notes-kanban-field")
}

/// Выпадающий список повтора.
pub fn repeat_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    let items: Vec<DropdownItem> = Repeat::ALL
        .iter()
        .map(|r| DropdownItem::new(r.key(), syngui::i18n::tr(&format!("notes.calendar.repeat.{}", r.key()))))
        .collect();
    let h = handle.clone();
    let id = card.id.clone();
    Dropdown::with_items(items)
        .selected(card.repeat.key())
        .width(width)
        .on_change(move |v| h.set_repeat(&id, Repeat::parse(v).unwrap_or_default()))
        .class("notes-kanban-field")
}

/// Оценка длительности: число и единица (часы/дни). Ноль — оценки нет;
/// смена единицы переносит само число (2 часа → 2 дня).
pub fn duration_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    let min = card.duration.unwrap_or(0);
    let in_days = min > 0 && min % DAY_MIN == 0;
    let unit = if in_days { DAY_MIN } else { 60 };
    let value = if min == 0 { 0.0 } else { (min as f64 / unit as f64 * 10.0).round() / 10.0 };
    let h = handle.clone();
    let id = card.id.clone();
    let spin = SpinBox::new()
        .range(0.0, 999.0)
        .step(1.0)
        .width((width - 84.0).max(56.0))
        .value(value)
        .on_change(move |v: f64| h.set_duration(&id, (v > 0.0).then(|| (v * unit as f64).round().max(1.0) as u32)))
        .class("notes-kanban-field");
    let h = handle.clone();
    let id = card.id.clone();
    let units = Dropdown::with_items(vec![
        DropdownItem::new("h", tr!("notes.kanban.duration.hours")),
        DropdownItem::new("d", tr!("notes.kanban.duration.days")),
    ])
    .selected(if in_days { "d" } else { "h" })
    .width(80.0)
    .on_change(move |v| {
        let unit = if v == "d" { DAY_MIN } else { 60 };
        h.set_duration(&id, (value > 0.0).then(|| (value * unit as f64).round().max(1.0) as u32));
    })
    .class("notes-kanban-field");
    Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(spin).child(units)
}

/// Дата плановой полосы: `start` либо `end`. Время момента (его ставит
/// агент) сохраняется — календарь показывает задачу в своих часах.
fn moment_control(handle: &KanbanHandle, card: &KanbanCard, width: f32, is_start: bool) -> impl Widget {
    let (own, other) = if is_start { (card.start_at(), card.end.clone()) } else { (card.end_at(), card.start.clone()) };
    let min = own.and_then(|m| m.min);
    let h = handle.clone();
    let id = card.id.clone();
    let mut picker = DatePicker::new()
        .placeholder(if is_start { tr!("notes.kanban.start") } else { tr!("notes.kanban.end") })
        .width(width)
        .on_change(move |d: Option<Date>| {
            let own = d.map(|d| Moment { day: days_from_civil(d.year as i64, d.month, d.day), min }.iso());
            let (start, end) = if is_start { (own, other.clone()) } else { (other.clone(), own) };
            h.set_schedule(&id, start, end);
        });
    if let Some(m) = own {
        let (y, mo, d) = civil_from_days(m.day);
        picker = picker.selected(Date::new(y as i32, mo, d));
    }
    picker.class("notes-kanban-field")
}

pub fn start_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    moment_control(handle, card, width, true)
}

pub fn end_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    moment_control(handle, card, width, false)
}

/// «В календарь»: меню дней (сегодня, завтра, послезавтра, по сроку) —
/// начало встаёт на выбранный день, конец считается по оценке. Последний
/// пункт снимает план.
pub fn schedule_button(handle: &KanbanHandle, card: &KanbanCard) -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let planned = card.schedule().is_some();
    let due = card.due.as_deref().and_then(parse_days);
    let mut items = vec![
        MenuItem::new("today", tr!("notes.kanban.schedule.today")).icon(MI_TODAY),
        MenuItem::new("tomorrow", tr!("notes.kanban.schedule.tomorrow")).icon(MI_EVENT),
        MenuItem::new("after", tr!("notes.kanban.schedule.after_tomorrow")).icon(MI_EVENT),
    ];
    if due.is_some() {
        items.push(MenuItem::new("due", tr!("notes.kanban.schedule.on_due")).icon(MI_CALENDAR_MONTH));
    }
    if planned {
        items.push(MenuItem::new("clear", tr!("notes.kanban.schedule.clear")).icon(MI_CLOSE));
    }
    let btn = ToolButton::new(MI_CALENDAR_ADD)
        .tooltip(tr!("notes.kanban.schedule"))
        .on_click_with_bounds(move |_, bounds| {
            pos.set(Point::new(bounds.origin.x, bounds.origin.y + bounds.size.height + 4.0));
            open.set(true);
        })
        .class(if planned { "notes-kanban-lane-btn selected" } else { "notes-kanban-lane-btn" });
    let h = handle.clone();
    let id = card.id.clone();
    let menu = PopupMenu::new().items(items).is_open(open).position(pos).on_select(move |action| {
        let today = today_days();
        match action {
            "clear" => h.unschedule_card(&id),
            "due" => {
                if let Some(d) = due {
                    h.schedule_card(&id, d, None);
                }
            }
            "tomorrow" => h.schedule_card(&id, today + 1, None),
            "after" => h.schedule_card(&id, today + 2, None),
            _ => h.schedule_card(&id, today, None),
        }
    });
    Stack::new().clip(false).child(btn).child(menu)
}

/// «План 09.09 → 11.09» либо «План 09.09 10:00–12:00» — плановая полоса
/// карточки на языке дат; `None` — карточка не в календаре.
pub fn schedule_text(card: &KanbanCard) -> Option<String> {
    let span = card.schedule()?;
    let body = match span.time {
        Some((a, b)) => format!("{} {}–{}", short_date(span.start_day), fmt_hm(a), fmt_hm(b)),
        None if span.start_day == span.end_day => short_date(span.start_day),
        None => format!("{} → {}", short_date(span.start_day), short_date(span.end_day)),
    };
    Some(format!("{} {body}", tr!("notes.kanban.planned")))
}

/// «Создана ДД.ММ · Сделана ДД.ММ» — штампы карточки; `None`, если их нет.
pub fn dates_text(card: &KanbanCard) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(d) = card.created.as_deref().and_then(parse_days) {
        parts.push(format!("{} {}", tr!("notes.kanban.created"), short_date(d)));
    }
    if let Some(d) = card.done.as_deref().and_then(parse_days) {
        parts.push(format!("{} {}", tr!("notes.kanban.done_at"), short_date(d)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Метки через запятую.
pub fn tags_control(handle: &KanbanHandle, card: &KanbanCard, width: f32) -> impl Widget {
    let h = handle.clone();
    let id = card.id.clone();
    TextField::with_text(card.tags.join(", "))
        .placeholder(tr!("notes.kanban.tags.hint"))
        .width(width)
        .submit_on_focus_lost(true)
        .on_submit(move |v: &str| h.set_tags(&id, parse_tags(v)))
        .class("notes-kanban-field")
}

/// Ширина полей внутри карточки: колонка минус отступы колонки и карточки,
/// полоса цвета и зазор.
fn field_width(lane_width: f32) -> f32 {
    (lane_width - 56.0).max(120.0)
}

/// Поля под карточкой, столбиком (в колонку шириной 300 в ряд они не
/// влезают): приоритет, срок, повтор + «в архив», метки + «удалить»,
/// штампы создания/закрытия.
fn card_fields(env: &BoardEnv, handle: &KanbanHandle, card: &KanbanCard, lane_width: f32) -> impl Widget {
    let h_del = handle.clone();
    let id_del = card.id.clone();
    let h_arc = handle.clone();
    let id_arc = card.id.clone();
    let w = field_width(lane_width);
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .class("notes-kanban-card-fields")
        .child(priority_control(handle, card, w))
        .child(due_control(handle, card, w))
        .child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(repeat_control(handle, card, w - 60.0))
                .child(attach_button(env, handle, card))
                .child(
                    ToolButton::new(MI_ARCHIVE)
                        .tooltip(tr!("notes.kanban.archive_card"))
                        .on_click(move || h_arc.archive_card(&id_arc))
                        .class("notes-kanban-lane-btn"),
                ),
        )
        .child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(duration_control(handle, card, w - 30.0))
                .child(schedule_button(handle, card)),
        )
        .child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(tags_control(handle, card, w - 30.0))
                .child(
                    ToolButton::new(MI_DELETE)
                        .tooltip(tr!("notes.kanban.delete_card"))
                        .on_click(move || h_del.delete_card(&id_del))
                        .class("notes-kanban-lane-btn"),
                ),
        );
    for f in &card.files {
        col = col.child(file_row(handle, card, f, w));
    }
    if let Some(text) = schedule_text(card) {
        col = col.child(Text::new(text).max_lines(1).class("notes-kanban-dates"));
    }
    if let Some(text) = dates_text(card) {
        col = col.child(Text::new(text).max_lines(2).class("notes-kanban-dates"));
    }
    col
}

/// Скрепка: диалог выбора файла → вложение бандла → карточка.
pub fn attach_button(env: &BoardEnv, handle: &KanbanHandle, card: &KanbanCard) -> impl Widget {
    let h = handle.clone();
    let id = card.id.clone();
    let pick = env.pick_file.clone();
    ToolButton::new(MI_ATTACH_FILE)
        .tooltip(tr!("notes.kanban.attach_file"))
        .on_click(move || {
            if let Some(file) = pick() {
                h.add_file(&id, file);
            }
        })
        .class("notes-kanban-lane-btn")
}

/// Строка вложения в полях выбранной карточки: имя + «убрать».
pub fn file_row(handle: &KanbanHandle, card: &KanbanCard, f: &CardFile, width: f32) -> impl Widget {
    let h = handle.clone();
    let id = card.id.clone();
    let url = f.url.clone();
    Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(if f.is_image() { MI_IMAGE_ICON } else { MI_ATTACH_FILE }).class("notes-kanban-file-icon"))
        .child(DecoratedBox::new().class("grow").child(Text::new(f.label()).max_lines(1).class("notes-kanban-file-text")))
        .child(
            ToolButton::new(MI_CLOSE)
                .tooltip(tr!("notes.kanban.remove_file"))
                .on_click(move || h.remove_file(&id, &url))
                .class("notes-kanban-lane-btn"),
        )
        .style("width", StyleValue::px(width))
}

/// ISO-дата по дням от эпохи (для тестов панели).
#[allow(dead_code)]
pub fn iso_from_ymd(y: i64, m: u32, d: u32) -> String {
    super::super::gantt::calendar::days_to_iso(days_from_civil(y, m, d))
}
