//! Шкала диаграммы Ганта — собственный элемент на CanvasContext.
//!
//! Слева от неё — колонка подписей задач из обычных виджетов
//! ([`super::view`]), поэтому здесь только время: месяцы в шапке, сетка
//! недель, выходные, линия «сегодня», бары задач и зависимости. Высоты
//! строк общие с колонкой подписей (`ROW_H`, `HEADER_H`).
//!
//! Жесты: бар таскается целиком (сдвиг обеих дат) или за края (одна
//! дата); кружок у правого конца бара тянет зависимость на другой бар;
//! клик по стрелке зависимости выделяет её, крестик на ней — удаляет;
//! Ctrl+колесо — масштаб (пишется в документ), колесо/драг фона —
//! горизонтальная прокрутка. Мутации уходят в документ только на MouseUp:
//! во время перетаскивания элемент живёт на локальном предпросмотре, и
//! перестройка по `revision` жест не рвёт.
//!
//! Цвета — из MSS класса `notes-gantt-chart`: `color` (текст),
//! `border-color` (сетка), `accent-color` (бар без своего цвета, линия
//! «сегодня»).

use std::any::Any;
use std::time::Duration;

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields, TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

use super::calendar::{civil_from_days, short_date, today_days, weekday_of};
use super::{GanttHandle, ZOOM_MAX, ZOOM_MIN};

pub const HEADER_H: f32 = 34.0;
pub const ROW_H: f32 = 36.0;
const BAR_H: f32 = 20.0;
const EDGE_PX: f32 = 7.0;
/// Кружок-коннектор зависимости у правого конца бара.
const DOT_R: f32 = 5.0;
const DOT_GAP: f32 = 9.0;
/// Радиус крестика удаления на выделенной зависимости.
const DEL_R: f32 = 7.0;

/// Задача (снимок из документа).
#[derive(Clone, Debug, PartialEq)]
struct Bar {
    id: String,
    /// Дни от эпохи, включительно.
    start: i64,
    end: i64,
    color: Option<String>,
}

pub struct GanttChart {
    pub handle: GanttHandle,
    /// Счётчик «показать сегодня» (тулбар).
    pub go_today: u64,
}

impl GanttChart {
    fn snapshot(handle: &GanttHandle) -> (Vec<Bar>, Vec<(String, String)>, f32) {
        let doc = handle.lock();
        let bars = doc
            .tasks
            .iter()
            .filter_map(|t| {
                let (start, end) = t.span_days()?;
                Some(Bar {
                    id: t.id.clone(),
                    start,
                    end,
                    color: (!t.color.is_empty()).then(|| t.color.clone()),
                })
            })
            .collect();
        let deps = doc.deps.iter().map(|d| (d.from.clone(), d.to.clone())).collect();
        (bars, deps, doc.zoom)
    }
}

impl Widget for GanttChart {
    fn create_element(&self) -> Box<dyn Element> {
        let (bars, deps, zoom) = Self::snapshot(&self.handle);
        let origin_day = bars.iter().map(|b| b.start).min().unwrap_or(today_days()) - 3;
        Box::new(GanttElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
            handle: self.handle.clone(),
            bars,
            deps,
            px_per_day: zoom,
            scroll_days: origin_day as f32,
            drag: None,
            hover: None,
            selected_dep: None,
            link: None,
            go_today: self.go_today,
        })
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

/// Жест над баром или фоном.
struct BarDrag {
    row_idx: usize,
    mode: DragMode,
    start_x: f32,
    /// Предпросмотр: сдвиги (start, end) в днях.
    delta: (i64, i64),
    moved: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
    Pan,
}

pub struct GanttElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
    handle: GanttHandle,
    bars: Vec<Bar>,
    deps: Vec<(String, String)>,
    px_per_day: f32,
    /// Левый край шкалы в днях от эпохи (дробное — плавный скролл).
    scroll_days: f32,
    drag: Option<BarDrag>,
    hover: Option<Point>,
    /// Выделенная зависимость (индекс в `deps`).
    selected_dep: Option<usize>,
    /// Тянущаяся зависимость: (строка-источник, курсор).
    link: Option<(usize, Point)>,
    go_today: u64,
}

impl GanttElement {
    fn text_color(&self) -> Color {
        self.mss.color.unwrap_or_else(|| Color::from_hex("#6B7280"))
    }

    fn grid_color(&self) -> Color {
        self.mss.border_color.unwrap_or_else(|| Color::from_hex("#000000").with_alpha(0.08))
    }

