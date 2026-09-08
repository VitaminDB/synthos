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
//! - `list` — дерево страниц (id, название, объём, объекты на странице);
//!   `search` — поиск по названиям и тексту; `read` — страница целиком
//!   (markdown, доски и диаграммы на ней, связи) или сразу пачка:
//!   `pages` (список), `depth` (подстраницы), `page="all"` (весь проект)
//!   — сколько влезает в живой бюджет контекста ([`ReadBudget`]),
//!   остальное перечислено по id.
//!   `blocks op=read` так же берёт `all` / «0,2,5-7» / массив.
//! - `create` / `update` / `move` / `delete` / `duplicate` — страницы.
//!   `update` умеет переименовать, сменить иконку и раскладку, заменить
//!   текст целиком (`content`, `mode=replace|append|prepend`) и точечно
//!   (`find`/`replace`).
//! - `open` — показать страницу пользователю; `attach` — файл с диска или
//!   вложение чата → вложение проекта + медиа-блок на странице.
//! - `kanban` / `gantt` — объекты-примитивы: создать на странице, прочитать,
//!   колонки/карточки и задачи/зависимости, стиль доски и масштаб
//!   диаграммы, удалить (врезки убираются со страниц, файл — из бандла).
//! - `chart` — график (линии, столбцы, круговая, радар, шкала): создать на
//!   странице или из блока-таблицы, прочитать данными, менять вид, подписи,
//!   ряды и оформление, удалить.
//! - `blocks` — блоки страницы как структура: список с индексами,
//!   геометрией и атрибутами; вставка, замена, перенос, удаление одного
//!   блока; любые атрибуты (стиль текста, координаты и размеры, параметры
//!   фигур); закрепление на холсте и снятие с него.
//! - `shape` — примитивы: создать фигуру или линию (концы — в абсолютных
//!   координатах холста), изменить, удалить, `connect` — стрелка между
//!   двумя закреплёнными блоками.
//!
//! Адресация: страница — id (12 hex) либо название (без регистра; при
//! совпадениях — путь «Родитель / Страница» или id); доска/диаграмма — id
//! объекта либо страница, на которой объект один; колонка — id или
//! название; карточка/задача — id или заголовок; блок — индекс верхнего
//! уровня из `blocks op=list` либо `find:<фрагмент>` с единственным
//! вхождением.
//!
//! **Служебный хвост.** Агент видит и правит «плоский» markdown без хвоста
//! ```` ```doc-layout ````: там по индексу блока лежат координаты и —
//! у блоков без места под инлайн-атрибуты (абзац, списки, код, таблица)
//! — все их свойства. При записи блоки, чей markdown не изменился, получают
//! свои прежние атрибуты ([`with_sidecar`]) — перестановка абзаца агентом
//! не сбивает ни холст, ни оформление. Точечные правки (`find`/`replace`,
//! `blocks`) идут по модели документа и атрибуты блока не теряют.
//!
//! Все сигналы — main-thread: действие целиком исполняется в
//! `run_on_main_thread`-замыкании, результат уходит через oneshot
//! (паттерн `pipelines`). Формат результата — секционный plain-text.

use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;
use syngui::widgets::input::document_editor::attrs::{parse_attr_block, serialize_attrs};
use syngui::widgets::input::document_editor::serialize::block_markdown;
use syngui::widgets::input::document_editor::{
    free, parse_document, props, serialize_document, shape, Attrs, BlockKind, DocBlock, DocModel, ShapeKind,
};

use crate::pages::notes::gantt::calendar::{days_to_iso, parse_days, today_days};
use crate::pages::notes::gantt::model::GanttDoc;
use crate::pages::notes::gantt::GanttHandle;
use crate::pages::notes::kanban::model::{
    fmt_duration, item_id, parse_duration, parse_tags, DropSpot, KanbanCard, KanbanColumn, Moment, Priority, PALETTE,
};
use crate::pages::notes::kanban::KanbanHandle;
use crate::pages::notes::calendar::model::{
    fmt_hm, parse_hm, CalEvent, CalView, CalendarStore, CalendarStyle, EventStyle, Repeat,
};
use crate::pages::notes::calendar::{CalendarHandle, CalendarStoreHandle, ExternalQuery};
use crate::pages::notes::chart::model::{self as chart_model, ChartDoc, ChartKind, GaugeZone, LegendPos, PieLabels};
use crate::pages::notes::chart::ChartHandle;
use crate::pages::notes::mindmap::model::{Curve, Direction, MindmapDoc, NodeShape};
use crate::pages::notes::mindmap::MindmapHandle;
use crate::pages::notes::project::{PageGrid, PageLayout};
use crate::pages::notes::state::{object_refs, LiveObject, NotesCtx};
use crate::pages::notes::{embeds, media};
use crate::syn_chat::attach::blobs;

use super::budget;
use super::executor::{ToolError, MAX_NOTES_OUTPUT_BYTES};

mod life;

/// Главный entrypoint из `executor::execute`.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: Json = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let action = str_field(&v, "action")
        .ok_or(ToolError::MissingField("action"))?
        .to_string();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    run_on_main_thread(move || {
        let ctx = use_context::<NotesCtx>();
        // Правки от имени агента — так они помечены в журнале проекта.
        let _agent = crate::pages::notes::activity::agent_scope();
        let _ = tx.send(dispatch(ctx, &action, &v));
    });
    rx.await
        .map_err(|e| ToolError::Spawn(e.to_string()))?
        .map_err(ToolError::Args)
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
        "blocks" => blocks_impl(ctx, v),
        "shape" => shape_impl(ctx, v),
        "mindmap" => mindmap_impl(ctx, v),
        "calendar" => calendar_impl(ctx, v),
        "chart" => chart_impl(ctx, v),
        "agenda" => life::agenda_impl(ctx, v),
        "tasks" => life::tasks_impl(ctx, v),
        "log" => life::log_impl(ctx, v),
        "journal" => life::journal_impl(ctx, v),
        other => Err(format!(
            "unknown action \"{other}\" (list | search | read | create | update | move | delete | \
             duplicate | open | attach | blocks | shape | kanban | gantt | mindmap | calendar | chart | \
             agenda | tasks | log | journal)"
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

/// Ссылка на блок: строка либо число (индекс).
fn ref_field(v: &Json, key: &str) -> Option<String> {
    match v.get(key)? {
        Json::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
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
/// по ней же ссылка «Открыть в заметках» в чате находит страницу. У
/// свободной раскладки к ней дописывается [`free_layout_hint`].
fn page_line(ctx: NotesCtx, id: &str) -> String {
    let mut s = format!("page: {id} · \"{}\" · path: {}", ctx.title_of(id), path_of(ctx, id));
    if let Some(hint) = free_layout_hint(ctx, id) {
        s.push('\n');
        s.push_str(&hint);
    }
    s
}

/// Свободная раскладка: блок без `x`/`y` уходит в колонку потока по центру
/// холста, а закреплённые стоят по координатам — две системы отсчёта на
/// одной странице, и текст оказывается под фигурами и досками. Модель
/// координат не видит и по умолчанию надеется, что редактор разложит блоки
/// сам, поэтому предупреждение висит в КАЖДОМ ответе по такой странице,
/// пока геометрия не проставлена (07.09.2026: страница «Тестовая» из
/// `create layout=free` + markdown легла кашей ровно так).
///
/// Новая страница — холст по умолчанию (`PageLayout::default`), но пока на
/// ней ничего не закреплено, она рисуется одной колонкой и выглядит как
/// обычный документ. Поэтому подсказка только при СМЕСИ: есть и
/// закреплённые блоки, и блоки без координат.
fn free_layout_hint(ctx: NotesCtx, id: &str) -> Option<String> {
    if !ctx.page_layout(id).free {
        return None;
    }
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
         boards, charts, mind maps and calendars) with blocks op=pin or op=set_attrs, or switch \
         the page to layout=flow. Pinned content currently ends at y={}.",
        flow.len(),
        model.blocks.len(),
        fnum(bottom)
    ))
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

fn kanban_handle(ctx: NotesCtx, v: &Json) -> Result<(String, KanbanHandle), String> {
    match resolve_object(ctx, v, "kanban", "board")? {
        LiveObject::Kanban { id, handle } => Ok((id, handle)),
        _ => Err("not a kanban board".to_string()),
    }
}

fn mindmap_handle(ctx: NotesCtx, v: &Json) -> Result<(String, MindmapHandle), String> {
    match resolve_object(ctx, v, "mindmap", "map")? {
        LiveObject::Mindmap { id, handle } => Ok((id, handle)),
        _ => Err("not a mind map".to_string()),
    }
}

fn calendar_handle(ctx: NotesCtx, v: &Json) -> Result<(String, CalendarHandle), String> {
    match resolve_object(ctx, v, "calendar", "calendar")? {
        LiveObject::Calendar { id, handle } => Ok((id, handle)),
        _ => Err("not a calendar widget".to_string()),
    }
}

fn gantt_handle(ctx: NotesCtx, v: &Json) -> Result<(String, GanttHandle), String> {
    // Диаграмму адресует ключ "gantt": "chart" с появлением графиков
    // (`![[chart:<id>]]`) означает их, но как старое имя ещё принимается.
    let key = if str_field(v, "gantt").is_some() { "gantt" } else { "chart" };
    match resolve_object(ctx, v, "gantt", key)? {
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
// Markdown: плоская форма и служебный хвост
// ─────────────────────────────────────────────────────────────────────────────

/// Снять с блока всё, что уходит в служебный хвост: геометрию — всегда, а
/// у блоков без места под инлайн-атрибуты — все атрибуты (иначе агент
/// увидел бы ```` ```doc-layout ```` с индексами и сдвинул бы их вставкой).
fn strip_sidecar(block: &mut DocBlock) {
    if free::has_inline_attrs(&block.kind) {
        for k in [free::ATTR_X, free::ATTR_Y, free::ATTR_W, free::ATTR_H] {
            block.attrs.remove(k);
        }
    } else {
        block.attrs = Attrs::default();
    }
}

/// Markdown страницы без служебного хвоста — то, что видит и правит агент.
pub fn plain_markdown(md: &str) -> String {
    let mut model = parse_document(md);
    for b in &mut model.blocks {
        strip_sidecar(b);
    }
    serialize_document(&model)
}

/// Текст страницы от агента + атрибуты старых блоков: блок, чей markdown
/// не изменился, остаётся на своём месте холста и в своём оформлении
/// (совпадение по тексту, по порядку, каждый старый блок — один раз).
pub fn with_sidecar(current: &str, new_plain: &str) -> String {
    let old = parse_document(current);
    let old_md: Vec<String> = old.blocks.iter().map(block_markdown).collect();
    let mut used = vec![false; old.blocks.len()];
    let mut fresh = parse_document(new_plain);
    for b in &mut fresh.blocks {
        let key = block_markdown(b);
        let Some(i) = (0..old.blocks.len()).find(|&i| !used[i] && old_md[i] == key) else { continue };
        used[i] = true;
        for (k, val) in old.blocks[i].attrs.0.iter() {
            if b.attrs.get(k).is_none() {
                b.attrs.set(k.clone(), val.clone());
            }
        }
    }
    serialize_document(&fresh)
}

/// Записать новый плоский текст страницы, сохранив атрибуты нетронутых блоков.
fn write_page(ctx: NotesCtx, id: &str, new_plain: &str) -> Result<(), String> {
    let current = ctx.page_markdown(id);
    let merged = with_sidecar(&current, new_plain);
    store_markdown(ctx, id, &merged)
}

fn store_markdown(ctx: NotesCtx, id: &str, md: &str) -> Result<(), String> {
    if ctx.set_page_markdown(id, md) {
        Ok(())
    } else {
        Err(format!("page {id} not found"))
    }
}

/// Модель страницы со служебным хвостом — для правок на уровне блоков.
fn load_model(ctx: NotesCtx, id: &str) -> DocModel {
    parse_document(&ctx.page_markdown(id))
}

fn store_model(ctx: NotesCtx, id: &str, model: &DocModel) -> Result<(), String> {
    store_markdown(ctx, id, &serialize_document(model))
}

// ─────────────────────────────────────────────────────────────────────────────
// Блоки: адресация, геометрия, вставка
// ─────────────────────────────────────────────────────────────────────────────

/// Ширина блока в потоке / без своей ширины (как `DocLayout::block_width`).
const DEFAULT_BLOCK_W: f32 = 520.0;

/// Куда вставлять фрагмент относительно верхнеуровневых блоков.
#[derive(Clone, Copy, Debug, PartialEq)]
enum InsertPos {
    End,
    Start,
    Index(usize),
}

/// Позиция из `index` / `after` / `before` (ссылки на блоки); без них —
/// `fallback`.
fn parse_pos(model: &DocModel, v: &Json, fallback: InsertPos) -> Result<InsertPos, String> {
    if let Some(i) = usize_field(v, "index") {
        return Ok(InsertPos::Index(i.min(model.blocks.len())));
    }
    if let Some(a) = ref_field(v, "after") {
        return Ok(InsertPos::Index(resolve_block(model, &a)? + 1));
    }
    if let Some(b) = ref_field(v, "before") {
        return Ok(InsertPos::Index(resolve_block(model, &b)?));
    }
    Ok(fallback)
}

fn pos_index(model: &DocModel, pos: InsertPos) -> usize {
    match pos {
        InsertPos::End => model.blocks.len(),
        InsertPos::Start => 0,
        InsertPos::Index(i) => i.min(model.blocks.len()),
    }
}

fn pos_text(pos: InsertPos) -> String {
    match pos {
        InsertPos::End => "at the end of the page".to_string(),
        InsertPos::Start => "at the start of the page".to_string(),
        InsertPos::Index(i) => format!("at block #{i}"),
    }
}

/// Геометрия из аргументов `x y w h`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Geom {
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
}

fn parse_geom(v: &Json) -> Result<Geom, String> {
    let num = |k: &str| -> Result<Option<f32>, String> {
        match v.get(k) {
            None | Some(Json::Null) => Ok(None),
            Some(_) => f32_field(v, k).filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("bad \"{k}\" — a number in px")),
        }
    };
    let g = Geom { x: num("x")?, y: num("y")?, w: num("w")?, h: num("h")? };
    if let Some(w) = g.w {
        if w < 40.0 {
            return Err("\"w\" must be at least 40 px".to_string());
        }
    }
    if let Some(h) = g.h {
        if h < 20.0 {
            return Err("\"h\" must be at least 20 px".to_string());
        }
    }
    if g.x.is_some() != g.y.is_some() {
        return Err("pass both \"x\" and \"y\" to place a block on the canvas".to_string());
    }
    Ok(g)
}

impl Geom {
    fn is_empty(&self) -> bool {
        self.x.is_none() && self.w.is_none() && self.h.is_none()
    }

    fn apply(&self, attrs: &mut Attrs) {
        if let (Some(x), Some(y)) = (self.x, self.y) {
            free::set_pos(attrs, x, y);
        }
        if let Some(w) = self.w {
            free::set_width(attrs, w);
        }
        if let Some(h) = self.h {
            free::set_height(attrs, h);
        }
    }
}

/// Число в атрибут: до десятых, целые — без хвоста.
fn fnum(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < f32::EPSILON {
        format!("{}", r.round() as i64)
    } else {
        format!("{r}")
    }
}

/// Блок по ссылке агента: индекс верхнего уровня (`3`, `#3`) либо
/// `find:<фрагмент>` / текст — единственное вхождение в markdown блока.
fn resolve_block(model: &DocModel, s: &str) -> Result<usize, String> {
    let t = s.trim();
    let t = t.strip_prefix("block:").unwrap_or(t).trim();
    let n = model.blocks.len();
    if let Ok(i) = t.trim_start_matches('#').parse::<usize>() {
        return (i < n).then_some(i).ok_or_else(|| format!("block #{i} does not exist — the page has {n} blocks (blocks op=list)"));
    }
    let needle = t.strip_prefix("find:").unwrap_or(t).trim().to_lowercase();
    if needle.is_empty() {
        return Err("empty block reference — pass an index from blocks op=list or find:<text>".to_string());
    }
    let hits: Vec<usize> =
        (0..n).filter(|&i| block_markdown(&model.blocks[i]).to_lowercase().contains(&needle)).collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!("no block contains \"{t}\" — see blocks op=list")),
        _ => Err(format!(
            "{} blocks contain \"{t}\" — use the index: {}",
            hits.len(),
            hits.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Оценка высоты блока в px (модель не знает раскладки текста — числа
/// приблизительные, в выводе помечаются тильдой).
fn est_height(b: &DocBlock, w: f32) -> f32 {
    fn text_lines(chars: usize, w: f32, glyph: f32) -> f32 {
        let per_line = ((w - 16.0) / glyph).max(8.0);
        (chars as f32 / per_line).ceil().max(1.0)
    }
    match &b.kind {
        BlockKind::Shape { shape } if shape.is_line() => shape::line_box(&b.attrs, *shape).height,
        BlockKind::Shape { .. } => shape::height_of(&b.attrs),
        BlockKind::Media { .. } => free::height_of(&b.attrs).unwrap_or(220.0),
        BlockKind::Divider => 17.0,
        BlockKind::Embed { .. } => free::height_of(&b.attrs).unwrap_or(200.0),
        BlockKind::Table { rows, .. } => (rows.len() as f32 + 1.0) * 30.0,
        BlockKind::CodeBlock { code, .. } => code.lines().count().max(1) as f32 * 20.0 + 16.0,
        BlockKind::Heading { level, text } => {
            let size = match level {
                1 => 32.0,
                2 => 26.0,
                3 => 22.0,
                _ => 18.0,
            };
            text_lines(text.text().chars().count(), w, size * 0.55) * size * 1.4 + 8.0
        }
        _ => {
            let chars = b.kind.text().map(|t| t.text().chars().count()).unwrap_or(0);
            let own = text_lines(chars, w, 8.5) * 24.0 + 8.0;
            let children: f32 = b.kind.children().map(|c| c.iter().map(|x| est_height(x, w - 24.0)).sum()).unwrap_or(0.0);
            own + children
        }
    }
}

fn block_width(b: &DocBlock) -> f32 {
    free::width_of(&b.attrs).unwrap_or(DEFAULT_BLOCK_W)
}

/// Прямоугольник закреплённого блока на холсте (высота — из `h` либо оценка).
fn block_rect(b: &DocBlock) -> Option<(f32, f32, f32, f32)> {
    let (x, y) = free::pos_of(&b.attrs)?;
    let w = block_width(b);
    let h = free::height_of(&b.attrs).unwrap_or_else(|| est_height(b, w));
    Some((x, y, w, h))
}

/// Параметры [`arrange_column`]. `x`/`y` — начало колонки (иначе — под
/// нижним закреплённым блоком у его левого края, на пустом холсте 40×40),
/// `w` — ширина всем разложенным (иначе своя у блока, иначе
/// [`DEFAULT_BLOCK_W`]), `all` — перекладывать и уже закреплённые.
struct Arrange {
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    gap: f32,
    all: bool,
}

impl Default for Arrange {
    fn default() -> Self {
        Self { x: None, y: None, w: None, gap: ARRANGE_GAP, all: false }
    }
}

/// Зазор между блоками колонки по умолчанию, px.
const ARRANGE_GAP: f32 = 24.0;
/// Левый край и верх колонки на пустом холсте, px.
const ARRANGE_ORIGIN: f32 = 40.0;

/// Разложить блоки колонкой в порядке документа: каждый следующий — под
/// предыдущим с зазором `gap`. Берёт неприкреплённые блоки (или все при
/// `all`). У объектов без своей высоты (фигуры, медиа, врезки) высота
/// фиксируется оценкой — чтобы редактор и агент считали одно и то же.
/// Возвращает индексы разложенных блоков.
fn arrange_column(model: &mut DocModel, a: &Arrange) -> Vec<usize> {
    let targets: Vec<usize> = (0..model.blocks.len())
        .filter(|&i| a.all || free::pos_of(&model.blocks[i].attrs).is_none())
        .collect();
    if targets.is_empty() {
        return targets;
    }
    let others: Vec<(f32, f32, f32, f32)> = model
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| !targets.contains(i))
        .filter_map(|(_, b)| block_rect(b))
        .collect();
    let x = a.x.unwrap_or_else(|| {
        others
            .iter()
            .map(|r| r.0)
            .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.min(v))))
            .unwrap_or(ARRANGE_ORIGIN)
            .max(0.0)
    });
    let mut y = a.y.unwrap_or_else(|| {
        others
            .iter()
            .map(|r| r.1 + r.3)
            .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
            .map(|bottom| bottom + a.gap)
            .unwrap_or(ARRANGE_ORIGIN)
    });
    for &i in &targets {
        let b = &mut model.blocks[i];
        let w = a.w.or_else(|| free::width_of(&b.attrs)).unwrap_or(DEFAULT_BLOCK_W);
        free::set_pos(&mut b.attrs, x.round(), y.round());
        free::set_width(&mut b.attrs, w);
        if needs_own_height(b) && free::height_of(&b.attrs).is_none() {
            let h = est_height(b, w).round();
            free::set_height(&mut b.attrs, h);
        }
        if let BlockKind::Shape { shape } = b.kind {
            if shape.is_line() {
                canonicalize_line(b, shape);
            }
        }
        let h = free::height_of(&b.attrs).unwrap_or_else(|| est_height(b, w));
        y = (y + h + a.gap).round();
    }
    targets
}

/// Блок, у которого нет собственной высоты по содержимому: без `h` редактор
/// возьмёт свою (200 px), и оценка агента с ней разойдётся.
fn needs_own_height(b: &DocBlock) -> bool {
    matches!(b.kind, BlockKind::Shape { .. } | BlockKind::Media { .. } | BlockKind::Embed { .. })
}

fn is_line(b: &DocBlock) -> bool {
    matches!(b.kind, BlockKind::Shape { shape } if shape.is_line())
}

/// Закреплённые блоки, чьи рамки пересекают рамку блока `i`, — строкой для
/// ответа (`!! overlaps #3 (x=… y=… w=… h=…)`), либо `None`. Не запрет, а
/// подсказка: высоты текста оценочные (в списке помечены `~`). Линии не в
/// счёт — стрелка между двумя блоками пересекает оба по замыслу.
fn overlap_note(model: &DocModel, i: usize) -> Option<String> {
    let me = model.blocks.get(i)?;
    if is_line(me) {
        return None;
    }
    let (x, y, w, h) = block_rect(me)?;
    let hits: Vec<String> = model
        .blocks
        .iter()
        .enumerate()
        .filter(|(j, b)| *j != i && !is_line(b))
        .filter_map(|(j, b)| {
            let (ox, oy, ow, oh) = block_rect(b)?;
            let overlaps = x < ox + ow - 1.0 && ox < x + w - 1.0 && y < oy + oh - 1.0 && oy < y + h - 1.0;
            overlaps.then(|| format!("#{j} (x={} y={} w={} h={})", fnum(ox), fnum(oy), fnum(ow), fnum(oh)))
        })
        .collect();
    (!hits.is_empty()).then(|| format!("!! overlaps {}", hits.join(", ")))
}

/// Строка блока для ответа плюс предупреждение о наложении, если есть.
fn block_line_checked(model: &DocModel, i: usize) -> String {
    let mut s = block_line(i, &model.blocks[i]);
    if let Some(note) = overlap_note(model, i) {
        s.push('\n');
        s.push_str(&note);
    }
    s
}

fn kind_label(b: &DocBlock) -> String {
    match &b.kind {
        BlockKind::Heading { level, .. } => format!("heading{level}"),
        BlockKind::Shape { shape } => format!("shape:{}", shape.name()),
        BlockKind::Embed { target } => format!("embed:{}", target.trim()),
        other => props::kind_name(other).to_string(),
    }
}

/// Абсолютные точки линейной фигуры (концы и, у кривой, направляющие) —
/// только у закреплённого блока; иначе локальные.
fn line_points_abs(b: &DocBlock, kind: ShapeKind) -> Vec<(f32, f32)> {
    let (ox, oy) = free::pos_of(&b.attrs).map(|(x, y)| (x + shape::LINE_PAD, y + shape::LINE_PAD)).unwrap_or((0.0, 0.0));
    shape::line_handles(&b.attrs, kind).into_iter().map(|(x, y)| (x + ox, y + oy)).collect()
}

/// Строка блока в `blocks op=list`.
fn block_line(i: usize, b: &DocBlock) -> String {
    let mut s = format!("#{i} {}", kind_label(b));
    let label = props::label_of(b).replace(['\n', '"'], " ");
    if !label.trim().is_empty() && !matches!(b.kind, BlockKind::Shape { .. } | BlockKind::Embed { .. }) {
        s.push_str(&format!(" \"{}\"", label.trim()));
    }
    let w = block_width(b);
    match free::pos_of(&b.attrs) {
        Some((x, y)) => s.push_str(&format!(" · x={} y={} w={}", fnum(x), fnum(y), fnum(w))),
        None => s.push_str(" · flow"),
    }
    match free::height_of(&b.attrs) {
        Some(h) => s.push_str(&format!(" h={}", fnum(h))),
        None => s.push_str(&format!(" h=~{}", fnum(est_height(b, w)))),
    }
    let mut rest = Attrs::default();
    for (k, v) in b.attrs.0.iter() {
        let is_point = matches!(b.kind, BlockKind::Shape { shape } if shape.is_line())
            && matches!(k.as_str(), "x1" | "y1" | "x2" | "y2" | "cx1" | "cy1" | "cx2" | "cy2");
        if !free::is_geom_key(k) && !is_point {
            rest.set(k.clone(), v.clone());
        }
    }
    if let BlockKind::Shape { shape } = &b.kind {
        if shape.is_line() {
            let pts = line_points_abs(b, *shape);
            let p = |i: usize| format!("({},{})", fnum(pts[i].0), fnum(pts[i].1));
            s.push_str(&format!(" · from {} to {}", p(0), p(1)));
            if shape.is_curve() && pts.len() == 4 {
                s.push_str(&format!(" via {} {}", p(2), p(3)));
            }
            if free::pos_of(&b.attrs).is_none() {
                s.push_str(" (relative — not pinned)");
            }
        }
    }
    if !rest.is_empty() {
        s.push_str(&format!(" · {}", serialize_attrs(&rest)));
    }
    s
}

fn blocks_text(model: &DocModel) -> String {
    if model.blocks.is_empty() {
        return "(no blocks)\n".to_string();
    }
    let mut out = String::new();
    for (i, b) in model.blocks.iter().enumerate() {
        out.push_str(&block_line(i, b));
        out.push('\n');
    }
    out
}

/// Разобрать фрагмент markdown в блоки; геометрия — первому (`last=false`)
/// или последнему блоку фрагмента.
fn fragment_blocks(md: &str, geom: &Geom, last: bool) -> Result<Vec<DocBlock>, String> {
    let mut blocks = parse_document(md).blocks;
    if blocks.is_empty() {
        return Err("the markdown fragment is empty".to_string());
    }
    if !geom.is_empty() {
        let idx = if last { blocks.len() - 1 } else { 0 };
        geom.apply(&mut blocks[idx].attrs);
    }
    Ok(blocks)
}

/// Вставить блоки в позицию; возвращает индексы вставленных.
fn insert_blocks(model: &mut DocModel, blocks: Vec<DocBlock>, pos: InsertPos) -> Vec<usize> {
    let at = pos_index(model, pos);
    let n = blocks.len();
    let tail = model.blocks.split_off(at);
    model.blocks.extend(blocks);
    model.blocks.extend(tail);
    (at..at + n).collect()
}

fn indices_text(idx: &[usize]) -> String {
    idx.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ")
}

/// Заменить блок `i` результатом разбора `md`; первый новый блок наследует
/// атрибуты старого (те, которых у него нет).
fn replace_block(model: &mut DocModel, i: usize, md: &str) -> Result<Vec<usize>, String> {
    let old = model.blocks.remove(i);
    let mut fresh = parse_document(md).blocks;
    if fresh.is_empty() {
        return Ok(Vec::new());
    }
    for (k, v) in old.attrs.0.iter() {
        if fresh[0].attrs.get(k).is_none() {
            fresh[0].attrs.set(k.clone(), v.clone());
        }
    }
    Ok(insert_blocks(model, fresh, InsertPos::Index(i)))
}

/// `find`/`replace` по блокам: совпадение ищется внутри markdown одного
/// верхнеуровневого блока, блок перепарсивается с сохранением атрибутов.
fn replace_in_blocks(model: &mut DocModel, find: &str, replace: &str, all: bool) -> Result<usize, String> {
    let per_block: Vec<usize> = model.blocks.iter().map(|b| block_markdown(b).matches(find).count()).collect();
    let n: usize = per_block.iter().sum();
    if n == 0 {
        let joined: String = model.blocks.iter().map(block_markdown).collect::<Vec<_>>().join("\n");
        return Err(if joined.contains(find) {
            "find text spans several blocks — replace it with blocks op=set_markdown, or update with \
             content"
                .to_string()
        } else {
            "find text not found on the page — read the page and copy the fragment exactly \
             (the page is compared as markdown, not as rendered text)"
                .to_string()
        });
    }
    if n > 1 && !all {
        return Err(format!(
            "find text occurs {n} times — pass all=true to replace every occurrence, or a longer \
             unique fragment"
        ));
    }
    // С конца — индексы впереди не съезжают.
    for i in (0..model.blocks.len()).rev() {
        if per_block[i] == 0 {
            continue;
        }
        let md = block_markdown(&model.blocks[i]);
        let new_md = if all { md.replace(find, replace) } else { md.replacen(find, replace, 1) };
        replace_block(model, i, &new_md)?;
    }
    Ok(n)
}

// ─────────────────────────────────────────────────────────────────────────────
// Атрибуты блока
// ─────────────────────────────────────────────────────────────────────────────

/// `#rrggbb` / `#rgb` / имя палитры / `none`; с `alpha` — и `#rrggbbaa`.
fn parse_hex_color(s: &str, alpha: bool) -> Result<String, String> {
    if let Ok(c) = parse_color(s) {
        return Ok(c);
    }
    let t = s.trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    if alpha && hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(format!("#{}", hex.to_uppercase()));
    }
    Err(format!(
        "bad color \"{s}\" — use #rrggbb{}, gray | orange | green | blue | purple | red | teal or none",
        if alpha { " / #rrggbbaa" } else { "" }
    ))
}

