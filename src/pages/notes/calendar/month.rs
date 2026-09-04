//! Месячная сетка календаря — собственный элемент: 6 недель × 7 дней,
//! номера недель по флагу, чипы событий (`chip` / `dot` / `bar`), «+n»,
//! чекбокс «сделано» у чипа, внешний слой (сроки досок, задачи Ганта)
//! приглушёнными чипами.
//!
//! Жесты: клик по дню — выбор дня, двойной — новое событие (весь день);
//! клик по чипу — попап правки, перенос чипа на другой день — `move_event`
//! (мутация на MouseUp, до него — локальный предпросмотр); клик по
//! квадратику — переключить «сделано»; клик по внешнему чипу — открыть
//! его страницу.

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

use super::model::{fmt_hm, EventStyle};
use super::view::{color_of, day_weekday, GridData, Palette};
use super::{CalendarEnv, CalendarHandle, ElementBase};
use crate::pages::notes::gantt::calendar::civil_from_days;

const HEADER_H: f32 = 22.0;
const DAY_ROW_H: f32 = 20.0;
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
            hover_day: None,
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
    event: String,
    start: Point,
    moved: bool,
    /// День под курсором.
    day: i64,
}

/// Чип в сетке: событие (id) либо внешний элемент (индекс).
#[derive(Clone, Debug)]
struct Chip {
    rect: Rect,
    day: i64,
    event: Option<String>,
    external: Option<usize>,
    done_box: Option<Rect>,
}

pub struct MonthElement {
    base: ElementBase,
    env: CalendarEnv,
    handle: CalendarHandle,
    data: GridData,
    hover_day: Option<i64>,
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

    /// Чипы всех дней (сверху вниз внутри ячейки), плюс «+n» считается при
    /// отрисовке.
    fn chips(&self) -> Vec<Chip> {
        let mut out = Vec::new();
        let (from, to) = self.data.range;
        let (_, ch) = self.cell_size();
        let chip_h = self.chip_h();
        let max = (((ch - DAY_ROW_H - 4.0) / (chip_h + 2.0)).floor() as usize).max(1);
        let style = self.data.doc.style.event_style;
        for day in from..=to {
            let Some(cell) = self.cell_rect(day) else { continue };
            let mut y = cell.origin.y + DAY_ROW_H + 2.0;
            let mut count = 0usize;
            let pad = if style == EventStyle::Bar { 0.0 } else { 3.0 };
            let items: Vec<(Option<String>, Option<usize>)> = self
                .data
                .occurrences
                .iter()
                .filter(|o| o.day == day)
                .map(|o| (Some(o.event.clone()), None))
                .chain(self.data.external.iter().enumerate().filter(|(_, e)| day >= e.day && day <= e.end_day).map(|(i, _)| (None, Some(i))))
                .collect();
            for (event, external) in items {
                if count >= max {
                    break;
                }
                let rect = Rect::new(Point::new(cell.origin.x + pad, y), Size::new(cell.size.width - pad * 2.0, chip_h));
                let done_box = (event.is_some() && style != EventStyle::Dot)
                    .then(|| Rect::new(Point::new(rect.origin.x + 4.0, rect.origin.y + (chip_h - 10.0) / 2.0), Size::new(10.0, 10.0)));
                out.push(Chip { rect, day, event, external, done_box });
                y += chip_h + 2.0;
                count += 1;
            }
        }
        out
    }

    fn overflow(&self, day: i64, shown: usize) -> usize {
        let total = self.data.occurrences.iter().filter(|o| o.day == day).count()
            + self.data.external.iter().filter(|e| day >= e.day && day <= e.end_day).count();
        total.saturating_sub(shown)
    }

    fn chip_at(&self, p: Point) -> Option<Chip> {
        self.chips().into_iter().rev().find(|c| c.rect.contains(p))
    }

    fn open_edit(&self, id: &str, anchor: Rect) {
        if let Some(e) = self.data.store.event(id) {
            self.handle.open_edit(e.clone(), anchor);
        }
    }
}

