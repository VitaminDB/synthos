//! Живые врезки `![[…]]` в страницах.
//!
//! `![[kanban:<id>]]` / `![[gantt:<id>]]` / `![[chart:<id>]]` — примитивы-
//! объекты проекта (канбан-доска, диаграмма Ганта, график), живые прямо в
//! странице
//! (ручки из пула `NotesCtx.objects`, автосейв подписан и на них). Высота
//! такой врезки — ключ `h` свободной раскладки (тянется за нижнюю кромку,
//! правится в свойствах); без него — дефолт. `![[Название]]` — другая
//! страница read-only (глубина ≤ 2, циклы отсекаются по цепочке id).

use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocumentEditor, EmbedCtx, EmbedFactory};

use crate::icons::*;

use super::state::{LiveObject, NotesCtx};

const MAX_DEPTH: usize = 2;

/// Высота врезки объекта по виду, пока её не растянули: доске нужно
/// место под несколько карточек, диаграмме хватает трёх строк и шапки.
pub fn default_object_h(kind: &str) -> f32 {
    match kind {
        "kanban" => 400.0,
        "mindmap" => 420.0,
        "calendar" => 480.0,
        "chart" => 320.0,
        _ => 340.0,
    }
}

/// Вид объекта по цели врезки (`kanban:<id>` → `kanban`).
pub fn object_kind_of(target: &str) -> Option<&str> {
    let (kind, _) = target.split_once(':')?;
    is_sized_object(target).then_some(kind)
}

/// Цель врезки — объект-примитив со своей высотой.
pub fn is_sized_object(target: &str) -> bool {
    target.starts_with("kanban:")
        || target.starts_with("gantt:")
        || target.starts_with("mindmap:")
        || target.starts_with("calendar:")
        || target.starts_with("chart:")
}

/// Окружение интеллект-карты: ссылка узла открывает страницу, «в список»
/// заменяет врезку карты вложенным списком на её странице.
pub fn mindmap_env(ctx: NotesCtx) -> super::mindmap::view::MapEnv {
    super::mindmap::view::MapEnv {
        open_page: Arc::new(move |id| {
            ctx.activate(id);
            crate::rail::navigate("notes");
        }),
        to_list: Arc::new(move |id| mindmap_to_list(ctx, id)),
    }
}

/// Окружение календаря: хранилище событий проекта, слой задач досок и
/// Ганта, их перенос, список досок для панели свойств, открытие страницы.
pub fn calendar_env(ctx: NotesCtx) -> super::calendar::CalendarEnv {
    super::calendar::CalendarEnv {
        store: ctx.calendar_store(),
        external: Arc::new(move |q| external_items(ctx, q)),
        shift_external: Arc::new(move |r, delta| shift_external(ctx, r, delta)),
        boards: Arc::new(move || project_boards(ctx).into_iter().map(|(id, _, title)| (id, title)).collect()),
        project_rev: ctx.objects_rev,
        open_page: Arc::new(move |id| {
            ctx.activate(id);
            crate::rail::navigate("notes");
        }),
    }
}

/// Окружение диаграммы Ганта: карточки выбранных досок строками, запись
/// новых дат обратно в карточку, список досок, открытие страницы.
pub fn gantt_env(ctx: NotesCtx) -> super::gantt::GanttEnv {
    super::gantt::GanttEnv {
        cards: Arc::new(move |boards| board_tasks(ctx, boards)),
        set_card_span: Arc::new(move |board, card, start, end| {
            if let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", board) {
                handle.set_card_span(card, start, end);
            }
        }),
        boards: Arc::new(move || project_boards(ctx).into_iter().map(|(id, _, title)| (id, title)).collect()),
        project_rev: ctx.objects_rev,
        open_page: Arc::new(move |id| {
            ctx.activate(id);
            crate::rail::navigate("notes");
        }),
    }
}

/// Запланированные карточки указанных досок — строки диаграммы Ганта.
fn board_tasks(ctx: NotesCtx, boards: &[String]) -> Vec<super::gantt::BoardTask> {
    use super::gantt::BoardTask;
    let mut out = Vec::new();
    if boards.is_empty() {
        return out;
    }
    for (oid, page_id, _) in project_boards(ctx) {
        if !boards.contains(&oid) {
            continue;
        }
        let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &oid) else { continue };
        let doc = handle.lock();
        for c in &doc.cards {
            let Some(span) = c.schedule() else { continue };
            out.push(BoardTask {
                board: oid.clone(),
                card: c.id.clone(),
                name: c.title.clone(),
                color: doc.columns.iter().find(|col| col.id == c.column).map(|col| col.color.clone()).unwrap_or_default(),
                start: span.start_day,
                end: span.end_day,
                page: page_id.clone(),
            });
        }
    }
    out
}

