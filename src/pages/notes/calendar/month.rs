//! Месячная сетка календаря — собственный элемент: 6 недель × 7 дней,
//! номера недель по флагу, полосы событий по дорожкам ([`layout`]):
//! многодневное — одна полоса через ячейки (плоский край там, где она
//! продолжается за строку), однодневное «весь день» — заливка, со
//! временем — точка + время + название (стиль «плашки»; «полосы» —
//! заливка всем, «точки» — всем точка), задачи досок и Ганта — подложка
//! с полоской слева. Текст режется по ширине с многоточием. Что не
//! влезло в ячейку — «ещё n», клик открывает список дня.
//!
//! Жесты: клик по дню — выбор дня, двойной — новое событие (весь день);
//! клик по полосе — попап правки, перенос полосы на другой день —
//! `move_event` на разницу дней (мутация на MouseUp, до него — локальный
//! предпросмотр); клик по маркеру слева — переключить «сделано»; клик по
//! внешней полосе — открыть её страницу; наведение подсвечивает полосу.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::core::canvas::CanvasContext;
use syngui::core::{Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, TextMeasure, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

use super::layout::{self, ItemRef};
use super::paint::{self, Bar, Look};
use super::view::{color_of, day_weekday, GridData, Palette};
use super::{CalendarEnv, CalendarHandle, ElementBase};
use crate::pages::notes::gantt::calendar::civil_from_days;

const HEADER_H: f32 = 22.0;
const DAY_ROW_H: f32 = 20.0;
const PAD_X: f32 = 3.0;
const LANE_GAP: f32 = 2.0;
const DRAG_THRESHOLD: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub struct MonthGrid {
    pub env: CalendarEnv,
    pub handle: CalendarHandle,
    pub data: GridData,
}

impl Widget for MonthGrid {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(MonthElement {
            base: ElementBase::new(),
            env: self.env.clone(),
            handle: self.handle.clone(),
            data: self.data.clone(),
            tm: None,
            hover: None,
            drag: None,
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

struct ChipDrag {
    target: ItemRef,
    start: Point,
    moved: bool,
    /// День, за который взялись.
    grab_day: i64,
    /// День под курсором.
    over: Option<i64>,
}

/// Видимый кусок полосы в сетке.
#[derive(Clone, Debug)]
struct Chip {
    rect: Rect,
    item: ItemRef,
    /// Первый день куска.
    day: i64,
    look: Look,
    /// Время начала — подпись (однодневные со временем).
    time: Option<u32>,
    done_box: Option<Rect>,
    flat_left: bool,
    flat_right: bool,
}

/// «Ещё n» в ячейке дня.
#[derive(Clone, Debug)]
struct More {
    rect: Rect,
    day: i64,
    n: usize,
}

struct Arrangement {
    chips: Vec<Chip>,
    more: Vec<More>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    Chip(usize),
    More(usize),
}

pub struct MonthElement {
    base: ElementBase,
    env: CalendarEnv,
    handle: CalendarHandle,
    data: GridData,
    tm: Option<Arc<dyn TextMeasure>>,
    hover: Option<Hover>,
    drag: Option<ChipDrag>,
    last_click: Option<(Instant, i64)>,
}

impl MonthElement {
    fn gutter(&self) -> f32 {
        if self.data.doc.style.show_week_numbers { 26.0 } else { 0.0 }
    }

    fn cell_size(&self) -> (f32, f32) {
        let b = self.base.bounds;
        (((b.size.width - self.gutter()) / 7.0).max(20.0), ((b.size.height - HEADER_H) / 6.0).max(30.0))
    }

    fn cell_rect(&self, day: i64) -> Option<Rect> {
        let (from, _) = self.data.range;
        let idx = day - from;
        if !(0..42).contains(&idx) {
            return None;
        }
        let (cw, ch) = self.cell_size();
        let b = self.base.bounds;
        let row = (idx / 7) as f32;
        let col = (idx % 7) as f32;
        Some(Rect::new(Point::new(b.origin.x + self.gutter() + col * cw, b.origin.y + HEADER_H + row * ch), Size::new(cw, ch)))
    }

    fn day_at(&self, p: Point) -> Option<i64> {
        let b = self.base.bounds;
        let (cw, ch) = self.cell_size();
        let x = p.x - b.origin.x - self.gutter();
        let y = p.y - b.origin.y - HEADER_H;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (col, row) = ((x / cw).floor() as i64, (y / ch).floor() as i64);
        (col < 7 && row < 6).then(|| self.data.range.0 + row * 7 + col)
    }

    fn chip_h(&self) -> f32 {
        let s = &self.data.doc.style;
        (s.font_size + if s.compact { 4.0 } else { 8.0 }).round()
    }

    /// Дорожек влезает в ячейку под номером дня (последняя — под «ещё n»
    /// при нехватке).
    fn capacity(&self) -> usize {
        let (_, ch) = self.cell_size();
        (((ch - DAY_ROW_H - 2.0) / (self.chip_h() + LANE_GAP)).floor() as usize).max(1)
    }

    /// Полосы и «ещё n» всех строк.
    fn arrange(&self) -> Arrangement {
        let (from, _) = self.data.range;
        let (cw, _) = self.cell_size();
        let chip_h = self.chip_h();
        let capacity = self.capacity();
        let style = self.data.doc.style.event_style;
        let segs = layout::segments(&self.data.occurrences, &self.data.external);
        let mut chips = Vec::new();
        let mut more = Vec::new();
        for row in 0..6i64 {
            let row_start = from + row * 7;
            let placed = layout::pack_row(&segs, row_start, 7);
            let (runs, hidden) = layout::visible(&placed, 7, capacity);
            for run in runs {
                let p = &placed[run.placed];
                let s = &segs[p.seg];
                let (Some(c0), Some(c1)) = (self.cell_rect(row_start + run.col0 as i64), self.cell_rect(row_start + run.col1 as i64)) else { continue };
                let flat_left = (run.col0 == p.col0 && p.cont_left) || run.col0 > p.col0;
                let flat_right = (run.col1 == p.col1 && p.cont_right) || run.col1 < p.col1;
                let x0 = c0.origin.x + if flat_left { 0.0 } else { PAD_X };
                let x1 = c1.origin.x + c1.size.width - if flat_right { 0.0 } else { PAD_X };
                let y = c0.origin.y + DAY_ROW_H + 2.0 + p.lane as f32 * (chip_h + LANE_GAP);
                let rect = Rect::new(Point::new(x0, y), Size::new((x1 - x0).max(4.0), chip_h));
                let is_event = s.is_event();
                let look = Look::of(style, is_event, s.time.is_none(), s.multi_day());
                let done_box = match look {
                    Look::Filled if is_event => Some(Rect::new(Point::new(rect.origin.x + 5.0, rect.origin.y + ((chip_h - 10.0) / 2.0).round()), Size::new(10.0, 10.0))),
                    Look::Plain if is_event => Some(Rect::new(Point::new(rect.origin.x + 1.0, rect.origin.y + ((chip_h - 16.0) / 2.0).round()), Size::new(16.0, 16.0))),
                    _ => None,
                };
                chips.push(Chip {
                    rect,
                    item: s.item.clone(),
                    day: row_start + run.col0 as i64,
                    look,
                    time: if s.multi_day() { None } else { s.time.map(|t| t.0) },
                    done_box,
                    flat_left,
                    flat_right,
                });
            }
            for (col, &n) in hidden.iter().enumerate() {
                if n == 0 {
                    continue;
                }
                let Some(cell) = self.cell_rect(row_start + col as i64) else { continue };
                let y = cell.origin.y + DAY_ROW_H + 2.0 + (capacity - 1) as f32 * (chip_h + LANE_GAP);
                more.push(More { rect: Rect::new(Point::new(cell.origin.x + PAD_X, y), Size::new(cw - PAD_X * 2.0, chip_h)), day: row_start + col as i64, n });
            }
        }
        Arrangement { chips, more }
    }

    fn hit(&self, arr: &Arrangement, p: Point) -> Option<Hover> {
        if let Some(i) = arr.more.iter().position(|m| m.rect.contains(p)) {
            return Some(Hover::More(i));
        }
        arr.chips.iter().rposition(|c| c.rect.contains(p)).map(Hover::Chip)
    }

    fn open_edit(&self, id: &str, anchor: Rect) {
        if let Some(e) = self.data.store.event(id) {
            self.handle.open_edit(e.clone(), anchor);
        }
    }

    /// Сдвиг предпросмотра переноса: из ячейки захвата в ячейку под
    /// курсором.
    fn drag_offset(&self) -> Option<(f32, f32)> {
        let d = self.drag.as_ref().filter(|d| d.moved)?;
        let (src, dst) = (self.cell_rect(d.grab_day)?, self.cell_rect(d.over?)?);
        Some((dst.origin.x - src.origin.x, dst.origin.y - src.origin.y))
    }
}

impl Element for MonthElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<MonthGrid>() else { return };
        self.handle = w.handle.clone();
        self.env = w.env.clone();
        self.data = w.data.clone();
        self.hover = None;
        self.base.dirty |= DirtyFlags::LAYOUT | DirtyFlags::RENDER;
        ctx.mark_layout_dirty();
    }

    fn mount(&mut self, tree: &mut ElementTree) {
        self.tm = tree.text_measure.clone();
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 600.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height.max(HEADER_H + 6.0 * 30.0) } else { HEADER_H + 6.0 * 80.0 };
        self.base.bounds.size = Size::new(w, h);
        self.base.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let b = self.base.bounds;
        let style = &self.data.doc.style;
        let pal = Palette::resolve(style, &self.base.mss);
        let font = style.font_size;
        let (cw, ch) = self.cell_size();
        let gutter = self.gutter();
        list.push_clip(b);
        // Фон и шапка дней недели.
        if pal.cell_bg.a > 0.0 {
            list.push_rect(b, pal.cell_bg, [0.0; 4]);
        }
        list.push_rect(Rect::new(b.origin, Size::new(b.size.width, HEADER_H)), pal.header_bg, [0.0; 4]);
        for col in 0..7u32 {
            let wd = (style.first_weekday + col) % 7;
            let x = b.origin.x + gutter + col as f32 * cw;
            list.push_text_styled_singleline(
                self.data.locale.weekday_short(wd),
                Rect::new(Point::new(x, b.origin.y + 3.0), Size::new(cw, HEADER_H - 4.0)),
                if self.data.locale.is_weekend(wd) { pal.muted } else { pal.text },
                font - 1.0,
                TextAlign::CENTER,
                TextDecoration::None,
                600,
                None,
            );
        }
        // Ячейки.
        let (from, to) = self.data.range;
        let (_, anchor_m, _) = civil_from_days(self.data.doc.anchor_days());
        let drag_over = self.drag.as_ref().filter(|d| d.moved).and_then(|d| d.over);
        let mut c = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
        for day in from..=to {
            let Some(cell) = self.cell_rect(day) else { continue };
            let (_, m, d) = civil_from_days(day);
            let wd = day_weekday(day);
            let outside = m != anchor_m;
            if self.data.locale.is_weekend(wd) {
                list.push_rect(cell, pal.weekend, [0.0; 4]);
            }
            if self.data.selected_day == Some(day) {
                list.push_rect(cell, pal.accent.with_alpha(0.08), [0.0; 4]);
            }
            if drag_over == Some(day) {
                list.push_rect(cell, pal.accent.with_alpha(0.14), [0.0; 4]);
            }
            // Номер дня; сегодня — кружок.
            let num_rect = Rect::new(Point::new(cell.origin.x + 4.0, cell.origin.y + 3.0), Size::new(cw - 8.0, DAY_ROW_H - 4.0));
            if day == self.data.today {
                let r = 9.0;
                list.push_rect(
                    Rect::new(Point::new(cell.origin.x + 4.0, cell.origin.y + 2.0), Size::new(r * 2.0, r * 2.0)),
                    pal.today,
                    [r; 4],
                );
                list.push_text_styled_singleline(
                    &format!("{d}"),
                    Rect::new(Point::new(cell.origin.x + 4.0, cell.origin.y + 3.0), Size::new(r * 2.0, r * 2.0 - 2.0)),
                    paint::text_on(pal.today),
                    font - 1.0,
                    TextAlign::CENTER,
                    TextDecoration::None,
                    700,
                    None,
                );
            } else {
                list.push_text_styled_singleline(
                    &format!("{d}"),
                    num_rect,
                    if outside { pal.muted.with_alpha(0.35) } else if self.data.locale.is_weekend(wd) { pal.muted } else { pal.text },
                    font - 1.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    if outside { 400 } else { 600 },
                    None,
                );
            }
            // Сетка.
            c.set_color(pal.grid);
            c.set_stroke_width(1.0);
            c.draw_line(cell.origin.x, cell.origin.y, cell.origin.x + cw, cell.origin.y);
            c.draw_line(cell.origin.x, cell.origin.y, cell.origin.x, cell.origin.y + ch);
        }
        // Номера недель.
        if gutter > 0.0 {
            for row in 0..6 {
                let day = from + row * 7;
                let date = syngui::widgets::Date::new(civil_from_days(day).0 as i32, civil_from_days(day).1, civil_from_days(day).2);
                let y = b.origin.y + HEADER_H + row as f32 * ch;
                list.push_text_styled_singleline(
                    &format!("{}", date.iso_week()),
                    Rect::new(Point::new(b.origin.x, y + 4.0), Size::new(gutter - 4.0, 14.0)),
                    pal.muted,
                    font - 2.0,
                    TextAlign::CENTER,
                    TextDecoration::None,
                    500,
                    None,
                );
            }
        }
        c.flush(list);

        // Полосы: переносимая — последней, поверх остальных.
        let arr = self.arrange();
        let drag_target = self.drag.as_ref().filter(|d| d.moved).map(|d| d.target.clone());
        let offset = self.drag_offset();
        let mut order: Vec<usize> = (0..arr.chips.len()).collect();
        order.sort_by_key(|&i| drag_target.as_ref() == Some(&arr.chips[i].item));
        for i in order {
            let chip = &arr.chips[i];
            let dragged = drag_target.as_ref() == Some(&chip.item);
            let mut rect = chip.rect;
            let mut done_box = chip.done_box;
            if dragged {
                if let Some((dx, dy)) = offset {
                    rect.origin.x += dx;
                    rect.origin.y += dy;
                    if let Some(bx) = done_box.as_mut() {
                        bx.origin.x += dx;
                        bx.origin.y += dy;
                    }
                }
            }
            let (color, title, done, selected) = match &chip.item {
                ItemRef::Event(id) => {
                    let e = self.data.store.event(id);
                    let color = e.map(|e| self.data.store.color_of(e)).and_then(|c| color_of(&c)).unwrap_or(pal.accent);
                    (color, e.map(|e| e.title.clone()).unwrap_or_default(), e.is_some_and(|e| e.done), self.data.selected.as_deref() == Some(id.as_str()))
                }
                ItemRef::External(i) => {
                    let ext = &self.data.external[*i];
                    (color_of(&ext.color).unwrap_or(pal.muted), ext.title.clone(), false, false)
                }
            };
            paint::draw_bar(
                list,
                self.tm.as_ref(),
                &Bar {
                    rect,
                    color,
                    look: chip.look,
                    title: &title,
                    time: chip.time,
                    done,
                    selected,
                    hover: self.hover == Some(Hover::Chip(i)) && self.drag.is_none(),
                    alpha: if dragged { 0.55 } else { 1.0 },
                    flat_left: chip.flat_left,
                    flat_right: chip.flat_right,
                    done_box,
                    font,
                    text: pal.text,
                    accent: pal.accent,
                },
            );
        }
        for (i, m) in arr.more.iter().enumerate() {
            let hovered = self.hover == Some(Hover::More(i));
            if hovered {
                list.push_rect(m.rect, pal.text.with_alpha(0.07), [4.0; 4]);
            }
            paint::draw_more(list, m.rect, m.n, font, if hovered { pal.accent } else { pal.muted });
        }
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.base.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                let arr = self.arrange();
                match self.hit(&arr, *position) {
                    Some(Hover::More(i)) => {
                        let m = &arr.more[i];
                        let anchor = self.cell_rect(m.day).unwrap_or(m.rect);
                        self.handle.open_day(m.day, anchor);
                        return EventResult::Handled;
                    }
                    Some(Hover::Chip(i)) => {
                        let chip = &arr.chips[i];
                        if let ItemRef::Event(id) = &chip.item {
                            if chip.done_box.is_some_and(|bx| bx.contains(*position)) {
                                let done = self.data.store.event(id).is_some_and(|e| e.done);
                                self.env.store.update_event(id, |e| e.done = !done);
                                return EventResult::Handled;
                            }
                            self.handle.select(Some(id.clone()));
                        }
                        let grab_day = self.day_at(*position).unwrap_or(chip.day);
                        self.drag = Some(ChipDrag { target: chip.item.clone(), start: *position, moved: false, grab_day, over: None });
                        ctx.capture();
                        return EventResult::Handled;
                    }
                    None => {}
                }
                let Some(day) = self.day_at(*position) else { return EventResult::Ignored };
                let now = Instant::now();
                let double = self.last_click.as_ref().is_some_and(|(t, d)| *d == day && now.duration_since(*t) < DOUBLE_CLICK);
                self.last_click = Some((now, day));
                self.handle.select(None);
                if double {
                    let cal = self.data.doc.calendars.first().cloned().or_else(|| self.data.store.calendars.first().map(|c| c.id.clone())).unwrap_or_default();
                    let anchor = self.cell_rect(day).unwrap_or(self.base.bounds);
                    self.handle.open_new(&cal, day, None, self.data.doc.style.slot_min, anchor);
                } else if self.handle.selected_day.get_untracked() != Some(day) {
                    self.handle.selected_day.set(Some(day));
                }
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                let over = self.day_at(*position);
                if let Some(drag) = &mut self.drag {
                    if (position.x - drag.start.x).abs() > DRAG_THRESHOLD || (position.y - drag.start.y).abs() > DRAG_THRESHOLD {
                        drag.moved = true;
                    }
                    if drag.moved {
                        drag.over = over;
                        ctx.set_cursor(CursorIcon::Grabbing);
                        self.base.dirty |= DirtyFlags::RENDER;
                    }
                    return EventResult::Handled;
                }
                let hover = if self.base.bounds.contains(*position) { self.hit(&self.arrange(), *position) } else { None };
                if hover != self.hover {
                    self.hover = hover;
                    self.base.dirty |= DirtyFlags::RENDER;
                }
                if hover.is_some() {
                    ctx.set_cursor(CursorIcon::Pointer);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let Some(drag) = self.drag.take() else { return EventResult::Ignored };
                let delta = self.day_at(*position).map(|d| d - drag.grab_day).filter(|d| *d != 0);
                match (&drag.target, drag.moved) {
                    (ItemRef::Event(id), true) => {
                        // Перенос на разницу дней: полоса, взятая за середину,
                        // не прыгает началом в день броска.
                        if let (Some(delta), Some((start, _))) = (delta, self.data.store.event(id).and_then(|e| e.span())) {
                            self.env.store.move_event(id, start + delta, None);
                        }
                    }
                    (ItemRef::Event(id), false) => {
                        let arr = self.arrange();
                        let anchor = arr.chips.iter().rev().find(|c| c.rect.contains(*position)).map(|c| c.rect).unwrap_or(self.base.bounds);
                        self.open_edit(id, anchor);
                    }
                    // Внешняя полоса: перенос сдвигает её задачу на столько
                    // же дней, клик без переноса — открывает её страницу.
                    (ItemRef::External(i), moved) => {
                        let Some(item) = self.data.external.get(*i) else { return EventResult::Handled };
                        match (moved, delta) {
                            (true, Some(delta)) => {
                                (self.env.shift_external)(&item.source, delta);
                            }
                            (false, _) => (self.env.open_page)(&item.page.clone()),
                            _ => {}
                        }
                    }
                }
                self.base.dirty |= DirtyFlags::RENDER;
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    element_boilerplate!("notes-calendar-month");
}
