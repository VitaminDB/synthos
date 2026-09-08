//! Адресация: страница по id/названию/пути, объект (доска, диаграмма,
//! карта, календарь, график) по id или по странице, заголовок ответа.

use super::*;

/// Путь страницы в дереве: «Родитель / Страница».
pub(super) fn path_of(ctx: NotesCtx, id: &str) -> String {
    ctx.tree
        .get_untracked()
        .path_of(id)
        .into_iter()
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Строка «page: <id> · "Название" · path: …» — единый заголовок в ответах;
/// по ней же ссылка «Открыть в заметках» в чате находит страницу. У
/// свободной раскладки к ней дописывается [`free_layout_hint`].
pub(super) fn page_line(ctx: NotesCtx, id: &str) -> String {
    let mut s = format!("page: {id} · \"{}\" · path: {}", ctx.title_of(id), path_of(ctx, id));
    if let Some(hint) = free_layout_hint(ctx, id) {
        s.push('\n');
        s.push_str(&hint);
    }
    s
}

/// Страница — всегда холст: блок без `x`/`y` уходит в колонку потока по
/// центру холста, а закреплённые стоят по координатам — две системы
/// отсчёта на одной странице, и текст оказывается под фигурами и досками.
/// Модель координат не видит и по умолчанию надеется, что редактор
/// разложит блоки сам, поэтому предупреждение висит в КАЖДОМ ответе по
/// такой странице, пока геометрия не проставлена (07.09.2026: страница
/// «Тестовая» из `create` + markdown легла кашей ровно так).
///
/// Пока на странице ничего не закреплено, она рисуется одной колонкой и
/// выглядит как обычный документ. Поэтому подсказка только при СМЕСИ: есть
/// и закреплённые блоки, и блоки без координат.
fn free_layout_hint(ctx: NotesCtx, id: &str) -> Option<String> {
    let model = load_model(ctx, id);
    let mut flow: Vec<usize> = Vec::new();
    let mut bottom: Option<f32> = None;
    for (i, b) in model.blocks.iter().enumerate() {
        match block_rect(b) {
            Some((_, y, _, h)) => bottom = Some(bottom.map_or(y + h, |m| m.max(y + h))),
            None => flow.push(i),
        }
    }
    let (Some(bottom), false) = (bottom, flow.is_empty()) else {
        return None;
    };
    const SHOWN: usize = 12;
    let shown = flow.iter().take(SHOWN).map(|i| format!("#{i}")).collect::<Vec<_>>().join(" ");
    let more = if flow.len() > SHOWN { format!(" … +{}", flow.len() - SHOWN) } else { String::new() };
    Some(format!(
        "!! free layout: {} of {} blocks have no x/y ({shown}{more}) — they are drawn as one \
         centred column ON TOP of the pinned blocks, so the page looks like a pile. Nothing \
         places blocks for you: give every block its own x y w (and h for shapes, media, \
         boards, charts, mind maps and calendars) with blocks op=pin or op=set_attrs (blocks \
         op=arrange lays the rest out in a column). Pinned content currently ends at y={}.",
        flow.len(),
        model.blocks.len(),
        fnum(bottom)
    ))
}

/// Страница по ссылке агента: id, название либо путь «A / B».
pub(super) fn resolve_page_ref(ctx: NotesCtx, raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let s = s.strip_prefix("id:").unwrap_or(s).trim();
    if s.is_empty() {
        return Err("empty page reference".to_string());
    }
    let tree = ctx.tree.get_untracked();
    if tree.find(s).is_some() {
        return Ok(s.to_string());
    }
    let by_title = ctx.find_by_title(s);
    if by_title.len() == 1 {
        return Ok(by_title[0].clone());
    }
    if s.contains('/') {
        let parts: Vec<String> = s
            .split('/')
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .collect();
        let hits: Vec<String> = tree
            .all()
            .into_iter()
            .filter(|n| {
                let path: Vec<String> =
                    tree.path_of(&n.id).into_iter().map(|(_, t)| t.trim().to_lowercase()).collect();
                path.len() >= parts.len() && path[path.len() - parts.len()..] == parts[..]
            })
            .map(|n| n.id.clone())
            .collect();
        match hits.len() {
            1 => return Ok(hits[0].clone()),
            0 => {}
            _ => return Err(ambiguous(ctx, s, &hits)),
        }
    }
    if by_title.len() > 1 {
        return Err(ambiguous(ctx, s, &by_title));
    }
    Err(format!(
        "page \"{s}\" not found — see notes list; pages are addressed by id (12 hex) or exact title"
    ))
}

fn ambiguous(ctx: NotesCtx, s: &str, hits: &[String]) -> String {
    let list: Vec<String> = hits.iter().map(|id| format!("{id} ({})", path_of(ctx, id))).collect();
    format!("\"{s}\" matches {} pages — use the id: {}", hits.len(), list.join(", "))
}

/// Обязательное поле `key` со ссылкой на страницу.
pub(super) fn page_arg(ctx: NotesCtx, v: &Json, key: &'static str) -> Result<String, String> {
    let s = str_field(v, key).ok_or_else(|| format!("missing \"{key}\" (page id or title)"))?;
    resolve_page_ref(ctx, s)
}

/// Все объекты вида `kind` в проекте: (id объекта, id страницы).
pub(super) fn all_objects(ctx: NotesCtx, kind: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pid in ctx.tree.get_untracked().all_ids() {
        for (k, oid) in object_refs(&ctx.page_markdown(&pid)) {
            if k == kind && !out.iter().any(|(o, _)| *o == oid) {
                out.push((oid, pid.clone()));
            }
        }
    }
    out
}

/// Объект `kind` по полю `key` (id объекта либо страница с единственным
/// таким объектом); без поля — по `page`, а без неё — единственный в проекте.
pub(super) fn resolve_object(ctx: NotesCtx, v: &Json, kind: &str, key: &str) -> Result<LiveObject, String> {
    let noun = match kind {
        "kanban" => "board",
        "gantt" => "gantt chart",
        "mindmap" => "map",
        "chart" => "chart",
        _ => "calendar",
    };
    let candidates: Vec<(String, String)> = match str_field(v, key) {
        Some(s) => {
            let s = s.strip_prefix(&format!("{kind}:")).unwrap_or(s).trim();
            if let Some(o) = ctx.object(kind, s) {
                return Ok(o);
            }
            let pid = resolve_page_ref(ctx, s)
                .map_err(|_| format!("{noun} \"{s}\" not found — ids are in notes list / read"))?;
            object_refs(&ctx.page_markdown(&pid))
                .into_iter()
                .filter(|(k, _)| k == kind)
                .map(|(_, oid)| (oid, pid.clone()))
                .collect()
        }
        None => match str_field(v, "page") {
            Some(p) => {
                let pid = resolve_page_ref(ctx, p)?;
                object_refs(&ctx.page_markdown(&pid))
                    .into_iter()
                    .filter(|(k, _)| k == kind)
                    .map(|(_, oid)| (oid, pid.clone()))
                    .collect()
            }
            None => all_objects(ctx, kind),
        },
    };
    match candidates.len() {
        1 => ctx
            .object(kind, &candidates[0].0)
            .ok_or_else(|| format!("{noun} {kind}:{} is referenced but its file is missing", candidates[0].0)),
        0 => Err(format!("no {noun} found — create one with {kind} op=create, page=<page>")),
        _ => Err(format!(
            "several {noun}s match — pass \"{key}\": {}",
            candidates
                .iter()
                .map(|(oid, pid)| format!("{kind}:{oid} (page \"{}\")", ctx.title_of(pid)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub(super) fn kanban_handle(ctx: NotesCtx, v: &Json) -> Result<(String, KanbanHandle), String> {
    match resolve_object(ctx, v, "kanban", "board")? {
        LiveObject::Kanban { id, handle } => Ok((id, handle)),
        _ => Err("not a kanban board".to_string()),
    }
}

pub(super) fn mindmap_handle(ctx: NotesCtx, v: &Json) -> Result<(String, MindmapHandle), String> {
    match resolve_object(ctx, v, "mindmap", "map")? {
        LiveObject::Mindmap { id, handle } => Ok((id, handle)),
        _ => Err("not a mind map".to_string()),
    }
}

pub(super) fn calendar_handle(ctx: NotesCtx, v: &Json) -> Result<(String, CalendarHandle), String> {
    match resolve_object(ctx, v, "calendar", "calendar")? {
        LiveObject::Calendar { id, handle } => Ok((id, handle)),
        _ => Err("not a calendar widget".to_string()),
    }
}

pub(super) fn gantt_handle(ctx: NotesCtx, v: &Json) -> Result<(String, GanttHandle), String> {
    // Диаграмму адресует ключ "gantt": "chart" с появлением графиков
    // (`![[chart:<id>]]`) означает их, но как старое имя ещё принимается.
    let key = if str_field(v, "gantt").is_some() { "gantt" } else { "chart" };
    match resolve_object(ctx, v, "gantt", key)? {
        LiveObject::Gantt { id, handle } => Ok((id, handle)),
        _ => Err("not a gantt chart".to_string()),
    }
}

/// Страница, на которой врезан объект (первая из ссылающихся).
pub(super) fn object_page(ctx: NotesCtx, kind: &str, id: &str) -> Option<String> {
    ctx.pages_referencing(kind, id).into_iter().next()
}

pub(super) fn object_page_line(ctx: NotesCtx, kind: &str, id: &str) -> String {
    match object_page(ctx, kind, id) {
        Some(pid) => page_line(ctx, &pid),
        None => "page: (not embedded in any page)".to_string(),
    }
}
