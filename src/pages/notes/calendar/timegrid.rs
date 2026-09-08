//! Сетка часов — неделя (7 колонок) и день (1 колонка): шапка с днями,
//! ряд «весь день», часы `hour_from..hour_to` слотами `slot_min`, линия
//! «сейчас» в сегодняшней колонке, события полосами с упаковкой
//! пересечений по дорожкам, внешний слой (задачи досок и Ганта): полоса с
//! часами встаёт в сетку наравне с событием, без часов — в ряд «весь
//! день».
//!
//! Жесты: клик по пустому слоту — выбор дня, двойной — новое событие в
//! слоте; клик по событию — попап правки, по внешней полосе — её
//! страница; drag тела — перенос (день и время со снапом к слоту), drag
//! нижней кромки — длительность; внешняя полоса переносится только по
//! дням (`shift_external`); мутации на MouseUp. События вне диапазона
//! часов помечаются стрелками у верха/низа колонки.

use std::any::Any;
use std::time::{Duration, Instant};

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

use super::model::{fmt_hm, lanes};
use super::view::{color_of, day_weekday, GridData, Palette};
use super::{CalendarEnv, CalendarHandle, ElementBase};
use crate::pages::notes::gantt::calendar::civil_from_days;

const HEADER_H: f32 = 32.0;
const GUTTER_W: f32 = 48.0;
const ALLDAY_ROW_H: f32 = 18.0;
const EDGE_PX: f32 = 6.0;
const DRAG_THRESHOLD: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub struct TimeGrid {
    pub env: CalendarEnv,
    pub handle: CalendarHandle,
    pub data: GridData,
}

