//! Формат канбан-доски: `notes/objects/<id>.kanban.json`.
//!
//! Доска самодостаточна: колонки (название, цвет, своя ширина), карточки
//! (заголовок + markdown-содержимое + приоритет, метки, срок) и настройки
//! внешнего вида. Порядок карточек в колонке — порядок в `cards`; никакой
//! «базы» под доской нет, всё правится прямо на ней и в панели свойств.
//! Прогресс чек-листа не хранится — считается по `- [ ]`/`- [x]` в
//! содержимом ([`checklist_progress`]).
//!
//! **Жизненный цикл карточки.** У колонки есть флаг `done`; карточка,
//! попавшая в такую колонку, получает штамп `done` (день), при возврате —
//! теряет его; `created` ставится при первом появлении на доске. Карточка
//! с `repeat` при закрытии рождает следующую со сдвинутым сроком, а
//! закрытые старше `archive_after` дней уходят в `archive` (с доски, но не
//! из файла). Всё это делает одна доводка после любой правки —
//! [`KanbanDoc::reconcile`]: она же возвращает список изменений для журнала
//! проекта, поэтому и мышиный перенос, и вызов агента журналируются
//! одинаково, без правок в каждом месте.
//!
//! **Таймеры колонок.** Карточка помнит, когда попала в свою колонку
//! (`entered`, unix-секунды; сбрасывается при любом переезде, а не считается
//! от создания). У колонки два правила: хранение `keep_hours` — пролежавшая
//! дольше карточка уходит в архив доски (мягкое удаление: возврат — в ту же
//! колонку), и эскалация `move_after_hours` + `move_to` — переезд в другую
//! колонку («В работе» 72 ч → «Просроченные»). Срабатывает то, что
//! наступает раньше. Правила проверяет та же доводка: после правок и раз в
//! минуту без них (`KanbanHandle::sweep`). Автоархив закрытых
//! (`archive_after`) от них не зависит — он считает дни от штампа `done`.

use serde::{Deserialize, Serialize};
use syngui::core::Color;

pub use super::super::calendar::model::Repeat;
use super::super::calendar::model::{fmt_hm, parse_hm};
use super::super::gantt::calendar::{civil_from_days, days_from_civil, days_to_iso, parse_days};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub columns: Vec<KanbanColumn>,
    #[serde(default)]
    pub cards: Vec<KanbanCard>,
    #[serde(default)]
    pub style: KanbanStyle,
    /// Архив: закрытые карточки, убранные с доски автоархивом или вручную.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive: Vec<KanbanCard>,
    /// Через сколько дней после закрытия карточка уходит в архив;
    /// `None` — никогда.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_after: Option<u32>,
}

fn default_version() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanColumn {
    pub id: String,
    pub name: String,
    /// `#rrggbb`; пусто — без цветной метки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// Своя ширина; без неё — общая из [`KanbanStyle::column_width`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Колонка «готово»: карточки в ней считаются закрытыми (штамп `done`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Хранение: через сколько часов пребывания в колонке карточка уходит в
    /// архив доски; `None` — хранится всегда.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_hours: Option<u32>,
    /// Эскалация: через сколько часов пребывания карточка переезжает в
    /// колонку `move_to`; без колонки правило не действует.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_after_hours: Option<u32>,
    /// id колонки эскалации.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
}

impl KanbanColumn {
    pub fn new(id: String, name: impl Into<String>, color: impl Into<String>, done: bool) -> Self {
        Self {
            id,
            name: name.into(),
            color: color.into(),
            width: None,
            done,
            keep_hours: None,
            move_after_hours: None,
            move_to: None,
        }
    }

    /// Задано ли у колонки хоть одно правило времени.
    pub fn has_timer(&self) -> bool {
        self.keep_hours.is_some() || (self.move_after_hours.is_some() && self.move_to.is_some())
    }
}

/// Секунд в часе — единица таймеров колонок.
pub const HOUR_SECS: i64 = 3600;

/// Сколько шагов по таймерам делает одна доводка: хватает на цепочку
/// A → B → C, пропущенную закрытым приложением, и не крутится вечно на
/// взаимных правилах A ⇄ B (остаток — на следующем такте).
const TIMER_HOPS: usize = 16;

/// Что таймер колонки сделает с карточкой.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimerAction {
    /// Срок хранения вышел — в архив доски.
    Expire,
    /// Эскалация — в колонку с этим id.
    Move(String),
}

/// Ближайшее срабатывание таймера карточки.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardTimer {
    /// Момент срабатывания, unix-секунды.
    pub at: i64,
    pub action: TimerAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanCard {
    pub id: String,
    /// id колонки.
    pub column: String,
    #[serde(default)]
    pub title: String,
    /// Markdown-содержимое (многострочное).
    #[serde(default)]
    pub md: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Срок, ISO `yyyy-mm-dd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Оценка длительности в минутах (в панели — часы/дни).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    /// Плановое начало: `yyyy-mm-dd` либо `yyyy-mm-ddThh:mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// Плановый конец (включительно), тот же формат.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// День появления на доске, ISO `yyyy-mm-dd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// День закрытия (переезда в колонку «готово»); снимается при возврате.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<String>,
    /// Когда карточка попала в свою колонку, unix-секунды — отсчёт таймеров
    /// колонки. Ставится доводкой при появлении и при каждом переезде.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered: Option<i64>,
    /// Повтор: закрытие рождает следующую карточку со сдвинутым сроком.
    #[serde(default, skip_serializing_if = "Repeat::is_none")]
    pub repeat: Repeat,
    /// Вложения: картинки (миниатюры) и файлы (скрепка) из бандла.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<CardFile>,
}

/// Вложение карточки — файл бандла `asset:<sha>.<ext>` с исходным именем.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFile {
    /// `asset:<sha256>.<ext>`.
    pub url: String,
    /// Имя для показа (исходное имя файла).
    #[serde(default)]
    pub name: String,
}

impl CardFile {
    pub fn new(url: impl Into<String>, name: impl Into<String>) -> Self {
        Self { url: url.into(), name: name.into() }
    }

    /// Расширение вложения (по ссылке).
    pub fn ext(&self) -> String {
        self.url.rsplit('.').next().unwrap_or_default().to_ascii_lowercase()
    }

    /// Картинка — показывается миниатюрой, остальное — скрепкой.
    pub fn is_image(&self) -> bool {
        matches!(self.ext().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg")
    }

    /// Имя для показа: своё либо хвост ссылки.
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            self.url.strip_prefix("asset:").unwrap_or(&self.url).to_string()
        } else {
            self.name.clone()
        }
    }
}

impl KanbanCard {
    /// Пустая карточка без полей — в колонке и в бандле не нужна.
    pub fn is_empty(&self) -> bool {
        self.title.trim().is_empty()
            && self.md.trim().is_empty()
            && self.priority.is_none()
            && self.tags.is_empty()
            && self.due.is_none()
            && self.duration.is_none()
            && self.start.is_none()
            && self.end.is_none()
            && self.files.is_empty()
    }

    /// Новая карточка колонки без содержимого.
    pub fn new(id: String, column: String) -> Self {
        Self {
            id,
            column,
            title: String::new(),
            md: String::new(),
            priority: None,
            tags: Vec::new(),
            due: None,
            duration: None,
            start: None,
            end: None,
            created: None,
            done: None,
            entered: None,
            repeat: Repeat::None,
            files: Vec::new(),
        }
    }

