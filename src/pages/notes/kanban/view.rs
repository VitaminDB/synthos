//! Виджет канбан-доски: колонки в горизонтальной прокрутке, карточки
//! перетаскиваются между колонками, внутри них и **между досками**
//! (payload — `<доска>|<карточка>`, см. [`super::drop_card`]).
//!
//! Карточка — заголовок + markdown-содержимое с цветной полосой колонки.
//! В просмотре: `Text` заголовка, разделитель, `MarkdownView` (событий не
//! перехватывает — карточка остаётся `Draggable`). В правке — один
//! `DocumentEditor` с ручкой из [`KanbanHandle::card_editor`]: первый блок
//! `## …` — заголовок, ниже — содержимое; автофокус, а потеря фокуса
//! (клик куда угодно ещё) закрывает правку. Карточки и хвосты колонок
//! обёрнуты в [`super::sinks::RectProbe`] — по этим прямоугольникам
//! редактор страницы отдаёт сюда блоки, отпущенные после переноса за ⋮⋮.
//! Цель дропа карточек — сама карточка («перед ней») либо хвост колонки
//! («в конец»): вложенных DropArea нет, иначе дроп получали бы обе.

use syngui::core::Color;
use syngui::input::CursorIcon;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditor;
use syngui::widgets::overlay::{Draggable, DropArea};
use syngui::widgets::{GestureDetector, MarkdownView};

use crate::icons::*;

use super::super::state::NotesCtx;
use super::drag_strip::DragStrip;
use super::model::{KanbanCard, KanbanColumn, KanbanDoc};
use super::sinks::{RectProbe, Sink};
use super::{drop_card, KanbanHandle};

pub const DRAG_TYPE_CARD: &str = "notes-kanban-card";

pub fn view(ctx: NotesCtx, board: String, handle: KanbanHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let editing = handle.editing.get();
        vec![build(ctx, &board, handle.clone(), editing)]
    })
}

fn build(ctx: NotesCtx, board: &str, handle: KanbanHandle, editing: Option<String>) -> Box<dyn Widget> {
    let doc = handle.lock().clone();
    let mut lanes = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban");
    for column in &doc.columns {
        lanes = lanes.child(lane(ctx, board, &handle, &doc, column, editing.as_deref()));
    }
    Box::new(ScrollView::new().horizontal().class("notes-kanban-scroll").child(lanes))
}

fn lane(
    ctx: NotesCtx,
    board: &str,
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    column: &KanbanColumn,
    editing: Option<&str>,
) -> impl Widget {
    let cards_in = doc.cards_of(&column.id);
    let col_id = column.id.clone();
    let width = doc.column_width(column);

    // Шапка: метка цвета (клик — следующий цвет), название, счётчик,
    // «+ карточка».
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
        cards = cards.child(Stack::new().children(vec![card(
            ctx,
            board,
            handle,
            doc,
            &column.color,
            c,
            editing == Some(c.id.as_str()),
            width,
        )]));
    }
    // Клик по пустому месту колонки закрывает правку карточки.
    let h_blur = handle.clone();
    let cards_area = GestureDetector::new()
        .on_click(move || {
            if let Some(id) = h_blur.editing.get_untracked() {
                h_blur.finish_editing(&id);
            }
        })
        .child(ScrollView::new().vertical().child(cards));

    // Хвост колонки — цель дропа «в конец» (карточек и блоков страницы) и
    // кнопка новой карточки.
    let h_tail = handle.clone();
    let id_tail = col_id.clone();
    let board_tail = board.to_string();
    let h_tail_add = handle.clone();
    let id_tail_add = col_id.clone();
    let tail = RectProbe::new(
        Sink::Tail { board: board.to_string(), column: col_id.clone() },
        Box::new(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_CARD.to_string()])
                .on_drop(move |data| drop_card(ctx, &data.payload, &board_tail, &h_tail, &id_tail, None))
                .child(
                    GestureDetector::new()
                        .cursor(CursorIcon::Pointer)
                        .on_click(move || {
                            h_tail_add.add_card(&id_tail_add);
                        })
                        .child(
                            DecoratedBox::new().class("notes-kanban-tail").child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(MI_ADD).class("notes-insert-icon"))
                                    .child(Text::new(tr!("notes.kanban.add_card")).class("notes-insert-label")),
                            ),
                        ),
                ),
        ),
    );

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
            .child(DecoratedBox::new().class("grow").child(cards_area))
            .child(tail),
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

/// Карточка: заголовок и markdown-содержимое с цветной полосой колонки;
/// перетаскивается, по клику — правка на месте; сама — цель дропа «перед
/// ней» и приёмник блоков страницы.
#[allow(clippy::too_many_arguments)]
fn card(
    ctx: NotesCtx,
    board: &str,
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    lane_color: &str,
    c: &KanbanCard,
    editing: bool,
    lane_width: f32,
) -> Box<dyn Widget> {
    let id = c.id.clone();
    let accent = if lane_color.is_empty() { "#8B95A6" } else { lane_color };
    let mut shell = DecoratedBox::new().class(if editing { "notes-kanban-card editing" } else { "notes-kanban-card" });
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
        let h_del = handle.clone();
        let id_del = id.clone();
        let body = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DocumentEditor::new()
                    .markdown((*source).clone())
                    .handle(&editor)
                    .autofocus(true)
                    .on_focus_lost(move || h_blur.finish_editing(&id_blur))
                    .class("notes-kanban-card-editor"),
            )
            .child(
                Row::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("notes-kanban-card-toolbar")
                    .child(DecoratedBox::new().class("grow"))
                    .child(
                        ToolButton::new(MI_DELETE)
                            .tooltip(tr!("notes.kanban.delete_card"))
                            .on_click(move || h_del.delete_card(&id_del))
                            .class("notes-kanban-lane-btn"),
                    ),
            );
        return Box::new(shell.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(accent_bar)
                .child(DecoratedBox::new().class("grow").child(body)),
        ));
    }

    let has_title = !c.title.trim().is_empty();
    let has_body = !c.md.trim().is_empty();
    let title = if has_title { c.title.clone() } else { tr!("notes.kanban.untitled") };
    let mut body = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if has_title || !has_body {
        body = body.child(Text::new(title.clone()).max_lines(3).class("notes-kanban-card-title"));
    }
    if has_title && has_body {
        body = body.child(DecoratedBox::new().class("notes-kanban-card-divider"));
    }
    if has_body {
        body = body.child(
            MarkdownView::new(c.md.clone())
                .selectable(false)
                .max_width((lane_width - 52.0).max(80.0))
                .class("notes-kanban-card-body"),
        );
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
    let h_drop = handle.clone();
    let board_drop = board.to_string();
    let column = c.column.clone();
    let id_drop = id.clone();
    let label = if has_title { title } else { c.md.lines().next().unwrap_or_default().to_string() };
    Box::new(RectProbe::new(
        Sink::Card { board: board.to_string(), card: id.clone() },
        Box::new(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_CARD.to_string()])
                .on_drop(move |data| {
                    drop_card(ctx, &data.payload, &board_drop, &h_drop, &column, Some(&id_drop))
                })
                .child(
                    Draggable::new(DRAG_TYPE_CARD, format!("{board}|{id}"))
                        .label(label)
                        .on_click(move || h_click.start_editing(&id_click))
                        .child(content),
                ),
        ),
    ))
}
