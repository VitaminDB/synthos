//! Формат интеллект-карты: `notes/objects/<id>.mindmap.json`.
//!
//! Дерево узлов: у каждого — родитель (у корня его нет), текст, заметка
//! (markdown), ссылка на страницу, иконка, цвет, форма, флаг «свёрнут» и
//! ручной сдвиг `dx/dy` относительно автораскладки. Порядок братьев —
//! порядок узлов в `nodes`. Кросс-ссылки между любыми узлами — `links`.
//! Раскладка (`layout`), оформление (`style`) и положение вида (`view`)
//! хранятся в файле, чтобы карта открывалась в том же виде.

use serde::{Deserialize, Serialize};

use super::super::kanban::model::item_id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MindmapDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub nodes: Vec<MindNode>,
    #[serde(default)]
    pub links: Vec<CrossLink>,
    #[serde(default)]
    pub layout: MapLayout,
    #[serde(default)]
    pub style: MindmapStyle,
    #[serde(default)]
    pub view: MapView,
}

fn default_version() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MindNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub text: String,
    /// Заметка узла — markdown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Страница проекта (id), к которой ведёт узел.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Эмодзи перед текстом.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// `#rrggbb`; пусто — цвет ветки по палитре.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "NodeShape::is_auto")]
    pub shape: NodeShape,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// Ручной сдвиг поддерева относительно автораскладки.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dx: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dy: f32,
}

fn is_zero(v: &f32) -> bool {
    v.abs() < 0.01
}

impl MindNode {
    pub fn new(parent: Option<&str>, text: &str) -> Self {
        Self {
            id: item_id("n"),
            parent: parent.map(str::to_string),
            text: text.to_string(),
            note: String::new(),
            link: None,
            icon: String::new(),
            color: String::new(),
            shape: NodeShape::Auto,
            collapsed: false,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

/// Форма узла; `auto` — по уровню (корень — капсула, первый уровень —
/// скруглённый прямоугольник, глубже — текст на линии).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeShape {
    #[default]
    Auto,
    Rect,
    Rounded,
    Pill,
    Ellipse,
    Text,
}

impl NodeShape {
    pub const ALL: [NodeShape; 6] =
        [NodeShape::Auto, NodeShape::Rect, NodeShape::Rounded, NodeShape::Pill, NodeShape::Ellipse, NodeShape::Text];

    fn is_auto(&self) -> bool {
        *self == NodeShape::Auto
    }

    pub fn key(self) -> &'static str {
        match self {
            NodeShape::Auto => "auto",
            NodeShape::Rect => "rect",
            NodeShape::Rounded => "rounded",
            NodeShape::Pill => "pill",
            NodeShape::Ellipse => "ellipse",
            NodeShape::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.key() == s.trim().to_ascii_lowercase())
    }
}

/// Направление роста дерева.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Right,
    Left,
    Both,
    Down,
    Radial,
}

impl Direction {
    pub const ALL: [Direction; 5] =
        [Direction::Right, Direction::Left, Direction::Both, Direction::Down, Direction::Radial];

    pub fn key(self) -> &'static str {
        match self {
            Direction::Right => "right",
            Direction::Left => "left",
            Direction::Both => "both",
            Direction::Down => "down",
            Direction::Radial => "radial",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.key() == s.trim().to_ascii_lowercase())
    }
}

/// Вид линий между узлами.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    #[default]
    Bezier,
    Straight,
    Elbow,
}

impl Curve {
    pub const ALL: [Curve; 3] = [Curve::Bezier, Curve::Straight, Curve::Elbow];

