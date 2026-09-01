//! Правая панель «Заметок»: TabBar «Вставка | Свойства | Связи».
//!
//! T1 — каркас: «Вставка» перечисляет типы блоков (клик вставляет через
//! slash-каталог придёт в T11 вместе с перетаскиванием), «Свойства» и
//! «Связи» — заглушки до этапов T3/T11.

use syngui::prelude::*;
use syngui::widgets::navigation::{Tab, TabBar};

use crate::icons::*;

use super::state::{NoteKind, NotesCtx};
use super::storage;

pub const TAB_INSERT: usize = 0;
pub const TAB_PROPS: usize = 1;
pub const TAB_LINKS: usize = 2;

pub fn header() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let tab = ctx.right_tab;
    let tabbar = TabBar::new()
        .tab(Tab::new(tr!("notes.right.tab.insert"), TAB_INSERT, &tab).icon(MI_ADD))
        .tab(Tab::new(tr!("notes.right.tab.props"), TAB_PROPS, &tab).icon(MI_TUNE))
        .tab(Tab::new(tr!("notes.right.tab.links"), TAB_LINKS, &tab).icon(MI_HUB))
        .class("right-panel-tabbar-inner");
    DecoratedBox::new().class("right-panel-tabbar").child(tabbar)
}

pub fn body() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let tab = ctx.right_tab.get();
        let content: Box<dyn Widget> = match tab {
            TAB_INSERT => Box::new(insert_tab()),
            TAB_PROPS => Box::new(props_tab(ctx)),
            _ => Box::new(links_tab(ctx)),
        };
        vec![content]
    })
}

/// Вкладка «Связи»: обратные и исходящие ссылки активной страницы.
fn links_tab(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(active) = ctx.active.get() else {
            return vec![Box::new(placeholder(MI_HUB, tr!("notes.right.links.empty")))];
        };
        let index = ctx.index.get();
        let backlinks = index.backlinks_of(&active);
        let outgoing = index.outgoing_of(&active);
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-links-list");
        // Мини-граф окрестности страницы.
        col = col.child(
            DecoratedBox::new().class("notes-mini-graph").child(
                super::graph::mini(ctx, active.clone()),
            ),
        );
        if backlinks.is_empty() && outgoing.is_empty() {
            col = col.child(
                Text::new(tr!("notes.right.links.none")).class("notes-empty-hint"),
            );
            return vec![Box::new(ScrollView::new().vertical().child(col))];
        }
        if !backlinks.is_empty() {
            col = col.child(
                Text::new(tr!("notes.links.backlinks")).class("notes-links-section"),
            );
            for rel in backlinks {
                col = col.child(Stack::new().children(vec![link_row(ctx, rel)]));
            }
        }
        if !outgoing.is_empty() {
            col = col.child(
                Text::new(tr!("notes.links.outgoing")).class("notes-links-section"),
            );
            for rel in outgoing {
                col = col.child(Stack::new().children(vec![link_row(ctx, rel)]));
            }
        }
        vec![Box::new(ScrollView::new().vertical().child(col))]
    })
}

fn link_row(ctx: NotesCtx, rel: String) -> Box<dyn Widget> {
    let title = storage::title_of(&rel);
    let row = DecoratedBox::new().class("notes-insert-row").child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_DESCRIPTION).class("notes-insert-icon"))
            .child(Text::new(title).max_lines(1).class("notes-insert-label")),
    );
    Box::new(
        syngui::widgets::GestureDetector::new()
            .cursor(syngui::input::CursorIcon::Pointer)
            .on_click(move || ctx.open_path(&rel))
            .child(row),
    )
}

/// Палитра блоков: клик дописывает блок в конец активной страницы
/// (по месту каретки вставляет slash-меню «/»); «База» и «Канвас»
/// создают файл и вставляют живую врезку.
fn insert_tab() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let is_page = ctx
            .active_note()
            .map(|n| n.kind == NoteKind::Page)
            .unwrap_or(false);
        if !is_page {
            return vec![Box::new(placeholder(MI_ADD, tr!("notes.insert.no_page")))];
        }
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-insert-list");
        col = col.child(Text::new(tr!("notes.insert.hint")).class("notes-insert-hint"));

        type Ins = (&'static str, String, &'static str);
        let rows: Vec<Ins> = vec![
            (MI_ARTICLE, tr!("notes.insert.text"), "\nНовый абзац"),
            (MI_BOOK, tr!("notes.insert.heading"), "## Заголовок"),
            (MI_HORIZONTAL_RULE, tr!("notes.insert.divider"), "---"),
            (MI_GRID_ON, tr!("notes.insert.table"), "| A | B |\n| --- | --- |\n| 1 | 2 |"),
            (MI_CODE, tr!("notes.insert.code"), "```\nкод\n```"),
            (MI_CHAT, tr!("notes.insert.callout"), "> [!note] Заметка"),
        ];
        for (icon, label, snippet) in rows {
            let c = ctx;
            col = col.child(insert_row(icon, label, move || {
                c.append_to_active(snippet);
            }));
        }
        // Живые врезки: файл создаётся рядом и сразу встраивается.
        let c1 = ctx;
        col = col.child(insert_row(MI_GRID_ON, tr!("notes.insert.base_embed"), move || {
            insert_linked_file(c1, ".base.json");
        }));
        let c2 = ctx;
        col = col.child(insert_row(MI_ACCOUNT_TREE, tr!("notes.insert.canvas_embed"), move || {
            insert_linked_file(c2, ".canvas.json");
        }));
        vec![Box::new(ScrollView::new().vertical().child(col))]
    })
}