    fn accent(&self) -> Color {
        self.mss.accent_color.unwrap_or_else(|| Color::from_hex("#4F8CFF"))
    }

    fn chart_rect(&self) -> Rect {
        Rect::new(
            Point::new(self.bounds.origin.x, self.bounds.origin.y + HEADER_H),
            Size::new(self.bounds.size.width.max(1.0), (self.bounds.size.height - HEADER_H).max(0.0)),
        )
    }

    fn day_to_x(&self, day: f32) -> f32 {
        self.bounds.origin.x + (day - self.scroll_days) * self.px_per_day
    }

    fn x_to_day(&self, x: f32) -> f32 {
        (x - self.bounds.origin.x) / self.px_per_day + self.scroll_days
    }

    fn row_y(&self, idx: usize) -> f32 {
        self.bounds.origin.y + HEADER_H + idx as f32 * ROW_H
    }

    /// Бар строки с учётом drag-предпросмотра.
    fn bar_rect(&self, idx: usize) -> Rect {
        let bar = &self.bars[idx];
        let (ds, de) = self
            .drag
            .as_ref()
            .filter(|d| d.row_idx == idx)
            .map(|d| d.delta)
            .unwrap_or((0, 0));
        let start = (bar.start + ds) as f32;
        let end = (bar.end + de) as f32 + 1.0;
        let x = self.day_to_x(start);
        let w = ((end - start) * self.px_per_day).max(4.0);
        Rect::new(Point::new(x, self.row_y(idx) + (ROW_H - BAR_H) / 2.0), Size::new(w, BAR_H))
    }

    /// Центр коннектора зависимости у правого конца бара.
    fn dot_center(&self, idx: usize) -> Point {
        let bar = self.bar_rect(idx);
        Point::new(bar.origin.x + bar.size.width + DOT_GAP, bar.origin.y + bar.size.height / 2.0)
    }

    fn hit_bar(&self, p: Point) -> Option<(usize, DragMode)> {
        for idx in 0..self.bars.len() {
            let bar = self.bar_rect(idx);
            if !bar.contains(p) {
                continue;
            }
            let mode = if p.x < bar.origin.x + EDGE_PX {
                DragMode::ResizeStart
            } else if p.x > bar.origin.x + bar.size.width - EDGE_PX {
                DragMode::ResizeEnd
            } else {
                DragMode::Move
            };
            return Some((idx, mode));
        }
        None
    }

    /// Строка, чей бар (любая точка) под курсором — цель зависимости.
    fn hit_row_bar(&self, p: Point) -> Option<usize> {
        (0..self.bars.len()).find(|&i| self.bar_rect(i).contains(p))
    }

    fn hit_dot(&self, p: Point) -> Option<usize> {
        (0..self.bars.len()).find(|&i| dist(p, self.dot_center(i)) <= DOT_R + 3.0)
    }

    /// Строка под курсором (для показа коннектора).
    fn hover_row(&self) -> Option<usize> {
        let p = self.hover?;
        let chart = self.chart_rect();
        if !chart.contains(p) {
            return None;
        }
        let idx = ((p.y - chart.origin.y) / ROW_H).floor();
        (idx >= 0.0 && (idx as usize) < self.bars.len()).then_some(idx as usize)
    }

    /// Ломаная зависимости: конец A → уголок → начало B.
    fn dep_path(&self, dep_idx: usize) -> Option<[Point; 4]> {
        let (from, to) = &self.deps[dep_idx];
        let a = self.bars.iter().position(|b| &b.id == from)?;
        let b = self.bars.iter().position(|b| &b.id == to)?;
        let from_bar = self.bar_rect(a);
        let to_bar = self.bar_rect(b);
        let x1 = from_bar.origin.x + from_bar.size.width;
        let y1 = from_bar.origin.y + from_bar.size.height / 2.0;
        let x2 = to_bar.origin.x;
        let y2 = to_bar.origin.y + to_bar.size.height / 2.0;
        let mid = (x1 + 10.0).min(x2 - 8.0).max(x1 + 4.0);
        Some([Point::new(x1, y1), Point::new(mid, y1), Point::new(mid, y2), Point::new(x2, y2)])
    }