    pub fn key(self) -> &'static str {
        match self {
            Curve::Bezier => "bezier",
            Curve::Straight => "straight",
            Curve::Elbow => "elbow",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.key() == s.trim().to_ascii_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapLayout {
    #[serde(default)]
    pub direction: Direction,
    /// Расстояние между уровнями (вдоль роста), px.
    #[serde(default = "default_h_gap")]
    pub h_gap: f32,
    /// Расстояние между соседними узлами (поперёк), px.
    #[serde(default = "default_v_gap")]
    pub v_gap: f32,
    #[serde(default)]
    pub curve: Curve,
}

pub const DEFAULT_H_GAP: f32 = 48.0;
pub const DEFAULT_V_GAP: f32 = 14.0;

fn default_h_gap() -> f32 {
    DEFAULT_H_GAP
}
fn default_v_gap() -> f32 {
    DEFAULT_V_GAP
}

impl Default for MapLayout {
    fn default() -> Self {
        Self { direction: Direction::Right, h_gap: DEFAULT_H_GAP, v_gap: DEFAULT_V_GAP, curve: Curve::Bezier }
    }
}

/// Кросс-ссылка между узлами (пунктирная стрелка с подписью).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossLink {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Длина штриха (0 — сплошная).
    #[serde(default = "default_link_dash")]
    pub dash: f32,
}

fn default_link_dash() -> f32 {
    6.0
}

/// Именованные палитры цветов веток.
pub const PALETTES: [(&str, [&str; 6]); 4] = [
    ("theme", ["#4F8CFF", "#4FBF7A", "#E8A33D", "#C08FE8", "#EE5E48", "#2EC4B6"]),
    ("rainbow", ["#FF5A5F", "#FF9F1C", "#FFD60A", "#2EC4B6", "#3A86FF", "#8338EC"]),
    ("pastel", ["#F7A1A1", "#F9C784", "#FFF3B0", "#A8E6CF", "#9AD0F5", "#CDB4F0"]),
    ("mono", ["#8B95A6", "#A0A8B6", "#B5BCC7", "#CAD0D8", "#DFE3E9", "#6B7280"]),
];

pub fn palette_by_key(key: &str) -> Option<&'static [&'static str; 6]> {
    PALETTES.iter().find(|(k, _)| *k == key).map(|(_, p)| p)
}

/// Оформление карты поверх темы; пустая строка/ноль — «как в теме».
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MindmapStyle {
    /// Цвета веток по порядку детей корня.
    #[serde(default = "default_palette")]
    pub palette: Vec<String>,
    /// Заливка узлов первого уровня (`#rrggbb[aa]`); пусто — цвет ветки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_fill: String,
    /// Обводка узлов; пусто — цвет ветки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_stroke: String,
    /// Цвет текста; пусто — из темы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text_color: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// `normal` | `bold`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub weight: String,
    #[serde(default = "default_radius")]
    pub radius: f32,
    #[serde(default = "default_padding")]
    pub padding: f32,
    /// Цвет линий; пусто — цвет ветки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub line_color: String,
    #[serde(default = "default_line_width")]
    pub line_width: f32,
    /// Пунктир линий (0 — сплошные).
    #[serde(default)]
    pub line_dash: f32,
    /// Фон карты; пусто — из темы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bg: String,
    #[serde(default = "default_true")]
    pub show_icons: bool,
    /// Максимальная ширина узла до переноса строк.
    #[serde(default = "default_max_node_w")]
    pub max_node_w: f32,
}

fn default_palette() -> Vec<String> {
    PALETTES[0].1.iter().map(|s| s.to_string()).collect()
}
fn default_font_size() -> f32 {
    13.0
}
fn default_radius() -> f32 {
    8.0
}
fn default_padding() -> f32 {
    8.0
}
fn default_line_width() -> f32 {
    2.0
}
fn default_true() -> bool {
    true
}
fn default_max_node_w() -> f32 {
    240.0
}

impl Default for MindmapStyle {
    fn default() -> Self {
        Self {
            palette: default_palette(),
            node_fill: String::new(),
            node_stroke: String::new(),
            text_color: String::new(),
            font_size: default_font_size(),
            weight: String::new(),
            radius: default_radius(),
            padding: default_padding(),
            line_color: String::new(),
            line_width: default_line_width(),
            line_dash: 0.0,
            bg: String::new(),
            show_icons: true,
            max_node_w: default_max_node_w(),
        }
    }
}

