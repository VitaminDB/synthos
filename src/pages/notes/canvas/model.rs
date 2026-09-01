//! Формат канваса заметок: `Имя.canvas.json`.
//!
//! Свой JSON (решение плана): карточки с markdown-текстом на бесконечном
//! холсте + рёбра со стрелками. Камера (пан/зум) хранится в файле, чтобы
//! канвас открывался там же, где его оставили.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub camera: Camera,
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub edges: Vec<CanvasEdge>,
}

fn default_version() -> u32 {
    1
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { pan_x: 0.0, pan_y: 0.0, zoom: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasNode {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Цвет-акцент карточки (`#rrggbb`, пусто — тема).
    #[serde(default)]
    pub color: String,
    /// Markdown-содержимое карточки.
    #[serde(default)]
    pub md: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
}

impl CanvasDoc {
    pub fn template() -> Self {
        Self {
            version: 1,
            camera: Camera::default(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_roundtrip() {
        let mut doc = CanvasDoc::template();
        doc.nodes.push(CanvasNode {
            id: "n1".into(),
            x: 10.0,
            y: 20.0,
            w: 260.0,
            h: 140.0,
            color: "#4f8cff".into(),
            md: "# Идея\n\nтекст".into(),
        });
        doc.edges.push(CanvasEdge {
            id: "e1".into(),
            from: "n1".into(),
            to: "n1".into(),
            label: String::new(),
        });
        let back = CanvasDoc::parse(&doc.serialize()).unwrap();
        assert_eq!(doc, back);
    }
}
