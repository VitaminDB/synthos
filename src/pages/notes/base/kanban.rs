//! Канбан-представление базы: колонки по опциям select-колонки
//! `group_by`, карточки перетаскиваются между колонками (Draggable /
//! DropArea; ghost — живой снимок карточки от фреймворка).
//!
//! Перестройка подписана на `revision`: канбан не держит текстовых
//! инпутов, поэтому дешёвый полный ре-рендер на каждую правку безопасен
//! и держит представление синхронным с таблицей.

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::overlay::{Draggable, DropArea};

use super::model::{BaseDoc, BaseRow, CellValue, ColumnKind, SelectOption};
use super::BaseHandle;

const DRAG_TYPE_CARD: &str = "notes-base-card";

pub fn view(handle: BaseHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.revision.get();
        let _ = handle.structure_rev.get();
        vec![build(handle.clone())]
    })
}

fn build(handle: BaseHandle) -> Box<dyn Widget> {
    let doc = handle.lock().clone();
    let view_idx = handle.view_sel.get_untracked().min(doc.views.len().saturating_sub(1));
    let Some(view) = doc.views.get(view_idx) else {
        return Box::new(Text::new(""));
    };
    // Колонка группировки: заданная в представлении либо первый select.
    let group_col = view
        .group_by
        .clone()
        .and_then(|id| doc.column(&id).cloned())
        .or_else(|| doc.columns.iter().find(|c| c.kind == ColumnKind::Select).cloned());
    let Some(group_col) = group_col else {
        return Box::new(
            Center::new()
                .child(Text::new(tr!("notes.base.kanban.no_select")).class("notes-empty-hint")),
        );
    };
    let label_col = view
        .label_col
        .clone()
        .or_else(|| doc.columns.first().map(|c| c.id.clone()))
        .unwrap_or_default();

    let mut lanes = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban");

    // «Без статуса» + по колонке на каждую опцию.
    let mut groups: Vec<(Option<SelectOption>, Vec<&BaseRow>)> = Vec::new();
    groups.push((None, Vec::new()));
    for opt in &group_col.options {
        groups.push((Some(opt.clone()), Vec::new()));
    }
    for row in &doc.rows {
        let value = doc.cell_text(row, &group_col.id);
        let idx = group_col
            .options
            .iter()
            .position(|o| o.id == value || o.name == value)
            .map(|i| i + 1)
            .unwrap_or(0);
        groups[idx].1.push(row);
    }

    for (opt, rows) in groups {
        // Пустую «Без статуса» не показываем, чтобы не занимать место.
        if opt.is_none() && rows.is_empty() {
            continue;
        }
        lanes = lanes.child(lane(&handle, &doc, &group_col.id, &label_col, opt, rows));
    }

    Box::new(
        ScrollView::new()
            .horizontal()
            .class("notes-kanban-scroll")
            .child(lanes),
    )
}

fn lane(
    handle: &BaseHandle,
    doc: &BaseDoc,
    group_col_id: &str,
    label_col: &str,
    opt: Option<SelectOption>,
    rows: Vec<&BaseRow>,
) -> impl Widget {
    let (name, color, opt_id) = match &opt {
        Some(o) => (o.name.clone(), o.color.clone(), Some(o.id.clone())),
        None => (tr!("notes.base.kanban.none"), String::new(), None),
    };

    let mut header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-kanban-lane-header");
    if !color.is_empty() {
        header = header.child(
            DecoratedBox::new()
                .style("background-color", Color::from_hex(&color))
                .class("notes-kanban-dot"),
        );
    }
    header = header
        .child(Text::new(name).class("notes-kanban-lane-title"))
        .child(Text::new(format!("{}", rows.len())).class("notes-kanban-lane-count"));

    let mut cards = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-kanban-cards");
    for row in rows {
        cards = cards.child(Stack::new().children(vec![card(handle, doc, label_col, row)]));
    }

    let h = handle.clone();
    let group_col_id = group_col_id.to_string();
    let lane_body = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header)
        .child(
            DecoratedBox::new().class("grow").child(
                ScrollView::new().vertical().child(cards),
            ),
        );

    DecoratedBox::new().class("notes-kanban-lane").child(
        DropArea::new()
            .accept_types(vec![DRAG_TYPE_CARD.to_string()])
            .on_drop(move |data| {
                // Перенос: строка получает статус колонки и уезжает в конец.
                let row_id = data.payload.clone();
                let value = opt_id.clone();
                h.edit_structural(|doc| {
                    if let Some(pos) = doc.rows.iter().position(|r| r.id == row_id) {
                        let mut row = doc.rows.remove(pos);
                        match &value {
                            Some(v) => {
                                row.cells
                                    .insert(group_col_id.clone(), CellValue::Text(v.clone()));
                            }
                            None => {
                                row.cells.remove(&group_col_id);
                            }
                        }
                        doc.rows.push(row);
                    }
                });
            })
            .child(lane_body),
    )
}

fn card(handle: &BaseHandle, doc: &BaseDoc, label_col: &str, row: &BaseRow) -> Box<dyn Widget> {
    let title = {
        let t = doc.cell_text(row, label_col);
        if t.is_empty() { tr!("notes.base.kanban.untitled") } else { t }
    };
    let mut col = Column::new()
        .gap(3.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title.clone()).max_lines(2).class("notes-kanban-card-title"));
    // Прочие непустые ячейки — мелкими строками.
    for c in &doc.columns {
        if c.id == label_col || c.kind == ColumnKind::Select {
            continue;
        }
        let text = doc.cell_text(row, &c.id);
        if text.is_empty() || text == "false" {
            continue;
        }
        let shown = if c.kind == ColumnKind::Checkbox {
            format!("✓ {}", c.name)
        } else {
            text
        };
        col = col.child(Text::new(shown).max_lines(1).class("notes-kanban-card-meta"));
    }
    let body = DecoratedBox::new().class("notes-kanban-card").child(col);
    let _ = handle;
    Box::new(
        Draggable::new(DRAG_TYPE_CARD, row.id.clone())
            .label(title)
            .child(body),
    )
}
