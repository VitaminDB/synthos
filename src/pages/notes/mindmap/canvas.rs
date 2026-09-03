//! Холст интеллект-карты — собственный элемент на `DisplayList`/`CanvasContext`.
//!
//! Узлы и связи рисуются самим элементом по геометрии [`layout::compute`];
//! текст измеряется через `TextMeasure` дерева (после `mount`), до него —
//! оценкой по числу символов. Живёт внутри `PanZoomViewport`: события
//! приходят уже в мировых координатах, а размер вьюпорта — в constraints
//! (по нему считается «вписать»).
//!
//! Жесты: клик — выбор, двойной клик / F2 — правка на месте (над узлом
//! монтируется `TextField` — элемент управляет своими детьми, как редактор
//! документа блоками), drag узла на другой — переподвесить, в пустоту —
//! ручной сдвиг поддерева (с привязкой 5 px), Ctrl+клик по узлу со ссылкой
//! — открыть страницу, правый клик — выбор (меню открывает обёртка
//! `ContextMenu` в [`super::view`]). Клавиши (элемент берёт фокус по клику,
//! как текстовый ввод): Tab — дочерний, Enter — соседний, Delete — удалить
//! поддерево, Space — свернуть/развернуть, стрелки — по дереву, Esc — снять
//! выбор. Мутации уходят в ручку сразу (перестройка по `structure_rev`
//! сохраняет элемент — `can_update`), drag живёт на локальном предпросмотре
//! до MouseUp.
//!
//! Цвета — из MSS класса `notes-mindmap-canvas`: `color` (текст),
//! `border-color` (обводка/линии без цвета ветки), `accent-color` (корень,
//! выбор), `background-color` (фон без своего цвета в стиле карты).

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::containers::Positioned;
use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, Key, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields, TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, TextMeasure, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, LayoutHint};
use syngui::widgets::input::document_editor::shape::{arrow_head, stroke_path};

use super::layout::{self, MapGeometry, Metrics, Side};
use super::model::{Curve, MindmapDoc, NodeShape};
use super::{MindmapHandle, OpenPage, ZOOM_MAX, ZOOM_MIN};

/// Порог, после которого нажатие считается переносом.
const DRAG_THRESHOLD: f32 = 4.0;
/// Привязка ручного сдвига.
const SNAP: f32 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Радиус значка «+N» у свёрнутого узла.
const BADGE_R: f32 = 8.0;

pub struct MindmapCanvas {
    pub handle: MindmapHandle,
    pub open_page: OpenPage,
    pub editing: Option<String>,
    pub selected: Option<String>,
    /// Счётчик «вписать» (тулбар).
    pub fit: u64,
}

impl Widget for MindmapCanvas {
    fn create_element(&self) -> Box<dyn Element> {
        let doc = self.handle.lock().clone();
        let mut el = MindmapElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
            handle: self.handle.clone(),
            open_page: self.open_page.clone(),
            doc,
            rev: self.handle.revision.get_untracked(),
            geom: MapGeometry::default(),
            tm: None,
            selected: self.selected.clone(),
            editing: self.editing.clone(),
            rebuild: self.editing.is_some(),
            drag: None,
            hover: None,
            last_click: None,
            viewport: Size::zero(),
            fit_seen: self.fit,
            fit_pending: false,
            focused: false,
            focus_request: false,
        };
        el.relayout();
        Box::new(el)
    }

    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct Drag {
    idx: usize,
    start: Point,
    delta: Point,
    moved: bool,
    /// Узел под курсором — будущий родитель.
    target: Option<usize>,
}

pub struct MindmapElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
    handle: MindmapHandle,
    open_page: OpenPage,
    doc: MindmapDoc,
    rev: u64,
    geom: MapGeometry,
    tm: Option<Arc<dyn TextMeasure>>,
    selected: Option<String>,
    editing: Option<String>,
    rebuild: bool,
    drag: Option<Drag>,
    hover: Option<usize>,
    last_click: Option<(Instant, String)>,
    viewport: Size,
    fit_seen: u64,
    fit_pending: bool,
    focused: bool,
    focus_request: bool,
}

