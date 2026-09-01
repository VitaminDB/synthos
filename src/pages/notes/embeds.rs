//! Живые врезки `![[Имя]]` в страницах заметок.
//!
//! Страница во врезке — read-only DocumentEditor со свежим содержимым с
//! диска (глубина ≤ 2, циклы отсекаются по цепочке целей). База и канвас
//! во врезке — живые и редактируемые: их handles берутся из открытой
//! плитки либо из скрытого пула `NotesCtx.embedded` (автосейв подписан и
//! на него — правки во врезке сохраняются как обычные).

use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocumentEditor, EmbedCtx, EmbedFactory};

use crate::icons::*;

use super::state::{NotePayload, NotesCtx};
use super::storage;

const MAX_DEPTH: usize = 2;

pub struct NotesEmbedFactory {
    ctx: NotesCtx,
}

pub fn factory(ctx: NotesCtx) -> Arc<NotesEmbedFactory> {
    Arc::new(NotesEmbedFactory { ctx })
}

impl EmbedFactory for NotesEmbedFactory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let rel = self.ctx.index.get_untracked().resolve(target)?;
        if ectx.depth >= MAX_DEPTH {
            return Some(note_stub(MI_EDIT_NOTE, tr!("notes.embed.too_deep")));
        }
        if ectx.chain.contains(&rel) {
            return Some(note_stub(MI_AUTORENEW, tr!("notes.embed.cycle")));
        }
        let ctx = self.ctx;
        let mut chain = ectx.chain.clone();
        chain.push(rel.clone());
        let inner_ctx = EmbedCtx { depth: ectx.depth + 1, chain };

        match storage::kind_of(&rel)? {
            storage::VaultEntryKind::Page => {
                let root = ctx.vault_path.get_untracked();
                let content = storage::load(&root, &rel).ok()?;
                let title = storage::title_of(&rel);
                Some(Box::new(
                    Column::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(embed_header(ctx, rel.clone(), title))
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
            storage::VaultEntryKind::Base => {
                let note = ctx.live_note(&rel)?;
                let NotePayload::Base(handle) = note.payload else { return None };
                Some(framed(
                    embed_header(ctx, rel, note.title),
                    Box::new(super::base::pane::view(handle)),
                ))
            }
            storage::VaultEntryKind::Canvas => {
                let note = ctx.live_note(&rel)?;
                let NotePayload::Canvas(handle) = note.payload else { return None };
                Some(framed(
                    embed_header(ctx, rel, note.title),
                    Box::new(super::canvas::pane::view(handle)),
                ))
            }
            storage::VaultEntryKind::Dir => None,
        }
    }
}

/// Шапка врезки: название + кнопка «открыть страницей».
fn embed_header(ctx: NotesCtx, rel: String, title: String) -> impl Widget {
    let open_rel = rel.clone();
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-embed-header")
        .child(Icon::new(MI_DESCRIPTION).class("notes-insert-icon"))
        .child(
            DecoratedBox::new()
                .class("grow")
                .child(Text::new(title).max_lines(1).class("notes-embed-title")),
        )
        .child(
            ToolButton::new(MI_OPEN_IN_NEW)
                .tooltip(tr!("notes.embed.open"))
                .on_click(move || {
                    ctx.open_path(&open_rel);
                    crate::rail::navigate("notes");
                }),
        )
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
            .child(Icon::new(icon).class("notes-insert-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
