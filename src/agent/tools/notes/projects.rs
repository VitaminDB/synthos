//! Несколько открытых проектов: в каком проекте исполняется вызов
//! (`project`, id страницы или объекта) и сводные ответы по всем открытым.
//!
//! UI показывает один проект, агенту видны все: `list`, `search` и обзоры
//! (`agenda`, `tasks`, `log`) без `project` идут по всем открытым,
//! остальные действия — в показанный либо в названный `project`.
//! Неактивный проект правится через [`NotesCtx::with_project`] —
//! пользователь остаётся на своей плитке.
//!
//! 17.09.2026, MyLife: открыты «SynthOS Notes» и «Notes», пользователь
//! спросил про второй. Модель передала файл в `path` — аргумент молча
//! игнорировался, и она дважды получила дерево первого проекта, а потом
//! решила, что файла нет.

use super::*;
use crate::pages::notes::project;
use crate::pages::notes::projects::ProjectError;

/// Исполнить действие в нужном проекте.
pub fn dispatch_any(ctx: NotesCtx, action: &str, v: &Json) -> Result<String, String> {
    let target = match target_project(ctx, action, v)? {
        Some(path) => Some(path),
        None => routed_by_ref(ctx, action, v)?,
    };
    let Some(path) = target else {
        let several = open_paths(ctx).len() > 1;
        return match action {
            "list" => Ok(list_all(ctx)),
            "search" if several => search_all(ctx, v),
            "agenda" | "tasks" | "log" if several && str_field(v, "board").is_none() && str_field(v, "page").is_none() => {
                overview_all(ctx, action, v)
            }
            _ => dispatch(ctx, action, v).map_err(|e| not_found_hint(ctx, e)),
        };
    };
    if action == "open" {
        // Показать страницу — значит показать и её проект.
        ctx.switch_project(&path);
        return dispatch(ctx, action, v);
    }
    let title = project::project_title(&path);
    let out = ctx
        .with_project(&path, |c| dispatch(c, action, v))
        .ok_or_else(|| format!("project \"{title}\" is not open"))?;
    match out {
        Ok(s) if action == "list" => Ok(s),
        Ok(s) => Ok(format!("project: \"{title}\" · {}\n{s}", path.display())),
        Err(e) => Err(format!("project \"{title}\": {e}")),
    }
}

/// Открытые проекты: показанный первым, дальше в порядке плиток.
fn open_paths(ctx: NotesCtx) -> Vec<PathBuf> {
    let active = ctx.project_path.get_untracked();
    let mut paths: Vec<PathBuf> = ctx.projects.get_untracked().into_iter().map(|p| p.path).collect();
    paths.sort_by_key(|p| *p != active);
    paths
}

fn shown_note(ctx: NotesCtx, path: &Path) -> &'static str {
    if ctx.project_path.get_untracked() == path {
        " · shown in Notes"
    } else {
        ""
    }
}

/// «"A" (путь, shown in Notes), "B" (путь)» — для ошибок.
fn open_list(ctx: NotesCtx) -> String {
    let list: Vec<String> = open_paths(ctx)
        .iter()
        .map(|p| {
            let shown = if shown_note(ctx, p).is_empty() { "" } else { ", shown in Notes" };
            format!("\"{}\" ({}{shown})", project::project_title(p), p.display())
        })
        .collect();
    if list.is_empty() {
        "none".to_string()
    } else {
        list.join(", ")
    }
}

const PROJECTS_HINT: &str = "Several projects are open: list, search, agenda, tasks and log cover all of them; \
     every other action works in the one shown in Notes unless you pass project=\"<title>\" (page and \
     board ids from another project find their project by themselves).\n";

/// `list` без `project`: деревья всех открытых проектов одним ответом и
/// недавно закрытые.
fn list_all(ctx: NotesCtx) -> String {
    let paths = open_paths(ctx);
    let mut out = String::new();
    if paths.len() > 1 {
        out.push_str(&format!("--- {} notes projects are open (a tile each) ---\n", paths.len()));
    }
    for path in &paths {
        let note = if paths.len() > 1 { shown_note(ctx, path) } else { "" };
        if let Some(tree) = ctx.with_project(path, |c| project_tree(c, note)) {
            out.push_str(&tree);
        }
    }
    let mut out = clip_list(out);
    let closed = ctx.recent_closed();
    if !closed.is_empty() {
        out.push_str("--- Closed projects (recently used; project=<path> opens one) ---\n");
        for p in closed {
            out.push_str(&format!("\"{}\" · {}\n", project::project_title(&p), p.display()));
        }
    }
    if paths.len() > 1 {
        out.push_str("---\n");
        out.push_str(PROJECTS_HINT);
    }
    out + LIST_HINT
}

