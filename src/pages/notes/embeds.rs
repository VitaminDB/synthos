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
    target.starts_with("kanban:") || target.starts_with("gantt:")
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
