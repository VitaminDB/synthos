//! Годовая сетка — 12 мини-месяцев (4×3, на узком холсте 3×4 / 2×6):
//! номера дней, «сегодня» кружком, точки событий по цвету календаря.
//! Клик по дню — дневной вид на нём, по названию месяца — месячный.

use std::any::Any;

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

use super::model::{days_in_month, week_start, CalView};
use super::view::{color_of, day_weekday, GridData, Palette};
use super::{CalendarEnv, CalendarHandle, ElementBase};
use crate::pages::notes::gantt::calendar::{civil_from_days, days_from_civil};

const TITLE_H: f32 = 20.0;
const WEEKDAY_H: f32 = 14.0;
const PAD: f32 = 6.0;

pub struct YearGrid {
    pub env: CalendarEnv,
    pub handle: CalendarHandle,
    pub data: GridData,
}

impl Widget for YearGrid {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(YearElement { base: ElementBase::new(), handle: self.handle.clone(), data: self.data.clone(), hover: None })
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

pub struct YearElement {
    base: ElementBase,
    handle: CalendarHandle,
    data: GridData,
    hover: Option<i64>,
}

/// Что под курсором в годовой сетке.
enum Hit {
    Day(i64),
    Month(u32),
}

impl YearElement {
    fn columns(&self) -> u32 {
        let w = self.base.bounds.size.width;
        if w >= 720.0 { 4 } else if w >= 480.0 { 3 } else { 2 }
    }

    fn year(&self) -> i64 {
        civil_from_days(self.data.doc.anchor_days()).0
    }

    fn mini_rect(&self, month: u32) -> Rect {
        let cols = self.columns();
        let rows = 12_u32.div_ceil(cols);
        let b = self.base.bounds;
        let (mw, mh) = (b.size.width / cols as f32, b.size.height / rows as f32);
        let idx = month - 1;
        Rect::new(
            Point::new(b.origin.x + (idx % cols) as f32 * mw, b.origin.y + (idx / cols) as f32 * mh),
            Size::new(mw, mh),
        )
    }

    /// Размер клетки дня и левый верхний угол сетки дней мини-месяца.
    fn day_grid(&self, month: u32) -> (Rect, f32, f32) {
        let r = self.mini_rect(month);
        let grid = Rect::new(
            Point::new(r.origin.x + PAD, r.origin.y + TITLE_H + WEEKDAY_H),
            Size::new(r.size.width - PAD * 2.0, r.size.height - TITLE_H - WEEKDAY_H - PAD),
        );
        (grid, grid.size.width / 7.0, grid.size.height / 6.0)
    }

    fn day_rect(&self, day: i64) -> Option<Rect> {
        let (y, m, _) = civil_from_days(day);
        if y != self.year() {
            return None;
        }
        let first = days_from_civil(y, m, 1);
        let start = week_start(first, self.data.doc.style.first_weekday);
        let idx = day - start;
        if !(0..42).contains(&idx) {
            return None;
        }
        let (grid, cw, ch) = self.day_grid(m);
        Some(Rect::new(
            Point::new(grid.origin.x + (idx % 7) as f32 * cw, grid.origin.y + (idx / 7) as f32 * ch),
            Size::new(cw, ch),
        ))
    }

