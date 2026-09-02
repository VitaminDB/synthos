//! Формат канбан-доски: `notes/objects/<id>.kanban.json`.
//!
//! Доска самодостаточна: колонки (название + цвет) и карточки с заголовком.
//! Порядок карточек в колонке — порядок в `cards`; никакой «базы» под
//! доской нет, всё правится прямо на ней.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub columns: Vec<KanbanColumn>,
    #[serde(default)]
    pub cards: Vec<KanbanCard>,
}

fn default_version() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanColumn {
    pub id: String,
    pub name: String,
    /// `#rrggbb`; пусто — без цветной метки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanCard {
    pub id: String,
    /// id колонки.
    pub column: String,
    #[serde(default)]
    pub title: String,
}

/// Палитра цветных меток (колонки доски, задачи диаграммы): клик по
/// метке переключает на следующий цвет по кругу.
pub const PALETTE: [&str; 7] =
    ["#8B95A6", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#EE5E48", "#3FB8C4"];

/// Следующий цвет палитры после `current` (незнакомый/пустой → первый).
pub fn next_color(current: &str) -> &'static str {
    let idx = PALETTE.iter().position(|c| c.eq_ignore_ascii_case(current));
    PALETTE[idx.map(|i| (i + 1) % PALETTE.len()).unwrap_or(0)]
}

/// Уникальный короткий id элемента доски/диаграммы (unix-мс + счётчик).
pub fn item_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}{}-{}", crate::config::now_millis(), N.fetch_add(1, Ordering::Relaxed))
}

impl KanbanDoc {
    /// Новая доска с тремя колонками; названия — на языке интерфейса.
    pub fn template(names: [&str; 3]) -> Self {
        let colors = ["#8B95A6", "#E8A33D", "#4FBF7A"];
        Self {
            version: 1,
            columns: names
                .iter()
                .zip(colors)
                .map(|(name, color)| KanbanColumn {
                    id: item_id("c"),
                    name: name.to_string(),
                    color: color.to_string(),
                })
                .collect(),
            cards: Vec::new(),
        }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Карточки колонки в порядке доски.
    pub fn cards_of(&self, column: &str) -> Vec<&KanbanCard> {
        self.cards.iter().filter(|c| c.column == column).collect()
    }

    /// Перенос карточки в колонку `column` перед карточкой `before`
    /// (`None` — в конец колонки). Возвращает `false`, если карточки нет.
    pub fn move_card(&mut self, card: &str, column: &str, before: Option<&str>) -> bool {
        let Some(pos) = self.cards.iter().position(|c| c.id == card) else { return false };
        if before == Some(card) {
            return true;
        }
        let mut moved = self.cards.remove(pos);
        moved.column = column.to_string();
        let at = before
            .and_then(|b| self.cards.iter().position(|c| c.id == b && c.column == column))
            .unwrap_or_else(|| {
                // В конец колонки: сразу после её последней карточки, чтобы
                // порядок колонок в `cards` не перемешивался.
                self.cards
                    .iter()
                    .rposition(|c| c.column == column)
                    .map(|i| i + 1)
                    .unwrap_or(self.cards.len())
            });
        self.cards.insert(at, moved);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> KanbanDoc {
        let mut d = KanbanDoc::template(["Todo", "Doing", "Done"]);
        for (i, col) in [0usize, 0, 1].into_iter().enumerate() {
            d.cards.push(KanbanCard {
                id: format!("k{i}"),
                column: d.columns[col].id.clone(),
                title: format!("Задача {i}"),
            });
        }
        d
    }

    #[test]
    fn template_roundtrip() {
        let d = doc();
        let back = KanbanDoc::parse(&d.serialize()).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.columns.len(), 3);
        assert_eq!(back.cards_of(&back.columns[0].id).len(), 2);
    }

    #[test]
    fn move_card_between_and_within_columns() {
        let mut d = doc();
        let (todo, doing) = (d.columns[0].id.clone(), d.columns[1].id.clone());
        // В конец другой колонки.
        assert!(d.move_card("k0", &doing, None));
        let ids: Vec<&str> = d.cards_of(&doing).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["k2", "k0"]);
        // Перед карточкой внутри колонки.
        assert!(d.move_card("k0", &doing, Some("k2")));
        let ids: Vec<&str> = d.cards_of(&doing).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["k0", "k2"]);
        assert_eq!(d.cards_of(&todo).len(), 1);
        assert!(!d.move_card("нет", &todo, None));
    }

    #[test]
    fn palette_cycles() {
        assert_eq!(next_color(""), PALETTE[0]);
        assert_eq!(next_color(PALETTE[0]), PALETTE[1]);
        assert_eq!(next_color(PALETTE[PALETTE.len() - 1]), PALETTE[0]);
    }
}