/// Проверить и нормализовать значение атрибута блока; `Ok(None)` — снять.
fn validate_attr(key: &str, value: &str) -> Result<Option<String>, String> {
    let v = value.trim();
    let cleared = v.is_empty() || matches!(v.to_ascii_lowercase().as_str(), "none" | "null" | "default");
    let range = |lo: f32, hi: f32| -> Result<Option<String>, String> {
        if cleared {
            return Ok(None);
        }
        let n: f32 = v.parse().map_err(|_| format!("bad \"{key}\" \"{v}\" — a number {lo}..{hi}"))?;
        if !(lo..=hi).contains(&n) {
            return Err(format!("\"{key}\" must be within {lo}..{hi}"));
        }
        Ok(Some(fnum(n)))
    };
    match key {
        "color" | "bg" | "fill" => {
            if cleared {
                return Ok(None);
            }
            parse_hex_color(v, true).map(Some)
        }
        "stroke" => {
            if v.is_empty() || matches!(v.to_ascii_lowercase().as_str(), "default" | "null") {
                return Ok(None);
            }
            if v.eq_ignore_ascii_case("none") {
                return Ok(Some("none".to_string()));
            }
            parse_hex_color(v, true).map(Some)
        }
        "size" => range(6.0, 160.0),
        "weight" => match v.to_ascii_lowercase().as_str() {
            "" | "none" | "null" | "default" => Ok(None),
            "bold" | "normal" => Ok(Some(v.to_ascii_lowercase())),
            _ => Err(format!("bad \"weight\" \"{v}\" (bold | normal)")),
        },
        "align" => match v.to_ascii_lowercase().as_str() {
            "" | "none" | "null" | "default" => Ok(None),
            "left" | "center" | "right" => Ok(Some(v.to_ascii_lowercase())),
            _ => Err(format!("bad \"align\" \"{v}\" (left | center | right)")),
        },
        "x" | "y" | "x1" | "y1" | "x2" | "y2" | "cx1" | "cy1" | "cx2" | "cy2" => range(-1.0e6, 1.0e6),
        "w" => range(40.0, 1.0e5),
        "h" => range(20.0, 1.0e5),
        "sw" => range(0.0, 40.0),
        "dash" => range(0.0, 60.0),
        "radius" => range(0.0, 200.0),
        "opacity" => range(0.0, 100.0),
        other => Err(format!(
            "unknown attribute \"{other}\" — allowed: color bg size weight align (text), x y w h (geometry), \
             fill stroke sw dash radius opacity (shape), x1 y1 x2 y2 cx1 cy1 cx2 cy2 (line ends, relative)"
        )),
    }
}

