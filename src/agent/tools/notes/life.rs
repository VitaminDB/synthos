//! Действия инструмента `notes` для «ведения жизни»: журнал изменений
//! (`log`), сводка дня (`agenda`), задачи по всем доскам с фильтрами
//! (`tasks`) и страница дня (`journal`).
//!
//! Смысл — одна команда вместо обхода проекта: локальная модель платит за
//! каждый вызов полным префиллом, а «что мне делать сегодня» раньше
//! собиралось чтением всех досок и календаря по очереди.

use super::*;
use crate::pages::notes::activity::{LogEntry, LogQuery};
use crate::pages::notes::calendar::model::Repeat;
use crate::pages::notes::gantt::calendar::weekday_of;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Карточка с адресом: доска, страница, колонка.
pub(super) struct TaskRow {
    pub board: String,
    pub page: String,
    pub page_title: String,
    pub column: String,
    pub column_done: bool,
    pub archived: bool,
    pub card: KanbanCard,
}

/// Все карточки проекта (с архивом), в порядке дерева и досок.
pub(super) fn all_cards(ctx: NotesCtx) -> Vec<TaskRow> {
    let mut out = Vec::new();
    for (oid, pid) in all_objects(ctx, "kanban") {
        let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &oid) else { continue };
        let doc = handle.lock().clone();
        let page_title = ctx.title_of(&pid);
        for c in &doc.cards {
            out.push(TaskRow {
                board: oid.clone(),
                page: pid.clone(),
                page_title: page_title.clone(),
                column: doc.column_name(&c.column),
                column_done: doc.is_done_column(&c.column),
                archived: false,
                card: c.clone(),
            });
        }
        for c in &doc.archive {
            out.push(TaskRow {
                board: oid.clone(),
                page: pid.clone(),
                page_title: page_title.clone(),
                column: doc.column_name(&c.column),
                column_done: true,
                archived: true,
                card: c.clone(),
            });
        }
    }
    out
}

fn task_line(r: &TaskRow) -> String {
    format!(
        "{} · column \"{}\"{} · kanban:{} · page \"{}\" ({})",
        card_line(&r.card),
        r.column,
        if r.archived { " · archived" } else { "" },
        r.board,
        r.page_title,
        r.page
    )
}

/// `since`: today | yesterday | week | month | Nd | yyyy-mm-dd; без поля —
/// `default_days` назад.
fn since_arg(v: &Json, key: &str, default_days: i64) -> Result<i64, String> {
    let today = today_days();
    let Some(s) = str_field(v, key) else { return Ok(today - default_days) };
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "today" => return Ok(today),
        "yesterday" => return Ok(today - 1),
        "week" => return Ok(today - 7),
        "month" => return Ok(today - 30),
        "all" | "ever" => return Ok(i64::MIN / 2),
        _ => {}
    }
    if let Some(n) = lower.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        return Ok(today - n);
    }
    parse_days(s).ok_or_else(|| format!("bad \"{key}\" \"{s}\" (yyyy-mm-dd | today | yesterday | week | month | 14d)"))
}

