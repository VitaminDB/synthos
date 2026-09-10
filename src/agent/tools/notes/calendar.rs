//! Действие `calendar`: события, повторы, календари проекта и стиль.

use super::*;

/// Дата из поля: `yyyy-mm-dd`, `today`, `tomorrow`.
pub(super) fn day_arg(v: &Json, key: &str) -> Result<Option<i64>, String> {
    parse_date_field(v, key)
}

/// Время из поля `HH:MM`.
fn time_arg(v: &Json, key: &str) -> Result<Option<u32>, String> {
    match raw_string(v, key) {
        None => Ok(None),
        Some(s) if s.trim().is_empty() || s.eq_ignore_ascii_case("none") => Ok(Some(u32::MAX)),
        Some(s) => parse_hm(&s).map(Some).ok_or_else(|| format!("bad \"{key}\" \"{s}\" (HH:MM, local time)")),
    }
}

/// Календарь по id либо названию.
fn resolve_calendar(store: &CalendarStore, s: &str) -> Result<String, String> {
    let t = s.trim();
    if store.calendar(t).is_some() {
        return Ok(t.to_string());
    }
    let key = t.to_lowercase();
    let hits: Vec<&str> = store.calendars.iter().filter(|c| c.name.trim().to_lowercase() == key).map(|c| c.id.as_str()).collect();
    match hits.len() {
        1 => Ok(hits[0].to_string()),
        0 => Err(format!(
            "calendar \"{s}\" not found — calendars: {}",
            store.calendars.iter().map(|c| format!("\"{}\" ({})", c.name, c.id)).collect::<Vec<_>>().join(", ")
        )),
        n => Err(format!("{n} calendars are named \"{s}\" — use the id", )),
    }
}

