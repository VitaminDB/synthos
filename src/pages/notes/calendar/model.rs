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

/// Дни недели повтора: биты 0…6, понедельник — бит 0. Пусто — фильтра нет
/// (повтор идёт каждый свой день). Так одно событие «каждый день, кроме
/// выходных» заменяет дюжину отдельных.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weekdays(u8);

impl Weekdays {
    /// Ключи в порядке битов: понедельник первый (как `weekday_of`).
    pub const KEYS: [&'static str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    const FULL: u8 = 0b0111_1111;
    pub const WEEKDAYS: Weekdays = Weekdays(0b0001_1111);
    pub const WEEKENDS: Weekdays = Weekdays(0b0110_0000);

    /// Маска из битов; все семь дней — то же самое, что «любой день».
    pub fn from_bits(bits: u8) -> Self {
        let b = bits & Self::FULL;
        Weekdays(if b == Self::FULL { 0 } else { b })
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    /// Фильтра нет — повтор идёт по всем дням.
    pub fn is_any(&self) -> bool {
        self.0 == 0
    }

    /// Разрешён ли день недели (0 — понедельник).
    pub fn has(self, weekday: u32) -> bool {
        self.is_any() || self.0 & (1 << (weekday % 7)) != 0
    }

    /// Разрешён ли календарный день (номер дня, как в `parse_days`).
    pub fn allows(self, day: i64) -> bool {
        self.has(weekday_of(day) as u32)
    }

    /// Переключить день: из «любых» разворачивается вся неделя, снятый
    /// последний день возвращает «любые».
    pub fn toggled(self, weekday: u32) -> Self {
        let base = if self.is_any() { Self::FULL } else { self.0 };
        let next = base ^ (1 << (weekday % 7));
        if next & Self::FULL == 0 {
            Weekdays(0)
        } else {
            Self::from_bits(next)
        }
    }

    pub fn keys(self) -> Vec<&'static str> {
        (0..7).filter(|i| self.0 & (1 << i) != 0).map(|i| Self::KEYS[i as usize]).collect()
    }

    /// `mon,wed,fri` — для строк инструмента и логов.
    pub fn label(self) -> String {
        self.keys().join(",")
    }

    /// Один токен: `mon` / `monday` / `пн` / `1`…`7` (1 — понедельник),
    /// либо группа `weekdays` / `weekends` / `all`.
    pub fn parse_token(s: &str) -> Option<Self> {
        let t = s.trim().trim_matches(|c: char| c == '"' || c == '\'').to_lowercase();
        if t.is_empty() {
            return None;
        }
        let group = match t.as_str() {
            "weekdays" | "weekday" | "workdays" | "workday" | "business" | "будни" | "рабочие" => Some(Self::WEEKDAYS),
            "weekends" | "weekend" | "выходные" => Some(Self::WEEKENDS),
            "all" | "any" | "every" | "everyday" | "daily" | "none" | "любые" | "все" => Some(Weekdays(0)),
            _ => None,
        };
        if group.is_some() {
            return group;
        }
        if let Ok(n) = t.parse::<u32>() {
            return (1..=7).contains(&n).then(|| Weekdays(1 << (n - 1)));
        }
        const RU: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
        const RU_FULL: [&str; 7] = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];
        // `monday` → `mon`, `mo` → `mon`, «среда» → «ср»; однобуквенные
        // сокращения неоднозначны и не разбираются.
        let idx = Self::KEYS
            .iter()
            .position(|k| t.starts_with(k))
            .or_else(|| RU.iter().position(|k| t.starts_with(k)))
            .or_else(|| RU_FULL.iter().position(|k| k.starts_with(t.as_str())))
            .or_else(|| (t.chars().count() >= 2).then(|| Self::KEYS.iter().position(|k| k.starts_with(t.as_str()))).flatten());
        idx.map(|i| Weekdays(1 << i))
    }

    /// Строка из нескольких дней: `mon,wed` / `пн вт` / `weekdays`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut bits = 0u8;
        let mut seen = false;
        for part in s.split([',', ';', '/', '|', ' ', '\t']).filter(|p| !p.trim().is_empty()) {
            let w = Self::parse_token(part)?;
            bits |= w.0;
            seen = true;
        }
        seen.then(|| Self::from_bits(bits))
    }

    /// Список из инструмента: строки-дни либо группы; пустой список —
    /// «любые дни».
    pub fn parse_list<S: AsRef<str>>(items: &[S]) -> Result<Self, String> {
        let mut bits = 0u8;
        for it in items {
            let w = Self::parse(it.as_ref()).ok_or_else(|| {
                format!("bad weekday \"{}\" (mon | tue | wed | thu | fri | sat | sun, 1…7, weekdays, weekends)", it.as_ref())
            })?;
            if w.is_any() && items.len() == 1 {
                return Ok(Weekdays(0));
            }
            bits |= w.0;
        }
        Ok(Self::from_bits(bits))
    }

    /// Убрать дни `other` (для `skip_days`): из «любых» сперва
    /// разворачивается вся неделя.
    pub fn without(self, other: Self) -> Self {
        if other.is_any() {
            return self;
        }
        let base = if self.is_any() { Self::FULL } else { self.0 };
        Self::from_bits(base & !other.0)
    }
}