/// Человеческая строка записи журнала.
fn log_line(ctx: NotesCtx, e: &LogEntry, boards: &mut Vec<(String, String)>) -> String {
    let when = e.ts.replacen('T', " ", 1);
    let what = match (e.kind.as_str(), e.action.as_str()) {
        ("card", "add") => format!("card \"{}\" added to \"{}\"", e.title, e.to),
        ("card", "delete") => format!("card \"{}\" deleted from \"{}\"", e.title, e.from),
        ("card", "move") => format!("card \"{}\" moved \"{}\" → \"{}\"", e.title, e.from, e.to),
        ("card", "done") => format!("card \"{}\" DONE (\"{}\" → \"{}\")", e.title, e.from, e.to),
        ("card", "reopen") => format!("card \"{}\" reopened (\"{}\" → \"{}\")", e.title, e.from, e.to),
        ("card", "due") => format!(
            "card \"{}\" due {} → {}",
            e.title,
            if e.from.is_empty() { "none" } else { &e.from },
            if e.to.is_empty() { "none" } else { &e.to }
        ),
        ("card", "priority") => format!(
            "card \"{}\" priority {} → {}",
            e.title,
            if e.from.is_empty() { "none" } else { &e.from },
            if e.to.is_empty() { "none" } else { &e.to }
        ),
        ("card", "archive") => format!("card \"{}\" archived (from \"{}\")", e.title, e.from),
        ("card", "restore") => format!("card \"{}\" restored to \"{}\"", e.title, e.to),
        ("card", "repeat") => format!("card \"{}\" next repeat created: {}", e.title, e.to),
        ("event", "add") => format!("event \"{}\" added on {}", e.title, e.to),
        ("event", "delete") => format!("event \"{}\" deleted (was {})", e.title, e.from),
        ("event", "done") => format!("event \"{}\" done ({})", e.title, e.to),
        ("event", "reopen") => format!("event \"{}\" reopened ({})", e.title, e.to),
        ("event", "move") => format!("event \"{}\" moved {} → {}", e.title, e.from, e.to),
        ("page", "create") => format!("page \"{}\" created", e.title),
        ("page", "rename") => format!("page renamed \"{}\" → \"{}\"", e.from, e.to),
        ("page", "delete") => format!("page \"{}\" deleted{}", e.title, if e.from.is_empty() { String::new() } else { format!(" with {}", e.from) }),
        (k, a) => format!("{k} \"{}\" {a} {} {}", e.title, e.from, e.to),
    };
    let mut place = String::new();
    if let Some(oid) = e.object.strip_prefix("kanban:") {
        let title = match boards.iter().find(|(o, _)| o == oid) {
            Some((_, t)) => t.clone(),
            None => {
                let t = object_page(ctx, "kanban", oid).map(|p| ctx.title_of(&p)).unwrap_or_default();
                boards.push((oid.to_string(), t.clone()));
                t
            }
        };
        place = format!(" · kanban:{oid} \"{title}\"");
    } else if e.kind == "page" {
        place = format!(" · page {}", e.item);
    } else if e.kind == "card" {
        place = format!(" · card {}", e.item);
    }
    format!("{when} · {} · {what}{place}", e.actor)
}

pub(super) fn log_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let since = since_arg(v, "since", 7)?;
    let until = day_arg(v, "until")?;
    let limit = usize_field(v, "limit").unwrap_or(60).clamp(1, 500);
    let mut q = LogQuery { since: Some(since), until, limit: Some(limit), ..Default::default() };
    if let Some(k) = str_field(v, "kind") {
        let k = k.to_ascii_lowercase();
        if !["card", "event", "page"].contains(&k.as_str()) {
            return Err(format!("bad \"kind\" \"{k}\" (card | event | page)"));
        }
        q.kind = Some(k);
    }
    if let Some(a) = str_field(v, "actor") {
        let a = a.to_ascii_lowercase();
        if !["user", "agent"].contains(&a.as_str()) {
            return Err(format!("bad \"actor\" \"{a}\" (user | agent)"));
        }
        q.actor = Some(a);
    }
    if let Some(a) = str_field(v, "op") {
        q.action = Some(a.to_ascii_lowercase());
    }
    if str_field(v, "board").is_some() {
        let (id, _) = kanban_handle(ctx, v)?;
        q.object = Some(format!("kanban:{id}"));
    }
    if let Some(c) = str_field(v, "card") {
        // Карточка по id — как есть; по названию — на доске (нужен board/page).
        let item = match kanban_handle(ctx, v) {
            Ok((_, handle)) => resolve_card(&handle, c).map(|k| k.id).unwrap_or_else(|_| c.to_string()),
            Err(_) => c.to_string(),
        };
        q.item = Some(item);
    }
    let entries = ctx.activity().query(&q);
    let total = ctx.activity().lock().query(&LogQuery { limit: None, ..q.clone() }).len();
    let mut out = format!(
        "--- Log since {}{} · {} entr{} (newest first{}) ---\n",
        if since < -1_000_000 { "the beginning".to_string() } else { days_to_iso(since) },
        until.map(|u| format!(" until {}", days_to_iso(u))).unwrap_or_default(),
        total,
        if total == 1 { "y" } else { "ies" },
        if total > entries.len() { format!(", showing {}", entries.len()) } else { String::new() }
    );
    if entries.is_empty() {
        out.push_str("  (nothing changed in this range)\n");
    }
    let mut boards = Vec::new();
    for e in &entries {
        out.push_str("  ");
        out.push_str(&log_line(ctx, e, &mut boards));
        out.push('\n');
    }
    out.push_str("---\nFilters: since (today | yesterday | week | month | 14d | yyyy-mm-dd), until, kind (card | event | page), actor (user | agent), op (done | move | add | …), board, card, limit.\n");
    Ok(out)
}

