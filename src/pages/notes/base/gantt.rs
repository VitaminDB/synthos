//! Ганнт-представление базы: собственный элемент на CanvasContext.
//!
//! Слева фиксированная колонка подписей, справа — прокручиваемая шкала
//! времени: месяцы в шапке, сетка недель, линия «сегодня», бары строк с
//! датами начала/конца. Бар таскается целиком (сдвиг обеих дат) или за
//! края (ресайз одной), Ctrl+колесо — зум шкалы, колесо/драг фона —
//! горизонтальная прокрутка. Зависимости рисуются уголками со стрелками.
//!
//! Мутации пишутся в документ только на MouseUp: во время перетаскивания
//! элемент живёт на локальном предпросмотре, поэтому rebuild по revision
//! не рвёт жест.

use std::any::Any;
use std::time::Duration;

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

use super::model::{CellValue, ColumnKind};
use super::BaseHandle;

const HEADER_H: f32 = 34.0;
const ROW_H: f32 = 32.0;
const LABEL_W: f32 = 180.0;
const BAR_H: f32 = 18.0;
const EDGE_PX: f32 = 7.0;

/// Строка ганнта (снимок из документа).
#[derive(Clone, Debug, PartialEq)]
struct GanttRow {
    id: String,
    label: String,
    /// Дни от эпохи (civil).
    start: i64,
    end: i64,
    color: Option<String>,
}

pub fn view(handle: BaseHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.revision.get();
        let _ = handle.structure_rev.get();
        vec![Box::new(GanttView { handle: handle.clone() })]
    })
}

pub struct GanttView {
    handle: BaseHandle,
}

impl GanttView {
    /// Снимок строк с датами + пары зависимостей.
    fn snapshot(handle: &BaseHandle) -> (Vec<GanttRow>, Vec<(String, String)>, String, String) {
        let doc = handle.lock().clone();
        let view_idx = handle.view_sel.get_untracked().min(doc.views.len().saturating_sub(1));
        let Some(view) = doc.views.get(view_idx) else {
            return (Vec::new(), Vec::new(), String::new(), String::new());
        };
        let start_col = view.start_col.clone().unwrap_or_default();
        let end_col = view.end_col.clone().unwrap_or_else(|| start_col.clone());
        let label_col = view
            .label_col
            .clone()
            .or_else(|| doc.columns.first().map(|c| c.id.clone()))
            .unwrap_or_default();
        let select_col = doc.columns.iter().find(|c| c.kind == ColumnKind::Select);

        let mut rows = Vec::new();
        for row in &doc.rows {
            let s = parse_days(&doc.cell_text(row, &start_col));
            let e = parse_days(&doc.cell_text(row, &end_col));
            let (Some(s), Some(e)) = (s, e.or(s)) else { continue };
            let (s, e) = if e < s { (e, s) } else { (s, e) };
            let color = select_col.and_then(|c| {
                let v = doc.cell_text(row, &c.id);
                c.options
                    .iter()
                    .find(|o| o.id == v || o.name == v)
                    .map(|o| o.color.clone())
                    .filter(|c| !c.is_empty())
            });
            rows.push(GanttRow {
                id: row.id.clone(),
                label: {
                    let l = doc.cell_text(row, &label_col);
                    if l.is_empty() { tr!("notes.base.kanban.untitled") } else { l }
                },
                start: s,
                end: e,
                color,
            });
        }
        (rows, view.deps.clone(), start_col, end_col)
    }
}

