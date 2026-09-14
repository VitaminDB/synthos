//! Сетка часов — неделя (7 колонок) и день (1 колонка): шапка с днями,
//! ряд «весь день» (полосы по дорожкам через колонки — [`layout`], не
//! влезло — «ещё n» со списком дня), окно суток стиля слотами
//! `slot_min`, линия «сейчас» в сегодняшней колонке, события полосами с
//! упаковкой пересечений по дорожкам, внешний слой (задачи досок и
//! Ганта) подложкой с полоской: полоса с часами встаёт в сетку наравне с
//! событием, без часов — в ряд «весь день». Текст полос режется по ширине
//! с многоточием и клипом.
//!
//! Окно суток задаётся началом и концом (`CalendarStyle::window`): при
//! конце не позже начала оно идёт через полночь (06:00 → 06:00 — сутки
//! со сдвигом), и колонка дня получает ночной хвост следующей даты.
//! Внутри сетка считает не абсолютные минуты, а смещение от начала окна
//! ([`TimeElement::place`] и обратное [`TimeElement::absolute`]);
//! `Slot::date` — календарная дата вхождения, у ночного хвоста она на
//! день больше колонки, где стоит полоса. События и задачи без времени
//! остаются в ряду «весь день» по своей дате.
//!
//! Жесты: клик по пустому слоту — выбор дня, двойной — новое событие в
//! слоте; клик по событию — попап правки, по внешней полосе — её
//! страница; drag тела — перенос (дни и время со снапом к слоту, событие
//! сдвигается на разницу — повтор не прыгает началом в день броска), drag
//! нижней кромки — длительность; внешняя полоса переносится только по
//! дням (`shift_external`); мутации на MouseUp. Окно вида — настройка
//! стиля, расширенная под события с часами в видимых днях: событие в
//! 22:30 получает свой слот, а не полоску у нижнего края; полные сутки
//! и окно через полночь не расширяются — расширять некуда.

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

use super::layout::{self, ItemRef, Segment};
use super::model::{fmt_hm, lanes, Occurrence};
use super::paint::{self, Bar, Look};
use super::view::{color_of, day_weekday, GridData, Palette};
use super::{CalendarEnv, CalendarHandle, ElementBase};
use crate::pages::notes::gantt::calendar::civil_from_days;

const HEADER_H: f32 = 32.0;
const GUTTER_W: f32 = 48.0;
const ALLDAY_ROW_H: f32 = 18.0;
/// Дорожек в ряду «весь день» максимум; дальше — «ещё n».
const ALLDAY_MAX: usize = 6;
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
            tm: None,
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

struct EventDrag {
    target: ItemRef,
    mode: DragMode,
    start: Point,
    moved: bool,
    /// Предпросмотр: сдвиг в днях и минутах (Move) либо новый конец (Resize).
    d_days: i64,
    d_min: i64,
    time: (u32, u32),
}

/// Прямоугольник события или внешней полосы в сетке.
#[derive(Clone, Debug)]
struct Slot {
    rect: Rect,
    item: ItemRef,
    /// Календарная дата вхождения: у ночного хвоста окна со сдвигом она
    /// на день больше колонки, в которой стоит полоса. Для полос «весь
    /// день» — первый видимый день.
    date: i64,
    all_day: bool,
    look: Look,
    flat_left: bool,
    flat_right: bool,
}

/// «Ещё n» в ряду «весь день» колонки.
#[derive(Clone, Debug)]
struct More {
    rect: Rect,
    day: i64,
    n: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    Slot(usize),
    More(usize),
}

pub struct TimeElement {
    base: ElementBase,
    env: CalendarEnv,
    handle: CalendarHandle,
    data: GridData,
    tm: Option<Arc<dyn TextMeasure>>,
    drag: Option<EventDrag>,
    hover: Option<Hover>,
    last_click: Option<(Instant, i64, u32)>,
}

impl TimeElement {
    fn days(&self) -> Vec<i64> {
        (self.data.range.0..=self.data.range.1).collect()
    }

    fn ndays(&self) -> usize {
        (self.data.range.1 - self.data.range.0 + 1).max(1) as usize
    }

    fn slot_h(&self) -> f32 {
        if self.data.doc.style.compact { 18.0 } else { 24.0 }
    }