impl MindmapElement {
    fn measure_text(&self, s: &str, font: f32, bold: bool) -> f32 {
        match &self.tm {
            Some(tm) => tm.measure_text_width_styled(s, font, s.chars().count(), bold, None),
            None => s.chars().count() as f32 * font * 0.58,
        }
    }

    fn relayout(&mut self) {
        let style = self.doc.style.clone();
        let measure = |s: &str, font: f32, bold: bool| self.measure_text(s, font, bold);
        let m = Metrics {
            measure: &measure,
            font_size: style.font_size,
            bold: style.weight == "bold",
            padding: style.padding,
            max_w: style.max_node_w,
            show_icons: style.show_icons,
        };
        self.geom = layout::compute(&self.doc, &self.doc.layout, &m);
    }

    fn text_color(&self) -> Color {
        color_of(&self.doc.style.text_color).or(self.mss.color).unwrap_or_else(|| Color::from_hex("#E6E8EE"))
    }

    fn accent(&self) -> Color {
        self.mss.accent_color.unwrap_or_else(|| Color::from_hex("#4F8CFF"))
    }

    fn muted(&self) -> Color {
        self.mss.border_color.unwrap_or_else(|| Color::from_hex("#8B95A6"))
    }

    fn palette_color(&self, k: usize) -> Color {
        let p = &self.doc.style.palette;
        if p.is_empty() {
            return self.accent();
        }
        color_of(&p[k % p.len()]).unwrap_or_else(|| self.accent())
    }

    /// Цвет ветки узла: свой цвет — ближайший вверх по дереву, иначе цвет
    /// ветки по палитре; у корня — акцент.
    fn branch_color(&self, idx: usize) -> Color {
        let mut cur = Some(self.geom.nodes[idx].id.clone());
        let mut guard = 0;
        while let Some(id) = cur {
            if let Some(n) = self.doc.node(&id) {
                if let Some(c) = color_of(&n.color) {
                    return c;
                }
                cur = n.parent.clone();
            } else {
                break;
            }
            guard += 1;
            if guard > self.doc.nodes.len() {
                break;
            }
        }
        match self.geom.nodes[idx].branch {
            Some(k) => self.palette_color(k),
            None => self.accent(),
        }
    }

    fn resolved_shape(&self, idx: usize) -> NodeShape {
        let n = &self.geom.nodes[idx];
        let explicit = self.doc.node(&n.id).map(|x| x.shape).unwrap_or_default();
        match (explicit, n.level) {
            (NodeShape::Auto, 0) => NodeShape::Pill,
            (NodeShape::Auto, 1) => NodeShape::Rounded,
            (NodeShape::Auto, _) => NodeShape::Text,
            (s, _) => s,
        }
    }

    fn node_rect(&self, idx: usize) -> Rect {
        let mut r = self.geom.nodes[idx].rect;
        r.origin.x += self.bounds.origin.x;
        r.origin.y += self.bounds.origin.y;
        if let Some(d) = self.drag.as_ref().filter(|d| d.moved) {
            if idx == d.idx || self.in_subtree(idx, d.idx) {
                r.origin.x += d.delta.x;
                r.origin.y += d.delta.y;
            }
        }
        r
    }

    fn in_subtree(&self, idx: usize, root: usize) -> bool {
        let (a, b) = (&self.geom.nodes[idx].id, &self.geom.nodes[root].id);
        self.doc.is_descendant(a, b)
    }

    fn to_local(&self, p: Point) -> Point {
        Point::new(p.x - self.bounds.origin.x, p.y - self.bounds.origin.y)
    }

    fn hit(&self, p: Point) -> Option<usize> {
        self.geom.hit(self.to_local(p))
    }

    fn selected_idx(&self) -> Option<usize> {
        self.selected.as_deref().and_then(|id| self.geom.index_of(id))
    }