    /// Закрыта ли карточка (лежит в колонке «готово»).
    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }

    /// Следующая карточка повтора: тот же текст с неотмеченным чек-листом,
    /// срок сдвинут на период от большего из срока и `today` (просроченная
    /// привычка не тянет за собой хвост пропущенных дат), штампы сброшены.
    pub fn next_repeat(&self, id: String, column: String, today: i64) -> Self {
        let prev = self.due.as_deref().and_then(parse_days);
        let base = prev.map(|d| d.max(today)).unwrap_or(today);
        let next_due = next_period(base, self.repeat);
        let mut next = self.clone();
        next.id = id;
        next.column = column;
        next.due = Some(days_to_iso(next_due));
        next.created = None;
        next.done = None;
        next.entered = None;
        next.md = uncheck(&self.md);
        // План (начало/конец) едет за сроком: на тот же период, чтобы
        // полоса следующей привычки встала в календарь сама.
        if let Some(s) = self.start_at() {
            let delta = match prev {
                Some(p) => next_due - p,
                None => next_period(s.day.max(today), self.repeat) - s.day,
            };
            next.shift_schedule(delta);
        }
        next
    }

    /// `(сделано, всего)` по пунктам чек-листа содержимого; `None` — их нет.
    pub fn checklist(&self) -> Option<(usize, usize)> {
        let (done, total) = checklist_progress(&self.md);
        (total > 0).then_some((done, total))
    }

    pub fn start_at(&self) -> Option<Moment> {
        self.start.as_deref().and_then(Moment::parse)
    }

    pub fn end_at(&self) -> Option<Moment> {
        self.end.as_deref().and_then(Moment::parse)
    }

    /// Полоса карточки на календаре и в Ганте. Недостающий конец берётся
    /// из оценки, недостающее начало — отсчётом назад от конца. Без обеих
    /// дат полосы нет: срок (`due`) остаётся точкой, как и был.
    pub fn schedule(&self) -> Option<CardSpan> {
        match (self.start_at(), self.end_at()) {
            (None, None) => None,
            (Some(s), Some(e)) => Some(span_between(s, e)),
            (Some(s), None) => Some(span_forward(s, self.duration)),
            (None, Some(e)) => Some(span_backward(e, self.duration)),
        }
    }

    /// «В календарь»: начало — момент `day` (и `min`), конец — по оценке.
    pub fn schedule_at(&mut self, day: i64, min: Option<u32>) {
        let s = Moment { day, min };
        let span = span_forward(s, self.duration);
        self.start = Some(s.iso());
        self.end = Some(Moment { day: span.end_day, min: span.time.map(|(_, e)| e) }.iso());
    }

    /// Снять план: полоса исчезает, срок и оценка остаются.
    pub fn unschedule(&mut self) -> bool {
        let had = self.start.is_some() || self.end.is_some();
        self.start = None;
        self.end = None;
        had
    }

    /// Перенести полосу на `delta` дней (время и оценка сохраняются) —
    /// перетаскивание в календаре и в Ганте.
    pub fn shift_schedule(&mut self, delta: i64) -> bool {
        if delta == 0 {
            return false;
        }
        let (s, e) = (self.start_at(), self.end_at());
        if s.is_none() && e.is_none() {
            return false;
        }
        if let Some(s) = s {
            self.start = Some(Moment { day: s.day + delta, min: s.min }.iso());
        }
        if let Some(e) = e {
            self.end = Some(Moment { day: e.day + delta, min: e.min }.iso());
        }
        true
    }

    /// Задать полосу днями (растягивание кромки бара): время начала и
    /// конца сохраняется, оценка следует за новой длиной.
    pub fn set_span_days(&mut self, start_day: i64, end_day: i64) {
        let end_day = end_day.max(start_day);
        let s_min = self.start_at().and_then(|m| m.min);
        let e_min = self.end_at().and_then(|m| m.min);
        self.start = Some(Moment { day: start_day, min: s_min }.iso());
        self.end = Some(Moment { day: end_day, min: e_min }.iso());
        self.duration = match (start_day == end_day, s_min, e_min) {
            (true, Some(a), Some(b)) if b > a => Some(b - a),
            (true, Some(_), _) => self.duration.filter(|d| *d < DAY_MIN).or(Some(60)),
            _ => Some(((end_day - start_day + 1) as u32).saturating_mul(DAY_MIN)),
        };
    }

    /// Полоса строкой — журнал, панель свойств, вывод агенту.
    pub fn span_text(&self) -> String {
        let Some(span) = self.schedule() else { return String::new() };
        match span.time {
            Some((a, b)) => format!("{} {}–{}", days_to_iso(span.start_day), fmt_hm(a), fmt_hm(b)),
            None if span.start_day == span.end_day => days_to_iso(span.start_day),
            None => format!("{} → {}", days_to_iso(span.start_day), days_to_iso(span.end_day)),
        }
    }
}

// ─── Планирование: момент, оценка, полоса ─────────────────────────────────

/// Минут в сутках — «один день» оценки.
pub const DAY_MIN: u32 = 24 * 60;

/// Момент карточки: день от эпохи и минуты с полуночи (`None` — без
/// времени, «весь день»).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub day: i64,
    pub min: Option<u32>,
}

impl Moment {
    /// `yyyy-mm-dd` либо `yyyy-mm-ddThh:mm` (вместо `T` годится пробел).
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        let (date, time) = match t.split_once(['T', 't', ' ']) {
            Some((d, rest)) => (d, Some(rest)),
            None => (t, None),
        };
        let day = parse_days(date)?;
        let min = match time.map(str::trim).filter(|t| !t.is_empty()) {
            None => None,
            Some(t) => Some(parse_hm(t)?),
        };
        Some(Self { day, min })
    }

    pub fn iso(&self) -> String {
        match self.min {
            None => days_to_iso(self.day),
            Some(m) => format!("{}T{}", days_to_iso(self.day), fmt_hm(m)),
        }
    }
}

/// Полоса карточки: дни включительно и — у однодневной задачи со временем
/// — минуты начала/конца.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardSpan {
    pub start_day: i64,
    pub end_day: i64,
    pub time: Option<(u32, u32)>,
}

impl CardSpan {
    pub fn days(&self) -> i64 {
        self.end_day - self.start_day + 1
    }
}

/// Оценка по умолчанию: час у момента со временем, сутки у «весь день».
fn default_duration(m: Moment) -> u32 {
    if m.min.is_some() {
        60
    } else {
        DAY_MIN
    }
}

/// Число суток, в которые укладывается оценка (минимум одни).
fn duration_days(min: u32) -> i64 {
    min.max(1).div_ceil(DAY_MIN) as i64
}

fn span_forward(s: Moment, duration: Option<u32>) -> CardSpan {
    let dur = duration.unwrap_or_else(|| default_duration(s)).max(1);
    match s.min {
        Some(from) if from + dur <= DAY_MIN => CardSpan { start_day: s.day, end_day: s.day, time: Some((from, from + dur)) },
        Some(from) => CardSpan { start_day: s.day, end_day: s.day + duration_days(from + dur) - 1, time: None },
        None => CardSpan { start_day: s.day, end_day: s.day + duration_days(dur) - 1, time: None },
    }
}

fn span_backward(e: Moment, duration: Option<u32>) -> CardSpan {
    let dur = duration.unwrap_or_else(|| default_duration(e)).max(1);
    match e.min {
        Some(to) if dur <= to => CardSpan { start_day: e.day, end_day: e.day, time: Some((to - dur, to)) },
        _ => CardSpan { start_day: e.day - duration_days(dur) + 1, end_day: e.day, time: None },
    }
}

fn span_between(s: Moment, e: Moment) -> CardSpan {
    let (s, e) = if (e.day, e.min.unwrap_or(0)) < (s.day, s.min.unwrap_or(0)) { (e, s) } else { (s, e) };
    if s.day == e.day && (s.min.is_some() || e.min.is_some()) {
        let from = s.min.unwrap_or(0);
        let to = e.min.unwrap_or(DAY_MIN).clamp(from + 5, DAY_MIN);
        return CardSpan { start_day: s.day, end_day: s.day, time: Some((from, to)) };
    }
    CardSpan { start_day: s.day, end_day: e.day, time: None }
}