    fn hit_dep(&self, p: Point) -> Option<usize> {
        (0..self.deps.len()).find(|&i| {
            self.dep_path(i)
                .map(|path| path.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= 6.0))
                .unwrap_or(false)
        })
    }

    /// Крестик удаления на выделенной зависимости — середина вертикали.
    fn del_center(&self, dep_idx: usize) -> Option<Point> {
        let path = self.dep_path(dep_idx)?;
        Some(Point::new(path[1].x, (path[1].y + path[2].y) / 2.0))
    }

    fn commit_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved || drag.delta == (0, 0) {
            return;
        }
        let Some(bar) = self.bars.get(drag.row_idx) else { return };
        let start = bar.start + drag.delta.0;
        let end = bar.end + drag.delta.1;
        self.handle.set_task_dates(&bar.id, start, end.max(start));
    }

    fn scroll_to_today(&mut self) {
        let visible = (self.chart_rect().size.width / self.px_per_day).max(1.0);
        self.scroll_days = today_days() as f32 - (visible / 3.0).floor();
    }

    /// Масштаб с якорем под точкой `anchor_x` (курсор либо центр).
    fn set_zoom_anchored(&mut self, px_per_day: f32, anchor_x: f32) {
        let anchor_day = self.x_to_day(anchor_x);
        self.px_per_day = px_per_day.clamp(ZOOM_MIN, ZOOM_MAX);
        let new_day = self.x_to_day(anchor_x);
        self.scroll_days += anchor_day - new_day;
    }
}