fn insert_row(
    icon: &'static str,
    label: String,
    on_click: impl Fn() + Send + Sync + 'static,
) -> impl Widget {
    syngui::widgets::GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Pointer)
        .on_click(on_click)
        .child(
            DecoratedBox::new().class("notes-insert-row").child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icon).class("notes-insert-icon"))
                    .child(Text::new(label).class("notes-insert-label")),
            ),
        )
}

/// Создать файл базы/канваса и вставить врезку в активную страницу.
fn insert_linked_file(ctx: NotesCtx, ext: &str) {
    let root = ctx.vault_path.get_untracked();
    let (title, content) = if ext == ".base.json" {
        (tr!("notes.untitled_base"), super::base::model::BaseDoc::template().serialize())
    } else {
        (
            tr!("notes.untitled_canvas"),
            super::canvas::model::CanvasDoc::template().serialize(),
        )
    };
    match storage::create_file(&root, &title, ext, &content) {
        Ok(rel) => {
            ctx.rescan();
            let embed_title = storage::title_of(&rel);
            ctx.append_to_active(&format!("![[{embed_title}]]"));
        }
        Err(e) => log::warn!("notes: не удалось создать файл врезки: {e}"),
    }
}

/// Инспектор «Свойства»: карточка канваса либо активная страница.
fn props_tab(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(note) = ctx.active_note() else {
            return vec![Box::new(placeholder(MI_TUNE, tr!("notes.right.props.empty")))];
        };
        // Выделенная карточка канваса.
        if let Some(handle) = note.canvas_handle() {
            if let Some(node_id) = handle.selected.get() {
                return vec![Box::new(canvas_card_props(handle.clone(), node_id))];
            }
        }
        if note.kind == NoteKind::Graph {
            return vec![Box::new(placeholder(MI_HUB, tr!("notes.right.props.empty")))];
        }
        vec![Box::new(page_props(ctx, note.path.clone(), note.title.clone()))]
    })
}

/// Свойства страницы: имя (Enter — переименовать файл) и путь.
fn page_props(ctx: NotesCtx, rel: String, title: String) -> impl Widget {
    let rename_rel = rel.clone();
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.name")).class("notes-links-section"))
        .child(
            TextField::new()
                .text(title)
                .submit_on_focus_lost(true)
                .on_submit(move |v| ctx.rename(&rename_rel, v))
                .class("notes-props-name"),
        )
        .child(Text::new(tr!("notes.props.path")).class("notes-links-section"))
        .child(Text::new(rel).max_lines(2).class("notes-props-path"))
}

/// Свойства выделенной карточки канваса: цвет и удаление.
fn canvas_card_props(handle: super::canvas::CanvasHandle, node_id: String) -> impl Widget {
    const PRESETS: &[&str] = &[
        "", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#8B95A6",
    ];
    let mut swatches = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for preset in PRESETS {
        let h = handle.clone();
        let id = node_id.clone();
        let color = preset.to_string();
        let mut dot = DecoratedBox::new().class("notes-props-swatch");
        if color.is_empty() {
            dot = dot.class("notes-props-swatch empty");
        } else {
            dot = dot.style("background-color", syngui::core::Color::from_hex(&color));
        }
        swatches = swatches.child(
            syngui::widgets::GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || h.set_node_color(&id, &color))
                .child(dot),
        );
    }
    let h_del = handle.clone();
    let id_del = node_id.clone();
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-props")
        .child(Text::new(tr!("notes.props.card_color")).class("notes-links-section"))
        .child(swatches)
        .child(
            Button::new(tr!("notes.canvas.delete"))
                .on_click(move || h_del.delete_node(&id_del))
                .class("notes-props-delete"),
        )
}

fn placeholder(icon: &'static str, text: String) -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("notes-empty-icon"))
            .child(Text::new(text).class("notes-empty-hint")),
    )
}
