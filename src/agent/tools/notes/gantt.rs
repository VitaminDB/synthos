//! Действие `gantt`: задачи диаграммы, зависимости, вехи и масштаб.

use super::*;

fn resolve_task(handle: &GanttHandle, s: &str) -> Result<crate::pages::notes::gantt::model::GanttTask, String> {
    let doc = handle.lock();
    if let Some(t) = doc.tasks.iter().find(|t| t.id == s.trim()) {
        return Ok(t.clone());
    }
    let key = s.trim().to_lowercase();
    let hits: Vec<_> = doc.tasks.iter().filter(|t| t.name.trim().to_lowercase() == key).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!("task \"{s}\" not found — ids and names are in gantt op=read")),
        n => Err(format!(
            "{n} tasks are named \"{s}\" — use the id: {}",
            hits.iter().map(|t| t.id.clone()).collect::<Vec<_>>().join(", ")
        )),
    }
}

pub(super) fn gantt_text(id: &str, handle: &GanttHandle) -> String {
    let doc = handle.lock();
    let mut out = format!(
        "gantt:{id} · tasks: {} · deps: {} · zoom: {} px/day · boards: {}\n",
        doc.tasks.len(),
        doc.deps.len(),
        doc.zoom,
        if doc.boards.is_empty() { "none".to_string() } else { doc.boards.join(", ") }
    );
    for t in &doc.tasks {
        let span = t.span_days().map(|(s, e)| e - s + 1).unwrap_or(0);
        out.push_str(&format!(
            "  task {} \"{}\" · {} → {} ({span} day{}){}\n",
            t.id,
            t.name,
            t.start,
            t.end,
            if span == 1 { "" } else { "s" },
            if t.color.is_empty() { String::new() } else { format!(" · color {}", t.color) }
        ));
    }
    for d in &doc.deps {
        let name = |id: &str| doc.tasks.iter().find(|t| t.id == id).map(|t| t.name.clone()).unwrap_or_default();
        out.push_str(&format!("  dep {} \"{}\" → {} \"{}\"\n", d.from, name(&d.from), d.to, name(&d.to)));
    }
    out
}

/// Текст интеллект-карты: шапка, дерево узлов с id, кросс-ссылки.
pub(super) fn map_text(id: &str, handle: &crate::pages::notes::mindmap::MindmapHandle) -> String {
    let doc = handle.lock();
    let mut out = format!(
        "mindmap:{id} · nodes: {} · links: {} · direction: {} · curve: {}\n",
        doc.nodes.len(),
        doc.links.len(),
        doc.layout.direction.key(),
        doc.layout.curve.key()
    );
    for line in doc.tree_text().lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    for l in &doc.links {
        let text = |id: &str| doc.node(id).map(|n| n.text.clone()).unwrap_or_default();
        out.push_str(&format!("  link {} \"{}\" → {} \"{}\"{}\n", l.from, text(&l.from), l.to, text(&l.to), if l.label.is_empty() { String::new() } else { format!(" · \"{}\"", l.label) }));
    }
    out
}

/// Строка виджета календаря: вид, якорь, фильтр календарей.
pub(super) fn calendar_widget_text(id: &str, handle: &crate::pages::notes::calendar::CalendarHandle) -> String {
    let doc = handle.lock();
    format!(
        "calendar:{id} · view: {} · anchor: {} · calendars: {}\n",
        doc.view.key(),
        doc.anchor,
        if doc.calendars.is_empty() { "all".to_string() } else { doc.calendars.join(", ") }
    )
}

pub(super) fn parse_date_field(v: &Json, key: &str) -> Result<Option<i64>, String> {
    match str_field(v, key) {
        None => Ok(None),
        Some(s) => {
            let lower = s.to_ascii_lowercase();
            if lower == "today" {
                return Ok(Some(today_days()));
            }
            if lower == "tomorrow" {
                return Ok(Some(today_days() + 1));
            }
            parse_days(s).map(Some).ok_or_else(|| format!("bad \"{key}\" \"{s}\" (yyyy-mm-dd)"))
        }
    }
}

