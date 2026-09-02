//! Дерево блоков активной страницы — вторая вкладка левой панели.
//!
//! Зачем оно: блок можно создать пустым (разделитель, свежая таблица,
//! параграф без текста) — в документе такой блок не видно вовсе, ни где он,
//! ни какого он размера. В дереве он есть строкой со своим типом: по клику
//! становится текущим (каретка внутрь, рамка габаритов в документе), и над
//! ним работает панель «Свойства».
//!
//! Источник — [`DocumentEditorHandle::outline`]; выбор общий с редактором
//! через сигнал `handle.selected()`.

use syngui::input::CursorIcon;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::input::document_editor::{BlockOutline, DocOp};
use syngui::widgets::GestureDetector;

use crate::icons::*;

use super::state::NotesCtx;

/// Иконка и человекочитаемое имя типа блока.
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "heading" => MI_FORMAT_BOLD,
        "bullet" => MI_FORMAT_LIST_BULLETED,
        "numbered" => MI_FORMAT_LIST_NUMBERED,
        "todo" => MI_CHECK,
        "toggle" => MI_EXPAND_MORE,
        "quote" => MI_WRAP_TEXT,
        "callout" => MI_CAMPAIGN,
        "code" => MI_CODE,
        "table" => MI_GRID_ON,
        "divider" => MI_HORIZONTAL_RULE,
        "media" => MI_IMAGE_ICON,
        "embed" => MI_ACCOUNT_TREE,
        "shape" => MI_CATEGORY,
        _ => MI_ARTICLE,
    }
}

pub fn kind_label(kind: &str, level: u8) -> String {
    match kind {
        "heading" => match level {
            1 => tr!("notes.block.h1"),
            2 => tr!("notes.block.h2"),
            _ => tr!("notes.block.h3"),
        },
        "bullet" => tr!("notes.block.bullet"),
        "numbered" => tr!("notes.block.numbered"),
        "todo" => tr!("notes.block.todo"),
        "toggle" => tr!("notes.block.toggle"),
        "quote" => tr!("notes.block.quote"),
        "callout" => tr!("notes.block.callout"),
        "code" => tr!("notes.block.code"),
        "table" => tr!("notes.block.table"),
        "divider" => tr!("notes.block.divider"),
        "media" => tr!("notes.block.media"),
        "embed" => tr!("notes.block.embed"),
        "shape" => tr!("notes.block.shape"),
        _ => tr!("notes.block.text"),
    }
}

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Перестраиваемся на правках документа и на смене страницы.
        let _ = ctx.doc_epoch.get();
        let Some(page) = ctx.active_page() else {
            return vec![Box::new(empty(tr!("notes.blocks.no_page")))];
        };
        let _ = page.handle.revision().get();
        let selected = page.handle.selected().get();
        let outline = page.handle.outline();
        if outline.is_empty() {
            return vec![Box::new(empty(tr!("notes.blocks.empty")))];
        }
        let mut col = Column::new()
            .gap(1.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-tree");
        let mut rows = Vec::new();
        flatten(&outline, 0, &mut rows);
        for (depth, b) in rows {
            let is_sel = selected == Some(b.id);
            col = col.child(Stack::new().children(vec![row(ctx, &b, depth, is_sel)]));
        }
        vec![Box::new(col)]
    });
    ScrollView::new().vertical().class("notes-tree-scroll").child(list)
}

fn flatten(nodes: &[BlockOutline], depth: usize, out: &mut Vec<(usize, BlockOutline)>) {
    for n in nodes {
        out.push((depth, n.clone()));
        flatten(&n.children, depth + 1, out);
    }
}

fn row(ctx: NotesCtx, b: &BlockOutline, depth: usize, selected: bool) -> Box<dyn Widget> {
    let id = b.id;
    let indent = 4.0 + depth as f32 * 12.0;
    // У фигуры подпись — машинное имя вида (`rect`), в дереве нужен
    // локализованный: имя вида и есть его тип.
    let label = if b.kind == "shape" {
        syngui::widgets::input::document_editor::ShapeKind::from_name(&b.label)
            .map(super::doc_menu::shape_label)
            .unwrap_or_else(|| kind_label(b.kind, b.level))
    } else if b.label.trim().is_empty() {
        kind_label(b.kind, b.level)
    } else {
        b.label.clone()
    };
    let mut class = String::from("notes-tree-row notes-block-row");
    if selected {
        class.push_str(" selected");
    }
    let mut line = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().style("width", StyleValue::px(indent)))
        .child(Icon::new(kind_icon(b.kind)).class("notes-block-icon"))
        .child(
            DecoratedBox::new()
                .class("grow")
                .child(Text::new(label).max_lines(1).class("notes-tree-name")),
        );
    // Закреплённый на холсте блок помечен — так видно, что он живёт по
    // своим координатам, а не в колонке потока.
    if b.pinned {
        line = line.child(Icon::new(MI_PUSH_PIN).class("notes-block-pin"));
    }
    Box::new(
        GestureDetector::new()
            .cursor(CursorIcon::Pointer)
            .on_click(move || ctx.doc_op(DocOp::Select(id)))
            .child(DecoratedBox::new().class(class).child(line)),
    )
}

fn empty(text: String) -> impl Widget {
    DecoratedBox::new()
        .class("notes-tree-empty")
        .child(Text::new(text).class("notes-tree-empty-text"))
}
