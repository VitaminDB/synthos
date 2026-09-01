//! Базы данных заметок: одна модель — несколько представлений.
//!
//! [`BaseHandle`] — общий доступ к документу базы (Mutex) с двумя
//! сигналами: `revision` растёт на каждую правку (автосейв),
//! `structure_rev` — только на структурные изменения (добавление/удаление
//! строк, смена представления): на него подписана перестройка UI, чтобы
//! набор в ячейке не пересоздавал таблицу и не ронял фокус.

pub mod gantt;
pub mod kanban;
pub mod model;
pub mod pane;
pub mod table;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

use model::{BaseDoc, BaseRow, CellValue};

#[derive(Clone)]
pub struct BaseHandle {
    doc: Arc<Mutex<BaseDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    /// Индекс выбранного представления.
    pub view_sel: RwSignal<usize>,
}

impl BaseHandle {
    pub fn new(doc: BaseDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            view_sel: use_signal(0),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, BaseDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    /// Правка данных (ячейки, ширины) — только автосейв.
    pub fn edit(&self, f: impl FnOnce(&mut BaseDoc)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
    }

    /// Структурная правка (строки/колонки/представления) — плюс перестройка.
    pub fn edit_structural(&self, f: impl FnOnce(&mut BaseDoc)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    /// Замена документа целиком (перечитка с диска).
    pub fn replace(&self, doc: BaseDoc) {
        *self.lock() = doc;
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    // ─── Операции ───────────────────────────────────────────────────────────

    pub fn set_cell(&self, row_id: &str, col_id: &str, value: Option<CellValue>) {
        self.edit(|doc| {
            if let Some(row) = doc.rows.iter_mut().find(|r| r.id == row_id) {
                match value {
                    Some(v) => {
                        row.cells.insert(col_id.to_string(), v);
                    }
                    None => {
                        row.cells.remove(col_id);
                    }
                }
            }
        });
    }

    pub fn add_row(&self) -> String {
        let id = model::new_id("r");
        let row_id = id.clone();
        self.edit_structural(move |doc| {
            doc.rows.push(BaseRow { id: row_id, cells: Default::default() });
        });
        id
    }

    pub fn delete_row(&self, row_id: &str) {
        self.edit_structural(|doc| {
            doc.rows.retain(|r| r.id != row_id);
            for view in doc.views.iter_mut() {
                view.deps.retain(|(a, b)| a != row_id && b != row_id);
            }
        });
    }

    pub fn set_column_width(&self, view_id: &str, col_id: &str, width: f32) {
        self.edit(|doc| {
            if let Some(view) = doc.views.iter_mut().find(|v| v.id == view_id) {
                view.widths.insert(col_id.to_string(), width);
            }
        });
    }
}
