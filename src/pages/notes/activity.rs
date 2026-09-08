//! Журнал изменений проекта: что, когда и кем сделано.
//!
//! Записи рождаются не в местах вызова, а в единых точках правки — доводке
//! доски ([`super::kanban::model::KanbanDoc::reconcile`] внутри
//! `KanbanHandle::edit`), сверке хранилища событий (`CalendarStoreHandle::
//! edit`) и операциях дерева страниц (`NotesCtx`), поэтому мышиный перенос
//! карточки и вызов агента попадают в журнал одинаково. Кто правил —
//! [`Actor`]: агент выставляет `Agent` на время своего действия
//! ([`agent_scope`]), всё остальное — пользователь.
//!
//! Хранение — помесячные файлы `notes/log/<yyyy-mm>.jsonl` в бандле (запись
//! на строку): автосейв переписывает только «грязный» месяц, а не растущий
//! журнал целиком. Загружаются все месяцы разом при первом обращении — у
//! личной записной книжки это тысячи строк, не миллионы.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use syngui::prelude::*;

use super::gantt::calendar::{days_to_iso, parse_days};
use super::project;

/// Кто сделал правку.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    User,
    Agent,
}

impl Actor {
    pub fn key(self) -> &'static str {
        match self {
            Actor::User => "user",
            Actor::Agent => "agent",
        }
    }
}

thread_local! {
    static ACTOR: Cell<Actor> = const { Cell::new(Actor::User) };
}

/// Текущий актор правок на этом потоке.
pub fn actor() -> Actor {
    ACTOR.with(|a| a.get())
}

/// На время жизни — правки идут от имени агента.
pub struct AgentScope(Actor);

pub fn agent_scope() -> AgentScope {
    let prev = ACTOR.with(|a| a.replace(Actor::Agent));
    AgentScope(prev)
}

impl Drop for AgentScope {
    fn drop(&mut self) {
        ACTOR.with(|a| a.set(self.0));
    }
}

/// Одна запись журнала.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Локальное время `yyyy-mm-ddTHH:MM`.
    pub ts: String,
    /// `user` | `agent`.
    pub actor: String,
    /// `card` | `event` | `page`.
    pub kind: String,
    /// `add | delete | move | done | reopen | due | priority | archive |
    /// restore | repeat` у карточек; `add | delete | done | reopen | move`
    /// у событий; `create | rename | delete` у страниц.
    pub action: String,
    /// Контейнер: `kanban:<id>`, `calendar`, либо id страницы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object: String,
    /// Элемент: id карточки / события / страницы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Откуда / что было.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// Куда / что стало.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub to: String,
}

impl LogEntry {
    /// Запись «сейчас» от текущего актора.
    pub fn now(kind: &str, action: &str) -> Self {
        Self {
            ts: now_stamp(),
            actor: actor().key().to_string(),
            kind: kind.to_string(),
            action: action.to_string(),
            object: String::new(),
            item: String::new(),
            title: String::new(),
            from: String::new(),
            to: String::new(),
        }
    }

    pub fn object(mut self, object: impl Into<String>) -> Self {
        self.object = object.into();
        self
    }

    pub fn item(mut self, item: impl Into<String>) -> Self {
        self.item = item.into();
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn from_to(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.from = from.into();
        self.to = to.into();
        self
    }

    /// День записи (дни от эпохи).
    pub fn day(&self) -> Option<i64> {
        parse_days(self.ts.get(..10)?)
    }

    /// `yyyy-mm` — файл месяца.
    pub fn month(&self) -> String {
        self.ts.get(..7).unwrap_or_default().to_string()
    }
}

/// Локальное «сейчас» как `yyyy-mm-ddTHH:MM`.
pub fn now_stamp() -> String {
    let (day, min) = crate::agent::time::local_now();
    format!("{}T{:02}:{:02}", days_to_iso(day), min / 60, min % 60)
}

/// Путь файла месяца в бандле.
pub fn month_path(month: &str) -> String {
    format!("{}/{month}.jsonl", project::LOG_DIR)
}

#[derive(Default)]
pub struct ActivityLog {
    pub entries: Vec<LogEntry>,
    /// Месяцы, чьи файлы надо переписать.
    dirty: BTreeSet<String>,
}

impl ActivityLog {
    /// Все месяцы из бандла; битые строки пропускаются.
    pub fn load(path: &Path) -> Self {
        let mut entries = Vec::new();
        for name in project::list_log_files(path) {
            if let Some(text) = project::read_text(path, &name) {
                entries.extend(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str::<LogEntry>(l).ok()));
            }
        }
        entries.sort_by(|a, b| a.ts.cmp(&b.ts));
        Self { entries, dirty: BTreeSet::new() }
    }

    pub fn push(&mut self, entry: LogEntry) {
        self.dirty.insert(entry.month());
        self.entries.push(entry);
    }

