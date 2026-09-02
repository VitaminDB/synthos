//! Центр «Заметок»: WYSIWYG-редактор активной страницы (либо граф связей).
//!
//! Страница держит свою [`DocumentEditorHandle`] — модель переживает
//! переключения; сам виджет пересоздаётся Reactive'ом (в том числе по
//! `doc_epoch` после операций контекстного меню), но по общей ручке
//! продолжает ту же модель. Правый клик в документе открывает
//! [`doc_menu`] — оно живёт в Stack рядом с редактором.
//!
//! Редактор держит высоту не меньше видимой области (`fill_height`), чтобы
//! клик и правый клик ниже последнего блока попадали в документ, а
//! раскладку страницы (поток/свободная, сетка, привязка) отдаёт
//! `NotesCtx::active_doc_layout` из настроек страницы в дереве. В
//! свободной раскладке страница прокручивается и по горизонтали: блок,
//! ушедший за правый край, растягивает холст (распорка редактора), а
//! колонка потока при этом держит ширину видимой области.

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
        let layout = ctx.active_doc_layout();
        let editor = DocumentEditor::new()
            .markdown((*page.source).clone())
            .handle(&page.handle)
            .links(super::links::provider(ctx))
            .media(super::media::resolver(ctx))
            .embeds(super::embeds::factory(ctx))
            .model_epoch(ctx.doc_epoch.get())
            .layout(layout)
            .fill_height(true)
            .slash_items(doc_menu::slash_items())
            .on_slash_custom(doc_menu::slash_custom(ctx))
            .on_context_menu(move |pos| {
                ctx.doc_menu_pos.set(pos);
                ctx.doc_menu_open.set(true);
            })
            .on_drop_file(move |file, token| {
                super::media::ingest_dropped_file(ctx, page_id.clone(), file, token);
            })
            // Блок, отпущенный над карточкой доски, уезжает в неё; карточка,
            // отпущенная на документ, становится его блоками.
            .on_block_drop(move |pos, block| super::kanban::sinks::take_block(ctx, pos, block))
            .on_drop_data(move |pos, data| super::kanban::sinks::drop_on_page(ctx, pos, data))
            .class("notes-editor");
        // Поток не выходит за ширину колонки — прокрутка только вниз;
        // холст свободной раскладки — в обе стороны.
        let scroller = if layout.free { Page::new().both() } else { Page::new().vertical() };
        vec![Box::new(
            Stack::new()
                .clip(false)
                .child(scroller.child(editor).class("notes-editor-page"))
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