/// Доски проекта: `(id доски, id страницы, название страницы)` в порядке
/// дерева.
pub fn project_boards(ctx: NotesCtx) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for pid in ctx.tree.get_untracked().all_ids() {
        for (kind, oid) in super::state::object_refs(&ctx.page_markdown(&pid)) {
            if kind == "kanban" && !out.iter().any(|(id, _, _)| *id == oid) {
                out.push((oid, pid.clone(), ctx.title_of(&pid)));
            }
        }
    }
    out
}

/// Задачи досок (плановые полосы и сроки) и задачи Ганта всех страниц в
/// диапазоне дней запроса.
fn external_items(ctx: NotesCtx, q: &super::calendar::ExternalQuery) -> Vec<super::calendar::ExternalItem> {
    use super::calendar::{ExternalItem, ExternalKind, ExternalRef};
    use super::gantt::calendar::parse_days;
    let (from, to) = (q.from, q.to);
    let mut out = Vec::new();
    for pid in ctx.tree.get_untracked().all_ids() {
        for (kind, oid) in super::state::object_refs(&ctx.page_markdown(&pid)) {
            match (kind.as_str(), ctx.object(&kind, &oid)) {
                ("kanban", Some(LiveObject::Kanban { handle, .. })) => {
                    if !q.boards.is_empty() && !q.boards.contains(&oid) {
                        continue;
                    }
                    let doc = handle.lock();
                    for c in &doc.cards {
                        let color = doc.columns.iter().find(|col| col.id == c.column).map(|col| col.color.clone()).unwrap_or_default();
                        let item = |kind, day, end_day, time| ExternalItem {
                            day,
                            end_day,
                            time,
                            title: c.title.clone(),
                            color: color.clone(),
                            page: pid.clone(),
                            source: ExternalRef { kind, object: oid.clone(), item: c.id.clone() },
                        };
                        // Есть план — полоса; срок точкой рисуется, только
                        // если он вне полосы (иначе одна задача дважды).
                        let span = q.spans.then(|| c.schedule()).flatten();
                        if let Some(s) = span {
                            if s.end_day >= from && s.start_day <= to {
                                out.push(item(ExternalKind::Card, s.start_day, s.end_day, s.time));
                            }
                        }
                        if q.due {
                            let Some(day) = c.due.as_deref().and_then(parse_days) else { continue };
                            let covered = span.is_some_and(|s| (s.start_day..=s.end_day).contains(&day));
                            if !covered && day >= from && day <= to {
                                out.push(item(ExternalKind::Due, day, day, None));
                            }
                        }
                    }
                }
                ("gantt", Some(LiveObject::Gantt { handle, .. })) => {
                    if !q.gantt {
                        continue;
                    }
                    for t in &handle.lock().tasks {
                        let Some((s, e)) = t.span_days() else { continue };
                        if e < from || s > to {
                            continue;
                        }
                        out.push(ExternalItem {
                            day: s,
                            end_day: e,
                            time: None,
                            title: t.name.clone(),
                            color: t.color.clone(),
                            page: pid.clone(),
                            source: ExternalRef { kind: ExternalKind::Task, object: oid.clone(), item: t.id.clone() },
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Перенос внешнего элемента на `delta` дней: срок карточки, её плановая
/// полоса либо задача Ганта.
fn shift_external(ctx: NotesCtx, r: &super::calendar::ExternalRef, delta: i64) -> bool {
    use super::calendar::ExternalKind;
    use super::gantt::calendar::{days_to_iso, parse_days};
    if delta == 0 {
        return false;
    }
    match r.kind {
        ExternalKind::Due | ExternalKind::Card => {
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &r.object) else { return false };
            if r.kind == ExternalKind::Card {
                handle.shift_card_schedule(&r.item, delta);
                return true;
            }
            let Some(day) = handle.card(&r.item).and_then(|c| c.due.as_deref().and_then(parse_days)) else { return false };
            handle.set_due(&r.item, Some(days_to_iso(day + delta)));
            true
        }
        ExternalKind::Task => {
            let Some(LiveObject::Gantt { handle, .. }) = ctx.object("gantt", &r.object) else { return false };
            let span = handle.lock().tasks.iter().find(|t| t.id == r.item).and_then(|t| t.span_days());
            let Some((s, e)) = span else { return false };
            handle.set_task_dates(&r.item, s + delta, e + delta);
            true
        }
    }
}

/// Карта → вложенный список на месте врезки (на всех страницах, где она
/// врезана); объект удаляется из бандла.
pub fn mindmap_to_list(ctx: NotesCtx, id: &str) {
    use syngui::widgets::input::document_editor::{parse_document, serialize_document, BlockKind};
    let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", id) else { return };
    let outline = handle.lock().to_outline();
    let target = format!("mindmap:{id}");
    for pid in ctx.pages_referencing("mindmap", id) {
        let mut model = parse_document(&ctx.page_markdown(&pid));
        let Some(pos) = model.blocks.iter().position(|b| matches!(&b.kind, BlockKind::Embed { target: t } if t.trim() == target)) else {
            continue;
        };
        model.blocks.remove(pos);
        let fresh = parse_document(&outline).blocks;
        let tail = model.blocks.split_off(pos);
        model.blocks.extend(fresh);
        model.blocks.extend(tail);
        ctx.set_page_markdown(&pid, &serialize_document(&model));
    }
    ctx.delete_object("mindmap", id);
}

pub struct NotesEmbedFactory {
    ctx: NotesCtx,
}

pub fn factory(ctx: NotesCtx) -> Arc<NotesEmbedFactory> {
    Arc::new(NotesEmbedFactory { ctx })
}

impl EmbedFactory for NotesEmbedFactory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let ctx = self.ctx;
        let target = target.trim();
        let height = ectx.height.unwrap_or_else(|| default_object_h(object_kind_of(target).unwrap_or_default()));
        if let Some(id) = target.strip_prefix("kanban:") {
            let LiveObject::Kanban { handle, id: oid } = ctx.object("kanban", id.trim())? else { return None };
            return Some(sized(Box::new(super::kanban::view::view(super::kanban::env(ctx), oid, handle)), height));
        }
        if let Some(id) = target.strip_prefix("gantt:") {
            let LiveObject::Gantt { handle, .. } = ctx.object("gantt", id.trim())? else { return None };
            return Some(sized(Box::new(super::gantt::view::view(gantt_env(ctx), handle)), height));
        }
        if let Some(id) = target.strip_prefix("mindmap:") {
            let LiveObject::Mindmap { handle, id: oid } = ctx.object("mindmap", id.trim())? else { return None };
            return Some(sized(Box::new(super::mindmap::view::view(mindmap_env(ctx), oid, handle)), height));
        }
        if let Some(id) = target.strip_prefix("calendar:") {
            let LiveObject::Calendar { handle, id: oid } = ctx.object("calendar", id.trim())? else { return None };
            return Some(sized(Box::new(super::calendar::view::view(calendar_env(ctx), oid, handle)), height));
        }
        if let Some(id) = target.strip_prefix("chart:") {
            let LiveObject::Chart { handle, .. } = ctx.object("chart", id.trim())? else { return None };
            return Some(sized(Box::new(super::chart::view::view(handle)), height));
        }
        let id = ctx.index.get_untracked().resolve(target)?;
        if ectx.depth >= MAX_DEPTH {
            return Some(note_stub(MI_EDIT_NOTE, tr!("notes.embed.too_deep")));
        }
        if ectx.chain.contains(&id) {
            return Some(note_stub(MI_AUTORENEW, tr!("notes.embed.cycle")));
        }
        let mut chain = ectx.chain.clone();
        chain.push(id.clone());
        let inner_ctx = EmbedCtx { depth: ectx.depth + 1, chain, height: None };
        let content = ctx.page_markdown(&id);
        let title = ctx.title_of(&id);
        Some(Box::new(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(page_header(ctx, id, title))
                .child(
                    DocumentEditor::new()
                        .markdown(content)
                        .read_only(true)
                        .links(super::links::provider(ctx))
                        .media(super::media::resolver(ctx))
                        .embeds(factory(ctx))
                        .embed_ctx(inner_ctx)
                        .class("notes-embed-page"),
                ),
        ))
    }

    fn has_own_height(&self, target: &str) -> bool {
        is_sized_object(target.trim())
    }
}

/// Шапка врезки страницы: название + «открыть страницей».
fn page_header(ctx: NotesCtx, id: String, title: String) -> impl Widget {
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-embed-header")
        .child(Icon::new(MI_DESCRIPTION).class("notes-embed-icon"))
        .child(
            DecoratedBox::new()
                .class("grow")
                .child(Text::new(title).max_lines(1).class("notes-embed-title")),
        )
        .child(
            ToolButton::new(MI_OPEN_IN_NEW)
                .tooltip(tr!("notes.embed.open"))
                .on_click(move || {
                    ctx.activate(&id);
                    crate::rail::navigate("notes");
                }),
        )
}

/// Доска/диаграмма живут в высоте своего блока — без рамки и шапки: у
/// них своё оформление, а выбранный блок и так виден в панели свойств.
fn sized(body: Box<dyn Widget>, height: f32) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .style("height", syngui::mss::StyleValue::px(height.max(80.0)))
            .child(crate::components::workspace_frame::expand(body)),
    )
}

fn note_stub(icon: &'static str, text: String) -> Box<dyn Widget> {
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("notes-embed-header")
            .child(Icon::new(icon).class("notes-embed-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