    /// Файлы грязных месяцев на запись (и сброс флага).
    pub fn take_dirty(&mut self) -> Vec<(String, Vec<u8>)> {
        let months = std::mem::take(&mut self.dirty);
        months
            .into_iter()
            .map(|m| {
                let mut text = String::new();
                for e in self.entries.iter().filter(|e| e.month() == m) {
                    if let Ok(line) = serde_json::to_string(e) {
                        text.push_str(&line);
                        text.push('\n');
                    }
                }
                (month_path(&m), text.into_bytes())
            })
            .collect()
    }

    /// Записи в диапазоне дней (включительно) с фильтрами; свежие первыми.
    pub fn query(&self, q: &LogQuery) -> Vec<&LogEntry> {
        let mut out: Vec<&LogEntry> = self
            .entries
            .iter()
            .filter(|e| {
                let day = e.day().unwrap_or(i64::MIN);
                q.since.is_none_or(|s| day >= s)
                    && q.until.is_none_or(|u| day <= u)
                    && q.kind.as_deref().is_none_or(|k| e.kind == k)
                    && q.actor.as_deref().is_none_or(|a| e.actor == a)
                    && q.object.as_deref().is_none_or(|o| e.object == o)
                    && q.item.as_deref().is_none_or(|i| e.item == i)
                    && q.action.as_deref().is_none_or(|a| e.action == a)
            })
            .collect();
        out.reverse();
        if let Some(limit) = q.limit {
            out.truncate(limit);
        }
        out
    }
}

/// Фильтр выборки журнала.
#[derive(Clone, Debug, Default)]
pub struct LogQuery {
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub kind: Option<String>,
    pub actor: Option<String>,
    pub object: Option<String>,
    pub item: Option<String>,
    pub action: Option<String>,
    pub limit: Option<usize>,
}

/// Ручка журнала: Mutex + ревизия для автосейва (как у хранилища событий).
#[derive(Clone)]
pub struct ActivityLogHandle {
    log: Arc<Mutex<ActivityLog>>,
    pub revision: RwSignal<u64>,
}

impl PartialEq for ActivityLogHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.log, &other.log)
    }
}

impl ActivityLogHandle {
    pub fn new(log: ActivityLog) -> Self {
        Self { log: Arc::new(Mutex::new(log)), revision: use_signal(0) }
    }

    pub fn lock(&self) -> MutexGuard<'_, ActivityLog> {
        self.log.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn record(&self, entry: LogEntry) {
        self.lock().push(entry);
        self.revision.set(self.revision.get_untracked() + 1);
    }

    pub fn record_all(&self, entries: impl IntoIterator<Item = LogEntry>) {
        let mut n = 0;
        {
            let mut log = self.lock();
            for e in entries {
                log.push(e);
                n += 1;
            }
        }
        if n > 0 {
            self.revision.set(self.revision.get_untracked() + 1);
        }
    }

    /// Файлы грязных месяцев (автосейв).
    pub fn take_dirty(&self) -> Vec<(String, Vec<u8>)> {
        self.lock().take_dirty()
    }

    pub fn query(&self, q: &LogQuery) -> Vec<LogEntry> {
        self.lock().query(q).into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_scope_switches_actor_and_restores() {
        assert_eq!(actor(), Actor::User);
        {
            let _s = agent_scope();
            assert_eq!(actor(), Actor::Agent);
            assert_eq!(LogEntry::now("card", "add").actor, "agent");
        }
        assert_eq!(actor(), Actor::User);
    }

    #[test]
    fn months_are_dirty_separately_and_query_filters() {
        let mut log = ActivityLog::default();
        let mut a = LogEntry::now("card", "add").object("kanban:b").item("k1").title("A");
        a.ts = "2026-08-30T10:00".into();
        let mut b = LogEntry::now("card", "done").object("kanban:b").item("k1").title("A");
        b.ts = "2026-09-02T11:30".into();
        let mut c = LogEntry::now("page", "create").item("p1").title("Стр");
        c.ts = "2026-09-03T09:00".into();
        log.push(a);
        log.push(b.clone());
        log.push(c);
        let files = log.take_dirty();
        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["notes/log/2026-08.jsonl", "notes/log/2026-09.jsonl"]);
        let sep: Vec<LogEntry> =
            String::from_utf8(files[1].1.clone()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(sep.len(), 2);
        assert_eq!(sep[0], b);
        assert!(log.take_dirty().is_empty(), "после записи грязных месяцев нет");
        // Выборка: с 1 сентября, только карточки — одна запись, свежая первой.
        let q = LogQuery { since: parse_days("2026-09-01"), kind: Some("card".into()), ..Default::default() };
        let hits = log.query(&q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, "done");
        let all = log.query(&LogQuery::default());
        assert_eq!(all.iter().map(|e| e.ts.as_str()).collect::<Vec<_>>(), ["2026-09-03T09:00", "2026-09-02T11:30", "2026-08-30T10:00"]);
        let limited = log.query(&LogQuery { limit: Some(1), ..Default::default() });
        assert_eq!(limited.len(), 1);
    }
}
