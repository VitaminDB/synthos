//! Формат диаграммы Ганта: `notes/objects/<id>.gantt.json`.
//!
//! Задачи с датами начала/конца (ISO `YYYY-MM-DD`, включительно), цветом и
//! зависимостями «конец A → начало B». Масштаб шкалы хранится в файле,
//! чтобы диаграмма открывалась в том же приближении.
//!
//! `boards` — доски, чьи запланированные карточки видны на диаграмме
//! отдельными строками (`start`/`end` карточки). В отличие от календаря,
//! который смотрит на весь проект, здесь пусто = только свои задачи:
//! диаграмма — про один план, а не про обзор.

use serde::{Deserialize, Serialize};

use super::calendar::{days_to_iso, parse_days, today_days};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GanttDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub tasks: Vec<GanttTask>,
    #[serde(default)]
    pub deps: Vec<GanttDep>,
    /// Пикселей на день.
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    /// Доски (id объектов `kanban`), чьи запланированные карточки видны
    /// строками диаграммы; пусто — только собственные задачи.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boards: Vec<String>,
}

fn default_version() -> u32 {
    1
}

pub fn default_zoom() -> f32 {
    26.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GanttTask {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub start: String,
    pub end: String,
    /// `#rrggbb`; пусто — акцент темы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

impl GanttTask {
    /// Даты в днях от эпохи; без начала задача не рисуется, без конца —
    /// однодневная, перепутанные — переставляются.
    pub fn span_days(&self) -> Option<(i64, i64)> {
        let s = parse_days(&self.start)?;
        let e = parse_days(&self.end).unwrap_or(s);
        Some(if e < s { (e, s) } else { (s, e) })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GanttDep {
    pub from: String,
    pub to: String,
}

impl GanttDoc {
    pub fn template() -> Self {
        Self { version: 1, tasks: Vec::new(), deps: Vec::new(), zoom: default_zoom(), boards: Vec::new() }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut doc: Self = serde_json::from_str(json)?;
        if !doc.zoom.is_finite() || doc.zoom < 1.0 {
            doc.zoom = default_zoom();
        }
        Ok(doc)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Новая задача: начинается сегодня, длится три дня; кладётся после
    /// `after` (id) либо в конец.
    pub fn new_task(name: &str) -> GanttTask {
        let today = today_days();
        GanttTask {
            id: super::super::kanban::model::item_id("t"),
            name: name.to_string(),
            start: days_to_iso(today),
            end: days_to_iso(today + 2),
            color: String::new(),
        }
    }

    /// Добавить зависимость; дубли и петли отбрасываются.
    pub fn add_dep(&mut self, from: &str, to: &str) -> bool {
        if from == to
            || !self.tasks.iter().any(|t| t.id == from)
            || !self.tasks.iter().any(|t| t.id == to)
            || self.deps.iter().any(|d| d.from == from && d.to == to)
        {
            return false;
        }
        self.deps.push(GanttDep { from: from.to_string(), to: to.to_string() });
        true
    }

    pub fn remove_task(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id);
        self.deps.retain(|d| d.from != id && d.to != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_deps() {
        let mut d = GanttDoc::template();
        let mut a = GanttDoc::new_task("A");
        a.id = "a".into();
        let mut b = GanttDoc::new_task("B");
        b.id = "b".into();
        b.start = "2026-09-10".into();
        b.end = "2026-09-08".into();
        d.tasks.push(a);
        d.tasks.push(b);
        assert!(d.add_dep("a", "b"));
        assert!(!d.add_dep("a", "b"), "дубль");
        assert!(!d.add_dep("a", "a"), "петля");
        assert!(!d.add_dep("a", "нет"));
        let back = GanttDoc::parse(&d.serialize()).unwrap();
        assert_eq!(d, back);
        // Перепутанные даты переставляются.
        let (s, e) = back.tasks[1].span_days().unwrap();
        assert_eq!(days_to_iso(s), "2026-09-08");
        assert_eq!(days_to_iso(e), "2026-09-10");
        d.remove_task("a");
        assert!(d.deps.is_empty(), "зависимости удалённой задачи уходят с ней");
    }

    #[test]
    fn bad_zoom_falls_back() {
        let doc = GanttDoc::parse(r#"{"tasks":[],"deps":[],"zoom":0}"#).unwrap();
        assert_eq!(doc.zoom, default_zoom());
    }
}
