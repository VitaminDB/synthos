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

use serde::{Deserialize, Serialize};

pub use super::super::calendar::model::Repeat;
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
    /// День появления на доске, ISO `yyyy-mm-dd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// День закрытия (переезда в колонку «готово»); снимается при возврате.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<String>,
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
            created: None,
            done: None,
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
        let base = self.due.as_deref().and_then(parse_days).map(|d| d.max(today)).unwrap_or(today);
        let mut next = self.clone();
        next.id = id;
        next.column = column;
        next.due = Some(days_to_iso(next_period(base, self.repeat)));
        next.created = None;
        next.done = None;
        next.md = uncheck(&self.md);
        next
    }

    /// `(сделано, всего)` по пунктам чек-листа содержимого; `None` — их нет.
    pub fn checklist(&self) -> Option<(usize, usize)> {
        let (done, total) = checklist_progress(&self.md);
        (total > 0).then_some((done, total))
    }
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
    Archived,
    Restored,
    /// Родилась следующая карточка повтора (`card` — новая).
    Repeated,
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
            CardChangeKind::Archived => "archive",
            CardChangeKind::Restored => "restore",
            CardChangeKind::Repeated => "repeat",
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
                .map(|(i, (name, color))| KanbanColumn {
                    id: item_id("c"),
                    name: name.to_string(),
                    color: color.to_string(),
                    width: None,
                    done: i == 2,
                })
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

    /// Доводка после правки против снимка `before`: штампы `created`/`done`,
    /// следующие карточки повторов, автоархив; возвращает изменения для
    /// журнала. Перенос в колонку «готово» записывается одним `Done` (с
    /// исходной колонкой), а не `Moved` + `Done`.
    pub fn reconcile(&mut self, before: &KanbanDoc, today: i64) -> Vec<CardChange> {
        let today_iso = days_to_iso(today);
        let mut changes = Vec::new();
        let mut spawned: Vec<KanbanCard> = Vec::new();
        for card in &mut self.cards {
            if card.created.is_none() {
                card.created = Some(today_iso.clone());
            }
            let prev = before.cards.iter().find(|c| c.id == card.id);
            let (cid, ctitle) = (card.id.clone(), card.title.clone());
            let mut change = |kind: CardChangeKind, from: String, to: String| {
                changes.push(CardChange { card: cid.clone(), title: ctitle.clone(), kind, from, to });
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
            changes.push(CardChange { card: p.id.clone(), title: p.title.clone(), kind, from: before.column_name(&p.column), to: String::new() });
        }
        for card in spawned {
            let column = self.column_name(&card.column);
            changes.push(CardChange {
                card: card.id.clone(),
                title: card.title.clone(),
                kind: CardChangeKind::Repeated,
                from: String::new(),
                to: format!("{column} · due {}", card.due.clone().unwrap_or_default()),
            });
            let mut card = card;
            card.created = Some(today_iso.clone());
            // Сразу под остальными карточками своей колонки.
            let at = self.cards.iter().rposition(|c| c.column == card.column).map(|i| i + 1).unwrap_or(self.cards.len());
            self.cards.insert(at, card);
        }
        if let Some(days) = self.archive_after {
            let mut i = 0;
            while i < self.cards.len() {
                let old = self.cards[i]
                    .done
                    .as_deref()
                    .and_then(parse_days)
                    .is_some_and(|d| today - d >= days as i64);
                if old {
                    let card = self.cards.remove(i);
                    changes.push(CardChange {
                        card: card.id.clone(),
                        title: card.title.clone(),
                        kind: CardChangeKind::Archived,
                        from: self.column_name(&card.column),
                        to: String::new(),
                    });
                    self.archive.push(card);
                } else {
                    i += 1;
                }
            }
        }
        changes
    }

    /// Доводка без правки (при загрузке): штампы и автоархив.
    pub fn sweep(&mut self, today: i64) -> Vec<CardChange> {
        let before = self.clone();
        self.reconcile(&before, today)
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

    /// Карточка из архива — обратно на доску, в колонку «готово» (иначе в
    /// свою прежнюю либо первую), в конец. Штамп закрытия обновляется на
    /// `today`: иначе автоархив унёс бы её обратно той же доводкой; прежняя
    /// дата остаётся в журнале.
    pub fn unarchive_card(&mut self, id: &str, today: i64) -> bool {
        let Some(pos) = self.archive.iter().position(|c| c.id == id) else { return false };
        let mut card = self.archive.remove(pos);
        let column = self
            .done_column()
            .or_else(|| self.columns.iter().find(|c| c.id == card.column))
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut d = doc();
        assert!(d.columns[2].done, "третья колонка шаблона — «готово»");
        let (todo, done_col) = (d.columns[0].id.clone(), d.columns[2].id.clone());
        // Первая доводка: штампы создания без «добавлений» в журнал (карточки уже были).
        let before = d.clone();
        let changes = d.reconcile(&before, today);
        assert!(changes.is_empty(), "{changes:?}");
        assert!(d.cards.iter().all(|c| c.created.as_deref() == Some("2026-09-08")));
        // Переезд в «готово» с недельным повтором.
        let before = d.clone();
        d.cards[0].repeat = Repeat::Weekly;
        d.cards[0].due = Some("2026-09-01".into());
        d.cards[0].md = "- [x] полить\n- [ ] удобрить".into();
        d.move_card("k0", &done_col, None);
        let changes = d.reconcile(&before, today);
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
        let changes = d.reconcile(&before, today + 1);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Reopened]);
        assert!(d.card("k0").unwrap().done.is_none());
        // Автоархив: закрытая 10 дней назад при archive_after=7 уходит в архив.
        d.move_card("k1", &done_col, None);
        let before = d.clone();
        d.reconcile(&before, today);
        d.card_mut("k1").unwrap().done = Some("2026-08-25".into());
        d.archive_after = Some(7);
        let changes = d.sweep(today);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Archived]);
        assert!(d.card("k1").is_none() && d.archive.iter().any(|c| c.id == "k1"));
        // Возврат из архива — в колонку «готово», запись Restored.
        let before = d.clone();
        assert!(d.unarchive_card("k1", today));
        let changes = d.reconcile(&before, today);
        assert_eq!(changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [CardChangeKind::Restored]);
        assert_eq!(d.card("k1").unwrap().column, done_col);
        assert_eq!(d.card("k1").unwrap().done.as_deref(), Some("2026-09-08"), "штамп обновлён, автоархив не уносит");
        // Удаление и добавление.
        let before = d.clone();
        d.cards.retain(|c| c.id != "k2");
        let mut fresh = KanbanCard::new("k9".into(), todo.clone());
        fresh.title = "Новая".into();
        d.cards.push(fresh);
        let changes = d.reconcile(&before, today);
        let mut kinds: Vec<CardChangeKind> = changes.iter().map(|c| c.kind).collect();
        kinds.sort_by_key(|k| k.key());
        assert_eq!(kinds, [CardChangeKind::Added, CardChangeKind::Deleted]);
        assert_eq!(d.card("k9").unwrap().created.as_deref(), Some("2026-09-08"));
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