    /// Окно суток сетки: начало и длина в минутах — настройка стиля,
    /// расширенная до целых часов под события и задачи с часами в
    /// видимых днях (сама настройка не меняется). Полные сутки и окно
    /// через полночь не расширяются: расширять некуда.
    fn win(&self) -> (u32, u32) {
        let s = &self.data.doc.style;
        let (from, len) = s.window();
        if from + len >= 24 * 60 {
            return (from, len);
        }
        let (d0, d1) = self.data.range;
        let (mut f, mut t) = (from, from + len);
        let times = self
            .data
            .occurrences
            .iter()
            .filter(|o| o.day >= d0 && o.day <= d1)
            .filter_map(|o| o.time)
            .chain(self.data.external.iter().filter(|e| e.day >= d0 && e.day <= d1).filter_map(|e| e.time));
        for (st, en) in times {
            f = f.min(st / 60 * 60);
            t = t.max((en.div_ceil(60) * 60).min(24 * 60));
        }
        (f, (t - f).max(s.slot_min))
    }

    /// Окно захватывает ночь следующей даты.
    fn wraps(&self) -> bool {
        let (from, len) = self.win();
        from + len > 24 * 60
    }

    /// Колонка и смещение от начала окна для времени `min` даты `day`:
    /// время раньше начала окна уходит в ночной хвост предыдущей
    /// колонки, а без перехода через полночь прижимается к началу окна
    /// (как раньше — событие вне часов вида жмётся к кромке).
    fn place(&self, day: i64, min: u32) -> (i64, i64) {
        let (from, _) = self.win();
        let off = min as i64 - from as i64;
        if off >= 0 {
            (day, off)
        } else if self.wraps() {
            (day - 1, off + 24 * 60)
        } else {
            (day, 0)
        }
    }

    /// Обратное к [`Self::place`]: дата и минуты суток по колонке и
    /// смещению — ночной хвост принадлежит следующей дате.
    fn absolute(&self, col: i64, off: i64) -> (i64, u32) {
        let (from, _) = self.win();
        let total = from as i64 + off;
        (col + total.div_euclid(24 * 60), total.rem_euclid(24 * 60) as u32)
    }

    fn slots_count(&self) -> u32 {
        let (_, len) = self.win();
        (len / self.data.doc.style.slot_min).max(1)
    }

    /// Отрезки «весь день» (события без времени, внешние без часов).
    /// Вхождения за краем диапазона колонок (их тянет ночной хвост) в
    /// ряд не идут: он живёт по календарным датам колонок.
    fn allday_segments(&self) -> Vec<Segment> {
        let (d0, d1) = self.data.range;
        let occ: Vec<Occurrence> = self.data.occurrences.iter().filter(|o| o.day >= d0 && o.day <= d1).cloned().collect();
        let mut segs = layout::segments(&occ, &self.data.external);
        segs.retain(|s| s.time.is_none());
        segs
    }

    fn allday_rows(&self) -> usize {
        let segs = self.allday_segments();
        layout::lanes_needed(&layout::pack_row(&segs, self.data.range.0, self.ndays())).min(ALLDAY_MAX)
    }

    fn allday_h(&self) -> f32 {
        (self.allday_rows() as f32 * ALLDAY_ROW_H + 6.0).max(22.0)
    }

    fn grid_top(&self) -> f32 {
        self.base.bounds.origin.y + HEADER_H + self.allday_h()
    }

    fn col_w(&self) -> f32 {
        ((self.base.bounds.size.width - GUTTER_W) / self.ndays() as f32).max(20.0)
    }

    fn col_x(&self, day: i64) -> f32 {
        self.base.bounds.origin.x + GUTTER_W + (day - self.data.range.0) as f32 * self.col_w()
    }

    /// Y смещения от начала окна.
    fn y_of(&self, off: i64) -> f32 {
        self.grid_top() + off as f32 / self.data.doc.style.slot_min as f32 * self.slot_h()
    }

    /// Смещение от начала окна по координате.
    fn off_at(&self, y: f32) -> i64 {
        let slots = ((y - self.grid_top()) / self.slot_h()).floor() as i64;
        slots * self.data.doc.style.slot_min as i64
    }

    fn day_at(&self, x: f32) -> Option<i64> {
        let i = ((x - self.base.bounds.origin.x - GUTTER_W) / self.col_w()).floor() as i64;
        (i >= 0 && i < self.ndays() as i64).then(|| self.data.range.0 + i)
    }

