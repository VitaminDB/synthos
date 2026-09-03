//! Автораскладка интеллект-карты — чистая функция от документа и метрик
//! текста.
//!
//! Классическое дерево: размер узла — по строкам текста (перенос по
//! `max_node_w`), поперечный «габарит» поддерева — сумма габаритов детей,
//! дети стоят стопкой по центру родителя, следующий уровень — на `h_gap`
//! дальше по оси роста. Направления `right`/`left`/`down` — одна и та же
//! схема с разными осями, `both` делит детей корня по чётности на две
//! стороны, `radial` ставит первый уровень по кругу, а глубже растит
//! обычные деревья вправо/влево по знаку угла. Ручной сдвиг `dx/dy` узла
//! переносит всё его поддерево. Результат нормализован: минимум в
//! [`PAD`], размер — bbox плюс поля.

use std::collections::HashMap;

use syngui::core::{Point, Rect, Size};

use super::model::{Direction, MapLayout, MindmapDoc};

/// Поле вокруг карты.
pub const PAD: f32 = 24.0;

/// Куда от узла растут дети (сторона крепления линий и значка «+N»).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeBox {
    pub id: String,
    pub rect: Rect,
    pub level: usize,
    /// Строки текста после переноса.
    pub lines: Vec<String>,
    /// Индекс ветки (ребёнок корня, к которому относится узел); у корня — `None`.
    pub branch: Option<usize>,
    /// Сторона роста детей.
    pub side: Side,
    /// Скрытых потомков у свёрнутого узла.
    pub hidden: usize,
    pub has_icon: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// Индексы в `MapGeometry::nodes`.
    pub from: usize,
    pub to: usize,
    pub a: Point,
    pub b: Point,
    pub c1: Point,
    pub c2: Point,
    pub horizontal: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapGeometry {
    pub nodes: Vec<NodeBox>,
    pub edges: Vec<Edge>,
    pub size: Size,
}

impl MapGeometry {
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.index_of(id).map(|i| self.nodes[i].rect)
    }

    /// Узел под точкой (верхний по порядку отрисовки — последний).
    pub fn hit(&self, p: Point) -> Option<usize> {
        (0..self.nodes.len()).rev().find(|&i| self.nodes[i].rect.contains(p))
    }
}

/// Метрики текста для раскладки.
pub struct Metrics<'a> {
    /// Ширина строки: (текст, кегль, жирный).
    pub measure: &'a dyn Fn(&str, f32, bool) -> f32,
    pub font_size: f32,
    pub bold: bool,
    pub padding: f32,
    pub max_w: f32,
    pub show_icons: bool,
}

impl Metrics<'_> {
    pub fn font_for(&self, level: usize) -> f32 {
        if level == 0 {
            self.font_size + 2.0
        } else {
            self.font_size
        }
    }

    pub fn line_h(&self, level: usize) -> f32 {
        (self.font_for(level) * 1.35).round()
    }

    pub fn bold_for(&self, level: usize) -> bool {
        self.bold || level == 0
    }

    /// Ширина иконки-эмодзи перед текстом.
    pub fn icon_w(&self, level: usize) -> f32 {
        self.font_for(level) * 1.3
    }
}