/// `search` без `project` при нескольких открытых: по каждому, с шапкой
/// проекта у тех, где нашлось.
fn search_all(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let query = str_field(v, "query").ok_or("missing \"query\"")?;
    let paths = open_paths(ctx);
    let mut out = String::new();
    for path in &paths {
        let Some(found) = ctx.with_project(path, |c| search_impl(c, v)) else { continue };
        let found = found?;
        if found.starts_with("no pages match") {
            continue;
        }
        out.push_str(&format!(
            "--- Project \"{}\"{} ---\n{found}",
            project::project_title(path),
            shown_note(ctx, path)
        ));
    }
    if out.is_empty() {
        let titles: Vec<String> = paths.iter().map(|p| format!("\"{}\"", project::project_title(p))).collect();
        return Ok(format!("no pages match \"{query}\" in any open project ({})\n", titles.join(", ")));
    }
    out.push_str("---\n");
    out.push_str(PROJECTS_HINT);
    Ok(out)
}

/// Пометки пустого ответа обзорных действий.
const NOTHING_MARKERS: [&str; 3] = ["(no boards, due dates or events yet", "(no cards match)", "(nothing changed in this range)"];

/// `agenda` / `tasks` / `log` без `project`, доски и страницы при
/// нескольких открытых: раздел на каждый проект, где что-то есть, пустые
/// — одной строкой, подсказка по фильтрам — один раз в конце.
fn overview_all(ctx: NotesCtx, action: &str, v: &Json) -> Result<String, String> {
    let mut out = String::new();
    let mut quiet = Vec::new();
    let mut footer = String::new();
    for path in open_paths(ctx) {
        let title = project::project_title(&path);
        let Some(res) = ctx.with_project(&path, |c| dispatch(c, action, v)) else { continue };
        let res = res.map_err(|e| format!("project \"{title}\": {e}"))?;
        let (body, tail) = match res.rfind("\n---\n") {
            Some(i) => res.split_at(i + 1),
            None => (res.as_str(), ""),
        };
        footer = tail.to_string();
        if NOTHING_MARKERS.iter().any(|m| body.contains(m)) {
            quiet.push(format!("\"{title}\""));
            continue;
        }
        out.push_str(&format!("=== Project \"{title}\"{} ===\n{body}", shown_note(ctx, &path)));
    }
    if !quiet.is_empty() {
        out.push_str(&format!("Nothing to show in project{} {}\n", if quiet.len() == 1 { "" } else { "s" }, quiet.join(", ")));
    }
    out.push_str(&footer);
    out.push_str(PROJECTS_HINT);
    Ok(out)
}

/// Проект из аргументов: `project`, а если его нет — файл `.syn` в `path`
/// (так его передала модель). У `attach` в `path` — вкладываемый файл.
fn target_project(ctx: NotesCtx, action: &str, v: &Json) -> Result<Option<PathBuf>, String> {
    let raw = match str_field(v, "project") {
        Some(p) => p,
        None => match str_field(v, "path") {
            Some(p) if is_syn_path(p) && action != "attach" && str_field(v, "op") != Some("attach") => p,
            _ => return Ok(None),
        },
    };
    resolve_project(ctx, raw).map(Some)
}

fn is_syn_path(s: &str) -> bool {
    Path::new(s.trim()).extension().is_some_and(|e| e.eq_ignore_ascii_case("syn"))
}