    /// Прямоугольники: ряд «весь день» полосами по дорожкам и часы с
    /// дорожками пересечений; плюс «ещё n» ряда «весь день».
    fn slots(&self) -> (Vec<Slot>, Vec<More>) {
        let mut out = Vec::new();
        let mut more = Vec::new();
        let col_w = self.col_w();
        let event_style = self.data.doc.style.event_style;
        let (_, len) = self.win();
        let max_off = len as i64;
        let allday_top = self.base.bounds.origin.y + HEADER_H + 3.0;
        // Весь день.
        let segs = self.allday_segments();
        let placed = layout::pack_row(&segs, self.data.range.0, self.ndays());
        let (runs, hidden) = layout::visible(&placed, self.ndays(), ALLDAY_MAX);
        for run in runs {
            let p = &placed[run.placed];
            let s = &segs[p.seg];
            let flat_left = (run.col0 == p.col0 && p.cont_left) || run.col0 > p.col0;
            let flat_right = (run.col1 == p.col1 && p.cont_right) || run.col1 < p.col1;
            let x0 = self.col_x(self.data.range.0 + run.col0 as i64) + if flat_left { 0.0 } else { 2.0 };
            let x1 = self.col_x(self.data.range.0 + run.col1 as i64) + col_w - if flat_right { 0.0 } else { 2.0 };
            out.push(Slot {
                rect: Rect::new(Point::new(x0, allday_top + p.lane as f32 * ALLDAY_ROW_H), Size::new((x1 - x0).max(4.0), ALLDAY_ROW_H - 2.0)),
                item: s.item.clone(),
                date: self.data.range.0 + run.col0 as i64,
                all_day: true,
                look: Look::of(event_style, s.is_event(), true, s.multi_day()),
                flat_left,
                flat_right,
            });
        }
        for (col, &n) in hidden.iter().enumerate() {
            if n > 0 {
                let x = self.col_x(self.data.range.0 + col as i64);
                more.push(More {
                    rect: Rect::new(Point::new(x + 2.0, allday_top + (ALLDAY_MAX - 1) as f32 * ALLDAY_ROW_H), Size::new(col_w - 4.0, ALLDAY_ROW_H - 2.0)),
                    day: self.data.range.0 + col as i64,
                    n,
                });
            }
        }
        // По часам — события и внешние полосы со временем, вместе в
        // одной упаковке дорожек: задача доски не наезжает на встречу.
        for day in self.days() {
            let x = self.col_x(day);
            // Колонка собирает вхождения своей даты и — у окна со
            // сдвигом — ночной хвост следующей: событие в 02:00 при
            // старте 06:00 стоит внизу предыдущей колонки.
            let timed: Vec<(ItemRef, i64, (i64, i64))> = self
                .data
                .occurrences
                .iter()
                .filter_map(|o| o.time.map(|t| (ItemRef::Event(o.event.clone()), o.day, t)))
                .chain(self.data.external.iter().enumerate().filter_map(|(i, e)| e.time.map(|t| (ItemRef::External(i), e.day, t))))
                .filter_map(|(item, date, (st, en))| {
                    let (col, off) = self.place(date, st);
                    if col != day || off >= max_off {
                        return None;
                    }
                    let dur = (en as i64 - st as i64).max(5);
                    let s = off.clamp(0, max_off - 5);
                    let e = (off + dur).clamp(s + 5, max_off);
                    Some((item, date, (s, e)))
                })
                .collect();
            let intervals: Vec<(u32, u32)> = timed.iter().map(|(_, _, (s, e))| (*s as u32, *e as u32)).collect();
            let packed = lanes(&intervals);
            for (k, (item, date, (s, e))) in timed.iter().enumerate() {
                let (lane, of) = packed[k];
                let lane_w = (col_w - 4.0) / of as f32;
                let rect = Rect::new(
                    Point::new(x + 2.0 + lane as f32 * lane_w, self.y_of(*s)),
                    Size::new((lane_w - 2.0).max(6.0), (self.y_of(*e) - self.y_of(*s)).max(self.slot_h() * 0.5)),
                );
                let is_event = matches!(item, ItemRef::Event(_));
                out.push(Slot {
                    rect,
                    item: item.clone(),
                    date: *date,
                    all_day: false,
                    look: if is_event { Look::Filled } else { Look::Tinted },
                    flat_left: false,
                    flat_right: false,
                });
            }
        }
        (out, more)
    }

    fn hit(&self, slots: &[Slot], more: &[More], p: Point) -> Option<Hover> {
        if let Some(i) = more.iter().position(|m| m.rect.contains(p)) {
            return Some(Hover::More(i));
        }
        slots.iter().rposition(|s| s.rect.contains(p)).map(Hover::Slot)
    }