    /// Вписать карту во вьюпорт: масштаб и сдвиг — в сигналы PanZoom.
    fn fit_view(&mut self) {
        if self.viewport.width <= 1.0 || self.viewport.height <= 1.0 {
            self.fit_pending = true;
            return;
        }
        self.fit_pending = false;
        let (gw, gh) = (self.geom.size.width.max(1.0), self.geom.size.height.max(1.0));
        let zoom = (self.viewport.width / gw).min(self.viewport.height / gh).min(1.0).clamp(ZOOM_MIN, ZOOM_MAX);
        let pan = Point::new(
            ((self.viewport.width - gw * zoom) / 2.0).max(0.0),
            ((self.viewport.height - gh * zoom) / 2.0).max(0.0),
        );
        self.handle.zoom.set(zoom);
        self.handle.pan.set(pan);
    }

    fn navigate(&self, key: Key) -> Option<String> {
        let sel = self.selected_idx()?;
        let node = &self.geom.nodes[sel];
        let doc = &self.doc;
        let parent = doc.node(&node.id).and_then(|n| n.parent.clone());
        let siblings: Vec<String> = parent.as_deref().map(|p| doc.children_of(p).iter().map(|n| n.id.clone()).collect()).unwrap_or_default();
        let pos = siblings.iter().position(|s| *s == node.id);
        let first_child = doc.children_of(&node.id).first().map(|n| n.id.clone()).filter(|_| !doc.node(&node.id).is_some_and(|n| n.collapsed));
        let (toward_children, toward_parent, prev, next) = match node.side {
            Side::Right => (Key::Right, Key::Left, Key::Up, Key::Down),
            Side::Left => (Key::Left, Key::Right, Key::Up, Key::Down),
            Side::Down => (Key::Down, Key::Up, Key::Left, Key::Right),
        };
        if key == toward_children {
            return first_child;
        }
        if key == toward_parent {
            return parent;
        }
        if key == prev {
            return pos.and_then(|p| p.checked_sub(1)).map(|p| siblings[p].clone());
        }
        if key == next {
            return pos.map(|p| p + 1).filter(|&p| p < siblings.len()).map(|p| siblings[p].clone());
        }
        None
    }

