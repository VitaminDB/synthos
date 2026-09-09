//! Календарь: единое хранилище событий проекта (`notes/calendar.json`) и
//! документ виджета (`notes/objects/<id>.calendar.json`).
//!
//! События живут в одном файле на проект — именованные календари с цветом
//! плюс события (дата, необязательные конечная дата и время, весь день,
//! «сделано», заметка, цвет, повтор, ссылка на страницу). Виджет на
//! странице — только вид (год / месяц / неделя / день), якорная дата,
//! фильтр календарей и стиль: добавил дело в недельном виджете — оно видно
//! в месячном на другой странице.

use serde::{Deserialize, Serialize};

use super::super::gantt::calendar::{civil_from_days, days_from_civil, days_to_iso, parse_days, weekday_of};
use super::super::kanban::model::{item_id, PALETTE};

// ─────────────────────────────────────────────────────────────────────────────
// Хранилище событий
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarStore {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub calendars: Vec<CalendarDef>,
    #[serde(default)]
    pub events: Vec<CalEvent>,
}

fn default_version() -> u32 {
    1
}

/// Именованный календарь (категория событий) со своим цветом.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

/// Повтор события.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Repeat {
    pub const ALL: [Repeat; 5] = [Repeat::None, Repeat::Daily, Repeat::Weekly, Repeat::Monthly, Repeat::Yearly];

    pub fn key(self) -> &'static str {
        match self {
            Repeat::None => "none",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
            Repeat::Monthly => "monthly",
            Repeat::Yearly => "yearly",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.key() == s.trim().to_ascii_lowercase())
    }

    pub fn is_none(&self) -> bool {
        *self == Repeat::None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalEvent {
    pub id: String,
    /// Календарь (id из `calendars`).
    #[serde(default)]
    pub calendar: String,
    #[serde(default)]
    pub title: String,
    /// ISO `YYYY-MM-DD`.
    pub date: String,
    /// Последний день многодневного события (включительно).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Начало/конец в минутах с полуночи; без них — весь день.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_day: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Свой цвет поверх цвета календаря.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Repeat::is_none")]
    pub repeat: Repeat,
    /// Последняя дата повтора (включительно).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Страница проекта (id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl CalEvent {
    pub fn new(calendar: &str, title: &str, day: i64) -> Self {
        Self {
            id: item_id("e"),
            calendar: calendar.to_string(),
            title: title.to_string(),
            date: days_to_iso(day),
            end_date: None,
            start: None,
            end: None,
            all_day: true,
            done: false,
            note: String::new(),
            color: String::new(),
            repeat: Repeat::None,
            until: None,
            link: None,
        }
    }

    pub fn day(&self) -> Option<i64> {
        parse_days(&self.date)
    }

    /// Дни события (первый, последний) — многодневное или один день.
    pub fn span(&self) -> Option<(i64, i64)> {
        let s = self.day()?;
        let e = self.end_date.as_deref().and_then(parse_days).unwrap_or(s);
        Some(if e < s { (e, s) } else { (s, e) })
    }

    /// Минуты начала/конца с учётом «весь день».
    pub fn time_span(&self) -> Option<(u32, u32)> {
        if self.all_day {
            return None;
        }
        let s = self.start?;
        let e = self.end.unwrap_or(s + 30).max(s + 5).min(24 * 60);
        Some((s.min(24 * 60 - 5), e))
    }
}

/// Вхождение события в конкретный день (развёртка повторов и многодневных).
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub event: String,
    pub day: i64,
    /// Минуты начала/конца, `None` — весь день.
    pub time: Option<(u32, u32)>,
    /// Первый/последний день многодневного события.
    pub first: bool,
    pub last: bool,
}

