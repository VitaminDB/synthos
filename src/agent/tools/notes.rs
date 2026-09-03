//! Tool `notes` — полный доступ агента Syn-чата к режиму «Заметки».
//!
//! Агент работает с тем же проектом `.syn` и теми же ручками, что и UI
//! (`NotesCtx`): дерево страниц, markdown страниц, канбан-доски и диаграммы
//! Ганта, вложения. Правки видны в интерфейсе сразу (перестройка по
//! `doc_epoch`/`revision`) и уходят на диск автосейвом; замена текста
//! страницы записывается в историю редактора — Ctrl+Z у пользователя
//! возвращает прежний текст.
//!
//! Действия:
//! - `list` — дерево страниц (id, название, объекты на странице);
//!   `search` — поиск по названиям и тексту; `read` — страница целиком
//!   (markdown, доски и диаграммы на ней, связи).
//! - `create` / `update` / `move` / `delete` / `duplicate` — страницы.
//!   `update` умеет переименовать, сменить иконку и раскладку, заменить
//!   текст целиком (`content`, `mode=replace|append|prepend`) и точечно
//!   (`find`/`replace`).
//! - `open` — показать страницу пользователю; `attach` — файл с диска или
//!   вложение чата → вложение проекта + медиа-блок на странице.
//! - `kanban` / `gantt` — объекты-примитивы: создать на странице, прочитать,
//!   колонки/карточки и задачи/зависимости, удалить (врезки убираются со
//!   страниц, файл — из бандла).
//!
//! Адресация: страница — id (12 hex) либо название (без регистра; при
//! совпадениях — путь «Родитель / Страница» или id); доска/диаграмма — id
//! объекта либо страница, на которой объект один; колонка — id или
//! название; карточка/задача — id или заголовок.
//!
//! **Геометрия свободной раскладки.** Агент видит и правит «плоский»
//! markdown без служебного хвоста ```` ```doc-layout ````: при записи
//! блоки, чей markdown не изменился, получают свои прежние координаты
//! ([`with_geometry`]) — перестановка абзаца агентом не сбивает холст.
//!
//! Все сигналы — main-thread: действие целиком исполняется в
//! `run_on_main_thread`-замыкании, результат уходит через oneshot
//! (паттерн `pipelines`). Формат результата — секционный plain-text.

use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;
use syngui::widgets::input::document_editor::serialize::block_markdown;
use syngui::widgets::input::document_editor::{free, parse_document, serialize_document, BlockKind, DocBlock};

use crate::pages::notes::gantt::calendar::{days_to_iso, parse_days, today_days};
use crate::pages::notes::gantt::model::GanttDoc;
use crate::pages::notes::gantt::GanttHandle;
use crate::pages::notes::kanban::model::{item_id, parse_tags, DropSpot, KanbanCard, KanbanColumn, Priority, PALETTE};
use crate::pages::notes::kanban::KanbanHandle;
use crate::pages::notes::project::PageLayout;
use crate::pages::notes::state::{object_refs, LiveObject, NotesCtx};
use crate::pages::notes::{embeds, media};
use crate::syn_chat::attach::blobs;

use super::executor::ToolError;

/// Главный entrypoint из `executor::execute`.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: Json = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let action = str_field(&v, "action")
        .ok_or(ToolError::MissingField("action"))?
        .to_string();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    run_on_main_thread(move || {
        let ctx = use_context::<NotesCtx>();
        let _ = tx.send(dispatch(ctx, &action, &v));
    });
    rx.await
        .map_err(|e| ToolError::Spawn(e.to_string()))?
        .map_err(ToolError::BadArgs)
}