impl Widget for GanttView {
    fn create_element(&self) -> Box<dyn Element> {
        let (rows, deps, start_col, end_col) = Self::snapshot(&self.handle);
        let origin_day = rows.iter().map(|r| r.start).min().unwrap_or(today_days()) - 7;
        Box::new(GanttElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            handle: self.handle.clone(),
            rows,
            deps,
            start_col,
            end_col,
            px_per_day: 26.0,
            scroll_days: origin_day as f32,
            drag: None,
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

/// Жест над баром.
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
    handle: BaseHandle,
    rows: Vec<GanttRow>,
    deps: Vec<(String, String)>,
    start_col: String,
    end_col: String,
    px_per_day: f32,
    /// Левый край шкалы в днях от эпохи (дробное — плавный скролл).
    scroll_days: f32,
    drag: Option<BarDrag>,
}

impl GanttElement {
    fn chart_rect(&self) -> Rect {
        Rect::new(
            Point::new(self.bounds.origin.x + LABEL_W, self.bounds.origin.y + HEADER_H),
            Size::new(
                (self.bounds.size.width - LABEL_W).max(40.0),
                (self.bounds.size.height - HEADER_H).max(0.0),
            ),
        )
    }

    fn day_to_x(&self, day: f32) -> f32 {
        self.chart_rect().origin.x + (day - self.scroll_days) * self.px_per_day
    }

    fn x_to_day(&self, x: f32) -> f32 {
        (x - self.chart_rect().origin.x) / self.px_per_day + self.scroll_days
    }

    fn row_y(&self, idx: usize) -> f32 {
        self.bounds.origin.y + HEADER_H + idx as f32 * ROW_H
    }

    /// Бар строки с учётом drag-предпросмотра.
    fn bar_rect(&self, idx: usize) -> Rect {
        let row = &self.rows[idx];
        let (ds, de) = self
            .drag
            .as_ref()
            .filter(|d| d.row_idx == idx)
            .map(|d| d.delta)
            .unwrap_or((0, 0));
        let start = (row.start + ds) as f32;
        let end = (row.end + de) as f32 + 1.0;
        let x = self.day_to_x(start);
        let w = ((end - start) * self.px_per_day).max(4.0);
        Rect::new(
            Point::new(x, self.row_y(idx) + (ROW_H - BAR_H) / 2.0),
            Size::new(w, BAR_H),
        )
    }

    fn hit_bar(&self, p: Point) -> Option<(usize, DragMode)> {
        for idx in 0..self.rows.len() {
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

    fn commit_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved || drag.delta == (0, 0) {
            return;
        }
        let Some(row) = self.rows.get(drag.row_idx) else { return };
        let new_start = row.start + drag.delta.0;
        let new_end = row.end + drag.delta.1;
        let (row_id, start_col, end_col) =
            (row.id.clone(), self.start_col.clone(), self.end_col.clone());
        self.handle.edit_structural(move |doc| {
            if let Some(r) = doc.rows.iter_mut().find(|r| r.id == row_id) {
                r.cells.insert(start_col, CellValue::Text(days_to_iso(new_start)));
                r.cells.insert(end_col, CellValue::Text(days_to_iso(new_end.max(new_start))));
            }
        });
    }
}

impl Element for GanttElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<GanttView>() else { return };
        self.handle = w.handle.clone();
        let (rows, deps, start_col, end_col) = GanttView::snapshot(&self.handle);
        if rows != self.rows || deps != self.deps {
            self.rows = rows;
            self.deps = deps;
            self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
            ctx.mark_layout_dirty();
        }
        self.start_col = start_col;
        self.end_col = end_col;
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let width = if constraints.max_width.is_finite() { constraints.max_width } else { 800.0 };
        let height = HEADER_H + (self.rows.len().max(3) as f32) * ROW_H + 16.0;
        let height = height.min(if constraints.max_height.is_finite() {
            constraints.max_height
        } else {
            height
        });
        self.bounds.size = Size::new(width, height);
        self.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let chart = self.chart_rect();
        let text_c = Color::from_hex("#6B7280");
        let grid_c = Color::from_hex("#000000").with_alpha(0.07);
        let today_c = Color::from_hex("#EE5E48");

        // Сетка и шапка — только в области шкалы.
        list.push_clip(Rect::new(
            Point::new(chart.origin.x, self.bounds.origin.y),
            Size::new(chart.size.width, self.bounds.size.height),
        ));
        let first_day = self.scroll_days.floor() as i64 - 1;
        let days_visible = (chart.size.width / self.px_per_day).ceil() as i64 + 2;
        for day in first_day..first_day + days_visible {
            let x = self.day_to_x(day as f32);
            let (y, m, d) = civil_from_days(day);
            // Недельные линии (понедельник) и границы месяцев.
            let weekday = weekday_of(day);
            if d == 1 || weekday == 0 {
                let strong = d == 1;
                list.push_rect(
                    Rect::new(
                        Point::new(x, chart.origin.y),
                        Size::new(1.0, chart.size.height),
                    ),
                    if strong { grid_c.with_alpha(0.16) } else { grid_c },
                    [0.0; 4],
                );
            }
            if d == 1 {
                list.push_text_styled_singleline(
                    &format!("{:02}.{}", m, y),
                    Rect::new(
                        Point::new(x + 6.0, self.bounds.origin.y + 4.0),
                        Size::new(80.0, 14.0),
                    ),
                    text_c,
                    11.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    600,
                    None,
                );
            }
            // Числа дней при крупном зуме.
            if self.px_per_day > 18.0 && weekday == 0 {
                list.push_text_styled_singleline(
                    &format!("{d}"),
                    Rect::new(
                        Point::new(x + 3.0, self.bounds.origin.y + 19.0),
                        Size::new(30.0, 12.0),
                    ),
                    text_c.with_alpha(0.8),
                    10.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    400,
                    None,
                );
            }
        }
        // Линия «сегодня».
        let today_x = self.day_to_x(today_days() as f32 + 0.5);
        if today_x > chart.origin.x && today_x < chart.origin.x + chart.size.width {
            list.push_rect(
                Rect::new(
                    Point::new(today_x, chart.origin.y),
                    Size::new(2.0, chart.size.height),
                ),
                today_c.with_alpha(0.7),
                [1.0; 4],
            );
        }

        // Бары.
        for idx in 0..self.rows.len() {
            let bar = self.bar_rect(idx);
            let color = self.rows[idx]
                .color
                .as_deref()
                .map(Color::from_hex)
                .unwrap_or_else(|| Color::from_hex("#EE5E48"));
            list.push_rect(bar, color.with_alpha(0.85), [5.0; 4]);
        }

        // Зависимости: конец A → начало B, уголок со стрелкой.
        let index_of = |id: &str| self.rows.iter().position(|r| r.id == id);
        let mut c = CanvasContext::new(Point::zero(), Size::new(4096.0, 4096.0));
        c.set_color(text_c.with_alpha(0.75));
        c.set_stroke_width(1.5);
        for (from, to) in &self.deps {
            let (Some(a), Some(b)) = (index_of(from), index_of(to)) else { continue };
            let from_bar = self.bar_rect(a);
            let to_bar = self.bar_rect(b);
            let x1 = from_bar.origin.x + from_bar.size.width;
            let y1 = from_bar.origin.y + from_bar.size.height / 2.0;
            let x2 = to_bar.origin.x;
            let y2 = to_bar.origin.y + to_bar.size.height / 2.0;
            let mid = x1 + 10.0;
            c.draw_line(x1, y1, mid, y1);
            c.draw_line(mid, y1, mid, y2);
            c.draw_line(mid, y2, x2 - 4.0, y2);
            c.fill_polygon(&[(x2 - 1.0, y2), (x2 - 8.0, y2 - 4.0), (x2 - 8.0, y2 + 4.0)]);
        }
        c.flush(list);
        list.pop_clip();

        // Колонка подписей (поверх, со своим фоном).
        list.push_rect(
            Rect::new(self.bounds.origin, Size::new(LABEL_W, self.bounds.size.height)),
            Color::from_hex("#ffffff").with_alpha(0.6),
            [0.0; 4],
        );
        for (idx, row) in self.rows.iter().enumerate() {
            list.push_text_styled_singleline(
                &row.label,
                Rect::new(
                    Point::new(self.bounds.origin.x + 12.0, self.row_y(idx) + 8.0),
                    Size::new(LABEL_W - 20.0, 16.0),
                ),
                Color::from_hex("#1C1D22"),
                12.5,
                TextAlign::DEFAULT,
                TextDecoration::None,
                500,
                None,
            );
        }
        if self.rows.is_empty() {
            list.push_text_styled_singleline(
                &tr!("notes.base.gantt.empty"),
                Rect::new(
                    Point::new(chart.origin.x + 16.0, chart.origin.y + 10.0),
                    Size::new(chart.size.width - 24.0, 16.0),
                ),
                text_c,
                12.0,
                TextAlign::DEFAULT,
                TextDecoration::None,
                400,
                None,
            );
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.bounds.contains(*position) || position.x < self.chart_rect().origin.x {
                    return EventResult::Ignored;
                }
                let drag = match self.hit_bar(*position) {
                    Some((row_idx, mode)) => BarDrag {
                        row_idx,
                        mode,
                        start_x: position.x,
                        delta: (0, 0),
                        moved: false,
                    },
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
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                let px_per_day = self.px_per_day;
                let Some(drag) = &mut self.drag else { return EventResult::Ignored };
                let dx = position.x - drag.start_x;
                match drag.mode {
                    DragMode::Pan => {
                        self.scroll_days -= dx / px_per_day;
                        if let Some(d) = &mut self.drag {
                            d.start_x = position.x;
                            d.moved = true;
                        }
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
                    }
                }
                self.mark_dirty(DirtyFlags::RENDER);
                EventResult::Handled
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
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
                    // Зум с якорем под курсором.
                    let anchor_day = self.x_to_day(position.x);
                    let factor = if *delta > 0.0 { 1.15 } else { 1.0 / 1.15 };
                    self.px_per_day = (self.px_per_day * factor).clamp(5.0, 90.0);
                    let new_day = self.x_to_day(position.x);
                    self.scroll_days += anchor_day - new_day;
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
        "notes-gantt"
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

// ─── Календарная арифметика (алгоритмы civil_from_days/days_from_civil) ────

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = понедельник.
fn weekday_of(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

fn today_days() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs.div_euclid(86400)
}

fn parse_days(iso: &str) -> Option<i64> {
    let mut parts = iso.trim().splitn(3, '-');
    let y = parts.next()?.parse::<i64>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

fn days_to_iso(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for iso in ["2026-09-01", "2000-02-29", "1999-12-31", "2026-01-01"] {
            let days = parse_days(iso).unwrap();
            assert_eq!(days_to_iso(days), iso);
        }
        // 2026-09-01 — вторник.
        assert_eq!(weekday_of(parse_days("2026-09-01").unwrap()), 1);
    }
}