/// Оценка из строки: `1w`, `1d`, `2h`, `90m`, `1.5h`, `1d 4h`, `30 мин`,
/// `2 дня`; голое число — часы. `None` — пусто либо мусор.
pub fn parse_duration(s: &str) -> Option<u32> {
    let t = s.trim().to_lowercase();
    if t.is_empty() || t == "none" || t == "null" || t == "off" {
        return None;
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut units = 0u32;
    // Буква без числа перед ней — часть слова («мин», «дня»), а не единица.
    let take = |num: &mut String, mult: f64, total: &mut f64, units: &mut u32| {
        let Ok(v) = num.parse::<f64>() else {
            num.clear();
            return;
        };
        *total += v * mult;
        num.clear();
        *units += 1;
    };
    for ch in t.chars() {
        match ch {
            c if c.is_ascii_digit() => num.push(c),
            '.' | ',' => num.push('.'),
            'w' => take(&mut num, 7.0 * DAY_MIN as f64, &mut total, &mut units),
            'd' | 'д' => take(&mut num, DAY_MIN as f64, &mut total, &mut units),
            'h' | 'ч' => take(&mut num, 60.0, &mut total, &mut units),
            'm' | 'м' => take(&mut num, 1.0, &mut total, &mut units),
            _ => {}
        }
    }
    if !num.is_empty() {
        // Голое число — часы: «оценка 2» читается как два часа.
        take(&mut num, 60.0, &mut total, &mut units);
    }
    (units > 0 && total >= 1.0).then(|| total.round().min(u32::MAX as f64) as u32)
}

/// Часы таймера колонки из строки: `72`, `72h`, `3d`, `1d 12h`, `90m`
/// (вверх до целого часа). Пусто, `0`, `∞`, `none`, `forever` — правила нет.
/// `Err` — мусор.
#[allow(clippy::result_unit_err)]
pub fn parse_hours(s: &str) -> Result<Option<u32>, ()> {
    let t = s.trim().to_lowercase();
    if matches!(t.as_str(), "" | "0" | "∞" | "inf" | "none" | "null" | "off" | "never" | "forever" | "always" | "всегда") {
        return Ok(None);
    }
    let min = parse_duration(&t).ok_or(())?;
    Ok(Some(min.div_ceil(60).max(1)))
}

/// Оценка строкой: `1d`, `2h`, `1d 4h`, `90m`; ноль — пусто.
pub fn fmt_duration(min: u32) -> String {
    if min == 0 {
        return String::new();
    }
    let (d, h, m) = (min / DAY_MIN, (min % DAY_MIN) / 60, min % 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    parts.join(" ")
}

/// Приоритет карточки: цветной бейдж в шапке.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub const ALL: [Priority; 4] = [Priority::Low, Priority::Medium, Priority::High, Priority::Urgent];

    pub fn key(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Ключ подписи в каталоге i18n.
    pub fn i18n_key(self) -> String {
        format!("notes.kanban.priority.{}", self.key())
    }

    pub fn color(self) -> &'static str {
        match self {
            Priority::Low => "#4F8CFF",
            Priority::Medium => "#E8A33D",
            Priority::High => "#EE5E48",
            Priority::Urgent => "#C03E3E",
        }
    }
}

/// `(сделано, всего)` по строкам `- [ ]` / `- [x]` (и `*`, и нумерованным).
pub fn checklist_progress(md: &str) -> (usize, usize) {
    let mut done = 0;
    let mut total = 0;
    for line in md.lines() {
        let t = line.trim_start();
        let rest = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| t.strip_prefix("+ "))
            .or_else(|| {
                let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
                (digits > 0).then(|| t[digits..].strip_prefix(". ")).flatten()
            });
        let Some(rest) = rest else { continue };
        if rest.starts_with("[ ] ") || rest == "[ ]" {
            total += 1;
        } else if rest.starts_with("[x] ") || rest.starts_with("[X] ") || rest == "[x]" || rest == "[X]" {
            total += 1;
            done += 1;
        }
    }
    (done, total)
}

/// Дата через один период повтора от `day`; без повтора — сам `day`.
pub fn next_period(day: i64, repeat: Repeat) -> i64 {
    match repeat {
        Repeat::None => day,
        Repeat::Daily => day + 1,
        Repeat::Weekly => day + 7,
        Repeat::Monthly | Repeat::Yearly => {
            let (y, m, d) = civil_from_days(day);
            let months = if repeat == Repeat::Monthly { 1 } else { 12 };
            let (ny, nm) = super::super::calendar::model::add_months(y, m, months);
            let last = super::super::calendar::model::days_in_month(ny, nm);
            days_from_civil(ny, nm, d.min(last))
        }
    }
}

/// Чек-лист без отметок: `- [x]` → `- [ ]` (для следующей карточки повтора).
pub fn uncheck(md: &str) -> String {
    md.lines()
        .map(|l| {
            let trimmed = l.trim_start();
            let indent = &l[..l.len() - trimmed.len()];
            let mut rest = trimmed;
            let mut prefix = String::new();
            for marker in ["- ", "* ", "+ "] {
                if let Some(r) = rest.strip_prefix(marker) {
                    prefix.push_str(marker);
                    rest = r;
                    break;
                }
            }
            if prefix.is_empty() {
                let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
                if digits > 0 && (rest[digits..].starts_with(". ") || rest[digits..].starts_with(") ")) {
                    prefix.push_str(&rest[..digits + 2]);
                    rest = &rest[digits + 2..];
                }
            }
            if !prefix.is_empty() && (rest.starts_with("[x]") || rest.starts_with("[X]")) {
                format!("{indent}{prefix}[ ]{}", &rest[3..])
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Названия колонки, по которым доска без явного флага узнаёт «готово».
pub const DONE_COLUMN_NAMES: [&str; 12] = [
    "готово", "сделано", "выполнено", "завершено", "закрыто", "готовые",
    "done", "complete", "completed", "finished", "closed", "fertig",
];

/// Изменение карточки для журнала проекта (результат [`KanbanDoc::reconcile`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardChange {
    pub card: String,
    pub title: String,
    pub kind: CardChangeKind,
    /// Откуда / что было (название колонки, прежний срок…).
    pub from: String,
    /// Куда / что стало.
    pub to: String,
    /// Изменение сделал таймер колонки, а не человек или агент.
    pub auto: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardChangeKind {
    Added,
    Deleted,
    Moved,
    Done,
    Reopened,
    DueChanged,
    PriorityChanged,
    /// Изменилась плановая полоса (начало/конец/оценка).
    Scheduled,
    Archived,
    Restored,
    /// Родилась следующая карточка повтора (`card` — новая).
    Repeated,
    /// Вышел срок хранения в колонке — карточка ушла в архив.
    Expired,
    /// Эскалация: таймер колонки перенёс карточку в другую.
    AutoMoved,
}

impl CardChangeKind {
    /// Ключ действия в журнале.
    pub fn key(self) -> &'static str {
        match self {
            CardChangeKind::Added => "add",
            CardChangeKind::Deleted => "delete",
            CardChangeKind::Moved => "move",
            CardChangeKind::Done => "done",
            CardChangeKind::Reopened => "reopen",
            CardChangeKind::DueChanged => "due",
            CardChangeKind::PriorityChanged => "priority",
            CardChangeKind::Scheduled => "schedule",
            CardChangeKind::Archived => "archive",
            CardChangeKind::Restored => "restore",
            CardChangeKind::Repeated => "repeat",
            CardChangeKind::Expired => "expire",
            CardChangeKind::AutoMoved => "auto_move",
        }
    }
}

/// Метки из строки ввода: через запятую/точку с запятой, без пустых и
/// повторов, в порядке ввода.
pub fn parse_tags(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in input.split([',', ';', '\n']) {
        let t = raw.trim().trim_start_matches('#').trim();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// Цвет метки — стабильно по её тексту (без отдельной настройки).
pub fn tag_color(tag: &str) -> &'static str {
    let mut h: u32 = 2166136261;
    for b in tag.to_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    // Первый цвет палитры — серый «без цвета», метки берут остальные.
    PALETTE[1 + (h as usize % (PALETTE.len() - 1))]
}

/// Текст карточки для просмотра: markdown без разметки, по строке на
/// абзац/пункт (`Text` покажет первые несколько строк).
pub fn preview_text(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in md.lines() {
        let mut t = raw.trim();
        if t.is_empty() || t.starts_with("```") || t.starts_with("![[") || t.starts_with("![") {
            continue;
        }
        // Заголовки, цитаты, списки, чекбоксы.
        t = t.trim_start_matches('#').trim_start();
        t = t.trim_start_matches('>').trim_start();
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            if let Some(rest) = t[digits..].strip_prefix(". ") {
                t = rest;
            }
        }
        for m in ["- ", "* ", "+ "] {
            if let Some(rest) = t.strip_prefix(m) {
                t = rest;
                break;
            }
        }
        let t = t
            .replace("[ ] ", "☐ ")
            .replace("[x] ", "☑ ")
            .replace("[X] ", "☑ ");
        // Инлайн-разметка: ссылки, жирный/курсив/код.
        let mut plain = String::with_capacity(t.len());
        let mut chars = t.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '~' => {}
                '[' => {
                    if chars.peek() == Some(&'[') {
                        chars.next();
                        let inner: String = chars.by_ref().take_while(|c| *c != ']').collect();
                        if chars.peek() == Some(&']') {
                            chars.next();
                        }
                        plain.push_str(inner.split('|').next_back().unwrap_or(""));
                    } else {
                        let inner: String = chars.by_ref().take_while(|c| *c != ']').collect();
                        if chars.peek() == Some(&'(') {
                            chars.next();
                            for c in chars.by_ref() {
                                if c == ')' {
                                    break;
                                }
                            }
                        }
                        plain.push_str(&inner);
                    }
                }
                _ => plain.push(c),
            }
        }
        let plain = plain.trim().to_string();
        if !plain.is_empty() {
            lines.push(plain);
        }
    }
    lines.join("\n")
}

/// Место вставки на доске: колонка и карточка, перед которой встать
/// (`None` — в конец колонки).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropSpot {
    pub column: String,
    pub before: Option<String>,
}