/// Атрибуты из аргумента `attrs`: JSON-объект либо строка `{k=v …}` /
/// `k=v k=v` / JSON-текст.
fn attrs_arg(v: &Json) -> Result<Vec<(String, String)>, String> {
    let raw = v.get("attrs").ok_or("missing \"attrs\" (object {key: value}; empty or null value clears)")?;
    let mut out = Vec::new();
    match raw {
        Json::Object(map) => {
            for (k, val) in map {
                let s = match val {
                    Json::Null => String::new(),
                    Json::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push((k.clone(), s));
            }
        }
        Json::String(s) => {
            let t = s.trim();
            if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(t) {
                return attrs_arg(&serde_json::json!({ "attrs": map }));
            }
            let braced = if t.starts_with('{') { t.to_string() } else { format!("{{{t}}}") };
            let parsed = parse_attr_block(&braced).ok_or_else(|| format!("can't parse attrs \"{t}\" — pass an object"))?;
            for (k, val) in parsed.0 {
                out.push((k, val));
            }
        }
        _ => return Err("\"attrs\" must be an object {key: value}".to_string()),
    }
    if out.is_empty() {
        return Err("\"attrs\" is empty".to_string());
    }
    Ok(out)
}

/// Применить проверенные атрибуты к блоку; возвращает описание изменений.
fn apply_attrs(block: &mut DocBlock, pairs: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for (k, v) in pairs {
        let key = k.trim().to_ascii_lowercase();
        match validate_attr(&key, v)? {
            Some(val) => {
                block.attrs.set(key.clone(), val.clone());
                changes.push(format!("{key}={val}"));
            }
            None => {
                block.attrs.remove(&key);
                changes.push(format!("{key} cleared"));
            }
        }
    }
    // Координаты — только парой; ширина/высота у линии пересчитываются по концам.
    let has_x = block.attrs.get("x").is_some();
    let has_y = block.attrs.get("y").is_some();
    if has_x != has_y {
        block.attrs.remove("x");
        block.attrs.remove("y");
        return Err("pass both x and y (or clear both) to place a block on the canvas".to_string());
    }
    if let BlockKind::Shape { shape } = block.kind {
        if shape.is_line() {
            canonicalize_line(block, shape);
        }
    }
    Ok(changes)
}

/// Привести линейную фигуру к канону: минимум точек — в нуле, рамка сдвинута
/// под них, ширина/высота — по bbox с полем.
fn canonicalize_line(block: &mut DocBlock, kind: ShapeKind) {
    let pts = shape::line_handles(&block.attrs, kind);
    let (min_x, min_y) = pts.iter().fold((f32::MAX, f32::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
    if min_x.abs() > 0.05 || min_y.abs() > 0.05 {
        let (p1, p2) = shape::endpoints_of(&block.attrs);
        shape::set_endpoints(&mut block.attrs, (p1.0 - min_x, p1.1 - min_y), (p2.0 - min_x, p2.1 - min_y));
        // Направляющие сдвигаются только те, что заданы явно: остальные
        // выводятся из концов заново.
        for (key, d) in [("cx1", min_x), ("cy1", min_y), ("cx2", min_x), ("cy2", min_y)] {
            if let Some(v) = block.attrs.get(key).and_then(|v| v.parse::<f32>().ok()) {
                block.attrs.set(key, fnum(v - d));
            }
        }
        if let Some((x, y)) = free::pos_of(&block.attrs) {
            free::set_pos(&mut block.attrs, x + min_x, y + min_y);
        }
    }
    let size = shape::line_box(&block.attrs, kind);
    free::set_width(&mut block.attrs, size.width.max(40.0));
    free::set_height(&mut block.attrs, size.height.max(20.0));
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
    out.push_str("--- Pages (indent = nesting; size in words; objects embedded in the page after ·) ---\n");
    if tree.is_empty() {
        out.push_str("(no pages yet — notes create makes one)\n");
    } else {
        fn walk(ctx: NotesCtx, nodes: &[crate::pages::notes::project::PageNode], depth: usize, out: &mut String) {
            for n in nodes {
                let icon = n.icon.as_deref().filter(|i| !i.is_empty()).map(|i| format!(" {i}")).unwrap_or_default();
                let md = ctx.page_markdown(&n.id);
                let objects: Vec<String> =
                    object_refs(&md).iter().map(|(k, id)| format!(" · {k}:{id}")).collect();
                out.push_str(&format!(
                    "{}{} \"{}\"{icon} · {} words{}\n",
                    "  ".repeat(depth),
                    n.id,
                    n.title,
                    count_words(&plain_markdown(&md)),
                    objects.concat()
                ));
                walk(ctx, &n.children, depth + 1, out);
            }
        }
        walk(ctx, &tree.roots, 0, &mut out);
        // Дерево на тысячу страниц само по себе больше окна. Клипа поверх
        // ответа `notes` нет (инструмент отвечает за свой размер сам),
        // поэтому список режется здесь — по строке, а не посреди неё.
        let budget = ReadBudget::take();
        if !budget.fits((0, 0), ReadBudget::cost(&out)) {
            out = budget.clip_lines(out, "the tree is longer than the context left");
        }
    }
    out.push_str(
        "---\nread {page} shows the markdown, boards and charts on the page and its links; \
         read {pages: [\"id\", \"id\"]} or {page, depth=all} or {page=\"all\"} brings back \
         several pages, a whole subtree or the whole project in ONE call — use it instead of \
         reading page by page; update {page, content | find+replace | mode=append} edits it; \
         kanban / gantt / chart {op=create, page} add a board, a Gantt chart or a \
         line/bar/pie/radar/gauge chart.\n",
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

/// Потолок ответа, когда бюджет не измерен: вызов идёт вне agent-loop
/// (юнит-тесты, чужой код) и остатка окна взять неоткуда. В самом ходе
/// потолка нет — границу ставит живое окно модели.
const READ_FALLBACK_TOKENS: usize = 8_000;

/// Вторая мера — байты, предохранитель исполнителя
/// ([`MAX_NOTES_OUTPUT_BYTES`]): он режет вывод молча и посреди строки, а
/// пачка обязана обрываться на границе страницы. Вычет — на шапку ответа и
/// список недочитанного.
const READ_MAX_BYTES: usize = MAX_NOTES_OUTPUT_BYTES - 8 * 1024;

/// Сколько инструмент может отдать этим ответом.
struct ReadBudget {
    /// Грант живого окна в токенах — половина того, что осталось.
    tokens: usize,
    bytes: usize,
    /// Сколько токенов окна осталось до конца контекста (0 — не измерено).
    left: usize,
    measured: bool,
}

impl ReadBudget {
    fn take() -> Self {
        // Потолка сверху нет: сколько окна модель даёт, столько и читаем —
        // проект на 14k токенов при свободном контексте уходит одним
        // ответом, а не тремя ходами с полным префиллом каждый.
        let g = budget::grant(usize::MAX);
        Self {
            tokens: if g.measured { g.tokens } else { READ_FALLBACK_TOKENS },
            bytes: READ_MAX_BYTES,
            left: g.left,
            measured: g.measured,
        }
    }

    /// Влезает ли ещё один кусок: считаем обеими мерами сразу.
    fn fits(&self, spent: (usize, usize), piece: (usize, usize)) -> bool {
        spent.0 + piece.0 <= self.tokens && spent.1 + piece.1 <= self.bytes
    }

    /// Обрыв случился из-за тесноты в окне, а не из-за предохранителя.
    fn by_window(&self, spent: (usize, usize), piece: (usize, usize)) -> bool {
        self.measured && spent.0 + piece.0 > self.tokens
    }

    /// Чего стоит кусок: токены (точно либо оценкой) и байты.
    fn cost(text: &str) -> (usize, usize) {
        (budget::count(text), text.len())
    }

    /// Хвост шапки: сколько занял ответ и сколько окна осталось. Модель
    /// должна видеть причину обрыва — «контекст кончается», а не «инструмент
    /// такой».
    fn note(&self, spent: usize) -> String {
        if !self.measured {
            return String::new();
        }
        format!(" · ~{spent} tokens of the ~{} left in context", self.left)
    }

    /// Текст длиннее бюджета: отрезаем по строкам (в markdown это границы
    /// блоков) и дописываем пометку — `tail(keep, total, spent)`. Доля
    /// берётся от цены целого и ужимается, пока ответ вместе с пометкой не
    /// влезет: считать токены по каждой строке слишком дорого.
    fn clip_by_lines(&self, text: String, tail: impl Fn(usize, usize, usize) -> String) -> String {
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let total = lines.len();
        let cost = Self::cost(&text);
        let mut share = (self.tokens as f32 / cost.0.max(1) as f32)
            .min(self.bytes as f32 / cost.1.max(1) as f32)
            .min(1.0);
        let mut last = String::new();
        for _ in 0..4 {
            share *= 0.8;
            let keep = ((total as f32 * share) as usize).clamp(1, total);
            let head: String = lines[..keep].concat();
            last = format!("{head}{}", tail(keep, total, Self::cost(&head).0));
            if self.fits((0, 0), Self::cost(&last)) {
                return last;
            }
        }
        last
    }

    /// Список страниц длиннее бюджета: дерево на тысячу страниц само по
    /// себе больше окна.
    fn clip_lines(&self, text: String, why: &str) -> String {
        self.clip_by_lines(text, |keep, total, spent| {
            format!("--- Cut after {keep} of {total} lines: {why}{} ---\n", self.note(spent))
        })
    }

    /// Одна страница больше всего бюджета: обрываем по строке и говорим,
    /// чем дочитать — `blocks op=read` умеет отдать остаток кусками.
    fn clip_page(&self, text: String, id: &str) -> String {
        self.clip_by_lines(text, |keep, total, spent| {
            format!(
                "--- Page truncated: {keep} of {total} lines{} ---\n\
                 Read the rest in pieces: blocks {{\"op\": \"read\", \"page\": \"{id}\", \
                 \"block\": \"...\"}}.\n{}",
                self.note(spent),
                self.advice(self.measured)
            )
        })
    }

    /// Подсказка, когда режет именно теснота окна.
    fn advice(&self, by_window: bool) -> &'static str {
        if by_window {
            "\nContext is filling up: read only what you need next, or ask the user to \
             compact the chat before reading the rest.\n"
        } else {
            ""
        }
    }
}

/// «Весь проект» в `page`/`pages` — но только если так не называется
/// настоящая страница.
fn is_all_pages(ctx: NotesCtx, s: &str) -> bool {
    let t = s.trim();
    matches!(t.to_ascii_lowercase().as_str(), "all" | "*" | "project" | "everything")
        && ctx.tree.get_untracked().find(t).is_none()
        && ctx.find_by_title(t).is_empty()
}

/// Одна ссылка на страницу либо несколько через запятую/перенос строки:
/// целая строка важнее — в названии страницы запятая законна.
fn resolve_many(ctx: NotesCtx, raw: &str) -> Result<Vec<String>, String> {
    let whole = resolve_page_ref(ctx, raw);
    if let Ok(id) = whole {
        return Ok(vec![id]);
    }
    let parts: Vec<&str> = raw.split([',', '\n']).map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return whole.map(|id| vec![id]);
    }
    parts.into_iter().map(|p| resolve_page_ref(ctx, p)).collect()
}

/// `depth` — сколько уровней подстраниц добрать к каждой запрошенной
/// странице: число, `true` либо «all» (всё поддерево).
fn depth_arg(v: &Json) -> Result<usize, String> {
    let level = |i: i64| if i < 0 { usize::MAX } else { i as usize };
    match v.get("depth") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(b)) => Ok(if *b { usize::MAX } else { 0 }),
        Some(Json::Number(n)) => n.as_i64().map(level).ok_or_else(|| "\"depth\" must be a whole number".to_string()),
        Some(Json::String(s)) => {
            let t = s.trim().to_ascii_lowercase();
            if t.is_empty() {
                return Ok(0);
            }
            if matches!(t.as_str(), "all" | "*" | "full" | "tree" | "subtree" | "max" | "true") {
                return Ok(usize::MAX);
            }
            t.parse::<i64>()
                .map(level)
                .map_err(|_| format!("unknown \"depth\" \"{s}\" (a number of levels or \"all\")"))
        }
        Some(other) => Err(format!("\"depth\" must be a number or \"all\", got {other}")),
    }
}

/// Страница и её потомки до глубины `depth` (DFS, как в дереве).
fn subtree_within(tree: &crate::pages::notes::project::ProjectTree, id: &str, depth: usize, out: &mut Vec<String>) {
    fn walk(n: &crate::pages::notes::project::PageNode, left: usize, out: &mut Vec<String>) {
        out.push(n.id.clone());
        if left == 0 {
            return;
        }
        for c in &n.children {
            walk(c, left - 1, out);
        }
    }
    if let Some(n) = tree.find(id) {
        walk(n, depth, out);
    }
}

/// Что читать: `page` и/или `pages` (массив либо строка через запятую),
/// плюс подстраницы по `depth`; `page="all"` — весь проект. Порядок — как
/// в дереве, повторы убираются, `limit` режет хвост.
fn read_targets(ctx: NotesCtx, v: &Json) -> Result<Vec<String>, String> {
    let mut raws: Vec<String> = Vec::new();
    match v.get("pages") {
        None | Some(Json::Null) => {}
        Some(Json::Array(a)) => {
            for e in a {
                match e {
                    Json::String(s) if !s.trim().is_empty() => raws.push(s.trim().to_string()),
                    Json::Number(n) => raws.push(n.to_string()),
                    other => return Err(format!("\"pages\" takes page ids or titles, got {other}")),
                }
            }
        }
        Some(Json::String(s)) if !s.trim().is_empty() => raws.push(s.trim().to_string()),
        Some(Json::String(_)) => {}
        Some(other) => {
            return Err(format!("\"pages\" must be an array of pages or a comma-separated string, got {other}"))
        }
    }
    if let Some(s) = str_field(v, "page") {
        raws.push(s.to_string());
    }
    if raws.is_empty() {
        return Err("missing \"page\" (page id or title; \"pages\" takes several at once, \
                    page=\"all\" the whole project)"
            .to_string());
    }
    let tree = ctx.tree.get_untracked();
    let mut roots: Vec<String> = Vec::new();
    let mut all = false;
    for raw in &raws {
        if is_all_pages(ctx, raw) {
            all = true;
            continue;
        }
        for id in resolve_many(ctx, raw)? {
            if !roots.contains(&id) {
                roots.push(id);
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    if all {
        out = tree.all_ids();
    } else {
        let depth = depth_arg(v)?;
        for id in &roots {
            let mut sub = Vec::new();
            subtree_within(&tree, id, depth, &mut sub);
            for s in sub {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    if out.is_empty() {
        return Err("no pages to read — the project has none yet (notes create makes one)".to_string());
    }
    if out.len() > 1 {
        if let Some(limit) = usize_field(v, "limit") {
            out.truncate(limit.max(1));
        }
    }
    Ok(out)
}

/// Одна страница целиком: markdown, её блоки (по запросу), доски,
/// диаграммы, карты и календари на ней, ссылки.
fn page_text(ctx: NotesCtx, id: &str, with_blocks: bool) -> String {
    let tree = ctx.tree.get_untracked();
    let Some(node) = tree.find(id) else {
        return format!("page: {id} · (gone from the tree — see notes list)\n");
    };
    let raw = ctx.page_markdown(id);
    let md = plain_markdown(&raw);
    let mut out = String::new();
    out.push_str(&page_line(ctx, id));
    out.push('\n');
    out.push_str(&format!(
        "icon: {} · {} · {} words · children: {}\n",
        node.icon.as_deref().unwrap_or("none"),
        layout_text(&node.layout),
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
    if with_blocks {
        out.push_str("--- Blocks (index · kind · canvas x y w h, ~ = estimated · attributes) ---\n");
        out.push_str(&blocks_text(&parse_document(&raw)));
    }
    let objects = object_refs(&md);
    if !objects.is_empty() {
        out.push_str("--- Objects on the page ---\n");
        for (kind, oid) in &objects {
            match ctx.object(kind, oid) {
                Some(LiveObject::Kanban { handle, .. }) => out.push_str(&board_text(oid, &handle, false)),
                Some(LiveObject::Gantt { handle, .. }) => out.push_str(&gantt_text(oid, &handle)),
                Some(LiveObject::Mindmap { handle, .. }) => out.push_str(&map_text(oid, &handle)),
                Some(LiveObject::Calendar { handle, .. }) => out.push_str(&calendar_widget_text(oid, &handle)),
                Some(LiveObject::Chart { handle, .. }) => out.push_str(&chart_text(oid, &handle)),
                None => out.push_str(&format!("{kind}:{oid} · (file missing)\n")),
            }
        }
    }
    let index = ctx.index.get_untracked();
    let outgoing = index.outgoing_of(id);
    let backlinks = index.backlinks_of(id);
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
    out
}

/// `read` — одна страница или сразу пачка (`pages`, `depth`, `page="all"`).
/// Чтение по одной странице за вызов у локальной модели стоит целого хода
/// с полным префиллом, поэтому дерево целиком отдаётся одним ответом,
/// сколько влезает в [`ReadBudget`].
fn read_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let ids = read_targets(ctx, v)?;
    let with_blocks = bool_field(v, "blocks").unwrap_or(false);
    let budget = ReadBudget::take();
    if ids.len() == 1 {
        let mut out = page_text(ctx, &ids[0], with_blocks);
        // Подстраницы называем сразу: иначе модель обходит дерево по
        // странице за ход, а это полный префилл на каждую.
        let kids = ctx.tree.get_untracked().find(&ids[0]).map(|n| n.children.clone()).unwrap_or_default();
        if !kids.is_empty() {
            out.push_str(&format!(
                "--- Sub-pages ({}) — read the whole subtree in ONE call with depth=all ---\n",
                kids.len()
            ));
            for k in &kids {
                out.push_str(&format!(
                    "{} \"{}\" · {} words · children: {}\n",
                    k.id,
                    k.title,
                    count_words(&plain_markdown(&ctx.page_markdown(&k.id))),
                    k.children.len()
                ));
            }
        }
        if !budget.fits((0, 0), ReadBudget::cost(&out)) {
            out = budget.clip_page(out, &ids[0]);
        }
        return Ok(out);
    }
    let total = ids.len();
    let mut body = String::new();
    let mut done = 0usize;
    let mut words = 0usize;
    let mut spent = (0usize, 0usize);
    let mut by_window = false;
    for (n, id) in ids.iter().enumerate() {
        let text = page_text(ctx, id, with_blocks);
        let cost = ReadBudget::cost(&text);
        if !body.is_empty() && !budget.fits(spent, cost) {
            by_window = budget.by_window(spent, cost);
            break;
        }
        spent = (spent.0 + cost.0, spent.1 + cost.1);
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&format!("=== page {}/{} ===\n", n + 1, total));
        body.push_str(&text);
        if !text.ends_with('\n') {
            body.push('\n');
        }
        words += count_words(&plain_markdown(&ctx.page_markdown(id)));
        done += 1;
    }
    let mut out = String::new();
    if done == total {
        out.push_str(&format!(
            "--- {total} pages · {words} words{} · all of them in this reply ---\n",
            budget.note(spent.0)
        ));
    } else {
        out.push_str(&format!(
            "--- {done} of {total} pages · {words} words{} · the rest did not fit in one reply ---\n",
            budget.note(spent.0)
        ));
    }
    out.push_str(&body);
    if done < total {
        let rest: Vec<String> =
            ids[done..].iter().map(|id| format!("{id} \"{}\"", ctx.title_of(id))).collect();
        out.push_str(&format!(
            "--- Not read ({}) ---\n{}\nRead them with one more call: pages=[\"{}\", …].\n{}",
            total - done,
            rest.join("\n"),
            ids[done],
            budget.advice(by_window)
        ));
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

/// id страницы-соседа с точно таким же названием — то самое совпадение, из-за
/// которого `insert_page` переименует новую страницу в «Название 2».
fn sibling_with_title(ctx: NotesCtx, parent: Option<&str>, title: &str) -> Option<String> {
    let tree = ctx.tree.get_untracked();
    let siblings = match parent {
        None => tree.roots.clone(),
        Some(pid) => tree.find(pid).map(|n| n.children.clone()).unwrap_or_default(),
    };
    siblings.into_iter().find(|n| n.title == title).map(|n| n.id)
}

fn create_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let title = str_field(v, "title").map(str::to_string).unwrap_or_else(|| tr!("notes.untitled"));
    let parent = parse_parent(ctx, v)?;
    let index = usize_field(v, "index");
    // Дубликат названия среди соседей ловим ДО вставки: `insert_page` тихо
    // переименует новую страницу («Туду — канбан 2»), и «created» в ответе
    // выглядит как полный успех. Агент, который не увидел свою же прошлую
    // страницу, на этом зацикливается — 04.09.2026 так родился 21 дубликат
    // подряд. Страницу всё равно создаём (одноимённые страницы законны:
    // «Журнал» в каждой сфере), но о совпадении говорим прямо.
    let clash = sibling_with_title(ctx, parent.as_deref(), &title);
    let id = ctx.insert_page(parent.as_deref(), index, &title, false);
    let mut out = String::from("created\n");
    if let Some(existing) = clash {
        out.push_str(&format!(
            "note: a sibling page \"{title}\" already exists (page: {existing}), so this new one \
             was named \"{}\". If you meant that existing page, do not create it again — use \
             update/open on {existing}.\n",
            ctx.title_of(&id)
        ));
    }
    if let Some(icon) = str_field(v, "icon") {
        ctx.set_icon(&id, Some(icon.to_string()));
    }
    apply_layout_args(ctx, &id, v)?;
    if let Some(content) = raw_string(v, "content") {
        write_page(ctx, &id, &content)?;
        out.push_str(&format!("content: {} words\n", count_words(&content)));
        // `create layout=free` + markdown: агент явно просил холст, а
        // раскладка применилась к ещё пустой странице и контент пришёл без
        // геометрии — кладём его колонкой, как при переключении раскладки,
        // иначе первая же фигура ляжет поверх текста. Без явного `layout`
        // страницу не трогаем: холст по умолчанию без закреплённых блоков
        // рисуется как обычный документ.
        let asked_free = str_field(v, "layout")
            .is_some_and(|l| matches!(l.trim().to_ascii_lowercase().as_str(), "free" | "canvas"));
        if asked_free && ctx.page_layout(&id).free {
            let pinned = pin_flow_blocks(ctx, &id)?;
            if pinned > 0 {
                out.push_str(&format!("layout: free — {pinned} blocks pinned in a column\n"));
            }
        }
    }
    if bool_field(v, "open").unwrap_or(false) {
        show_page(ctx, Some(&id));
    }
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
}

/// Раскладка страницы одной строкой (для `read`).
fn layout_text(l: &PageLayout) -> String {
    let grid = match l.grid {
        PageGrid::None => "none".to_string(),
        g => format!("{} step {}", grid_name(g), fnum(l.grid_step)),
    };
    format!(
        "layout: {} · grid: {grid} · snap: {} · bg: {}",
        if l.free { "free" } else { "flow" },
        if l.snap { format!("on step {}", fnum(l.snap_step)) } else { "off".to_string() },
        if l.bg.is_empty() { "theme" } else { l.bg.as_str() }
    )
}

fn grid_name(g: PageGrid) -> &'static str {
    match g {
        PageGrid::None => "none",
        PageGrid::Dots => "dots",
        PageGrid::Lines => "lines",
        PageGrid::Cross => "cross",
    }
}

/// Раскладка страницы из аргументов `layout grid grid_step snap snap_step`;
/// возвращает список изменений.
fn apply_layout_args(ctx: NotesCtx, id: &str, v: &Json) -> Result<Vec<String>, String> {
    let mut l: PageLayout = ctx.page_layout(id);
    let was_free = l.free;
    let mut changes = Vec::new();
    if let Some(layout) = str_field(v, "layout") {
        match layout.to_ascii_lowercase().as_str() {
            "free" | "canvas" => l.free = true,
            "flow" | "document" | "column" => l.free = false,
            other => return Err(format!("unknown layout \"{other}\" (free | flow)")),
        }
        changes.push(format!("layout: {}", if l.free { "free" } else { "flow" }));
    }
    if let Some(grid) = str_field(v, "grid") {
        l.grid = match grid.to_ascii_lowercase().as_str() {
            "none" | "off" => PageGrid::None,
            "dots" | "dot" => PageGrid::Dots,
            "lines" | "line" => PageGrid::Lines,
            "cross" | "crosses" => PageGrid::Cross,
            other => return Err(format!("unknown grid \"{other}\" (none | dots | lines | cross)")),
        };
        changes.push(format!("grid: {}", grid_name(l.grid)));
    }
    if let Some(step) = f32_field(v, "grid_step") {
        if !(2.0..=200.0).contains(&step) {
            return Err("\"grid_step\" must be within 2..200 px".to_string());
        }
        l.grid_step = step.round();
        changes.push(format!("grid_step: {}", fnum(l.grid_step)));
    }
    if let Some(snap) = bool_field(v, "snap") {
        l.snap = snap;
        changes.push(format!("snap: {}", if snap { "on" } else { "off" }));
    }
    if let Some(step) = f32_field(v, "snap_step") {
        if !(1.0..=100.0).contains(&step) {
            return Err("\"snap_step\" must be within 1..100 px".to_string());
        }
        l.snap_step = step.round();
        changes.push(format!("snap_step: {}", fnum(l.snap_step)));
    }
    if let Some(bg) = raw_string(v, "bg") {
        l.bg = if bg.trim().is_empty() { String::new() } else { parse_hex_color(&bg, true)? };
        changes.push(if l.bg.is_empty() { "bg: theme".to_string() } else { format!("bg: {}", l.bg) });
    }
    if !changes.is_empty() {
        let now_free = l.free;
        ctx.set_page_layout(id, l);
        if now_free && !was_free {
            // Переход поток → холст сохраняет расположение: блоки без
            // координат встают колонкой в порядке документа. Редактор на
            // экране сделал бы то же по настоящим прямоугольникам; агенту
            // они недоступны, поэтому колонка — по оценке высот.
            let pinned = pin_flow_blocks(ctx, id)?;
            if pinned > 0 {
                changes.push(format!("{pinned} blocks pinned in a column where the flow had them"));
            }
        }
    }
    Ok(changes)
}

/// Разложить неприкреплённые блоки страницы колонкой (см.
/// [`arrange_column`]); возвращает, сколько блоков получили координаты.
fn pin_flow_blocks(ctx: NotesCtx, id: &str) -> Result<usize, String> {
    let mut model = load_model(ctx, id);
    let placed = arrange_column(&mut model, &Arrange::default());
    if !placed.is_empty() {
        store_model(ctx, id, &model)?;
    }
    Ok(placed.len())
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
    changes.extend(apply_layout_args(ctx, &id, v)?);

    let mode = str_field(v, "mode").map(|m| m.to_ascii_lowercase()).unwrap_or_else(|| "replace".to_string());
    if let Some(content) = raw_string(v, "content") {
        match mode.as_str() {
            "replace" => {
                write_page(ctx, &id, &content)?;
                changes.push(format!("content replaced: {} words now", count_words(&content)));
            }
            "append" | "prepend" | "insert" => {
                let mut model = load_model(ctx, &id);
                let fallback = if mode == "prepend" { InsertPos::Start } else { InsertPos::End };
                let pos = parse_pos(&model, v, fallback)?;
                let blocks = fragment_blocks(&content, &parse_geom(v)?, false)?;
                let idx = insert_blocks(&mut model, blocks, pos);
                store_model(ctx, &id, &model)?;
                changes.push(format!("inserted {} block{} {} ({})", idx.len(), if idx.len() == 1 { "" } else { "s" }, pos_text(pos), indices_text(&idx)));
            }
            other => return Err(format!("unknown mode \"{other}\" (replace | append | prepend | insert)")),
        }
    }

    if let Some(find) = raw_string(v, "find") {
        if find.is_empty() {
            return Err("\"find\" is empty".to_string());
        }
        let replace = raw_string(v, "replace").unwrap_or_default();
        let all = bool_field(v, "all").unwrap_or(false);
        let mut model = load_model(ctx, &id);
        let n = replace_in_blocks(&mut model, &find, &replace, all)?;
        store_model(ctx, &id, &model)?;
        changes.push(format!("replaced {n} occurrence{}", if n == 1 { "" } else { "s" }));
    }

    if changes.is_empty() {
        return Err(
            "nothing to update: pass title, icon, layout/grid/snap, content (+mode) or find/replace".to_string()
        );
    }
    let mut out = changes.join("\n");
    out.push('\n');
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
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

/// Байты вложения из аргументов: `path` (файл на диске) либо `attachment`
/// (вложение чата). Возвращает `(bytes, ext, stem, имя файла)`. Модель не
/// передаёт байты сама — только ссылку на файл.
fn attachment_bytes(v: &Json) -> Result<(Vec<u8>, String, String, String), String> {
    if let Some(p) = str_field(v, "path") {
        let path = expand_home(p);
        let bytes = std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Ok((bytes, ext, stem, name))
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
        Ok((bytes, ext, stem, a.original_name.clone()))
    } else {
        Err("pass \"path\" (file on disk) or \"attachment\" (chat attachment: name | sha | last)".to_string())
    }
}

fn attach_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let (bytes, ext, default_caption, _) = attachment_bytes(v)?;
    let size = bytes.len();
    let url = media::ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
    let caption = str_field(v, "caption").map(str::to_string).unwrap_or(default_caption);
    let caption = caption.replace(['[', ']', '\n'], " ");
    let block = format!("![{caption}]({url})");
    let mut model = load_model(ctx, &id);
    let pos = parse_pos(&model, v, InsertPos::End)?;
    let blocks = fragment_blocks(&block, &parse_geom(v)?, false)?;
    let idx = insert_blocks(&mut model, blocks, pos);
    store_model(ctx, &id, &model)?;
    Ok(format!(
        "attached {url} ({size} bytes) as a media block {} ({})\n{}\n",
        pos_text(pos),
        indices_text(&idx),
        page_line(ctx, &id)
    ))
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

/// Момент плана карточки: `yyyy-mm-dd`, `yyyy-mm-ddThh:mm`, `today`,
/// `tomorrow` (можно со временем: `today 10:00`); пусто/`none` — снять.
fn parse_plan_moment(s: &str) -> Result<Option<String>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    for (word, delta) in [("today", 0i64), ("tomorrow", 1)] {
        let Some(rest) = lower.strip_prefix(word) else { continue };
        let rest = rest.trim_start_matches(['t', ' ']).trim();
        let min = match rest.is_empty() {
            true => None,
            false => Some(parse_hm(rest).ok_or_else(|| format!("bad time \"{rest}\" (hh:mm)"))?),
        };
        return Ok(Some(Moment { day: today_days() + delta, min }.iso()));
    }
    Moment::parse(t)
        .map(|m| Some(m.iso()))
        .ok_or_else(|| format!("bad date \"{s}\" (yyyy-mm-dd | yyyy-mm-ddThh:mm | today | tomorrow | none)"))
}

/// Оценка длительности: `1d`, `2h`, `90m`, `1d 4h`, голое число — часы;
/// пусто/`none` — снять.
fn parse_duration_arg(s: &str) -> Result<Option<u32>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" || lower == "0" {
        return Ok(None);
    }
    parse_duration(t)
        .map(Some)
        .ok_or_else(|| format!("bad duration \"{s}\" (1d | 2h | 90m | \"1d 4h\" | a bare number = hours | none)"))
}

/// Повтор карточки: `none | daily | weekly | monthly | yearly`.
fn parse_repeat(s: &str) -> Result<Repeat, String> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() || t == "null" || t == "off" {
        return Ok(Repeat::None);
    }
    Repeat::parse(&t).ok_or_else(|| format!("bad repeat \"{s}\" (none | daily | weekly | monthly | yearly)"))
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
    if let Some(d) = c.duration {
        s.push_str(&format!(" · duration: {}", fmt_duration(d)));
    }
    let span = c.span_text();
    if !span.is_empty() {
        s.push_str(&format!(" · planned: {span}"));
    }
    if let Some((done, total)) = c.checklist() {
        s.push_str(&format!(" · checklist {done}/{total}"));
    }
    if c.repeat != Repeat::None {
        s.push_str(&format!(" · repeat: {}", c.repeat.key()));
    }
    if let Some(d) = &c.created {
        s.push_str(&format!(" · created: {d}"));
    }
    if let Some(d) = &c.done {
        s.push_str(&format!(" · done: {d}"));
    }
    if !c.files.is_empty() {
        s.push_str(&format!(" · files: {}", c.files.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")));
    }
    s
}

/// Текст доски: шапка, колонки, карточки по колонкам; `full` — с
/// markdown-содержимым карточек.
fn board_text(id: &str, handle: &KanbanHandle, full: bool) -> String {
    let doc = handle.lock();
    let mut out = String::new();
    out.push_str(&format!(
        "kanban:{id} · columns: {} · cards: {}{}{}{}\n",
        doc.columns.len(),
        doc.cards.len(),
        if doc.archive.is_empty() { String::new() } else { format!(" · archived: {}", doc.archive.len()) },
        doc.archive_after.map(|d| format!(" · archive_after: {d} days")).unwrap_or_default(),
        if full {
            format!(
                " · style: column_width={} lane_bg={} card_bg={} counts={}",
                doc.style.column_width,
                if doc.style.lane_bg.is_empty() { "theme" } else { doc.style.lane_bg.as_str() },
                if doc.style.card_bg.is_empty() { "theme" } else { doc.style.card_bg.as_str() },
                if doc.style.show_counts { "on" } else { "off" }
            )
        } else {
            String::new()
        }
    ));
    if doc.done_column().is_none() {
        out.push_str("  !! no done column: cards never get a done stamp — kanban op=update_column column=<name> done=true\n");
    }
    for col in &doc.columns {
        let cards = doc.cards_of(&col.id);
        out.push_str(&format!(
            "  column {} \"{}\"{}{}{} · {} card{}\n",
            col.id,
            col.name,
            if col.done { " · DONE column" } else { "" },
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

/// Шпаргалка операций доски с их полями — уходит модели вместо голого
/// списка имён, когда `op` пропущен или неизвестен. Живой чат (MyLife,
/// 08.09.2026): модель дважды подряд собирала `add_card` без `op` и с текстом
/// карточки в `card`; список имён ей не помог, сигнатуры — помогают.
const KANBAN_OPS_HELP: &str = "kanban ops (every op but create addresses the board by board=<id> \
or page=<page with one board>): \
create {page, columns, done_column, title, x y w h} · read {archived} · \
set_style {column_width, lane_bg, card_bg, show_counts, archive_after} · \
add_column {name, color, width, done} · update_column {column, name, color, width, done} · \
delete_column {column} · \
add_card {title = the new card's text, column, md, priority, tags, due, duration, start, end, repeat, before} · \
update_card {card, title, md, priority, tags, due, duration, start, end, repeat, column, before} · \
schedule {card, date = today | tomorrow | yyyy-mm-dd, time, duration} · unschedule {card} · \
move_card {card, column | to_board, before} · delete_card {card} · archive {card} · \
unarchive {card} · attach {card, path | attachment, name} · detach {card, file} · delete. \
\"card\" = an existing card (id or exact title); a new card's text goes in \"title\".";

fn kanban_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or_else(|| format!("missing \"op\". {KANBAN_OPS_HELP}"))?;
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
                            done: false,
                        })
                        .collect();
                    doc.detect_done_column();
                });
            }
            if let Some(dc) = str_field(v, "done_column") {
                let col = resolve_column(&handle, dc)?;
                handle.edit(|doc| {
                    for c in &mut doc.columns {
                        c.done = c.id == col.id;
                    }
                });
            }
            let (pos, idx) = embed_object(ctx, &pid, "kanban", &id, v)?;
            Ok(format!(
                "created board {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                board_text(&id, &handle, false),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let mut out = board_text(&id, &handle, true);
            if bool_field(v, "archived").unwrap_or(false) {
                let doc = handle.lock();
                out.push_str(&format!("--- Archive ({}) ---\n", doc.archive.len()));
                for c in &doc.archive {
                    out.push_str(&format!("    {}\n", card_line(c)));
                }
            }
            Ok(format!("{out}{}\n", object_page_line(ctx, "kanban", &id)))
        }
        "set_style" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let mut changes = Vec::new();
            if let Some(raw) = raw_string(v, "archive_after") {
                let t = raw.trim().to_ascii_lowercase();
                let days = if t.is_empty() || t == "none" || t == "off" || t == "0" {
                    None
                } else {
                    Some(t.parse::<u32>().map_err(|_| format!("bad \"archive_after\" \"{raw}\" (days, 0/none = off)"))?)
                };
                handle.set_archive_after(days);
                changes.push(days.map(|d| format!("archive_after {d} days")).unwrap_or_else(|| "archive_after off".to_string()));
            }
            let column_width = f32_field(v, "column_width");
            let lane_bg = match raw_string(v, "lane_bg") {
                Some(c) => Some(parse_hex_color(&c, true).or_else(|e| if c.trim().is_empty() { Ok(String::new()) } else { Err(e) })?),
                None => None,
            };
            let card_bg = match raw_string(v, "card_bg") {
                Some(c) => Some(parse_hex_color(&c, true).or_else(|e| if c.trim().is_empty() { Ok(String::new()) } else { Err(e) })?),
                None => None,
            };
            let show_counts = bool_field(v, "show_counts");
            if let Some(w) = column_width {
                if !(crate::pages::notes::kanban::model::MIN_COLUMN_WIDTH..=crate::pages::notes::kanban::model::MAX_COLUMN_WIDTH).contains(&w) {
                    return Err(format!(
                        "\"column_width\" must be within {}..{} px",
                        crate::pages::notes::kanban::model::MIN_COLUMN_WIDTH,
                        crate::pages::notes::kanban::model::MAX_COLUMN_WIDTH
                    ));
                }
                changes.push(format!("column_width {}", fnum(w)));
            }
            if let Some(c) = &lane_bg {
                changes.push(if c.is_empty() { "lane_bg cleared".to_string() } else { format!("lane_bg {c}") });
            }
            if let Some(c) = &card_bg {
                changes.push(if c.is_empty() { "card_bg cleared".to_string() } else { format!("card_bg {c}") });
            }
            if let Some(s) = show_counts {
                changes.push(format!("counts {}", if s { "on" } else { "off" }));
            }
            if changes.is_empty() {
                return Err("nothing to change: pass column_width, lane_bg, card_bg, show_counts or archive_after".to_string());
            }
            handle.set_style(|s| {
                if let Some(w) = column_width {
                    s.column_width = w;
                }
                if let Some(c) = lane_bg {
                    s.lane_bg = c;
                }
                if let Some(c) = card_bg {
                    s.card_bg = c;
                }
                if let Some(v) = show_counts {
                    s.show_counts = v;
                }
            });
            Ok(format!("style: {}\n{}", changes.join(", "), board_text(&id, &handle, true)))
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
            if let Some(d) = bool_field(v, "done") {
                handle.set_column_done(&cid, d);
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
            if let Some(d) = bool_field(v, "done") {
                handle.set_column_done(&col.id, d);
                changes.push(if d { "done column".to_string() } else { "not a done column".to_string() });
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, color, width or done".to_string());
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
            // Текст новой карточки модель кладёт в "card" — в остальных op так
            // адресуют существующую, а у add_card другого смысла у поля нет.
            let title = str_field(v, "title")
                .or_else(|| str_field(v, "card"))
                .ok_or("missing \"title\" — the new card's text (\"card\" names an existing card in update_card/move_card/delete_card)")?;
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
            if let Some(r) = str_field(v, "repeat") {
                card.repeat = parse_repeat(r)?;
            }
            if let Some(d) = str_field(v, "duration") {
                card.duration = parse_duration_arg(d)?;
            }
            if let Some(d) = str_field(v, "start") {
                card.start = parse_plan_moment(d)?;
            }
            if let Some(d) = str_field(v, "end") {
                card.end = parse_plan_moment(d)?;
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
            if let Some(r) = raw_string(v, "repeat") {
                handle.set_repeat(&card.id, parse_repeat(&r)?);
                changes.push("repeat".to_string());
            }
            if let Some(d) = raw_string(v, "duration") {
                handle.set_duration(&card.id, parse_duration_arg(&d)?);
                changes.push("duration".to_string());
            }
            if raw_string(v, "start").is_some() || raw_string(v, "end").is_some() {
                let now = handle.card(&card.id).unwrap_or_else(|| card.clone());
                let start = match raw_string(v, "start") {
                    Some(d) => parse_plan_moment(&d)?,
                    None => now.start.clone(),
                };
                let end = match raw_string(v, "end") {
                    Some(d) => parse_plan_moment(&d)?,
                    None => now.end.clone(),
                };
                handle.set_schedule(&card.id, start, end);
                changes.push("plan".to_string());
            }
            if str_field(v, "column").is_some() || str_field(v, "before").is_some() {
                let col = match str_field(v, "column") {
                    Some(c) => resolve_column(&handle, c)?,
                    None => resolve_column(&handle, &card.column)?,
                };
                let spot = match str_field(v, "before") {
                    Some(b) => {
                        let b = resolve_card(&handle, b)?;
                        if b.column != col.id {
                            return Err(format!("card \"{}\" is not in column \"{}\"", b.title, col.name));
                        }
                        DropSpot::before(&col.id, &b.id)
                    }
                    None => DropSpot::end(&col.id),
                };
                handle.move_card(&card.id, &spot);
                changes.push(format!("moved to \"{}\"", col.name));
            }
            if changes.is_empty() {
                return Err(
                    "nothing to update: pass title, md, priority, tags, due, duration, start, end, repeat, column or before"
                        .to_string(),
                );
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
        "archive" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            handle.archive_card(&card.id);
            Ok(format!("archived card \"{}\" (kanban op=unarchive brings it back)\n{}", card.title, board_text(&id, &handle, false)))
        }
        "unarchive" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let key = str_field(v, "card").ok_or("missing \"card\"")?;
            let found = {
                let doc = handle.lock();
                let lower = key.trim().to_lowercase();
                let hits: Vec<&KanbanCard> =
                    doc.archive.iter().filter(|c| c.id == key.trim() || c.title.trim().to_lowercase() == lower).collect();
                match hits.len() {
                    1 => hits[0].id.clone(),
                    0 => return Err(format!("card \"{key}\" is not in the archive — kanban op=read archived=true lists it")),
                    n => return Err(format!("{n} archived cards are titled \"{key}\" — pass the id")),
                }
            };
            handle.unarchive_card(&found);
            let line = handle.card(&found).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("restored {line}\n{}", board_text(&id, &handle, false)))
        }
        "attach" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let (bytes, ext, stem, filename) = attachment_bytes(v)?;
            let size = bytes.len();
            let url = media::ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
            let name = str_field(v, "name").map(str::to_string).unwrap_or(if filename.is_empty() { stem } else { filename });
            let file = crate::pages::notes::kanban::model::CardFile::new(url.clone(), name.clone());
            let kind = if file.is_image() { "image (thumbnail on the card)" } else { "file (paperclip on the card)" };
            handle.add_file(&card.id, file);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("attached {url} ({size} bytes) as {kind} \"{name}\" to {line}\n"))
        }
        "detach" => {
            let (_, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let key = str_field(v, "file").ok_or("missing \"file\" (attachment name or asset url)")?;
            let lower = key.trim().to_lowercase();
            let hits: Vec<&crate::pages::notes::kanban::model::CardFile> =
                card.files.iter().filter(|f| f.url == key.trim() || f.label().to_lowercase() == lower).collect();
            let url = match hits.len() {
                1 => hits[0].url.clone(),
                0 => return Err(format!("card \"{}\" has no attachment \"{key}\"", card.title)),
                n => return Err(format!("{n} attachments are named \"{key}\" — pass the asset url")),
            };
            handle.remove_file(&card.id, &url);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("removed attachment {url} from {line}\n"))
        }
        "delete" => {
            let (id, _) = kanban_handle(ctx, v)?;
            delete_object(ctx, "kanban", &id)
        }
        "schedule" | "unschedule" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            if op == "unschedule" {
                handle.unschedule_card(&card.id);
                let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
                return Ok(format!("unscheduled: {line}\n{}", board_text(&id, &handle, false)));
            }
            if let Some(d) = raw_string(v, "duration") {
                handle.set_duration(&card.id, parse_duration_arg(&d)?);
            }
            // День и время плана: `date` (или `start`) + `time`; без них —
            // сегодня. Конец досчитывается по оценке.
            let iso = match str_field(v, "date").or_else(|| str_field(v, "start")) {
                Some(d) => parse_plan_moment(d)?.ok_or("\"date\" is empty — use op=unschedule to clear the plan")?,
                None => Moment { day: today_days(), min: None }.iso(),
            };
            let mut moment = Moment::parse(&iso).ok_or("bad date")?;
            if let Some(t) = str_field(v, "time") {
                moment.min = Some(parse_hm(t).ok_or_else(|| format!("bad time \"{t}\" (hh:mm)"))?);
            }
            handle.schedule_card(&card.id, moment.day, moment.min);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("scheduled: {line}\n{}", board_text(&id, &handle, false)))
        }
        "set_boards" => Err(
            // Доски выбирает тот виджет, который их показывает; подсказка
            // здесь — чтобы модель не искала операцию у самой доски.
            "boards are picked on the widget that shows them: calendar op=set_view {calendar, boards} \
             or gantt op=set_boards {gantt, boards}"
                .to_string(),
        ),
        other => Err(format!("unknown kanban op \"{other}\". {KANBAN_OPS_HELP}")),
    }
}

/// Врезать объект на страницу: `### title` (если задан) + `![[kind:id]]`
/// с высотой по умолчанию; позиция и геометрия — из аргументов (геометрия
/// достаётся самой врезке).
fn embed_object(ctx: NotesCtx, pid: &str, kind: &str, id: &str, v: &Json) -> Result<(InsertPos, Vec<usize>), String> {
    embed_object_with(ctx, pid, kind, id, v, str_field(v, "title"))
}

/// То же, но заголовок над врезкой задаётся явно: у карты `title` — текст
/// корневого узла, а не подпись блока.
fn embed_object_with(
    ctx: NotesCtx,
    pid: &str,
    kind: &str,
    id: &str,
    v: &Json,
    heading: Option<&str>,
) -> Result<(InsertPos, Vec<usize>), String> {
    let geom = parse_geom(v)?;
    let h = geom.h.unwrap_or_else(|| embeds::default_object_h(kind));
    let embed = format!("![[{kind}:{id}]]{{h={}}}", fnum(h));
    let md = match heading {
        Some(t) => format!("### {t}\n\n{embed}"),
        None => embed,
    };
    let mut model = load_model(ctx, pid);
    let pos = parse_pos(&model, v, InsertPos::End)?;
    let blocks = fragment_blocks(&md, &Geom { h: None, ..geom }, true)?;
    let idx = insert_blocks(&mut model, blocks, pos);
    store_model(ctx, pid, &model)?;
    Ok((pos, idx))
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

fn gantt_text(id: &str, handle: &GanttHandle) -> String {
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
fn map_text(id: &str, handle: &crate::pages::notes::mindmap::MindmapHandle) -> String {
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
fn calendar_widget_text(id: &str, handle: &crate::pages::notes::calendar::CalendarHandle) -> String {
    let doc = handle.lock();
    format!(
        "calendar:{id} · view: {} · anchor: {} · calendars: {}\n",
        doc.view.key(),
        doc.anchor,
        if doc.calendars.is_empty() { "all".to_string() } else { doc.calendars.join(", ") }
    )
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

// ─────────────────────────────────────────────────────────────────────────────
// Blocks
// ─────────────────────────────────────────────────────────────────────────────

/// Блок целиком: строка списка, markdown и атрибуты — тело `blocks op=read`.
fn block_read_text(i: usize, b: &DocBlock) -> String {
    let mut out = format!("{}\n--- Markdown ---\n{}", block_line(i, b), block_markdown(b));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !b.attrs.is_empty() {
        out.push_str(&format!("--- Attributes ---\n{}\n", serialize_attrs(&b.attrs)));
    }
    out
}

/// Ссылки сразу на несколько блоков для `op=read`: `all`, список индексов
/// и диапазонов («0,2,5-7»), массив ссылок либо одна ссылка (индекс или
/// `find:<текст>`) — как в остальных операциях.
fn resolve_block_list(model: &DocModel, v: &Json) -> Result<Vec<usize>, String> {
    let n = model.blocks.len();
    let check = |i: usize| -> Result<usize, String> {
        (i < n).then_some(i).ok_or_else(|| {
            format!("block #{i} does not exist — the page has {n} blocks (blocks op=list)")
        })
    };
    let mut out: Vec<usize> = Vec::new();
    match v.get("block").or_else(|| v.get("blocks")) {
        Some(Json::Array(a)) => {
            for e in a {
                let s = match e {
                    Json::String(s) => s.trim().to_string(),
                    Json::Number(num) => num.to_string(),
                    other => return Err(format!("\"block\" list takes indices or find:<text>, got {other}")),
                };
                out.push(resolve_block(model, &s)?);
            }
        }
        Some(Json::Number(num)) => out.push(resolve_block(model, &num.to_string())?),
        Some(Json::String(raw)) => {
            let t = raw.trim();
            let t = t.strip_prefix("block:").unwrap_or(t).trim();
            let numeric = !t.is_empty()
                && t.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '-' | '#' | ' '))
                && (t.contains(',') || t.contains('-'));
            if matches!(t.to_ascii_lowercase().as_str(), "all" | "*" | "every") {
                out.extend(0..n);
            } else if numeric {
                for part in t.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let num = |s: &str| -> Result<usize, String> {
                        s.trim()
                            .trim_start_matches('#')
                            .parse::<usize>()
                            .map_err(|_| format!("bad block index \"{part}\" (\"0,2,5-7\" or \"all\")"))
                    };
                    match part.split_once('-') {
                        Some((a, b)) => {
                            let (a, b) = (num(a)?, num(b)?);
                            if a > b {
                                return Err(format!("bad block range \"{part}\" — it starts after it ends"));
                            }
                            for i in a..=b {
                                out.push(check(i)?);
                            }
                        }
                        None => out.push(check(num(part)?)?),
                    }
                }
            } else {
                out.push(resolve_block(model, t)?);
            }
        }
        _ => {
            return Err("missing \"block\" (index from blocks op=list, find:<text>, a list like \
                        \"0,2,5-7\" or \"all\" for the whole page)"
                .to_string())
        }
    }
    let mut uniq: Vec<usize> = Vec::with_capacity(out.len());
    for i in out {
        if !uniq.contains(&i) {
            uniq.push(i);
        }
    }
    Ok(uniq)
}

fn blocks_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op")
        .ok_or("missing \"op\" (list | read | insert | set_markdown | delete | move | set_attrs | pin | unpin)")?;
    let id = page_arg(ctx, v, "page")?;
    let mut model = load_model(ctx, &id);
    let block_arg = |model: &DocModel| -> Result<usize, String> {
        resolve_block(model, &ref_field(v, "block").ok_or("missing \"block\" (index from blocks op=list or find:<text>)")?)
    };
    match op {
        "list" => Ok(format!("{}{}\n", blocks_text(&model), page_line(ctx, &id))),
        "read" => {
            let idx = resolve_block_list(&model, v)?;
            if idx.is_empty() {
                return Ok(format!("(no blocks)\n{}\n", page_line(ctx, &id)));
            }
            if idx.len() == 1 {
                return Ok(block_read_text(idx[0], &model.blocks[idx[0]]));
            }
            let budget = ReadBudget::take();
            let mut body = String::new();
            let mut done = 0usize;
            let mut spent = (0usize, 0usize);
            let mut by_window = false;
            for &i in &idx {
                let text = block_read_text(i, &model.blocks[i]);
                let cost = ReadBudget::cost(&text);
                if !body.is_empty() && !budget.fits(spent, cost) {
                    by_window = budget.by_window(spent, cost);
                    break;
                }
                spent = (spent.0 + cost.0, spent.1 + cost.1);
                body.push_str(&text);
                body.push('\n');
                done += 1;
            }
            let mut out = if done == idx.len() {
                format!("--- {} blocks ---\n", idx.len())
            } else {
                format!(
                    "--- {done} of {} blocks{} · the rest did not fit in one reply ---\n",
                    idx.len(),
                    budget.note(spent.0)
                )
            };
            out.push_str(&body);
            if done < idx.len() {
                out.push_str(&format!(
                    "--- Not read: {} ---\n{}",
                    indices_text(&idx[done..]),
                    budget.advice(by_window)
                ));
            }
            out.push_str(&format!("{}\n", page_line(ctx, &id)));
            Ok(out)
        }
        "insert" => {
            let md = raw_string(v, "md").or_else(|| raw_string(v, "content")).ok_or("missing \"md\" (markdown to insert)")?;
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let blocks = fragment_blocks(&md, &parse_geom(v)?, false)?;
            let idx = insert_blocks(&mut model, blocks, pos);
            store_model(ctx, &id, &model)?;
            let lines: Vec<String> = idx.iter().map(|&i| block_line_checked(&model, i)).collect();
            Ok(format!("inserted {} {}\n{}\n{}\n", indices_text(&idx), pos_text(pos), lines.join("\n"), page_line(ctx, &id)))
        }
        "set_markdown" => {
            let i = block_arg(&model)?;
            let md = raw_string(v, "md").or_else(|| raw_string(v, "content")).ok_or("missing \"md\" (new markdown of the block)")?;
            let idx = replace_block(&mut model, i, &md)?;
            store_model(ctx, &id, &model)?;
            if idx.is_empty() {
                return Ok(format!("block #{i} removed (empty markdown)\n{}\n", page_line(ctx, &id)));
            }
            let lines: Vec<String> = idx.iter().map(|&i| block_line(i, &model.blocks[i])).collect();
            Ok(format!("replaced block #{i} with {}\n{}\n{}\n", indices_text(&idx), lines.join("\n"), page_line(ctx, &id)))
        }
        "delete" => {
            let i = block_arg(&model)?;
            let line = block_line(i, &model.blocks[i]);
            model.blocks.remove(i);
            store_model(ctx, &id, &model)?;
            Ok(format!("deleted {line}\n{} blocks left\n{}\n", model.blocks.len(), page_line(ctx, &id)))
        }
        "move" => {
            let i = block_arg(&model)?;
            let geom = parse_geom(v)?;
            let has_pos = usize_field(v, "index").is_some() || ref_field(v, "after").is_some() || ref_field(v, "before").is_some();
            if !has_pos && geom.x.is_none() {
                return Err("pass index / after / before (order) and/or x + y (place on the canvas)".to_string());
            }
            let mut changes = Vec::new();
            let mut at = i;
            if has_pos {
                let pos = parse_pos(&model, v, InsertPos::End)?;
                let mut target = pos_index(&model, pos);
                let block = model.blocks.remove(i);
                if target > i {
                    target -= 1;
                }
                let target = target.min(model.blocks.len());
                model.blocks.insert(target, block);
                at = target;
                changes.push(format!("order #{i} → #{target}"));
            }
            if geom.x.is_some() {
                geom.apply(&mut model.blocks[at].attrs);
                if let BlockKind::Shape { shape } = model.blocks[at].kind {
                    if shape.is_line() {
                        canonicalize_line(&mut model.blocks[at], shape);
                    }
                }
                changes.push("placed on the canvas".to_string());
            }
            store_model(ctx, &id, &model)?;
            Ok(format!("moved: {}\n{}\n{}\n", changes.join(", "), block_line_checked(&model, at), page_line(ctx, &id)))
        }
        "set_attrs" => {
            let i = block_arg(&model)?;
            let pairs = attrs_arg(v)?;
            let changes = apply_attrs(&mut model.blocks[i], &pairs)?;
            store_model(ctx, &id, &model)?;
            Ok(format!("set {}\n{}\n{}\n", changes.join(", "), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "pin" => {
            let i = block_arg(&model)?;
            let geom = parse_geom(v)?;
            let (x, y) = match (geom.x, geom.y) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    // Под самым нижним закреплённым блоком, у левого края холста.
                    let pinned: Vec<(f32, f32, f32, f32)> = model.blocks.iter().filter_map(block_rect).collect();
                    match pinned.iter().map(|r| r.1 + r.3).fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v)))) {
                        Some(bottom) => (pinned.iter().map(|r| r.0).fold(f32::MAX, f32::min).max(0.0), bottom + 20.0),
                        None => (40.0, 40.0),
                    }
                }
            };
            free::set_pos(&mut model.blocks[i].attrs, x, y);
            if let Some(w) = geom.w {
                free::set_width(&mut model.blocks[i].attrs, w);
            }
            if let Some(h) = geom.h {
                free::set_height(&mut model.blocks[i].attrs, h);
            }
            if let BlockKind::Shape { shape } = model.blocks[i].kind {
                if shape.is_line() {
                    canonicalize_line(&mut model.blocks[i], shape);
                }
            }
            store_model(ctx, &id, &model)?;
            Ok(format!("pinned at x={} y={}\n{}\n{}\n", fnum(x), fnum(y), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "arrange" => {
            let geom = parse_geom(v)?;
            let all = match str_field(v, "only").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                None | Some("flow") | Some("unpinned") => false,
                Some("all") => true,
                Some(other) => return Err(format!("unknown \"only\" \"{other}\" (flow | all)")),
            };
            let gap = f32_field(v, "gap").unwrap_or(ARRANGE_GAP);
            if !(0.0..=400.0).contains(&gap) {
                return Err("\"gap\" must be within 0..400 px".to_string());
            }
            let a = Arrange { x: geom.x, y: geom.y, w: geom.w, gap, all };
            let placed = arrange_column(&mut model, &a);
            if placed.is_empty() {
                return Ok(format!(
                    "nothing to arrange: every block already has coordinates (pass only=all to re-stack them)\n{}\n",
                    page_line(ctx, &id)
                ));
            }
            store_model(ctx, &id, &model)?;
            let first = block_rect(&model.blocks[placed[0]]).map(|r| r.0).unwrap_or(0.0);
            let bottom = placed
                .iter()
                .filter_map(|&i| block_rect(&model.blocks[i]))
                .map(|r| r.1 + r.3)
                .fold(0.0f32, f32::max);
            let lines: Vec<String> = placed.iter().map(|&i| block_line_checked(&model, i)).collect();
            Ok(format!(
                "arranged {} blocks in a column at x={} (gap {}); the column ends at y={}\n{}\n{}\n",
                placed.len(),
                fnum(first),
                fnum(gap),
                fnum(bottom),
                lines.join("\n"),
                page_line(ctx, &id)
            ))
        }
        "unpin" => {
            let i = block_arg(&model)?;
            free::clear(&mut model.blocks[i].attrs);
            store_model(ctx, &id, &model)?;
            Ok(format!("unpinned — the block flows in the column again\n{}\n{}\n", block_line(i, &model.blocks[i]), page_line(ctx, &id)))
        }
        other => Err(format!(
            "unknown blocks op \"{other}\" (list | read | insert | set_markdown | delete | move | set_attrs | pin | unpin | arrange)"
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shapes
// ─────────────────────────────────────────────────────────────────────────────

const SHAPE_STYLE_KEYS: [&str; 6] = ["fill", "stroke", "sw", "dash", "radius", "opacity"];
const LINE_POINT_KEYS: [&str; 8] = ["x1", "y1", "x2", "y2", "cx1", "cy1", "cx2", "cy2"];

fn parse_shape_kind(s: &str) -> Result<ShapeKind, String> {
    ShapeKind::from_name(&s.trim().to_ascii_lowercase()).ok_or_else(|| {
        format!(
            "unknown shape \"{s}\" — rect | ellipse | triangle | diamond | line | arrow | arrow2 | curve | \
             curve-arrow | curve-arrow2"
        )
    })
}

/// Стиль фигуры из аргументов (`fill stroke sw dash radius opacity`).
fn shape_style_pairs(v: &Json) -> Vec<(String, String)> {
    SHAPE_STYLE_KEYS
        .iter()
        .filter_map(|k| raw_string(v, k).map(|val| (k.to_string(), val)))
        .collect()
}

/// Абсолютные точки линии из аргументов `x1 y1 x2 y2 [cx1 cy1 cx2 cy2]`.
fn line_points_arg(v: &Json) -> Result<Option<Vec<(String, f32)>>, String> {
    let mut out = Vec::new();
    for k in LINE_POINT_KEYS {
        if v.get(k).is_some_and(|x| !x.is_null()) {
            let n = f32_field(v, k).filter(|f| f.is_finite()).ok_or_else(|| format!("bad \"{k}\" — a number in px"))?;
            out.push((k.to_string(), n));
        }
    }
    if out.is_empty() {
        return Ok(None);
    }
    let has = |k: &str| out.iter().any(|(key, _)| key == k);
    if has("x1") != has("y1") || has("x2") != has("y2") || has("cx1") != has("cy1") || has("cx2") != has("cy2") {
        return Err("line points come in pairs: x1 y1, x2 y2, cx1 cy1, cx2 cy2".to_string());
    }
    Ok(Some(out))
}

/// Записать абсолютные точки линии в блок: пересчитать рамку и относительные
/// координаты (канон: минимум точек в нуле). У незакреплённого блока точки
/// считаются относительными.
fn set_line_points(block: &mut DocBlock, kind: ShapeKind, points: &[(String, f32)]) {
    let pinned = free::pos_of(&block.attrs);
    let (ox, oy) = pinned.map(|(x, y)| (x + shape::LINE_PAD, y + shape::LINE_PAD)).unwrap_or((0.0, 0.0));
    // Текущие абсолютные значения, затем перекрываем заданными.
    let (p1, p2) = shape::endpoints_of(&block.attrs);
    let mut abs: Vec<(String, f32)> = vec![
        ("x1".into(), p1.0 + ox),
        ("y1".into(), p1.1 + oy),
        ("x2".into(), p2.0 + ox),
        ("y2".into(), p2.1 + oy),
    ];
    if kind.is_curve() {
        for k in ["cx1", "cy1", "cx2", "cy2"] {
            if let Some(val) = block.attrs.get(k).and_then(|s| s.parse::<f32>().ok()) {
                abs.push((k.to_string(), val + if k.starts_with("cx") { ox } else { oy }));
            }
        }
    }
    for (k, val) in points {
        if let Some(e) = abs.iter_mut().find(|(key, _)| key == k) {
            e.1 = *val;
        } else if kind.is_curve() {
            abs.push((k.clone(), *val));
        }
    }
    let xs: Vec<f32> = abs.iter().filter(|(k, _)| k.ends_with("x1") || k.ends_with("x2")).map(|(_, v)| *v).collect();
    let ys: Vec<f32> = abs.iter().filter(|(k, _)| k.ends_with("y1") || k.ends_with("y2")).map(|(_, v)| *v).collect();
    let min_x = xs.iter().copied().fold(f32::MAX, f32::min);
    let min_y = ys.iter().copied().fold(f32::MAX, f32::min);
    for k in LINE_POINT_KEYS {
        block.attrs.remove(k);
    }
    for (k, val) in &abs {
        let d = if k.ends_with("x1") || k.ends_with("x2") { min_x } else { min_y };
        block.attrs.set(k.clone(), fnum(val - d));
    }
    if pinned.is_some() || points.iter().any(|(k, _)| k == "x1" || k == "x2") {
        free::set_pos(&mut block.attrs, min_x - shape::LINE_PAD, min_y - shape::LINE_PAD);
    }
    let size = shape::line_box(&block.attrs, kind);
    free::set_width(&mut block.attrs, size.width.max(40.0));
    free::set_height(&mut block.attrs, size.height.max(20.0));
}

/// Середина стороны прямоугольника; `auto` — сторона, обращённая к `other`.
fn side_point(rect: (f32, f32, f32, f32), side: &str, other: (f32, f32)) -> Result<(f32, f32), String> {
    let (x, y, w, h) = rect;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let side = match side.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => {
            let (dx, dy) = (other.0 - cx, other.1 - cy);
            if dx.abs() >= dy.abs() {
                if dx >= 0.0 { "right" } else { "left" }
            } else if dy >= 0.0 {
                "bottom"
            } else {
                "top"
            }
        }
        "left" | "l" => "left",
        "right" | "r" => "right",
        "top" | "t" => "top",
        "bottom" | "b" => "bottom",
        "center" | "c" => "center",
        other => return Err(format!("bad side \"{other}\" (auto | left | right | top | bottom | center)")),
    };
    Ok(match side {
        "left" => (x, cy),
        "right" => (x + w, cy),
        "top" => (cx, y),
        "bottom" => (cx, y + h),
        _ => (cx, cy),
    })
}

fn shape_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or("missing \"op\" (create | update | delete | connect)")?;
    let id = page_arg(ctx, v, "page")?;
    let mut model = load_model(ctx, &id);
    match op {
        "create" => {
            let kind = parse_shape_kind(str_field(v, "kind").unwrap_or("rect"))?;
            let mut block = DocBlock { id: model.alloc_id(), kind: BlockKind::Shape { shape: kind }, attrs: Attrs::default() };
            apply_attrs(&mut block, &shape_style_pairs(v))?;
            let geom = parse_geom(v)?;
            if kind.is_line() {
                let points = line_points_arg(v)?.unwrap_or_else(|| {
                    let (x, y) = (geom.x.unwrap_or(40.0), geom.y.unwrap_or(40.0));
                    vec![("x1".into(), x), ("y1".into(), y), ("x2".into(), x + shape::DEFAULT_W), ("y2".into(), y)]
                });
                if let (Some(x), Some(y)) = (geom.x, geom.y) {
                    free::set_pos(&mut block.attrs, x, y);
                }
                set_line_points(&mut block, kind, &points);
            } else {
                geom.apply(&mut block.attrs);
                if geom.w.is_none() {
                    free::set_width(&mut block.attrs, shape::DEFAULT_W);
                }
                if geom.h.is_none() {
                    free::set_height(&mut block.attrs, shape::DEFAULT_H);
                }
            }
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let idx = insert_blocks(&mut model, vec![block], pos);
            store_model(ctx, &id, &model)?;
            let i = idx[0];
            Ok(format!("created shape {}\n{}\n{}\n", pos_text(pos), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "update" => {
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\"")?)?;
            let BlockKind::Shape { shape: mut kind } = model.blocks[i].kind else {
                return Err(format!("block #{i} is not a shape (blocks op=set_attrs edits other blocks)"));
            };
            let mut changes = Vec::new();
            if let Some(k) = str_field(v, "kind") {
                let new_kind = parse_shape_kind(k)?;
                if new_kind != kind {
                    model.blocks[i].kind = BlockKind::Shape { shape: new_kind };
                    if new_kind.is_line() != kind.is_line() {
                        for key in LINE_POINT_KEYS {
                            model.blocks[i].attrs.remove(key);
                        }
                    }
                    kind = new_kind;
                    changes.push(format!("kind {}", new_kind.name()));
                }
            }
            let style = shape_style_pairs(v);
            if !style.is_empty() {
                changes.extend(apply_attrs(&mut model.blocks[i], &style)?);
            }
            let geom = parse_geom(v)?;
            if !geom.is_empty() {
                geom.apply(&mut model.blocks[i].attrs);
                changes.push("geometry".to_string());
            }
            if let Some(points) = line_points_arg(v)? {
                if !kind.is_line() {
                    return Err("x1/y1/x2/y2 apply to lines and arrows; frame shapes take x y w h".to_string());
                }
                set_line_points(&mut model.blocks[i], kind, &points);
                changes.push("points".to_string());
            } else if kind.is_line() {
                canonicalize_line(&mut model.blocks[i], kind);
            }
            if changes.is_empty() {
                return Err("nothing to change: pass kind, fill/stroke/sw/dash/radius/opacity, x y w h or line points".to_string());
            }
            store_model(ctx, &id, &model)?;
            Ok(format!("updated {}\n{}\n{}\n", changes.join(", "), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "delete" => {
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\"")?)?;
            if !matches!(model.blocks[i].kind, BlockKind::Shape { .. }) {
                return Err(format!("block #{i} is not a shape — blocks op=delete removes any block"));
            }
            let line = block_line(i, &model.blocks[i]);
            model.blocks.remove(i);
            store_model(ctx, &id, &model)?;
            Ok(format!("deleted {line}\n{}\n", page_line(ctx, &id)))
        }
        "connect" => {
            let from = resolve_block(&model, &ref_field(v, "from").ok_or("missing \"from\" (block)")?)?;
            let to = resolve_block(&model, &ref_field(v, "to").ok_or("missing \"to\" (block)")?)?;
            if from == to {
                return Err("\"from\" and \"to\" are the same block".to_string());
            }
            let need = |i: usize| block_rect(&model.blocks[i]).ok_or_else(|| format!("block #{i} has no coordinates — blocks op=pin it first"));
            let (ra, rb) = (need(from)?, need(to)?);
            let center = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
            let p1 = side_point(ra, str_field(v, "from_side").unwrap_or("auto"), center(rb))?;
            let p2 = side_point(rb, str_field(v, "to_side").unwrap_or("auto"), center(ra))?;
            let kind = parse_shape_kind(str_field(v, "kind").unwrap_or("arrow"))?;
            if !kind.is_line() {
                return Err("connect draws a line: kind must be line | arrow | arrow2 | curve | curve-arrow | curve-arrow2".to_string());
            }
            let mut block = DocBlock { id: model.alloc_id(), kind: BlockKind::Shape { shape: kind }, attrs: Attrs::default() };
            apply_attrs(&mut block, &shape_style_pairs(v))?;
            let points = vec![("x1".into(), p1.0), ("y1".into(), p1.1), ("x2".into(), p2.0), ("y2".into(), p2.1)];
            set_line_points(&mut block, kind, &points);
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let idx = insert_blocks(&mut model, vec![block], pos);
            store_model(ctx, &id, &model)?;
            let i = idx[0];
            Ok(format!(
                "connected #{from} → #{to} with {}\n{}\n{}\n",
                kind.name(),
                block_line(i, &model.blocks[i]),
                page_line(ctx, &id)
            ))
        }
        other => Err(format!("unknown shape op \"{other}\" (create | update | delete | connect)")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mindmap
// ─────────────────────────────────────────────────────────────────────────────

/// Узел карты по id либо тексту (единственное совпадение); `root` — корень.
fn resolve_node(handle: &MindmapHandle, s: &str) -> Result<String, String> {
    let doc = handle.lock();
    let t = s.trim();
    if t.eq_ignore_ascii_case("root") {
        return Ok(doc.root_id());
    }
    if doc.node(t).is_some() {
        return Ok(t.to_string());
    }
    let key = t.to_lowercase();
    let hits: Vec<&str> = doc.nodes.iter().filter(|n| n.text.trim().to_lowercase() == key).map(|n| n.id.as_str()).collect();
    match hits.len() {
        1 => Ok(hits[0].to_string()),
        0 => Err(format!("node \"{s}\" not found — ids and texts are in mindmap op=read")),
        n => Err(format!("{n} nodes are named \"{s}\" — use the id: {}", hits.join(", "))),
    }
}

/// Стиль карты из аргумента `style` (объект); возвращает описание изменений.
fn apply_map_style(handle: &MindmapHandle, v: &Json) -> Result<Vec<String>, String> {
    let Some(raw) = v.get("style") else { return Ok(Vec::new()) };
    let pairs = style_pairs(raw)?;
    let mut changes = Vec::new();
    let mut err = None;
    handle.set_style(|s| {
        for (k, val) in &pairs {
            let key = k.trim().to_ascii_lowercase();
            let num = || val.trim().parse::<f32>().ok();
            match key.as_str() {
                "palette" => {
                    let colors: Vec<String> = val
                        .split([',', ' '])
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(|c| parse_hex_color(c, false).unwrap_or_else(|_| c.to_string()))
                        .collect();
                    let colors = if colors.len() == 1 {
                        match crate::pages::notes::mindmap::model::palette_by_key(&colors[0].to_lowercase()) {
                            Some(p) => p.iter().map(|x| x.to_string()).collect(),
                            None => colors,
                        }
                    } else {
                        colors
                    };
                    if !colors.is_empty() {
                        s.palette = colors;
                        changes.push("palette".to_string());
                    }
                }
                "node_fill" | "node_stroke" | "text_color" | "line_color" | "bg" => {
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
                        "node_fill" => s.node_fill = color,
                        "node_stroke" => s.node_stroke = color,
                        "text_color" => s.text_color = color,
                        "line_color" => s.line_color = color,
                        _ => s.bg = color,
                    }
                    changes.push(key.clone());
                }
                "font_size" | "radius" | "padding" | "line_width" | "line_dash" | "max_node_w" => {
                    let Some(n) = num() else {
                        err = Some(format!("bad \"{key}\" \"{val}\" — a number"));
                        return;
                    };
                    match key.as_str() {
                        "font_size" => s.font_size = n,
                        "radius" => s.radius = n,
                        "padding" => s.padding = n,
                        "line_width" => s.line_width = n,
                        "line_dash" => s.line_dash = n,
                        _ => s.max_node_w = n,
                    }
                    changes.push(format!("{key}={}", fnum(n)));
                }
                "weight" => {
                    s.weight = if val.eq_ignore_ascii_case("bold") { "bold".to_string() } else { String::new() };
                    changes.push("weight".to_string());
                }
                "show_icons" => {
                    s.show_icons = matches!(val.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
                    changes.push("show_icons".to_string());
                }
                other => {
                    err = Some(format!(
                        "unknown style key \"{other}\" — palette, node_fill, node_stroke, text_color, line_color, bg, \
                         font_size, weight, radius, padding, line_width, line_dash, show_icons, max_node_w"
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

/// Пары ключ-значение из объекта либо строки `k=v …`.
fn style_pairs(raw: &Json) -> Result<Vec<(String, String)>, String> {
    match raw {
        Json::Object(map) => Ok(map
            .iter()
            .map(|(k, v)| {
                let s = match v {
                    Json::Null => String::new(),
                    Json::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect()),
        Json::String(s) => {
            let t = s.trim();
            if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(t) {
                return style_pairs(&Json::Object(map));
            }
            let braced = if t.starts_with('{') { t.to_string() } else { format!("{{{t}}}") };
            let parsed = parse_attr_block(&braced).ok_or_else(|| format!("can't parse style \"{t}\" — pass an object"))?;
            Ok(parsed.0.into_iter().collect())
        }
        _ => Err("\"style\" must be an object {key: value}".to_string()),
    }
}

/// Поля узла из аргументов; возвращает описание изменений.
fn apply_node_fields(ctx: NotesCtx, handle: &MindmapHandle, id: &str, v: &Json) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if let Some(t) = raw_string(v, "text") {
        handle.set_text(id, &t);
        changes.push("text".to_string());
    }
    if let Some(n) = raw_string(v, "note") {
        handle.set_note(id, &n);
        changes.push("note".to_string());
    }
    if let Some(l) = raw_string(v, "link") {
        let page = if l.trim().is_empty() || l.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(resolve_page_ref(ctx, &l)?)
        };
        handle.set_link(id, page);
        changes.push("link".to_string());
    }
    if let Some(c) = raw_string(v, "color") {
        let color = if c.trim().is_empty() || c.eq_ignore_ascii_case("none") { None } else { Some(parse_hex_color(&c, false)?) };
        handle.set_color(id, color);
        changes.push("color".to_string());
    }
    if let Some(sh) = str_field(v, "shape") {
        let shape = NodeShape::parse(sh)
            .ok_or_else(|| format!("bad shape \"{sh}\" (auto | rect | rounded | pill | ellipse | text)"))?;
        handle.set_shape(id, shape);
        changes.push(format!("shape {}", shape.key()));
    }
    if let Some(i) = raw_string(v, "icon") {
        handle.set_icon(id, &i);
        changes.push("icon".to_string());
    }
    if let Some(c) = bool_field(v, "collapsed") {
        handle.set_collapsed(id, c);
        changes.push(format!("collapsed {c}"));
    }
    let (dx, dy) = (f32_field(v, "dx"), f32_field(v, "dy"));
    if dx.is_some() || dy.is_some() {
        let (cur_dx, cur_dy) = handle.lock().node(id).map(|n| (n.dx, n.dy)).unwrap_or((0.0, 0.0));
        handle.offset(id, dx.unwrap_or(cur_dx) - cur_dx, dy.unwrap_or(cur_dy) - cur_dy);
        changes.push("offset".to_string());
    }
    Ok(changes)
}

fn mindmap_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | add_node | update_node | move_node | delete_node | \
         add_link | delete_link | set_layout | set_style | from_list | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let root = str_field(v, "title").unwrap_or(&tr!("notes.mindmap.root")).to_string();
            let doc = match raw_string(v, "outline").filter(|o| !o.trim().is_empty()) {
                Some(outline) => MindmapDoc::from_outline(&outline, &root),
                None => MindmapDoc::template(&root),
            };
            let mut doc = doc;
            if let Some(d) = str_field(v, "direction") {
                doc.layout.direction =
                    Direction::parse(d).ok_or_else(|| format!("bad direction \"{d}\" (right | left | both | down | radial)"))?;
            }
            let id = ctx.create_mindmap(doc);
            let (pos, idx) = embed_object_with(ctx, &pid, "mindmap", &id, v, None)?;
            let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &id) else {
                return Err("map vanished".to_string());
            };
            Ok(format!(
                "created mind map {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                map_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            Ok(format!("{}{}\n", map_text(&id, &handle), object_page_line(ctx, "mindmap", &id)))
        }
        "add_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let parent = match str_field(v, "parent") {
                Some(p) => resolve_node(&handle, p)?,
                None => handle.lock().root_id(),
            };
            let text = str_field(v, "text").unwrap_or("").to_string();
            let index = usize_field(v, "index");
            let mut node = None;
            handle.edit(|d| node = d.add_node(&parent, &text, index));
            let node = node.ok_or("parent vanished")?;
            handle.editing.set(None);
            apply_node_fields(ctx, &handle, &node, v)?;
            Ok(format!("added node {node} \"{text}\"\n{}", map_text(&id, &handle)))
        }
        "update_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let changes = apply_node_fields(ctx, &handle, &node, v)?;
            if changes.is_empty() {
                return Err("nothing to update: pass text, note, link, color, shape, icon, collapsed, dx or dy".to_string());
            }
            Ok(format!("updated node {node}: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "move_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let parent = resolve_node(&handle, str_field(v, "parent").ok_or("missing \"parent\"")?)?;
            if !handle.reparent(&node, &parent, usize_field(v, "index")) {
                return Err("move failed: the root can't be moved and a node can't go into its own subtree".to_string());
            }
            Ok(format!("moved node {node} under {parent}\n{}", map_text(&id, &handle)))
        }
        "delete_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let n = handle.delete(&node);
            if n == 0 {
                return Err("the root node can't be deleted — delete the whole map with op=delete".to_string());
            }
            Ok(format!("deleted {n} node{}\n{}", if n == 1 { "" } else { "s" }, map_text(&id, &handle)))
        }
        "add_link" | "delete_link" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let from = resolve_node(&handle, str_field(v, "from").ok_or("missing \"from\"")?)?;
            let to = resolve_node(&handle, str_field(v, "to").ok_or("missing \"to\"")?)?;
            let ok = if op == "add_link" {
                handle.add_link(&from, &to, str_field(v, "label").unwrap_or(""))
            } else {
                handle.delete_link(&from, &to)
            };
            if !ok {
                return Err(if op == "add_link" {
                    "link not added: the nodes are the same or the link already exists".to_string()
                } else {
                    "no such link".to_string()
                });
            }
            Ok(format!("{op} {from} → {to}\n{}", map_text(&id, &handle)))
        }
        "set_layout" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let mut changes = Vec::new();
            let direction = match str_field(v, "direction") {
                Some(d) => Some(Direction::parse(d).ok_or_else(|| format!("bad direction \"{d}\" (right | left | both | down | radial)"))?),
                None => None,
            };
            let curve = match str_field(v, "curve") {
                Some(c) => Some(Curve::parse(c).ok_or_else(|| format!("bad curve \"{c}\" (bezier | straight | elbow)"))?),
                None => None,
            };
            let h_gap = f32_field(v, "h_gap");
            let v_gap = f32_field(v, "v_gap");
            if let Some(d) = direction {
                changes.push(format!("direction {}", d.key()));
            }
            if let Some(c) = curve {
                changes.push(format!("curve {}", c.key()));
            }
            if let Some(g) = h_gap {
                changes.push(format!("h_gap {}", fnum(g)));
            }
            if let Some(g) = v_gap {
                changes.push(format!("v_gap {}", fnum(g)));
            }
            if !changes.is_empty() {
                handle.set_layout(|l| {
                    if let Some(d) = direction {
                        l.direction = d;
                    }
                    if let Some(c) = curve {
                        l.curve = c;
                    }
                    if let Some(g) = h_gap {
                        l.h_gap = g;
                    }
                    if let Some(g) = v_gap {
                        l.v_gap = g;
                    }
                });
            }
            if bool_field(v, "reset").unwrap_or(false) {
                handle.reset_offsets();
                changes.push("offsets reset".to_string());
            }
            if changes.is_empty() {
                return Err("nothing to change: pass direction, curve, h_gap, v_gap or reset".to_string());
            }
            Ok(format!("layout: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "set_style" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let changes = apply_map_style(&handle, v)?;
            if changes.is_empty() {
                return Err("nothing to change: pass style with palette, colors, font_size, radius, padding, lines or show_icons".to_string());
            }
            Ok(format!("style: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "from_list" => {
            let pid = page_arg(ctx, v, "page")?;
            let mut model = load_model(ctx, &pid);
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\" (list or heading to convert)")?)?;
            let md = block_markdown(&model.blocks[i]);
            let root = str_field(v, "title").unwrap_or(&tr!("notes.mindmap.root")).to_string();
            let doc = MindmapDoc::from_outline(&md, &root);
            let nodes = doc.nodes.len();
            let id = ctx.create_mindmap(doc);
            let geom = model.blocks[i].attrs.clone();
            let embed = format!("![[mindmap:{id}]]{{h={}}}", fnum(embeds::default_object_h("mindmap")));
            let idx = replace_block(&mut model, i, &embed)?;
            // Врезка встаёт на место блока и наследует его координаты.
            for (k, val) in geom.0.iter() {
                if free::is_geom_key(k) && model.blocks[idx[0]].attrs.get(k).is_none() {
                    model.blocks[idx[0]].attrs.set(k.clone(), val.clone());
                }
            }
            store_model(ctx, &pid, &model)?;
            let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &id) else {
                return Err("map vanished".to_string());
            };
            Ok(format!(
                "block #{i} → mind map with {nodes} nodes\n{}{}\n",
                map_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "delete" => {
            let (id, _) = mindmap_handle(ctx, v)?;
            delete_object(ctx, "mindmap", &id)
        }
        other => Err(format!(
            "unknown mindmap op \"{other}\" (create | read | add_node | update_node | move_node | \
             delete_node | add_link | delete_link | set_layout | set_style | from_list | delete)"
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chart
// ─────────────────────────────────────────────────────────────────────────────

fn chart_handle(ctx: NotesCtx, v: &Json) -> Result<(String, ChartHandle), String> {
    match resolve_object(ctx, v, "chart", "chart")? {
        LiveObject::Chart { id, handle } => Ok((id, handle)),
        _ => Err("not a chart".to_string()),
    }
}

/// Число из поля: и `12`, и `"12"` — модели шлют по-разному.
fn f64_field(v: &Json, key: &str) -> Option<f64> {
    match v.get(key)? {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Ряд чисел из поля: массив либо строка «12, 24, 18».
fn data_field(v: &Json, key: &str) -> Option<Vec<f64>> {
    match v.get(key)? {
        Json::Array(a) => Some(
            a.iter()
                .map(|x| match x {
                    Json::Number(n) => n.as_f64().unwrap_or(0.0),
                    Json::String(s) => chart_model::parse_num(s),
                    _ => 0.0,
                })
                .collect(),
        ),
        Json::String(s) => Some(chart_model::parse_values(s)),
        Json::Number(n) => Some(vec![n.as_f64().unwrap_or(0.0)]),
        _ => None,
    }
}

/// Ряд графика по id или названию.
fn resolve_series(handle: &ChartHandle, s: &str) -> Result<String, String> {
    handle
        .lock()
        .find_series(s)
        .map(|x| x.id.clone())
        .ok_or_else(|| format!("series \"{s}\" not found — ids and names are in chart op=read"))
}

/// Вид графика из поля `kind`.
fn chart_kind_field(v: &Json, key: &str) -> Result<Option<ChartKind>, String> {
    match str_field(v, key) {
        Some(k) => Ok(Some(
            ChartKind::parse(k).ok_or_else(|| format!("bad kind \"{k}\" (line | bar | pie | radar | gauge)"))?,
        )),
        None => Ok(None),
    }
}

/// Текст графика: шапка с видом и настройками, подписи и ряды таблицей.
fn chart_text(id: &str, handle: &ChartHandle) -> String {
    let doc = handle.lock();
    let o = &doc.options;
    let mut out = format!(
        "chart:{id} · kind: {} · series: {} · points: {}{}\n",
        doc.kind.key(),
        doc.series.len(),
        doc.categories.len(),
        if doc.title.is_empty() { String::new() } else { format!(" · title: \"{}\"", doc.title) }
    );
    match doc.kind {
        ChartKind::Gauge => {
            out.push_str(&format!(
                "  value {} · range {}…{}{}{}\n",
                chart_model::fmt_num(doc.gauge_value()),
                chart_model::fmt_num(o.gauge_min),
                chart_model::fmt_num(o.gauge_max),
                if o.unit.is_empty() { String::new() } else { format!(" · unit \"{}\"", o.unit) },
                if o.zones.is_empty() { String::new() } else { format!(" · {} zone(s)", o.zones.len()) }
            ));
        }
        _ => {
            out.push_str(&format!("  labels: {}\n", doc.categories.join(", ")));
            for s in &doc.series {
                out.push_str(&format!(
                    "  series {} \"{}\" · {}{}\n",
                    s.id,
                    s.name,
                    chart_model::values_text(&s.data),
                    if s.color.is_empty() { String::new() } else { format!(" · color {}", s.color) }
                ));
            }
        }
    }
    out.push_str(&format!("  legend {} · tooltip {} · animate {}", o.legend.key(), o.tooltip, o.animate));
    match doc.kind {
        ChartKind::Line => out.push_str(&format!(" · smooth {} · points {} · area {}", o.smooth, o.points, o.area)),
        ChartKind::Bar => out.push_str(&format!(
            " · stacked {} · horizontal {} · value_labels {}",
            o.stacked, o.horizontal, o.value_labels
        )),
        ChartKind::Pie => out.push_str(&format!(
            " · donut {} · labels {} · percentage {}",
            o.donut,
            o.pie_labels.key(),
            o.percentage
        )),
        ChartKind::Radar => out.push_str(&format!(
            " · grid {} · levels {} · max {}",
            if o.radar_circle { "circle" } else { "polygon" },
            o.radar_levels,
            o.radar_max.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string())
        )),
        ChartKind::Gauge => out.push_str(&format!(" · needle {} · ticks {} · labels {}", o.needle, o.ticks, o.gauge_labels)),
    }
    if doc.kind.has_axes() {
        out.push_str(&format!(
            " · grid {} · y {}…{}",
            o.grid,
            o.y_min.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string()),
            o.y_max.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string())
        ));
    }
    out.push('\n');
    out
}

/// Данные графика из аргументов: таблица целиком либо подписи и ряд.
fn apply_chart_data(handle: &ChartHandle, v: &Json) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if let Some(table) = raw_string(v, "table").filter(|t| !t.trim().is_empty()) {
        let kind = handle.kind();
        let fresh = ChartDoc::from_table(&table, kind);
        handle.edit(|d| {
            d.categories = fresh.categories.clone();
            d.series = fresh.series.clone();
        });
        changes.push(format!("{} labels, {} series from the table", fresh.categories.len(), fresh.series.len()));
    }
    if let Some(labels) = list_field(v, "categories") {
        handle.set_categories(labels.clone());
        changes.push(format!("{} labels", labels.len()));
    }
    if let Some(data) = data_field(v, "data") {
        // Ряд по имени/id, а без него — первый: у круговой и шкалы он
        // единственный, и указывать его каждый раз бессмысленно.
        let sid = match str_field(v, "series") {
            Some(s) => resolve_series(handle, s)?,
            None => handle.lock().series.first().map(|s| s.id.clone()).unwrap_or_default(),
        };
        match f64_field(v, "value").zip(usize_field(v, "index")) {
            Some((value, i)) => {
                handle.set_value(&sid, i, value);
                changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
            }
            None => {
                handle.set_series_data(&sid, data.clone());
                changes.push(format!("{} values", data.len()));
            }
        }
    } else if let Some(value) = f64_field(v, "value") {
        let sid = match str_field(v, "series") {
            Some(s) => resolve_series(handle, s)?,
            None => handle.lock().series.first().map(|s| s.id.clone()).unwrap_or_default(),
        };
        let i = usize_field(v, "index").unwrap_or(0);
        handle.set_value(&sid, i, value);
        changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
    }
    Ok(changes)
}

/// Оформление графика из объекта `style`.
fn apply_chart_style(handle: &ChartHandle, v: &Json) -> Result<Vec<String>, String> {
    let Some(raw) = v.get("style") else { return Ok(Vec::new()) };
    let pairs = style_pairs(raw)?;
    let mut changes = Vec::new();
    let mut err = None;
    handle.set_options(|o| {
        for (k, val) in &pairs {
            let key = k.trim().to_ascii_lowercase();
            let val = val.trim();
            let flag = |err: &mut Option<String>| -> Option<bool> {
                match val.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "1" | "on" => Some(true),
                    "false" | "no" | "0" | "off" => Some(false),
                    _ => {
                        *err = Some(format!("bad \"{key}\" \"{val}\" — true or false"));
                        None
                    }
                }
            };
            let num = |err: &mut Option<String>| -> Option<f64> {
                match val.parse::<f64>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        *err = Some(format!("bad \"{key}\" \"{val}\" — a number"));
                        None
                    }
                }
            };
            // Пустое значение и «auto» снимают границу оси.
            let auto = val.is_empty() || val.eq_ignore_ascii_case("auto") || val.eq_ignore_ascii_case("none");
            match key.as_str() {
                "legend" => match LegendPos::parse(val) {
                    Some(p) => o.legend = p,
                    None => {
                        err = Some(format!("bad legend \"{val}\" (top | bottom | left | right | none)"));
                        return;
                    }
                },
                "pie_labels" | "labels" => match PieLabels::parse(val) {
                    Some(p) => o.pie_labels = p,
                    None => {
                        err = Some(format!("bad pie_labels \"{val}\" (outside | inside | none)"));
                        return;
                    }
                },
                "tooltip" | "animate" | "grid" | "smooth" | "points" | "stacked" | "horizontal"
                | "value_labels" | "percentage" | "radar_circle" | "needle" | "ticks" | "gauge_labels" => {
                    let Some(b) = flag(&mut err) else { return };
                    match key.as_str() {
                        "tooltip" => o.tooltip = b,
                        "animate" => o.animate = b,
                        "grid" => o.grid = b,
                        "smooth" => o.smooth = b,
                        "points" => o.points = b,
                        "stacked" => o.stacked = b,
                        "horizontal" => o.horizontal = b,
                        "value_labels" => o.value_labels = b,
                        "percentage" => o.percentage = b,
                        "radar_circle" => o.radar_circle = b,
                        "needle" => o.needle = b,
                        "ticks" => o.ticks = b,
                        _ => o.gauge_labels = b,
                    }
                }
                "area" | "donut" | "bar_radius" | "radar_levels" | "gauge_min" | "gauge_max" => {
                    let Some(n) = num(&mut err) else { return };
                    match key.as_str() {
                        "area" => o.area = n as f32,
                        "donut" => o.donut = n as f32,
                        "bar_radius" => o.bar_radius = n as f32,
                        "radar_levels" => o.radar_levels = n.max(1.0) as usize,
                        "gauge_min" => o.gauge_min = n,
                        _ => o.gauge_max = n,
                    }
                }
                "y_min" | "y_max" | "radar_max" => {
                    let value = if auto {
                        None
                    } else {
                        let Some(n) = num(&mut err) else { return };
                        Some(n)
                    };
                    match key.as_str() {
                        "y_min" => o.y_min = value,
                        "y_max" => o.y_max = value,
                        _ => o.radar_max = value,
                    }
                }
                "x_title" | "y_title" | "unit" => {
                    let text = if auto { String::new() } else { val.to_string() };
                    match key.as_str() {
                        "x_title" => o.x_title = text,
                        "y_title" => o.y_title = text,
                        _ => o.unit = text,
                    }
                }
                "zones" => {
                    match parse_zones(val) {
                        Ok(z) => o.zones = z,
                        Err(e) => {
                            err = Some(e);
                            return;
                        }
                    };
                }
                other => {
                    err = Some(format!(
                        "unknown style key \"{other}\" (legend, tooltip, animate, grid, x_title, y_title, \
                         y_min, y_max, smooth, points, area, stacked, horizontal, value_labels, bar_radius, \
                         donut, pie_labels, percentage, radar_circle, radar_levels, radar_max, gauge_min, \
                         gauge_max, needle, ticks, gauge_labels, unit, zones)"
                    ));
                    return;
                }
            }
            changes.push(key);
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(changes),
    }
}

/// Зоны шкалы: «0-50 green, 50-80 #E8A33D» либо JSON-массив
/// `[{"from":0,"to":50,"color":"green"}]`.
fn parse_zones(raw: &str) -> Result<Vec<GaugeZone>, String> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    if let Ok(Json::Array(items)) = serde_json::from_str::<Json>(t) {
        let mut out = Vec::new();
        for it in &items {
            let from = f64_field(it, "from").ok_or("zone needs \"from\"")?;
            let to = f64_field(it, "to").ok_or("zone needs \"to\"")?;
            let color = str_field(it, "color").unwrap_or("#4FBF7A");
            out.push(GaugeZone { from, to, color: parse_hex_color(color, false)? });
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    for part in t.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (range, color) = part.split_once(char::is_whitespace).unwrap_or((part, "#4FBF7A"));
        let (from, to) = range
            .split_once(['-', '…', ':'])
            .ok_or_else(|| format!("bad zone \"{part}\" — «from-to color», e.g. «0-50 green»"))?;
        out.push(GaugeZone {
            from: chart_model::parse_num(from),
            to: chart_model::parse_num(to),
            color: parse_hex_color(color.trim(), false)?,
        });
    }
    Ok(out)
}

fn chart_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | update | set_data | add_series | update_series | \
         delete_series | set_style | from_table | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let kind = chart_kind_field(v, "kind")?.unwrap_or(ChartKind::Line);
            let mut doc = match raw_string(v, "table").filter(|t| !t.trim().is_empty()) {
                Some(table) => ChartDoc::from_table(&table, kind),
                None => ChartDoc::template(kind, &tr!("notes.chart.series")),
            };
            if let Some(t) = str_field(v, "title") {
                doc.title = t.trim().to_string();
            }
            if let Some(labels) = list_field(v, "categories") {
                doc.categories = labels;
            }
            if let Some(data) = data_field(v, "data") {
                let name = str_field(v, "name").unwrap_or(&tr!("notes.chart.series")).to_string();
                doc.series = vec![chart_model::ChartSeries::new(&name, data)];
            }
            doc.sanitize();
            let id = ctx.create_chart(doc);
            let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &id) else {
                return Err("chart vanished".to_string());
            };
            apply_chart_style(&handle, v)?;
            let (pos, idx) = embed_object_with(ctx, &pid, "chart", &id, v, None)?;
            Ok(format!(
                "created chart {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                chart_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = chart_handle(ctx, v)?;
            Ok(format!(
                "{}{}{}\n",
                chart_text(&id, &handle),
                handle.lock().to_table(),
                object_page_line(ctx, "chart", &id)
            ))
        }
        "update" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let mut changes = Vec::new();
            if let Some(kind) = chart_kind_field(v, "kind")? {
                handle.set_kind(kind);
                changes.push(format!("kind {}", kind.key()));
            }
            if let Some(t) = str_field(v, "title") {
                handle.set_title(t);
                changes.push("title".to_string());
            }
            changes.extend(apply_chart_data(&handle, v)?);
            changes.extend(apply_chart_style(&handle, v)?);
            if changes.is_empty() {
                return Err("nothing to update: pass kind, title, categories, data, table or style".to_string());
            }
            Ok(format!("updated chart: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "set_data" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let changes = apply_chart_data(&handle, v)?;
            if changes.is_empty() {
                return Err("nothing to set: pass table, categories, data or value with index".to_string());
            }
            Ok(format!("data: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "add_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            if handle.kind().single_series() {
                return Err(format!(
                    "a {} chart shows one series — change its data instead (op=set_data) or switch the kind",
                    handle.kind().key()
                ));
            }
            let name = str_field(v, "name")
                .map(str::to_string)
                .unwrap_or_else(|| format!("{} {}", tr!("notes.chart.series"), handle.lock().series.len() + 1));
            let sid = handle.add_series(&name, data_field(v, "data").unwrap_or_default());
            if let Some(c) = str_field(v, "color") {
                handle.set_series_color(&sid, chart_color(c)?);
            }
            Ok(format!("added series {sid} \"{name}\"\n{}", chart_text(&id, &handle)))
        }
        "update_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let sid = match str_field(v, "series") {
                Some(s) => resolve_series(&handle, s)?,
                None => handle.lock().series.first().map(|s| s.id.clone()).ok_or("the chart has no series")?,
            };
            let mut changes = Vec::new();
            if let Some(name) = str_field(v, "name") {
                handle.rename_series(&sid, name);
                changes.push("name".to_string());
            }
            if let Some(c) = str_field(v, "color") {
                handle.set_series_color(&sid, chart_color(c)?);
                changes.push("color".to_string());
            }
            if let Some(data) = data_field(v, "data") {
                handle.set_series_data(&sid, data.clone());
                changes.push(format!("{} values", data.len()));
            }
            if let (Some(value), Some(i)) = (f64_field(v, "value"), usize_field(v, "index")) {
                handle.set_value(&sid, i, value);
                changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, color, data, or value with index".to_string());
            }
            Ok(format!("updated series {sid}: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "delete_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let sid = resolve_series(&handle, str_field(v, "series").ok_or("missing \"series\"")?)?;
            if !handle.delete_series(&sid) {
                return Err("the last series can't be deleted — delete the whole chart with op=delete".to_string());
            }
            Ok(format!("deleted series {sid}\n{}", chart_text(&id, &handle)))
        }
        "set_style" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let changes = apply_chart_style(&handle, v)?;
            if changes.is_empty() {
                return Err(
                    "nothing to change: pass style with legend, tooltip, animate, grid, axis titles, \
                     y_min/y_max, smooth, points, area, stacked, horizontal, value_labels, bar_radius, \
                     donut, pie_labels, percentage, radar_circle, radar_levels, radar_max, gauge_min, \
                     gauge_max, needle, ticks, gauge_labels, unit or zones"
                        .to_string(),
                );
            }
            Ok(format!("style: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "from_table" => {
            let pid = page_arg(ctx, v, "page")?;
            let mut model = load_model(ctx, &pid);
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\" (table to convert)")?)?;
            let md = block_markdown(&model.blocks[i]);
            if !md.trim_start().starts_with('|') {
                return Err(format!("block #{i} is not a table — chart op=from_table converts markdown tables"));
            }
            let kind = chart_kind_field(v, "kind")?.unwrap_or(ChartKind::Bar);
            let mut doc = ChartDoc::from_table(&md, kind);
            if let Some(t) = str_field(v, "title") {
                doc.title = t.trim().to_string();
            }
            let points = doc.categories.len();
            let rows = doc.series.len();
            let id = ctx.create_chart(doc);
            let geom = model.blocks[i].attrs.clone();
            let embed = format!("![[chart:{id}]]{{h={}}}", fnum(embeds::default_object_h("chart")));
            let idx = replace_block(&mut model, i, &embed)?;
            // Врезка встаёт на место таблицы и наследует её координаты.
            for (k, val) in geom.0.iter() {
                if free::is_geom_key(k) && model.blocks[idx[0]].attrs.get(k).is_none() {
                    model.blocks[idx[0]].attrs.set(k.clone(), val.clone());
                }
            }
            store_model(ctx, &pid, &model)?;
            let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &id) else {
                return Err("chart vanished".to_string());
            };
            apply_chart_style(&handle, v)?;
            Ok(format!(
                "block #{i} → chart with {rows} series over {points} points\n{}{}\n",
                chart_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "delete" => {
            let (id, _) = chart_handle(ctx, v)?;
            delete_object(ctx, "chart", &id)
        }
        other => Err(format!(
            "unknown chart op \"{other}\" (create | read | update | set_data | add_series | \
             update_series | delete_series | set_style | from_table | delete)"
        )),
    }
}

/// Цвет ряда или доли: `none` снимает его (тогда цвет берётся из палитры).
fn chart_color(s: &str) -> Result<Option<String>, String> {
    if s.trim().is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    parse_hex_color(s, false).map(Some)
}

// ─────────────────────────────────────────────────────────────────────────────
// Calendar
// ─────────────────────────────────────────────────────────────────────────────

/// Дата из поля: `yyyy-mm-dd`, `today`, `tomorrow`.
fn day_arg(v: &Json, key: &str) -> Result<Option<i64>, String> {
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
                "first_weekday" | "hour_from" | "hour_to" | "slot_min" => {
                    let Ok(n) = val.trim().parse::<u32>() else {
                        err = Some(format!("bad \"{key}\" \"{val}\" — a whole number"));
                        return;
                    };
                    match key.as_str() {
                        "first_weekday" => s.first_weekday = n,
                        "hour_from" => s.hour_from = n,
                        "hour_to" => s.hour_to = n,
                        _ => s.slot_min = n,
                    }
                    changes.push(format!("{key}={n}"));
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
                         hour_from, hour_to, slot_min, compact, font_size, weekend_tint, today_color, header_bg, \
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
        e.repeat = Repeat::parse(r).ok_or_else(|| format!("bad repeat \"{r}\" (none | daily | weekly | monthly | yearly)"))?;
        changes.push("repeat".to_string());
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

fn calendar_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
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
                        "calendar:{id} · view: {} · anchor: {} · calendars: {} · first weekday: {} · hours {}–{} · slot {} min · events as {}{}{}\n",
                        d.view.key(),
                        d.anchor,
                        if d.calendars.is_empty() { "all".to_string() } else { d.calendars.join(", ") },
                        d.style.first_weekday,
                        d.style.hour_from,
                        d.style.hour_to,
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
                     color, note, done, repeat, until or link"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::pages::notes::project;

    /// Контекст заметок над временным проектом. Каталоги строк — русские:
    /// названия колонок шаблона и корня журнала берутся через `tr!`.
    fn ctx() -> NotesCtx {
        syngui::i18n::register_catalogs(&crate::i18n::CATALOGS);
        syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));
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

    /// Живой чат MyLife (08.09.2026): модель дважды собирала `add_card` без
    /// `op` и с текстом карточки в `card`. Ошибка без `op` называет сигнатуры
    /// операций, а `card` у `add_card` принимается как заголовок.
    #[test]
    fn kanban_add_card_tolerates_card_as_title_and_explains_ops() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Доска"})));
        call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["К выполнению", "Готово"]}));

        let err = dispatch(ctx, "kanban", &serde_json::json!({"page": &page, "column": "К выполнению", "card": "Порядок страниц"})).unwrap_err();
        assert!(err.starts_with("missing \"op\". kanban ops"), "{err}");
        assert!(err.contains("add_card {title = the new card's text, column, md, priority, tags, due, duration, start, end, repeat, before}"), "{err}");

        let out = call(
            ctx,
            "kanban",
            serde_json::json!({"op": "add_card", "page": &page, "column": "К выполнению", "card": "Порядок страниц", "priority": "medium", "tags": ["Баг", "Заметки"]}),
        );
        assert!(out.contains("added card to \"К выполнению\"") && out.contains("\"Порядок страниц\"") && out.contains("tags: Баг, Заметки"), "{out}");
        // Явный title важнее: "card" тогда — просто лишнее поле.
        let out = call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "Заголовок", "card": "Не заголовок"}));
        assert!(out.contains("\"Заголовок\"") && !out.contains("\"Не заголовок\""), "{out}");

        let err = dispatch(ctx, "kanban", &serde_json::json!({"op": "add_card", "page": &page})).unwrap_err();
        assert!(err.starts_with("missing \"title\"") && err.contains("existing card"), "{err}");
        let err = dispatch(ctx, "kanban", &serde_json::json!({"op": "add_kard", "page": &page, "title": "x"})).unwrap_err();
        assert!(err.starts_with("unknown kanban op \"add_kard\". kanban ops") && err.contains("update_card {card, title"), "{err}");
    }

    /// Повторный `create` с тем же названием обязан сказать, что страница
    /// уже есть: молчаливое переименование в «… 2» выглядит как успех, и
    /// агент, не увидевший свою же прошлую страницу, зацикливается.
    #[test]
    fn create_reports_a_title_clash() {
        let ctx = ctx();
        let first = call(ctx, "create", serde_json::json!({"title": "Туду — канбан"}));
        assert!(!first.contains("already exists"), "первой странице ругаться не на что: {first}");
        let second = call(ctx, "create", serde_json::json!({"title": "Туду — канбан"}));
        assert!(second.contains("already exists"), "{second}");
        assert!(second.contains(&page_id(&first)), "нужен id существующей страницы: {second}");
        assert!(second.contains("Туду — канбан 2"), "{second}");
        // Одноимённые страницы у разных родителей законны — там не ругаемся.
        let parent = page_id(&call(ctx, "create", serde_json::json!({"title": "Работа"})));
        let child = call(
            ctx,
            "create",
            serde_json::json!({"title": "Туду — канбан", "parent": parent}),
        );
        assert!(!child.contains("already exists"), "{child}");
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
        let merged = with_sidecar(md, &edited);
        assert!(merged.contains("Новый абзац"), "{merged}");
        assert!(merged.contains("0 {w=200 x=40 y=300}"), "заголовок потерял координаты: {merged}");
        assert!(!merged.contains("x=400"), "переписанный абзац не должен наследовать координаты: {merged}");
        // Вставка блока перед заголовком не сбивает его координаты.
        let shifted = format!("Преамбула\n\n{plain}");
        let merged = with_sidecar(md, &shifted);
        assert!(merged.contains("1 {w=200 x=40 y=300}"), "{merged}");
        assert!(merged.contains("2 {w=200 x=400 y=60}"), "{merged}");
    }

    /// Стили абзаца живут в том же хвосте, что и координаты: агент их не
    /// видит, а нетронутый абзац их не теряет.
    #[test]
    fn plain_markdown_strips_style_sidecar_and_write_keeps_it() {
        let md = "Первый\n\nВторой\n\n```doc-layout\n1 {bg=#243149 color=#FF8800 x=40 y=60}\n```\n";
        let plain = plain_markdown(md);
        assert!(!plain.contains("doc-layout") && !plain.contains("bg="), "{plain}");
        let merged = with_sidecar(md, &format!("Нулевой\n\n{plain}"));
        assert!(merged.contains("2 {bg=#243149 color=#FF8800 x=40 y=60}"), "{merged}");
        // Правка через find/replace идёт по блоку — атрибуты остаются.
        let mut model = parse_document(md);
        let n = replace_in_blocks(&mut model, "Второй", "Второй и главный", false).unwrap();
        assert_eq!(n, 1);
        let out = serialize_document(&model);
        assert!(out.contains("Второй и главный") && out.contains("1 {bg=#243149 color=#FF8800 x=40 y=60}"), "{out}");
        let err = replace_in_blocks(&mut model, "Первый\n\nВторой", "x", false).unwrap_err();
        assert!(err.contains("spans several blocks"), "{err}");
        assert!(replace_in_blocks(&mut model, "нет такого", "x", false).unwrap_err().contains("not found"));
    }

    /// Блоки: список с геометрией, вставка в позицию, атрибуты, закрепление,
    /// перенос и удаление — всё по индексам, с сохранением остального.
    #[test]
    fn blocks_ops_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Холст", "content": "# Схема\n\nАбзац\n"})));
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("#0 heading1 \"Схема\" · flow h=~"), "{listed}");
        assert!(listed.contains("#1 paragraph \"Абзац\""), "{listed}");

        // Вставка с геометрией после заголовка.
        let out = call(ctx, "blocks", serde_json::json!({"op": "insert", "page": &page, "md": "Заметка", "after": 0, "x": 100, "y": 200, "w": 240}));
        assert!(out.contains("inserted #1 at block #1") && out.contains("x=100 y=200 w=240"), "{out}");
        assert!(ctx.page_markdown(&page).contains("1 {w=240 x=100 y=200}"), "{}", ctx.page_markdown(&page));

        // Атрибуты: стиль ставится и снимается; чужой ключ — ошибка.
        let out = call(ctx, "blocks", serde_json::json!({"op": "set_attrs", "page": &page, "block": "find:Заметка", "attrs": {"bg": "#243149", "align": "center", "size": 18}}));
        assert!(out.contains("align=center") && out.contains("bg=#243149"), "{out}");
        assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "set_attrs", "page": &page, "block": 1, "attrs": {"font": "x"}})).is_err());
        call(ctx, "blocks", serde_json::json!({"op": "set_attrs", "page": &page, "block": 1, "attrs": {"size": null}}));
        assert!(!ctx.page_markdown(&page).contains("size="));
        // Замена markdown блока сохраняет его координаты и стиль.
        call(ctx, "blocks", serde_json::json!({"op": "set_markdown", "page": &page, "block": 1, "md": "Заметка подробнее"}));
        let md = ctx.page_markdown(&page);
        assert!(md.contains("Заметка подробнее") && md.contains("align=center") && md.contains("x=100"), "{md}");

        // pin без координат — под нижним закреплённым; unpin — снова в потоке; move меняет порядок.
        let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 2}));
        assert!(out.contains("pinned at x=100 y="), "{out}");
        call(ctx, "blocks", serde_json::json!({"op": "unpin", "page": &page, "block": 2}));
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("#2 paragraph \"Абзац\" · flow"), "{listed}");
        call(ctx, "blocks", serde_json::json!({"op": "move", "page": &page, "block": 2, "index": 0}));
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.starts_with("#0 paragraph \"Абзац\""), "{listed}");
        let out = call(ctx, "blocks", serde_json::json!({"op": "delete", "page": &page, "block": 0}));
        assert!(out.contains("2 blocks left"), "{out}");
        let read = call(ctx, "read", serde_json::json!({"page": &page, "blocks": true}));
        assert!(read.contains("--- Blocks") && read.contains("grid: dots step 20 · snap: on step 5"), "{read}");
        assert!(ctx.page(&page).unwrap().handle.history_state().get_untracked().0);
    }

    /// Переход поток → холст сохраняет расположение: блоки получают
    /// координаты колонкой в порядке документа, подсказка про свободную
    /// раскладку исчезает; `create layout=free` с контентом рождает страницу
    /// уже разложенной (07.09.2026: страница «Тестовая» без этого легла кашей).
    #[test]
    fn switching_to_free_layout_pins_blocks_in_a_column() {
        let ctx = ctx();
        // Новая страница — холст по умолчанию, но без закреплённых блоков
        // она документ: ни закрепления, ни подсказки.
        let out = call(ctx, "create", serde_json::json!({"title": "Переход", "content": "# Заголовок\n\nАбзац\n\n![[shape:rect]]\n"}));
        assert!(!out.contains("pinned") && !out.contains("!! free layout"), "{out}");
        let page = page_id(&out);
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("· flow") && !listed.contains("!! free layout"), "{listed}");
        // Поток → холст: блоки встают колонкой.
        call(ctx, "update", serde_json::json!({"page": &page, "layout": "flow"}));
        let out = call(ctx, "update", serde_json::json!({"page": &page, "layout": "free"}));
        assert!(out.contains("layout: free") && out.contains("3 blocks pinned in a column"), "{out}");
        assert!(!out.contains("!! free layout"), "{out}");
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("#0 heading1 \"Заголовок\" · x=40 y=40 w=520"), "{listed}");
        assert!(!listed.contains("· flow"), "{listed}");
        // Каждый следующий блок ниже предыдущего; фигура получила свою высоту.
        let model = load_model(ctx, &page);
        let rects: Vec<_> = model.blocks.iter().map(|b| block_rect(b).unwrap()).collect();
        assert!(rects[1].1 >= rects[0].1 + rects[0].3 + 24.0, "{rects:?}");
        assert!(rects[2].1 >= rects[1].1 + rects[1].3 + 24.0, "{rects:?}");
        assert!(free::height_of(&model.blocks[2].attrs).is_some(), "{}", ctx.page_markdown(&page));
        // Повторное включение холста ничего не перекладывает; обратно в поток
        // координаты остаются.
        call(ctx, "update", serde_json::json!({"page": &page, "layout": "flow"}));
        let out = call(ctx, "update", serde_json::json!({"page": &page, "layout": "free"}));
        assert!(!out.contains("pinned in a column"), "{out}");

        // create layout=free + content — сразу колонкой.
        let out = call(ctx, "create", serde_json::json!({"title": "Сразу холст", "layout": "free", "content": "Один\n\nДва\n"}));
        assert!(out.contains("2 blocks pinned in a column") && !out.contains("!! free layout"), "{out}");

        // Смесь на странице по умолчанию: фигура закреплена, текст в потоке —
        // подсказка появляется и пропадает после arrange.
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Смесь", "content": "Текст\n"})));
        let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 40, "y": 40, "w": 200, "h": 100}));
        assert!(out.contains("!! free layout: 1 of 2 blocks have no x/y (#0)") && out.contains("ends at y=140"), "{out}");
        let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
        assert!(out.contains("arranged 1 blocks in a column at x=40") && out.contains("y=164") && !out.contains("!! free layout"), "{out}");
    }

    /// `blocks op=arrange`: одна команда кладёт неприкреплённые блоки под
    /// закреплённые; `only=all` перекладывает всё; предупреждение о наложении
    /// появляется у pin/shape и не мешает линиям.
    #[test]
    fn arrange_stacks_blocks_and_overlaps_are_reported() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Раскладка-2", "layout": "free", "content": "Один\n\nДва\n"})));
        // Дописанный контент без геометрии → подсказка; arrange её снимает.
        call(ctx, "update", serde_json::json!({"page": &page, "content": "Три\n", "mode": "append"}));
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("!! free layout: 1 of 3"), "{listed}");
        let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
        assert!(out.contains("arranged 1 blocks in a column at x=40"), "{out}");
        assert!(!out.contains("!! free layout") && !out.contains("!! overlaps"), "{out}");
        // Новый блок встал под нижним закреплённым.
        let model = load_model(ctx, &page);
        let r1 = block_rect(&model.blocks[1]).unwrap();
        let r2 = block_rect(&model.blocks[2]).unwrap();
        assert!(r2.1 >= r1.1 + r1.3 + 24.0, "{r1:?} {r2:?}");
        // Повтор — нечего раскладывать; only=all перекладывает всё заново.
        let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
        assert!(out.contains("nothing to arrange"), "{out}");
        let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page, "only": "all", "x": 100, "y": 100, "w": 300, "gap": 0}));
        assert!(out.contains("arranged 3 blocks in a column at x=100 (gap 0)") && out.contains("x=100 y=100 w=300"), "{out}");
        assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "arrange", "page": &page, "only": "some"})).is_err());
        assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "arrange", "page": &page, "gap": 900})).is_err());

        // Наложение: фигура поверх первого блока — предупреждение с его рамкой.
        let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 100, "y": 100, "w": 200, "h": 100}));
        assert!(out.contains("!! overlaps #0 (x=100 y=100 w=300 h="), "{out}");
        // Линия поверх тех же блоков — не в счёт.
        let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "arrow", "x1": 100, "y1": 100, "x2": 300, "y2": 200}));
        assert!(!out.contains("!! overlaps"), "{out}");
        // pin в свободное место — тихо; pin поверх фигуры — с предупреждением.
        let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 1000, "y": 1000, "w": 200}));
        assert!(!out.contains("!! overlaps"), "{out}");
        let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 150, "y": 150, "w": 200}));
        assert!(out.contains("!! overlaps") && out.contains("#3 (x=100 y=100 w=200 h=100)"), "{out}");
    }

    /// Фигуры: рамка из x y w h, линия из абсолютных концов (рамка считается
    /// сама), connect между закреплёнными блоками, ошибка для незакреплённого.
    #[test]
    fn shapes_create_update_and_connect() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Фигуры", "content": "Старт\n\nФиниш\n"})));
        let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 40, "y": 40, "w": 200, "h": 100, "fill": "blue", "sw": 3}));
        assert!(out.contains("#2 shape:rect · x=40 y=40 w=200 h=100") && out.contains("fill=#") && out.contains("sw=3"), "{out}");
        let md = ctx.page_markdown(&page);
        assert!(md.contains("![[shape:rect]]{fill=") && md.contains("2 {h=100 w=200 x=40 y=40}"), "{md}");

        // Линия по абсолютным концам: рамка — bbox с полем 12.
        let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "arrow", "x1": 100, "y1": 300, "x2": 300, "y2": 340}));
        assert!(out.contains("shape:arrow · x=88 y=288 w=224 h=64 · from (100,300) to (300,340)"), "{out}");
        // update концов пересчитывает рамку; смена вида сохраняет оформление.
        let out = call(ctx, "shape", serde_json::json!({"op": "update", "page": &page, "block": 3, "x2": 500, "y2": 300, "kind": "line", "stroke": "red", "dash": 6}));
        assert!(out.contains("shape:line · x=88 y=288 w=424 h=24 · from (100,300) to (500,300)") && out.contains("dash=6"), "{out}");

        // connect: незакреплённые блоки — ошибка; закрепим и соединим.
        let err = dispatch(ctx, "shape", &serde_json::json!({"op": "connect", "page": &page, "from": 0, "to": 1})).unwrap_err();
        assert!(err.contains("no coordinates"), "{err}");
        call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 0, "y": 0, "w": 100, "h": 40}));
        call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 1, "x": 400, "y": 0, "w": 100, "h": 40}));
        let out = call(ctx, "shape", serde_json::json!({"op": "connect", "page": &page, "from": "find:Старт", "to": "find:Финиш"}));
        assert!(out.contains("connected #0 → #1 with arrow") && out.contains("from (100,20) to (400,20)"), "{out}");
        let out = call(ctx, "shape", serde_json::json!({"op": "delete", "page": &page, "block": 4}));
        assert!(out.contains("deleted #4 shape:arrow"), "{out}");
        assert!(dispatch(ctx, "shape", &serde_json::json!({"op": "delete", "page": &page, "block": 0})).is_err());
    }

    /// Страница: сетка/привязка через update, объекты — в позицию с высотой,
    /// стиль доски и зум диаграммы.
    #[test]
    fn layout_objects_style_and_zoom_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Раскладка", "content": "Один\n\nДва\n"})));
        let out = call(ctx, "update", serde_json::json!({"page": &page, "grid": "lines", "grid_step": 32, "snap": false}));
        assert!(out.contains("grid: lines") && out.contains("snap: off"), "{out}");
        let l = ctx.page_layout(&page);
        assert_eq!((l.grid, l.grid_step, l.snap), (PageGrid::Lines, 32.0, false));
        assert!(dispatch(ctx, "update", &serde_json::json!({"page": &page, "snap_step": 0})).is_err());
        let read = call(ctx, "read", serde_json::json!({"page": &page}));
        assert!(read.contains("layout: free · grid: lines step 32 · snap: off · bg: theme"), "{read}");
        let out = call(ctx, "update", serde_json::json!({"page": &page, "bg": "#243149"}));
        assert!(out.contains("bg: #243149"), "{out}");
        assert_eq!(ctx.page_layout(&page).bg, "#243149");
        assert!(dispatch(ctx, "update", &serde_json::json!({"page": &page, "bg": "plaid"})).is_err());
        call(ctx, "update", serde_json::json!({"page": &page, "bg": "none"}));
        assert!(ctx.page_layout(&page).bg.is_empty());

        // append в позицию и attach-подобная вставка с геометрией.
        let out = call(ctx, "update", serde_json::json!({"page": &page, "content": "Между", "mode": "append", "after": 0, "x": 10, "y": 20}));
        assert!(out.contains("inserted 1 block at block #1 (#1)"), "{out}");
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("#1 paragraph \"Между\" · x=10 y=20"), "{listed}");

        // Доска в начале страницы со своей высотой, стиль и чтение.
        let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "index": 0, "h": 500, "x": 0, "y": 600, "title": "Доска"}));
        assert!(out.contains("created board at block #0 (#0, #1)"), "{out}");
        let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        assert!(listed.contains("#0 heading3 \"Доска\" · flow") && listed.contains("#1 embed:kanban:") && listed.contains("x=0 y=600 w=520 h=500"), "{listed}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "page": &page, "column_width": 320, "lane_bg": "#4F8CFF33", "show_counts": false}));
        assert!(out.contains("column_width=320 lane_bg=#4F8CFF33 card_bg=theme counts=off"), "{out}");
        assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "set_style", "page": &page, "column_width": 10})).is_err());
        call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "А"}));
        call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "Б"}));
        call(ctx, "kanban", serde_json::json!({"op": "update_card", "page": &page, "card": "Б", "before": "А"}));
        let (_, board) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "kanban").unwrap();
        let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &board) else { panic!() };
        assert_eq!(handle.lock().cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Б", "А"]);

        // Диаграмма: зум в пределах, «сегодня».
        call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
        let out = call(ctx, "gantt", serde_json::json!({"op": "set_zoom", "page": &page, "zoom": 40}));
        assert!(out.contains("zoom: 40 px/day"), "{out}");
        assert!(dispatch(ctx, "gantt", &serde_json::json!({"op": "set_zoom", "page": &page, "zoom": 500})).is_err());
        assert!(call(ctx, "gantt", serde_json::json!({"op": "show_today", "page": &page})).contains("today"));
    }

    /// График через инструмент: создание из таблицы, ряды, точечные
    /// значения, вид, оформление, «из блока-таблицы», удаление.
    #[test]
    fn chart_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Отчёт"})));
        let out = call(
            ctx,
            "chart",
            serde_json::json!({
                "op": "create",
                "page": &page,
                "kind": "bar",
                "title": "Квартал",
                "table": "| Месяц | План | Факт |\n| --- | --- | --- |\n| Янв | 10 | 12 |\n| Фев | 20 | 18 |",
                "style": {"stacked": true, "legend": "top", "y_max": 40}
            }),
        );
        assert!(out.contains("kind: bar") && out.contains("series: 2") && out.contains("points: 2"), "{out}");
        assert!(out.contains("\"Квартал\"") && out.contains("stacked true") && out.contains("legend top"), "{out}");
        let (_, chart) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "chart").unwrap();
        let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &chart) else { panic!() };
        assert_eq!(handle.lock().options.y_max, Some(40.0));

        // Ряд: добавление, переименование, цвет, замена значений.
        let out = call(ctx, "chart", serde_json::json!({"op": "add_series", "chart": &chart, "name": "Прогноз", "data": [11, 19], "color": "green"}));
        assert!(out.contains("Прогноз") && out.contains("series: 3"), "{out}");
        call(ctx, "chart", serde_json::json!({"op": "update_series", "chart": &chart, "series": "Прогноз", "data": [15, 25], "color": "none"}));
        {
            let doc = handle.lock();
            let s = doc.find_series("Прогноз").unwrap();
            assert_eq!(s.data, vec![15.0, 25.0]);
            assert!(s.color.is_empty(), "«none» снимает цвет: {s:?}");
        }
        // Точечная правка и подписи.
        call(ctx, "chart", serde_json::json!({"op": "set_data", "chart": &chart, "series": "План", "value": 30, "index": 1}));
        call(ctx, "chart", serde_json::json!({"op": "set_data", "chart": &chart, "categories": ["Янв", "Фев", "Мар"]}));
        {
            let doc = handle.lock();
            assert_eq!(doc.find_series("План").unwrap().at(1), 30.0);
            assert_eq!(doc.categories.len(), 3, "новая подпись — новая точка у всех рядов");
            assert_eq!(doc.series[0].data.len(), 3);
        }

        // Вид: у круговой один ряд, и лишние отбрасываются.
        let out = call(ctx, "chart", serde_json::json!({"op": "update", "chart": &chart, "kind": "pie"}));
        assert!(out.contains("kind: pie") && out.contains("series: 1"), "{out}");
        assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "add_series", "chart": &chart, "name": "Ещё"})).is_err());
        assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "update", "chart": &chart, "kind": "sausage"})).is_err());

        // Шкала: значение, границы, зоны и единица.
        call(ctx, "chart", serde_json::json!({"op": "update", "chart": &chart, "kind": "gauge", "value": 72,
            "style": {"gauge_max": 120, "unit": "%", "zones": "0-60 green, 60-120 red"}}));
        {
            let doc = handle.lock();
            assert_eq!(doc.gauge_value(), 72.0);
            assert_eq!(doc.options.gauge_max, 120.0);
            assert_eq!(doc.options.unit, "%");
            assert_eq!(doc.options.zones.len(), 2);
            assert_eq!(doc.options.zones[1].color, "#EE5E48");
        }
        assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "set_style", "chart": &chart, "style": {"nope": 1}})).is_err());
        assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "set_style", "chart": &chart, "style": {"legend": "sideways"}})).is_err());

        // График из блока-таблицы страницы: таблица заменяется врезкой.
        call(ctx, "update", serde_json::json!({"page": &page, "content": "| Город | Людей |\n| --- | --- |\n| Алматы | 2 |\n| Астана | 1 |\n", "mode": "append"}));
        let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        let block = idx.lines().find(|l| l.contains("table")).unwrap()[1..2].to_string();
        let out = call(ctx, "chart", serde_json::json!({"op": "from_table", "page": &page, "block": &block, "kind": "line"}));
        assert!(out.contains("chart with 1 series over 2 points") && out.contains("kind: line"), "{out}");
        assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "chart").count(), 2);
        // Не таблица — понятная ошибка, а не пустой график.
        let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        if let Some(line) = idx.lines().find(|l| l.contains("paragraph")) {
            let b = line[1..2].to_string();
            assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "from_table", "page": &page, "block": &b})).is_err());
        }

        // Чтение отдаёт и таблицу значений, и страницу.
        let out = call(ctx, "chart", serde_json::json!({"op": "read", "chart": &chart}));
        assert!(out.contains("value 72") && out.contains(&page), "{out}");

        // Удаление: врезка уходит со страницы.
        let out = call(ctx, "chart", serde_json::json!({"op": "delete", "chart": &chart}));
        assert!(out.contains("deleted chart:"), "{out}");
        assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "chart").count(), 1);
    }

    /// Интеллект-карта через инструмент: создание из списка, узлы, перенос,
    /// кросс-ссылки, раскладка, стиль, «из блока».
    #[test]
    fn mindmap_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Идеи"})));
        let out = call(
            ctx,
            "mindmap",
            serde_json::json!({"op": "create", "page": &page, "title": "Проект", "outline": "# Проект\n\n- Идеи\n  - Первая\n- Сроки\n", "direction": "both"}),
        );
        assert!(out.contains("nodes: 4") && out.contains("direction: both"), "{out}");
        assert!(out.contains("(#0)"), "карта — один блок, title идёт в корень: {out}");
        assert!(out.contains("\"Проект\"") && out.contains("\"Первая\""), "{out}");
        let (_, map) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "mindmap").unwrap();
        let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &map) else { panic!() };

        // Узел: добавление под родителем по тексту, поля, свёрнутость.
        let out = call(ctx, "mindmap", serde_json::json!({"op": "add_node", "map": &map, "parent": "Сроки", "text": "Дедлайн", "color": "red", "icon": "🔥"}));
        assert!(out.contains("Дедлайн") && out.contains("icon 🔥"), "{out}");
        call(ctx, "mindmap", serde_json::json!({"op": "update_node", "page": &page, "node": "Дедлайн", "note": "- [ ] проверить", "link": "Идеи", "shape": "pill", "collapsed": true}));
        {
            let doc = handle.lock();
            let n = doc.nodes.iter().find(|n| n.text == "Дедлайн").unwrap();
            assert_eq!(n.link.as_deref(), Some(page.as_str()));
            assert_eq!(n.shape.key(), "pill");
            assert!(n.collapsed && n.note.contains("проверить"));
            assert_eq!(n.color, PALETTE[5]);
        }
        // Перенос: в своё поддерево нельзя, корень не переносится.
        assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "move_node", "map": &map, "node": "root", "parent": "Идеи"})).is_err());
        assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "move_node", "map": &map, "node": "Идеи", "parent": "Первая"})).is_err());
        call(ctx, "mindmap", serde_json::json!({"op": "move_node", "map": &map, "node": "Дедлайн", "parent": "Идеи", "index": 0}));
        {
            // Один захват мьютекса на выражение: вложенный lock() — дедлок.
            let doc = handle.lock();
            let ideas = doc.nodes.iter().find(|n| n.text == "Идеи").unwrap().id.clone();
            assert_eq!(doc.children_of(&ideas)[0].text, "Дедлайн");
        }

        // Кросс-ссылка, раскладка, стиль.
        let out = call(ctx, "mindmap", serde_json::json!({"op": "add_link", "map": &map, "from": "Первая", "to": "Сроки", "label": "см."}));
        assert!(out.contains("link") && out.contains("\"см.\""), "{out}");
        assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "add_link", "map": &map, "from": "Первая", "to": "Сроки"})).is_err());
        call(ctx, "mindmap", serde_json::json!({"op": "set_layout", "map": &map, "direction": "down", "curve": "elbow", "h_gap": 60}));
        let l = handle.layout();
        assert_eq!((l.direction.key(), l.curve.key(), l.h_gap), ("down", "elbow", 60.0));
        call(ctx, "mindmap", serde_json::json!({"op": "set_style", "map": &map, "style": {"palette": "rainbow", "font_size": 15, "show_icons": false}}));
        let st = handle.style();
        assert_eq!((st.font_size, st.show_icons, st.palette[0].as_str()), (15.0, false, "#FF5A5F"));
        assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "set_style", "map": &map, "style": {"nope": 1}})).is_err());

        // Удаление узла с поддеревом и карта из блока страницы.
        let out = call(ctx, "mindmap", serde_json::json!({"op": "delete_node", "map": &map, "node": "Идеи"}));
        assert!(out.contains("deleted 3 nodes"), "перенесённый «Дедлайн» уходит с поддеревом: {out}");
        call(ctx, "update", serde_json::json!({"page": &page, "content": "- Альфа\n  - А1\n- Бета\n", "mode": "append"}));
        let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
        let block = idx.lines().find(|l| l.contains("bullet")).unwrap()[1..2].to_string();
        let out = call(ctx, "mindmap", serde_json::json!({"op": "from_list", "page": &page, "block": &block, "title": "Список"}));
        // Каждый пункт верхнего уровня — свой блок: берётся «Альфа» с «А1».
        assert!(out.contains("mind map with 3 nodes") && out.contains("\"А1\""), "{out}");
        assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "mindmap").count(), 2);

        let out = call(ctx, "mindmap", serde_json::json!({"op": "delete", "map": &map}));
        assert!(out.contains("embeds removed from 1 page"), "{out}");
        assert!(!ctx.page_markdown(&page).contains(&format!("mindmap:{map}")));
    }

    /// Календарь: события в общем хранилище, повторы, диапазон, виджет.
    #[test]
    fn calendar_through_the_tool() {
        let ctx = ctx();
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "План"})));
        let out = call(ctx, "calendar", serde_json::json!({"op": "create", "page": &page, "view": "week", "anchor": "2026-09-03"}));
        assert!(out.contains("(week view)"), "{out}");
        let (_, widget) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "calendar").unwrap();

        // Событие со временем и повтором; список за диапазон.
        let out = call(
            ctx,
            "calendar",
            serde_json::json!({"op": "add_event", "title": "Стендап", "date": "2026-09-01", "start_time": "09:30", "end_time": "10:00", "repeat": "weekly", "until": "2026-09-30", "color": "blue"}),
        );
        assert!(out.contains("09:30–10:00") && out.contains("repeat weekly until 2026-09-30"), "{out}");
        call(ctx, "calendar", serde_json::json!({"op": "add_event", "title": "Отпуск", "date": "2026-09-10", "end_date": "2026-09-12"}));
        let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-01", "to": "2026-09-30"}));
        assert!(out.contains("Стендап") && out.contains("Отпуск") && out.contains("2026-09-10 → 2026-09-12"), "{out}");
        let narrow = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-11", "to": "2026-09-11"}));
        assert!(narrow.contains("Отпуск") && !narrow.contains("Стендап"), "{narrow}");

        // Календари: свой, фильтр виджета, переезд событий при удалении.
        let work = call(ctx, "calendar", serde_json::json!({"op": "add_calendar", "name": "Работа", "color": "green"}));
        assert!(work.contains("Работа"), "{work}");
        call(ctx, "calendar", serde_json::json!({"op": "update_event", "event": "Стендап", "calendar": "Работа"}));
        let store = ctx.calendar_store();
        assert_eq!(store.lock().calendars.len(), 2);
        call(ctx, "calendar", serde_json::json!({"op": "set_view", "calendar": &widget, "view": "month", "calendars": ["Работа"]}));
        {
            let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &widget) else { panic!() };
            let d = handle.lock();
            assert_eq!(d.view.key(), "month");
            assert_eq!(d.calendars.len(), 1);
        }
        // Перенос, «сделано», удаление.
        call(ctx, "calendar", serde_json::json!({"op": "move_event", "event": "Отпуск", "date": "2026-09-17"}));
        {
            let s = store.lock();
            let e = s.events.iter().find(|e| e.title == "Отпуск").unwrap();
            assert_eq!((e.date.as_str(), e.end_date.as_deref()), ("2026-09-17", Some("2026-09-19")), "многодневность сохраняется");
        }
        let out = call(ctx, "calendar", serde_json::json!({"op": "complete", "event": "Отпуск"}));
        assert!(out.contains("done"), "{out}");
        assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "move_event", "event": "Отпуск"})).is_err());
        call(ctx, "calendar", serde_json::json!({"op": "delete_event", "event": "Отпуск"}));
        assert_eq!(store.lock().events.len(), 1);

        // Стиль виджета и слои; удаление виджета не трогает события.
        call(ctx, "calendar", serde_json::json!({"op": "set_style", "calendar": &widget, "style": {"preset": "light", "hour_from": 7, "hour_to": 22, "slot_min": 15, "show_kanban_due": true}}));
        {
            let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &widget) else { panic!() };
            let st = handle.style();
            assert_eq!((st.hour_from, st.hour_to, st.slot_min, st.show_kanban_due), (7, 22, 15, true));
            assert_eq!(st.preset, "light");
        }
        assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "set_style", "calendar": &widget, "style": {"nope": 1}})).is_err());
        let first = store.lock().calendars[0].id.clone();
        assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "delete_calendar", "calendar": &first})).is_ok());
        assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "delete_calendar", "calendar": "Работа"})).is_err(), "последний календарь");
        let out = call(ctx, "calendar", serde_json::json!({"op": "delete", "calendar": &widget}));
        assert!(out.contains("the events stay"), "{out}");
        assert_eq!(store.lock().events.len(), 1, "события остаются в проекте");
    }

    /// Каждый ключ, который читает инструмент, обязан быть в схеме — иначе
    /// валидатор (`additionalProperties: false`) его отрежет.
    #[test]
    fn schema_covers_every_argument() {
        let schema = crate::agent::tools::catalog::notes_schema();
        let props = schema["properties"].as_object().unwrap();
        let src = include_str!("notes.rs");
        let body = src.split("#[cfg(test)]").next().unwrap();
        let life = include_str!("notes/life.rs");
        let body = format!("{body}\n{}", life.split("#[cfg(test)]").next().unwrap());
        let body = body.as_str();
        let mut missing = Vec::new();
        for pat in [
            "str_field(v, \"",
            "ref_field(v, \"",
            "raw_string(v, \"",
            "bool_field(v, \"",
            "usize_field(v, \"",
            "f32_field(v, \"",
            "f64_field(v, \"",
            "data_field(v, \"",
            "list_field(v, \"",
        ] {
            for (i, _) in body.match_indices(pat) {
                let rest = &body[i + pat.len()..];
                let key = rest.split('"').next().unwrap();
                if !props.contains_key(key) && !missing.contains(&key) {
                    missing.push(key);
                }
            }
        }
        for key in ["x", "y", "w", "h", "attrs"] {
            assert!(props.contains_key(key), "schema lacks {key}");
        }
        assert!(missing.is_empty(), "schema lacks {missing:?}");
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
        assert_eq!(fnum(12.0), "12");
        assert_eq!(fnum(12.34), "12.3");
        assert_eq!(validate_attr("size", "22").unwrap().as_deref(), Some("22"));
        assert!(validate_attr("size", "500").is_err());
        assert_eq!(validate_attr("stroke", "none").unwrap().as_deref(), Some("none"));
        assert_eq!(validate_attr("bg", "").unwrap(), None);
        assert_eq!(validate_attr("fill", "#4F8CFF80").unwrap().as_deref(), Some("#4F8CFF80"));
        assert!(validate_attr("font", "x").is_err());
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

    /// Массовое чтение: страница с подстраницами, список страниц и весь
    /// проект — одним вызовом. По странице за ход локальная модель платит
    /// полным префиллом за каждую подстраницу.
    #[test]
    fn read_takes_many_pages_at_once() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        let root = page_id(&call(ctx, "create", serde_json::json!({"title": "Проект", "content": "Корень\n"})));
        let a = page_id(&call(
            ctx,
            "create",
            serde_json::json!({"title": "Раздел А", "parent": &root, "content": "Текст А\n"}),
        ));
        let b = page_id(&call(
            ctx,
            "create",
            serde_json::json!({"title": "Раздел Б", "parent": &root, "content": "Текст Б\n"}),
        ));
        call(ctx, "create", serde_json::json!({"title": "Подраздел", "parent": &a, "content": "Глубокий текст\n"}));

        // Одна страница — прежний формат, без шапки пачки и без соседей.
        let one = call(ctx, "read", serde_json::json!({"page": &root}));
        assert!(one.starts_with("page: "), "{one}");
        assert!(!one.contains("Текст А"), "{one}");

        // Поддерево целиком.
        assert!(one.contains("--- Sub-pages (2)"), "одиночное чтение называет подстраницы: {one}");

        let tree = call(ctx, "read", serde_json::json!({"page": &root, "depth": "all"}));
        assert!(tree.starts_with("--- 4 pages"), "{tree}");
        for t in ["Корень", "Текст А", "Текст Б", "Глубокий текст"] {
            assert!(tree.contains(t), "нет «{t}»: {tree}");
        }

        // Один уровень — дети без внуков.
        let level1 = call(ctx, "read", serde_json::json!({"page": &root, "depth": 1}));
        assert!(level1.contains("Текст А") && !level1.contains("Глубокий текст"), "{level1}");

        // Явный список страниц и весь проект.
        let pair = call(ctx, "read", serde_json::json!({"pages": [&a, &b]}));
        assert!(pair.contains("Текст А") && pair.contains("Текст Б") && !pair.contains("Корень"), "{pair}");
        let all = call(ctx, "read", serde_json::json!({"page": "all"}));
        assert!(all.contains("=== page 4/4 ==="), "{all}");
        let two = call(ctx, "read", serde_json::json!({"page": "all", "limit": 2}));
        assert!(two.starts_with("--- 2 pages"), "{two}");
    }

    /// Пачка обязана делиться сама: иначе укладка выхлопа в окно
    /// (`fit_for_prompt`) вырежет из ответа середину — целые страницы, о
    /// пропаже которых модель узнаёт только по дыре в тексте.
    #[test]
    fn notes_answers_are_not_clipped_by_history() {
        // Клипа поверх `notes` нет: инструмент сам меряет ответ живым окном
        // и обрывает его на границе страницы. Статический клип вырезал бы
        // у честной пачки середину — целые страницы, о пропаже которых
        // модель узнаёт только по дыре в тексте.
        assert_eq!(
            super::super::executor::history_limit(super::super::catalog::KEY_NOTES),
            None
        );
        assert!(
            READ_MAX_BYTES < MAX_NOTES_OUTPUT_BYTES,
            "предохранитель исполнителя не оставил места шапке и списку недочитанного"
        );
    }

    /// Свободное окно — весь проект одним ответом: своего потолка у
    /// инструмента больше нет, границу ставит только контекст.
    #[test]
    fn a_roomy_window_reads_the_whole_project_in_one_reply() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        let long = "Строка текста для объёма.\n\n".repeat(110);
        for i in 1..=12 {
            call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
        }
        // Без бюджета те же страницы в один ответ не влезают (фолбэк —
        // 8000 токенов), с живым окном на 200k — влезают все.
        assert!(call(ctx, "read", serde_json::json!({"page": "all"})).contains("did not fit"));

        let _budget = budget::arm(200_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
        let all = call(ctx, "read", serde_json::json!({"page": "all"}));
        assert!(all.contains("all of them in this reply"), "{}", &all[..160]);
        assert_eq!(all.matches("=== page ").count(), 12);
        assert!(all.contains("left in context"), "шапка называет цену ответа и остаток окна");
        assert!(!all.contains("Context is filling up"), "окно свободно — пугать нечем");
    }

    /// Одна страница больше бюджета: обрывается по строке, называет
    /// остаток и способ его дочитать — а не уезжает под клип истории.
    #[test]
    fn a_huge_single_page_is_cut_with_a_way_to_finish_it() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        let long = "Строка текста для объёма.\n\n".repeat(400);
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Полотно", "content": &long})));

        let _budget = budget::arm(2_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
        let out = call(ctx, "read", serde_json::json!({"page": &page}));
        assert!(out.contains("--- Page truncated: "), "{}", &out[..200]);
        assert!(out.contains("blocks {\"op\": \"read\""), "нужен способ дочитать остаток");
        assert!(
            budget::count(&out) <= 1_000,
            "ответ на ~{} токенов больше гранта",
            budget::count(&out)
        );
        // Обрыв по границе строки: половинок строк в ответе нет.
        let body = out.split("--- Page truncated").next().unwrap();
        assert!(
            body.matches("Строка текста для объёма.").count() > 0
                && body.ends_with('\n'),
            "страница обязана обрываться на строке"
        );
    }

    /// Тесное окно опускает потолок ответа, и модель узнаёт об этом из
    /// шапки — вместо молчаливой дыры в середине.
    #[test]
    fn a_tight_context_shrinks_the_reply_and_says_so() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        let long = "Строка текста для объёма.\n\n".repeat(60);
        for i in 1..=12 {
            call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
        }
        let roomy = call(ctx, "read", serde_json::json!({"page": "all"}));

        // 4000 токенов окна → грант 2000 → страниц влезает меньше.
        let _budget = budget::arm(4_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
        let tight = call(ctx, "read", serde_json::json!({"page": "all"}));
        let pages = |s: &str| s.matches("=== page ").count();
        assert!(
            pages(&tight) < pages(&roomy),
            "тесное окно ({} страниц) должно отдавать меньше свободного ({})",
            pages(&tight),
            pages(&roomy)
        );
        assert!(tight.contains("left in context"), "шапка обязана назвать остаток окна: {}", &tight[..120]);
        assert!(tight.contains("Context is filling up"), "модель должна узнать причину обрыва");
    }

    /// Массовое чтение обрывается на границе страницы и называет остаток.
    #[test]
    fn read_stops_on_a_page_boundary_and_lists_the_rest() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        // Каждая страница — заметно больше десятой доли бюджета, так что
        // в один ответ они все не влезают.
        let long = "Строка текста для объёма.\n\n".repeat(110);
        for i in 1..=12 {
            call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
        }
        let all = call(ctx, "read", serde_json::json!({"page": "all"}));
        assert!(all.starts_with("--- ") && all.contains("did not fit in one reply"), "{}", &all[..200]);
        assert!(all.contains("--- Not read ("), "остаток должен быть перечислен по id");
        assert!(
            budget::count(&all) <= READ_FALLBACK_TOKENS && all.len() <= READ_MAX_BYTES,
            "ответ на ~{} токенов вышел за бюджет",
            budget::count(&all)
        );
        // Обрыв ровно на границе: сколько страниц названо в шапке, столько
        // их и в ответе — целиком, а не с оборванным хвостом у последней.
        let done: usize = all.split(' ').nth(1).and_then(|n| n.parse().ok()).expect("шапка пачки");
        assert!(done > 0 && done < 12, "прочитано {done} из 12");
        assert_eq!(all.matches("=== page ").count(), done);
        assert_eq!(
            all.matches("Строка текста для объёма.").count(),
            done * 110,
            "страница попала в ответ не целиком"
        );
    }

    /// Блоки страницы читаются пачкой: `all`, список и диапазон; одиночная
    /// ссылка отвечает как прежде.
    #[test]
    fn blocks_read_takes_a_list_and_the_whole_page() {
        let _serial = budget::test_serial();
        let ctx = ctx();
        let page = page_id(&call(
            ctx,
            "create",
            serde_json::json!({"title": "Холст-чтение", "content": "# Схема\n\nПервый\n\nВторой\n\nТретий\n"}),
        ));
        let all = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "all"}));
        assert!(all.starts_with("--- 4 blocks ---"), "{all}");
        assert!(all.contains("# Схема") && all.contains("Первый") && all.contains("Третий"), "{all}");

        let some = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "1,3"}));
        assert!(some.starts_with("--- 2 blocks ---"), "{some}");
        assert!(some.contains("Первый") && some.contains("Третий") && !some.contains("Второй"), "{some}");

        let range = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "2-3"}));
        assert!(range.contains("Второй") && range.contains("Третий") && !range.contains("Первый"), "{range}");

        // Одиночная ссылка (индекс или find:) — прежний ответ без шапки.
        let single = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "find:Второй"}));
        assert!(single.starts_with("#2 paragraph"), "{single}");
        assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "read", "page": &page, "block": "9"})).is_err());
    }

    /// Панель свойств помнится за страницей и уезжает в бандл: у каждой
    /// страницы своё положение разделителя и свой «скрыта».
    #[test]
    fn props_panel_state_is_per_page() {
        let ctx = ctx();
        let a = page_id(&call(ctx, "create", serde_json::json!({"title": "Холст"})));
        let b = page_id(&call(ctx, "create", serde_json::json!({"title": "Заметка"})));
        assert_eq!(ctx.props_panel(&a), (None, false), "новая страница — без своих значений");

        ctx.set_props_panel(&a, 0.72, true);
        ctx.set_props_panel(&b, 0.5, false);
        assert_eq!(ctx.props_panel(&a), (Some(0.72), true));
        assert_eq!(ctx.props_panel(&b), (Some(0.5), false));

        // Положение квантуется — перетаскивание разделителя не поднимает
        // ревизию дерева на каждый пиксель.
        let rev = ctx.tree_rev.get_untracked();
        ctx.set_props_panel(&a, 0.7201, true);
        assert_eq!(ctx.tree_rev.get_untracked(), rev, "тот же квант — без правки дерева");
        ctx.set_props_panel(&a, 0.75, true);
        assert!(ctx.tree_rev.get_untracked() > rev);

        // Пережило запись и чтение проекта.
        let json = ctx.tree.get_untracked().serialize();
        let back = crate::pages::notes::project::ProjectTree::parse(&json).unwrap();
        assert_eq!(back.layout_of(&a).props_ratio, Some(0.75));
        assert!(back.layout_of(&a).props_hidden);
        assert!(!back.layout_of(&b).props_hidden);
    }

    /// Календарь по доскам (09.09.2026): оценка длительности плюс «в
    /// календарь» дают карточке полосу; календарь видит её отрезком и
    /// двигает, диаграмма Ганта — своей строкой, agenda кладёт её в
    /// «Сегодня» даже без срока.
    #[test]
    fn board_cards_become_calendar_bars_and_gantt_rows() {
        use crate::pages::notes::calendar::{ExternalKind, ExternalQuery};
        let ctx = ctx();
        let today = today_days();
        let iso = |d: i64| days_to_iso(today + d);
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Спринт"})));
        let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["Бэклог", "Готово"]}));
        let board = out.lines().find_map(|l| l.strip_prefix("kanban:")).unwrap().split(' ').next().unwrap().to_string();
        call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": "Импорт"}));

        // «Оценка 1 день» + «в календарь на завтра» — карточка получает полосу.
        let out = call(
            ctx,
            "kanban",
            serde_json::json!({"op": "schedule", "board": &board, "card": "Импорт", "date": "tomorrow", "duration": "1d"}),
        );
        assert!(out.contains("duration: 1d") && out.contains(&format!("planned: {}", iso(1))), "{out}");

        // Календарь видит её полосой; срок не выдуман — его нет.
        let env = embeds::calendar_env(ctx);
        let q = ExternalQuery { from: today, to: today + 7, boards: Vec::new(), due: true, spans: true, gantt: true };
        let items = (env.external)(&q);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!((items[0].day, items[0].end_day, items[0].source.kind), (today + 1, today + 1, ExternalKind::Card));

        // Перенос полосы в календаре двигает start/end карточки.
        assert!((env.shift_external)(&items[0].source, 2));
        let card = |ctx: NotesCtx| match ctx.object("kanban", &board) {
            Some(LiveObject::Kanban { handle, .. }) => handle.card(&items[0].source.item).unwrap(),
            _ => panic!("доска пропала"),
        };
        assert_eq!(card(ctx).start.as_deref(), Some(iso(3).as_str()));
        assert_eq!(card(ctx).end.as_deref(), Some(iso(3).as_str()));

        // Диаграмма Ганта показывает ту же карточку строкой и правит её даты.
        let g = call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
        let gid = g.lines().find_map(|l| l.strip_prefix("created chart gantt:")).unwrap().split(' ').next().unwrap().to_string();
        let out = call(ctx, "gantt", serde_json::json!({"op": "set_boards", "gantt": &gid, "boards": [&board]}));
        assert!(out.contains(&format!("boards: {board}")), "{out}");
        let genv = embeds::gantt_env(ctx);
        assert!((genv.cards)(&[]).is_empty(), "без выбранных досок диаграмма показывает только свои задачи");
        let rows = (genv.cards)(&[board.clone()]);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].start, rows[0].end, rows[0].name.as_str()), (today + 3, today + 3, "Импорт"));
        (genv.set_card_span)(&board, &rows[0].card, today, today + 1);
        assert_eq!(card(ctx).span_text(), format!("{} → {}", iso(0), iso(1)));

        // Часовая полоса: время попадает в отрезок дневной сетки.
        call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": "Созвон"}));
        call(
            ctx,
            "kanban",
            serde_json::json!({"op": "schedule", "board": &board, "card": "Созвон", "date": "today", "time": "10:00", "duration": "2h"}),
        );
        let timed = (env.external)(&q);
        let call_item = timed.iter().find(|i| i.title == "Созвон").expect("полоса созвона");
        assert_eq!((call_item.day, call_item.time), (today, Some((600, 720))));

        // agenda: задача со start = сегодня в «Сегодня», хотя срока нет.
        let out = call(ctx, "agenda", serde_json::json!({}));
        assert!(out.contains("Today (2):") && out.contains("\"Импорт\"") && out.contains("\"Созвон\""), "{out}");

        // Снятие плана: полоса исчезает, карточка остаётся.
        call(ctx, "kanban", serde_json::json!({"op": "unschedule", "board": &board, "card": "Созвон"}));
        assert_eq!((env.external)(&q).len(), 1, "осталась только полоса «Импорта»");
    }

    /// Ведение жизни: колонка «готово» по названию, штампы и повтор при
    /// закрытии, журнал с актором, agenda/tasks по всем доскам, архив,
    /// страница дня, вложения карточки, сроки в календаре по умолчанию.
    #[test]
    fn life_management_through_the_tool() {
        let ctx = ctx();
        let today = today_days();
        let iso = |d: i64| days_to_iso(today + d);
        let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Работа"})));
        let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["Бэклог", "В работе", "Готово"]}));
        assert!(out.contains("\"Готово\" · DONE column"), "колонка «Готово» узнаётся по названию: {out}");
        let board = out.lines().find_map(|l| l.strip_prefix("kanban:")).unwrap().split(' ').next().unwrap().to_string();
        let add = |title: &str, extra: serde_json::Value| {
            let mut args = serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": title});
            for (k, v) in extra.as_object().unwrap() {
                args[k] = v.clone();
            }
            call(ctx, "kanban", args)
        };
        let out = add("Импорт", serde_json::json!({"due": iso(-3), "tags": "core"}));
        assert!(out.contains(&format!("created: {}", iso(0))), "штамп создания: {out}");
        add("Отчёт", serde_json::json!({"due": "today"}));
        add("Звонок", serde_json::json!({"due": "tomorrow", "priority": "high"}));
        add("План", serde_json::json!({"due": iso(5)}));
        add("Идея", serde_json::json!({"priority": "urgent"}));
        let out = add("Привычка", serde_json::json!({"due": "today", "repeat": "daily", "md": "- [x] шаг"}));
        assert!(out.contains("repeat: daily"), "{out}");
        call(ctx, "calendar", serde_json::json!({"op": "add_event", "title": "Стендап", "date": iso(1), "start_time": "10:00", "end_time": "10:30"}));

        // agenda: одним вызовом — просрочено, сегодня, завтра, скоро, важное без срока, события, доски.
        let out = call(ctx, "agenda", serde_json::json!({}));
        assert!(out.contains("Overdue (1):") && out.contains("\"Импорт\"") && out.contains("3 days late"), "{out}");
        assert!(out.contains("Today (2):") && out.contains("Tomorrow (1):") && out.contains("Next 7 days (1):"), "{out}");
        assert!(out.contains("Important without a due date (1):") && out.contains("\"Идея\""), "{out}");
        assert!(out.contains("\"Стендап\"") && out.contains("10:00–10:30"), "{out}");
        assert!(out.contains("Boards:") && out.contains("Бэклог 6") && out.contains("Готово ✓ 0"), "{out}");

        // tasks: фильтры.
        let out = call(ctx, "tasks", serde_json::json!({"due": "overdue"}));
        assert!(out.contains("1 card") && out.contains("\"Импорт\"") && !out.contains("\"Отчёт\""), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"tag": "core"}));
        assert!(out.contains("\"Импорт\"") && out.contains("1 card"), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"priority": "urgent", "due": "none"}));
        assert!(out.contains("\"Идея\"") && out.contains("1 card"), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"due": "week", "sort": "priority"}));
        let pos = |t: &str| out.find(t).unwrap_or(usize::MAX);
        assert!(pos("\"Звонок\"") < pos("\"Отчёт\""), "high раньше без приоритета: {out}");

        // Закрытие: перенос в «Готово» — штамп done и следующая карточка повтора; agenda видит «сделано».
        let out = call(ctx, "kanban", serde_json::json!({"op": "move_card", "board": &board, "card": "Привычка", "column": "Готово"}));
        assert!(out.contains(&format!("done: {}", iso(0))), "{out}");
        let doc = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().clone(), _ => panic!() };
        let habits: Vec<&KanbanCard> = doc.cards.iter().filter(|c| c.title == "Привычка").collect();
        assert_eq!(habits.len(), 2, "{doc:?}");
        let next = habits.iter().find(|c| c.done.is_none()).unwrap();
        assert_eq!((next.due.as_deref(), next.md.as_str(), doc.column_name(&next.column).as_str()), (Some(iso(1).as_str()), "- [ ] шаг", "Бэклог"));
        let out = call(ctx, "agenda", serde_json::json!({}));
        assert!(out.contains("Done in the last 3 days (1):"), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"done": true}));
        assert!(out.contains("1 card") && out.contains("done: "), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"done": "any", "query": "привыч"}));
        assert!(out.contains("2 cards"), "{out}");

        // Журнал: добавления, DONE, повтор; актор — user вне агентского скоупа, agent внутри.
        let out = call(ctx, "log", serde_json::json!({"since": "today"}));
        assert!(out.contains("card \"Привычка\" DONE (\"Бэклог\" → \"Готово\")"), "{out}");
        assert!(out.contains("next repeat created") && out.contains("card \"Импорт\" added to \"Бэклог\""), "{out}");
        assert!(out.contains("event \"Стендап\" added on") && out.contains("page \"Работа\" created"), "{out}");
        assert!(out.lines().filter(|l| l.contains(" · user · ")).count() > 5 && !out.contains(" · agent · "), "{out}");
        {
            let _agent = crate::pages::notes::activity::agent_scope();
            call(ctx, "kanban", serde_json::json!({"op": "update_card", "board": &board, "card": "План", "due": iso(6)}));
        }
        let out = call(ctx, "log", serde_json::json!({"since": "today", "actor": "agent"}));
        assert!(out.contains("1 entry") && out.contains(&format!("card \"План\" due {} → {}", iso(5), iso(6))), "{out}");
        let out = call(ctx, "log", serde_json::json!({"since": "today", "kind": "event"}));
        assert!(out.contains("1 entry"), "{out}");
        let out = call(ctx, "log", serde_json::json!({"since": "today", "op": "done", "board": &board}));
        assert!(out.contains("1 entry") && out.contains("DONE"), "{out}");
        assert!(dispatch(ctx, "log", &serde_json::json!({"since": "позавчера"})).is_err());

        // Архив: archive_after=0 уносит закрытые сразу; read archived=true; unarchive по названию.
        let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "board": &board, "archive_after": 0}));
        assert!(out.contains("archive_after off"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "board": &board, "archive_after": "1"}));
        assert!(out.contains("archive_after 1 days"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "archive", "board": &board, "card": "Отчёт"}));
        assert!(out.contains("archived card \"Отчёт\"") && out.contains("archived: 1"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "read", "board": &board, "archived": true}));
        assert!(out.contains("--- Archive (1) ---") && out.contains("\"Отчёт\""), "{out}");
        let out = call(ctx, "tasks", serde_json::json!({"done": "any", "archived": true, "query": "отчёт"}));
        assert!(out.contains("· archived"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "unarchive", "board": &board, "card": "Отчёт"}));
        assert!(out.contains("restored") && out.contains("archived: 0") || !out.contains("archived:"), "{out}");
        let doc = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().clone(), _ => panic!() };
        let report = doc.cards.iter().find(|c| c.title == "Отчёт").unwrap();
        assert!(doc.is_done_column(&report.column) && report.done.is_some());
        let out = call(ctx, "log", serde_json::json!({"since": "today", "card": "Отчёт", "board": &board}));
        assert!(out.contains("archived (from") && out.contains("restored to \"Готово\""), "{out}");

        // Флаг колонки: снять и поставить.
        let out = call(ctx, "kanban", serde_json::json!({"op": "update_column", "board": &board, "column": "Готово", "done": false}));
        assert!(out.contains("!! no done column"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "add_column", "board": &board, "name": "Сделано", "done": true}));
        assert!(out.contains("\"Сделано\" · DONE column") && !out.contains("!! no done column"), "{out}");

        // Страница дня.
        let out = call(ctx, "journal", serde_json::json!({"date": "2026-09-08", "content": "- сделал импорт"}));
        assert!(out.contains("journal page for 2026-09-08 (Tue) · created") && out.contains("1 block appended"), "{out}");
        assert!(out.contains("- сделал импорт"), "{out}");
        let day = page_id(&out);
        let out = call(ctx, "journal", serde_json::json!({"date": "2026-09-08"}));
        assert!(!out.contains("· created") && page_id(&out) == day, "{out}");
        let tree = ctx.tree.get_untracked();
        let path: Vec<String> = tree.path_of(&day).into_iter().map(|(_, t)| t).collect();
        assert_eq!(path, ["Журнал", "2026-09", "2026-09-08"]);
        assert_eq!(ctx.find_journal_page(parse_days("2026-09-08").unwrap()), Some(day.clone()));
        assert!(ctx.find_journal_page(parse_days("2026-09-09").unwrap()).is_none());
        assert_eq!(crate::pages::notes::state::date_title("2026-09-08"), parse_days("2026-09-08"));
        assert!(crate::pages::notes::state::date_title("Заметка").is_none());

        // Вложения: файл с диска → бандл; картинка — миниатюра, файл — скрепка; detach по имени.
        let dir = std::env::temp_dir().join(format!("synthos-notes-attach-{}", project::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pic = dir.join("фото.png");
        std::fs::write(&pic, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let doc_file = dir.join("договор.pdf");
        std::fs::write(&doc_file, b"%PDF-1.4 fake").unwrap();
        let out = call(ctx, "kanban", serde_json::json!({"op": "attach", "board": &board, "card": "Импорт", "path": pic.display().to_string()}));
        assert!(out.contains("as image (thumbnail on the card) \"фото.png\"") && out.contains("files: фото.png"), "{out}");
        let out = call(ctx, "kanban", serde_json::json!({"op": "attach", "board": &board, "card": "Импорт", "path": doc_file.display().to_string(), "name": "Договор"}));
        assert!(out.contains("as file (paperclip on the card) \"Договор\"") && out.contains("files: фото.png, Договор"), "{out}");
        let card = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().cards.iter().find(|c| c.title == "Импорт").cloned().unwrap(), _ => panic!() };
        assert_eq!(card.files.len(), 2);
        assert!(card.files[0].is_image() && !card.files[1].is_image());
        assert!(crate::pages::notes::media::asset_file(&ctx.project_path.get_untracked(), &card.files[0].url).is_some_and(|p| p.is_file()));
        let out = call(ctx, "kanban", serde_json::json!({"op": "detach", "board": &board, "card": "Импорт", "file": "договор"}));
        assert!(out.contains("removed attachment") && !out.contains("Договор"), "{out}");
        assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "detach", "board": &board, "card": "Импорт", "file": "нет"})).is_err());

        // Календарь: сроки досок в выдаче по умолчанию, с пометкой «не событие».
        let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": iso(-3), "to": iso(7)}));
        assert!(out.contains("external:") && out.contains("\"Импорт\"") && out.contains("not an event"), "{out}");
        let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": iso(-3), "to": iso(7), "include_external": false}));
        assert!(!out.contains("external:"), "{out}");
    }
}