    fn commit_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved {
            return;
        }
        let id = self.geom.nodes[drag.idx].id.clone();
        if let Some(t) = drag.target {
            let target = self.geom.nodes[t].id.clone();
            if self.handle.reparent(&id, &target, None) {
                return;
            }
        }
        let dx = (drag.delta.x / SNAP).round() * SNAP;
        let dy = (drag.delta.y / SNAP).round() * SNAP;
        self.handle.offset(&id, dx, dy);
    }

    fn edge_points(&self, e: &layout::Edge) -> (Point, Point, Point, Point) {
        let o = self.bounds.origin;
        let shift = |idx: usize| -> Point {
            match self.drag.as_ref().filter(|d| d.moved) {
                Some(d) if idx == d.idx || self.in_subtree(idx, d.idx) => d.delta,
                _ => Point::zero(),
            }
        };
        let (sa, sb) = (shift(e.from), shift(e.to));
        let a = Point::new(e.a.x + o.x + sa.x, e.a.y + o.y + sa.y);
        let b = Point::new(e.b.x + o.x + sb.x, e.b.y + o.y + sb.y);
        let (c1, c2) = if e.horizontal {
            let mid = (a.x + b.x) / 2.0;
            (Point::new(mid, a.y), Point::new(mid, b.y))
        } else {
            let mid = (a.y + b.y) / 2.0;
            (Point::new(a.x, mid), Point::new(b.x, mid))
        };
        (a, b, c1, c2)
    }

    fn draw_edges(&self, list: &mut DisplayList) {
        let style = &self.doc.style;
        let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
        c.set_stroke_width(style.line_width);
        for e in &self.geom.edges {
            let (a, b, c1, c2) = self.edge_points(e);
            let color = color_of(&style.line_color).unwrap_or_else(|| self.branch_color(e.to));
            c.set_color(color.with_alpha(0.9));
            let pts: Vec<(f32, f32)> = match self.doc.layout.curve {
                Curve::Straight => vec![(a.x, a.y), (b.x, b.y)],
                Curve::Elbow => {
                    if e.horizontal {
                        vec![(a.x, a.y), (c1.x, a.y), (c1.x, b.y), (b.x, b.y)]
                    } else {
                        vec![(a.x, a.y), (a.x, c1.y), (b.x, c1.y), (b.x, b.y)]
                    }
                }
                Curve::Bezier => cubic_points(a, c1, c2, b, 18),
            };
            stroke_path(&mut c, &pts, style.line_dash);
        }
        // Кросс-ссылки: пунктир от границы к границе, стрелка, подпись.
        for l in &self.doc.links {
            let (Some(fi), Some(ti)) = (self.geom.index_of(&l.from), self.geom.index_of(&l.to)) else { continue };
            let (fr, tr) = (self.node_rect(fi), self.node_rect(ti));
            let (fc, tc) = (center(fr), center(tr));
            let Some(p1) = exit_point(fr, fc, tc) else { continue };
            let Some(p2) = exit_point(tr, tc, fc) else { continue };
            let color = self.muted().with_alpha(0.95);
            c.set_color(color);
            c.set_stroke_width(1.5);
            // Лёгкая дуга, чтобы не сливаться с ветками.
            let (dx, dy) = (p2.x - p1.x, p2.y - p1.y);
            let n = Point::new(-dy, dx);
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let bow = (len * 0.15).min(60.0);
            let m = Point::new((p1.x + p2.x) / 2.0 + n.x / len * bow, (p1.y + p2.y) / 2.0 + n.y / len * bow);
            let pts = cubic_points(p1, m, m, p2, 16);
            stroke_path(&mut c, &pts, l.dash.max(0.0));
            if let (Some(last), Some(prev)) = (pts.last(), pts.get(pts.len().saturating_sub(2))) {
                let d = (last.0 - prev.0, last.1 - prev.1);
                let dl = (d.0 * d.0 + d.1 * d.1).sqrt().max(0.001);
                arrow_head(&mut c, *last, (d.0 / dl, d.1 / dl), 9.0, color);
            }
            if !l.label.trim().is_empty() {
                list.push_text_styled_singleline(
                    l.label.trim(),
                    Rect::new(Point::new(m.x - 60.0, m.y - 8.0), Size::new(120.0, 16.0)),
                    self.muted(),
                    10.0,
                    TextAlign::CENTER,
                    TextDecoration::None,
                    500,
                    None,
                );
            }
        }
        c.flush(list);
    }

    fn draw_node(&self, list: &mut DisplayList, idx: usize) {
        let style = &self.doc.style;
        let n = &self.geom.nodes[idx];
        let r = self.node_rect(idx);
        let branch = self.branch_color(idx);
        let shape = self.resolved_shape(idx);
        let level = n.level;
        let dragged = self.drag.as_ref().is_some_and(|d| d.moved && (idx == d.idx || self.in_subtree(idx, d.idx)));
        let alpha = if dragged { 0.55 } else { 1.0 };
        let is_target = self.drag.as_ref().and_then(|d| d.target) == Some(idx);
        let selected = self.selected.as_deref() == Some(n.id.as_str());

        // Заливка и обводка по форме.
        let fill = match (shape, level) {
            (NodeShape::Text, _) => None,
            (_, 0) => Some(branch),
            _ => Some(color_of(&style.node_fill).unwrap_or_else(|| branch.with_alpha(0.22))),
        };
        let stroke = match shape {
            NodeShape::Text => None,
            _ if level == 0 => None,
            _ => Some(color_of(&style.node_stroke).unwrap_or(branch)),
        };
        let radius = match shape {
            NodeShape::Rect => 0.0,
            NodeShape::Pill => r.size.height / 2.0,
            _ => style.radius,
        };
        if shape == NodeShape::Ellipse {
            let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
            let (cx, cy) = (r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0);
            let pts: Vec<(f32, f32)> = (0..48)
                .map(|i| {
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    (cx + t.cos() * r.size.width / 2.0, cy + t.sin() * r.size.height / 2.0)
                })
                .collect();
            if let Some(f) = fill {
                c.set_color(f.with_alpha(f.a * alpha));
                c.fill_polygon(&pts);
            }
            if let Some(s) = stroke {
                c.set_color(s.with_alpha(alpha));
                c.set_stroke_width(1.5);
                let mut closed = pts.clone();
                closed.push(pts[0]);
                c.draw_polyline(&closed);
            }
            c.flush(list);
        } else {
            if let Some(f) = fill {
                list.push_rect(r, f.with_alpha(f.a * alpha), [radius; 4]);
            }
            if let Some(s) = stroke {
                let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                c.set_color(s.with_alpha(alpha));
                c.set_stroke_width(1.5);
                c.draw_rect(r.origin.x, r.origin.y, r.size.width, r.size.height);
                c.flush(list);
            }
            if shape == NodeShape::Text {
                // Текст на линии: подчёркивание в цвет ветки.
                list.push_rect(
                    Rect::new(Point::new(r.origin.x, r.origin.y + r.size.height - 2.0), Size::new(r.size.width, 2.0)),
                    branch.with_alpha(0.9 * alpha),
                    [1.0; 4],
                );
            }
        }
        if self.hover == Some(idx) && !dragged {
            list.push_rect(r, self.text_color().with_alpha(0.06), [radius.max(4.0); 4]);
        }
        if selected || is_target {
            let ring = Rect::new(
                Point::new(r.origin.x - 3.0, r.origin.y - 3.0),
                Size::new(r.size.width + 6.0, r.size.height + 6.0),
            );
            let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
            c.set_color(if is_target { self.accent() } else { self.accent().with_alpha(if self.focused { 1.0 } else { 0.6 }) });
            c.set_stroke_width(2.0);
            c.draw_rect(ring.origin.x, ring.origin.y, ring.size.width, ring.size.height);
            c.flush(list);
        }

        // Текст: иконка + строки.
        let m = self.metrics_snapshot();
        let font = m.font_for(level);
        let bold = m.bold_for(level);
        let line_h = m.line_h(level);
        let text_color = if level == 0 && shape != NodeShape::Text {
            Color::from_hex("#FFFFFF").with_alpha(0.95 * alpha)
        } else {
            self.text_color().with_alpha(alpha)
        };
        let mut x = r.origin.x + style.padding;
        if n.has_icon {
            if let Some(node) = self.doc.node(&n.id) {
                list.push_text_styled_singleline(
                    node.icon.trim(),
                    Rect::new(Point::new(x, r.origin.y + style.padding), Size::new(m.icon_w(level), line_h)),
                    text_color,
                    font,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    400,
                    None,
                );
            }
            x += m.icon_w(level);
        }
        if self.editing.as_deref() != Some(n.id.as_str()) {
            for (i, line) in n.lines.iter().enumerate() {
                list.push_text_styled_singleline(
                    line,
                    Rect::new(
                        Point::new(x, r.origin.y + style.padding + i as f32 * line_h),
                        Size::new((r.size.width - (x - r.origin.x) - style.padding).max(4.0), line_h),
                    ),
                    text_color,
                    font,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    if bold { 700 } else { 400 },
                    None,
                );
            }
        }

        // Значок «+N» у свёрнутого узла — со стороны роста.
        if n.hidden > 0 {
            let cpt = match n.side {
                Side::Right => Point::new(r.origin.x + r.size.width + BADGE_R + 2.0, r.origin.y + r.size.height / 2.0),
                Side::Left => Point::new(r.origin.x - BADGE_R - 2.0, r.origin.y + r.size.height / 2.0),
                Side::Down => Point::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height + BADGE_R + 2.0),
            };
            list.push_rect(
                Rect::new(Point::new(cpt.x - BADGE_R, cpt.y - BADGE_R), Size::new(BADGE_R * 2.0, BADGE_R * 2.0)),
                branch.with_alpha(alpha),
                [BADGE_R; 4],
            );
            list.push_text_styled_singleline(
                &format!("+{}", n.hidden),
                Rect::new(Point::new(cpt.x - BADGE_R, cpt.y - 6.0), Size::new(BADGE_R * 2.0, 12.0)),
                Color::from_hex("#FFFFFF"),
                9.0,
                TextAlign::CENTER,
                TextDecoration::None,
                700,
                None,
            );
        }
        // Ссылка на страницу — точка в углу.
        if self.doc.node(&n.id).is_some_and(|x| x.link.is_some()) {
            list.push_rect(
                Rect::new(Point::new(r.origin.x + r.size.width - 5.0, r.origin.y - 3.0), Size::new(8.0, 8.0)),
                self.accent().with_alpha(alpha),
                [4.0; 4],
            );
        }
    }

    /// Метрики без измерителя (для геометрии текста при отрисовке).
    fn metrics_snapshot(&self) -> Metrics<'static> {
        fn zero(_: &str, _: f32, _: bool) -> f32 {
            0.0
        }
        let s = &self.doc.style;
        Metrics {
            measure: &zero,
            font_size: s.font_size,
            bold: s.weight == "bold",
            padding: s.padding,
            max_w: s.max_node_w,
            show_icons: s.show_icons,
        }
    }
}