impl Element for GanttElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<GanttChart>() else { return };
        self.handle = w.handle.clone();
        let (bars, deps, zoom) = GanttChart::snapshot(&self.handle);
        if bars != self.bars || deps != self.deps {
            let grew = bars.len() != self.bars.len();
            self.bars = bars;
            self.deps = deps;
            self.selected_dep = self.selected_dep.filter(|&i| i < self.deps.len());
            self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
            if grew {
                ctx.mark_layout_dirty();
            }
        }
        // Масштаб из документа (кнопки тулбара) — якорь по центру шкалы.
        if (zoom - self.px_per_day).abs() > 0.01 {
            let center = self.bounds.origin.x + self.bounds.size.width / 2.0;
            self.set_zoom_anchored(zoom, center);
            self.mark_dirty(DirtyFlags::RENDER);
        }
        if w.go_today != self.go_today {
            self.go_today = w.go_today;
            self.scroll_to_today();
            self.mark_dirty(DirtyFlags::RENDER);
        }
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let width = if constraints.max_width.is_finite() { constraints.max_width } else { 800.0 };
        let height = HEADER_H + (self.bars.len().max(3) as f32) * ROW_H + 8.0;
        let height = if constraints.max_height.is_finite() { height.min(constraints.max_height) } else { height };
        self.bounds.size = Size::new(width, height);
        self.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let chart = self.chart_rect();
        let text_c = self.text_color();
        let grid_c = self.grid_color();
        let accent = self.accent();

        list.push_clip(self.bounds);
        let first_day = self.scroll_days.floor() as i64 - 1;
        let days_visible = (self.bounds.size.width / self.px_per_day).ceil() as i64 + 2;
        for day in first_day..first_day + days_visible {
            let x = self.day_to_x(day as f32);
            let (y, m, d) = civil_from_days(day);
            let weekday = weekday_of(day);
            // Выходные — лёгкая заливка.
            if weekday >= 5 {
                list.push_rect(
                    Rect::new(Point::new(x, chart.origin.y), Size::new(self.px_per_day, chart.size.height)),
                    grid_c.with_alpha(grid_c.a * 0.5),
                    [0.0; 4],
                );
            }
            // Недельные линии (понедельник) и границы месяцев.
            if d == 1 || weekday == 0 {
                let strong = d == 1;
                list.push_rect(
                    Rect::new(Point::new(x, chart.origin.y), Size::new(1.0, chart.size.height)),
                    if strong { grid_c.with_alpha((grid_c.a * 2.2).min(1.0)) } else { grid_c },
                    [0.0; 4],
                );
            }
            if d == 1 {
                list.push_text_styled_singleline(
                    &format!("{m:02}.{y}"),
                    Rect::new(Point::new(x + 6.0, self.bounds.origin.y + 4.0), Size::new(80.0, 14.0)),
                    text_c,
                    11.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    600,
                    None,
                );
            }
            // Числа дней при крупном масштабе.
            if self.px_per_day > 18.0 && weekday == 0 {
                list.push_text_styled_singleline(
                    &format!("{d}"),
                    Rect::new(Point::new(x + 3.0, self.bounds.origin.y + 19.0), Size::new(30.0, 12.0)),
                    text_c.with_alpha(0.8),
                    10.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    400,
                    None,
                );
            }
        }
        // Линия шапки.
        list.push_rect(
            Rect::new(Point::new(self.bounds.origin.x, chart.origin.y - 1.0), Size::new(self.bounds.size.width, 1.0)),
            grid_c,
            [0.0; 4],
        );
        // Линия «сегодня».
        let today_x = self.day_to_x(today_days() as f32 + 0.5);
        if today_x > chart.origin.x && today_x < chart.origin.x + chart.size.width {
            list.push_rect(
                Rect::new(Point::new(today_x, chart.origin.y), Size::new(2.0, chart.size.height)),
                accent.with_alpha(0.7),
                [1.0; 4],
            );
        }

        // Бары с датами внутри, когда влезают.
        let hover_row = self.hover_row();
        for idx in 0..self.bars.len() {
            let bar = self.bar_rect(idx);
            let color = self.bars[idx].color.as_deref().map(Color::from_hex).unwrap_or(accent);
            list.push_rect(bar, color.with_alpha(0.85), [5.0; 4]);
            if bar.size.width > 92.0 {
                let (ds, de) = self
                    .drag
                    .as_ref()
                    .filter(|d| d.row_idx == idx)
                    .map(|d| d.delta)
                    .unwrap_or((0, 0));
                let label = format!(
                    "{} – {}",
                    short_date(self.bars[idx].start + ds),
                    short_date(self.bars[idx].end + de)
                );
                list.push_text_styled_singleline(
                    &label,
                    Rect::new(
                        Point::new(bar.origin.x + 8.0, bar.origin.y + 3.0),
                        Size::new(bar.size.width - 16.0, 14.0),
                    ),
                    Color::from_hex("#ffffff").with_alpha(0.92),
                    10.5,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    500,
                    None,
                );
            }
            // Коннектор зависимости — у строки под курсором и у источника
            // тянущейся связи.
            let show_dot = hover_row == Some(idx) || self.link.map(|(i, _)| i == idx).unwrap_or(false);
            if show_dot {
                let c = self.dot_center(idx);
                list.push_rect(
                    Rect::new(Point::new(c.x - DOT_R, c.y - DOT_R), Size::new(DOT_R * 2.0, DOT_R * 2.0)),
                    text_c.with_alpha(0.9),
                    [DOT_R; 4],
                );
            }
        }

        // Зависимости: конец A → начало B, уголок со стрелкой.
        let mut c = CanvasContext::new(Point::zero(), Size::new(8192.0, 8192.0));
        for i in 0..self.deps.len() {
            let Some(path) = self.dep_path(i) else { continue };
            let selected = self.selected_dep == Some(i);
            c.set_color(if selected { accent } else { text_c.with_alpha(0.75) });
            c.set_stroke_width(if selected { 2.5 } else { 1.5 });
            for w in path.windows(2) {
                c.draw_line(w[0].x, w[0].y, w[1].x, w[1].y);
            }
            let end = path[3];
            c.fill_polygon(&[(end.x - 1.0, end.y), (end.x - 8.0, end.y - 4.0), (end.x - 8.0, end.y + 4.0)]);
        }
        // Тянущаяся зависимость — прямая до курсора.
        if let Some((from, cursor)) = self.link {
            if from < self.bars.len() {
                let a = self.dot_center(from);
                c.set_color(accent.with_alpha(0.9));
                c.set_stroke_width(1.5);
                c.draw_line(a.x, a.y, cursor.x, cursor.y);
            }
        }
        c.flush(list);

        // Крестик удаления на выделенной зависимости.
        if let Some(center) = self.selected_dep.and_then(|i| self.del_center(i)) {
            list.push_rect(
                Rect::new(Point::new(center.x - DEL_R, center.y - DEL_R), Size::new(DEL_R * 2.0, DEL_R * 2.0)),
                accent,
                [DEL_R; 4],
            );
            let mut x = CanvasContext::new(Point::zero(), Size::new(8192.0, 8192.0));
            x.set_color(Color::from_hex("#ffffff"));
            x.set_stroke_width(1.6);
            x.draw_line(center.x - 3.0, center.y - 3.0, center.x + 3.0, center.y + 3.0);
            x.draw_line(center.x - 3.0, center.y + 3.0, center.x + 3.0, center.y - 3.0);
            x.flush(list);
        }
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                // Крестик на выделенной зависимости — удалить её.
                if let Some(i) = self.selected_dep {
                    if self.del_center(i).is_some_and(|c| dist(*position, c) <= DEL_R + 2.0) {
                        let (from, to) = self.deps[i].clone();
                        self.selected_dep = None;
                        self.handle.delete_dep(&from, &to);
                        return EventResult::Handled;
                    }
                }
                if let Some(idx) = self.hit_dot(*position) {
                    self.link = Some((idx, *position));
                    self.selected_dep = None;
                    ctx.capture();
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                if let Some(dep) = self.hit_dep(*position) {
                    self.selected_dep = Some(dep);
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                self.selected_dep = None;
                let drag = match self.hit_bar(*position) {
                    Some((row_idx, mode)) => {
                        BarDrag { row_idx, mode, start_x: position.x, delta: (0, 0), moved: false }
                    }
                    None => BarDrag {
                        row_idx: usize::MAX,
                        mode: DragMode::Pan,
                        start_x: position.x,
                        delta: (0, 0),
                        moved: false,
                    },
                };
                self.drag = Some(drag);
                ctx.capture();
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                let inside = self.bounds.contains(*position);
                let prev_hover = self.hover_row();
                self.hover = inside.then_some(*position);
                if let Some((_, cursor)) = &mut self.link {
                    *cursor = *position;
                    ctx.set_cursor(CursorIcon::Crosshair);
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                let px_per_day = self.px_per_day;
                if let Some(drag) = &mut self.drag {
                    let dx = position.x - drag.start_x;
                    match drag.mode {
                        DragMode::Pan => {
                            self.scroll_days -= dx / px_per_day;
                            if let Some(d) = &mut self.drag {
                                d.start_x = position.x;
                                d.moved = true;
                            }
                            ctx.set_cursor(CursorIcon::Grabbing);
                        }
                        mode => {
                            let days = (dx / px_per_day).round() as i64;
                            drag.moved = drag.moved || days != 0;
                            drag.delta = match mode {
                                DragMode::Move => (days, days),
                                DragMode::ResizeStart => (days, 0),
                                DragMode::ResizeEnd => (0, days),
                                DragMode::Pan => (0, 0),
                            };
                            ctx.set_cursor(if mode == DragMode::Move { CursorIcon::Grabbing } else { CursorIcon::ColResize });
                        }
                    }
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                if inside {
                    let cursor = if self.hit_dot(*position).is_some() {
                        CursorIcon::Crosshair
                    } else if self.selected_dep.and_then(|i| self.del_center(i)).is_some_and(|c| dist(*position, c) <= DEL_R + 2.0)
                        || self.hit_dep(*position).is_some()
                    {
                        CursorIcon::Pointer
                    } else {
                        match self.hit_bar(*position) {
                            Some((_, DragMode::Move)) => CursorIcon::Grab,
                            Some(_) => CursorIcon::ColResize,
                            None => CursorIcon::Default,
                        }
                    };
                    ctx.set_cursor(cursor);
                }
                if prev_hover != self.hover_row() {
                    self.mark_dirty(DirtyFlags::RENDER);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                if let Some((from, _)) = self.link.take() {
                    if let Some(to) = self.hit_row_bar(*position) {
                        if from != to {
                            let (a, b) = (self.bars[from].id.clone(), self.bars[to].id.clone());
                            self.handle.add_dep(&a, &b);
                        }
                    }
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                if self.drag.is_some() {
                    self.commit_drag();
                    self.mark_dirty(DirtyFlags::RENDER);
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            Event::MouseWheel { delta, delta_x, position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                if ctx.modifiers.ctrl {
                    let factor = if *delta > 0.0 { 1.15 } else { 1.0 / 1.15 };
                    let zoom = (self.px_per_day * factor).clamp(ZOOM_MIN, ZOOM_MAX);
                    self.set_zoom_anchored(zoom, position.x);
                    // В документ — чтобы масштаб пережил перезапуск; сам
                    // элемент уже в новом масштабе, `update` его не тронет.
                    self.handle.set_zoom(self.px_per_day);
                } else {
                    let d = if delta_x.abs() > delta.abs() { *delta_x } else { *delta };
                    self.scroll_days += d * 20.0 / self.px_per_day;
                }
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }

    fn element_type_name(&self) -> &str {
        "notes-gantt-chart"
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

fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Расстояние от точки до отрезка.
fn dist_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    dist(p, Point::new(a.x + t * dx, a.y + t * dy))
}