impl DropSpot {
    pub fn end(column: &str) -> Self {
        Self { column: column.to_string(), before: None }
    }

    pub fn before(column: &str, card: &str) -> Self {
        Self { column: column.to_string(), before: Some(card.to_string()) }
    }
}

/// Внешний вид доски (панель «Свойства» ▸ «Внешний вид»).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KanbanStyle {
    /// Ширина колонки по умолчанию.
    #[serde(default = "default_column_width")]
    pub column_width: f32,
    /// Фон колонки, `#rrggbb`; пусто — тема.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lane_bg: String,
    /// Фон карточки, `#rrggbb`; пусто — тема.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub card_bg: String,
    /// Счётчик карточек в шапке колонки.
    #[serde(default = "default_true")]
    pub show_counts: bool,
}

pub const DEFAULT_COLUMN_WIDTH: f32 = 300.0;
pub const MIN_COLUMN_WIDTH: f32 = 140.0;
pub const MAX_COLUMN_WIDTH: f32 = 800.0;

fn default_column_width() -> f32 {
    DEFAULT_COLUMN_WIDTH
}

fn default_true() -> bool {
    true
}

impl Default for KanbanStyle {
    fn default() -> Self {
        Self {
            column_width: DEFAULT_COLUMN_WIDTH,
            lane_bg: String::new(),
            card_bg: String::new(),
            show_counts: true,
        }
    }
}

/// Палитра цветных меток (колонки доски, задачи диаграммы): клик по
/// метке переключает на следующий цвет по кругу.
pub const PALETTE: [&str; 7] =
    ["#8B95A6", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#EE5E48", "#3FB8C4"];

/// Следующий цвет палитры после `current` (незнакомый/пустой → первый).
pub fn next_color(current: &str) -> &'static str {
    let idx = PALETTE.iter().position(|c| c.eq_ignore_ascii_case(current));
    PALETTE[idx.map(|i| (i + 1) % PALETTE.len()).unwrap_or(0)]
}

/// Уникальный короткий id элемента доски/диаграммы (unix-мс + счётчик).
pub fn item_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}{}-{}", crate::config::now_millis(), N.fetch_add(1, Ordering::Relaxed))
}

/// Название колонки по id среди `columns` (когда `self` уже занят
/// изменяемым заимствованием карточки).
fn self_column_name(columns: &[KanbanColumn], id: &str) -> String {
    columns.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default()
}

impl KanbanDoc {
    /// Новая доска с тремя колонками; названия — на языке интерфейса.
    pub fn template(names: [&str; 3]) -> Self {
        let colors = ["#8B95A6", "#E8A33D", "#4FBF7A"];
        Self {
            version: 1,
            columns: names
                .iter()
                .zip(colors)
                .enumerate()
                .map(|(i, (name, color))| KanbanColumn::new(item_id("c"), *name, color, i == 2))
                .collect(),
            cards: Vec::new(),
            style: KanbanStyle::default(),
            archive: Vec::new(),
            archive_after: None,
        }
    }