fn color_of(hex: &str) -> Option<Color> {
    let t = hex.trim();
    (t.starts_with('#') && (t.len() == 7 || t.len() == 9)).then(|| Color::from_hex(t))
}

fn center(r: Rect) -> Point {
    Point::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0)
}

/// Точка выхода луча `from → to` из прямоугольника `r` (`from` внутри).
fn exit_point(r: Rect, from: Point, to: Point) -> Option<Point> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dx.abs() < 0.001 && dy.abs() < 0.001 {
        return None;
    }
    let mut t = f32::MAX;
    if dx.abs() > 0.001 {
        let edge = if dx > 0.0 { r.origin.x + r.size.width } else { r.origin.x };
        t = t.min((edge - from.x) / dx);
    }
    if dy.abs() > 0.001 {
        let edge = if dy > 0.0 { r.origin.y + r.size.height } else { r.origin.y };
        t = t.min((edge - from.y) / dy);
    }
    (t.is_finite() && t > 0.0).then(|| Point::new(from.x + dx * t, from.y + dy * t))
}

/// Кубическая Безье по точкам.
fn cubic_points(a: Point, c1: Point, c2: Point, b: Point, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let x = u * u * u * a.x + 3.0 * u * u * t * c1.x + 3.0 * u * t * t * c2.x + t * t * t * b.x;
            let y = u * u * u * a.y + 3.0 * u * u * t * c1.y + 3.0 * u * t * t * c2.y + t * t * t * b.y;
            (x, y)
        })
        .collect()
}