/// Перенос текста по словам в ширину `max_w`.
pub fn wrap(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if line.is_empty() || measure(&candidate) <= max_w {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

struct Sized {
    id: String,
    level: usize,
    lines: Vec<String>,
    w: f32,
    h: f32,
    has_icon: bool,
    hidden: usize,
    children: Vec<usize>,
    dx: f32,
    dy: f32,
}

struct Builder<'a> {
    doc: &'a MindmapDoc,
    layout: &'a MapLayout,
    nodes: Vec<Sized>,
    rects: Vec<Rect>,
    sides: Vec<Side>,
    ext_cache: HashMap<(usize, bool), f32>,
}

impl Builder<'_> {
    /// Поперечный габарит поддерева: по высоте (`vertical=true`, дети
    /// стопкой) либо по ширине (дети в ряд, направление `down`).
    fn ext(&mut self, i: usize, vertical: bool) -> f32 {
        if let Some(v) = self.ext_cache.get(&(i, vertical)) {
            return *v;
        }
        let own = if vertical { self.nodes[i].h } else { self.nodes[i].w };
        let kids = self.nodes[i].children.clone();
        let value = if kids.is_empty() {
            own
        } else {
            let sum: f32 = kids.iter().map(|&c| self.ext(c, vertical)).sum::<f32>()
                + self.layout.v_gap * (kids.len() as f32 - 1.0);
            own.max(sum)
        };
        self.ext_cache.insert((i, vertical), value);
        value
    }

    /// Горизонтальный рост: `near_x` — край узла, обращённый к родителю,
    /// `cy` — центр по вертикали; `sign` +1 вправо, −1 влево.
    fn place_h(&mut self, i: usize, near_x: f32, cy: f32, sign: f32) {
        let (w, h) = (self.nodes[i].w, self.nodes[i].h);
        let x = if sign > 0.0 { near_x } else { near_x - w };
        self.rects[i] = Rect::new(Point::new(x, cy - h / 2.0), Size::new(w, h));
        self.sides[i] = if sign > 0.0 { Side::Right } else { Side::Left };
        let kids = self.nodes[i].children.clone();
        if kids.is_empty() {
            return;
        }
        let total: f32 = kids.iter().map(|&c| self.ext(c, true)).sum::<f32>()
            + self.layout.v_gap * (kids.len() as f32 - 1.0);
        let mut cursor = cy - total / 2.0;
        let child_near = if sign > 0.0 { x + w + self.layout.h_gap } else { x - self.layout.h_gap };
        for c in kids {
            let e = self.ext(c, true);
            self.place_h(c, child_near, cursor + e / 2.0, sign);
            cursor += e + self.layout.v_gap;
        }
    }

    /// Рост вниз: `cx` — центр по горизонтали, `top` — верхний край.
    fn place_v(&mut self, i: usize, cx: f32, top: f32) {
        let (w, h) = (self.nodes[i].w, self.nodes[i].h);
        self.rects[i] = Rect::new(Point::new(cx - w / 2.0, top), Size::new(w, h));
        self.sides[i] = Side::Down;
        let kids = self.nodes[i].children.clone();
        if kids.is_empty() {
            return;
        }
        let total: f32 = kids.iter().map(|&c| self.ext(c, false)).sum::<f32>()
            + self.layout.v_gap * (kids.len() as f32 - 1.0);
        let mut cursor = cx - total / 2.0;
        let child_top = top + h + self.layout.h_gap;
        for c in kids {
            let e = self.ext(c, false);
            self.place_v(c, cursor + e / 2.0, child_top);
            cursor += e + self.layout.v_gap;
        }
    }

    fn subtree_count(&self, i: usize) -> usize {
        1 + self.nodes[i].children.iter().map(|&c| self.subtree_count(c)).sum::<usize>()
    }

    fn place_root(&mut self) {
        let (w, h) = (self.nodes[0].w, self.nodes[0].h);
        match self.layout.direction {
            Direction::Right => self.place_h(0, 0.0, 0.0, 1.0),
            Direction::Left => self.place_h(0, 0.0, 0.0, -1.0),
            Direction::Down => self.place_v(0, 0.0, 0.0),
            Direction::Both => {
                self.rects[0] = Rect::new(Point::new(-w / 2.0, -h / 2.0), Size::new(w, h));
                self.sides[0] = Side::Right;
                let kids = self.nodes[0].children.clone();
                let right: Vec<usize> = kids.iter().copied().enumerate().filter(|(k, _)| k % 2 == 0).map(|(_, c)| c).collect();
                let left: Vec<usize> = kids.iter().copied().enumerate().filter(|(k, _)| k % 2 == 1).map(|(_, c)| c).collect();
                for (list, sign) in [(right, 1.0f32), (left, -1.0f32)] {
                    if list.is_empty() {
                        continue;
                    }
                    let total: f32 = list.iter().map(|&c| self.ext(c, true)).sum::<f32>()
                        + self.layout.v_gap * (list.len() as f32 - 1.0);
                    let mut cursor = -total / 2.0;
                    let near = if sign > 0.0 { w / 2.0 + self.layout.h_gap } else { -w / 2.0 - self.layout.h_gap };
                    for c in list {
                        let e = self.ext(c, true);
                        self.place_h(c, near, cursor + e / 2.0, sign);
                        cursor += e + self.layout.v_gap;
                    }
                }
            }
            Direction::Radial => {
                self.rects[0] = Rect::new(Point::new(-w / 2.0, -h / 2.0), Size::new(w, h));
                self.sides[0] = Side::Right;
                let kids = self.nodes[0].children.clone();
                if kids.is_empty() {
                    return;
                }
                let weights: Vec<f32> = kids.iter().map(|&c| self.subtree_count(c) as f32).collect();
                let total: f32 = weights.iter().sum();
                let radius = (w.max(h) / 2.0 + self.layout.h_gap * 2.0).max(90.0);
                let mut angle = -std::f32::consts::FRAC_PI_2;
                for (k, &c) in kids.iter().enumerate() {
                    let span = std::f32::consts::TAU * weights[k] / total;
                    let a = angle + span / 2.0;
                    angle += span;
                    let (cw, ch) = (self.nodes[c].w, self.nodes[c].h);
                    let cx = a.cos() * (radius + cw / 2.0);
                    let cy = a.sin() * (radius + ch / 2.0);
                    let sign = if a.cos() >= 0.0 { 1.0 } else { -1.0 };
                    let near = if sign > 0.0 { cx - cw / 2.0 } else { cx + cw / 2.0 };
                    self.place_h(c, near, cy, sign);
                }
            }
        }
    }

    /// Ручные сдвиги: поддерево едет вместе с узлом.
    fn apply_offsets(&mut self, i: usize, off: Point) {
        let off = Point::new(off.x + self.nodes[i].dx, off.y + self.nodes[i].dy);
        self.rects[i].origin.x += off.x;
        self.rects[i].origin.y += off.y;
        for c in self.nodes[i].children.clone() {
            self.apply_offsets(c, off);
        }
    }
}

