//! Граф wiki-связей vault'а: force-layout (Фрухтерман–Рейнгольд) в
//! `Element::animate`, рендер узлов и рёбер на CanvasContext.
//!
//! Полный режим — страница-плитка внутри PanZoomViewport (drag узлов,
//! клик открывает страницу); мини-режим — вкладка «Связи»: локальная
//! окрестность активной страницы (≤2 хопа), вписанная в свои bounds.
//! Наивный O(n²) с ранней остановкой — до пары тысяч страниц хватает.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};
use syngui::containers::PanZoomViewport;

use super::state::NotesCtx;
use super::storage;

#[derive(Clone, Copy, PartialEq)]
pub enum GraphMode {
    Full,
    /// Окрестность страницы (центр фиксируется в середине).
    Mini,
}

/// Страница-граф: вьюпорт с пан/зумом.
pub fn page(ctx: NotesCtx) -> impl Widget {
    let pan = use_signal(Point::zero());
    let zoom = use_signal(1.0f32);
    PanZoomViewport::new()
        .pan(pan)
        .zoom(zoom)
        .zoom_range(0.3, 3.0)
        .grid(false)
        .child(GraphView { ctx, mode: GraphMode::Full, center: None })
        .class("notes-graph-viewport")
}

/// Мини-граф текущей страницы для вкладки «Связи».
pub fn mini(ctx: NotesCtx, center: String) -> impl Widget {
    GraphView { ctx, mode: GraphMode::Mini, center: Some(center) }
}

pub struct GraphView {
    ctx: NotesCtx,
    mode: GraphMode,
    center: Option<String>,
}

struct SimNode {
    rel: String,
    title: String,
    pos: Point,
    degree: usize,
    is_center: bool,
}

pub struct GraphElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    ctx: NotesCtx,
    mode: GraphMode,
    center: Option<String>,
    nodes: Vec<SimNode>,
    /// Индексы пар рёбер.
    edges: Vec<(usize, usize)>,
    temperature: f32,
    hover: Option<usize>,
    drag: Option<(usize, Point, bool)>,
    /// Отпечаток данных индекса — для рестарта симуляции.
    data_fp: u64,
}

impl Widget for GraphView {
    fn create_element(&self) -> Box<dyn Element> {
        let mut el = GraphElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            ctx: self.ctx,
            mode: self.mode,
            center: self.center.clone(),
            nodes: Vec::new(),
            edges: Vec::new(),
            temperature: 0.0,
            hover: None,
            drag: None,
            data_fp: 0,
        };
        el.rebuild_data();
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

impl GraphElement {
    fn world_size(&self) -> Size {
        match self.mode {
            GraphMode::Full => Size::new(2200.0, 1500.0),
            GraphMode::Mini => self.bounds.size,
        }
    }

    /// Пересборка узлов/рёбер из индекса; позиции — детерминированный круг.
    fn rebuild_data(&mut self) {
        let index = self.ctx.index.get_untracked();
        let mut pages: Vec<String> = index.pages();
        let mut edge_pairs: Vec<(String, String)> = Vec::new();
        for rel in &pages {
            for to in index.outgoing_of(rel) {
                edge_pairs.push((rel.clone(), to));
            }
        }

        // Мини-режим: окрестность центра ≤ 2 хопа.
        if let Some(center) = &self.center {
            let mut keep: HashSet<String> = HashSet::new();
            keep.insert(center.clone());
            for _ in 0..2 {
                let snapshot: Vec<String> = keep.iter().cloned().collect();
                for (a, b) in &edge_pairs {
                    for k in &snapshot {
                        if a == k {
                            keep.insert(b.clone());
                        }
                        if b == k {
                            keep.insert(a.clone());
                        }
                    }
                }
            }
            pages.retain(|p| keep.contains(p));
            edge_pairs.retain(|(a, b)| keep.contains(a) && keep.contains(b));
        }

        let fp = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            pages.hash(&mut h);
            edge_pairs.hash(&mut h);
            h.finish()
        };
        if fp == self.data_fp && !self.nodes.is_empty() {
            return;
        }
        self.data_fp = fp;

        let world = self.world_size();
        let cx = world.width / 2.0;
        let cy = world.height / 2.0;
        let r = (world.width.min(world.height) * 0.35).max(60.0);
        let n = pages.len().max(1);
        let index_of: HashMap<&String, usize> =
            pages.iter().enumerate().map(|(i, p)| (p, i)).collect();

