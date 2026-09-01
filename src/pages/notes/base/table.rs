//! Табличное представление базы: syngui `TableView` с инлайн-редакторами
//! в ячейках (TextField/Dropdown/DatePicker/Checkbox).
//!
//! Перестройка подписана только на `structure_rev` — правка ячейки пишет
//! в модель без пересоздания таблицы, фокус ввода не теряется.

use syngui::prelude::*;
use syngui::widgets::GestureDetector;
use syngui::widgets::{Date, DatePicker, DropdownItem, TableColumn, TableView};

use crate::icons::*;

use super::model::{CellValue, ColumnKind, ViewKind};
use super::BaseHandle;

pub fn view(handle: BaseHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        vec![build_table(handle.clone())]
    })
}

fn build_table(handle: BaseHandle) -> Box<dyn Widget> {
    let doc = handle.lock().clone();
    let view_idx = handle.view_sel.get_untracked().min(doc.views.len().saturating_sub(1));
    let view = doc.views.get(view_idx).cloned();
    let view_id = view.as_ref().map(|v| v.id.clone()).unwrap_or_default();
    let is_table = view.as_ref().map(|v| v.kind == ViewKind::Table).unwrap_or(true);
    let widths = view.map(|v| v.widths).unwrap_or_default();
    let _ = is_table;

    let row_ids: Vec<String> = doc.rows.iter().map(|r| r.id.clone()).collect();
    let rows_data: Vec<Vec<String>> = doc
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<String> = doc
                .columns
                .iter()
                .map(|col| doc.cell_text(row, &col.id))
                .collect();
            cells.push(String::new()); // Колонка действий.
            cells
        })
        .collect();

    let mut columns: Vec<TableColumn> = Vec::new();
    for (ci, col) in doc.columns.iter().enumerate() {
        let width = widths.get(&col.id).copied().unwrap_or(default_width(col.kind));
        let h = handle.clone();
        let col_id = col.id.clone();
        let kind = col.kind;
        let options = col.options.clone();
        let ids = row_ids.clone();
        let doc_snapshot = doc.clone();
        let tc = TableColumn::fixed(col.name.clone(), width)
            .resizable(true)
            .sortable(false)
            .cell_renderer_with_row(move |row_idx, _row| {
                let Some(row_id) = ids.get(row_idx).cloned() else {
                    return Box::new(Text::new(""));
                };
                let current = doc_snapshot
                    .rows
                    .iter()
                    .find(|r| r.id == row_id)
                    .map(|r| doc_snapshot.cell_text(r, &col_id))
                    .unwrap_or_default();
                cell_editor(h.clone(), row_id, col_id.clone(), kind, &options, current)
            });
        let _ = ci;
        columns.push(tc);
    }
    // Колонка удаления строки.
    {
        let h = handle.clone();
        let ids = row_ids.clone();
        columns.push(
            TableColumn::fixed("", 40.0)
                .resizable(false)
                .sortable(false)
                .cell_renderer_with_row(move |row_idx, _row| {
                    let Some(row_id) = ids.get(row_idx).cloned() else {
                        return Box::new(Text::new(""));
                    };
                    let h = h.clone();
                    Box::new(
                        ToolButton::new(MI_CLOSE)
                            .tooltip(tr!("app.delete"))
                            .on_click(move || h.delete_row(&row_id))
                            .class("notes-base-row-delete"),
                    )
                }),
        );
    }

    let col_count = doc.columns.len();
    let h_resize = handle.clone();
    let col_ids: Vec<String> = doc.columns.iter().map(|c| c.id.clone()).collect();
    let table = TableView::new(columns, rows_data)
        .sortable(false)
        .striped(true)
        .row_height(34.0)
        .on_column_resize(move |idx, w| {
            if idx < col_count {
                h_resize.set_column_width(&view_id, &col_ids[idx], w);
            }
        })
        .class("notes-base-table");

    let h_add = handle.clone();
    let add_row = GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Pointer)
        .on_click(move || {
            h_add.add_row();
        })
        .child(
            DecoratedBox::new().class("notes-base-add-row").child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(MI_ADD).class("notes-insert-icon"))
                    .child(Text::new(tr!("notes.base.add_row")).class("notes-insert-label")),
            ),
        );

    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow").child(table))
            .child(add_row),
    )
}

fn default_width(kind: ColumnKind) -> f32 {
    match kind {
        ColumnKind::Text => 220.0,
        ColumnKind::Number => 100.0,
        ColumnKind::Date => 150.0,
        ColumnKind::Select => 140.0,
        ColumnKind::MultiSelect => 180.0,
        ColumnKind::Checkbox => 80.0,
    }
}

fn cell_editor(
    handle: BaseHandle,
    row_id: String,
    col_id: String,
    kind: ColumnKind,
    options: &[super::model::SelectOption],
    current: String,
) -> Box<dyn Widget> {
    match kind {
        ColumnKind::Text => Box::new(
            TextField::new()
                .text(current)
                .on_change(move |v| {
                    handle.set_cell(&row_id, &col_id, Some(CellValue::Text(v.to_string())));
                })
                .class("notes-base-cell-input"),
        ),
        ColumnKind::Number => Box::new(
            TextField::new()
                .text(current)
                .on_change(move |v| {
                    let value = v.trim().replace(',', ".").parse::<f64>().ok();
                    handle.set_cell(&row_id, &col_id, value.map(CellValue::Number));
                })
                .class("notes-base-cell-input"),
        ),
        ColumnKind::Date => {
            let mut picker = DatePicker::new().placeholder("—").on_change({
                let handle = handle.clone();
                move |d: Option<Date>| {
                    let value = d.map(|d| {
                        CellValue::Text(format!("{:04}-{:02}-{:02}", d.year, d.month, d.day))
                    });
                    handle.set_cell(&row_id, &col_id, value);
                }
            });
            if let Some(date) = parse_iso_date(&current) {
                picker = picker.selected(date);
            }
            Box::new(picker.class("notes-base-cell-date"))
        }
        ColumnKind::Select => {
            let items: Vec<DropdownItem> = options
                .iter()
                .map(|o| DropdownItem::new(o.id.clone(), o.name.clone()))
                .collect();
            // В ячейке хранится id опции.
            let selected = options
                .iter()
                .find(|o| o.id == current || o.name == current)
                .map(|o| o.id.clone())
                .unwrap_or_default();
            Box::new(
                syngui::widgets::Dropdown::new()
                    .items(items)
                    .selected(selected)
                    .on_change(move |v| {
                        handle.set_cell(&row_id, &col_id, Some(CellValue::Text(v.to_string())));
                    })
                    .class("notes-base-cell-select"),
            )
        }
        ColumnKind::MultiSelect => Box::new(
            TextField::new()
                .text(current)
                .on_change(move |v| {
                    let list: Vec<String> = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let value = (!list.is_empty()).then_some(CellValue::List(list));
                    handle.set_cell(&row_id, &col_id, value);
                })
                .class("notes-base-cell-input"),
        ),
        ColumnKind::Checkbox => {
            let checked = current == "true";
            Box::new(
                Center::new().child(Checkbox::checked(checked).on_change(move |v| {
                    handle.set_cell(&row_id, &col_id, Some(CellValue::Bool(v)));
                })),
            )
        }
    }
}

/// `YYYY-MM-DD` → Date.
fn parse_iso_date(s: &str) -> Option<Date> {
    let mut parts = s.trim().splitn(3, '-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&day).then_some(())?;
    Some(Date { year, month, day })
}