/// Диспетчер действий; вынесен из `run`, чтобы тесты звали его с
/// собственным `NotesCtx` без main-thread.
pub fn dispatch(ctx: NotesCtx, action: &str, v: &Json) -> Result<String, String> {
    match action {
        "list" => list_impl(ctx),
        "search" => search_impl(ctx, v),
        "read" => read_impl(ctx, v),
        "create" => create_impl(ctx, v),
        "update" => update_impl(ctx, v),
        "move" => move_impl(ctx, v),
        "delete" => delete_impl(ctx, v),
        "duplicate" => duplicate_impl(ctx, v),
        "open" => open_impl(ctx, v),
        "attach" => attach_impl(ctx, v),
        "kanban" => kanban_impl(ctx, v),
        "gantt" => gantt_impl(ctx, v),
        other => Err(format!(
            "unknown action \"{other}\" (list | search | read | create | update | move | delete | \
             duplicate | open | attach | kanban | gantt)"
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Аргументы
// ─────────────────────────────────────────────────────────────────────────────

/// Непустая строка (trim). Числа тоже принимаем строкой — модели шлют id
/// и индексы как попало.
fn str_field<'a>(v: &'a Json, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Строка как есть (возможно пустая — «очистить»); `null` — поля нет.
fn raw_string(v: &Json, key: &str) -> Option<String> {
    match v.get(key)? {
        Json::String(s) => Some(s.clone()),
        Json::Null => None,
        other => Some(other.to_string()),
    }
}

fn bool_field(v: &Json, key: &str) -> Option<bool> {
    match v.get(key)? {
        Json::Bool(b) => Some(*b),
        Json::String(s) => Some(matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        )),
        Json::Number(n) => Some(n.as_i64().is_some_and(|i| i != 0)),
        _ => None,
    }
}

fn usize_field(v: &Json, key: &str) -> Option<usize> {
    match v.get(key)? {
        Json::Number(n) => n.as_u64().map(|x| x as usize),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn f32_field(v: &Json, key: &str) -> Option<f32> {
    match v.get(key)? {
        Json::Number(n) => n.as_f64().map(|x| x as f32),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Список строк: JSON-массив либо строка через запятую / перевод строки.
fn list_field(v: &Json, key: &str) -> Option<Vec<String>> {
    match v.get(key)? {
        Json::Array(a) => Some(
            a.iter()
                .filter_map(|x| match x {
                    Json::String(s) => Some(s.trim().to_string()),
                    Json::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        Json::String(s) => Some(
            s.split([',', '\n'])
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
        ),
        Json::Null => None,
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Адресация
// ─────────────────────────────────────────────────────────────────────────────

/// Путь страницы в дереве: «Родитель / Страница».
fn path_of(ctx: NotesCtx, id: &str) -> String {
    ctx.tree
        .get_untracked()
        .path_of(id)
        .into_iter()
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Строка «page: <id> · "Название" · path: …» — единый заголовок в ответах;
/// по ней же ссылка «Открыть в заметках» в чате находит страницу.
fn page_line(ctx: NotesCtx, id: &str) -> String {
    format!("page: {id} · \"{}\" · path: {}", ctx.title_of(id), path_of(ctx, id))
}

/// Страница по ссылке агента: id, название либо путь «A / B».
fn resolve_page_ref(ctx: NotesCtx, raw: &str) -> Result<String, String> {
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
fn page_arg(ctx: NotesCtx, v: &Json, key: &'static str) -> Result<String, String> {
    let s = str_field(v, key).ok_or_else(|| format!("missing \"{key}\" (page id or title)"))?;
    resolve_page_ref(ctx, s)
}

/// Все объекты вида `kind` в проекте: (id объекта, id страницы).
fn all_objects(ctx: NotesCtx, kind: &str) -> Vec<(String, String)> {
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
fn resolve_object(ctx: NotesCtx, v: &Json, kind: &str, key: &str) -> Result<LiveObject, String> {
    let noun = if kind == "kanban" { "board" } else { "chart" };
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

fn kanban_handle(ctx: NotesCtx, v: &Json) -> Result<(String, KanbanHandle), String> {
    match resolve_object(ctx, v, "kanban", "board")? {
        LiveObject::Kanban { id, handle } => Ok((id, handle)),
        _ => Err("not a kanban board".to_string()),
    }
}

fn gantt_handle(ctx: NotesCtx, v: &Json) -> Result<(String, GanttHandle), String> {
    match resolve_object(ctx, v, "gantt", "chart")? {
        LiveObject::Gantt { id, handle } => Ok((id, handle)),
        _ => Err("not a gantt chart".to_string()),
    }
}

/// Страница, на которой врезан объект (первая из ссылающихся).
fn object_page(ctx: NotesCtx, kind: &str, id: &str) -> Option<String> {
    ctx.pages_referencing(kind, id).into_iter().next()
}

fn object_page_line(ctx: NotesCtx, kind: &str, id: &str) -> String {
    match object_page(ctx, kind, id) {
        Some(pid) => page_line(ctx, &pid),
        None => "page: (not embedded in any page)".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Markdown: плоская форма и геометрия
// ─────────────────────────────────────────────────────────────────────────────

fn strip_geom(block: &mut DocBlock) {
    for k in [free::ATTR_X, free::ATTR_Y, free::ATTR_W, free::ATTR_H] {
        block.attrs.remove(k);
    }
}

/// Markdown страницы без служебной геометрии свободной раскладки — то, что
/// видит и правит агент.
pub fn plain_markdown(md: &str) -> String {
    let mut model = parse_document(md);
    for b in &mut model.blocks {
        strip_geom(b);
    }
    serialize_document(&model)
}

/// Текст страницы от агента + геометрия старых блоков: блок, чей markdown
/// не изменился, остаётся на своём месте холста (совпадение по тексту, по
/// порядку, каждый старый блок — один раз).
pub fn with_geometry(current: &str, new_plain: &str) -> String {
    let old = parse_document(current);
    let old_md: Vec<String> = old.blocks.iter().map(block_markdown).collect();
    let mut used = vec![false; old.blocks.len()];
    let mut fresh = parse_document(new_plain);
    for b in &mut fresh.blocks {
        let key = block_markdown(b);
        let Some(i) = (0..old.blocks.len()).find(|&i| !used[i] && old_md[i] == key) else { continue };
        used[i] = true;
        for k in [free::ATTR_X, free::ATTR_Y, free::ATTR_W, free::ATTR_H] {
            if let Some(val) = old.blocks[i].attrs.get(k) {
                if b.attrs.get(k).is_none() {
                    b.attrs.set(k, val.to_string());
                }
            }
        }
    }
    serialize_document(&fresh)
}

/// Записать новый плоский текст страницы, сохранив геометрию.
fn write_page(ctx: NotesCtx, id: &str, new_plain: &str) -> Result<(), String> {
    let current = ctx.page_markdown(id);
    let merged = with_geometry(&current, new_plain);
    if ctx.set_page_markdown(id, &merged) {
        Ok(())
    } else {
        Err(format!("page {id} not found"))
    }
}

/// Убрать врезки `![[kind:id]]` из markdown (и из вложенных блоков).
fn remove_embed(md: &str, target: &str) -> Option<String> {
    fn walk(blocks: &mut Vec<DocBlock>, target: &str) -> bool {
        let before = blocks.len();
        blocks.retain(|b| !matches!(&b.kind, BlockKind::Embed { target: t } if t.trim() == target));
        let mut removed = blocks.len() != before;
        for b in blocks.iter_mut() {
            if let Some(children) = b.kind.children_mut() {
                removed |= walk(children, target);
            }
        }
        removed
    }
    let mut model = parse_document(md);
    walk(&mut model.blocks, target).then(|| serialize_document(&model))
}

fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

// ─────────────────────────────────────────────────────────────────────────────
// list / search / read
// ─────────────────────────────────────────────────────────────────────────────

fn list_impl(ctx: NotesCtx) -> Result<String, String> {
    let tree = ctx.tree.get_untracked();
    let mut out = String::new();
    out.push_str("--- Project ---\n");
    out.push_str(&format!("file: {}\n", ctx.project_path.get_untracked().display()));
    let active = ctx
        .active
        .get_untracked()
        .filter(|id| tree.find(id).is_some())
        .map(|id| format!("{id} \"{}\"", ctx.title_of(&id)))
        .unwrap_or_else(|| "none".to_string());
    out.push_str(&format!("pages: {} · active: {active}\n", tree.all().len()));
    out.push_str("--- Pages (indent = nesting; objects embedded in the page after ·) ---\n");
    if tree.is_empty() {
        out.push_str("(no pages yet — notes create makes one)\n");
    } else {
        fn walk(ctx: NotesCtx, nodes: &[crate::pages::notes::project::PageNode], depth: usize, out: &mut String) {
            for n in nodes {
                let icon = n.icon.as_deref().filter(|i| !i.is_empty()).map(|i| format!(" {i}")).unwrap_or_default();
                let objects: Vec<String> =
                    object_refs(&ctx.page_markdown(&n.id)).iter().map(|(k, id)| format!(" · {k}:{id}")).collect();
                out.push_str(&format!(
                    "{}{} \"{}\"{icon}{}\n",
                    "  ".repeat(depth),
                    n.id,
                    n.title,
                    objects.concat()
                ));
                walk(ctx, &n.children, depth + 1, out);
            }
        }
        walk(ctx, &tree.roots, 0, &mut out);
    }
    out.push_str(
        "---\nread {page} shows the markdown, boards and charts on the page and its links; \
         update {page, content | find+replace | mode=append} edits it; \
         kanban / gantt {op=create, page} add a board or chart.\n",
    );
    Ok(out)
}

fn search_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let query = str_field(v, "query").ok_or("missing \"query\"")?;
    let q = query.to_lowercase();
    let limit = usize_field(v, "limit").unwrap_or(20).clamp(1, 100);
    let tree = ctx.tree.get_untracked();
    let mut out = String::new();
    let mut hits = 0usize;
    for node in tree.all() {
        let md = plain_markdown(&ctx.page_markdown(&node.id));
        let title_hit = node.title.to_lowercase().contains(&q);
        let lines: Vec<(usize, &str)> = md
            .lines()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .take(4)
            .collect();
        if !title_hit && lines.is_empty() {
            continue;
        }
        hits += 1;
        if hits > limit {
            break;
        }
        out.push_str(&format!("{} \"{}\" · path: {}\n", node.id, node.title, path_of(ctx, &node.id)));
        for (n, l) in lines {
            let mut t = l.trim().to_string();
            if t.chars().count() > 160 {
                t = t.chars().take(160).collect::<String>() + "…";
            }
            out.push_str(&format!("  L{}: {t}\n", n + 1));
        }
    }
    if hits == 0 {
        out.push_str(&format!("no pages match \"{query}\"\n"));
    } else if hits > limit {
        out.push_str(&format!("… more than {limit} pages match — narrow the query\n"));
    }
    Ok(out)
}

fn read_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let tree = ctx.tree.get_untracked();
    let node = tree.find(&id).ok_or("page vanished")?;
    let md = plain_markdown(&ctx.page_markdown(&id));
    let mut out = String::new();
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    out.push_str(&format!(
        "icon: {} · layout: {} · {} words · children: {}\n",
        node.icon.as_deref().unwrap_or("none"),
        if node.layout.free { "free" } else { "flow" },
        count_words(&md),
        node.children.len()
    ));
    out.push_str("--- Markdown ---\n");
    if md.trim().is_empty() {
        out.push_str("(empty page)\n");
    } else {
        out.push_str(&md);
        if !md.ends_with('\n') {
            out.push('\n');
        }
    }
    let objects = object_refs(&md);
    if !objects.is_empty() {
        out.push_str("--- Objects on the page ---\n");
        for (kind, oid) in &objects {
            match ctx.object(kind, oid) {
                Some(LiveObject::Kanban { handle, .. }) => out.push_str(&board_text(oid, &handle, false)),
                Some(LiveObject::Gantt { handle, .. }) => out.push_str(&chart_text(oid, &handle)),
                None => out.push_str(&format!("{kind}:{oid} · (file missing)\n")),
            }
        }
    }
    let index = ctx.index.get_untracked();
    let outgoing = index.outgoing_of(&id);
    let backlinks = index.backlinks_of(&id);
    if !outgoing.is_empty() || !backlinks.is_empty() {
        out.push_str("--- Links ---\n");
        let fmt = |ids: &[String]| {
            ids.iter().map(|i| format!("\"{}\" ({i})", ctx.title_of(i))).collect::<Vec<_>>().join(", ")
        };
        if !outgoing.is_empty() {
            out.push_str(&format!("outgoing: {}\n", fmt(&outgoing)));
        }
        if !backlinks.is_empty() {
            out.push_str(&format!("backlinks: {}\n", fmt(&backlinks)));
        }
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
// Страницы: create / update / move / delete / duplicate / open / attach
// ─────────────────────────────────────────────────────────────────────────────

fn parse_parent(ctx: NotesCtx, v: &Json) -> Result<Option<String>, String> {
    match str_field(v, "parent") {
        None => Ok(None),
        Some(p) if matches!(p.to_ascii_lowercase().as_str(), "root" | "/" | "none" | "null") => Ok(None),
        Some(p) => resolve_page_ref(ctx, p).map(Some),
    }
}

fn create_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let title = str_field(v, "title").map(str::to_string).unwrap_or_else(|| tr!("notes.untitled"));
    let parent = parse_parent(ctx, v)?;
    let index = usize_field(v, "index");
    let id = ctx.insert_page(parent.as_deref(), index, &title, false);
    let mut out = String::from("created\n");
    if let Some(icon) = str_field(v, "icon") {
        ctx.set_icon(&id, Some(icon.to_string()));
    }
    if let Some(layout) = str_field(v, "layout") {
        set_layout(ctx, &id, layout)?;
    }
    if let Some(content) = raw_string(v, "content") {
        write_page(ctx, &id, &content)?;
        out.push_str(&format!("content: {} words\n", count_words(&content)));
    }
    if bool_field(v, "open").unwrap_or(false) {
        show_page(ctx, Some(&id));
    }
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
}

fn set_layout(ctx: NotesCtx, id: &str, layout: &str) -> Result<(), String> {
    let mut l: PageLayout = ctx.page_layout(id);
    match layout.to_ascii_lowercase().as_str() {
        "free" | "canvas" => l.free = true,
        "flow" | "document" | "column" => l.free = false,
        other => return Err(format!("unknown layout \"{other}\" (free | flow)")),
    }
    ctx.set_page_layout(id, l);
    Ok(())
}

fn update_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let mut changes: Vec<String> = Vec::new();

    if let Some(title) = str_field(v, "title") {
        let old = ctx.title_of(&id);
        ctx.rename_page(&id, title);
        changes.push(format!("renamed \"{old}\" → \"{}\"", ctx.title_of(&id)));
    }
    if let Some(icon) = raw_string(v, "icon") {
        let icon = icon.trim().to_string();
        let cleared = icon.is_empty() || matches!(icon.to_ascii_lowercase().as_str(), "none" | "null");
        ctx.set_icon(&id, (!cleared).then_some(icon.clone()));
        changes.push(if cleared { "icon cleared".to_string() } else { format!("icon set to {icon}") });
    }
    if let Some(layout) = str_field(v, "layout") {
        set_layout(ctx, &id, layout)?;
        changes.push(format!("layout: {layout}"));
    }

    let mode = str_field(v, "mode").map(|m| m.to_ascii_lowercase()).unwrap_or_else(|| "replace".to_string());
    if let Some(content) = raw_string(v, "content") {
        let current = plain_markdown(&ctx.page_markdown(&id));
        let new_plain = match mode.as_str() {
            "replace" => content.clone(),
            "append" => join_blocks(&current, &content),
            "prepend" => join_blocks(&content, &current),
            other => return Err(format!("unknown mode \"{other}\" (replace | append | prepend)")),
        };
        write_page(ctx, &id, &new_plain)?;
        changes.push(format!("content {mode}: {} words now", count_words(&new_plain)));
    }

    if let Some(find) = raw_string(v, "find") {
        if find.is_empty() {
            return Err("\"find\" is empty".to_string());
        }
        let replace = raw_string(v, "replace").unwrap_or_default();
        let current = plain_markdown(&ctx.page_markdown(&id));
        let n = current.matches(&find).count();
        if n == 0 {
            return Err(
                "find text not found on the page — read the page and copy the fragment exactly \
                 (the page is compared as markdown, not as rendered text)"
                    .to_string(),
            );
        }
        let all = bool_field(v, "all").unwrap_or(false);
        if n > 1 && !all {
            return Err(format!(
                "find text occurs {n} times — pass all=true to replace every occurrence, or a \
                 longer unique fragment"
            ));
        }
        let new_plain = if all { current.replace(&find, &replace) } else { current.replacen(&find, &replace, 1) };
        write_page(ctx, &id, &new_plain)?;
        changes.push(format!("replaced {n} occurrence{}", if n == 1 { "" } else { "s" }));
    }

    if changes.is_empty() {
        return Err(
            "nothing to update: pass title, icon, layout, content (+mode) or find/replace".to_string()
        );
    }
    let mut out = changes.join("\n");
    out.push('\n');
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
}

/// Два markdown-фрагмента через пустую строку.
fn join_blocks(a: &str, b: &str) -> String {
    let a = a.trim_end();
    let b = b.trim_start();
    match (a.is_empty(), b.is_empty()) {
        (true, _) => format!("{b}\n"),
        (_, true) => format!("{a}\n"),
        _ => format!("{a}\n\n{b}\n"),
    }
}

fn move_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let parent = parse_parent(ctx, v)?;
    let index = usize_field(v, "index");
    if !ctx.move_page(&id, parent.as_deref(), index) {
        return Err("move failed: a page can't be moved into its own subtree".to_string());
    }
    Ok(format!("moved\n{}\n", page_line(ctx, &id)))
}

fn delete_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let tree = ctx.tree.get_untracked();
    let doomed = tree.subtree_ids(&id);
    let objects: usize = doomed.iter().map(|p| object_refs(&ctx.page_markdown(p)).len()).sum();
    let line = page_line(ctx, &id);
    drop(tree);
    ctx.delete_page(&id);
    Ok(format!(
        "deleted {} page{} and {objects} embedded object{}\n{line}\n",
        doomed.len(),
        if doomed.len() == 1 { "" } else { "s" },
        if objects == 1 { "" } else { "s" }
    ))
}

fn duplicate_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let copy = ctx.duplicate_page_with(&id, false).ok_or("page vanished")?;
    Ok(format!("duplicated\n{}\n", page_line(ctx, &copy)))
}

/// Показать страницу пользователю: плитка проекта, активная страница,
/// маршрут «Заметки».
fn show_page(ctx: NotesCtx, id: Option<&str>) {
    ctx.open_tile();
    if let Some(id) = id {
        ctx.activate(id);
    }
    crate::rail::navigate("notes");
}

fn open_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = match str_field(v, "page") {
        Some(p) => Some(resolve_page_ref(ctx, p)?),
        None => None,
    };
    show_page(ctx, id.as_deref());
    Ok(match id {
        Some(id) => format!("opened\n{}\n", page_line(ctx, &id)),
        None => "opened Notes\n".to_string(),
    })
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    PathBuf::from(p)
}

fn attach_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let (bytes, ext, default_caption): (Vec<u8>, String, String) = if let Some(p) = str_field(v, "path") {
        let path = expand_home(p);
        let bytes = std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        (bytes, ext, stem)
    } else if let Some(r) = str_field(v, "attachment") {
        let r = r.strip_prefix("attachment:").unwrap_or(r);
        let a = super::pipelines::find_attachment(r)
            .ok_or_else(|| format!("chat attachment \"{r}\" not found (name, sha prefix or last)"))?;
        let path = blobs::blob_path(&a.sha256, &a.ext);
        let bytes = std::fs::read(&path).map_err(|e| format!("can't read attachment blob: {e}"))?;
        let ext = if a.ext.is_empty() {
            Path::new(&a.original_name).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            a.ext.clone()
        };
        let stem = Path::new(&a.original_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        (bytes, ext, stem)
    } else {
        return Err("pass \"path\" (file on disk) or \"attachment\" (chat attachment: name | sha | last)".to_string());
    };
    let size = bytes.len();
    let url = media::ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
    let caption = str_field(v, "caption").map(str::to_string).unwrap_or(default_caption);
    let caption = caption.replace(['[', ']', '\n'], " ");
    let block = format!("![{caption}]({url})");
    let current = plain_markdown(&ctx.page_markdown(&id));
    write_page(ctx, &id, &join_blocks(&current, &block))?;
    Ok(format!("attached {url} ({size} bytes) as a media block at the end of the page\n{}\n", page_line(ctx, &id)))
}

// ─────────────────────────────────────────────────────────────────────────────
// Kanban
// ─────────────────────────────────────────────────────────────────────────────

/// Цвет метки: `#rrggbb`, имя из палитры либо пусто/`none`.
fn parse_color(s: &str) -> Result<String, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(String::new());
    }
    let named = match lower.as_str() {
        "gray" | "grey" => Some(PALETTE[0]),
        "orange" | "yellow" => Some(PALETTE[1]),
        "green" => Some(PALETTE[2]),
        "blue" => Some(PALETTE[3]),
        "purple" | "violet" => Some(PALETTE[4]),
        "red" => Some(PALETTE[5]),
        "teal" | "cyan" => Some(PALETTE[6]),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c.to_string());
    }
    let hex = t.strip_prefix('#').unwrap_or(t);
    if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let full: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_string()
        };
        return Ok(format!("#{}", full.to_uppercase()));
    }
    Err(format!(
        "bad color \"{s}\" — use #rrggbb or gray | orange | green | blue | purple | red | teal | none"
    ))
}

fn parse_priority(s: &str) -> Result<Option<Priority>, String> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    Priority::parse(&lower)
        .map(Some)
        .ok_or_else(|| format!("bad priority \"{s}\" (low | medium | high | urgent | none)"))
}

fn parse_due(s: &str) -> Result<Option<String>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    if lower == "today" {
        return Ok(Some(days_to_iso(today_days())));
    }
    if lower == "tomorrow" {
        return Ok(Some(days_to_iso(today_days() + 1)));
    }
    let days = parse_days(t).ok_or_else(|| format!("bad date \"{s}\" (yyyy-mm-dd)"))?;
    Ok(Some(days_to_iso(days)))
}

/// Колонка по id, названию (без регистра) или номеру с единицы.
fn resolve_column(handle: &KanbanHandle, s: &str) -> Result<KanbanColumn, String> {
    let doc = handle.lock();
    let key = s.trim().to_lowercase();
    if let Some(c) = doc.columns.iter().find(|c| c.id == s.trim()) {
        return Ok(c.clone());
    }
    let by_name: Vec<&KanbanColumn> = doc.columns.iter().filter(|c| c.name.trim().to_lowercase() == key).collect();
    if by_name.len() == 1 {
        return Ok(by_name[0].clone());
    }
    if let Ok(n) = key.parse::<usize>() {
        if (1..=doc.columns.len()).contains(&n) {
            return Ok(doc.columns[n - 1].clone());
        }
    }
    Err(format!(
        "column \"{s}\" not found — columns: {}",
        doc.columns.iter().map(|c| format!("\"{}\" ({})", c.name, c.id)).collect::<Vec<_>>().join(", ")
    ))
}

/// Карточка по id либо заголовку (без регистра, единственная).
fn resolve_card(handle: &KanbanHandle, s: &str) -> Result<KanbanCard, String> {
    let doc = handle.lock();
    if let Some(c) = doc.card(s.trim()) {
        return Ok(c.clone());
    }
    let key = s.trim().to_lowercase();
    let hits: Vec<&KanbanCard> = doc.cards.iter().filter(|c| c.title.trim().to_lowercase() == key).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!("card \"{s}\" not found — ids and titles are in kanban op=read")),
        n => Err(format!(
            "{n} cards are titled \"{s}\" — use the id: {}",
            hits.iter().map(|c| c.id.clone()).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn card_line(c: &KanbanCard) -> String {
    let mut s = format!("{} \"{}\"", c.id, c.title);
    if let Some(p) = c.priority {
        s.push_str(&format!(" · priority: {}", p.key()));
    }
    if !c.tags.is_empty() {
        s.push_str(&format!(" · tags: {}", c.tags.join(", ")));
    }
    if let Some(d) = &c.due {
        s.push_str(&format!(" · due: {d}"));
    }
    if let Some((done, total)) = c.checklist() {
        s.push_str(&format!(" · checklist {done}/{total}"));
    }
    s
}

/// Текст доски: шапка, колонки, карточки по колонкам; `full` — с
/// markdown-содержимым карточек.
fn board_text(id: &str, handle: &KanbanHandle, full: bool) -> String {
    let doc = handle.lock();
    let mut out = String::new();
    out.push_str(&format!(
        "kanban:{id} · columns: {} · cards: {}{}\n",
        doc.columns.len(),
        doc.cards.len(),
        if full {
            format!(
                " · style: column_width={} counts={}",
                doc.style.column_width,
                if doc.style.show_counts { "on" } else { "off" }
            )
        } else {
            String::new()
        }
    ));
    for col in &doc.columns {
        let cards = doc.cards_of(&col.id);
        out.push_str(&format!(
            "  column {} \"{}\"{}{} · {} card{}\n",
            col.id,
            col.name,
            if col.color.is_empty() { String::new() } else { format!(" · color {}", col.color) },
            col.width.map(|w| format!(" · width {w}")).unwrap_or_default(),
            cards.len(),
            if cards.len() == 1 { "" } else { "s" }
        ));
        for c in cards {
            out.push_str(&format!("    {}\n", card_line(c)));
            if full && !c.md.trim().is_empty() {
                for l in c.md.trim_end().lines() {
                    out.push_str(&format!("      | {l}\n"));
                }
            }
        }
    }
    out
}

fn kanban_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | add_column | update_column | delete_column | add_card | \
         update_card | move_card | delete_card | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let id = ctx.create_object("kanban").ok_or("failed to create the board")?;
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &id) else {
                return Err("board vanished".to_string());
            };
            if let Some(names) = list_field(v, "columns").filter(|c| !c.is_empty()) {
                handle.edit(|doc| {
                    doc.columns = names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| KanbanColumn {
                            id: item_id("c"),
                            name: name.clone(),
                            color: PALETTE[i % PALETTE.len()].to_string(),
                            width: None,
                        })
                        .collect();
                });
            }
            let embed = format!("![[kanban:{id}]]{{h={}}}", embeds::DEFAULT_OBJECT_H as i64);
            let block = match str_field(v, "title") {
                Some(t) => format!("### {t}\n\n{embed}"),
                None => embed,
            };
            let current = plain_markdown(&ctx.page_markdown(&pid));
            write_page(ctx, &pid, &join_blocks(&current, &block))?;
            Ok(format!("created board at the end of the page\n{}{}\n", board_text(&id, &handle, false), page_line(ctx, &pid)))
        }
        "read" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            Ok(format!("{}{}\n", board_text(&id, &handle, true), object_page_line(ctx, "kanban", &id)))
        }
        "add_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let name = str_field(v, "name").ok_or("missing \"name\"")?;
            let cid = handle.add_column(name);
            if let Some(c) = str_field(v, "color") {
                let color = parse_color(c)?;
                handle.edit(|doc| {
                    if let Some(col) = doc.columns.iter_mut().find(|c| c.id == cid) {
                        col.color = color;
                    }
                });
            }
            if let Some(w) = f32_field(v, "width") {
                handle.set_column_width(&cid, (w > 0.0).then_some(w));
            }
            Ok(format!("added column {cid} \"{name}\"\n{}", board_text(&id, &handle, false)))
        }
        "update_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let col = resolve_column(&handle, str_field(v, "column").ok_or("missing \"column\"")?)?;
            let mut changes = Vec::new();
            if let Some(name) = str_field(v, "name") {
                handle.rename_column(&col.id, name);
                changes.push(format!("renamed to \"{name}\""));
            }
            if let Some(c) = str_field(v, "color") {
                let color = parse_color(c)?;
                let cid = col.id.clone();
                handle.edit(|doc| {
                    if let Some(col) = doc.columns.iter_mut().find(|c| c.id == cid) {
                        col.color = color;
                    }
                });
                changes.push(format!("color {c}"));
            }
            if let Some(w) = f32_field(v, "width") {
                handle.set_column_width(&col.id, (w > 0.0).then_some(w));
                changes.push(format!("width {w}"));
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, color or width".to_string());
            }
            Ok(format!("column {}: {}\n{}", col.id, changes.join(", "), board_text(&id, &handle, false)))
        }
        "delete_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let col = resolve_column(&handle, str_field(v, "column").ok_or("missing \"column\"")?)?;
            handle.delete_column(&col.id);
            Ok(format!(
                "deleted column \"{}\" (its cards moved to the neighbouring column)\n{}",
                col.name,
                board_text(&id, &handle, false)
            ))
        }
        "add_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let column = match str_field(v, "column") {
                Some(c) => resolve_column(&handle, c)?,
                None => handle.lock().columns.first().cloned().ok_or("the board has no columns")?,
            };
            let title = str_field(v, "title").ok_or("missing \"title\"")?;
            let mut card = KanbanCard::new(item_id("k"), column.id.clone());
            card.title = title.to_string();
            card.md = raw_string(v, "md").unwrap_or_default().trim().to_string();
            if let Some(p) = str_field(v, "priority") {
                card.priority = parse_priority(p)?;
            }
            if let Some(tags) = list_field(v, "tags") {
                card.tags = parse_tags(&tags.join(","));
            }
            if let Some(d) = str_field(v, "due") {
                card.due = parse_due(d)?;
            }
            let before = match str_field(v, "before") {
                Some(b) => Some(resolve_card(&handle, b)?),
                None => None,
            };
            let cid = card.id.clone();
            let spot = match before {
                Some(b) if b.column == column.id => DropSpot::before(&column.id, &b.id),
                _ => DropSpot::end(&column.id),
            };
            handle.insert_card(card, &spot);
            let line = handle.card(&cid).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("added card to \"{}\": {line}\n{}", column.name, board_text(&id, &handle, false)))
        }
        "update_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let mut changes = Vec::new();
            if let Some(t) = str_field(v, "title") {
                handle.set_card_title(&card.id, t);
                changes.push("title".to_string());
            }
            if let Some(md) = raw_string(v, "md") {
                let md = md.trim().to_string();
                let cid = card.id.clone();
                handle.edit(|doc| {
                    if let Some(c) = doc.cards.iter_mut().find(|c| c.id == cid) {
                        c.md = md;
                    }
                });
                changes.push("md".to_string());
            }
            if let Some(p) = raw_string(v, "priority") {
                handle.set_priority(&card.id, parse_priority(&p)?);
                changes.push("priority".to_string());
            }
            if let Some(tags) = list_field(v, "tags") {
                handle.set_tags(&card.id, parse_tags(&tags.join(",")));
                changes.push("tags".to_string());
            }
            if let Some(d) = raw_string(v, "due") {
                handle.set_due(&card.id, parse_due(&d)?);
                changes.push("due".to_string());
            }
            if let Some(c) = str_field(v, "column") {
                let col = resolve_column(&handle, c)?;
                handle.move_card(&card.id, &DropSpot::end(&col.id));
                changes.push(format!("moved to \"{}\"", col.name));
            }
            if changes.is_empty() {
                return Err("nothing to update: pass title, md, priority, tags, due or column".to_string());
            }
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("updated {}: {line}\n{}", changes.join(", "), board_text(&id, &handle, false)))
        }
        "move_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            // Другая доска: карточка изымается и кладётся туда.
            if let Some(to) = str_field(v, "to_board") {
                let to_v = serde_json::json!({ "board": to });
                let (to_id, to_handle) = kanban_handle(ctx, &to_v)?;
                if to_id != id {
                    let column = match str_field(v, "column") {
                        Some(c) => resolve_column(&to_handle, c)?,
                        None => to_handle.lock().columns.first().cloned().ok_or("the target board has no columns")?,
                    };
                    let taken = handle.take_card(&card.id).ok_or("card vanished")?;
                    to_handle.insert_card(taken, &DropSpot::end(&column.id));
                    return Ok(format!(
                        "moved card \"{}\" to board kanban:{to_id}, column \"{}\"\n{}",
                        card.title,
                        column.name,
                        board_text(&to_id, &to_handle, false)
                    ));
                }
            }
            let column = match str_field(v, "column") {
                Some(c) => resolve_column(&handle, c)?,
                None => resolve_column(&handle, &card.column)?,
            };
            let spot = match str_field(v, "before") {
                Some(b) => {
                    let b = resolve_card(&handle, b)?;
                    if b.column != column.id {
                        return Err(format!("card \"{}\" is not in column \"{}\"", b.title, column.name));
                    }
                    DropSpot::before(&column.id, &b.id)
                }
                None => DropSpot::end(&column.id),
            };
            handle.move_card(&card.id, &spot);
            Ok(format!("moved card \"{}\" to \"{}\"\n{}", card.title, column.name, board_text(&id, &handle, false)))
        }
        "delete_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            handle.delete_card(&card.id);
            Ok(format!("deleted card \"{}\"\n{}", card.title, board_text(&id, &handle, false)))
        }
        "delete" => {
            let (id, _) = kanban_handle(ctx, v)?;
            delete_object(ctx, "kanban", &id)
        }
        other => Err(format!(
            "unknown kanban op \"{other}\" (create | read | add_column | update_column | delete_column | \
             add_card | update_card | move_card | delete_card | delete)"
        )),
    }
}