impl Element for MindmapElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<MindmapCanvas>() else { return };
        self.handle = w.handle.clone();
        self.open_page = w.open_page.clone();
        let rev = self.handle.revision.get_untracked();
        if rev != self.rev {
            self.rev = rev;
            self.doc = self.handle.lock().clone();
            self.relayout();
            self.hover = None;
            self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
            ctx.mark_layout_dirty();
        }
        if w.selected != self.selected {
            self.selected = w.selected.clone();
            self.mark_dirty(DirtyFlags::RENDER);
        }
        if w.editing != self.editing {
            if self.editing.is_some() && w.editing.is_none() {
                self.focus_request = true;
            }
            self.editing = w.editing.clone();
            self.rebuild = true;
            self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
            ctx.mark_layout_dirty();
        }
        if w.fit != self.fit_seen {
            self.fit_seen = w.fit;
            self.fit_view();
        }
    }

    fn mount(&mut self, tree: &mut ElementTree) {
        self.tm = tree.text_measure.clone();
        self.relayout();
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        let vw = if constraints.max_width.is_finite() { constraints.max_width } else { 0.0 };
        let vh = if constraints.max_height.is_finite() { constraints.max_height } else { 0.0 };
        if vw > 1.0 && vh > 1.0 {
            self.viewport = Size::new(vw, vh);
            if self.fit_pending {
                self.fit_view();
            }
        }
        // Холст не меньше вьюпорта: клик по пустому месту должен доходить
        // до элемента (снятие выбора).
        let size = Size::new(self.geom.size.width.max(vw), self.geom.size.height.max(vh));
        self.bounds.size = size;
        size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Stack { expand: false }
    }

    fn manages_own_children(&self) -> bool {
        true
    }

    fn needs_rebuild(&self) -> bool {
        self.rebuild
    }

    fn build_children(&self) -> Vec<Box<dyn Widget>> {
        let Some(id) = self.editing.as_deref() else { return Vec::new() };
        let Some(idx) = self.geom.index_of(id) else { return Vec::new() };
        let n = &self.geom.nodes[idx];
        let r = n.rect;
        let text = self.doc.node(id).map(|x| x.text.clone()).unwrap_or_default();
        let w = r.size.width.max(160.0);
        let h = r.size.height.max(30.0);
        let h_submit = self.handle.clone();
        let h_esc = self.handle.clone();
        let id_submit = id.to_string();
        let id_esc = id.to_string();
        vec![Box::new(
            Positioned::new(
                TextField::with_text(text)
                    .autofocus(true)
                    .submit_on_focus_lost(true)
                    .width(w)
                    .on_submit(move |t| h_submit.finish_editing(&id_submit, Some(t)))
                    .on_escape(move || h_esc.finish_editing(&id_esc, None))
                    .class("notes-mindmap-node-editor"),
            )
            .at(r.origin.x, r.origin.y)
            .size(Size::new(w, h)),
        )]
    }

    fn clear_rebuild(&mut self) {
        self.rebuild = false;
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_request)
    }

    fn wants_tab(&self) -> bool {
        true
    }

    fn accessibility_info(&self) -> Option<syngui::a11y::AccessibilityInfo> {
        // Роль текстового ввода — чтобы клик отдавал холсту фокус (клавиши
        // Tab/Enter/Delete) и снимал каретку с блока над врезкой.
        Some(syngui::a11y::AccessibilityInfo {
            role: syngui::a11y::Role::TextField,
            state: syngui::a11y::NodeState { focused: self.focused, ..Default::default() },
            properties: syngui::a11y::NodeProperties::default(),
        })
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        if let Some(bg) = color_of(&self.doc.style.bg).or(self.mss.background_color) {
            list.push_rect(self.bounds, bg, [0.0; 4]);
        }
        self.draw_edges(list);
        // Переносимое поддерево — поверх остальных.
        let dragged = self.drag.as_ref().filter(|d| d.moved).map(|d| d.idx);
        for idx in 0..self.geom.nodes.len() {
            if dragged.is_some_and(|d| idx == d || self.in_subtree(idx, d)) {
                continue;
            }
            self.draw_node(list, idx);
        }
        if let Some(d) = dragged {
            for idx in 0..self.geom.nodes.len() {
                if idx == d || self.in_subtree(idx, d) {
                    self.draw_node(list, idx);
                }
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                let Some(idx) = self.hit(*position) else {
                    if self.selected.is_some() {
                        self.handle.select(None);
                    }
                    return EventResult::Ignored;
                };
                let id = self.geom.nodes[idx].id.clone();
                if ctx.modifiers.ctrl {
                    if let Some(page) = self.doc.node(&id).and_then(|n| n.link.clone()) {
                        (self.open_page)(&page);
                        return EventResult::Handled;
                    }
                }
                let now = Instant::now();
                let double = self.last_click.as_ref().is_some_and(|(t, i)| *i == id && now.duration_since(*t) < DOUBLE_CLICK);
                self.last_click = Some((now, id.clone()));
                if double {
                    self.handle.start_editing(&id);
                    self.drag = None;
                    return EventResult::Handled;
                }
                self.handle.select(Some(id));
                self.drag = Some(Drag { idx, start: *position, delta: Point::zero(), moved: false, target: None });
                ctx.capture();
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            Event::MouseDown { button: MouseButton::Right, position } => {
                if let Some(idx) = self.hit(*position) {
                    let id = self.geom.nodes[idx].id.clone();
                    self.handle.select(Some(id));
                }
                // Само меню открывает обёртка ContextMenu.
                EventResult::Ignored
            }
            Event::MouseMove(position) => {
                if let Some(drag) = &mut self.drag {
                    drag.delta = Point::new(position.x - drag.start.x, position.y - drag.start.y);
                    if drag.delta.x.abs() > DRAG_THRESHOLD || drag.delta.y.abs() > DRAG_THRESHOLD {
                        drag.moved = true;
                    }
                    if drag.moved {
                        let idx = drag.idx;
                        let hit = self.geom.hit(self.to_local(*position));
                        let target = hit.filter(|&t| t != idx && !self.in_subtree(t, idx));
                        if let Some(d) = &mut self.drag {
                            d.target = target;
                        }
                        ctx.set_cursor(CursorIcon::Grabbing);
                        self.mark_dirty(DirtyFlags::RENDER);
                    }
                    return EventResult::Handled;
                }
                let inside = self.bounds.contains(*position);
                let hover = if inside { self.hit(*position) } else { None };
                if hover != self.hover {
                    self.hover = hover;
                    self.mark_dirty(DirtyFlags::RENDER);
                }
                if hover.is_some() {
                    ctx.set_cursor(CursorIcon::Pointer);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if self.drag.is_some() {
                    self.commit_drag();
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            Event::FocusGained => {
                self.focused = true;
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            Event::FocusLost => {
                self.focused = false;
                self.drag = None;
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            Event::KeyDown(key) => {
                if self.editing.is_some() {
                    return EventResult::Ignored;
                }
                let selected = self.selected.clone();
                let root = self.doc.root_id();
                match key {
                    Key::Tab => {
                        self.handle.add_child(selected.as_deref(), "");
                        EventResult::Handled
                    }
                    Key::Enter => {
                        match selected.as_deref() {
                            Some(id) if id != root => self.handle.add_sibling(id, ""),
                            _ => self.handle.add_child(Some(&root), ""),
                        };
                        EventResult::Handled
                    }
                    Key::F2 => {
                        if let Some(id) = selected {
                            self.handle.start_editing(&id);
                        }
                        EventResult::Handled
                    }
                    Key::Space => {
                        if let Some(id) = selected {
                            self.handle.toggle_collapsed(&id);
                        }
                        EventResult::Handled
                    }
                    Key::Delete | Key::Backspace => {
                        if let Some(id) = selected.filter(|s| *s != root) {
                            self.handle.delete(&id);
                        }
                        EventResult::Handled
                    }
                    Key::Escape => {
                        self.handle.select(None);
                        EventResult::Handled
                    }
                    Key::Left | Key::Right | Key::Up | Key::Down => {
                        if let Some(next) = self.navigate(*key) {
                            self.handle.select(Some(next));
                        }
                        EventResult::Handled
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }

    fn element_type_name(&self) -> &str {
        "notes-mindmap-canvas"
    }

    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, s: &ComputedStyle) {
        self.mss.apply(s);
        self.mark_dirty(DirtyFlags::RENDER);
    }

    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn set_content_size(&mut self, size: Size) {
        self.bounds.size = size;
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
}