/// Фильтр по сроку для `tasks`.
enum DueFilter {
    Any,
    None,
    Overdue,
    On(i64),
    Within(i64),
    Range(i64, i64),
}

fn due_filter(s: &str) -> Result<DueFilter, String> {
    let today = today_days();
    let lower = s.trim().to_ascii_lowercase();
    Ok(match lower.as_str() {
        "any" | "all" | "" => DueFilter::Any,
        "none" | "no" | "without" => DueFilter::None,
        "overdue" | "late" => DueFilter::Overdue,
        "today" => DueFilter::On(today),
        "tomorrow" => DueFilter::On(today + 1),
        "week" => DueFilter::Within(7),
        "month" => DueFilter::Within(30),
        _ => {
            if let Some(n) = lower.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
                DueFilter::Within(n)
            } else if let Some((a, b)) = lower.split_once("..") {
                let a = parse_days(a).ok_or_else(|| format!("bad due range start \"{a}\""))?;
                let b = parse_days(b).ok_or_else(|| format!("bad due range end \"{b}\""))?;
                DueFilter::Range(a.min(b), a.max(b))
            } else {
                DueFilter::On(parse_days(&lower).ok_or_else(|| {
                    format!("bad \"due\" \"{s}\" (overdue | today | tomorrow | week | month | 14d | none | any | yyyy-mm-dd | a..b)")
                })?)
            }
        }
    })
}

impl DueFilter {
    fn keep(&self, due: Option<i64>, today: i64) -> bool {
        match self {
            DueFilter::Any => true,
            DueFilter::None => due.is_none(),
            DueFilter::Overdue => due.is_some_and(|d| d < today),
            DueFilter::On(d) => due == Some(*d),
            DueFilter::Within(n) => due.is_some_and(|d| d >= today && d <= today + n),
            DueFilter::Range(a, b) => due.is_some_and(|d| d >= *a && d <= *b),
        }
    }
}