pub(super) fn gantt_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op")
        .ok_or(
            "missing \"op\" (create | read | add_task | update_task | delete_task | add_dep | delete_dep | \
             set_zoom | show_today | set_boards {boards = kanban boards whose planned cards show as rows} | delete)",
        )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let id = ctx.create_object("gantt").ok_or("failed to create the chart")?;
            let (pos, idx) = embed_object(ctx, &pid, "gantt", &id, v)?;
            Ok(format!("created chart gantt:{id} {} ({})\n{}\n", pos_text(pos), indices_text(&idx), page_line(ctx, &pid)))
        }
        "read" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            Ok(format!("{}{}\n", gantt_text(&id, &handle), object_page_line(ctx, "gantt", &id)))
        }
        "set_zoom" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let zoom = f32_field(v, "zoom").ok_or("missing \"zoom\" (px per day, 5..90)")?;
            if !(crate::pages::notes::gantt::ZOOM_MIN..=crate::pages::notes::gantt::ZOOM_MAX).contains(&zoom) {
                return Err(format!(
                    "\"zoom\" must be within {}..{} px per day",
                    crate::pages::notes::gantt::ZOOM_MIN,
                    crate::pages::notes::gantt::ZOOM_MAX
                ));
            }
            handle.set_zoom(zoom);
            Ok(format!("zoom {} px/day\n{}", fnum(zoom), gantt_text(&id, &handle)))
        }
        "set_boards" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            // Доски, чьи запланированные карточки идут строками диаграммы;
            // пустой список — только свои задачи.
            let names = list_field(v, "boards").unwrap_or_default();
            let known: Vec<String> = embeds::project_boards(ctx).into_iter().map(|(bid, _, _)| bid).collect();
            let mut ids = Vec::new();
            for n in &names {
                let hit = known.iter().find(|b| *b == n).cloned();
                match hit {
                    Some(b) => ids.push(b),
                    None => {
                        let v = serde_json::json!({ "board": n });
                        ids.push(kanban_handle(ctx, &v)?.0);
                    }
                }
            }
            ids.dedup();
            handle.edit(|doc| doc.boards = ids.clone());
            Ok(format!(
                "chart boards: {}\n{}",
                if ids.is_empty() { "none".to_string() } else { ids.join(", ") },
                gantt_text(&id, &handle)
            ))
        }
        "show_today" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            handle.show_today();
            Ok(format!("scrolled the chart to today\n{}", gantt_text(&id, &handle)))
        }
        "add_task" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let name = str_field(v, "name").ok_or("missing \"name\"")?;
            let start = parse_date_field(v, "start")?.unwrap_or_else(today_days);
            let end = parse_date_field(v, "end")?.unwrap_or(start + 2);
            if end < start {
                return Err("\"end\" is before \"start\"".to_string());
            }
            let mut task = GanttDoc::new_task(name);
            task.start = days_to_iso(start);
            task.end = days_to_iso(end);
            if let Some(c) = str_field(v, "color") {
                task.color = parse_color(c)?;
            }
            let tid = task.id.clone();
            handle.edit(|doc| doc.tasks.push(task));
            if let Some(after) = str_field(v, "after") {
                let dep = resolve_task(&handle, after)?;
                handle.add_dep(&dep.id, &tid);
            }
            Ok(format!("added task {tid} \"{name}\"\n{}", gantt_text(&id, &handle)))
        }
        "update_task" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let task = resolve_task(&handle, str_field(v, "task").ok_or("missing \"task\"")?)?;
            let mut changes = Vec::new();
            if let Some(name) = str_field(v, "name") {
                handle.rename_task(&task.id, name);
                changes.push("name".to_string());
            }
            let start = parse_date_field(v, "start")?;
            let end = parse_date_field(v, "end")?;
            if start.is_some() || end.is_some() {
                let (cur_s, cur_e) = task.span_days().unwrap_or((today_days(), today_days()));
                let s = start.unwrap_or(cur_s);
                let e = end.unwrap_or(if start.is_some() { s + (cur_e - cur_s) } else { cur_e });
                if e < s {
                    return Err("\"end\" is before \"start\"".to_string());
                }
                handle.set_task_dates(&task.id, s, e);
                changes.push(format!("dates {} → {}", days_to_iso(s), days_to_iso(e)));
            }
            if let Some(c) = str_field(v, "color") {
                let color = parse_color(c)?;
                let tid = task.id.clone();
                handle.edit(|doc| {
                    if let Some(t) = doc.tasks.iter_mut().find(|t| t.id == tid) {
                        t.color = color;
                    }
                });
                changes.push("color".to_string());
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, start, end or color".to_string());
            }
            Ok(format!("updated task {}: {}\n{}", task.id, changes.join(", "), gantt_text(&id, &handle)))
        }
        "delete_task" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let task = resolve_task(&handle, str_field(v, "task").ok_or("missing \"task\"")?)?;
            handle.delete_task(&task.id);
            Ok(format!("deleted task \"{}\"\n{}", task.name, gantt_text(&id, &handle)))
        }
        "add_dep" | "delete_dep" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let from = resolve_task(&handle, str_field(v, "from").ok_or("missing \"from\"")?)?;
            let to = resolve_task(&handle, str_field(v, "to").ok_or("missing \"to\"")?)?;
            if op == "add_dep" {
                if from.id == to.id {
                    return Err("a task can't depend on itself".to_string());
                }
                handle.add_dep(&from.id, &to.id);
                Ok(format!("dependency \"{}\" → \"{}\"\n{}", from.name, to.name, gantt_text(&id, &handle)))
            } else {
                handle.delete_dep(&from.id, &to.id);
                Ok(format!("removed dependency \"{}\" → \"{}\"\n{}", from.name, to.name, gantt_text(&id, &handle)))
            }
        }
        "delete" => {
            let (id, _) = gantt_handle(ctx, v)?;
            delete_object(ctx, "gantt", &id)
        }
        other => Err(format!(
            "unknown gantt op \"{other}\" (create | read | add_task | update_task | delete_task | add_dep | \
             delete_dep | set_zoom | show_today | delete)"
        )),
    }
}