impl Widget for TimeGrid {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(TimeElement {
            base: ElementBase::new(),
            env: self.env.clone(),
            handle: self.handle.clone(),
            data: self.data.clone(),
            drag: None,
            hover: None,
            last_click: None,
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

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    Resize,
}

/// Что тащим: событие календаря либо внешнюю полосу (индекс в
/// `data.external`).
#[derive(Clone, Debug, PartialEq)]
enum DragTarget {
    Event(String),
    External(usize),
}

struct EventDrag {
    target: DragTarget,
    mode: DragMode,
    start: Point,
    moved: bool,
    /// Предпросмотр: сдвиг в днях и минутах (Move) либо новый конец (Resize).
    d_days: i64,
    d_min: i64,
    day: i64,
    time: (u32, u32),
}

/// Прямоугольник события или внешней полосы в сетке.
#[derive(Clone, Debug)]
struct Slot {
    rect: Rect,
    event: Option<String>,
    external: Option<usize>,
    day: i64,
    all_day: bool,
}

impl Slot {
    fn target(&self) -> Option<DragTarget> {
        match (&self.event, self.external) {
            (Some(id), _) => Some(DragTarget::Event(id.clone())),
            (None, Some(i)) => Some(DragTarget::External(i)),
            _ => None,
        }
    }
}

pub struct TimeElement {
    base: ElementBase,
    env: CalendarEnv,
    handle: CalendarHandle,
    data: GridData,
    drag: Option<EventDrag>,
    hover: Option<Point>,
    last_click: Option<(Instant, i64, u32)>,
}

impl TimeElement {
    fn days(&self) -> Vec<i64> {
        (self.data.range.0..=self.data.range.1).collect()
    }

    fn slot_h(&self) -> f32 {
        if self.data.doc.style.compact { 18.0 } else { 24.0 }
    }

    fn slots_count(&self) -> u32 {
        let s = &self.data.doc.style;
        ((s.hour_to - s.hour_from) * 60 / s.slot_min).max(1)
    }

    fn allday_rows(&self) -> usize {
        self.days()
            .iter()
            .map(|&d| {
                self.data.occurrences.iter().filter(|o| o.day == d && o.time.is_none()).count()
                    + self.data.external.iter().filter(|e| e.time.is_none() && d >= e.day && d <= e.end_day).count()
            })
            .max()
            .unwrap_or(0)
            .min(4)
    }

    fn allday_h(&self) -> f32 {
        (self.allday_rows() as f32 * ALLDAY_ROW_H + 6.0).max(22.0)
    }

    fn grid_top(&self) -> f32 {
        self.base.bounds.origin.y + HEADER_H + self.allday_h()
    }

    fn col_w(&self) -> f32 {
        ((self.base.bounds.size.width - GUTTER_W) / self.days().len().max(1) as f32).max(20.0)
    }

    fn col_x(&self, day: i64) -> f32 {
        self.base.bounds.origin.x + GUTTER_W + (day - self.data.range.0) as f32 * self.col_w()
    }

    fn y_of(&self, min: u32) -> f32 {
        let s = &self.data.doc.style;
        self.grid_top() + (min as f32 - (s.hour_from * 60) as f32) / s.slot_min as f32 * self.slot_h()
    }

    fn min_at(&self, y: f32) -> i64 {
        let s = &self.data.doc.style;
        let slots = ((y - self.grid_top()) / self.slot_h()).floor() as i64;
        (s.hour_from * 60) as i64 + slots * s.slot_min as i64
    }

    fn day_at(&self, x: f32) -> Option<i64> {
        let i = ((x - self.base.bounds.origin.x - GUTTER_W) / self.col_w()).floor() as i64;
        (i >= 0 && i < self.days().len() as i64).then(|| self.data.range.0 + i)
    }

    /// Прямоугольники событий: ряд «весь день» и полосы по часам с дорожками.
    fn slots(&self) -> Vec<Slot> {
        let mut out = Vec::new();
        let col_w = self.col_w();
        let style = &self.data.doc.style;
        let (from_min, to_min) = ((style.hour_from * 60) as i64, (style.hour_to * 60) as i64);
        let allday_top = self.base.bounds.origin.y + HEADER_H + 3.0;
        for day in self.days() {
            let x = self.col_x(day);
            // Весь день.
            let mut row = 0usize;
            for o in self.data.occurrences.iter().filter(|o| o.day == day && o.time.is_none()) {
                if row >= 4 {
                    break;
                }
                out.push(Slot {
                    rect: Rect::new(Point::new(x + 2.0, allday_top + row as f32 * ALLDAY_ROW_H), Size::new(col_w - 4.0, ALLDAY_ROW_H - 2.0)),
                    event: Some(o.event.clone()),
                    external: None,
                    day,
                    all_day: true,
                });
                row += 1;
            }
            for (i, e) in self.data.external.iter().enumerate() {
                if e.time.is_some() || day < e.day || day > e.end_day || row >= 4 {
                    continue;
                }
                out.push(Slot {
                    rect: Rect::new(Point::new(x + 2.0, allday_top + row as f32 * ALLDAY_ROW_H), Size::new(col_w - 4.0, ALLDAY_ROW_H - 2.0)),
                    event: None,
                    external: Some(i),
                    day,
                    all_day: true,
                });
                row += 1;
            }
            // По часам — события и внешние полосы со временем, вместе в
            // одной упаковке дорожек: задача доски не наезжает на встречу.
            let timed: Vec<(Option<String>, Option<usize>, (u32, u32))> = self
                .data
                .occurrences
                .iter()
                .filter(|o| o.day == day)
                .filter_map(|o| o.time.map(|t| (Some(o.event.clone()), None, t)))
                .chain(
                    self.data
                        .external
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.day == day)
                        .filter_map(|(i, e)| e.time.map(|t| (None, Some(i), t))),
                )
                .collect();
            let intervals: Vec<(u32, u32)> = timed.iter().map(|(_, _, t)| *t).collect();
            let packed = lanes(&intervals);
            for (k, (id, ext, (s, e))) in timed.iter().enumerate() {
                let (lane, of) = packed[k];
                let s = (*s as i64).clamp(from_min, to_min - 5) as u32;
                let e = (*e as i64).clamp(s as i64 + 5, to_min) as u32;
                let lane_w = (col_w - 4.0) / of as f32;
                let rect = Rect::new(
                    Point::new(x + 2.0 + lane as f32 * lane_w, self.y_of(s)),
                    Size::new((lane_w - 2.0).max(6.0), (self.y_of(e) - self.y_of(s)).max(self.slot_h() * 0.5)),
                );
                out.push(Slot { rect, event: id.clone(), external: *ext, day, all_day: false });
            }
        }
        out
    }

    fn slot_at(&self, p: Point) -> Option<Slot> {
        self.slots().into_iter().rev().find(|s| s.rect.contains(p))
    }

    /// Прямоугольник с учётом предпросмотра переноса/растяжения.
    fn preview_rect(&self, slot: &Slot) -> Rect {
        let Some(d) = self.drag.as_ref().filter(|d| d.moved && slot.target().as_ref() == Some(&d.target)) else {
            return slot.rect;
        };
        let mut r = slot.rect;
        match d.mode {
            DragMode::Move => {
                r.origin.x += d.d_days as f32 * self.col_w();
                if !slot.all_day {
                    r.origin.y += d.d_min as f32 / self.data.doc.style.slot_min as f32 * self.slot_h();
                }
            }
            DragMode::Resize => {
                let new_end = (d.time.1 as i64 + d.d_min).max(d.time.0 as i64 + self.data.doc.style.slot_min as i64) as u32;
                r.size.height = (self.y_of(new_end) - r.origin.y).max(self.slot_h() * 0.5);
            }
        }
        r
    }

    fn commit_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        if !d.moved {
            return;
        }
        // Внешняя полоса живёт в доске или Ганте: её переносим целиком по
        // дням, время задачи остаётся её собственным.
        let id = match &d.target {
            DragTarget::External(i) => {
                if let Some(item) = self.data.external.get(*i) {
                    (self.env.shift_external)(&item.source, d.d_days);
                }
                return;
            }
            DragTarget::Event(id) => id.clone(),
        };
        match d.mode {
            DragMode::Move => {
                let day = d.day + d.d_days;
                let start = if self.data.store.event(&id).is_some_and(|e| e.all_day) {
                    None
                } else {
                    Some(((d.time.0 as i64 + d.d_min).clamp(0, 24 * 60 - 5)) as u32)
                };
                self.env.store.move_event(&id, day, start);
            }
            DragMode::Resize => {
                let slot_min = self.data.doc.style.slot_min as i64;
                let new_end = ((d.time.1 as i64 + d.d_min).max(d.time.0 as i64 + slot_min)).min(24 * 60) as u32;
                self.env.store.update_event(&id, |e| e.end = Some(new_end));
            }
        }
    }