/// Раскладка видимой части дерева.
pub fn compute(doc: &MindmapDoc, layout: &MapLayout, m: &Metrics) -> MapGeometry {
    let Some(root) = doc.root() else { return MapGeometry::default() };

    // Видимые узлы в порядке DFS, корень первый.
    let mut nodes: Vec<Sized> = Vec::new();
    let mut order: Vec<(String, usize, Option<usize>)> = vec![(root.id.clone(), 0, None)];
    let mut index_of: HashMap<String, usize> = HashMap::new();
    while let Some((id, level, parent)) = order.pop() {
        let Some(n) = doc.node(&id) else { continue };
        let i = nodes.len();
        index_of.insert(id.clone(), i);
        let has_icon = m.show_icons && !n.icon.trim().is_empty();
        let font = m.font_for(level);
        let bold = m.bold_for(level);
        let icon_w = if has_icon { m.icon_w(level) } else { 0.0 };
        let text_max = (m.max_w - 2.0 * m.padding - icon_w).max(40.0);
        let lines = wrap(n.text.trim(), text_max, |s| (m.measure)(s, font, bold));
        let text_w = lines.iter().map(|l| (m.measure)(l, font, bold)).fold(0.0, f32::max);
        let w = (text_w + icon_w + 2.0 * m.padding).max(if level == 0 { 60.0 } else { 36.0 });
        let h = lines.len() as f32 * m.line_h(level) + 2.0 * m.padding;
        let hidden = if n.collapsed { doc.subtree_ids(&id).len().saturating_sub(1) } else { 0 };
        nodes.push(Sized { id: id.clone(), level, lines, w, h, has_icon, hidden, children: Vec::new(), dx: n.dx, dy: n.dy });
        if let Some(p) = parent {
            nodes[p].children.push(i);
        }
        if !n.collapsed {
            for c in doc.children_of(&id).iter().rev() {
                order.push((c.id.clone(), level + 1, Some(i)));
            }
        }
    }
    // DFS со стеком выдаёт детей в обратном порядке вставки — вернём прямой.
    for n in &mut nodes {
        n.children.sort_unstable();
    }

    let count = nodes.len();
    let mut b = Builder {
        doc,
        layout,
        nodes,
        rects: vec![Rect::zero(); count],
        sides: vec![Side::Right; count],
        ext_cache: HashMap::new(),
    };
    b.place_root();
    b.apply_offsets(0, Point::zero());
    let _ = b.doc;

    // Нормализация: минимум в PAD.
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for r in &b.rects {
        min_x = min_x.min(r.origin.x);
        min_y = min_y.min(r.origin.y);
        max_x = max_x.max(r.origin.x + r.size.width);
        max_y = max_y.max(r.origin.y + r.size.height);
    }
    let shift = Point::new(PAD - min_x, PAD - min_y);
    for r in &mut b.rects {
        r.origin.x += shift.x;
        r.origin.y += shift.y;
    }
    let size = Size::new(max_x - min_x + 2.0 * PAD, max_y - min_y + 2.0 * PAD);

    // Ветка каждого узла — ребёнок корня на пути к нему.
    let mut branch: Vec<Option<usize>> = vec![None; count];
    for (k, &c) in b.nodes[0].children.iter().enumerate() {
        let mut stack = vec![c];
        while let Some(i) = stack.pop() {
            branch[i] = Some(k);
            stack.extend(b.nodes[i].children.iter().copied());
        }
    }

    // Рёбра: точки крепления по стороне роста.
    let mut edges = Vec::new();
    for i in 0..count {
        for &c in &b.nodes[i].children {
            let (pr, cr) = (b.rects[i], b.rects[c]);
            let horizontal = match layout.direction {
                Direction::Down => false,
                Direction::Radial if i == 0 => {
                    let dx = (cr.origin.x + cr.size.width / 2.0) - (pr.origin.x + pr.size.width / 2.0);
                    let dy = (cr.origin.y + cr.size.height / 2.0) - (pr.origin.y + pr.size.height / 2.0);
                    dx.abs() >= dy.abs()
                }
                _ => true,
            };
            let (a, bp) = if horizontal {
                let to_right = cr.origin.x + cr.size.width / 2.0 >= pr.origin.x + pr.size.width / 2.0;
                if to_right {
                    (
                        Point::new(pr.origin.x + pr.size.width, pr.origin.y + pr.size.height / 2.0),
                        Point::new(cr.origin.x, cr.origin.y + cr.size.height / 2.0),
                    )
                } else {
                    (
                        Point::new(pr.origin.x, pr.origin.y + pr.size.height / 2.0),
                        Point::new(cr.origin.x + cr.size.width, cr.origin.y + cr.size.height / 2.0),
                    )
                }
            } else {
                let below = cr.origin.y >= pr.origin.y + pr.size.height / 2.0;
                if below {
                    (
                        Point::new(pr.origin.x + pr.size.width / 2.0, pr.origin.y + pr.size.height),
                        Point::new(cr.origin.x + cr.size.width / 2.0, cr.origin.y),
                    )
                } else {
                    (
                        Point::new(pr.origin.x + pr.size.width / 2.0, pr.origin.y),
                        Point::new(cr.origin.x + cr.size.width / 2.0, cr.origin.y + cr.size.height),
                    )
                }
            };
            let (c1, c2) = if horizontal {
                let mid = (a.x + bp.x) / 2.0;
                (Point::new(mid, a.y), Point::new(mid, bp.y))
            } else {
                let mid = (a.y + bp.y) / 2.0;
                (Point::new(a.x, mid), Point::new(bp.x, mid))
            };
            edges.push(Edge { from: i, to: c, a, b: bp, c1, c2, horizontal });
        }
    }

    let nodes = b
        .nodes
        .into_iter()
        .enumerate()
        .map(|(i, n)| NodeBox {
            id: n.id,
            rect: b.rects[i],
            level: n.level,
            lines: n.lines,
            branch: branch[i],
            side: b.sides[i],
            hidden: n.hidden,
            has_icon: n.has_icon,
        })
        .collect();
    MapGeometry { nodes, edges, size }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(s: &str, font: f32, _bold: bool) -> f32 {
        s.chars().count() as f32 * font * 0.6
    }

    fn metrics<'a>(measure: &'a dyn Fn(&str, f32, bool) -> f32) -> Metrics<'a> {
        Metrics { measure, font_size: 10.0, bold: false, padding: 5.0, max_w: 200.0, show_icons: true }
    }

    fn sample() -> MindmapDoc {
        let mut d = MindmapDoc::template("R");
        let r = d.root_id();
        let a = d.add_node(&r, "A", None).unwrap();
        d.add_node(&r, "B", None);
        d.add_node(&a, "A1", None);
        d.add_node(&a, "A2", None);
        d
    }

    #[test]
    fn right_layout_centers_children_on_parent() {
        let d = sample();
        let g = compute(&d, &d.layout, &metrics(&mono));
        assert_eq!(g.nodes.len(), 5);
        let r = g.rect_of(&d.root_id()).unwrap();
        let a = g.nodes[1].rect;
        let b = g.nodes[4].rect;
        assert!(a.origin.x > r.origin.x + r.size.width, "дети правее корня");
        let cy = |x: Rect| x.origin.y + x.size.height / 2.0;
        assert!(cy(a) < cy(r) && cy(r) < cy(b), "корень между детьми");
        // Листья A1/A2 одинаковой высоты — A ровно посередине между ними.
        let (a1, a2) = (g.nodes[2].rect, g.nodes[3].rect);
        assert!((cy(a) - (cy(a1) + cy(a2)) / 2.0).abs() < 0.5, "A по центру своих детей");
        assert!(a1.origin.y + a1.size.height <= a2.origin.y, "дети не пересекаются");
        assert!(g.nodes[2].rect.origin.x > a.origin.x + a.size.width, "внуки дальше");
        assert_eq!(g.edges.len(), 4);
        assert!(g.edges.iter().all(|e| e.horizontal));
        assert!(g.nodes.iter().all(|n| n.rect.origin.x >= PAD - 0.01 && n.rect.origin.y >= PAD - 0.01));
        assert_eq!(g.nodes[2].branch, Some(0));
        assert_eq!(g.nodes[4].branch, Some(1));
        assert_eq!(g.nodes[0].branch, None);
    }

    #[test]
    fn both_splits_and_down_stacks_horizontally() {
        let mut d = sample();
        d.layout.direction = Direction::Both;
        let g = compute(&d, &d.layout, &metrics(&mono));
        let r = g.rect_of(&d.root_id()).unwrap();
        let a = g.nodes[1].rect;
        let b = g.nodes[4].rect;
        assert!(a.origin.x > r.origin.x + r.size.width, "A справа");
        assert!(b.origin.x + b.size.width < r.origin.x, "B слева");
        assert_eq!(g.nodes[4].side, Side::Left);
        d.layout.direction = Direction::Down;
        let g = compute(&d, &d.layout, &metrics(&mono));
        let r = g.rect_of(&d.root_id()).unwrap();
        let a = g.nodes[1].rect;
        let b = g.nodes[4].rect;
        assert!(a.origin.y > r.origin.y + r.size.height && (a.origin.y - b.origin.y).abs() < 0.5, "дети ниже, в ряд");
        assert!(a.origin.x + a.size.width <= b.origin.x, "без пересечений");
        assert!(g.edges.iter().all(|e| !e.horizontal));
    }

    #[test]
    fn collapsed_hides_subtree_and_offsets_move_it() {
        let mut d = sample();
        let a = d.nodes[1].id.clone();
        d.node_mut(&a).unwrap().collapsed = true;
        let g = compute(&d, &d.layout, &metrics(&mono));
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.nodes[1].hidden, 2);
        d.node_mut(&a).unwrap().collapsed = false;
        let before = compute(&d, &d.layout, &metrics(&mono));
        d.node_mut(&a).unwrap().dy = 100.0;
        let after = compute(&d, &d.layout, &metrics(&mono));
        // Размер вырос на сдвиг, поддерево A ушло вниз вместе с A.
        let da = after.nodes[1].rect.origin.y - before.nodes[1].rect.origin.y;
        let da1 = after.nodes[2].rect.origin.y - before.nodes[2].rect.origin.y;
        assert!((da - da1).abs() < 0.01, "A1 едет вместе с A");
        assert!(after.size.height > before.size.height);
    }

    #[test]
    fn radial_places_first_level_around_root_without_overlap() {
        let mut d = MindmapDoc::template("Центр");
        let r = d.root_id();
        for i in 0..5 {
            d.add_node(&r, &format!("Узел {i}"), None);
        }
        d.layout.direction = Direction::Radial;
        let g = compute(&d, &d.layout, &metrics(&mono));
        let root = g.nodes[0].rect;
        for i in 1..g.nodes.len() {
            let n = g.nodes[i].rect;
            assert!(!rects_overlap(root, n), "узел {i} накрывает корень");
            for j in 1..g.nodes.len() {
                if i != j {
                    assert!(!rects_overlap(n, g.nodes[j].rect), "узлы {i} и {j} пересекаются");
                }
            }
        }
    }

    fn rects_overlap(a: Rect, b: Rect) -> bool {
        a.origin.x < b.origin.x + b.size.width
            && b.origin.x < a.origin.x + a.size.width
            && a.origin.y < b.origin.y + b.size.height
            && b.origin.y < a.origin.y + a.size.height
    }

    #[test]
    fn wrap_breaks_long_text() {
        let lines = wrap("один два три четыре", 60.0, |s| s.chars().count() as f32 * 6.0);
        assert_eq!(lines, ["один два", "три четыре"]);
        let lines = wrap("один два три четыре", 40.0, |s| s.chars().count() as f32 * 6.0);
        assert_eq!(lines, ["один", "два", "три", "четыре"]);
        assert_eq!(wrap("", 60.0, |_| 0.0), [""]);
    }
}