impl MindmapStyle {
    /// Санитизация после чтения/правки: границы значений и непустая палитра.
    pub fn sanitize(&mut self) {
        if self.palette.is_empty() {
            self.palette = default_palette();
        }
        if !self.font_size.is_finite() || !(8.0..=40.0).contains(&self.font_size) {
            self.font_size = default_font_size();
        }
        self.radius = if self.radius.is_finite() { self.radius.clamp(0.0, 40.0) } else { default_radius() };
        self.padding = if self.padding.is_finite() { self.padding.clamp(2.0, 30.0) } else { default_padding() };
        self.line_width = if self.line_width.is_finite() { self.line_width.clamp(0.5, 8.0) } else { default_line_width() };
        self.line_dash = if self.line_dash.is_finite() { self.line_dash.clamp(0.0, 30.0) } else { 0.0 };
        self.max_node_w = if self.max_node_w.is_finite() { self.max_node_w.clamp(80.0, 800.0) } else { default_max_node_w() };
    }
}

/// Положение вида: сдвиг и масштаб `PanZoomViewport`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    #[serde(default)]
    pub pan: (f32, f32),
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

fn default_zoom() -> f32 {
    1.0
}

impl Default for MapView {
    fn default() -> Self {
        Self { pan: (0.0, 0.0), zoom: 1.0 }
    }
}

impl MindmapDoc {
    /// Карта с одним корнем.
    pub fn template(root_text: &str) -> Self {
        Self {
            version: 1,
            nodes: vec![MindNode::new(None, root_text)],
            links: Vec::new(),
            layout: MapLayout::default(),
            style: MindmapStyle::default(),
            view: MapView::default(),
        }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut doc: Self = serde_json::from_str(json)?;
        doc.sanitize();
        Ok(doc)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Починить дерево: единственный корень, сироты — к корню, ссылки на
    /// несуществующие узлы — прочь, числа в границах.
    pub fn sanitize(&mut self) {
        if self.nodes.is_empty() {
            self.nodes.push(MindNode::new(None, ""));
        }
        let ids: Vec<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        let root = self.nodes.iter().position(|n| n.parent.is_none()).unwrap_or(0);
        let root_id = self.nodes[root].id.clone();
        for (i, n) in self.nodes.iter_mut().enumerate() {
            if i == root {
                n.parent = None;
                continue;
            }
            match &n.parent {
                Some(p) if ids.contains(p) && *p != n.id => {}
                _ => n.parent = Some(root_id.clone()),
            }
            if !n.dx.is_finite() {
                n.dx = 0.0;
            }
            if !n.dy.is_finite() {
                n.dy = 0.0;
            }
        }
        // Циклы (a→b→a) — разрываются переподвешиванием к корню.
        let mut i = 0;
        while i < self.nodes.len() {
            if i != root && self.is_descendant(&self.nodes[i].id, &self.nodes[i].id) {
                self.nodes[i].parent = Some(root_id.clone());
            }
            i += 1;
        }
        self.links.retain(|l| l.from != l.to && ids.contains(&l.from) && ids.contains(&l.to));
        self.style.sanitize();
        if !self.layout.h_gap.is_finite() || !(8.0..=400.0).contains(&self.layout.h_gap) {
            self.layout.h_gap = DEFAULT_H_GAP;
        }
        if !self.layout.v_gap.is_finite() || !(0.0..=200.0).contains(&self.layout.v_gap) {
            self.layout.v_gap = DEFAULT_V_GAP;
        }
        if !self.view.zoom.is_finite() || !(0.1..=5.0).contains(&self.view.zoom) {
            self.view.zoom = 1.0;
        }
        if !self.view.pan.0.is_finite() || !self.view.pan.1.is_finite() {
            self.view.pan = (0.0, 0.0);
        }
    }

    pub fn root(&self) -> Option<&MindNode> {
        self.nodes.iter().find(|n| n.parent.is_none())
    }

    pub fn root_id(&self) -> String {
        self.root().map(|n| n.id.clone()).unwrap_or_default()
    }

    pub fn node(&self, id: &str) -> Option<&MindNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut MindNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Дети в порядке следования в `nodes`.
    pub fn children_of(&self, id: &str) -> Vec<&MindNode> {
        self.nodes.iter().filter(|n| n.parent.as_deref() == Some(id)).collect()
    }

    pub fn depth_of(&self, id: &str) -> usize {
        let mut depth = 0;
        let mut cur = self.node(id).and_then(|n| n.parent.clone());
        while let Some(p) = cur {
            depth += 1;
            if depth > self.nodes.len() {
                break;
            }
            cur = self.node(&p).and_then(|n| n.parent.clone());
        }
        depth
    }

    /// Поддерево (включая сам узел), DFS в порядке братьев.
    pub fn subtree_ids(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            if out.contains(&cur) {
                continue;
            }
            out.push(cur.clone());
            for c in self.children_of(&cur).iter().rev() {
                stack.push(c.id.clone());
            }
        }
        out
    }