/// Проект по ссылке агента: название плитки (имя файла без `.syn`, без
/// регистра) либо путь к файлу. Недавно закрытый проект или `.syn` с диска
/// открывается плиткой, не уводя пользователя с показанного.
pub(super) fn resolve_project(ctx: NotesCtx, raw: &str) -> Result<PathBuf, String> {
    let s = raw.trim();
    let active = ctx.project_path.get_untracked();
    if s.is_empty() {
        return Ok(active);
    }
    let looks_like_path = s.contains('/') || s.contains('\\') || s.starts_with('~');
    if looks_like_path {
        let path = project::normalize_path(&project::with_syn_extension(Path::new(s)));
        if ctx.is_open(&path) {
            return Ok(path);
        }
        return ctx.open_project_quietly(&path).map_err(|e| {
            let why = match e {
                ProjectError::Missing(p) => format!("no file {}", p.display()),
                ProjectError::NotNotes(p) => format!("{} is not a notes project", p.display()),
                ProjectError::Io(e) => format!("can't open {}: {e}", path.display()),
                other => other.message(),
            };
            format!("{why}. Open notes projects: {}", open_list(ctx))
        });
    }
    let name = if is_syn_path(s) { &s[..s.len() - 4] } else { s }.trim().to_lowercase();
    let titled = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        paths.into_iter().filter(|p| project::project_title(p).to_lowercase() == name).collect()
    };
    let open = titled(open_paths(ctx));
    match open.len() {
        1 => return Ok(open[0].clone()),
        0 => {}
        _ => {
            let list: Vec<String> = open.iter().map(|p| p.display().to_string()).collect();
            return Err(format!("\"{s}\" matches {} open projects — pass the path: {}", open.len(), list.join(", ")));
        }
    }
    if matches!(name.as_str(), "active" | "current" | "shown") && !active.as_os_str().is_empty() {
        return Ok(active);
    }
    if let Some(closed) = titled(ctx.recent_closed()).into_iter().next() {
        return ctx.open_project_quietly(&closed).map_err(|e| e.message());
    }
    Err(format!("no notes project \"{s}\". Open notes projects: {}", open_list(ctx)))
}

/// Страница или объект другого открытого проекта по id: `list`, `search`
/// и обзоры отдают id всех проектов, и модель зовёт `read {page: id}` или
/// `kanban {board: id}`, не называя проект.
fn routed_by_ref(ctx: NotesCtx, action: &str, v: &Json) -> Result<Option<PathBuf>, String> {
    if matches!(action, "list" | "search") {
        return Ok(None);
    }
    let bare = |raw: &str, prefix: &str| {
        let t = raw.trim();
        t.strip_prefix(prefix).unwrap_or(t).trim().to_string()
    };
    let pick = |id: &str, hits: Vec<PathBuf>| -> Result<Option<PathBuf>, String> {
        match hits.len() {
            0 => Ok(None),
            1 => Ok(hits.into_iter().next()),
            _ => {
                let titles: Vec<String> = hits.iter().map(|p| format!("\"{}\"", project::project_title(p))).collect();
                Err(format!("{id} is in several open projects ({}) — pass project", titles.join(", ")))
            }
        }
    };
    let listed = v.get("pages").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str);
    let tree = ctx.tree.get_untracked();
    for raw in str_field(v, "page").into_iter().chain(listed) {
        let id = bare(raw, "id:");
        if is_page_id(&id) && tree.find(&id).is_none() {
            if let Some(path) = pick(&id, ctx.projects_with_page(&id))? {
                return Ok(Some(path));
            }
        }
    }
    let object_keys: &[&str] = match action {
        "kanban" | "tasks" | "log" => &["board"],
        "gantt" => &["gantt", "chart"],
        "mindmap" => &["map"],
        "calendar" => &["calendar"],
        "chart" => &["chart"],
        _ => &[],
    };
    let kind = match action {
        "tasks" | "log" => "kanban",
        other => other,
    };
    for key in object_keys {
        let Some(raw) = str_field(v, key) else { continue };
        let id = bare(raw, &format!("{kind}:"));
        if is_page_id(&id) && !ctx.has_object(kind, &id) {
            if let Some(path) = pick(&id, ctx.projects_with_object(kind, &id))? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn is_page_id(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Страница не нашлась, а открыт не один проект: сказать, где искали.
fn not_found_hint(ctx: NotesCtx, e: String) -> String {
    if !e.contains("not found") || open_paths(ctx).len() < 2 {
        return e;
    }
    format!(
        "{e} (looked in project \"{}\", the one shown in Notes; open projects: {} — pass project=\"<title>\")",
        ctx.project_title.get_untracked(),
        open_list(ctx)
    )
}
