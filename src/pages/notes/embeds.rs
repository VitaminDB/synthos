//! Живые врезки `![[…]]` в страницах.
//!
//! `![[base:<id>]]` / `![[canvas:<id>]]` — объекты проекта, живые и
//! редактируемые прямо в странице (ручки из пула `NotesCtx.objects`,
//! автосейв подписан и на них). `![[Название]]` — другая страница read-only
//! (глубина ≤ 2, циклы отсекаются по цепочке id).

use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocumentEditor, EmbedCtx, EmbedFactory};

use crate::icons::*;

use super::state::{LiveObject, NotesCtx, TAB_PROPS};

const MAX_DEPTH: usize = 2;

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
        if let Some(id) = target.strip_prefix("base:") {
            let LiveObject::Base { handle, .. } = ctx.object("base", id.trim())? else { return None };
            return Some(framed(
                object_header(MI_GRID_ON, tr!("notes.embed.base")),
                Box::new(super::base::pane::view(handle)),
            ));
        }
        if let Some(id) = target.strip_prefix("canvas:") {
            let LiveObject::Canvas { handle, .. } = ctx.object("canvas", id.trim())? else { return None };
            // Выделение карточки переключает правую панель на «Свойства».
            handle.set_on_select(move || ctx.right_tab.set(TAB_PROPS));
            return Some(framed(
                object_header(MI_ACCOUNT_TREE, tr!("notes.embed.canvas")),
                Box::new(super::canvas::pane::view(handle)),
            ));
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
        let inner_ctx = EmbedCtx { depth: ectx.depth + 1, chain };
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

fn object_header(icon: &'static str, title: String) -> impl Widget {
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-embed-header")
        .child(Icon::new(icon).class("notes-embed-icon"))
        .child(Text::new(title).max_lines(1).class("notes-embed-title"))
}

/// База/канвас во врезке живут в фиксированной высоте.
fn framed(header: impl Widget + 'static, body: Box<dyn Widget>) -> Box<dyn Widget> {
    Box::new(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(
                DecoratedBox::new()
                    .style("height", syngui::mss::StyleValue::px(340.0))
                    .class("notes-embed-frame")
                    .child(crate::components::workspace_frame::expand(body)),
            ),
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
