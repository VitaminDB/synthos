//! График — примитив страницы: живая врезка `![[chart:<id>]]` над объектом
//! `notes/objects/<id>.chart.json`.
//!
//! Рисуют графики виджеты библиотеки syngui (`syngui::widgets::charts`):
//! линии, столбцы, круговая, радар и шкала — [`view`] собирает нужный из
//! документа ([`model::ChartDoc`]). Своего редактирования на самом графике
//! нет: данные и оформление правятся в панели свойств блока и инструментом
//! агента, поэтому [`ChartHandle`] — только документ (Mutex) и сигналы:
//! `revision` — на каждую правку (автосейв), `structure_rev` — правка,
//! которую видно на картинке (перестройка виджета), `selected` — ряд,
//! открытый в панели свойств.

pub mod model;
pub mod view;

#[cfg(all(test, feature = "testing"))]
mod harness_tests;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

use model::{ChartDoc, ChartKind, ChartOptions};

#[derive(Clone)]
pub struct ChartHandle {
    doc: Arc<Mutex<ChartDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    /// id ряда, выбранного в панели свойств.
    pub selected: RwSignal<Option<String>>,
}

impl ChartHandle {
    pub fn new(doc: ChartDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            selected: use_signal(None),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, ChartDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    fn bump(&self) {
        self.revision.set(self.revision.get_untracked() + 1);
    }

    /// Правка, которую видно на графике: автосейв + перестройка виджета.
    pub fn edit(&self, f: impl FnOnce(&mut ChartDoc)) {
        {
            let mut doc = self.lock();
            f(&mut doc);
            doc.sanitize();
        }
        self.bump();
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    /// Правка без перестройки (название ряда по ходу набора — оно уходит в
    /// легенду только по завершении ввода).
    pub fn edit_data(&self, f: impl FnOnce(&mut ChartDoc)) {
        {
            let mut doc = self.lock();
            f(&mut doc);
            doc.sanitize();
        }
        self.bump();
    }

    pub fn kind(&self) -> ChartKind {
        self.lock().kind
    }

    /// Сменить вид: настройки прежнего вида остаются в документе — вернув
    /// вид назад, пользователь получает свой график, а не умолчания.
    pub fn set_kind(&self, kind: ChartKind) {
        if self.kind() == kind {
            return;
        }
        self.edit(|d| d.kind = kind);
    }

    pub fn set_title(&self, title: &str) {
        let title = title.trim().to_string();
        if self.lock().title == title {
            return;
        }
        self.edit(|d| d.title = title);
    }

    /// Подписи целиком: сколько подписей — столько точек в каждом ряду
    /// (лишние значения отрезаются, недостающие становятся нулями).
    pub fn set_categories(&self, labels: Vec<String>) {
        if self.lock().categories == labels {
            return;
        }
        self.edit(|d| {
            let n = labels.len();
            d.categories = labels;
            for s in &mut d.series {
                s.data.resize(n, 0.0);
            }
        });
    }

    pub fn set_category(&self, i: usize, text: &str) {
        let text = text.trim().to_string();
        let same = self.lock().categories.get(i).is_some_and(|c| *c == text);
        if same {
            return;
        }
        self.edit(|d| {
            if let Some(c) = d.categories.get_mut(i) {
                *c = text;
            }
        });
    }

    pub fn set_category_color(&self, i: usize, color: Option<String>) {
        self.edit(|d| d.set_color_at(i, color));
    }

    pub fn options(&self) -> ChartOptions {
        self.lock().options.clone()
    }

    pub fn set_options(&self, f: impl FnOnce(&mut ChartOptions)) {
        let mut o = self.options();
        f(&mut o);
        o.sanitize();
        if self.lock().options == o {
            return;
        }
        self.edit(|d| d.options = o);
    }

    // ─── Ряды ─────────────────────────────────────────────────────────────

    pub fn add_series(&self, name: &str, data: Vec<f64>) -> String {
        let mut id = String::new();
        self.edit(|d| id = d.add_series(name, data));
        self.selected.set(Some(id.clone()));
        id
    }

    pub fn rename_series(&self, id: &str, name: &str) {
        let name = name.trim().to_string();
        let same = self.lock().series(id).is_some_and(|s| s.name == name);
        if same {
            return;
        }
        self.edit(|d| {
            if let Some(s) = d.series_mut(id) {
                s.name = name;
            }
        });
    }

    pub fn set_series_color(&self, id: &str, color: Option<String>) {
        self.edit(|d| {
            if let Some(s) = d.series_mut(id) {
                s.color = color.unwrap_or_default();
            }
        });
    }

    pub fn set_series_data(&self, id: &str, data: Vec<f64>) {
        let same = self.lock().series(id).is_some_and(|s| s.data == data);
        if same {
            return;
        }
        self.edit(|d| {
            if let Some(s) = d.series_mut(id) {
                s.data = data;
            }
        });
    }

    /// Значение одной точки (панель свойств круговой и шкалы).
    pub fn set_value(&self, id: &str, index: usize, value: f64) {
        let same = self.lock().series(id).is_some_and(|s| (s.at(index) - value).abs() < 1e-9);
        if same {
            return;
        }
        self.edit(|d| {
            if let Some(s) = d.series_mut(id) {
                while s.data.len() <= index {
                    s.data.push(0.0);
                }
                s.data[index] = value;
            }
        });
    }

    pub fn delete_series(&self, id: &str) -> bool {
        let mut ok = false;
        self.edit(|d| ok = d.remove_series(id));
        if ok && self.selected.get_untracked().as_deref() == Some(id) {
            self.selected.set(None);
        }
        ok
    }

    /// Новая подпись с нулями во всех рядах (кнопка «+» в панели).
    pub fn add_category(&self, label: &str) {
        let label = label.trim().to_string();
        self.edit(|d| {
            if d.kind == ChartKind::Gauge {
                return;
            }
            let n = d.categories.len();
            d.categories.push(if label.is_empty() { format!("{}", n + 1) } else { label });
            for s in &mut d.series {
                while s.data.len() < n + 1 {
                    s.data.push(0.0);
                }
            }
        });
    }

    pub fn delete_category(&self, i: usize) {
        self.edit(|d| {
            if i >= d.categories.len() {
                return;
            }
            d.categories.remove(i);
            if i < d.colors.len() {
                d.colors.remove(i);
            }
            for s in &mut d.series {
                if i < s.data.len() {
                    s.data.remove(i);
                }
            }
        });
    }
}