    fn hit(&self, p: Point) -> Option<Hit> {
        for m in 1..=12u32 {
            let r = self.mini_rect(m);
            if !r.contains(p) {
                continue;
            }
            if p.y < r.origin.y + TITLE_H {
                return Some(Hit::Month(m));
            }
            let (grid, cw, ch) = self.day_grid(m);
            if !grid.contains(p) {
                return None;
            }
            let col = ((p.x - grid.origin.x) / cw).floor() as i64;
            let row = ((p.y - grid.origin.y) / ch).floor() as i64;
            let first = days_from_civil(self.year(), m, 1);
            let start = week_start(first, self.data.doc.style.first_weekday);
            let day = start + row * 7 + col;
            let (_, dm, _) = civil_from_days(day);
            return (dm == m).then_some(Hit::Day(day));
        }
        None
    }
}

impl Element for YearElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<YearGrid>() else { return };
        self.handle = w.handle.clone();
        self.data = w.data.clone();
        self.base.dirty |= DirtyFlags::LAYOUT | DirtyFlags::RENDER;
        ctx.mark_layout_dirty();
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 720.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height.max(240.0) } else { 480.0 };
        self.base.bounds.size = Size::new(w, h);
        self.base.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let b = self.base.bounds;
        let style = &self.data.doc.style;
        let pal = Palette::resolve(style, &self.base.mss);
        let font = (style.font_size - 2.0).max(8.0);
        let year = self.year();
        list.push_clip(b);
        if pal.cell_bg.a > 0.0 {
            list.push_rect(b, pal.cell_bg, [0.0; 4]);
        }
        let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
        for m in 1..=12u32 {
            let r = self.mini_rect(m);
            list.push_text_styled_singleline(
                &self.data.locale.month_title(m),
                Rect::new(Point::new(r.origin.x + PAD, r.origin.y + 3.0), Size::new(r.size.width - PAD * 2.0, TITLE_H - 4.0)),
                if self.hover == Some(-(m as i64)) { pal.accent } else { pal.text },
                font + 1.0,
                TextAlign::DEFAULT,
                TextDecoration::None,
                700,
                None,
            );
            let (grid, cw, ch) = self.day_grid(m);
            for col in 0..7u32 {
                let wd = (style.first_weekday + col) % 7;
                let name: String = self.data.locale.weekday_short(wd).chars().take(1).collect();
                list.push_text_styled_singleline(
                    &name,
                    Rect::new(Point::new(grid.origin.x + col as f32 * cw, r.origin.y + TITLE_H), Size::new(cw, WEEKDAY_H)),
                    pal.muted,
                    font - 1.0,
                    TextAlign::CENTER,
                    TextDecoration::None,
                    500,
                    None,
                );
            }
            let first = days_from_civil(year, m, 1);
            let n = days_in_month(year, m);
            for d in 1..=n {
                let day = first + d as i64 - 1;
                let Some(cell) = self.day_rect(day) else { continue };
                let wd = day_weekday(day);
                let is_today = day == self.data.today;
                if is_today {
                    let rr = (cw.min(ch) / 2.0 - 1.0).max(6.0);
                    let cx = cell.origin.x + cw / 2.0;
                    let cy = cell.origin.y + ch / 2.0;
                    list.push_rect(Rect::new(Point::new(cx - rr, cy - rr), Size::new(rr * 2.0, rr * 2.0)), pal.today, [rr; 4]);
                } else if self.hover == Some(day) {
                    list.push_rect(cell, pal.accent.with_alpha(0.15), [3.0; 4]);
                }
                list.push_text_styled_singleline(
                    &format!("{d}"),
                    Rect::new(Point::new(cell.origin.x, cell.origin.y + (ch - font - 3.0) / 2.0), Size::new(cw, font + 3.0)),
                    if is_today { Color::from_hex("#FFFFFF") } else if self.data.locale.is_weekend(wd) { pal.muted } else { pal.text },
                    font,
                    TextAlign::CENTER,
                    TextDecoration::None,
                    if is_today { 700 } else { 400 },
                    None,
                );
                // Точки событий (до трёх, по цветам).
                let mut colors: Vec<Color> = self
                    .data
                    .occurrences
                    .iter()
                    .filter(|o| o.day == day)
                    .filter_map(|o| self.data.store.event(&o.event).map(|e| self.data.store.color_of(e)))
                    .filter_map(|c| color_of(&c))
                    .collect();
                colors.extend(self.data.external.iter().filter(|e| day >= e.day && day <= e.end_day).map(|e| color_of(&e.color).unwrap_or(pal.muted)));
                colors.dedup_by(|a, b| a == b);
                let dots = colors.len().min(3);
                if dots > 0 {
                    let r = 1.6;
                    let total = dots as f32 * (r * 2.0 + 1.5) - 1.5;
                    let mut x = cell.origin.x + cw / 2.0 - total / 2.0 + r;
                    let y = cell.origin.y + ch - r - 1.5;
                    for col in colors.iter().take(dots) {
                        c.set_color(col.with_alpha(if is_today { 1.0 } else { 0.95 }));
                        c.fill_circle(x, y, r);
                        x += r * 2.0 + 1.5;
                    }
                }
            }
        }
        c.flush(list);
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.base.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                match self.hit(*position) {
                    Some(Hit::Day(day)) => {
                        self.handle.set_anchor(day);
                        self.handle.selected_day.set(Some(day));
                        self.handle.set_view(CalView::Day);
                        EventResult::Handled
                    }
                    Some(Hit::Month(m)) => {
                        self.handle.set_anchor(days_from_civil(self.year(), m, 1));
                        self.handle.set_view(CalView::Month);
                        EventResult::Handled
                    }
                    None => EventResult::Ignored,
                }
            }
            Event::MouseMove(position) => {
                let hover = if self.base.bounds.contains(*position) {
                    match self.hit(*position) {
                        Some(Hit::Day(d)) => Some(d),
                        Some(Hit::Month(m)) => Some(-(m as i64)),
                        None => None,
                    }
                } else {
                    None
                };
                if hover != self.hover {
                    self.hover = hover;
                    self.base.dirty |= DirtyFlags::RENDER;
                }
                if hover.is_some() {
                    ctx.set_cursor(CursorIcon::Pointer);
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    element_boilerplate!("notes-calendar-year");
}