    fn open_edit(&self, id: &str, anchor: Rect) {
        if let Some(e) = self.data.store.event(id) {
            self.handle.open_edit(e.clone(), anchor);
        }
    }
}

impl Element for TimeElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<TimeGrid>() else { return };
        self.handle = w.handle.clone();
        self.env = w.env.clone();
        self.data = w.data.clone();
        self.base.dirty |= DirtyFlags::LAYOUT | DirtyFlags::RENDER;
        ctx.mark_layout_dirty();
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 700.0 };
        let h = HEADER_H + self.allday_h() + self.slots_count() as f32 * self.slot_h() + 8.0;
        self.base.bounds.size = Size::new(w, h);
        self.base.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let b = self.base.bounds;
        let style = &self.data.doc.style;
        let pal = Palette::resolve(style, &self.base.mss);
        let font = style.font_size;
        let col_w = self.col_w();
        let days = self.days();
        let grid_top = self.grid_top();
        let slot_h = self.slot_h();
        let slots = self.slots_count();
        list.push_clip(b);
        if pal.cell_bg.a > 0.0 {
            list.push_rect(b, pal.cell_bg, [0.0; 4]);
        }
        // Шапка.
        list.push_rect(Rect::new(b.origin, Size::new(b.size.width, HEADER_H)), pal.header_bg, [0.0; 4]);
        for &day in &days {
            let x = self.col_x(day);
            let (_, m, d) = civil_from_days(day);
            let wd = day_weekday(day);
            let is_today = day == self.data.today;
            if self.data.locale.is_weekend(wd) {
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots as f32 * slot_h)), pal.weekend, [0.0; 4]);
            }
            if is_today {
                list.push_rect(Rect::new(Point::new(x, b.origin.y), Size::new(col_w, HEADER_H)), pal.today.with_alpha(0.18), [0.0; 4]);
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots as f32 * slot_h)), pal.today.with_alpha(0.05), [0.0; 4]);
            }
            if self.data.selected_day == Some(day) && days.len() > 1 {
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots as f32 * slot_h)), pal.accent.with_alpha(0.06), [0.0; 4]);
            }
            let label = if days.len() > 1 {
                format!("{} {d}.{m:02}", self.data.locale.weekday_short(wd))
            } else {
                format!("{} {d} {}", self.data.locale.weekday_short(wd), self.data.locale.month_name(m))
            };
            list.push_text_styled_singleline(
                &label,
                Rect::new(Point::new(x + 4.0, b.origin.y + (HEADER_H - font - 4.0) / 2.0), Size::new(col_w - 8.0, font + 4.0)),
                if is_today { pal.today } else if self.data.locale.is_weekend(wd) { pal.muted } else { pal.text },
                font,
                TextAlign::CENTER,
                TextDecoration::None,
                if is_today { 700 } else { 600 },
                None,
            );
        }
        // Ряд «весь день» и часы: линии.
        let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
        c.set_stroke_width(1.0);
        c.set_color(pal.grid);
        c.draw_line(b.origin.x, b.origin.y + HEADER_H, b.origin.x + b.size.width, b.origin.y + HEADER_H);
        c.draw_line(b.origin.x, grid_top, b.origin.x + b.size.width, grid_top);
        let per_hour = (60 / style.slot_min).max(1);
        for i in 0..=slots {
            let y = grid_top + i as f32 * slot_h;
            let hour_line = i % per_hour == 0;
            c.set_color(if hour_line { pal.grid } else { pal.grid.with_alpha(pal.grid.a * 0.45) });
            c.draw_line(b.origin.x + GUTTER_W, y, b.origin.x + b.size.width, y);
            if hour_line && i < slots {
                let minute = style.hour_from * 60 + i / per_hour * 60;
                list.push_text_styled_singleline(
                    &fmt_hm(minute),
                    Rect::new(Point::new(b.origin.x + 4.0, y + 2.0), Size::new(GUTTER_W - 8.0, font + 2.0)),
                    pal.muted,
                    font - 2.0,
                    TextAlign::RIGHT,
                    TextDecoration::None,
                    500,
                    None,
                );
            }
        }
        for &day in &days {
            let x = self.col_x(day);
            c.set_color(pal.grid);
            c.draw_line(x, b.origin.y + HEADER_H, x, grid_top + slots as f32 * slot_h);
        }
        c.flush(list);

        // События.
        for slot in self.slots() {
            let rect = self.preview_rect(&slot);
            let dragged = self.drag.as_ref().is_some_and(|d| d.moved && slot.target().as_ref() == Some(&d.target));
            let (color, title, done, selected, sub) = match (&slot.event, slot.external) {
                (Some(id), _) => {
                    let e = self.data.store.event(id);
                    let color = e.map(|e| self.data.store.color_of(e)).and_then(|c| color_of(&c)).unwrap_or(pal.accent);
                    let occ = self.data.occurrences.iter().find(|o| &o.event == id && o.day == slot.day);
                    let sub = occ.and_then(|o| o.time).map(|(s, e)| format!("{}–{}", fmt_hm(s), fmt_hm(e))).unwrap_or_default();
                    (color, e.map(|e| e.title.clone()).unwrap_or_default(), e.is_some_and(|e| e.done), self.data.selected.as_deref() == Some(id.as_str()), sub)
                }
                (None, Some(i)) => {
                    let ext = &self.data.external[i];
                    let sub = ext.time.map(|(s, e)| format!("{}–{}", fmt_hm(s), fmt_hm(e))).unwrap_or_default();
                    (color_of(&ext.color).unwrap_or(pal.muted).with_alpha(0.7), format!("◆ {}", ext.title), false, false, sub)
                }
                _ => continue,
            };
            let alpha = if dragged { 0.55 } else if done { 0.45 } else { 1.0 };
            list.push_rect(rect, color.with_alpha(0.85 * alpha), [4.0; 4]);
            let white = Color::from_hex("#FFFFFF").with_alpha(alpha);
            list.push_text_styled_singleline(
                &title,
                Rect::new(Point::new(rect.origin.x + 5.0, rect.origin.y + 2.0), Size::new((rect.size.width - 8.0).max(4.0), font + 3.0)),
                white,
                font - 1.0,
                TextAlign::DEFAULT,
                if done { TextDecoration::LineThrough } else { TextDecoration::None },
                600,
                None,
            );
            if !slot.all_day && !sub.is_empty() && rect.size.height > font * 2.0 + 8.0 {
                list.push_text_styled_singleline(
                    &sub,
                    Rect::new(Point::new(rect.origin.x + 5.0, rect.origin.y + font + 5.0), Size::new((rect.size.width - 8.0).max(4.0), font + 2.0)),
                    white.with_alpha(0.8 * alpha),
                    font - 2.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    400,
                    None,
                );
            }
            if selected {
                let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                k.set_color(pal.accent);
                k.set_stroke_width(1.5);
                k.draw_rect(rect.origin.x - 1.0, rect.origin.y - 1.0, rect.size.width + 2.0, rect.size.height + 2.0);
                k.flush(list);
            }
        }
        // События вне диапазона часов — стрелки у верха/низа колонки.
        let (from_min, to_min) = (style.hour_from * 60, style.hour_to * 60);
        for &day in &days {
            let x = self.col_x(day);
            let mut above = false;
            let mut below = false;
            for o in self.data.occurrences.iter().filter(|o| o.day == day) {
                if let Some((s, e)) = o.time {
                    above |= e <= from_min;
                    below |= s >= to_min;
                }
            }
            if above {
                list.push_text_styled_singleline("▲", Rect::new(Point::new(x + col_w - 16.0, grid_top + 2.0), Size::new(14.0, 12.0)), pal.accent, 9.0, TextAlign::CENTER, TextDecoration::None, 700, None);
            }
            if below {
                list.push_text_styled_singleline("▼", Rect::new(Point::new(x + col_w - 16.0, grid_top + slots as f32 * slot_h - 14.0), Size::new(14.0, 12.0)), pal.accent, 9.0, TextAlign::CENTER, TextDecoration::None, 700, None);
            }
        }
        // Линия «сейчас».
        if days.contains(&self.data.today) && (from_min..=to_min).contains(&self.data.now_min) {
            let x = self.col_x(self.data.today);
            let y = self.y_of(self.data.now_min);
            list.push_rect(Rect::new(Point::new(x, y - 1.0), Size::new(col_w, 2.0)), pal.today.with_alpha(0.9), [1.0; 4]);
            list.push_rect(Rect::new(Point::new(x - 3.0, y - 3.5), Size::new(7.0, 7.0)), pal.today, [3.5; 4]);
        }
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.base.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                if let Some(target) = self.slot_at(*position).and_then(|slot| slot.target().map(|t| (slot, t))) {
                    let (slot, target) = target;
                    let (time, mode) = match &target {
                        DragTarget::Event(id) => {
                            let time = self.data.occurrences.iter().find(|o| &o.event == id && o.day == slot.day).and_then(|o| o.time).unwrap_or((0, 0));
                            let mode = if !slot.all_day && position.y > slot.rect.origin.y + slot.rect.size.height - EDGE_PX {
                                DragMode::Resize
                            } else {
                                DragMode::Move
                            };
                            self.handle.select(Some(id.clone()));
                            (time, mode)
                        }
                        // Внешнюю полосу за кромку не растягиваем: её длина
                        // — оценка задачи, она правится в её карточке.
                        DragTarget::External(_) => ((0, 0), DragMode::Move),
                    };
                    self.drag = Some(EventDrag { target, mode, start: *position, moved: false, d_days: 0, d_min: 0, day: slot.day, time });
                    ctx.capture();
                    return EventResult::Handled;
                }
                let Some(day) = self.day_at(position.x) else { return EventResult::Ignored };
                let in_hours = position.y >= self.grid_top();
                let start = if in_hours { self.min_at(position.y).clamp(0, 24 * 60 - 5) as u32 } else { 0 };
                let now = Instant::now();
                let double = self.last_click.as_ref().is_some_and(|(t, d, s)| *d == day && *s == start && now.duration_since(*t) < DOUBLE_CLICK);
                self.last_click = Some((now, day, start));
                self.handle.select(None);
                if double {
                    let cal = self.data.doc.calendars.first().cloned().or_else(|| self.data.store.calendars.first().map(|c| c.id.clone())).unwrap_or_default();
                    let anchor = Rect::new(
                        Point::new(self.col_x(day), if in_hours { self.y_of(start) } else { self.base.bounds.origin.y + HEADER_H }),
                        Size::new(self.col_w(), self.slot_h()),
                    );
                    self.handle.open_new(&cal, day, in_hours.then_some(start), self.data.doc.style.slot_min, anchor);
                } else if self.handle.selected_day.get_untracked() != Some(day) {
                    self.handle.selected_day.set(Some(day));
                }
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                let col_w = self.col_w();
                let slot_h = self.slot_h();
                let slot_min = self.data.doc.style.slot_min as i64;
                if let Some(d) = &mut self.drag {
                    if (position.x - d.start.x).abs() > DRAG_THRESHOLD || (position.y - d.start.y).abs() > DRAG_THRESHOLD {
                        d.moved = true;
                    }
                    if d.moved {
                        let dd = ((position.x - d.start.x) / col_w).round() as i64;
                        let dm = ((position.y - d.start.y) / slot_h).round() as i64 * slot_min;
                        d.d_days = if d.mode == DragMode::Move { dd } else { 0 };
                        d.d_min = dm;
                        let resize = d.mode == DragMode::Resize;
                        ctx.set_cursor(if resize { CursorIcon::RowResize } else { CursorIcon::Grabbing });
                        self.base.dirty |= DirtyFlags::RENDER;
                    }
                    return EventResult::Handled;
                }
                let inside = self.base.bounds.contains(*position);
                self.hover = inside.then_some(*position);
                if inside {
                    if let Some(slot) = self.slot_at(*position) {
                        let resize = !slot.all_day && slot.event.is_some() && position.y > slot.rect.origin.y + slot.rect.size.height - EDGE_PX;
                        ctx.set_cursor(if resize { CursorIcon::RowResize } else { CursorIcon::Pointer });
                    }
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let Some(d) = self.drag.as_ref() else { return EventResult::Ignored };
                if d.moved {
                    self.commit_drag();
                } else {
                    let target = d.target.clone();
                    self.drag = None;
                    match target {
                        DragTarget::Event(id) => {
                            let anchor = self.slot_at(*position).map(|s| s.rect).unwrap_or(self.base.bounds);
                            self.open_edit(&id, anchor);
                        }
                        DragTarget::External(i) => {
                            if let Some(page) = self.data.external.get(i).map(|e| e.page.clone()) {
                                (self.env.open_page)(&page);
                            }
                        }
                    }
                }
                self.base.dirty |= DirtyFlags::RENDER;
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    element_boilerplate!("notes-calendar-timegrid");
}