impl Serialize for Weekdays {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.keys().serialize(ser)
    }
}

impl<'de> Deserialize<'de> for Weekdays {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bits(u8),
            One(String),
            Many(Vec<String>),
        }
        Ok(match Raw::deserialize(de)? {
            Raw::Bits(b) => Weekdays::from_bits(b),
            Raw::One(s) => Weekdays::parse(&s).unwrap_or_default(),
            Raw::Many(v) => {
                let bits = v.iter().filter_map(|s| Weekdays::parse(s)).fold(0u8, |a, w| a | w.0);
                Weekdays::from_bits(bits)
            }
        })
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
    /// «Сделано» разового события. У повторяющегося не используется: у
    /// каждого повтора своя отметка в `done_on`, иначе отметка пятницы
    /// оставалась и в понедельник.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Сделанные повторы: ISO-дата первого дня вхождения.
    #[serde(default, rename = "done_dates", skip_serializing_if = "Vec::is_empty")]
    pub done_on: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Свой цвет поверх цвета календаря.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Repeat::is_none")]
    pub repeat: Repeat,
    /// Дни недели повтора (пусто — любые): «каждый день, кроме выходных» —
    /// это `repeat: daily` плюс `only_days: пн…пт`.
    #[serde(default, rename = "only_days", skip_serializing_if = "Weekdays::is_any")]
    pub days: Weekdays,
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
            done_on: Vec::new(),
            note: String::new(),
            color: String::new(),
            repeat: Repeat::None,
            days: Weekdays::default(),
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

    pub fn is_repeating(&self) -> bool {
        !self.repeat.is_none()
    }

    /// Первый день вхождения, накрывающего `day`; `None` — в этот день
    /// события нет.
    pub fn instance_at(&self, day: i64) -> Option<i64> {
        let mut out = Vec::new();
        self.expand(day, day, &mut out);
        out.first().map(|o| o.start)
    }

    /// Сделано ли вхождение, накрывающее `day`: у разового — `done`, у
    /// повтора — отметка именно этого повтора.
    pub fn done_at(&self, day: i64) -> bool {
        if !self.is_repeating() {
            return self.done;
        }
        self.instance_at(day).is_some_and(|start| self.instance_done(start))
    }

    /// Отметить вхождение, накрывающее `day`. `false` — у повтора нет
    /// вхождения в этот день (отмечать нечего).
    pub fn set_done_at(&mut self, day: i64, done: bool) -> bool {
        if !self.is_repeating() {
            self.done = done;
            return true;
        }
        let Some(start) = self.instance_at(day) else { return false };
        let iso = days_to_iso(start);
        self.done_on.retain(|d| *d != iso);
        if done {
            self.done_on.push(iso);
            self.done_on.sort();
        }
        true
    }

    fn instance_done(&self, start: i64) -> bool {
        self.done_on.iter().any(|d| parse_days(d) == Some(start))
    }

    /// Отметки по форме события: у повтора общий `done` не живёт (прежние
    /// файлы хранили его на всю серию — он снимается, а не размазывается
    /// на каждый день), у разового не живут отметки повторов.
    pub fn fix_done(&mut self) {
        if self.is_repeating() {
            self.done = false;
            self.done_on.retain(|d| parse_days(d).is_some());
            self.done_on.sort();
            self.done_on.dedup();
        } else {
            self.done_on.clear();
        }
    }

    /// Вхождения события в диапазон дней `[from, to]` (включительно), без
    /// сортировки.
    pub fn expand(&self, from: i64, to: i64, out: &mut Vec<Occurrence>) {
        let e = self;
        let Some((s, en)) = e.span() else { return };
        let len = en - s;
        let until = e.until.as_deref().and_then(parse_days).unwrap_or(i64::MAX);
        let time = e.time_span();
        let done_days: Vec<i64> = e.done_on.iter().filter_map(|d| parse_days(d)).collect();
        let mut push = |start_day: i64| {
            let done = if e.is_repeating() { done_days.contains(&start_day) } else { e.done };
            for d in start_day..=start_day + len {
                if d < from || d > to {
                    continue;
                }
                out.push(Occurrence { event: e.id.clone(), day: d, start: start_day, time, first: d == start_day, last: d == start_day + len, done });
            }
        };
        match e.repeat {
            Repeat::None => push(s),
            // Фильтр по дням недели превращает и еженедельный повтор в
            // обход по дням: «каждую неделю по пн/ср/пт» — это те же
            // дни маски, а не одно вхождение в неделю.
            Repeat::Daily | Repeat::Weekly if !e.days.is_any() => {
                let mut d = s.max(from - len);
                while d <= to && d <= until {
                    if d >= s && e.days.allows(d) {
                        push(d);
                    }
                    d += 1;
                }
            }
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
                        if d >= s && e.days.allows(d) {
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
                        if d >= s && e.days.allows(d) {
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
}

/// Вхождение события в конкретный день (развёртка повторов и многодневных).
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub event: String,
    pub day: i64,
    /// Первый день вхождения (у повтора — день этого повтора).
    pub start: i64,
    /// Минуты начала/конца, `None` — весь день.
    pub time: Option<(u32, u32)>,
    /// Первый/последний день многодневного события.
    pub first: bool,
    pub last: bool,
    /// Сделано ли это вхождение (у повтора — свой день, не вся серия).
    pub done: bool,
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
            e.days = Weekdays::from_bits(e.days.bits());
            e.fix_done();
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
        event.fix_done();
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
            e.expand(from, to, &mut out);
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

/// `ДД.ММ` — день повтора в подписи «Сделано».
pub fn day_month(day: i64) -> String {
    let (_, m, d) = civil_from_days(day);
    format!("{d:02}.{m:02}")
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
    fn weekday_mask_filters_repeats() {
        let mut s = CalendarStore::template("Личное");
        let cal = s.calendars[0].id.clone();
        let d = |iso: &str| parse_days(iso).unwrap();
        // 2026-09-14 — понедельник: одно событие «каждый будний день» вместо
        // десяти отдельных.
        let mut work = CalEvent::new(&cal, "Планёрка", d("2026-09-14"));
        work.repeat = Repeat::Daily;
        work.days = Weekdays::WEEKDAYS;
        work.until = Some("2026-09-25".into());
        s.add_event(work);
        let occ = s.occurrences(d("2026-09-14"), d("2026-09-30"), &[]);
        assert_eq!(occ.len(), 10, "две рабочие недели");
        assert!(occ.iter().all(|o| weekday_of(o.day) < 5), "выходных быть не должно");
        assert_eq!(occ.last().unwrap().day, d("2026-09-25"), "until обрывает повтор");

        // Еженедельный повтор с маской — это дни маски каждую неделю.
        let mut gym = CalEvent::new(&cal, "Зал", d("2026-09-14"));
        gym.repeat = Repeat::Weekly;
        gym.days = Weekdays::parse("mon,wed").unwrap();
        s.add_event(gym);
        let days: Vec<i64> = s
            .occurrences(d("2026-09-14"), d("2026-09-27"), &[])
            .iter()
            .filter(|o| s.event(&o.event).unwrap().title == "Зал")
            .map(|o| o.day - d("2026-09-14"))
            .collect();
        assert_eq!(days, [0, 2, 7, 9]);

        // Месячный повтор пропускает вхождение, попавшее под фильтр.
        let mut pay = CalEvent::new(&cal, "Оплата", d("2026-09-05"));
        pay.repeat = Repeat::Monthly;
        pay.days = Weekdays::WEEKDAYS;
        s.add_event(pay);
        let sept = s.occurrences(d("2026-09-01"), d("2026-09-30"), &[]);
        assert!(!sept.iter().any(|o| s.event(&o.event).unwrap().title == "Оплата"), "5 сентября — суббота");
        let oct = s.occurrences(d("2026-10-01"), d("2026-10-31"), &[]);
        assert_eq!(oct.iter().filter(|o| s.event(&o.event).unwrap().title == "Оплата").count(), 1, "5 октября — понедельник");
    }

    #[test]
    fn repeat_done_is_per_occurrence() {
        let d = |iso: &str| parse_days(iso).unwrap();
        // «Забрать» по будням: отметка пятницы не перетекает на понедельник.
        let mut s = CalendarStore::template("Личное");
        let mut e = CalEvent::new(&s.calendars[0].id, "Забрать", d("2026-09-07"));
        e.repeat = Repeat::Daily;
        e.days = Weekdays::WEEKDAYS;
        assert!(e.set_done_at(d("2026-09-11"), true));
        assert!(e.done_at(d("2026-09-11")));
        assert!(!e.done_at(d("2026-09-14")), "понедельник открыт");
        assert!(!e.done, "общий флаг серии не ставится");
        assert!(!e.set_done_at(d("2026-09-12"), true), "в субботу вхождения нет");
        s.add_event(e.clone());
        let marks: Vec<(i64, bool)> = s.occurrences(d("2026-09-10"), d("2026-09-14"), &[]).iter().map(|o| (o.day - d("2026-09-10"), o.done)).collect();
        assert_eq!(marks, [(0, false), (1, true), (4, false)]);
        let json = s.serialize();
        assert!(json.contains("done_dates") && !json.contains("\"done\""), "{json}");
        assert_eq!(CalendarStore::parse(&json).unwrap(), s);
        assert!(e.set_done_at(d("2026-09-11"), false));
        assert!(e.done_on.is_empty(), "снять — только свой день");

        // Многодневный повтор отмечается днём начала вхождения.
        let mut trip = CalEvent::new("c", "Выезд", d("2026-09-05"));
        trip.end_date = Some("2026-09-06".into());
        trip.repeat = Repeat::Weekly;
        assert!(trip.set_done_at(d("2026-09-13"), true), "воскресенье второй поездки");
        assert_eq!(trip.done_on, ["2026-09-12"]);
        assert!(trip.done_at(d("2026-09-12")) && !trip.done_at(d("2026-09-06")));

        // Прежние файлы хранили `done` на всей серии: он снимается, а не
        // красит каждый день.
        let old = CalendarStore::parse(r#"{"events":[{"id":"x","calendar":"c","title":"t","date":"2026-09-07","repeat":"daily","only_days":"weekdays","done":true}]}"#).unwrap();
        assert!(!old.events[0].done_at(d("2026-09-14")));
        assert!(!old.serialize().contains("\"done\""));

        // Разовое событие — по-прежнему один флаг; отметок повторов у него нет.
        let mut once = CalEvent::new("c", "Разово", d("2026-09-14"));
        assert!(once.set_done_at(d("2020-01-01"), true));
        assert!(once.done && once.done_at(d("2026-09-14")));
        trip.repeat = Repeat::None;
        trip.fix_done();
        assert!(trip.done_on.is_empty());
    }

    #[test]
    fn weekdays_parse_and_roundtrip() {
        assert_eq!(Weekdays::parse("weekdays"), Some(Weekdays::WEEKDAYS));
        assert_eq!(Weekdays::parse("выходные"), Some(Weekdays::WEEKENDS));
        assert_eq!(Weekdays::parse("пн, ср, пт").unwrap().label(), "mon,wed,fri");
        assert_eq!(Weekdays::parse("Monday tue 3").unwrap().label(), "mon,tue,wed");
        assert_eq!(Weekdays::parse("суббота").unwrap().label(), "sat");
        assert!(Weekdays::parse("mon,funday").is_none());
        // Все семь дней — это отсутствие фильтра.
        assert!(Weekdays::parse("mon tue wed thu fri sat sun").unwrap().is_any());
        assert!(Weekdays::WEEKENDS.has(5) && !Weekdays::WEEKENDS.has(0));
        assert_eq!(Weekdays::default().without(Weekdays::WEEKENDS), Weekdays::WEEKDAYS);
        assert!(Weekdays::WEEKDAYS.toggled(5).toggled(6).is_any(), "вся неделя — снова «любые»");

        let mut e = CalEvent::new("c", "t", parse_days("2026-09-14").unwrap());
        e.repeat = Repeat::Daily;
        e.days = Weekdays::WEEKDAYS;
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""only_days":["mon","tue","wed","thu","fri"]"#), "{json}");
        let back: CalEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.days, Weekdays::WEEKDAYS);
        // Старые файлы без поля читаются как «любые дни».
        let old = CalendarStore::parse(r#"{"events":[{"id":"x","calendar":"c","title":"t","date":"2026-01-01","repeat":"daily"}]}"#).unwrap();
        assert!(old.events[0].days.is_any());
        // Строкой тоже: "weekends" в руках человека, правившего JSON.
        let str_form = CalendarStore::parse(r#"{"events":[{"id":"x","calendar":"c","title":"t","date":"2026-01-01","repeat":"daily","only_days":"weekends"}]}"#).unwrap();
        assert_eq!(str_form.events[0].days, Weekdays::WEEKENDS);
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
