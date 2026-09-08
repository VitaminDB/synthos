//! Диаграмма Ганта — примитив страницы: живая врезка `![[gantt:<id>]]`
//! над объектом `notes/objects/<id>.gantt.json`.
//!
//! [`GanttHandle`] держит документ (Mutex) и сигналы: `revision` — на
//! каждую правку (автосейв + перестройка виджета), `go_today` — просьба
//! показать «сегодня» (тулбар → элемент шкалы).
//!
//! [`GanttEnv`] — доски проекта: карточки выбранных досок с планом
//! (`start`/`end`) идут строками наравне со своими задачами, и бар,
//! который тянут мышью, пишет новые даты обратно в карточку. Так одна и та
//! же задача видна и в календаре, и на диаграмме.

pub mod calendar;
pub mod chart;
pub mod model;
pub mod view;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

use super::kanban::model::next_color;
use calendar::days_to_iso;
use model::GanttDoc;

/// Карточка доски строкой диаграммы (read-only подпись, живой бар).
#[derive(Clone, Debug, PartialEq)]
pub struct BoardTask {
    /// id объекта `kanban`.
    pub board: String,
    pub card: String,
    pub name: String,
    pub color: String,
    /// Дни от эпохи, включительно.
    pub start: i64,
    pub end: i64,
    /// Страница, где живёт доска.
    pub page: String,
}

/// Окружение диаграммы: карточки досок и запись новых дат обратно.
#[derive(Clone)]
pub struct GanttEnv {
    /// Запланированные карточки выбранных досок (пусто на входе — пусто на
    /// выходе: диаграмма показывает только свои задачи).
    pub cards: Arc<dyn Fn(&[String]) -> Vec<BoardTask> + Send + Sync>,
    /// Новая полоса карточки после переноса/растяжения бара.
    pub set_card_span: Arc<dyn Fn(&str, &str, i64, i64) + Send + Sync>,
    /// Доски проекта для панели свойств: `(id, название страницы)`.
    pub boards: Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>,
    /// Ревизия объектов проекта: правка карточки перерисовывает диаграмму.
    pub project_rev: RwSignal<u64>,
    pub open_page: Arc<dyn Fn(&str) + Send + Sync>,
}

impl GanttEnv {
    /// Окружение без проекта (тесты, превью шаблона).
    pub fn detached() -> Self {
        Self {
            cards: Arc::new(|_| Vec::new()),
            set_card_span: Arc::new(|_, _, _, _| {}),
            boards: Arc::new(Vec::new),
            project_rev: use_signal(0),
            open_page: Arc::new(|_| {}),
        }
    }
}

/// Пределы масштаба шкалы (px/день).
pub const ZOOM_MIN: f32 = 5.0;
pub const ZOOM_MAX: f32 = 90.0;

#[derive(Clone)]
pub struct GanttHandle {
    doc: Arc<Mutex<GanttDoc>>,
    pub revision: RwSignal<u64>,
    /// Бамп — прокрутить шкалу к сегодняшнему дню.
    pub go_today: RwSignal<u64>,
    /// Общая ревизия объектов проекта ([`super::state::NotesCtx`]).
    project_rev: Option<RwSignal<u64>>,
}

impl GanttHandle {
    pub fn new(doc: GanttDoc) -> Self {
        Self { doc: Arc::new(Mutex::new(doc)), revision: use_signal(0), go_today: use_signal(0), project_rev: None }
    }

    /// Общая ревизия объектов проекта: по ней перестраивается слой досок.
    pub fn with_project_rev(mut self, rev: RwSignal<u64>) -> Self {
        self.project_rev = Some(rev);
        self
    }

    pub fn lock(&self) -> MutexGuard<'_, GanttDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    pub fn edit(&self, f: impl FnOnce(&mut GanttDoc)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
        if let Some(rev) = self.project_rev {
            rev.set(rev.get_untracked() + 1);
        }
    }

    pub fn add_task(&self, name: &str) -> String {
        let task = GanttDoc::new_task(name);
        let id = task.id.clone();
        self.edit(|doc| doc.tasks.push(task));
        id
    }

    pub fn rename_task(&self, id: &str, name: &str) {
        let name = name.trim();
        let changed = self.lock().tasks.iter().any(|t| t.id == id && t.name != name);
        if changed {
            self.edit(|doc| {
                if let Some(t) = doc.tasks.iter_mut().find(|t| t.id == id) {
                    t.name = name.to_string();
                }
            });
        }
    }

    /// Даты в днях от эпохи (конец не раньше начала).
    pub fn set_task_dates(&self, id: &str, start: i64, end: i64) {
        self.edit(|doc| {
            if let Some(t) = doc.tasks.iter_mut().find(|t| t.id == id) {
                t.start = days_to_iso(start);
                t.end = days_to_iso(end.max(start));
            }
        });
    }

    pub fn cycle_task_color(&self, id: &str) {
        self.edit(|doc| {
            if let Some(t) = doc.tasks.iter_mut().find(|t| t.id == id) {
                t.color = next_color(&t.color).to_string();
            }
        });
    }

    pub fn delete_task(&self, id: &str) {
        self.edit(|doc| doc.remove_task(id));
    }

    pub fn add_dep(&self, from: &str, to: &str) {
        let mut added = false;
        self.edit(|doc| added = doc.add_dep(from, to));
        let _ = added;
    }

    pub fn delete_dep(&self, from: &str, to: &str) {
        self.edit(|doc| doc.deps.retain(|d| !(d.from == from && d.to == to)));
    }

    pub fn zoom(&self) -> f32 {
        self.lock().zoom
    }

    pub fn set_zoom(&self, px_per_day: f32) {
        let z = px_per_day.clamp(ZOOM_MIN, ZOOM_MAX);
        if (self.lock().zoom - z).abs() < 0.01 {
            return;
        }
        self.edit(|doc| doc.zoom = z);
    }

    pub fn show_today(&self) {
        self.go_today.set(self.go_today.get_untracked() + 1);
    }

    pub fn boards(&self) -> Vec<String> {
        self.lock().boards.clone()
    }

    pub fn toggle_board(&self, id: &str) {
        self.edit(|doc| match doc.boards.iter().position(|b| b == id) {
            Some(pos) => {
                doc.boards.remove(pos);
            }
            None => doc.boards.push(id.to_string()),
        });
    }
}