    /// `id` лежит в поддереве `of` (сам `of` — тоже «потомок» ради проверок циклов).
    pub fn is_descendant(&self, id: &str, of: &str) -> bool {
        let mut cur = self.node(id).and_then(|n| n.parent.clone());
        let mut guard = 0;
        while let Some(p) = cur {
            if p == of {
                return true;
            }
            guard += 1;
            if guard > self.nodes.len() {
                return true;
            }
            cur = self.node(&p).and_then(|n| n.parent.clone());
        }
        false
    }

    /// Позиция в `nodes`, куда вставить нового ребёнка `parent` с индексом
    /// `index` среди братьев (`None` — в конец).
    fn insert_slot(&self, parent: &str, index: Option<usize>) -> usize {
        let siblings: Vec<usize> =
            self.nodes.iter().enumerate().filter(|(_, n)| n.parent.as_deref() == Some(parent)).map(|(i, _)| i).collect();
        match index {
            Some(i) if i < siblings.len() => siblings[i],
            _ => siblings
                .last()
                .map(|&last| last + 1)
                .or_else(|| self.nodes.iter().position(|n| n.id == parent).map(|p| p + 1))
                .unwrap_or(self.nodes.len()),
        }
    }

    /// Новый узел под `parent`; возвращает id.
    pub fn add_node(&mut self, parent: &str, text: &str, index: Option<usize>) -> Option<String> {
        self.node(parent)?;
        let node = MindNode::new(Some(parent), text);
        let id = node.id.clone();
        let slot = self.insert_slot(parent, index);
        self.nodes.insert(slot.min(self.nodes.len()), node);
        Some(id)
    }

    /// Перенести узел под нового родителя (в своё поддерево нельзя, корень
    /// не переносится); `index` — место среди братьев.
    pub fn move_node(&mut self, id: &str, new_parent: &str, index: Option<usize>) -> bool {
        if id == new_parent || self.node(id).is_none() || self.node(new_parent).is_none() {
            return false;
        }
        if self.node(id).is_some_and(|n| n.parent.is_none()) || self.is_descendant(new_parent, id) {
            return false;
        }
        let pos = self.nodes.iter().position(|n| n.id == id).unwrap();
        let mut node = self.nodes.remove(pos);
        node.parent = Some(new_parent.to_string());
        let slot = self.insert_slot(new_parent, index);
        self.nodes.insert(slot.min(self.nodes.len()), node);
        true
    }

    /// Удалить поддерево; корень не удаляется. Возвращает число узлов.
    pub fn remove_subtree(&mut self, id: &str) -> usize {
        if self.node(id).is_none_or(|n| n.parent.is_none()) {
            return 0;
        }
        let doomed = self.subtree_ids(id);
        self.nodes.retain(|n| !doomed.contains(&n.id));
        self.links.retain(|l| !doomed.contains(&l.from) && !doomed.contains(&l.to));
        doomed.len()
    }

    pub fn add_link(&mut self, from: &str, to: &str, label: &str) -> bool {
        if from == to
            || self.node(from).is_none()
            || self.node(to).is_none()
            || self.links.iter().any(|l| l.from == from && l.to == to)
        {
            return false;
        }
        self.links.push(CrossLink { from: from.to_string(), to: to.to_string(), label: label.to_string(), dash: default_link_dash() });
        true
    }