    /// Прямоугольник с учётом предпросмотра переноса/растяжения.
    fn preview_rect(&self, slot: &Slot) -> Rect {
        let Some(d) = self.drag.as_ref().filter(|d| d.moved && slot.item == d.target) else {
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
                let slot_min = self.data.doc.style.slot_min as i64;
                let new_end = (d.time.1 as i64 + d.d_min).max(d.time.0 as i64 + slot_min);
                let dur = (new_end - d.time.0 as i64) as f32 / slot_min as f32 * self.slot_h();
                r.size.height = dur.max(self.slot_h() * 0.5);
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
            ItemRef::External(i) => {
                if let Some(item) = self.data.external.get(*i) {
                    if d.d_days != 0 {
                        (self.env.shift_external)(&item.source, d.d_days);
                    }
                }
                return;
            }
            ItemRef::Event(id) => id.clone(),
        };
        match d.mode {
            DragMode::Move => {
                // На разницу дней от собственной даты события: повтор и
                // многодневное не прыгают началом в колонку броска.
                // Уход за полночь (окно со сдвигом это позволяет) —
                // ещё день сдвига, а не упор в 23:55.
                let Some(ev) = self.data.store.event(&id) else { return };
                let Some((start_day, _)) = ev.span() else { return };
                let (over, start) = if ev.all_day {
                    (0, None)
                } else {
                    let abs = d.time.0 as i64 + d.d_min;
                    (abs.div_euclid(24 * 60), Some(abs.rem_euclid(24 * 60) as u32))
                };
                if d.d_days != 0 || d.d_min != 0 {
                    self.env.store.move_event(&id, start_day + d.d_days + over, start);
                }
            }
            DragMode::Resize => {
                let slot_min = self.data.doc.style.slot_min as i64;
                let new_end = ((d.time.1 as i64 + d.d_min).max(d.time.0 as i64 + slot_min)).min(24 * 60) as u32;
                self.env.store.update_event(&id, |e| e.end = Some(new_end));
            }
        }
    }

    fn open_edit(&self, id: &str, day: Option<i64>, anchor: Rect) {
        if let Some(e) = self.data.store.event(id) {
            let day = day.or_else(|| e.day()).unwrap_or_default();
            self.handle.open_edit(e.clone(), day, anchor);
        }
    }
}