impl Element for MonthElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<MonthGrid>() else { return };
        self.handle = w.handle.clone();
        self.env = w.env.clone();
        self.data = w.data.clone();
        self.base.dirty |= DirtyFlags::LAYOUT | DirtyFlags::RENDER;
        ctx.mark_layout_dirty();
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

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
            if self.hover_day == Some(day) && self.drag.as_ref().is_some_and(|dr| dr.moved) {
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
                    Color::from_hex("#FFFFFF"),
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

        // Чипы.
        let chips = self.chips();
        let mut shown_per_day: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        for chip in &chips {
            *shown_per_day.entry(chip.day).or_default() += 1;
            let dragged = self.drag.as_ref().is_some_and(|d| d.moved && chip.event.as_deref() == Some(d.event.as_str()));
            let mut rect = chip.rect;
            if dragged {
                if let (Some(target), Some(src)) = (self.hover_day.and_then(|d| self.cell_rect(d)), self.cell_rect(chip.day)) {
                    rect.origin.x += target.origin.x - src.origin.x;
                    rect.origin.y += target.origin.y - src.origin.y;
                }
            }
            let (color, title, done, selected) = match (&chip.event, chip.external) {
                (Some(id), _) => {
                    let e = self.data.store.event(id);
                    let color = e.map(|e| self.data.store.color_of(e)).and_then(|c| color_of(&c)).unwrap_or(pal.accent);
                    let occ = self.data.occurrences.iter().find(|o| &o.event == id && o.day == chip.day);
                    let mut title = e.map(|e| e.title.clone()).unwrap_or_default();
                    if let Some((s, _)) = occ.and_then(|o| o.time) {
                        title = format!("{} {title}", fmt_hm(s));
                    }
                    if occ.is_some_and(|o| !o.first) {
                        title = format!("… {title}");
                    }
                    (color, title, e.is_some_and(|e| e.done), self.data.selected.as_deref() == Some(id.as_str()))
                }
                (None, Some(i)) => {
                    let ext = &self.data.external[i];
                    (color_of(&ext.color).unwrap_or(pal.muted).with_alpha(0.7), format!("◆ {}", ext.title), false, false)
                }
                _ => continue,
            };
            let alpha = if dragged { 0.5 } else if done { 0.45 } else { 1.0 };
            let text_x_pad = if chip.done_box.is_some() { 18.0 } else { 6.0 };
            match self.data.doc.style.event_style {
                EventStyle::Chip => {
                    list.push_rect(rect, color.with_alpha(0.85 * alpha), [4.0; 4]);
                }
                EventStyle::Bar => {
                    list.push_rect(rect, color.with_alpha(0.9 * alpha), [0.0; 4]);
                }
                EventStyle::Dot => {
                    let r = 3.0;
                    list.push_rect(
                        Rect::new(Point::new(rect.origin.x + 4.0, rect.origin.y + rect.size.height / 2.0 - r), Size::new(r * 2.0, r * 2.0)),
                        color.with_alpha(alpha),
                        [r; 4],
                    );
                }
            }
            if let Some(bx) = chip.done_box {
                let mut bx = bx;
                bx.origin.x += rect.origin.x - chip.rect.origin.x;
                bx.origin.y += rect.origin.y - chip.rect.origin.y;
                list.push_rect(bx, Color::from_hex("#FFFFFF").with_alpha(if done { 0.9 } else { 0.35 }), [2.0; 4]);
                if done {
                    let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                    k.set_color(color.with_alpha(1.0));
                    k.set_stroke_width(1.6);
                    k.draw_polyline(&[(bx.origin.x + 2.0, bx.origin.y + 5.0), (bx.origin.x + 4.5, bx.origin.y + 8.0), (bx.origin.x + 8.5, bx.origin.y + 2.5)]);
                    k.flush(list);
                }
            }
            let text_color = if self.data.doc.style.event_style == EventStyle::Dot { pal.text } else { Color::from_hex("#FFFFFF") };
            list.push_text_styled_singleline(
                &title,
                Rect::new(
                    Point::new(rect.origin.x + text_x_pad, rect.origin.y + (rect.size.height - font - 2.0) / 2.0),
                    Size::new((rect.size.width - text_x_pad - 4.0).max(4.0), font + 4.0),
                ),
                text_color.with_alpha(alpha),
                font - 1.0,
                TextAlign::DEFAULT,
                if done { TextDecoration::LineThrough } else { TextDecoration::None },
                500,
                None,
            );
            if selected {
                let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                k.set_color(pal.accent);
                k.set_stroke_width(1.5);
                k.draw_rect(rect.origin.x - 1.0, rect.origin.y - 1.0, rect.size.width + 2.0, rect.size.height + 2.0);
                k.flush(list);
            }
        }
        // «+n».
        for day in from..=to {
            let shown = shown_per_day.get(&day).copied().unwrap_or(0);
            let more = self.overflow(day, shown);
            if more == 0 {
                continue;
            }
            let Some(cell) = self.cell_rect(day) else { continue };
            list.push_text_styled_singleline(
                &format!("+{more}"),
                Rect::new(Point::new(cell.origin.x + 4.0, cell.origin.y + cell.size.height - font - 6.0), Size::new(cw - 8.0, font + 4.0)),
                pal.muted,
                font - 2.0,
                TextAlign::DEFAULT,
                TextDecoration::None,
                600,
                None,
            );
        }
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.base.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                if let Some(chip) = self.chip_at(*position) {
                    if let Some(id) = chip.event {
                        if chip.done_box.is_some_and(|bx| bx.contains(*position)) {
                            let done = self.data.store.event(&id).is_some_and(|e| e.done);
                            self.env.store.update_event(&id, |e| e.done = !done);
                            return EventResult::Handled;
                        }
                        self.handle.select(Some(id.clone()));
                        self.drag = Some(ChipDrag { event: id, start: *position, moved: false, day: chip.day });
                        ctx.capture();
                        return EventResult::Handled;
                    }
                    if let Some(i) = chip.external {
                        let page = self.data.external[i].page.clone();
                        (self.env.open_page)(&page);
                        return EventResult::Handled;
                    }
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
                let inside = self.base.bounds.contains(*position);
                if let Some(drag) = &mut self.drag {
                    if (position.x - drag.start.x).abs() > DRAG_THRESHOLD || (position.y - drag.start.y).abs() > DRAG_THRESHOLD {
                        drag.moved = true;
                    }
                    if drag.moved {
                        self.hover_day = self.day_at(*position);
                        ctx.set_cursor(CursorIcon::Grabbing);
                        self.base.dirty |= DirtyFlags::RENDER;
                    }
                    return EventResult::Handled;
                }
                if inside && self.chip_at(*position).is_some() {
                    ctx.set_cursor(CursorIcon::Pointer);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let Some(drag) = self.drag.take() else { return EventResult::Ignored };
                if drag.moved {
                    if let Some(day) = self.day_at(*position).filter(|d| *d != drag.day) {
                        self.env.store.move_event(&drag.event, day, None);
                    }
                } else {
                    let anchor = self.chip_at(*position).map(|c| c.rect).unwrap_or(self.base.bounds);
                    self.open_edit(&drag.event, anchor);
                }
                self.hover_day = None;
                self.base.dirty |= DirtyFlags::RENDER;
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    element_boilerplate!("notes-calendar-month");
}