/// Событие по id либо названию (с необязательным уточнением по дате).
fn resolve_event(store: &CalendarStore, s: &str, on: Option<i64>) -> Result<String, String> {
    let t = s.trim();
    if store.event(t).is_some() {
        return Ok(t.to_string());
    }
    let key = t.to_lowercase();
    let hits: Vec<&CalEvent> = store
        .events
        .iter()
        .filter(|e| e.title.trim().to_lowercase() == key)
        .filter(|e| on.is_none_or(|d| e.day() == Some(d)))
        .collect();
    match hits.len() {
        1 => Ok(hits[0].id.clone()),
        0 => Err(format!("event \"{s}\" not found — ids and titles are in calendar op=list_events")),
        n => Err(format!(
            "{n} events are titled \"{s}\" — pass the id or a date: {}",
            hits.iter().map(|e| format!("{} ({})", e.id, e.date)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn event_line(store: &CalendarStore, e: &CalEvent) -> String {
    let mut s = format!("{} \"{}\" · {}", e.id, e.title, e.date);
    if let Some(end) = &e.end_date {
        s.push_str(&format!(" → {end}"));
    }
    match e.time_span() {
        Some((from, to)) => s.push_str(&format!(" · {}–{}", fmt_hm(from), fmt_hm(to))),
        None => s.push_str(" · all day"),
    }
    if let Some(c) = store.calendar(&e.calendar) {
        s.push_str(&format!(" · calendar \"{}\"", c.name));
    }
    if e.repeat != Repeat::None {
        s.push_str(&format!(" · repeat {}", e.repeat.key()));
        if !e.days.is_any() {
            s.push_str(&format!(" on {}", e.days.label()));
        }
        if let Some(u) = &e.until {
            s.push_str(&format!(" until {u}"));
        }
    }
    if e.done {
        s.push_str(" · done");
    }
    if !e.color.is_empty() {
        s.push_str(&format!(" · color {}", e.color));
    }
    if let Some(l) = &e.link {
        s.push_str(&format!(" · page {l}"));
    }
    if !e.note.trim().is_empty() {
        s.push_str(&format!(" · note {} chars", e.note.chars().count()));
    }
    s
}

/// Текст хранилища: календари + события диапазона (по вхождениям).
fn calendar_text(store: &CalendarStore, from: i64, to: i64, filter: &[String], external: &[String]) -> String {
    let mut out = format!(
        "calendars: {}\n",
        store.calendars.iter().map(|c| format!("{} \"{}\" {}", c.id, c.name, if c.color.is_empty() { "theme" } else { c.color.as_str() })).collect::<Vec<_>>().join(", ")
    );
    out.push_str(&format!("--- Events {} … {} ---\n", days_to_iso(from), days_to_iso(to)));
    let occ = store.occurrences(from, to, filter);
    let mut seen: Vec<&str> = Vec::new();
    for o in &occ {
        if seen.contains(&o.event.as_str()) {
            continue;
        }
        seen.push(&o.event);
        if let Some(e) = store.event(&o.event) {
            out.push_str(&format!("  {}\n", event_line(store, e)));
        }
    }
    if seen.is_empty() {
        out.push_str("  (no events in this range)\n");
    }
    for line in external {
        out.push_str(&format!("  external: {line}\n"));
    }
    out
}

/// Стиль виджета календаря из аргумента `style`.
fn apply_calendar_style(handle: &CalendarHandle, v: &Json) -> Result<Vec<String>, String> {
    let Some(raw) = v.get("style") else { return Ok(Vec::new()) };
    let pairs = style_pairs(raw)?;
    let mut changes = Vec::new();
    let mut err = None;
    handle.set_style(|s| {
        for (k, val) in &pairs {
            let key = k.trim().to_ascii_lowercase();
            let flag = || matches!(val.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
            match key.as_str() {
                "preset" => {
                    if !CalendarStyle::PRESETS.contains(&val.trim()) {
                        err = Some(format!("bad preset \"{val}\" (theme | light | contrast | pastel)"));
                        return;
                    }
                    s.apply_preset(val.trim());
                    changes.push(format!("preset {}", val.trim()));
                }
                "event_style" => match EventStyle::parse(val) {
                    Some(e) => {
                        s.event_style = e;
                        changes.push(format!("event_style {}", e.key()));
                    }
                    None => err = Some(format!("bad event_style \"{val}\" (chip | dot | bar)")),
                },
                "weekend_tint" | "today_color" | "header_bg" | "cell_bg" | "grid_color" | "text_color" => {
                    let color = if val.trim().is_empty() || val.eq_ignore_ascii_case("none") {
                        String::new()
                    } else {
                        match parse_hex_color(val, true) {
                            Ok(c) => c,
                            Err(e) => {
                                err = Some(e);
                                return;
                            }
                        }
                    };
                    match key.as_str() {
                        "weekend_tint" => s.weekend_tint = color,
                        "today_color" => s.today_color = color,
                        "header_bg" => s.header_bg = color,
                        "cell_bg" => s.cell_bg = color,
                        "grid_color" => s.grid_color = color,
                        _ => s.text_color = color,
                    }
                    s.preset.clear();
                    changes.push(key.clone());
                }
                "first_weekday" | "slot_min" => {
                    let Ok(n) = val.trim().parse::<u32>() else {
                        err = Some(format!("bad \"{key}\" \"{val}\" — a whole number"));
                        return;
                    };
                    match key.as_str() {
                        "first_weekday" => s.first_weekday = n,
                        _ => s.slot_min = n,
                    }
                    changes.push(format!("{key}={n}"));
                }
                // Границы окна суток: «HH:MM» либо целый час («6» = 06:00
                // — так же, как в старых ключах hour_from/hour_to).
                "day_from" | "day_to" | "hour_from" | "hour_to" => {
                    let t = val.trim();
                    let Some(min) = parse_hm(t).or_else(|| t.parse::<u32>().ok().filter(|h| *h <= 24).map(|h| h * 60)) else {
                        err = Some(format!("bad \"{key}\" \"{val}\" (HH:MM or a whole hour)"));
                        return;
                    };
                    if key == "day_from" || key == "hour_from" {
                        s.from_min = min;
                        if s.full_day {
                            s.to_min = min;
                        }
                    } else {
                        s.to_min = min;
                    }
                    changes.push(format!("{key}={}", fmt_hm(min.min(24 * 60 - 1))));
                }
                "full_day" => {
                    s.full_day = flag();
                    if s.full_day {
                        s.to_min = s.from_min;
                    }
                    changes.push(key.clone());
                }
                "font_size" => match val.trim().parse::<f32>() {
                    Ok(n) => {
                        s.font_size = n;
                        changes.push(format!("font_size={}", fnum(n)));
                    }
                    Err(_) => err = Some(format!("bad \"font_size\" \"{val}\" — a number")),
                },
                "show_week_numbers" => {
                    s.show_week_numbers = flag();
                    changes.push(key.clone());
                }
                "compact" => {
                    s.compact = flag();
                    changes.push(key.clone());
                }
                "show_kanban_due" => {
                    s.show_kanban_due = flag();
                    changes.push(key.clone());
                }
                "show_gantt" => {
                    s.show_gantt = flag();
                    changes.push(key.clone());
                }
                "show_kanban_spans" | "show_spans" => {
                    s.show_kanban_spans = flag();
                    changes.push("show_kanban_spans".to_string());
                }
                other => {
                    err = Some(format!(
                        "unknown style key \"{other}\" — preset, event_style, first_weekday, show_week_numbers, \
                         day_from, day_to, full_day, slot_min, compact, font_size, weekend_tint, today_color, header_bg, \
                         cell_bg, grid_color, text_color, show_kanban_due, show_kanban_spans, show_gantt"
                    ));
                }
            }
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(changes),
    }
}

/// Поля события из аргументов (используют add_event и update_event).
fn apply_event_fields(ctx: NotesCtx, store: &CalendarStore, e: &mut CalEvent, v: &Json) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if let Some(t) = str_field(v, "title") {
        e.title = t.to_string();
        changes.push("title".to_string());
    }
    if let Some(d) = day_arg(v, "date")? {
        // Многодневное событие переезжает целиком: конечная дата сдвигается
        // на ту же дельту, если её не задали явно.
        if v.get("end_date").is_none() {
            if let (Some(old), Some(end)) = (e.day(), e.end_date.as_deref().and_then(parse_days)) {
                e.end_date = Some(days_to_iso(end + (d - old)));
            }
        }
        e.date = days_to_iso(d);
        changes.push("date".to_string());
    }
    if let Some(raw) = raw_string(v, "end_date") {
        e.end_date = if raw.trim().is_empty() || raw.eq_ignore_ascii_case("none") {
            None
        } else {
            let d = day_arg(&serde_json::json!({ "end_date": raw }), "end_date")?.ok_or("bad \"end_date\"")?;
            Some(days_to_iso(d))
        };
        changes.push("end_date".to_string());
    }
    if let Some(s) = time_arg(v, "start_time")? {
        if s == u32::MAX {
            e.start = None;
            e.end = None;
            e.all_day = true;
        } else {
            let dur = match (e.start, e.end) {
                (Some(a), Some(b)) if b > a => b - a,
                _ => 60,
            };
            e.start = Some(s);
            e.end = Some((s + dur).min(24 * 60));
            e.all_day = false;
        }
        changes.push("start_time".to_string());
    }
    if let Some(t) = time_arg(v, "end_time")? {
        if t != u32::MAX {
            e.end = Some(t.max(e.start.unwrap_or(0) + 5).min(24 * 60));
            e.all_day = false;
            changes.push("end_time".to_string());
        }
    }
    if let Some(a) = bool_field(v, "all_day") {
        e.all_day = a;
        if a {
            e.start = None;
            e.end = None;
        } else if e.start.is_none() {
            e.start = Some(9 * 60);
            e.end = Some(10 * 60);
        }
        changes.push("all_day".to_string());
    }
    if let Some(c) = str_field(v, "calendar") {
        e.calendar = resolve_calendar(store, c)?;
        changes.push("calendar".to_string());
    }
    if let Some(c) = raw_string(v, "color") {
        e.color = if c.trim().is_empty() || c.eq_ignore_ascii_case("none") { String::new() } else { parse_hex_color(&c, false)? };
        changes.push("color".to_string());
    }
    if let Some(n) = raw_string(v, "note") {
        e.note = n;
        changes.push("note".to_string());
    }
    if let Some(d) = bool_field(v, "done") {
        e.done = d;
        changes.push("done".to_string());
    }
    if let Some(r) = str_field(v, "repeat") {
        // `weekdays` / `weekends` — сокращения: ежедневный повтор плюс маска
        // дней недели (одно событие вместо дюжины отдельных).
        match r.trim().to_ascii_lowercase().as_str() {
            "weekdays" | "workdays" | "будни" => {
                e.repeat = Repeat::Daily;
                e.days = Weekdays::WEEKDAYS;
            }
            "weekends" | "weekend" | "выходные" => {
                e.repeat = Repeat::Daily;
                e.days = Weekdays::WEEKENDS;
            }
            _ => {
                e.repeat = Repeat::parse(r).ok_or_else(|| {
                    format!("bad repeat \"{r}\" (none | daily | weekly | monthly | yearly | weekdays | weekends)")
                })?;
                if e.repeat == Repeat::None {
                    e.days = Weekdays::default();
                }
            }
        }
        changes.push("repeat".to_string());
    }
    // Дни недели повтора: `only_days` оставляет перечисленные, `skip_days`
    // выбрасывает их. Пустой список (или "none"/"all") снимает фильтр.
    // `days` — та же маска: у события это ничего другого значить не может,
    // а модель нет-нет да и напишет короткое имя.
    let only = list_field(v, "only_days").or_else(|| list_field(v, "days"));
    if let Some(list) = &only {
        e.days = Weekdays::parse_list(list)?;
        changes.push("only_days".to_string());
    }
    if let Some(list) = list_field(v, "skip_days") {
        let skip = Weekdays::parse_list(&list)?;
        // Считаем от целой недели (или от only_days этого же вызова), а не
        // от прежней маски: «skip_days: [sun]» — это «все дни, кроме вс».
        let base = if only.is_some() { e.days } else { Weekdays::default() };
        e.days = if skip.is_any() { Weekdays::default() } else { base.without(skip) };
        changes.push("skip_days".to_string());
    }
    if !e.days.is_any() && e.repeat == Repeat::None {
        return Err(
            "only_days/skip_days need a repeat: pass repeat=daily (or weekly/monthly/yearly) \
             — one repeating event with a weekday filter replaces a dozen single ones"
                .to_string(),
        );
    }
    if let Some(raw) = raw_string(v, "until") {
        e.until = if raw.trim().is_empty() || raw.eq_ignore_ascii_case("none") {
            None
        } else {
            let d = day_arg(&serde_json::json!({ "until": raw }), "until")?.ok_or("bad \"until\"")?;
            Some(days_to_iso(d))
        };
        changes.push("until".to_string());
    }
    if let Some(l) = raw_string(v, "link") {
        e.link = if l.trim().is_empty() || l.eq_ignore_ascii_case("none") { None } else { Some(resolve_page_ref(ctx, &l)?) };
        changes.push("link".to_string());
    }
    Ok(changes)
}

/// Хранилище событий проекта плюс диапазон по умолчанию (вид виджета,
/// иначе месяц вокруг сегодня).
fn calendar_range(ctx: NotesCtx, v: &Json) -> Result<(CalendarStoreHandle, i64, i64, Vec<String>), String> {
    let store = ctx.calendar_store();
    let widget = calendar_handle(ctx, v).ok();
    let (view, anchor, filter, first) = match &widget {
        Some((_, h)) => {
            let d = h.lock();
            (d.view, d.anchor_days(), d.calendars.clone(), d.style.first_weekday)
        }
        None => (CalView::Month, today_days(), Vec::new(), 0),
    };
    let (mut from, mut to) = crate::pages::notes::calendar::model::range_of(view, anchor, first);
    if let Some(f) = day_arg(v, "from")? {
        from = f;
        to = to.max(f);
    }
    if let Some(t) = day_arg(v, "to")? {
        to = t;
    }
    if to < from {
        std::mem::swap(&mut from, &mut to);
    }
    let filter = match str_field(v, "calendar") {
        Some(c) => vec![resolve_calendar(&store.lock(), c)?],
        None => filter,
    };
    Ok((store, from, to, filter))
}

pub(super) fn calendar_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | set_view | set_style | add_event | update_event | move_event | \
         delete_event | complete | list_events | add_calendar | update_calendar | delete_calendar | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let view = match str_field(v, "view") {
                Some(s) => CalView::parse(s).ok_or_else(|| format!("bad view \"{s}\" (year | month | week | day)"))?,
                None => CalView::Month,
            };
            let id = ctx.create_calendar(view);
            let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &id) else {
                return Err("calendar vanished".to_string());
            };
            if let Some(a) = day_arg(v, "anchor")? {
                handle.set_anchor(a);
            }
            let (pos, idx) = embed_object_with(ctx, &pid, "calendar", &id, v, str_field(v, "heading"))?;
            Ok(format!(
                "created calendar:{id} ({} view) {} ({})\n{}\n",
                view.key(),
                pos_text(pos),
                indices_text(&idx),
                page_line(ctx, &pid)
            ))
        }
        "read" | "list_events" => {
            let (store, from, to, filter) = calendar_range(ctx, v)?;
            let mut out = String::new();
            if op == "read" {
                if let Ok((id, handle)) = calendar_handle(ctx, v) {
                    let d = handle.lock();
                    out.push_str(&format!(
                        "calendar:{id} · view: {} · anchor: {} · calendars: {} · first weekday: {} · day window {}–{}{} · slot {} min · events as {}{}{}\n",
                        d.view.key(),
                        d.anchor,
                        if d.calendars.is_empty() { "all".to_string() } else { d.calendars.join(", ") },
                        d.style.first_weekday,
                        fmt_hm(d.style.window().0),
                        fmt_hm((d.style.window().0 + d.style.window().1) % (24 * 60)),
                        if d.style.window().1 >= 24 * 60 { " (24 h)" } else { "" },
                        d.style.slot_min,
                        d.style.event_style.key(),
                        if d.style.show_kanban_due { " · board due dates" } else { "" },
                        if d.style.show_gantt { " · gantt tasks" } else { "" }
                    ));
                    out.push_str(&format!(
                        "  boards: {}{}\n",
                        if d.boards.is_empty() { "all".to_string() } else { d.boards.join(", ") },
                        if d.style.show_kanban_spans { " · task bars (card start/end)" } else { " · task bars off" }
                    ));
                    drop(d);
                    out.push_str(&object_page_line(ctx, "calendar", &id));
                    out.push('\n');
                }
            }
            let external = if bool_field(v, "include_external").unwrap_or(true) {
                let mut items = Vec::new();
                let q = ExternalQuery { from, to, boards: Vec::new(), due: true, spans: true, gantt: true };
                for it in (embeds::calendar_env(ctx).external)(&q) {
                    items.push(format!(
                        "{}{}{} \"{}\" · page {} (a board task or Gantt task, not an event — don't duplicate it as an event)",
                        days_to_iso(it.day),
                        it.time.map(|(a, b)| format!(" {}–{}", fmt_hm(a), fmt_hm(b))).unwrap_or_default(),
                        if it.end_day != it.day { format!(" → {}", days_to_iso(it.end_day)) } else { String::new() },
                        it.title,
                        it.page
                    ));
                }
                items
            } else {
                Vec::new()
            };
            out.push_str(&calendar_text(&store.lock(), from, to, &filter, &external));
            Ok(out)
        }
        "set_view" => {
            let (id, handle) = calendar_handle(ctx, v)?;
            let mut changes = Vec::new();
            if let Some(s) = str_field(v, "view") {
                let view = CalView::parse(s).ok_or_else(|| format!("bad view \"{s}\" (year | month | week | day)"))?;
                handle.set_view(view);
                changes.push(format!("view {}", view.key()));
            }
            if let Some(a) = day_arg(v, "anchor")? {
                handle.set_anchor(a);
                changes.push(format!("anchor {}", days_to_iso(a)));
            }
            if let Some(list) = list_field(v, "calendars") {
                let store = ctx.calendar_store();
                let store = store.lock();
                let ids: Result<Vec<String>, String> = list.iter().map(|c| resolve_calendar(&store, c)).collect();
                drop(store);
                let ids = ids?;
                handle.set_calendars(ids.clone());
                changes.push(format!("calendars {}", if ids.is_empty() { "all".to_string() } else { ids.join(", ") }));
            }
            if let Some(list) = list_field(v, "boards") {
                // Доски-источники задач; пустой список — все доски проекта.
                let known: Vec<String> = embeds::project_boards(ctx).into_iter().map(|(bid, _, _)| bid).collect();
                let mut ids = Vec::new();
                for n in &list {
                    match known.iter().find(|b| *b == n).cloned() {
                        Some(b) => ids.push(b),
                        None => ids.push(kanban_handle(ctx, &serde_json::json!({ "board": n }))?.0),
                    }
                }
                ids.dedup();
                handle.set_boards(ids.clone());
                changes.push(format!("boards {}", if ids.is_empty() { "all".to_string() } else { ids.join(", ") }));
            }
            if changes.is_empty() {
                return Err("nothing to change: pass view, anchor, calendars or boards".to_string());
            }
            Ok(format!("{}\n{}", changes.join(", "), calendar_widget_text(&id, &handle)))
        }
        "set_style" => {
            let (id, handle) = calendar_handle(ctx, v)?;
            let changes = apply_calendar_style(&handle, v)?;
            if changes.is_empty() {
                return Err("nothing to change: pass style with preset, event_style, hours, colors or layers".to_string());
            }
            Ok(format!("style: {}\n{}", changes.join(", "), calendar_widget_text(&id, &handle)))
        }
        "add_event" => {
            let store = ctx.calendar_store();
            let title = str_field(v, "title").ok_or("missing \"title\"")?.to_string();
            let day = day_arg(v, "date")?.unwrap_or_else(today_days);
            let snapshot = store.lock().clone();
            let calendar = match str_field(v, "calendar") {
                Some(c) => resolve_calendar(&snapshot, c)?,
                None => snapshot.calendars.first().map(|c| c.id.clone()).unwrap_or_default(),
            };
            let mut event = CalEvent::new(&calendar, &title, day);
            apply_event_fields(ctx, &snapshot, &mut event, v)?;
            let id = store.add_event(event);
            let s = store.lock();
            let e = s.event(&id).ok_or("event vanished")?;
            Ok(format!("added event {}\n", event_line(&s, e)))
        }
        "update_event" | "move_event" | "complete" | "delete_event" => {
            let store = ctx.calendar_store();
            let snapshot = store.lock().clone();
            let on = day_arg(v, "on")?;
            let id = resolve_event(&snapshot, str_field(v, "event").ok_or("missing \"event\"")?, on)?;
            if op == "delete_event" {
                let title = snapshot.event(&id).map(|e| e.title.clone()).unwrap_or_default();
                store.remove_event(&id);
                return Ok(format!("deleted event \"{title}\" ({id})\n"));
            }
            if op == "complete" {
                let done = bool_field(v, "done").unwrap_or(true);
                store.update_event(&id, |e| e.done = done);
                let s = store.lock();
                return Ok(format!("event {}\n", event_line(&s, s.event(&id).ok_or("event vanished")?)));
            }
            let mut event = snapshot.event(&id).cloned().ok_or("event vanished")?;
            let mut changes = apply_event_fields(ctx, &snapshot, &mut event, v)?;
            if op == "move_event" && changes.is_empty() {
                return Err("nothing to move: pass date and/or start_time".to_string());
            }
            if changes.is_empty() {
                return Err(
                    "nothing to update: pass title, date, end_date, start_time, end_time, all_day, calendar, \
                     color, note, done, repeat, only_days, skip_days, until or link"
                        .to_string(),
                );
            }
            store.update_event(&id, |e| {
                let keep = e.id.clone();
                *e = event;
                e.id = keep;
            });
            changes.dedup();
            let s = store.lock();
            Ok(format!("updated {}: {}\n", changes.join(", "), event_line(&s, s.event(&id).ok_or("event vanished")?)))
        }
        "add_calendar" => {
            let store = ctx.calendar_store();
            let name = str_field(v, "name").ok_or("missing \"name\"")?;
            let color = match str_field(v, "color") {
                Some(c) => parse_hex_color(c, false)?,
                None => String::new(),
            };
            let id = store.add_calendar(name, &color);
            let s = store.lock();
            Ok(format!("added calendar {id} \"{name}\"\n{}", calendar_text(&s, today_days(), today_days(), &[], &[])))
        }
        "update_calendar" => {
            let store = ctx.calendar_store();
            let snapshot = store.lock().clone();
            let id = resolve_calendar(&snapshot, str_field(v, "calendar").ok_or("missing \"calendar\"")?)?;
            let name = str_field(v, "name").map(str::to_string);
            let color = match str_field(v, "color") {
                Some(c) => Some(parse_hex_color(c, false)?),
                None => None,
            };
            if name.is_none() && color.is_none() {
                return Err("nothing to update: pass name or color".to_string());
            }
            store.edit(|s| {
                if let Some(c) = s.calendars.iter_mut().find(|c| c.id == id) {
                    if let Some(n) = &name {
                        c.name = n.clone();
                    }
                    if let Some(col) = &color {
                        c.color = col.clone();
                    }
                }
            });
            let s = store.lock();
            Ok(format!("updated calendar {id}\n{}", calendar_text(&s, today_days(), today_days(), &[], &[])))
        }
        "delete_calendar" => {
            let store = ctx.calendar_store();
            let snapshot = store.lock().clone();
            let id = resolve_calendar(&snapshot, str_field(v, "calendar").ok_or("missing \"calendar\"")?)?;
            if !store.remove_calendar(&id) {
                return Err("the last calendar can't be deleted".to_string());
            }
            let s = store.lock();
            Ok(format!("deleted calendar {id} (its events moved to \"{}\")\n", s.calendars[0].name))
        }
        "delete" => {
            let (id, _) = calendar_handle(ctx, v)?;
            let out = delete_object(ctx, "calendar", &id)?;
            Ok(format!("{out}the events stay in the project calendar\n"))
        }
        other => Err(format!(
            "unknown calendar op \"{other}\" (create | read | set_view | set_style | add_event | update_event | \
             move_event | delete_event | complete | list_events | add_calendar | update_calendar | \
             delete_calendar | delete)"
        )),
    }
}