impl Element for TimeElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<TimeGrid>() else { return };
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
        let slots_n = self.slots_count();
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
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots_n as f32 * slot_h)), pal.weekend, [0.0; 4]);
            }
            if is_today {
                list.push_rect(Rect::new(Point::new(x, b.origin.y), Size::new(col_w, HEADER_H)), pal.today.with_alpha(0.18), [0.0; 4]);
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots_n as f32 * slot_h)), pal.today.with_alpha(0.05), [0.0; 4]);
            }
            if self.data.selected_day == Some(day) && days.len() > 1 {
                list.push_rect(Rect::new(Point::new(x, grid_top), Size::new(col_w, slots_n as f32 * slot_h)), pal.accent.with_alpha(0.06), [0.0; 4]);
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
        let (win_from, win_len) = self.win();
        for i in 0..=slots_n {
            let y = grid_top + i as f32 * slot_h;
            // Час — по абсолютному времени, а не по счёту слотов: окно
            // может начинаться в 06:30, и линии часов остаются на часах.
            let minute = (win_from + i * style.slot_min) % (24 * 60);
            let hour_line = minute % 60 == 0;
            c.set_color(if hour_line { pal.grid } else { pal.grid.with_alpha(pal.grid.a * 0.45) });
            c.draw_line(b.origin.x + GUTTER_W, y, b.origin.x + b.size.width, y);
            if hour_line && i < slots_n {
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
            c.draw_line(x, b.origin.y + HEADER_H, x, grid_top + slots_n as f32 * slot_h);
        }
        c.flush(list);

        // Полосы: переносимая — последней, поверх остальных.
        let (slots, more) = self.slots();
        let drag_target = self.drag.as_ref().filter(|d| d.moved).map(|d| d.target.clone());
        let mut order: Vec<usize> = (0..slots.len()).collect();
        order.sort_by_key(|&i| drag_target.as_ref() == Some(&slots[i].item));
        for i in order {
            let slot = &slots[i];
            let rect = self.preview_rect(slot);
            let dragged = drag_target.as_ref() == Some(&slot.item);
            let hover = self.hover == Some(Hover::Slot(i)) && self.drag.is_none();
            let (color, title, done, selected, time) = match &slot.item {
                ItemRef::Event(id) => {
                    let e = self.data.store.event(id);
                    let color = e.map(|e| self.data.store.color_of(e)).and_then(|c| color_of(&c)).unwrap_or(pal.accent);
                    let occ = self.data.occurrences.iter().find(|o| &o.event == id && o.day == slot.date);
                    (color, e.map(|e| e.title.clone()).unwrap_or_default(), e.is_some_and(|e| e.done_at(slot.date)), self.data.selected.as_deref() == Some(id.as_str()), occ.and_then(|o| o.time))
                }
                ItemRef::External(i) => {
                    let ext = &self.data.external[*i];
                    (color_of(&ext.color).unwrap_or(pal.muted), ext.title.clone(), false, false, ext.time)
                }
            };
            let alpha = if dragged { 0.55 } else { 1.0 };
            if slot.all_day {
                let done_box = matches!((slot.look, &slot.item), (Look::Filled, ItemRef::Event(_)))
                    .then(|| Rect::new(Point::new(rect.origin.x + 5.0, rect.origin.y + ((rect.size.height - 10.0) / 2.0).round()), Size::new(10.0, 10.0)));
                paint::draw_bar(
                    list,
                    self.tm.as_ref(),
                    &Bar {
                        rect,
                        color,
                        look: slot.look,
                        title: &title,
                        time: None,
                        done,
                        selected,
                        hover,
                        alpha,
                        flat_left: slot.flat_left,
                        flat_right: slot.flat_right,
                        done_box,
                        font,
                        text: pal.text,
                        accent: pal.accent,
                    },
                );
                continue;
            }
            // Полоса по часам: название и время второй строкой, если влезает.
            let a = alpha * if done { 0.5 } else { 1.0 };
            let text_color = match slot.look {
                Look::Tinted => {
                    list.push_rect(rect, color.with_alpha(if hover { 0.32 } else { 0.22 } * a), [4.0; 4]);
                    list.push_rect(Rect::new(rect.origin, Size::new(3.0, rect.size.height)), color.with_alpha(0.95 * a), [1.5, 0.0, 0.0, 1.5]);
                    pal.text.with_alpha(0.92 * a)
                }
                _ => {
                    list.push_rect(rect, color.with_alpha(if hover { 1.0 } else { 0.88 } * a), [4.0; 4]);
                    paint::text_on(color).with_alpha(a)
                }
            };
            let tx = rect.origin.x + if slot.look == Look::Tinted { 8.0 } else { 5.0 };
            let tw = rect.origin.x + rect.size.width - 4.0 - tx;
            list.push_clip(rect);
            let line = paint::ellipsize(self.tm.as_ref(), &title, font - 1.0, true, tw);
            list.push_text_styled_singleline(
                &line,
                Rect::new(Point::new(tx, rect.origin.y + 2.0), Size::new(tw.max(4.0), font + 3.0)),
                text_color,
                font - 1.0,
                TextAlign::DEFAULT,
                if done { TextDecoration::LineThrough } else { TextDecoration::None },
                600,
                None,
            );
            if let Some((s, e)) = time.filter(|_| rect.size.height > font * 2.0 + 8.0) {
                let sub = paint::ellipsize(self.tm.as_ref(), &format!("{}–{}", fmt_hm(s), fmt_hm(e)), font - 2.0, false, tw);
                list.push_text_styled_singleline(
                    &sub,
                    Rect::new(Point::new(tx, rect.origin.y + font + 5.0), Size::new(tw.max(4.0), font + 2.0)),
                    text_color.with_alpha(text_color.a * 0.8),
                    font - 2.0,
                    TextAlign::DEFAULT,
                    TextDecoration::None,
                    400,
                    None,
                );
            }
            list.pop_clip();
            if selected {
                let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                k.set_color(pal.accent);
                k.set_stroke_width(1.5);
                k.draw_rect(rect.origin.x - 1.0, rect.origin.y - 1.0, rect.size.width + 2.0, rect.size.height + 2.0);
                k.flush(list);
            }
        }
        for (i, m) in more.iter().enumerate() {
            let hovered = self.hover == Some(Hover::More(i));
            if hovered {
                list.push_rect(m.rect, pal.text.with_alpha(0.07), [4.0; 4]);
            }
            paint::draw_more(list, m.rect, m.n, font, if hovered { pal.accent } else { pal.muted });
        }
        // Линия «сейчас»: у окна со сдвигом ночь попадает в колонку
        // предыдущего дня — 03:00 при старте 06:00 рисуется в хвосте
        // вчерашней колонки.
        let raw = self.data.now_min as i64 - win_from as i64;
        let (now_col, now_off) = if raw >= 0 { (self.data.today, raw) } else { (self.data.today - 1, raw + 24 * 60) };
        if days.contains(&now_col) && (0..=win_len as i64).contains(&now_off) {
            let x = self.col_x(now_col);
            let y = self.y_of(now_off);
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
                let (slots, more) = self.slots();
                match self.hit(&slots, &more, *position) {
                    Some(Hover::More(i)) => {
                        let m = &more[i];
                        let anchor = Rect::new(Point::new(self.col_x(m.day), self.base.bounds.origin.y), Size::new(self.col_w(), HEADER_H));
                        self.handle.open_day(m.day, anchor);
                        return EventResult::Handled;
                    }
                    Some(Hover::Slot(i)) => {
                        let slot = &slots[i];
                        let (time, mode) = match &slot.item {
                            ItemRef::Event(id) => {
                                let time = self.data.occurrences.iter().find(|o| &o.event == id && o.day == slot.date).and_then(|o| o.time).unwrap_or((0, 0));
                                let mode = if !slot.all_day && position.y > slot.rect.origin.y + slot.rect.size.height - EDGE_PX {
                                    DragMode::Resize
                                } else {
                                    DragMode::Move
                                };
                                self.handle.select_at(id.clone(), slot.date);
                                (time, mode)
                            }
                            // Внешнюю полосу за кромку не растягиваем: её длина
                            // — оценка задачи, она правится в её карточке.
                            ItemRef::External(_) => ((0, 0), DragMode::Move),
                        };
                        self.drag = Some(EventDrag { target: slot.item.clone(), mode, start: *position, moved: false, d_days: 0, d_min: 0, time });
                        ctx.capture();
                        return EventResult::Handled;
                    }
                    None => {}
                }
                let Some(day) = self.day_at(position.x) else { return EventResult::Ignored };
                let in_hours = position.y >= self.grid_top();
                let (_, len) = self.win();
                let slot_min = self.data.doc.style.slot_min as i64;
                let off = if in_hours { self.off_at(position.y).clamp(0, (len as i64 - slot_min).max(0)) } else { 0 };
                // Ночной хвост принадлежит следующей дате: событие,
                // созданное в 01:00 при старте 06:00, ложится на завтра.
                let (target_day, start) = if in_hours { self.absolute(day, off) } else { (day, 0) };
                let now = Instant::now();
                let double = self.last_click.as_ref().is_some_and(|(t, d, s)| *d == target_day && *s == start && now.duration_since(*t) < DOUBLE_CLICK);
                self.last_click = Some((now, target_day, start));
                self.handle.select(None);
                if double {
                    let cal = self.data.doc.calendars.first().cloned().or_else(|| self.data.store.calendars.first().map(|c| c.id.clone())).unwrap_or_default();
                    let anchor = Rect::new(
                        Point::new(self.col_x(day), if in_hours { self.y_of(off) } else { self.base.bounds.origin.y + HEADER_H }),
                        Size::new(self.col_w(), self.slot_h()),
                    );
                    self.handle.open_new(&cal, target_day, in_hours.then_some(start), self.data.doc.style.slot_min, anchor);
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
                let (slots, more) = self.slots();
                let hover = if inside { self.hit(&slots, &more, *position) } else { None };
                if hover != self.hover {
                    self.hover = hover;
                    self.base.dirty |= DirtyFlags::RENDER;
                }
                match hover {
                    Some(Hover::Slot(i)) => {
                        let slot = &slots[i];
                        let resize = !slot.all_day && matches!(slot.item, ItemRef::Event(_)) && position.y > slot.rect.origin.y + slot.rect.size.height - EDGE_PX;
                        ctx.set_cursor(if resize { CursorIcon::RowResize } else { CursorIcon::Pointer });
                    }
                    Some(Hover::More(_)) => ctx.set_cursor(CursorIcon::Pointer),
                    None => {}
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
                        ItemRef::Event(id) => {
                            let (slots, _) = self.slots();
                            let slot = slots.iter().rev().find(|s| s.rect.contains(*position));
                            let anchor = slot.map(|s| s.rect).unwrap_or(self.base.bounds);
                            let day = slot.map(|s| s.date).or_else(|| self.handle.selected_at.get_untracked());
                            self.open_edit(&id, day, anchor);
                        }
                        ItemRef::External(i) => {
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
