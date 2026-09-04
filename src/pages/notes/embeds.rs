//! Живые врезки `![[…]]` в страницах.
//!
//! `![[kanban:<id>]]` / `![[gantt:<id>]]` — примитивы-объекты проекта
//! (канбан-доска, диаграмма Ганта), живые и редактируемые прямо в странице
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

/// Окружение календаря: хранилище событий проекта, слой сроков досок и
/// задач Ганта (read-only), открытие страницы.
pub fn calendar_env(ctx: NotesCtx) -> super::calendar::CalendarEnv {
    super::calendar::CalendarEnv {
        store: ctx.calendar_store(),
        external: Arc::new(move |from, to| external_items(ctx, from, to)),
        open_page: Arc::new(move |id| {
            ctx.activate(id);
            crate::rail::navigate("notes");
        }),
    }
}

/// Сроки карточек досок и задачи Ганта всех страниц в диапазоне дней.
fn external_items(ctx: NotesCtx, from: i64, to: i64) -> Vec<super::calendar::ExternalItem> {
    use super::calendar::ExternalItem;
    use super::gantt::calendar::parse_days;
    let mut out = Vec::new();
    for pid in ctx.tree.get_untracked().all_ids() {
        for (kind, oid) in super::state::object_refs(&ctx.page_markdown(&pid)) {
            match (kind.as_str(), ctx.object(&kind, &oid)) {
                ("kanban", Some(LiveObject::Kanban { handle, .. })) => {
                    let doc = handle.lock();
                    for c in &doc.cards {
                        let Some(day) = c.due.as_deref().and_then(parse_days) else { continue };
                        if day < from || day > to {
                            continue;
                        }
                        let color = doc.columns.iter().find(|col| col.id == c.column).map(|col| col.color.clone()).unwrap_or_default();
                        out.push(ExternalItem { day, end_day: day, title: c.title.clone(), color, page: pid.clone() });
                    }
                }
                ("gantt", Some(LiveObject::Gantt { handle, .. })) => {
                    for t in &handle.lock().tasks {
                        let Some((s, e)) = t.span_days() else { continue };
                        if e < from || s > to {
                            continue;
                        }
                        out.push(ExternalItem { day: s, end_day: e, title: t.name.clone(), color: t.color.clone(), page: pid.clone() });
                    }
                }
                _ => {}
            }
        }
    }
    out
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
            return Some(sized(Box::new(super::gantt::view::view(handle)), height));
        }
        if let Some(id) = target.strip_prefix("mindmap:") {
            let LiveObject::Mindmap { handle, id: oid } = ctx.object("mindmap", id.trim())? else { return None };
            return Some(sized(Box::new(super::mindmap::view::view(mindmap_env(ctx), oid, handle)), height));
        }
        if let Some(id) = target.strip_prefix("calendar:") {
            let LiveObject::Calendar { handle, id: oid } = ctx.object("calendar", id.trim())? else { return None };
            return Some(sized(Box::new(super::calendar::view::view(calendar_env(ctx), oid, handle)), height));
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