    /// Разбор файла; доска без флага «готово» узнаёт колонку по названию.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut doc: Self = serde_json::from_str(json)?;
        doc.detect_done_column();
        Ok(doc)
    }

    /// Колонка «готово» по названию, если флаг не стоит ни у одной
    /// (доски до появления флага и созданные агентом со своими колонками).
    /// `true` — что-то отмечено.
    pub fn detect_done_column(&mut self) -> bool {
        if self.columns.iter().any(|c| c.done) {
            return false;
        }
        let mut hit = false;
        for c in &mut self.columns {
            let name = c.name.trim().to_lowercase();
            if DONE_COLUMN_NAMES.contains(&name.as_str()) {
                c.done = true;
                hit = true;
            }
        }
        hit
    }

    pub fn is_done_column(&self, column: &str) -> bool {
        self.columns.iter().any(|c| c.id == column && c.done)
    }

    pub fn column_name(&self, column: &str) -> String {
        self.columns.iter().find(|c| c.id == column).map(|c| c.name.clone()).unwrap_or_default()
    }

    /// Первая колонка «готово».
    pub fn done_column(&self) -> Option<&KanbanColumn> {
        self.columns.iter().find(|c| c.done)
    }

    /// Первая колонка не «готово» — туда рождаются повторы.
    pub fn open_column(&self) -> Option<&KanbanColumn> {
        self.columns.iter().find(|c| !c.done).or(self.columns.first())
    }

    /// Доводка после правки против снимка `before`: штампы `created`/`done`/
    /// `entered`, следующие карточки повторов, таймеры колонок, автоархив;
    /// возвращает изменения для журнала. Перенос в колонку «готово»
    /// записывается одним `Done` (с исходной колонкой), а не `Moved` + `Done`.
    /// `today` — локальный день, `now` — unix-секунды (таймеры колонок).
    pub fn reconcile(&mut self, before: &KanbanDoc, today: i64, now: i64) -> Vec<CardChange> {
        let mut changes = self.stamp(before, today, now);
        // Таймеры: снимок → срабатывание → та же сверка против снимка. Всё,
        // что она нашла, сделал таймер (`auto`); переезд в «готово» так же
        // закрывает карточку и рождает повтор.
        let hops = if self.columns.iter().any(KanbanColumn::has_timer) { TIMER_HOPS } else { 0 };
        for _ in 0..hops {
            let mid = self.clone();
            let fired = self.fire_timers(now);
            if fired.is_empty() {
                break;
            }
            for mut c in self.stamp(&mid, today, now) {
                let action = fired.iter().find(|(id, _)| *id == c.card).map(|(_, a)| a);
                c.kind = match (c.kind, action) {
                    (CardChangeKind::Moved, Some(TimerAction::Move(_))) => CardChangeKind::AutoMoved,
                    (CardChangeKind::Archived, Some(TimerAction::Expire)) => CardChangeKind::Expired,
                    (kind, _) => kind,
                };
                c.auto = true;
                changes.push(c);
            }
        }
        self.auto_archive(today, &mut changes);
        changes
    }

    /// Ближайший таймер карточки: хранение или эскалация её колонки — что
    /// наступит раньше (в один момент — переезд). `None` — правил нет, цель
    /// эскалации пропала или у карточки ещё нет штампа `entered`.
    pub fn next_timer(&self, card: &KanbanCard) -> Option<CardTimer> {
        let entered = card.entered?;
        let col = self.columns.iter().find(|c| c.id == card.column)?;
        let at = |h: u32| entered + h as i64 * HOUR_SECS;
        let expire = col.keep_hours.filter(|h| *h > 0).map(|h| CardTimer { at: at(h), action: TimerAction::Expire });
        let target = col.move_to.as_deref().filter(|t| *t != col.id && self.columns.iter().any(|c| c.id == *t));
        let escalate = match (col.move_after_hours.filter(|h| *h > 0), target) {
            (Some(h), Some(to)) => Some(CardTimer { at: at(h), action: TimerAction::Move(to.to_string()) }),
            _ => None,
        };
        match (expire, escalate) {
            (Some(e), Some(m)) => Some(if e.at < m.at { e } else { m }),
            (e, m) => e.or(m),
        }
    }

    /// Сработавшие к `now` таймеры — по шагу на карточку: истёкшая уходит в
    /// архив, эскалация переносит в конец целевой колонки со штампом
    /// `entered` на момент срабатывания, а не на «сейчас»: цепочка правил,
    /// пропущенная закрытым приложением, проходится с настоящими сроками.
    fn fire_timers(&mut self, now: i64) -> Vec<(String, TimerAction)> {
        let due: Vec<(String, CardTimer)> = self
            .cards
            .iter()
            .filter_map(|c| self.next_timer(c).filter(|t| t.at <= now).map(|t| (c.id.clone(), t)))
            .collect();
        for (id, timer) in &due {
            match &timer.action {
                TimerAction::Expire => {
                    if let Some(pos) = self.cards.iter().position(|c| c.id == *id) {
                        let card = self.cards.remove(pos);
                        self.archive.push(card);
                    }
                }
                TimerAction::Move(to) => {
                    self.move_card(id, to, None);
                    if let Some(c) = self.card_mut(id) {
                        c.entered = Some(timer.at);
                    }
                }
            }
        }
        due.into_iter().map(|(id, t)| (id, t.action)).collect()
    }

    /// Штампы и повторы против снимка `before` — без таймеров и автоархива.
    fn stamp(&mut self, before: &KanbanDoc, today: i64, now: i64) -> Vec<CardChange> {
        let today_iso = days_to_iso(today);
        let mut changes = Vec::new();
        let mut spawned: Vec<KanbanCard> = Vec::new();
        for card in &mut self.cards {
            if card.created.is_none() {
                card.created = Some(today_iso.clone());
            }
            let prev = before.cards.iter().find(|c| c.id == card.id);
            // Отсчёт пребывания: с появления на доске (новая, из архива, с
            // другой доски) и с каждого переезда. Переезд по таймеру ставит
            // свой штамп — его не трогаем; старым карточкам без штампа —
            // «сейчас», а не дата создания.
            let entered_now = match prev {
                None => true,
                Some(p) => (p.column != card.column && card.entered == p.entered) || card.entered.is_none(),
            };
            if entered_now {
                card.entered = Some(now);
            }
            let (cid, ctitle) = (card.id.clone(), card.title.clone());
            let mut change = |kind: CardChangeKind, from: String, to: String| {
                changes.push(CardChange { card: cid.clone(), title: ctitle.clone(), kind, from, to, auto: false });
            };
            match prev {
                None => {
                    if before.archive.iter().any(|c| c.id == card.id) {
                        change(CardChangeKind::Restored, String::new(), before.column_name(&card.column));
                    } else {
                        change(CardChangeKind::Added, String::new(), before.column_name(&card.column));
                    }
                }
                Some(p) => {
                    if p.due != card.due {
                        change(CardChangeKind::DueChanged, p.due.clone().unwrap_or_default(), card.due.clone().unwrap_or_default());
                    }
                    if p.priority != card.priority {
                        change(
                            CardChangeKind::PriorityChanged,
                            p.priority.map(|x| x.key().to_string()).unwrap_or_default(),
                            card.priority.map(|x| x.key().to_string()).unwrap_or_default(),
                        );
                    }
                    if (p.start.as_deref(), p.end.as_deref(), p.duration) != (card.start.as_deref(), card.end.as_deref(), card.duration) {
                        change(CardChangeKind::Scheduled, p.span_text(), card.span_text());
                    }
                }
            }
            let in_done = self.columns.iter().any(|c| c.id == card.column && c.done);
            let from_column = prev.map(|p| p.column.clone()).unwrap_or_default();
            let moved = prev.is_some_and(|p| p.column != card.column);
            if in_done && card.done.is_none() {
                card.done = Some(today_iso.clone());
                change(CardChangeKind::Done, before.column_name(&from_column), self_column_name(&self.columns, &card.column));
                if card.repeat != Repeat::None {
                    let column = self.columns.iter().find(|c| !c.done).or(self.columns.first()).map(|c| c.id.clone());
                    if let Some(column) = column {
                        spawned.push(card.next_repeat(item_id("k"), column, today));
                    }
                }
            } else if !in_done && card.done.is_some() {
                card.done = None;
                change(CardChangeKind::Reopened, before.column_name(&from_column), self_column_name(&self.columns, &card.column));
            } else if moved {
                change(CardChangeKind::Moved, before.column_name(&from_column), self_column_name(&self.columns, &card.column));
            }
        }
        for p in &before.cards {
            if self.cards.iter().any(|c| c.id == p.id) {
                continue;
            }
            let kind = if self.archive.iter().any(|c| c.id == p.id) { CardChangeKind::Archived } else { CardChangeKind::Deleted };
            changes.push(CardChange {
                card: p.id.clone(),
                title: p.title.clone(),
                kind,
                from: before.column_name(&p.column),
                to: String::new(),
                auto: false,
            });
        }
        for card in spawned {
            let column = self.column_name(&card.column);
            changes.push(CardChange {
                card: card.id.clone(),
                title: card.title.clone(),
                kind: CardChangeKind::Repeated,
                from: String::new(),
                to: format!("{column} · due {}", card.due.clone().unwrap_or_default()),
                auto: false,
            });
            let mut card = card;
            card.created = Some(today_iso.clone());
            card.entered = Some(now);
            // Сразу под остальными карточками своей колонки.
            let at = self.cards.iter().rposition(|c| c.column == card.column).map(|i| i + 1).unwrap_or(self.cards.len());
            self.cards.insert(at, card);
        }
        changes
    }

    /// Автоархив: закрытые `archive_after` и более дней назад уходят с доски.
    fn auto_archive(&mut self, today: i64, changes: &mut Vec<CardChange>) {
        let Some(days) = self.archive_after else { return };
        let mut i = 0;
        while i < self.cards.len() {
            let old = self.cards[i].done.as_deref().and_then(parse_days).is_some_and(|d| today - d >= days as i64);
            if old {
                let card = self.cards.remove(i);
                changes.push(CardChange {
                    card: card.id.clone(),
                    title: card.title.clone(),
                    kind: CardChangeKind::Archived,
                    from: self.column_name(&card.column),
                    to: String::new(),
                    auto: false,
                });
                self.archive.push(card);
            } else {
                i += 1;
            }
        }
    }

    /// Доводка без правки (при загрузке и по минутному такту): штампы,
    /// таймеры колонок, автоархив.
    pub fn sweep(&mut self, today: i64, now: i64) -> Vec<CardChange> {
        let before = self.clone();
        self.reconcile(&before, today, now)
    }

    /// Карточка с доски — в архив (закрывается штампом, если ещё открыта).
    pub fn archive_card(&mut self, id: &str, today: i64) -> bool {
        let Some(pos) = self.cards.iter().position(|c| c.id == id) else { return false };
        let mut card = self.cards.remove(pos);
        if card.done.is_none() {
            card.done = Some(days_to_iso(today));
        }
        self.archive.push(card);
        true
    }

    /// Карточка из архива — обратно на доску, в конец колонки: закрытая — в
    /// колонку «готово», незакрытая (ушла по сроку хранения колонки) — в
    /// свою прежнюю; нет такой — в первую. Штамп закрытия обновляется на
    /// `today`: иначе автоархив унёс бы её обратно той же доводкой; прежняя
    /// дата остаётся в журнале. Отсчёт таймеров колонки начнётся заново —
    /// доводка ставит `entered` вернувшейся карточке.
    pub fn unarchive_card(&mut self, id: &str, today: i64) -> bool {
        let Some(pos) = self.archive.iter().position(|c| c.id == id) else { return false };
        let mut card = self.archive.remove(pos);
        let own = self.columns.iter().find(|c| c.id == card.column);
        let column = if card.done.is_some() { self.done_column().or(own) } else { own.or_else(|| self.open_column()) }
            .or_else(|| self.columns.first())
            .map(|c| c.id.clone())
            .unwrap_or_default();
        card.column = column.clone();
        card.done = self.is_done_column(&column).then(|| days_to_iso(today));
        let at = self.cards.iter().rposition(|c| c.column == column).map(|i| i + 1).unwrap_or(self.cards.len());
        self.cards.insert(at, card);
        true
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Карточки колонки в порядке доски.
    pub fn cards_of(&self, column: &str) -> Vec<&KanbanCard> {
        self.cards.iter().filter(|c| c.column == column).collect()
    }

    pub fn card(&self, id: &str) -> Option<&KanbanCard> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn card_mut(&mut self, id: &str) -> Option<&mut KanbanCard> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    /// Место сразу после карточки: перед следующей в её колонке либо конец.
    pub fn spot_after(&self, card: &str) -> Option<DropSpot> {
        let c = self.card(card)?;
        let next = self
            .cards
            .iter()
            .skip_while(|x| x.id != card)
            .skip(1)
            .find(|x| x.column == c.column)
            .map(|x| x.id.clone());
        Some(DropSpot { column: c.column.clone(), before: next })
    }

    /// Место перед карточкой.
    pub fn spot_before(&self, card: &str) -> Option<DropSpot> {
        let c = self.card(card)?;
        Some(DropSpot::before(&c.column, card))
    }

    /// Место совпадает с текущим положением карточки (перенос ничего не
    /// изменит) — плейсхолдер там не показываем.
    pub fn is_own_spot(&self, card: &str, spot: &DropSpot) -> bool {
        self.spot_before(card).as_ref() == Some(spot) || self.spot_after(card).as_ref() == Some(spot)
    }

    /// Ширина колонки: своя либо общая.
    pub fn column_width(&self, column: &KanbanColumn) -> f32 {
        column
            .width
            .unwrap_or(self.style.column_width)
            .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
    }

    /// Перенос карточки в колонку `column` перед карточкой `before`
    /// (`None` — в конец колонки). Возвращает `false`, если карточки нет.
    pub fn move_card(&mut self, card: &str, column: &str, before: Option<&str>) -> bool {
        let Some(pos) = self.cards.iter().position(|c| c.id == card) else { return false };
        if before == Some(card) {
            return true;
        }
        let mut moved = self.cards.remove(pos);
        moved.column = column.to_string();
        let at = before
            .and_then(|b| self.cards.iter().position(|c| c.id == b && c.column == column))
            .unwrap_or_else(|| {
                // В конец колонки: сразу после её последней карточки, чтобы
                // порядок колонок в `cards` не перемешивался.
                self.cards
                    .iter()
                    .rposition(|c| c.column == column)
                    .map(|i| i + 1)
                    .unwrap_or(self.cards.len())
            });
        self.cards.insert(at, moved);
        true
    }
}