impl CalendarStore {
    pub fn template(default_name: &str) -> Self {
        Self {
            version: 1,
            calendars: vec![CalendarDef { id: item_id("c"), name: default_name.to_string(), color: PALETTE[3].to_string() }],
            events: Vec::new(),
        }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut s: Self = serde_json::from_str(json)?;
        s.sanitize("Calendar");
        Ok(s)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Хотя бы один календарь; события без календаря — в первый; даты
    /// нечитаемые — сегодня.
    pub fn sanitize(&mut self, default_name: &str) {
        if self.calendars.is_empty() {
            self.calendars.push(CalendarDef { id: item_id("c"), name: default_name.to_string(), color: PALETTE[3].to_string() });
        }
        let first = self.calendars[0].id.clone();
        for e in &mut self.events {
            if !self.calendars.iter().any(|c| c.id == e.calendar) {
                e.calendar = first.clone();
            }
            if parse_days(&e.date).is_none() {
                e.date = days_to_iso(super::super::gantt::calendar::today_days());
            }
            if e.start.is_none() {
                e.all_day = true;
            }
            if let Some(s) = e.start {
                e.start = Some(s.min(24 * 60 - 5));
            }
            if let Some(en) = e.end {
                e.end = Some(en.min(24 * 60));
            }
        }
    }

    pub fn calendar(&self, id: &str) -> Option<&CalendarDef> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn event(&self, id: &str) -> Option<&CalEvent> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn event_mut(&mut self, id: &str) -> Option<&mut CalEvent> {
        self.events.iter_mut().find(|e| e.id == id)
    }

    pub fn add_calendar(&mut self, name: &str, color: &str) -> String {
        let id = item_id("c");
        let color = if color.is_empty() { PALETTE[self.calendars.len() % PALETTE.len()].to_string() } else { color.to_string() };
        self.calendars.push(CalendarDef { id: id.clone(), name: name.to_string(), color });
        id
    }

    /// Удалить календарь; его события переезжают в первый оставшийся
    /// (последний календарь удалить нельзя).
    pub fn remove_calendar(&mut self, id: &str) -> bool {
        if self.calendars.len() <= 1 || !self.calendars.iter().any(|c| c.id == id) {
            return false;
        }
        self.calendars.retain(|c| c.id != id);
        let heir = self.calendars[0].id.clone();
        for e in &mut self.events {
            if e.calendar == id {
                e.calendar = heir.clone();
            }
        }
        true
    }

    pub fn add_event(&mut self, mut event: CalEvent) -> String {
        if !self.calendars.iter().any(|c| c.id == event.calendar) {
            event.calendar = self.calendars.first().map(|c| c.id.clone()).unwrap_or_default();
        }
        if event.start.is_none() {
            event.all_day = true;
        }
        let id = event.id.clone();
        self.events.push(event);
        id
    }

    pub fn remove_event(&mut self, id: &str) -> bool {
        let before = self.events.len();
        self.events.retain(|e| e.id != id);
        before != self.events.len()
    }

    /// Цвет события: свой либо цвет календаря.
    pub fn color_of(&self, e: &CalEvent) -> String {
        if !e.color.is_empty() {
            return e.color.clone();
        }
        self.calendar(&e.calendar).map(|c| c.color.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| PALETTE[3].to_string())
    }

    /// Вхождения событий в диапазон дней `[from, to]` (включительно),
    /// по календарям `filter` (пусто — все). Отсортированы по дню, затем
    /// «весь день» первыми, затем по времени начала.
    pub fn occurrences(&self, from: i64, to: i64, filter: &[String]) -> Vec<Occurrence> {
        let mut out = Vec::new();
        for e in &self.events {
            if !filter.is_empty() && !filter.iter().any(|c| *c == e.calendar) {
                continue;
            }
            let Some((s, en)) = e.span() else { continue };
            let len = en - s;
            let until = e.until.as_deref().and_then(parse_days).unwrap_or(i64::MAX);
            let time = e.time_span();
            let mut push = |start_day: i64| {
                for d in start_day..=start_day + len {
                    if d < from || d > to {
                        continue;
                    }
                    out.push(Occurrence { event: e.id.clone(), day: d, time, first: d == start_day, last: d == start_day + len });
                }
            };
            match e.repeat {
                Repeat::None => push(s),
                Repeat::Daily => {
                    let mut d = s.max(from - len);
                    while d <= to && d <= until {
                        if d >= s {
                            push(d);
                        }
                        d += 1;
                    }
                }
                Repeat::Weekly => {
                    let mut d = s;
                    if from - len > s {
                        d = s + ((from - len - s) / 7) * 7;
                    }
                    while d <= to && d <= until {
                        if d >= s {
                            push(d);
                        }
                        d += 7;
                    }
                }
                Repeat::Monthly => {
                    let (y0, m0, day0) = civil_from_days(s);
                    let mut k = 0i64;
                    loop {
                        let (y, m) = add_months(y0, m0, k);
                        if let Some(d) = valid_day(y, m, day0) {
                            if d > to || d > until {
                                break;
                            }
                            if d >= s {
                                push(d);
                            }
                        } else if days_from_civil(y, m, 1) > to {
                            break;
                        }
                        k += 1;
                        if k > 12 * 200 {
                            break;
                        }
                    }
                }
                Repeat::Yearly => {
                    let (y0, m0, day0) = civil_from_days(s);
                    let mut y = y0;
                    loop {
                        if let Some(d) = valid_day(y, m0, day0) {
                            if d > to || d > until {
                                break;
                            }
                            if d >= s {
                                push(d);
                            }
                        } else if days_from_civil(y, m0, 1) > to {
                            break;
                        }
                        y += 1;
                        if y > y0 + 200 {
                            break;
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.day.cmp(&b.day).then_with(|| match (a.time, b.time) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(x), Some(y)) => x.0.cmp(&y.0),
            })
        });
        out
    }
}

/// `y-m` плюс `k` месяцев.
pub fn add_months(y: i64, m: u32, k: i64) -> (i64, u32) {
    let total = y * 12 + (m as i64 - 1) + k;
    (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32)
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    let (ny, nm) = add_months(y, m, 1);
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

/// День `d` месяца, если он в нём есть.
fn valid_day(y: i64, m: u32, d: u32) -> Option<i64> {
    (d <= days_in_month(y, m)).then(|| days_from_civil(y, m, d))
}

/// `HH:MM` → минуты с полуночи.
pub fn parse_hm(s: &str) -> Option<u32> {
    let t = s.trim();
    let (h, m) = t.split_once(':').or_else(|| t.split_once('.'))?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn fmt_hm(min: u32) -> String {
    format!("{:02}:{:02}", (min / 60).min(23), min % 60)
}

// ─────────────────────────────────────────────────────────────────────────────
// Документ виджета
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalView {
    Year,
    #[default]
    Month,
    Week,
    Day,
}

impl CalView {
    pub const ALL: [CalView; 4] = [CalView::Year, CalView::Month, CalView::Week, CalView::Day];

    pub fn key(self) -> &'static str {
        match self {
            CalView::Year => "year",
            CalView::Month => "month",
            CalView::Week => "week",
            CalView::Day => "day",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.key() == s.trim().to_ascii_lowercase())
    }
}

/// Как рисовать события в месяце.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventStyle {
    #[default]
    Chip,
    Dot,
    Bar,
}

impl EventStyle {
    pub const ALL: [EventStyle; 3] = [EventStyle::Chip, EventStyle::Dot, EventStyle::Bar];

    pub fn key(self) -> &'static str {
        match self {
            EventStyle::Chip => "chip",
            EventStyle::Dot => "dot",
            EventStyle::Bar => "bar",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.key() == s.trim().to_ascii_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub view: CalView,
    /// Якорная дата вида (ISO).
    #[serde(default)]
    pub anchor: String,
    /// Видимые календари (id); пусто — все.
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Доски-источники задач (id объектов `kanban`); пусто — все доски
    /// проекта. Их карточки видны полосами наравне с событиями.
    #[serde(default)]
    pub boards: Vec<String>,
    #[serde(default)]
    pub style: CalendarStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarStyle {
    /// 0 — понедельник … 6 — воскресенье.
    #[serde(default)]
    pub first_weekday: u32,
    #[serde(default)]
    pub show_week_numbers: bool,
    /// Подложка выходных (`#rrggbbaa`); пусто — тема.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub weekend_tint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub today_color: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header_bg: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cell_bg: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub grid_color: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text_color: String,
    #[serde(default)]
    pub event_style: EventStyle,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Окно суток дневного/недельного вида в минутах от полуночи.
    /// Конец не позже начала — окно идёт через полночь: 06:00 → 06:00
    /// это сутки со сдвигом, 22:00 → 06:00 — ночная смена.
    #[serde(default = "default_from_min")]
    pub from_min: u32,
    #[serde(default = "default_to_min")]
    pub to_min: u32,
    /// Полные сутки: окно ровно 24 часа от `from_min`, конец зеркалит
    /// начало.
    #[serde(default)]
    pub full_day: bool,
    /// Целые часы окна — формат документов до 09.09.2026. Читаются и
    /// разворачиваются в минуты (`sanitize`), обратно не пишутся.
    #[serde(default, rename = "hour_from", skip_serializing)]
    legacy_hour_from: Option<u32>,
    #[serde(default, rename = "hour_to", skip_serializing)]
    legacy_hour_to: Option<u32>,
    /// Слот в минутах.
    #[serde(default = "default_slot_min")]
    pub slot_min: u32,
    #[serde(default)]
    pub compact: bool,
    /// Слой сроков карточек канбана / задач Ганта проекта;
    /// у новых виджетов включён — иначе календарь молчит о сроках задач.
    #[serde(default = "default_true_style")]
    pub show_kanban_due: bool,
    /// Плановые полосы карточек (`start`/`end`): день и неделя рисуют их
    /// отрезками, а не точками.
    #[serde(default = "default_true_style")]
    pub show_kanban_spans: bool,
    #[serde(default = "default_true_style")]
    pub show_gantt: bool,
    /// Имя пресета стиля (для панели свойств).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preset: String,
}

fn default_font_size() -> f32 {
    12.0
}
fn default_true_style() -> bool {
    true
}
fn default_from_min() -> u32 {
    8 * 60
}
fn default_to_min() -> u32 {
    20 * 60
}
fn default_slot_min() -> u32 {
    30
}

impl Default for CalendarStyle {
    fn default() -> Self {
        Self {
            first_weekday: 0,
            show_week_numbers: false,
            weekend_tint: String::new(),
            today_color: String::new(),
            header_bg: String::new(),
            cell_bg: String::new(),
            grid_color: String::new(),
            text_color: String::new(),
            event_style: EventStyle::Chip,
            font_size: default_font_size(),
            from_min: default_from_min(),
            to_min: default_to_min(),
            full_day: false,
            legacy_hour_from: None,
            legacy_hour_to: None,
            slot_min: default_slot_min(),
            compact: false,
            show_kanban_due: true,
            show_kanban_spans: true,
            show_gantt: true,
            preset: String::new(),
        }
    }
}

impl CalendarStyle {
    pub fn sanitize(&mut self) {
        self.first_weekday = self.first_weekday.min(6);
        if !self.font_size.is_finite() || !(8.0..=24.0).contains(&self.font_size) {
            self.font_size = default_font_size();
        }
        if let Some(h) = self.legacy_hour_from.take() {
            self.from_min = h.min(23) * 60;
        }
        if let Some(h) = self.legacy_hour_to.take() {
            self.to_min = h.clamp(1, 24) * 60;
        }
        self.from_min = self.from_min.min(23 * 60 + 55) / 5 * 5;
        self.to_min = self.to_min.clamp(5, 24 * 60) / 5 * 5;
        if ![5, 10, 15, 20, 30, 60].contains(&self.slot_min) {
            self.slot_min = default_slot_min();
        }
    }

    /// Окно суток: начало в минутах от полуночи и длина в минутах.
    /// Конец, не превышающий начала, означает переход через полночь —
    /// колонка дня получает ночной хвост следующей даты.
    pub fn window(&self) -> (u32, u32) {
        let from = self.from_min.min(24 * 60 - 1);
        if self.full_day {
            return (from, 24 * 60);
        }
        let len = (self.to_min + 24 * 60 - from) % (24 * 60);
        (from, if len == 0 { 24 * 60 } else { len })
    }

    /// Окно уходит за полночь: в колонке дня видна часть следующего.
    pub fn wraps(&self) -> bool {
        let (from, len) = self.window();
        from + len > 24 * 60
    }

    /// Пресеты оформления: тема (пусто), светлый, контраст, пастель.
    pub const PRESETS: [&'static str; 4] = ["theme", "light", "contrast", "pastel"];

    pub fn apply_preset(&mut self, name: &str) {
        let (weekend, today, header, cell, grid, text) = match name {
            "light" => ("#00000010", "#4F8CFF", "#FFFFFF", "#FAFAFA", "#E5E7EB", "#1F2937"),
            "contrast" => ("#FFFFFF14", "#FFB020", "#0B0F19", "#141A26", "#3A4256", "#F5F7FA"),
            "pastel" => ("#F7A1A122", "#8B5CF6", "#2B2440", "#241F36", "#4A3F6B", "#EDE9FE"),
            _ => ("", "", "", "", "", ""),
        };
        self.weekend_tint = weekend.into();
        self.today_color = today.into();
        self.header_bg = header.into();
        self.cell_bg = cell.into();
        self.grid_color = grid.into();
        self.text_color = text.into();
        self.preset = if name == "theme" { String::new() } else { name.to_string() };
    }
}

impl CalendarDoc {
    pub fn template(view: CalView, today: i64) -> Self {
        Self { version: 1, view, anchor: days_to_iso(today), calendars: Vec::new(), boards: Vec::new(), style: CalendarStyle::default() }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut d: Self = serde_json::from_str(json)?;
        d.style.sanitize();
        if parse_days(&d.anchor).is_none() {
            d.anchor = days_to_iso(super::super::gantt::calendar::today_days());
        }
        Ok(d)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn anchor_days(&self) -> i64 {
        parse_days(&self.anchor).unwrap_or_else(super::super::gantt::calendar::today_days)
    }
}

/// Диапазон дней вида `[from, to]` вокруг якоря: год — весь год, месяц —
/// шесть недель сетки (с первого дня недели), неделя — 7 дней, день — сам.
pub fn range_of(view: CalView, anchor: i64, first_weekday: u32) -> (i64, i64) {
    match view {
        CalView::Day => (anchor, anchor),
        CalView::Week => {
            let start = week_start(anchor, first_weekday);
            (start, start + 6)
        }
        CalView::Month => {
            let (y, m, _) = civil_from_days(anchor);
            let first = days_from_civil(y, m, 1);
            let start = week_start(first, first_weekday);
            (start, start + 6 * 7 - 1)
        }
        CalView::Year => {
            let (y, _, _) = civil_from_days(anchor);
            (days_from_civil(y, 1, 1), days_from_civil(y + 1, 1, 1) - 1)
        }
    }
}

/// Первый день недели, содержащей `day` (0 = понедельник).
pub fn week_start(day: i64, first_weekday: u32) -> i64 {
    let wd = weekday_of(day) as i64;
    let shift = (wd - first_weekday as i64).rem_euclid(7);
    day - shift
}

/// Раскладка пересекающихся интервалов по «дорожкам»: для каждого — (дорожка,
/// всего дорожек в его кластере). Интервалы — минуты `(start, end)`.
pub fn lanes(items: &[(u32, u32)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| (items[i].0, items[i].1));
    let mut out = vec![(0usize, 1usize); items.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<u32> = Vec::new();
    let mut cluster_end = 0u32;
    let flush = |cluster: &mut Vec<usize>, lanes: usize, out: &mut Vec<(usize, usize)>| {
        for &i in cluster.iter() {
            out[i].1 = lanes.max(1);
        }
        cluster.clear();
    };
    for &i in &order {
        let (s, e) = items[i];
        if !cluster.is_empty() && s >= cluster_end {
            let n = lane_ends.len();
            flush(&mut cluster, n, &mut out);
            lane_ends.clear();
        }
        let lane = match lane_ends.iter().position(|&end| end <= s) {
            Some(l) => {
                lane_ends[l] = e;
                l
            }
            None => {
                lane_ends.push(e);
                lane_ends.len() - 1
            }
        };
        out[i].0 = lane;
        cluster.push(i);
        cluster_end = cluster_end.max(e);
    }
    let n = lane_ends.len();
    flush(&mut cluster, n, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip_and_calendars() {
        let mut s = CalendarStore::template("Личное");
        let c1 = s.calendars[0].id.clone();
        let work = s.add_calendar("Работа", "");
        let day = parse_days("2026-09-03").unwrap();
        let mut e = CalEvent::new(&work, "Стендап", day);
        e.start = Some(9 * 60);
        e.end = Some(9 * 60 + 30);
        e.all_day = false;
        let id = s.add_event(e);
        let back = CalendarStore::parse(&s.serialize()).unwrap();
        assert_eq!(back, s);
        assert!(s.remove_calendar(&work), "удаление непоследнего календаря");
        assert_eq!(s.event(&id).unwrap().calendar, c1, "события переезжают в оставшийся");
        assert!(!s.remove_calendar(&c1), "последний не удаляется");
        // Событие без календаря — в первый.
        let orphan = CalendarStore::parse(r#"{"events":[{"id":"x","calendar":"нет","title":"t","date":"2026-01-01"}]}"#).unwrap();
        assert_eq!(orphan.events[0].calendar, orphan.calendars[0].id);
        assert!(orphan.events[0].all_day);
        assert_eq!(parse_hm("09:05"), Some(545));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(fmt_hm(545), "09:05");
    }

    #[test]
    fn occurrences_expand_repeats_and_spans() {
        let mut s = CalendarStore::template("Личное");
        let cal = s.calendars[0].id.clone();
        let d = |iso: &str| parse_days(iso).unwrap();
        let mut weekly = CalEvent::new(&cal, "Еженедельно", d("2026-09-01"));
        weekly.repeat = Repeat::Weekly;
        weekly.until = Some("2026-09-30".into());
        weekly.start = Some(600);
        weekly.end = Some(660);
        weekly.all_day = false;
        s.add_event(weekly);
        let mut multi = CalEvent::new(&cal, "Отпуск", d("2026-09-10"));
        multi.end_date = Some("2026-09-12".into());
        s.add_event(multi);
        let mut monthly = CalEvent::new(&cal, "31-е", d("2026-01-31"));
        monthly.repeat = Repeat::Monthly;
        s.add_event(monthly);
        let mut yearly = CalEvent::new(&cal, "ДР", d("2020-02-29"));
        yearly.repeat = Repeat::Yearly;
        s.add_event(yearly);

        let sept = s.occurrences(d("2026-09-01"), d("2026-09-30"), &[]);
        let weekly_days: Vec<i64> = sept.iter().filter(|o| o.time.is_some()).map(|o| o.day - d("2026-09-01")).collect();
        assert_eq!(weekly_days, [0, 7, 14, 21, 28], "еженедельные вторники до until");
        let vacation: Vec<(bool, bool)> = sept.iter().filter(|o| o.time.is_none() && (d("2026-09-10")..=d("2026-09-12")).contains(&o.day) && s.event(&o.event).unwrap().title == "Отпуск").map(|o| (o.first, o.last)).collect();
        assert_eq!(vacation, [(true, false), (false, false), (false, true)]);
        // Месячный повтор 31-го: сентябрь пропускается.
        let sept31 = sept.iter().filter(|o| s.event(&o.event).unwrap().title == "31-е").count();
        assert_eq!(sept31, 0);
        let oct = s.occurrences(d("2026-10-01"), d("2026-10-31"), &[]);
        assert_eq!(oct.iter().filter(|o| s.event(&o.event).unwrap().title == "31-е").count(), 1);
        // Годовой 29 февраля — только в високосные.
        let feb28 = s.occurrences(d("2028-02-01"), d("2028-02-29"), &[]);
        assert_eq!(feb28.iter().filter(|o| s.event(&o.event).unwrap().title == "ДР").count(), 1);
        let feb27 = s.occurrences(d("2027-02-01"), d("2027-02-28"), &[]);
        assert_eq!(feb27.iter().filter(|o| s.event(&o.event).unwrap().title == "ДР").count(), 0);
        // Фильтр по календарю.
        let other = s.add_calendar("Другой", "red");
        assert!(s.occurrences(d("2026-09-01"), d("2026-09-30"), &[other]).is_empty());
        // Порядок в дне: весь день раньше времени.
        let day10 = s.occurrences(d("2026-09-08"), d("2026-09-08"), &[]);
        assert!(day10.windows(2).all(|w| w[0].time.is_none() || w[1].time.is_some()));
    }

    #[test]
    fn ranges_weeks_and_lanes() {
        let d = |iso: &str| parse_days(iso).unwrap();
        // 2026-09-03 — четверг; неделя с понедельника — с 31.08.
        assert_eq!(week_start(d("2026-09-03"), 0), d("2026-08-31"));
        assert_eq!(week_start(d("2026-09-03"), 6), d("2026-08-30"), "неделя с воскресенья");
        let (from, to) = range_of(CalView::Month, d("2026-09-03"), 0);
        assert_eq!((days_to_iso(from), to - from + 1), ("2026-08-31".to_string(), 42));
        let (from, to) = range_of(CalView::Year, d("2026-09-03"), 0);
        assert_eq!((days_to_iso(from), days_to_iso(to)), ("2026-01-01".to_string(), "2026-12-31".to_string()));
        assert_eq!(range_of(CalView::Day, 5, 0), (5, 5));
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(add_months(2026, 12, 1), (2027, 1));
        assert_eq!(add_months(2026, 1, -1), (2025, 12));
        // Дорожки: два пересекающихся + один отдельный.
        let l = lanes(&[(540, 600), (570, 630), (700, 760)]);
        assert_eq!(l, [(0, 2), (1, 2), (0, 1)]);
        let mut st = CalendarStyle::default();
        st.from_min = 5 * 60 + 3;
        st.to_min = 3 * 60;
        st.slot_min = 7;
        st.sanitize();
        assert_eq!((st.from_min, st.to_min, st.slot_min), (5 * 60, 3 * 60, 30));
        // Конец не позже начала — окно через полночь: 05:00 → 03:00.
        assert_eq!(st.window(), (5 * 60, 22 * 60));
        assert!(st.wraps());
        st.full_day = true;
        assert_eq!(st.window(), (5 * 60, 24 * 60), "полные сутки — 24 часа от начала");
        st.full_day = false;
        st.to_min = 20 * 60;
        st.from_min = 8 * 60;
        assert_eq!(st.window(), (8 * 60, 12 * 60));
        assert!(!st.wraps());
        // Документы до 09.09.2026 хранили целые часы — они разворачиваются
        // в минуты и обратно не пишутся.
        let doc = CalendarDoc::parse(r#"{"version":1,"view":"day","anchor":"2026-09-03","style":{"hour_from":6,"hour_to":21}}"#).unwrap();
        assert_eq!((doc.style.from_min, doc.style.to_min), (6 * 60, 21 * 60));
        assert!(!doc.serialize().contains("hour_from"), "старый ключ обратно не пишется");

        st.apply_preset("light");
        assert_eq!(st.preset, "light");
        assert!(!st.header_bg.is_empty());
        st.apply_preset("theme");
        assert!(st.preset.is_empty() && st.header_bg.is_empty());
    }
}