        // Сохраняем позиции уже известных узлов (плавное обновление).
        let old: HashMap<String, Point> =
            self.nodes.iter().map(|n| (n.rel.clone(), n.pos)).collect();

        self.nodes = pages
            .iter()
            .enumerate()
            .map(|(i, rel)| {
                let angle = i as f32 / n as f32 * std::f32::consts::TAU;
                let default = Point::new(cx + r * angle.cos(), cy + r * angle.sin());
                SimNode {
                    title: storage::title_of(rel),
                    pos: old.get(rel).copied().unwrap_or(default),
                    degree: 0,
                    is_center: self.center.as_deref() == Some(rel.as_str()),
                    rel: rel.clone(),
                }
            })
            .collect();
        self.edges = edge_pairs
            .iter()
            .filter_map(|(a, b)| Some((*index_of.get(a)?, *index_of.get(b)?)))
            .filter(|(a, b)| a != b)
            .collect();
        for (a, b) in &self.edges {
            self.nodes[*a].degree += 1;
            self.nodes[*b].degree += 1;
        }
        // Центр мини-графа прибит к середине.
        if let Some(c) = self.nodes.iter_mut().find(|n| n.is_center) {
            c.pos = Point::new(cx, cy);
        }
        self.temperature = 1.0;
    }

    /// Один шаг force-layout.
    fn step(&mut self) {
        let world = self.world_size();
        let n = self.nodes.len();
        if n < 2 {
            self.temperature = 0.0;
            return;
        }
        let area = world.width * world.height;
        let k = (area / n as f32).sqrt() * 0.6;
        let mut disp = vec![Point::zero(); n];

        // Отталкивание.
        for i in 0..n {
            for j in i + 1..n {
                let dx = self.nodes[i].pos.x - self.nodes[j].pos.x;
                let dy = self.nodes[i].pos.y - self.nodes[j].pos.y;
                let d2 = (dx * dx + dy * dy).max(1.0);
                let d = d2.sqrt();
                let force = k * k / d / d;
                let (fx, fy) = (dx / d * force * 40.0, dy / d * force * 40.0);
                disp[i].x += fx;
                disp[i].y += fy;
                disp[j].x -= fx;
                disp[j].y -= fy;
            }
        }
        // Притяжение по рёбрам.
        for (a, b) in &self.edges {
            let dx = self.nodes[*a].pos.x - self.nodes[*b].pos.x;
            let dy = self.nodes[*a].pos.y - self.nodes[*b].pos.y;
            let d = (dx * dx + dy * dy).sqrt().max(0.5);
            let force = d * d / k * 0.012;
            let (fx, fy) = (dx / d * force, dy / d * force);
            disp[*a].x -= fx;
            disp[*a].y -= fy;
            disp[*b].x += fx;
            disp[*b].y += fy;
        }

        let max_step = 18.0 * self.temperature;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if node.is_center && self.mode == GraphMode::Mini {
                continue;
            }
            if let Some((di, _, _)) = self.drag {
                if di == i {
                    continue;
                }
            }
            let d = (disp[i].x * disp[i].x + disp[i].y * disp[i].y).sqrt().max(0.001);
            let step = d.min(max_step);
            node.pos.x = (node.pos.x + disp[i].x / d * step).clamp(30.0, world.width - 30.0);
            node.pos.y = (node.pos.y + disp[i].y / d * step).clamp(24.0, world.height - 24.0);
        }
        self.temperature = (self.temperature * 0.96).max(0.0);
        if self.temperature < 0.02 {
            self.temperature = 0.0;
        }
    }

    fn radius(&self, node: &SimNode) -> f32 {
        let base = 5.0 + (node.degree as f32).sqrt() * 2.4;
        if node.is_center { base + 3.0 } else { base }
    }

    fn node_at(&self, p: Point) -> Option<usize> {
        let o = self.bounds.origin;
        self.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            let dx = p.x - (o.x + n.pos.x);
            let dy = p.y - (o.y + n.pos.y);
            let r = self.radius(n) + 4.0;
            (dx * dx + dy * dy <= r * r).then_some(i)
        })
    }
}