/// Удалить объект: врезки — со страниц, файл — из бандла.
fn delete_object(ctx: NotesCtx, kind: &str, id: &str) -> Result<String, String> {
    let target = format!("{kind}:{id}");
    let pages = ctx.pages_referencing(kind, id);
    for pid in &pages {
        if let Some(md) = remove_embed(&ctx.page_markdown(pid), &target) {
            ctx.set_page_markdown(pid, &md);
        }
    }
    ctx.delete_object(kind, id);
    Ok(format!(
        "deleted {target} (embeds removed from {} page{})\n{}",
        pages.len(),
        if pages.len() == 1 { "" } else { "s" },
        pages.first().map(|p| page_line(ctx, p) + "\n").unwrap_or_default()
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Gantt
// ─────────────────────────────────────────────────────────────────────────────

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

fn chart_text(id: &str, handle: &GanttHandle) -> String {
    let doc = handle.lock();
    let mut out = format!(
        "gantt:{id} · tasks: {} · deps: {} · zoom: {} px/day\n",
        doc.tasks.len(),
        doc.deps.len(),
        doc.zoom
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

fn parse_date_field(v: &Json, key: &str) -> Result<Option<i64>, String> {
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

fn gantt_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op")
        .ok_or("missing \"op\" (create | read | add_task | update_task | delete_task | add_dep | delete_dep | delete)")?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let id = ctx.create_object("gantt").ok_or("failed to create the chart")?;
            let embed = format!("![[gantt:{id}]]{{h={}}}", embeds::DEFAULT_OBJECT_H as i64);
            let block = match str_field(v, "title") {
                Some(t) => format!("### {t}\n\n{embed}"),
                None => embed,
            };
            let current = plain_markdown(&ctx.page_markdown(&pid));
            write_page(ctx, &pid, &join_blocks(&current, &block))?;
            Ok(format!("created chart gantt:{id} at the end of the page\n{}\n", page_line(ctx, &pid)))
        }
        "read" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            Ok(format!("{}{}\n", chart_text(&id, &handle), object_page_line(ctx, "gantt", &id)))
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
            Ok(format!("added task {tid} \"{name}\"\n{}", chart_text(&id, &handle)))
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
            Ok(format!("updated task {}: {}\n{}", task.id, changes.join(", "), chart_text(&id, &handle)))
        }
        "delete_task" => {
            let (id, handle) = gantt_handle(ctx, v)?;
            let task = resolve_task(&handle, str_field(v, "task").ok_or("missing \"task\"")?)?;
            handle.delete_task(&task.id);
            Ok(format!("deleted task \"{}\"\n{}", task.name, chart_text(&id, &handle)))
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
                Ok(format!("dependency \"{}\" → \"{}\"\n{}", from.name, to.name, chart_text(&id, &handle)))
            } else {
                handle.delete_dep(&from.id, &to.id);
                Ok(format!("removed dependency \"{}\" → \"{}\"\n{}", from.name, to.name, chart_text(&id, &handle)))
            }
        }
        "delete" => {
            let (id, _) = gantt_handle(ctx, v)?;
            delete_object(ctx, "gantt", &id)
        }
        other => Err(format!(
            "unknown gantt op \"{other}\" (create | read | add_task | update_task | delete_task | add_dep | \
             delete_dep | delete)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::pages::notes::project;

    /// Контекст заметок над временным проектом.
    fn ctx() -> NotesCtx {
        let dir = std::env::temp_dir().join(format!("synthos-notes-tool-{}", project::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = AppConfig {
            notes_project_path: dir.join("p.syn").display().to_string(),
            notes_vault_path: dir.join("no-vault").display().to_string(),
            ..AppConfig::default()
        };
        NotesCtx::new_or_restore(&cfg)
    }

    fn call(ctx: NotesCtx, action: &str, args: serde_json::Value) -> String {
        dispatch(ctx, action, &args).unwrap_or_else(|e| panic!("{action} {args}: {e}"))
    }

    fn page_id(out: &str) -> String {
        let line = out.lines().find(|l| l.starts_with("page: ")).expect("page line");
        line[6..18].to_string()
    }

    /// Агент видит страницу без служебного хвоста, а после его правки
    /// нетронутые блоки сохраняют координаты холста.
    #[test]
    fn plain_markdown_hides_geometry_and_write_keeps_it() {
        let md = "# Заголовок\n\nАбзац\n\n- пункт\n\n```doc-layout\n0 40 300 200\n1 400 60 200\n```\n";
        let plain = plain_markdown(md);
        assert!(!plain.contains("doc-layout"), "{plain}");
        assert!(plain.contains("# Заголовок"));

        // Абзац переписан, заголовок и пункт остались — координаты у них на месте.
        let edited = plain.replace("Абзац", "Новый абзац");
        let merged = with_geometry(md, &edited);
        assert!(merged.contains("Новый абзац"), "{merged}");
        assert!(merged.contains("0 {w=200 x=40 y=300}"), "заголовок потерял координаты: {merged}");
        assert!(!merged.contains("x=400"), "переписанный абзац не должен наследовать координаты: {merged}");
        // Вставка блока перед заголовком не сбивает его координаты.
        let shifted = format!("Преамбула\n\n{plain}");
        let merged = with_geometry(md, &shifted);
        assert!(merged.contains("1 {w=200 x=40 y=300}"), "{merged}");
        assert!(merged.contains("2 {w=200 x=400 y=60}"), "{merged}");
    }

    #[test]
    fn colors_priorities_and_dates_parse() {
        assert_eq!(parse_color("red").unwrap(), PALETTE[5]);
        assert_eq!(parse_color("#abc").unwrap(), "#AABBCC");
        assert_eq!(parse_color("none").unwrap(), "");
        assert!(parse_color("plaid").is_err());
        assert_eq!(parse_priority("High").unwrap(), Some(Priority::High));
        assert_eq!(parse_priority("none").unwrap(), None);
        assert!(parse_priority("meh").is_err());
        assert_eq!(parse_due("2026-09-10").unwrap().as_deref(), Some("2026-09-10"));
        assert!(parse_due("вчера").is_err());
        assert_eq!(join_blocks("а\n", "\nб"), "а\n\nб\n");
        assert_eq!(join_blocks("", "б"), "б\n");
        let v = serde_json::json!({"tags": "ui, дизайн", "n": "3", "b": "yes"});
        assert_eq!(list_field(&v, "tags").unwrap(), vec!["ui", "дизайн"]);
        assert_eq!(usize_field(&v, "n"), Some(3));
        assert_eq!(bool_field(&v, "b"), Some(true));
    }

    #[test]
    fn pages_lifecycle_through_the_tool() {
        let ctx = ctx();
        let out = call(ctx, "create", serde_json::json!({"title": "Проект", "content": "# Проект\n\nВступление\n"}));
        let root = page_id(&out);
        let out = call(ctx, "create", serde_json::json!({"title": "Идеи", "parent": "Проект", "icon": "💡"}));
        let child = page_id(&out);
        assert_eq!(ctx.tree.get_untracked().parent_of(&child).as_deref(), Some(root.as_str()));

        // Адресация: id, название, путь; неоднозначность — ошибка.
        assert_eq!(resolve_page_ref(ctx, "идеи").unwrap(), child);
        assert_eq!(resolve_page_ref(ctx, "Проект / Идеи").unwrap(), child);
        call(ctx, "create", serde_json::json!({"title": "Идеи"}));
        assert!(resolve_page_ref(ctx, "Идеи").unwrap_err().contains("matches 2"));
        assert_eq!(resolve_page_ref(ctx, "Проект/Идеи").unwrap(), child);

        // list и read.
        let listed = call(ctx, "list", serde_json::json!({}));
        assert!(listed.contains(&format!("  {child} \"Идеи\" 💡")), "{listed}");
        let read = call(ctx, "read", serde_json::json!({"page": &root}));
        assert!(read.contains("Вступление"), "{read}");
        assert!(read.contains("children: 1"), "{read}");

        // update: append, find/replace, переименование, раскладка.
        call(ctx, "update", serde_json::json!({"page": &root, "content": "- [ ] задача", "mode": "append"}));
        let md = ctx.page_markdown(&root);
        assert!(md.contains("Вступление\n\n- [ ] задача"), "{md}");
        let err = dispatch(ctx, "update", &serde_json::json!({"page": &root, "find": "нет такого", "replace": "x"}))
            .unwrap_err();
        assert!(err.contains("not found"), "{err}");
        call(ctx, "update", serde_json::json!({"page": &root, "find": "- [ ] задача", "replace": "- [x] задача", "title": "План", "layout": "flow"}));
        assert!(ctx.page_markdown(&root).contains("- [x] задача"));
        assert_eq!(ctx.title_of(&root), "План");
        assert!(!ctx.page_layout(&root).free);
        assert!(ctx.page(&root).unwrap().handle.history_state().get_untracked().0, "правка агента должна отменяться");

        // search, duplicate, move, delete.
        let found = call(ctx, "search", serde_json::json!({"query": "ЗАДАЧА"}));
        assert!(found.contains(&root), "{found}");
        let copy = page_id(&call(ctx, "duplicate", serde_json::json!({"page": &root})));
        assert_ne!(copy, root);
        assert_eq!(ctx.title_of(&copy), "План (копия)");
        call(ctx, "move", serde_json::json!({"page": &child, "parent": "root", "index": 0}));
        assert_eq!(ctx.tree.get_untracked().parent_of(&child), None);
        assert!(dispatch(ctx, "move", &serde_json::json!({"page": &root, "parent": &copy})).is_ok());
        assert!(dispatch(ctx, "move", &serde_json::json!({"page": &copy, "parent": &root})).is_err());
        // Копия сделана до переноса «Идей» в корень — в ней своя копия
        // «Идей», плюс перенесённый внутрь оригинал: три страницы.
        let out = call(ctx, "delete", serde_json::json!({"page": &copy}));
        assert!(out.starts_with("deleted 3 pages"), "{out}");
        assert!(ctx.tree.get_untracked().find(&root).is_none());
    }

    #[test]
    fn kanban_and_gantt_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Спринт"})));
        let out = call(
            ctx,
            "kanban",
            serde_json::json!({"op": "create", "page": "Спринт", "title": "Доска", "columns": ["Бэклог", "В работе", "Готово"]}),
        );
        assert!(out.contains("column"), "{out}");
        let md = ctx.page_markdown(&page);
        assert!(md.contains("### Доска") && md.contains("![[kanban:"), "{md}");
        let (_, board_id) = object_refs(&md).into_iter().next().unwrap();

        // Доска находится и по странице, и по id; карточки — по заголовку.
        let out = call(
            ctx,
            "kanban",
            serde_json::json!({"op": "add_card", "page": "Спринт", "column": "Бэклог", "title": "Импорт", "md": "- [ ] парсер\n- [x] тесты", "priority": "high", "tags": "core, io", "due": "2026-09-10"}),
        );
        assert!(out.contains("priority: high") && out.contains("checklist 1/2") && out.contains("due: 2026-09-10"), "{out}");
        call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board_id, "column": 1, "title": "Экспорт"}));
        call(ctx, "kanban", serde_json::json!({"op": "move_card", "board": &board_id, "card": "Экспорт", "column": "Готово"}));
        call(ctx, "kanban", serde_json::json!({"op": "update_card", "board": &board_id, "card": "Импорт", "priority": "none", "column": "В работе"}));
        let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &board_id) else { panic!() };
        {
            let doc = handle.lock();
            assert_eq!(doc.cards.len(), 2);
            let import = doc.cards.iter().find(|c| c.title == "Импорт").unwrap();
            assert_eq!(import.priority, None);
            assert_eq!(doc.columns.iter().find(|c| c.id == import.column).unwrap().name, "В работе");
            assert_eq!(doc.cards_of(&doc.columns[2].id).len(), 1);
        }
        let out = call(ctx, "kanban", serde_json::json!({"op": "read", "board": &board_id}));
        assert!(out.contains("| - [ ] парсер"), "{out}");
        assert!(out.contains(&format!("page: {page}")), "{out}");
        call(ctx, "kanban", serde_json::json!({"op": "add_column", "board": &board_id, "name": "Ревью", "color": "purple"}));
        call(ctx, "kanban", serde_json::json!({"op": "delete_column", "board": &board_id, "column": "Готово"}));
        assert_eq!(handle.lock().columns.len(), 3);
        assert_eq!(handle.lock().cards.len(), 2, "карточки удалённой колонки переезжают");
        call(ctx, "kanban", serde_json::json!({"op": "delete_card", "board": &board_id, "card": "Экспорт"}));
        assert_eq!(handle.lock().cards.len(), 1);

        // Диаграмма.
        call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
        let out = call(ctx, "gantt", serde_json::json!({"op": "add_task", "page": &page, "name": "Дизайн", "start": "2026-09-01", "end": "2026-09-03"}));
        assert!(out.contains("2026-09-01 → 2026-09-03 (3 days)"), "{out}");
        call(ctx, "gantt", serde_json::json!({"op": "add_task", "page": &page, "name": "Код", "start": "2026-09-04", "after": "Дизайн", "color": "green"}));
        let out = call(ctx, "gantt", serde_json::json!({"op": "read", "page": &page}));
        assert!(out.contains("dep") && out.contains("\"Дизайн\" →"), "{out}");
        call(ctx, "gantt", serde_json::json!({"op": "update_task", "page": &page, "task": "Код", "start": "2026-09-05"}));
        let (_, chart_id) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "gantt").unwrap();
        let Some(LiveObject::Gantt { handle: gh, .. }) = ctx.object("gantt", &chart_id) else { panic!() };
        let code = gh.lock().tasks.iter().find(|t| t.name == "Код").cloned().unwrap();
        assert_eq!((code.start.as_str(), code.end.as_str()), ("2026-09-05", "2026-09-07"), "длительность сохраняется");
        assert_eq!(code.color, PALETTE[2]);

        // Удаление объектов убирает врезки со страницы.
        let out = call(ctx, "gantt", serde_json::json!({"op": "delete", "chart": &chart_id}));
        assert!(out.contains("embeds removed from 1 page"), "{out}");
        assert!(!ctx.page_markdown(&page).contains("gantt:"), "{}", ctx.page_markdown(&page));
        call(ctx, "kanban", serde_json::json!({"op": "delete", "board": &board_id}));
        assert!(!ctx.page_markdown(&page).contains("kanban:"));
        assert!(ctx.page_markdown(&page).contains("### Доска"), "заголовок над доской остаётся");
        assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "read", "page": &page})).is_err());
    }
}
