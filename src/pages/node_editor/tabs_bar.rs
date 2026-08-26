//! TabsBar для node-editor — стиль терминальных вкладок (Zed/VSCode):
//! тонкий бар, accent-полоса под активной, hover-эффекты, × появляется
//! на hover. См. `terminal_tabs.mss` для эталона; здесь свои MSS-классы
//! (`.ne-tab-*`) живут в `node_editor_tabs.mss`.
//!
//! Архитектура:
//! - Reactive подписан на `tabs` + `active`
//! - tab_chip = GestureDetector(on_click=activate) > DecoratedBox(.ne-tab[--active]) > Row{icon, title, ×}
//! - Add-кнопка справа создаёт новую Untitled-вкладку.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{DecoratedBox, GestureDetector, Reactive, Row, ToolButton};

use crate::icons::{MI_ADD, MI_CLOSE, MI_PSYCHOLOGY, MI_PUBLIC, MI_TUNE};

use super::tabs::{EditorWorkspace, OpenTab};

pub fn view() -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ws = use_context::<EditorWorkspace>();
        let tabs = ws.tabs.get();
        let active = ws.active.get();

        let mut row = Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Start);
        // Скрытые (агентские) вкладки в полосе не показываются; `.get()` —
        // чтобы reveal из чата перерисовал бар.
        for tab in tabs.iter().filter(|t| !t.hidden.get()) {
            let is_active = active == Some(tab.id);
            row = row.child(tab_chip(ws, tab.clone(), is_active));
        }
        let add_btn = ToolButton::new(MI_ADD)
            .tooltip(tr!("nodes.tabs.new"))
            .on_click(move || {
                ws.template_picker_open.set(true);
            })
            .class("ne-tab-add");
        row = row.child(add_btn);

        let bar = DecoratedBox::new()
            .child(
                Row::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .child(row)
                    .child(DecoratedBox::new().class("ne-tabs-spacer")),
            )
            .class("ne-tabs-bar");
        vec![Box::new(bar)]
    })
}

fn tab_chip(ws: EditorWorkspace, tab: OpenTab, is_active: bool) -> impl Widget {
    let id = tab.id;
    let title_signal = tab.title;
    let dirty_signal = tab.dirty;
    let class = if is_active { "ne-tab ne-tab--active" } else { "ne-tab" };

    // Иконка зависит от происхождения вкладки: агентская (раскрытая из
    // чата), builtin/custom-template или Untitled.
    let icon_glyph = if tab.agent_chat.get_untracked().is_some() {
        MI_PSYCHOLOGY
    } else if tab.source.get_untracked().is_some() {
        MI_PUBLIC
    } else {
        MI_TUNE
    };
    let icon = Text::new(icon_glyph).class("ne-tab-icon");

    let title_box = DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let dirty = dirty_signal.get();
            let title = title_signal.get();
            let label = if dirty { format!("• {title}") } else { title };
            vec![Box::new(Text::new(label).class("ne-tab-title-text"))]
        }))
        .class("ne-tab-title");

    let close_btn = ToolButton::new(MI_CLOSE)
        .tooltip(tr!("nodes.tabs.close"))
        .on_click(move || {
            ws.close(id);
        })
        .class("ne-tab-close");

    let inner = DecoratedBox::new()
        .child(mgui! {
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Start) => [
                    icon,
                    title_box,
                    close_btn,
                ]
        })
        .class(class);

    GestureDetector::new()
        .on_click(move || {
            ws.activate(id);
        })
        .child(inner)
}

