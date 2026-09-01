//! Формат «базы данных» заметок: `Имя.base.json`.
//!
//! Одна база = типизированные колонки + строки + представления
//! (Таблица/Канбан/Ганнт) над одними данными, как database в Notion.
//! Файл — обычный pretty-JSON в vault'е, переносимый и диффабельный.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    pub columns: Vec<BaseColumn>,
    #[serde(default)]
    pub rows: Vec<BaseRow>,
    #[serde(default)]
    pub views: Vec<BaseView>,
}

fn default_version() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseColumn {
    pub id: String,
    pub name: String,
    pub kind: ColumnKind,
    /// Для select/multi_select.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Text,
    Number,
    Date,
    Select,
    MultiSelect,
    Checkbox,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseRow {
    pub id: String,
    #[serde(default)]
    pub cells: BTreeMap<String, CellValue>,
}

/// Значение ячейки. `untagged`: bool/число/строка/список — естественный JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellValue {
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<String>),
}

impl CellValue {
    pub fn as_text(&self) -> String {
        match self {
            CellValue::Bool(b) => if *b { "true".into() } else { "false".into() },
            CellValue::Number(n) => {
                if n.fract() == 0.0 { format!("{}", *n as i64) } else { format!("{n}") }
            }
            CellValue::Text(s) => s.clone(),
            CellValue::List(v) => v.join(", "),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseView {
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    /// Таблица: ширины колонок по id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub widths: BTreeMap<String, f32>,
    /// Канбан: колонка-select группировки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    /// Ганнт: колонки дат начала/конца и подписи.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_col: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_col: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_col: Option<String>,
    /// Ганнт: зависимости (id строки → id строки).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Table,
    Kanban,
    Gantt,
}

/// Уникальный короткий id (unix-мс + счётчик).
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}{}-{}", crate::config::now_millis(), N.fetch_add(1, Ordering::Relaxed))
}

impl BaseDoc {
    /// Шаблон новой базы: Название/Статус/Срок + три представления.
    pub fn template() -> Self {
        let status_opts = vec![
            SelectOption { id: "todo".into(), name: "Todo".into(), color: "#8b95a6".into() },
            SelectOption { id: "doing".into(), name: "In progress".into(), color: "#e0a030".into() },
            SelectOption { id: "done".into(), name: "Done".into(), color: "#4fbf7a".into() },
        ];
        Self {
            version: 1,
            columns: vec![
                BaseColumn { id: "title".into(), name: "Название".into(), kind: ColumnKind::Text, options: vec![] },
                BaseColumn { id: "status".into(), name: "Статус".into(), kind: ColumnKind::Select, options: status_opts },
                BaseColumn { id: "start".into(), name: "Начало".into(), kind: ColumnKind::Date, options: vec![] },
                BaseColumn { id: "due".into(), name: "Срок".into(), kind: ColumnKind::Date, options: vec![] },
                BaseColumn { id: "done".into(), name: "Готово".into(), kind: ColumnKind::Checkbox, options: vec![] },
            ],
            rows: Vec::new(),
            views: vec![
                BaseView {
                    id: "table".into(),
                    name: "Таблица".into(),
                    kind: ViewKind::Table,
                    widths: BTreeMap::new(),
                    group_by: None,
                    start_col: None,
                    end_col: None,
                    label_col: None,
                    deps: Vec::new(),
                },
                BaseView {
                    id: "kanban".into(),
                    name: "Канбан".into(),
                    kind: ViewKind::Kanban,
                    widths: BTreeMap::new(),
                    group_by: Some("status".into()),
                    start_col: None,
                    end_col: None,
                    label_col: Some("title".into()),
                    deps: Vec::new(),
                },
                BaseView {
                    id: "gantt".into(),
                    name: "Ганнт".into(),
                    kind: ViewKind::Gantt,
                    widths: BTreeMap::new(),
                    group_by: None,
                    start_col: Some("start".into()),
                    end_col: Some("due".into()),
                    label_col: Some("title".into()),
                    deps: Vec::new(),
                },
            ],
        }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn column(&self, id: &str) -> Option<&BaseColumn> {
        self.columns.iter().find(|c| c.id == id)
    }

    pub fn cell_text(&self, row: &BaseRow, col_id: &str) -> String {
        row.cells.get(col_id).map(|v| v.as_text()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_roundtrip() {
        let doc = BaseDoc::template();
        let json = doc.serialize();
        let back = BaseDoc::parse(&json).unwrap();
        assert_eq!(doc, back);
        assert_eq!(back.views.len(), 3);
        assert_eq!(back.columns[1].kind, ColumnKind::Select);
    }

    #[test]
    fn cell_values_untagged() {
        let json = r#"{"version":1,"columns":[],"rows":[
            {"id":"r1","cells":{"a":true,"b":3.5,"c":"текст","d":["x","y"]}}
        ],"views":[]}"#;
        let doc = BaseDoc::parse(json).unwrap();
        let row = &doc.rows[0];
        assert_eq!(row.cells["a"], CellValue::Bool(true));
        assert_eq!(row.cells["b"], CellValue::Number(3.5));
        assert_eq!(row.cells["c"], CellValue::Text("текст".into()));
        assert_eq!(row.cells["d"], CellValue::List(vec!["x".into(), "y".into()]));
    }
}