pub(super) fn tasks_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let today = today_days();
    let due = due_filter(str_field(v, "due").unwrap_or("any"))?;
    // done: true | false (по умолчанию — только открытые) | any.
    let done: Option<bool> = match raw_string(v, "done").map(|s| s.trim().to_ascii_lowercase()) {
        None => Some(false),
        Some(s) if ["any", "all", "both"].contains(&s.as_str()) => None,
        Some(s) => Some(matches!(s.as_str(), "true" | "1" | "yes" | "on")),
    };
    let archived = bool_field(v, "archived").unwrap_or(false);
    let tag = str_field(v, "tag").map(|t| t.to_lowercase());
    let priority = match str_field(v, "priority") {
        Some(p) => Some(parse_priority(p)?),
        None => None,
    };
    let board = match str_field(v, "board").or_else(|| str_field(v, "page")) {
        Some(_) => Some(kanban_handle(ctx, v)?.0),
        None => None,
    };
    let column = str_field(v, "column").map(|c| c.to_lowercase());
    let query = str_field(v, "query").map(|q| q.to_lowercase());
    let limit = usize_field(v, "limit").unwrap_or(100).clamp(1, 1000);
    let sort = str_field(v, "sort").unwrap_or("due").to_ascii_lowercase();
    let mut rows: Vec<TaskRow> = all_cards(ctx)
        .into_iter()
        .filter(|r| archived || !r.archived)
        .filter(|r| done.is_none_or(|d| r.card.is_done() == d))
        .filter(|r| due.keep(r.card.due.as_deref().and_then(parse_days), today))
        .filter(|r| tag.as_deref().is_none_or(|t| r.card.tags.iter().any(|x| x.to_lowercase() == t)))
        .filter(|r| priority.is_none_or(|p| r.card.priority == p))
        .filter(|r| board.as_deref().is_none_or(|b| r.board == b))
        .filter(|r| column.as_deref().is_none_or(|c| r.column.to_lowercase() == c))
        .filter(|r| query.as_deref().is_none_or(|q| r.card.title.to_lowercase().contains(q) || r.card.md.to_lowercase().contains(q)))
        .collect();
    let prio_rank = |p: Option<Priority>| match p {
        Some(Priority::Urgent) => 0,
        Some(Priority::High) => 1,
        Some(Priority::Medium) => 2,
        Some(Priority::Low) => 3,
        None => 4,
    };
    match sort.as_str() {
        "due" => rows.sort_by_key(|r| (r.card.due.as_deref().and_then(parse_days).unwrap_or(i64::MAX), prio_rank(r.card.priority))),
        "priority" => rows.sort_by_key(|r| (prio_rank(r.card.priority), r.card.due.as_deref().and_then(parse_days).unwrap_or(i64::MAX))),
        "created" => rows.sort_by(|a, b| b.card.created.cmp(&a.card.created)),
        "done" => rows.sort_by(|a, b| b.card.done.cmp(&a.card.done)),
        other => return Err(format!("bad \"sort\" \"{other}\" (due | priority | created | done)")),
    }
    let total = rows.len();
    rows.truncate(limit);
    let mut out = format!(
        "--- Tasks · {total} card{}{} · today {} ---\n",
        if total == 1 { "" } else { "s" },
        if total > rows.len() { format!(" (showing {})", rows.len()) } else { String::new() },
        days_to_iso(today)
    );
    if rows.is_empty() {
        out.push_str("  (no cards match)\n");
    }
    let mut last_board = String::new();
    for r in &rows {
        if r.board != last_board {
            out.push_str(&format!("kanban:{} · page \"{}\" ({})\n", r.board, r.page_title, r.page));
            last_board = r.board.clone();
        }
        out.push_str(&format!(
            "  {} · column \"{}\"{}\n",
            card_line(&r.card),
            r.column,
            if r.archived { " · archived" } else { "" }
        ));
    }
    out.push_str(
        "---\nFilters: due (overdue | today | tomorrow | week | month | 14d | none | any | yyyy-mm-dd | a..b), \
         done (false by default | true | any), archived=true, tag, priority, board | page, column, query, \
         sort (due | priority | created | done), limit. Change a card with kanban op=update_card / move_card by id.\n",
    );
    Ok(out)
}

