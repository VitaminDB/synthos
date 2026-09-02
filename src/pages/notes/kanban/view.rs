//! Виджет канбан-доски: колонки в горизонтальной прокрутке, карточки
//! перетаскиваются между колонками и внутри них (Draggable / DropArea;
//! ghost — снимок карточки от фреймворка).
//!
//! Цель дропа — сама карточка («перед ней») либо хвост колонки («в
//! конец»): вложенных DropArea нет, иначе дроп получали бы обе. Заголовок
//! карточки правится по клику (TextField на месте карточки), название
//! колонки — прямо в шапке, цветная метка переключается кликом.

use syngui::core::Color;
use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::overlay::{Draggable, DropArea};
use syngui::widgets::GestureDetector;

use crate::icons::*;

use super::model::{KanbanCard, KanbanColumn, KanbanDoc};
use super::KanbanHandle;

const DRAG_TYPE_CARD: &str = "notes-kanban-card";

pub fn view(handle: KanbanHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.revision.get();
        let editing = handle.editing.get();
        vec![build(handle.clone(), editing)]
    })
}

fn build(handle: KanbanHandle, editing: Option<String>) -> Box<dyn Widget> {
    let doc = handle.lock().clone();
    let mut lanes = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban");
    for column in &doc.columns {
        lanes = lanes.child(lane(&handle, &doc, column, editing.as_deref()));
    }
    lanes = lanes.child(add_column_lane(&handle));
    Box::new(ScrollView::new().horizontal().class("notes-kanban-scroll").child(lanes))
}

fn lane(
    handle: &KanbanHandle,
    doc: &KanbanDoc,
    column: &KanbanColumn,
    editing: Option<&str>,
) -> impl Widget {
    let cards_in = doc.cards_of(&column.id);
    let col_id = column.id.clone();

    // Шапка: метка цвета (клик — следующий цвет), название, счётчик,
    // «+ карточка», удалить колонку.
    let h_color = handle.clone();
    let id_color = col_id.clone();
    let mut dot = DecoratedBox::new().class("notes-kanban-dot");
    if column.color.is_empty() {
        dot = dot.class("notes-kanban-dot empty");
    } else {
        dot = dot.style("background-color", Color::from_hex(&column.color));
    }
    let h_name = handle.clone();
    let id_name = col_id.clone();
    let h_add = handle.clone();
    let id_add = col_id.clone();
    let h_del = handle.clone();
    let id_del = col_id.clone();
    let header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-kanban-lane-header")
        .child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || h_color.cycle_column_color(&id_color))
                .child(dot),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                TextField::new()
                    .text(column.name.clone())
                    .submit_on_focus_lost(true)
                    .on_submit(move |v: &str| h_name.rename_column(&id_name, v))
                    .class("notes-kanban-lane-name"),
            ),
        )
        .child(Text::new(format!("{}", cards_in.len())).class("notes-kanban-lane-count"))
        .child(
            ToolButton::new(MI_ADD)
                .tooltip(tr!("notes.kanban.add_card"))
                .on_click(move || {
                    h_add.add_card(&id_add);
                })
                .class("notes-kanban-lane-btn"),
        )
        .child(
            ToolButton::new(MI_CLOSE)
                .tooltip(tr!("notes.kanban.delete_column"))
                .on_click(move || h_del.delete_column(&id_del))
                .class("notes-kanban-lane-btn"),
        );

    let mut cards = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban-cards");
    for c in cards_in {
        cards = cards.child(Stack::new().children(vec![card(handle, &col_id, c, editing == Some(c.id.as_str()))]));
    }

    // Хвост колонки — цель дропа «в конец» и кнопка новой карточки.
    let h_tail = handle.clone();
    let id_tail = col_id.clone();
    let h_tail_add = handle.clone();
    let id_tail_add = col_id.clone();
    let tail = DropArea::new()
        .accept_types(vec![DRAG_TYPE_CARD.to_string()])
        .on_drop(move |data| h_tail.move_card(&data.payload, &id_tail, None))
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
        );

    DecoratedBox::new().class("notes-kanban-lane").child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(cards)))
            .child(tail),
    )
}

/// Карточка: перетаскивается, по клику — правка заголовка; сама — цель
/// дропа «перед ней».
fn card(handle: &KanbanHandle, column: &str, c: &KanbanCard, editing: bool) -> Box<dyn Widget> {
    let id = c.id.clone();
    if editing {
        let h_submit = handle.clone();
        let id_submit = id.clone();
        let h_esc = handle.clone();
        let id_esc = id.clone();
        let h_del = handle.clone();
        let id_del = id.clone();
        return Box::new(
            DecoratedBox::new().class("notes-kanban-card editing").child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        DecoratedBox::new().class("grow").child(
                            TextField::new()
                                .text(c.title.clone())
                                .placeholder(tr!("notes.kanban.card_placeholder"))
                                .autofocus(true)
                                .submit_on_focus_lost(true)
                                .on_submit(move |v: &str| h_submit.finish_editing(&id_submit, v))
                                .on_escape(move || {
                                    let title = h_esc
                                        .lock()
                                        .cards
                                        .iter()
                                        .find(|c| c.id == id_esc)
                                        .map(|c| c.title.clone())
                                        .unwrap_or_default();
                                    h_esc.finish_editing(&id_esc, &title);
                                })
                                .class("notes-kanban-card-input"),
                        ),
                    )
                    .child(
                        ToolButton::new(MI_DELETE)
                            .tooltip(tr!("notes.kanban.delete_card"))
                            .on_click(move || h_del.delete_card(&id_del))
                            .class("notes-kanban-lane-btn"),
                    ),
            ),
        );
    }
    let title = if c.title.is_empty() { tr!("notes.kanban.untitled") } else { c.title.clone() };
    let body = DecoratedBox::new()
        .class("notes-kanban-card")
        .child(Text::new(title.clone()).max_lines(3).class("notes-kanban-card-title"));
    let h_click = handle.clone();
    let id_click = id.clone();
    let h_drop = handle.clone();
    let column = column.to_string();
    let id_drop = id.clone();
    Box::new(
        DropArea::new()
            .accept_types(vec![DRAG_TYPE_CARD.to_string()])
            .on_drop(move |data| h_drop.move_card(&data.payload, &column, Some(&id_drop)))
            .child(
                Draggable::new(DRAG_TYPE_CARD, id)
                    .label(title)
                    .on_click(move || h_click.editing.set(Some(id_click.clone())))
                    .child(body),
            ),
    )
}

/// Призрачная колонка «+ Колонка» в конце доски.
fn add_column_lane(handle: &KanbanHandle) -> impl Widget {
    let h = handle.clone();
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            h.add_column(&tr!("notes.kanban.new_column"));
        })
        .child(
            DecoratedBox::new().class("notes-kanban-lane ghost").child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(MI_ADD).class("notes-insert-icon"))
                    .child(Text::new(tr!("notes.kanban.add_column")).class("notes-insert-label")),
            ),
        )
}