impl Element for GraphElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<GraphView>() {
            self.ctx = w.ctx;
            self.mode = w.mode;
            if self.center != w.center {
                self.center = w.center.clone();
                self.data_fp = 0;
            }
            self.rebuild_data();
            self.mark_dirty(DirtyFlags::RENDER);
        }
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = match self.mode {
            GraphMode::Full => self.world_size(),
            GraphMode::Mini => Size::new(
                if constraints.max_width.is_finite() { constraints.max_width } else { 240.0 },
                220.0,
            ),
        };
        self.bounds.size = size;
        self.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let o = self.bounds.origin;
        let neighbor: HashSet<usize> = match self.hover {
            Some(h) => self
                .edges
                .iter()
                .flat_map(|(a, b)| {
                    if *a == h {
                        vec![*b]
                    } else if *b == h {
                        vec![*a]
                    } else {
                        Vec::new()
                    }
                })
                .chain(std::iter::once(h))
                .collect(),
            None => HashSet::new(),
        };

        let mut c = CanvasContext::new(o, self.bounds.size);
        for (a, b) in &self.edges {
            let active = self.hover.map(|h| *a == h || *b == h).unwrap_or(false);
            c.set_color(if active {
                Color::from_hex("#EE5E48").with_alpha(0.8)
            } else {
                Color::from_hex("#9CA3AF").with_alpha(0.35)
            });
            c.set_stroke_width(if active { 1.8 } else { 1.1 });
            let pa = self.nodes[*a].pos;
            let pb = self.nodes[*b].pos;
            c.draw_line(pa.x, pa.y, pb.x, pb.y);
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let dim = self.hover.is_some() && !neighbor.contains(&i);
            let base = if node.is_center {
                Color::from_hex("#EE5E48")
            } else {
                Color::from_hex("#5B6472")
            };
            c.set_color(if dim { base.with_alpha(0.25) } else { base });
            c.fill_circle(node.pos.x, node.pos.y, self.radius(node));
        }
        c.flush(list);

        // Подписи: у наведённого и соседей (в полном) / у всех (мини — до 12).
        let show_all = self.mode == GraphMode::Mini && self.nodes.len() <= 14;
        for (i, node) in self.nodes.iter().enumerate() {
            let show = show_all
                || self.hover == Some(i)
                || (self.hover.is_some() && neighbor.contains(&i))
                || (self.hover.is_none() && node.degree >= 2)
                || node.is_center;
            if !show {
                continue;
            }
            let dim = self.hover.is_some() && !neighbor.contains(&i);
            list.push_text_styled_singleline(
                &node.title,
                Rect::new(
                    Point::new(o.x + node.pos.x + self.radius(node) + 4.0, o.y + node.pos.y - 7.0),
                    Size::new(180.0, 14.0),
                ),
                Color::from_hex("#1C1D22").with_alpha(if dim { 0.3 } else { 0.85 }),
                11.0,
                TextAlign::DEFAULT,
                TextDecoration::None,
                if node.is_center { 600 } else { 400 },
                None,
            );
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseMove(p) => {
                if let Some((i, start, moved)) = &mut self.drag {
                    let o = self.bounds.origin;
                    let np = Point::new(p.x - o.x, p.y - o.y);
                    *moved = *moved
                        || (p.x - start.x).abs() + (p.y - start.y).abs() > 4.0;
                    if *moved {
                        self.nodes[*i].pos = np;
                        self.temperature = self.temperature.max(0.25);
                        self.mark_dirty(DirtyFlags::RENDER);
                    }
                    return EventResult::Handled;
                }
                let hover = self.node_at(*p);
                if hover != self.hover {
                    self.hover = hover;
                    self.mark_dirty(DirtyFlags::RENDER);
                }
                if hover.is_some() {
                    ctx.set_cursor(CursorIcon::Pointer);
                }
                EventResult::Ignored
            }
            Event::MouseDown { button: MouseButton::Left, position } => {
                if let Some(i) = self.node_at(*position) {
                    self.drag = Some((i, *position, false));
                    ctx.capture();
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if let Some((i, _, moved)) = self.drag.take() {
                    if !moved {
                        let rel = self.nodes[i].rel.clone();
                        self.ctx.open_path(&rel);
                        crate::rail::navigate("notes");
                    }
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        if self.temperature <= 0.0 {
            return false;
        }
        for _ in 0..3 {
            self.step();
        }
        self.mark_dirty(DirtyFlags::RENDER);
        true
    }

    fn wants_animate_tick(&self) -> bool {
        self.temperature > 0.0
    }

    fn element_type_name(&self) -> &str {
        "notes-graph"
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