pub(super) fn agenda_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let today = today_days();
    let days = usize_field(v, "days").unwrap_or(7).clamp(1, 60) as i64;
    let done_days = usize_field(v, "done_days").unwrap_or(3).clamp(0, 60) as i64;
    let rows = all_cards(ctx);
    let open: Vec<&TaskRow> = rows.iter().filter(|r| !r.archived && !r.card.is_done()).collect();
    let due_of = |r: &TaskRow| r.card.due.as_deref().and_then(parse_days);
    let mut out = format!("--- Agenda · {} ({}) ---\n", days_to_iso(today), WEEKDAYS[weekday_of(today) as usize]);
    let mut section = |out: &mut String, title: &str, items: Vec<&TaskRow>, with_due: bool| {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("{title} ({}):\n", items.len()));
        for r in items {
            let late = if with_due {
                due_of(r).map(|d| if d < today { format!(" · {} day{} late", today - d, if today - d == 1 { "" } else { "s" }) } else { String::new() }).unwrap_or_default()
            } else {
                String::new()
            };
            out.push_str(&format!("  {}{late}\n", task_line(r)));
        }
    };
    // Планом карточка попадает в день не хуже срока: «начал сегодня» — это
    // тоже сегодняшнее дело, даже если срок ещё не наступил.
    let covers = |r: &TaskRow, day: i64| r.card.schedule().is_some_and(|s| (s.start_day..=s.end_day).contains(&day));
    let starts_on = |r: &TaskRow, day: i64| r.card.schedule().is_some_and(|s| s.start_day == day);
    let mut overdue: Vec<&TaskRow> = open.iter().copied().filter(|r| due_of(r).is_some_and(|d| d < today)).collect();
    overdue.sort_by_key(|r| due_of(r));
    let listed: Vec<String> = overdue.iter().map(|r| r.card.id.clone()).collect();
    section(&mut out, "Overdue", overdue, true);
    let today_rows: Vec<&TaskRow> = open
        .iter()
        .copied()
        .filter(|r| !listed.contains(&r.card.id))
        .filter(|r| due_of(r) == Some(today) || covers(r, today))
        .collect();
    let today_ids: Vec<String> = today_rows.iter().map(|r| r.card.id.clone()).collect();
    section(&mut out, "Today", today_rows, false);
    section(
        &mut out,
        "Tomorrow",
        open.iter()
            .copied()
            .filter(|r| !today_ids.contains(&r.card.id))
            .filter(|r| due_of(r) == Some(today + 1) || starts_on(r, today + 1))
            .collect(),
        false,
    );
    let mut soon: Vec<&TaskRow> = open
        .iter()
        .copied()
        .filter(|r| {
            let by_due = due_of(r).is_some_and(|d| d > today + 1 && d <= today + days);
            let by_plan = r.card.schedule().is_some_and(|s| s.start_day > today + 1 && s.start_day <= today + days);
            by_due || by_plan
        })
        .collect();
    soon.sort_by_key(|r| due_of(r).or_else(|| r.card.schedule().map(|s| s.start_day)));
    section(&mut out, &format!("Next {days} days"), soon, false);
    let important: Vec<&TaskRow> = open
        .iter()
        .copied()
        .filter(|r| due_of(r).is_none() && r.card.schedule().is_none() && matches!(r.card.priority, Some(Priority::High) | Some(Priority::Urgent)))
        .collect();
    section(&mut out, "Important without a due date", important, false);
    // События.
    let store = ctx.calendar_store();
    let s = store.lock();
    let occ = s.occurrences(today, today + days, &[]);
    let events: Vec<String> = occ
        .iter()
        .filter(|o| o.first)
        .filter_map(|o| {
            let e = s.event(&o.event)?;
            let time = o.time.map(|(a, b)| format!(" {}–{}", fmt_hm(a), fmt_hm(b))).unwrap_or_else(|| " all day".to_string());
            let cal = s.calendar(&e.calendar).map(|c| format!(" · calendar \"{}\"", c.name)).unwrap_or_default();
            Some(format!(
                "  {}{time} \"{}\" ({}){}{}{}",
                days_to_iso(o.day),
                e.title,
                e.id,
                cal,
                if e.done { " · done" } else { "" },
                if e.repeat != Repeat::None { format!(" · repeat {}", e.repeat.key()) } else { String::new() }
            ))
        })
        .collect();
    drop(s);
    if !events.is_empty() {
        out.push_str(&format!("Events {} … {} ({}):\n", days_to_iso(today), days_to_iso(today + days), events.len()));
        for l in events {
            out.push_str(&l);
            out.push('\n');
        }
    }
    // Сделано недавно.
    let mut recent: Vec<&TaskRow> = rows
        .iter()
        .filter(|r| r.card.done.as_deref().and_then(parse_days).is_some_and(|d| today - d <= done_days))
        .collect();
    recent.sort_by(|a, b| b.card.done.cmp(&a.card.done));
    if !recent.is_empty() {
        out.push_str(&format!("Done in the last {done_days} day{} ({}):\n", if done_days == 1 { "" } else { "s" }, recent.len()));
        for r in recent {
            out.push_str(&format!("  {} · done {}{}\n", task_line(r), r.card.done.clone().unwrap_or_default(), if r.archived { " · archived" } else { "" }));
        }
    }
    // Доски: счётчики по колонкам.
    let boards = all_objects(ctx, "kanban");
    if !boards.is_empty() {
        out.push_str("Boards:\n");
        for (oid, pid) in boards {
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &oid) else { continue };
            let doc = handle.lock();
            let cols: Vec<String> = doc
                .columns
                .iter()
                .map(|c| format!("{}{} {}", c.name, if c.done { " ✓" } else { "" }, doc.cards_of(&c.id).len()))
                .collect();
            out.push_str(&format!(
                "  kanban:{oid} \"{}\": {}{}\n",
                ctx.title_of(&pid),
                cols.join(" · "),
                if doc.archive.is_empty() { String::new() } else { format!(" · archive {}", doc.archive.len()) }
            ));
        }
    }
    if out.lines().count() == 1 {
        out.push_str("  (no boards, due dates or events yet — kanban op=create and calendar op=add_event start them)\n");
    }
    out.push_str(
        "---\ntasks {due=overdue|today|week|none, tag, priority} lists cards with filters; log {since=7d} shows what \
         changed and when; journal {date=today} opens the day page; a task is done = move_card to the ✓ column. \
         Today and Tomorrow also list cards planned for that day (kanban op=schedule {card, date, duration}).\n",
    );
    Ok(out)
}

