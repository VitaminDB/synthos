//! Правая панель «Заметок»: TabBar «Вставка | Свойства | Связи».
//!
//! T1 — каркас: «Вставка» перечисляет типы блоков (клик вставляет через
//! slash-каталог придёт в T11 вместе с перетаскиванием), «Свойства» и
//! «Связи» — заглушки до этапов T3/T11.

use syngui::prelude::*;
use syngui::widgets::navigation::{Tab, TabBar};

use crate::icons::*;

use super::state::NotesCtx;
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
            TAB_PROPS => Box::new(placeholder(MI_TUNE, tr!("notes.right.props.empty"))),
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

/// Палитра блоков: пока справочная (что умеет «/»); перетаскивание в
/// документ придёт в T11.
fn insert_tab() -> impl Widget {
    let rows: &[(&str, String)] = &[
        (MI_ARTICLE, tr!("notes.insert.text")),
        (MI_BOOK, tr!("notes.insert.heading")),
        (MI_HORIZONTAL_RULE, tr!("notes.insert.divider")),
        (MI_GRID_ON, tr!("notes.insert.table")),
        (MI_CODE, tr!("notes.insert.code")),
        (MI_CHAT, tr!("notes.insert.callout")),
        (MI_ACCOUNT_TREE, tr!("notes.insert.canvas")),
    ];
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-insert-list");
    col = col.child(
        Text::new(tr!("notes.insert.hint"))
            .class("notes-insert-hint"),
    );
    for (icon, label) in rows.iter() {
        let icon: &'static str = icon;
        col = col.child(
            DecoratedBox::new().class("notes-insert-row").child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icon).class("notes-insert-icon"))
                    .child(Text::new(label.clone()).class("notes-insert-label")),
            ),
        );
    }
    ScrollView::new().vertical().child(col)
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
