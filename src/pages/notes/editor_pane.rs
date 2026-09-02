//! Центр «Заметок»: WYSIWYG-редактор активной страницы (либо граф связей).
//!
//! Страница держит свою [`DocumentEditorHandle`] — модель переживает
//! переключения; сам виджет пересоздаётся Reactive'ом (в том числе по
//! `doc_epoch` после операций контекстного меню), но по общей ручке
//! продолжает ту же модель. Правый клик в документе открывает
//! [`doc_menu`] — оно живёт в Stack рядом с редактором.

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditor;
use syngui::widgets::Page;

use crate::icons::*;

use super::doc_menu;
use super::state::NotesCtx;

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if ctx.show_graph.get() {
            return vec![Box::new(super::graph::page(ctx))];
        }
        let Some(page) = ctx.active_page() else {
            return vec![Box::new(empty_state())];
        };
        let page_id = page.id.clone();
        let editor = DocumentEditor::new()
            .markdown((*page.source).clone())
            .handle(&page.handle)
            .links(super::links::provider(ctx))
            .media(super::media::resolver(ctx))
            .embeds(super::embeds::factory(ctx))
            .model_epoch(ctx.doc_epoch.get())
            .slash_items(doc_menu::slash_items())
            .on_slash_custom(doc_menu::slash_custom(ctx))
            .on_context_menu(move |pos| {
                ctx.doc_menu_pos.set(pos);
                ctx.doc_menu_open.set(true);
            })
            .on_drop_file(move |file, token| {
                super::media::ingest_dropped_file(ctx, page_id.clone(), file, token);
            })
            .class("notes-editor");
        vec![Box::new(
            Stack::new()
                .clip(false)
                .child(Page::new().child(editor).class("notes-editor-page"))
                .child(doc_menu::popup(ctx)),
        )]
    })
}

fn empty_state() -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_EDIT_NOTE).class("notes-empty-icon"))
            .child(Text::new(tr!("notes.empty.title")).class("notes-empty-title"))
            .child(Text::new(tr!("notes.empty.hint")).class("notes-empty-hint")),
    )
}