pub(super) fn journal_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let day = match str_field(v, "date").map(|s| s.to_ascii_lowercase()) {
        None => today_days(),
        Some(s) if s == "today" => today_days(),
        Some(s) if s == "yesterday" => today_days() - 1,
        Some(s) if s == "tomorrow" => today_days() + 1,
        Some(s) => parse_days(&s).ok_or_else(|| format!("bad \"date\" \"{s}\" (yyyy-mm-dd | today | yesterday | tomorrow)"))?,
    };
    let existed = ctx.find_journal_page(day).is_some();
    let id = ctx.journal_page(day);
    let mut changes = Vec::new();
    if let Some(content) = raw_string(v, "content") {
        let mode = str_field(v, "mode").map(|m| m.to_ascii_lowercase()).unwrap_or_else(|| "append".to_string());
        match mode.as_str() {
            "replace" => {
                write_page(ctx, &id, &content)?;
                changes.push("content replaced".to_string());
            }
            "append" | "prepend" => {
                let mut model = load_model(ctx, &id);
                let pos = if mode == "prepend" { InsertPos::Start } else { InsertPos::End };
                let blocks = fragment_blocks(&content, &parse_geom(v)?, false)?;
                let idx = insert_blocks(&mut model, blocks, pos);
                store_model(ctx, &id, &model)?;
                changes.push(format!("{} block{} {}", idx.len(), if idx.len() == 1 { "" } else { "s" }, if mode == "prepend" { "prepended" } else { "appended" }));
            }
            other => return Err(format!("unknown mode \"{other}\" (append | prepend | replace)")),
        }
    }
    if bool_field(v, "open").unwrap_or(false) {
        ctx.activate(&id);
    }
    let md = plain_markdown(&ctx.page_markdown(&id));
    let mut out = format!(
        "journal page for {} ({}){}\n{}\n--- Markdown ---\n",
        days_to_iso(day),
        WEEKDAYS[weekday_of(day) as usize],
        if existed { String::new() } else { " · created".to_string() },
        page_line(ctx, &id)
    );
    if changes.is_empty() {
        out.insert_str(0, "");
    } else {
        out = format!("{}\n{out}", changes.join(", "));
    }
    if md.trim().is_empty() {
        out.push_str("(empty page)\n");
    } else {
        out.push_str(&md);
        if !md.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}