    pub fn remove_link(&mut self, from: &str, to: &str) -> bool {
        let before = self.links.len();
        self.links.retain(|l| !(l.from == from && l.to == to));
        before != self.links.len()
    }

    /// Снять ручные сдвиги — вернуться к автораскладке.
    pub fn reset_offsets(&mut self) {
        for n in &mut self.nodes {
            n.dx = 0.0;
            n.dy = 0.0;
        }
    }

    /// Карта из markdown-структуры: заголовок либо первый абзац — корень,
    /// списки (с вложенностью) и абзацы — узлы. Без текста корень —
    /// `default_root`.
    pub fn from_outline(md: &str, default_root: &str) -> Self {
        use syngui::widgets::input::document_editor::{parse_document, BlockKind, DocBlock};

        fn text_of(b: &DocBlock) -> String {
            match &b.kind {
                BlockKind::Heading { text, .. } => text.text(),
                BlockKind::Toggle { summary, .. } => summary.text(),
                BlockKind::Callout { title, .. } => title.text(),
                BlockKind::CodeBlock { code, .. } => code.lines().next().unwrap_or("").to_string(),
                other => other.text().map(|t| t.text()).unwrap_or_default(),
            }
            .trim()
            .to_string()
        }

        fn add_blocks(doc: &mut MindmapDoc, parent: &str, blocks: &[DocBlock]) {
            for b in blocks {
                let text = text_of(b);
                let children = b.kind.children();
                if text.is_empty() {
                    if let Some(c) = children {
                        add_blocks(doc, parent, c);
                    }
                    continue;
                }
                let id = doc.add_node(parent, &text, None).unwrap_or_default();
                if let BlockKind::Todo { checked: true, .. } = &b.kind {
                    if let Some(n) = doc.node_mut(&id) {
                        n.icon = "✓".to_string();
                    }
                }
                if let Some(c) = children {
                    add_blocks(doc, &id, c);
                }
            }
        }

        let model = parse_document(md);
        let mut blocks = model.blocks.as_slice();
        let root_text = match blocks.first() {
            Some(b) if matches!(b.kind, BlockKind::Heading { .. } | BlockKind::Paragraph(_)) && !text_of(b).is_empty() => {
                let t = text_of(b);
                blocks = &blocks[1..];
                t
            }
            _ => default_root.to_string(),
        };
        let mut doc = MindmapDoc::template(&root_text);
        let root = doc.root_id();
        add_blocks(&mut doc, &root, blocks);
        doc
    }

    /// Обратная операция: `# корень` + вложенный список.
    pub fn to_outline(&self) -> String {
        fn walk(doc: &MindmapDoc, id: &str, depth: usize, out: &mut String) {
            for c in doc.children_of(id) {
                out.push_str(&"  ".repeat(depth));
                out.push_str("- ");
                if !c.icon.is_empty() {
                    out.push_str(&c.icon);
                    out.push(' ');
                }
                out.push_str(c.text.trim());
                out.push('\n');
                walk(doc, &c.id, depth + 1, out);
            }
        }
        let mut out = String::new();
        if let Some(root) = self.root() {
            if !root.text.trim().is_empty() {
                out.push_str(&format!("# {}\n\n", root.text.trim()));
            }
            walk(self, &root.id, 0, &mut out);
        }
        out
    }