/// Цвета чипа (фон, текст) по цвету метки/приоритета. Фон — сам цвет,
/// текст — чёрный или белый по яркости фона (`Color::readable_on`): так чип
/// читается на любой карточке и в любой теме. Полупрозрачная подложка с
/// текстом того же цвета «сливалась» — разница яркости была почти нулевой.
pub fn chip_colors(hex: &str) -> (Color, Color) {
    let bg = Color::from_hex(hex);
    (bg, bg.readable_on())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Текст чипа контрастен фону на всей палитре: коэффициент контраста
    /// WCAG не ниже 4.5 (норма для мелкого текста).
    #[test]
    fn chip_text_contrasts_with_every_palette_color() {
        for hex in PALETTE.iter().copied().chain(["#4F8CFF", "#E8A33D", "#EE5E48", "#C03E3E"]) {
            let (bg, fg) = chip_colors(hex);
            let (a, b) = (bg.relative_luminance(), fg.relative_luminance());
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 4.5, "{hex}: contrast {ratio:.2}");
        }
        assert_eq!(chip_colors("#E8A33D").1, Color::BLACK);
        assert_eq!(chip_colors("#C03E3E").1, Color::WHITE);
    }

    /// Оценка длительности читается в минутах и печатается обратно.
    #[test]
    fn duration_parses_units_and_formats_back() {
        assert_eq!(parse_duration("1d"), Some(DAY_MIN));
        assert_eq!(parse_duration("2h"), Some(120));
        assert_eq!(parse_duration("90m"), Some(90));
        assert_eq!(parse_duration("1.5h"), Some(90));
        assert_eq!(parse_duration("1d 4h"), Some(DAY_MIN + 240));
        assert_eq!(parse_duration("30 мин"), Some(30), "русские единицы");
        assert_eq!(parse_duration("2 дня"), Some(2 * DAY_MIN));
        assert_eq!(parse_duration("3"), Some(180), "голое число — часы");
        assert_eq!(parse_duration("none"), None);
        assert_eq!(parse_duration("мусор"), None);
        assert_eq!(fmt_duration(DAY_MIN + 240), "1d 4h");
        assert_eq!(fmt_duration(90), "1h 30m");
        assert_eq!(fmt_duration(0), "");
    }

    /// Момент карточки: дата с временем и без.
    #[test]
    fn moment_roundtrip() {
        let day = parse_days("2026-09-09").unwrap();
        assert_eq!(Moment::parse("2026-09-09"), Some(Moment { day, min: None }));
        assert_eq!(Moment::parse("2026-09-09T10:30"), Some(Moment { day, min: Some(630) }));
        assert_eq!(Moment::parse("2026-09-09 10:30"), Some(Moment { day, min: Some(630) }));
        assert_eq!(Moment { day, min: Some(630) }.iso(), "2026-09-09T10:30");
        assert_eq!(Moment { day, min: None }.iso(), "2026-09-09");
        assert!(Moment::parse("мусор").is_none());
        assert!(Moment::parse("2026-09-09T25:00").is_none());
    }

    /// Полоса карточки: оценка в днях даёт многодневную полосу, оценка в
    /// часах — отрезок внутри дня; срок сам по себе полосой не становится.
    #[test]
    fn schedule_spans_days_and_hours() {
        let d = |iso: &str| parse_days(iso).unwrap();
        let mut c = KanbanCard::new("k".into(), "col".into());
        c.due = Some("2026-09-12".into());
        assert!(c.schedule().is_none(), "срок — точка, не полоса");

        // «Оценка 1 день» + «в календарь на завтра» — полоса в один день.
        c.duration = Some(DAY_MIN);
        c.schedule_at(d("2026-09-10"), None);
        assert_eq!(c.start.as_deref(), Some("2026-09-10"));
        assert_eq!(c.schedule(), Some(CardSpan { start_day: d("2026-09-10"), end_day: d("2026-09-10"), time: None }));
        assert_eq!(c.span_text(), "2026-09-10");

        // Три дня — полоса до 12-го включительно.
        c.duration = Some(3 * DAY_MIN);
        c.schedule_at(d("2026-09-10"), None);
        assert_eq!(c.schedule().unwrap().days(), 3);
        assert_eq!(c.span_text(), "2026-09-10 → 2026-09-12");

        // Два часа с 10:00 — отрезок внутри дня.
        c.duration = Some(120);
        c.schedule_at(d("2026-09-10"), Some(600));
        assert_eq!(c.schedule(), Some(CardSpan { start_day: d("2026-09-10"), end_day: d("2026-09-10"), time: Some((600, 720)) }));
        assert_eq!(c.span_text(), "2026-09-10 10:00–12:00");

        // Перенос сохраняет время и длительность.
        assert!(c.shift_schedule(2));
        assert_eq!(c.schedule().unwrap().start_day, d("2026-09-12"));
        assert_eq!(c.schedule().unwrap().time, Some((600, 720)));

        // Растягивание кромки задаёт дни и пересчитывает оценку.
        c.set_span_days(d("2026-09-12"), d("2026-09-15"));
        assert_eq!(c.duration, Some(4 * DAY_MIN));
        assert_eq!(c.schedule().unwrap().days(), 4);

        // Только конец — полоса отсчитывается назад.
        let mut back = KanbanCard::new("b".into(), "col".into());
        back.end = Some("2026-09-15".into());
        back.duration = Some(2 * DAY_MIN);
        assert_eq!(back.schedule(), Some(CardSpan { start_day: d("2026-09-14"), end_day: d("2026-09-15"), time: None }));

        assert!(c.unschedule());
        assert!(c.schedule().is_none() && c.due.is_some(), "план снят, срок остался");
    }

    /// Повтор двигает план вместе со сроком.
    #[test]
    fn repeat_shifts_the_plan_with_the_due_date() {
        let d = |iso: &str| parse_days(iso).unwrap();
        let mut c = KanbanCard::new("k".into(), "col".into());
        c.repeat = Repeat::Weekly;
        c.due = Some("2026-09-10".into());
        c.duration = Some(DAY_MIN);
        c.schedule_at(d("2026-09-10"), None);
        let next = c.next_repeat("k2".into(), "col".into(), d("2026-09-10"));
        assert_eq!(next.due.as_deref(), Some("2026-09-17"));
        assert_eq!(next.schedule().unwrap().start_day, d("2026-09-17"), "полоса едет за сроком");
    }

    fn doc() -> KanbanDoc {
        let mut d = KanbanDoc::template(["Todo", "Doing", "Done"]);
        for (i, col) in [0usize, 0, 1].into_iter().enumerate() {
            let mut c = KanbanCard::new(format!("k{i}"), d.columns[col].id.clone());
            c.title = format!("Задача {i}");
            d.cards.push(c);
        }
        d
    }

    #[test]
    fn template_roundtrip() {
        let mut d = doc();
        d.columns[1].width = Some(320.0);
        d.style.card_bg = "#243149".into();
        let json = d.serialize();
        let back = KanbanDoc::parse(&json).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.columns.len(), 3);
        assert_eq!(back.cards_of(&back.columns[0].id).len(), 2);
        assert_eq!(back.column_width(&back.columns[0]), DEFAULT_COLUMN_WIDTH);
        assert_eq!(back.column_width(&back.columns[1]), 320.0);
    }

    /// Доводка: переезд в «готово» ставит штамп и пишется одной записью
    /// `Done`, возврат снимает штамп, новые карточки получают `created`,
    /// повтор рождает следующую карточку, автоархив уносит старые.
    #[test]
    fn reconcile_stamps_repeats_and_archives() {
        let today = days_from_civil(2026, 9, 8);
        let now = today * 86_400;
        let mut d = doc();
        assert!(d.columns[2].done, "третья колонка шаблона — «готово»");
        let (todo, done_col) = (d.columns[0].id.clone(), d.columns[2].id.clone());
        // Первая доводка: штампы создания без «добавлений» в журнал (карточки уже были).
        let before = d.clone();
        let changes = d.reconcile(&before, today, now);
        assert!(changes.is_empty(), "{changes:?}");
        assert!(d.cards.iter().all(|c| c.created.as_deref() == Some("2026-09-08")));
        // Переезд в «готово» с недельным повтором.
        let before = d.clone();
        d.cards[0].repeat = Repeat::Weekly;
        d.cards[0].due = Some("2026-09-01".into());
        d.cards[0].md = "- [x] полить\n- [ ] удобрить".into();
        d.move_card("k0", &done_col, None);
        let changes = d.reconcile(&before, today, now);
        let kinds: Vec<CardChangeKind> = changes.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&CardChangeKind::Done) && !kinds.contains(&CardChangeKind::Moved), "{changes:?}");
        assert!(kinds.contains(&CardChangeKind::DueChanged) && kinds.contains(&CardChangeKind::Repeated), "{changes:?}");
        let done = changes.iter().find(|c| c.kind == CardChangeKind::Done).unwrap();
        assert_eq!((done.from.as_str(), done.to.as_str()), ("Todo", "Done"));
        assert_eq!(d.card("k0").unwrap().done.as_deref(), Some("2026-09-08"));
        // Следующая карточка: в первой открытой колонке, срок через неделю от сегодня (срок был просрочен), чек-лист снят.
        let next = d.cards.iter().find(|c| c.id != "k0" && c.title == "Задача 0").expect("повтор");
        assert_eq!(next.column, todo);
        assert_eq!(next.due.as_deref(), Some("2026-09-15"));
        assert_eq!(next.md, "- [ ] полить\n- [ ] удобрить");
        assert!(next.done.is_none() && next.created.as_deref() == Some("2026-09-08"));
        assert_eq!(next.repeat, Repeat::Weekly);
        // Возврат снимает штамп.
        let before = d.clone();
        d.move_card("k0", &todo, None);
        let changes = d.reconcile(&before, today + 1, now);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Reopened]);
        assert!(d.card("k0").unwrap().done.is_none());
        // Автоархив: закрытая 10 дней назад при archive_after=7 уходит в архив.
        d.move_card("k1", &done_col, None);
        let before = d.clone();
        d.reconcile(&before, today, now);
        d.card_mut("k1").unwrap().done = Some("2026-08-25".into());
        d.archive_after = Some(7);
        let changes = d.sweep(today, now);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Archived]);
        assert!(d.card("k1").is_none() && d.archive.iter().any(|c| c.id == "k1"));
        // Возврат из архива — в колонку «готово», запись Restored.
        let before = d.clone();
        assert!(d.unarchive_card("k1", today));
        let changes = d.reconcile(&before, today, now);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Restored]);
        assert_eq!(d.card("k1").unwrap().column, done_col);
        assert_eq!(d.card("k1").unwrap().done.as_deref(), Some("2026-09-08"), "штамп обновлён, автоархив не уносит");
        // Удаление и добавление.
        let before = d.clone();
        d.cards.retain(|c| c.id != "k2");
        let mut fresh = KanbanCard::new("k9".into(), todo.clone());
        fresh.title = "Новая".into();
        d.cards.push(fresh);
        let changes = d.reconcile(&before, today, now);
        let mut kinds: Vec<CardChangeKind> = changes.iter().map(|c| c.kind).collect();
        kinds.sort_by_key(|k| k.key());
        assert_eq!(kinds, [CardChangeKind::Added, CardChangeKind::Deleted]);
        assert_eq!(d.card("k9").unwrap().created.as_deref(), Some("2026-09-08"));
    }

    /// Таймеры колонок: отсчёт от входа в колонку (не от создания), ручной
    /// перенос сбрасывает его, эскалация ставит штамп на момент срабатывания
    /// — цепочка A → B → C проходится одной доводкой и закрывает карточку в
    /// «готово»; хранение уносит в архив, возврат — в свою колонку с новым
    /// отсчётом; при равных сроках — переезд, негодная цель гасит правило;
    /// взаимные правила не зацикливают доводку.
    #[test]
    fn column_timers_move_expire_and_chain() {
        let today = days_from_civil(2026, 9, 14);
        let h = HOUR_SECS;
        let t0 = today * 86_400;
        let mut d = doc();
        let (todo, doing, done_col) = (d.columns[0].id.clone(), d.columns[1].id.clone(), d.columns[2].id.clone());
        // Старые карточки без штампа получают «сейчас», а не дату создания.
        let before = d.clone();
        d.reconcile(&before, today, t0);
        assert!(d.cards.iter().all(|c| c.entered == Some(t0)), "{d:?}");
        // Todo: через 2 ч → Doing; Doing: через 3 ч → Done, хранение 10 ч.
        d.columns[0].move_after_hours = Some(2);
        d.columns[0].move_to = Some(doing.clone());
        d.columns[1].move_after_hours = Some(3);
        d.columns[1].move_to = Some(done_col.clone());
        d.columns[1].keep_hours = Some(10);
        // Ручной перенос сбрасывает отсчёт.
        let before = d.clone();
        d.move_card("k1", &doing, None);
        d.reconcile(&before, today, t0 + h);
        assert_eq!(d.card("k1").unwrap().entered, Some(t0 + h));
        assert_eq!(
            d.next_timer(d.card("k1").unwrap()),
            Some(CardTimer { at: t0 + 4 * h, action: TimerAction::Move(done_col.clone()) })
        );
        // 5 ч без правок: k0 прошёл Todo → Doing (в t0+2ч) → Done (в t0+5ч) за одну доводку.
        let changes = d.sweep(today, t0 + 5 * h);
        let k0 = d.card("k0").unwrap();
        assert_eq!(k0.column, done_col);
        assert_eq!(k0.entered, Some(t0 + 5 * h), "штамп — момент срабатывания");
        assert!(k0.done.is_some(), "переезд в «готово» по таймеру закрывает карточку");
        let k0_changes: Vec<(CardChangeKind, bool)> = changes.iter().filter(|c| c.card == "k0").map(|c| (c.kind, c.auto)).collect();
        assert_eq!(k0_changes, [(CardChangeKind::AutoMoved, true), (CardChangeKind::Done, true)]);
        assert!(d.cards_of(&doing).is_empty(), "k1 и k2 тоже уехали: {d:?}");

        // Хранение: Doing хранит 1 ч без переноса — новая карточка уходит в архив незакрытой.
        d.columns[1].move_to = None;
        d.columns[1].keep_hours = Some(1);
        let before = d.clone();
        let mut k3 = KanbanCard::new("k3".into(), doing.clone());
        k3.title = "Зависла".into();
        d.cards.push(k3);
        d.reconcile(&before, today, t0 + 6 * h);
        let changes = d.sweep(today, t0 + 7 * h);
        assert_eq!(changes.iter().map(|c| (c.kind, c.auto)).collect::<Vec<_>>(), [(CardChangeKind::Expired, true)]);
        assert!(d.card("k3").is_none() && d.archive.iter().any(|c| c.id == "k3" && c.done.is_none()), "{d:?}");
        // Возврат — в свою колонку, отсчёт заново.
        let before = d.clone();
        assert!(d.unarchive_card("k3", today));
        let changes = d.reconcile(&before, today, t0 + 8 * h);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Restored]);
        assert_eq!(d.card("k3").unwrap().column, doing);
        assert_eq!(d.card("k3").unwrap().entered, Some(t0 + 8 * h));

        // Равные сроки — переезд; цель — сама колонка или пропавшая — правило гасит.
        d.columns[1].move_after_hours = Some(1);
        d.columns[1].move_to = Some(todo.clone());
        let k3 = d.card("k3").unwrap().clone();
        assert_eq!(d.next_timer(&k3).unwrap().action, TimerAction::Move(todo.clone()));
        d.columns[1].move_to = Some(doing.clone());
        assert_eq!(d.next_timer(&k3).unwrap().action, TimerAction::Expire);
        d.columns[1].move_to = Some("нет".into());
        d.columns[1].keep_hours = None;
        assert!(d.next_timer(&k3).is_none());

        // Взаимные правила A ⇄ B: не больше TIMER_HOPS шагов за доводку.
        let mut ping = doc();
        let (a, b) = (ping.columns[0].id.clone(), ping.columns[1].id.clone());
        ping.columns[0].move_after_hours = Some(1);
        ping.columns[0].move_to = Some(b);
        ping.columns[1].move_after_hours = Some(1);
        ping.columns[1].move_to = Some(a);
        ping.sweep(today, t0);
        let changes = ping.sweep(today, t0 + 1000 * h);
        assert!(changes.len() <= TIMER_HOPS * ping.cards.len(), "{}", changes.len());

        // Часы из строки.
        assert_eq!(parse_hours("72"), Ok(Some(72)));
        assert_eq!(parse_hours("3d"), Ok(Some(72)));
        assert_eq!(parse_hours("90m"), Ok(Some(2)));
        assert_eq!(parse_hours("∞"), Ok(None));
        assert_eq!(parse_hours(" "), Ok(None));
        assert!(parse_hours("мусор").is_err());
    }

    #[test]
    fn done_column_is_detected_by_name_and_repeat_periods() {
        let json = r#"{"columns":[{"id":"a","name":"Бэклог"},{"id":"b","name":"В работе"},{"id":"c","name":"Готово"}],"cards":[]}"#;
        let d = KanbanDoc::parse(json).unwrap();
        assert_eq!(d.done_column().map(|c| c.id.as_str()), Some("c"));
        assert_eq!(d.open_column().map(|c| c.id.as_str()), Some("a"));
        // Явный флаг не перебивается названием.
        let json = r#"{"columns":[{"id":"a","name":"Done"},{"id":"b","name":"Archive","done":true}],"cards":[]}"#;
        let d = KanbanDoc::parse(json).unwrap();
        assert_eq!(d.done_column().map(|c| c.id.as_str()), Some("b"));
        // Периоды повтора.
        let d0 = days_from_civil(2026, 1, 31);
        assert_eq!(days_to_iso(next_period(d0, Repeat::Daily)), "2026-02-01");
        assert_eq!(days_to_iso(next_period(d0, Repeat::Weekly)), "2026-02-07");
        assert_eq!(days_to_iso(next_period(d0, Repeat::Monthly)), "2026-02-28");
        assert_eq!(days_to_iso(next_period(d0, Repeat::Yearly)), "2027-01-31");
        assert_eq!(uncheck("1. [X] а\n  * [x] б\n- [ ] в\nтекст"), "1. [ ] а\n  * [ ] б\n- [ ] в\nтекст");
        // Вложения: картинка против файла.
        let img = CardFile::new("asset:abc.PNG", "фото.png");
        assert!(img.is_image() && img.label() == "фото.png");
        let pdf = CardFile::new("asset:abc.pdf", "");
        assert!(!pdf.is_image() && pdf.label() == "abc.pdf");
    }

    #[test]
    fn first_wave_card_still_reads() {
        let json = r#"{"columns":[{"id":"c","name":"A"}],"cards":[{"id":"k","column":"c","title":"старое"}]}"#;
        let d = KanbanDoc::parse(json).unwrap();
        assert_eq!(d.cards[0].title, "старое");
        assert!(d.cards[0].md.is_empty());
        assert!(!d.cards[0].is_empty());
        assert_eq!(d.style, KanbanStyle::default());
    }

    #[test]
    fn move_card_between_and_within_columns() {
        let mut d = doc();
        let (todo, doing) = (d.columns[0].id.clone(), d.columns[1].id.clone());
        // В конец другой колонки.
        assert!(d.move_card("k0", &doing, None));
        let ids: Vec<&str> = d.cards_of(&doing).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["k2", "k0"]);
        // Перед карточкой внутри колонки.
        assert!(d.move_card("k0", &doing, Some("k2")));
        let ids: Vec<&str> = d.cards_of(&doing).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["k0", "k2"]);
        assert_eq!(d.cards_of(&todo).len(), 1);
        assert!(!d.move_card("нет", &todo, None));
    }

    #[test]
    fn spots_around_cards() {
        let d = doc();
        let todo = d.columns[0].id.clone();
        assert_eq!(d.spot_before("k0"), Some(DropSpot::before(&todo, "k0")));
        assert_eq!(d.spot_after("k0"), Some(DropSpot::before(&todo, "k1")));
        assert_eq!(d.spot_after("k1"), Some(DropSpot::end(&todo)));
        assert!(d.is_own_spot("k0", &DropSpot::before(&todo, "k1")));
        assert!(!d.is_own_spot("k1", &DropSpot::before(&todo, "k0")));
        assert_eq!(d.spot_after("нет"), None);
    }

    #[test]
    fn checklist_tags_and_preview() {
        assert_eq!(checklist_progress("- [x] a\n- [ ] b\n* [X] c\n1. [ ] d\n- обычный"), (2, 4));
        assert_eq!(checklist_progress("текст"), (0, 0));
        assert_eq!(parse_tags(" UI, #дизайн ;ui,, "), vec!["UI", "дизайн"]);
        assert_eq!(tag_color("ui"), tag_color("UI"));
        assert_ne!(tag_color("ui"), PALETTE[0]);
        let p = preview_text("## Заголовок\n\n- [ ] пункт **жирный**\n> цитата [[Стр|Ссылка]] и [текст](http://x)\n```\nкод\n```\n![[img.png]]");
        assert_eq!(p, "Заголовок\n☐ пункт жирный\nцитата Ссылка и текст\nкод");
        let mut c = KanbanCard::new("k".into(), "c".into());
        assert!(c.is_empty());
        c.priority = Some(Priority::High);
        assert!(!c.is_empty());
        assert_eq!(Priority::parse("urgent"), Some(Priority::Urgent));
        assert_eq!(Priority::parse("x"), None);
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"priority\":\"high\""), "{json}");
        assert!(!json.contains("tags"), "пустые поля не пишутся: {json}");
    }

    #[test]
    fn palette_cycles() {
        assert_eq!(next_color(""), PALETTE[0]);
        assert_eq!(next_color(PALETTE[0]), PALETTE[1]);
        assert_eq!(next_color(PALETTE[PALETTE.len() - 1]), PALETTE[0]);
    }
}
