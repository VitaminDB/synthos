//! Канбан-доска — примитив страницы: живая врезка `![[kanban:<id>]]` над
//! объектом `notes/objects/<id>.kanban.json`.
//!
//! [`KanbanHandle`] держит документ (Mutex) и два сигнала: `revision`
//! растёт на каждую правку (автосейв и перестройка виджета — доска не
//! держит открытых инпутов, кроме карточки в режиме правки, поэтому полный
//! ре-рендер дёшев и безопасен), `editing` — карточка, чей заголовок
//! сейчас набирают.

pub mod model;
pub mod view;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

use model::{item_id, next_color, KanbanCard, KanbanColumn, KanbanDoc};

#[derive(Clone)]
pub struct KanbanHandle {
    doc: Arc<Mutex<KanbanDoc>>,
    pub revision: RwSignal<u64>,
    /// Карточка в режиме правки заголовка.
    pub editing: RwSignal<Option<String>>,
}

impl KanbanHandle {
    pub fn new(doc: KanbanDoc) -> Self {
        Self { doc: Arc::new(Mutex::new(doc)), revision: use_signal(0), editing: use_signal(None) }
    }

    pub fn lock(&self) -> MutexGuard<'_, KanbanDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    /// Правка документа + бамп ревизии (автосейв, перестройка).
    pub fn edit(&self, f: impl FnOnce(&mut KanbanDoc)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
    }

    // ─── Колонки ──────────────────────────────────────────────────────────

    pub fn add_column(&self, name: &str) -> String {
        let id = item_id("c");
        let column = KanbanColumn { id: id.clone(), name: name.to_string(), color: String::new() };
        self.edit(|doc| doc.columns.push(column));
        id
    }

    pub fn rename_column(&self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let changed = self.lock().columns.iter().any(|c| c.id == id && c.name != name);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                    c.name = name.to_string();
                }
            });
        }
    }

    /// Следующий цвет палитры у метки колонки.
    pub fn cycle_column_color(&self, id: &str) {
        self.edit(|doc| {
            if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                c.color = next_color(&c.color).to_string();
            }
        });
    }

    /// Удалить колонку; её карточки переезжают в соседнюю (слева, иначе
    /// справа), а без соседей — удаляются вместе с ней.
    pub fn delete_column(&self, id: &str) {
        self.edit(|doc| {
            let Some(idx) = doc.columns.iter().position(|c| c.id == id) else { return };
            let heir = idx
                .checked_sub(1)
                .or_else(|| (idx + 1 < doc.columns.len()).then_some(idx + 1))
                .map(|i| doc.columns[i].id.clone());
            doc.columns.remove(idx);
            match heir {
                Some(h) => doc.cards.iter_mut().filter(|c| c.column == id).for_each(|c| c.column = h.clone()),
                None => doc.cards.retain(|c| c.column != id),
            }
        });
    }

    // ─── Карточки ─────────────────────────────────────────────────────────

    /// Новая пустая карточка в конце колонки; сразу в режиме правки.
    pub fn add_card(&self, column: &str) -> String {
        let id = item_id("k");
        let card = KanbanCard { id: id.clone(), column: column.to_string(), title: String::new() };
        self.edit(|doc| doc.cards.push(card));
        self.editing.set(Some(id.clone()));
        id
    }

    pub fn set_card_title(&self, id: &str, title: &str) {
        let title = title.trim().to_string();
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.title != title);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.title = title;
                }
            });
        }
    }

    pub fn delete_card(&self, id: &str) {
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        self.edit(|doc| doc.cards.retain(|c| c.id != id));
    }

    /// Перенос карточки (дроп): в колонку перед `before` либо в её конец.
    pub fn move_card(&self, card: &str, column: &str, before: Option<&str>) {
        let mut moved = false;
        self.edit(|doc| moved = doc.move_card(card, column, before));
        let _ = moved;
    }

    /// Закончить правку заголовка: пустая новая карточка выбрасывается,
    /// чтобы на доске не копились безымянные.
    pub fn finish_editing(&self, id: &str, title: &str) {
        if title.trim().is_empty() {
            self.delete_card(id);
        } else {
            self.set_card_title(id, title);
        }
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
    }
}