    /// Текст с отступами (для агента и отладки): `id "текст" [+]`.
    pub fn tree_text(&self) -> String {
        fn walk(doc: &MindmapDoc, id: &str, depth: usize, out: &mut String) {
            let Some(n) = doc.node(id) else { return };
            out.push_str(&"  ".repeat(depth));
            out.push_str(&format!("{} \"{}\"", n.id, n.text));
            if !n.icon.is_empty() {
                out.push_str(&format!(" icon {}", n.icon));
            }
            if n.collapsed {
                out.push_str(" [collapsed]");
            }
            if let Some(l) = &n.link {
                out.push_str(&format!(" → page {l}"));
            }
            if !n.note.is_empty() {
                out.push_str(&format!(" · note {} chars", n.note.chars().count()));
            }
            out.push('\n');
            for c in doc.children_of(id) {
                walk(doc, &c.id, depth + 1, out);
            }
        }
        let mut out = String::new();
        if let Some(root) = self.root() {
            walk(self, &root.id, 0, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_defaults_and_tree_ops() {
        let mut d = MindmapDoc::template("Проект");
        let root = d.root_id();
        let a = d.add_node(&root, "A", None).unwrap();
        let b = d.add_node(&root, "B", None).unwrap();
        let a1 = d.add_node(&a, "A1", None).unwrap();
        let first = d.add_node(&root, "Первый", Some(0)).unwrap();
        assert_eq!(d.children_of(&root).iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["Первый", "A", "B"]);
        assert_eq!(d.depth_of(&a1), 2);
        assert_eq!(d.subtree_ids(&a), vec![a.clone(), a1.clone()]);
        assert!(d.is_descendant(&a1, &root));
        assert!(!d.move_node(&a, &a1, None), "в своё поддерево нельзя");
        assert!(!d.move_node(&root, &a, None), "корень не переносится");
        assert!(d.move_node(&b, &a, Some(0)));
        assert_eq!(d.children_of(&a).iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["B", "A1"]);
        assert!(d.add_link(&a1, &first, "см."));
        assert!(!d.add_link(&a1, &first, ""), "дубль");
        let json = d.serialize();
        assert!(!json.contains("\"dx\""), "нулевые сдвиги не пишутся: {json}");
        assert!(!json.contains("\"shape\""), "auto не пишется");
        let back = MindmapDoc::parse(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(d.remove_subtree(&a), 3);
        assert!(d.links.is_empty(), "ссылка удалённого узла уходит с ним");
        assert_eq!(d.remove_subtree(&root), 0);
    }

    #[test]
    fn sanitize_repairs_orphans_and_cycles() {
        let json = r#"{"nodes":[{"id":"r","text":"root"},{"id":"x","parent":"нет","text":"x"},{"id":"a","parent":"b","text":"a"},{"id":"b","parent":"a","text":"b"}],
            "links":[{"from":"r","to":"zzz"},{"from":"a","to":"a"}],"style":{"font_size":900,"palette":[]},"view":{"zoom":0}}"#;
        let d = MindmapDoc::parse(json).unwrap();
        assert_eq!(d.node("x").unwrap().parent.as_deref(), Some("r"));
        assert!(!d.is_descendant("a", "a") || !d.is_descendant("b", "b"), "цикл разорван");
        assert!(d.links.is_empty());
        assert_eq!(d.style.font_size, 13.0);
        assert_eq!(d.style.palette.len(), 6);
        assert_eq!(d.view.zoom, 1.0);
        assert_eq!(d.depth_of("a") + d.depth_of("b"), 3);
    }

    #[test]
    fn outline_roundtrip() {
        let md = "# План\n\n- Идеи\n  - [x] Готово\n  - Позже\n- Сроки\n\nАбзац тоже узел\n";
        let d = MindmapDoc::from_outline(md, "Карта");
        assert_eq!(d.root().unwrap().text, "План");
        let root = d.root_id();
        let kids: Vec<&str> = d.children_of(&root).iter().map(|n| n.text.as_str()).collect();
        assert_eq!(kids, ["Идеи", "Сроки", "Абзац тоже узел"]);
        let ideas = d.children_of(&root)[0].id.clone();
        let sub: Vec<(&str, &str)> = d.children_of(&ideas).iter().map(|n| (n.text.as_str(), n.icon.as_str())).collect();
        assert_eq!(sub, [("Готово", "✓"), ("Позже", "")]);
        let out = d.to_outline();
        assert_eq!(out, "# План\n\n- Идеи\n  - ✓ Готово\n  - Позже\n- Сроки\n- Абзац тоже узел\n");
        let again = MindmapDoc::from_outline(&out, "Карта");
        assert_eq!(again.to_outline(), out);
        // Без заголовка — корень по умолчанию.
        let d = MindmapDoc::from_outline("- один\n- два\n", "Карта");
        assert_eq!(d.root().unwrap().text, "Карта");
        assert_eq!(d.children_of(&d.root_id()).len(), 2);
    }
}
