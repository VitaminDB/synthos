//! Таб-бар центра страницы SynExplorer.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::feedback::Tooltip;

use crate::icons::{MI_BOOK, MI_EDIT, MI_LIST_ALT, MI_VISIBILITY};

use super::state::{SynExplorerCtx, TabKind};

pub fn tab_bar() -> impl Widget {
    DecoratedBox::new().class("syn-tab-bar").child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let active = ctx.current_tab.get();
        let row = mgui! {
            Row::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    tab_item(active, TabKind::Overview, MI_BOOK, "Метаданные пакета и статистика чанков"),
                    tab_item(active, TabKind::Files, MI_LIST_ALT, "Все файлы внутри пакета — имя, тип, размер, хеши"),
                    tab_item(active, TabKind::Metadata, MI_EDIT, "Редактирование BundleMeta: id, version, arch, purpose, компоненты"),
                    tab_item(active, TabKind::Preview, MI_VISIBILITY, "Просмотр содержимого выбранного файла"),
                ]
        };
        vec![Box::new(row)]
    }))
}

fn tab_item(active: TabKind, kind: TabKind, icon: &'static str, tooltip: &'static str) -> impl Widget {
    let is_active = active == kind;
    let class = if is_active { "syn-tab active" } else { "syn-tab" };
    let content = mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(icon).class("syn-tab-icon"),
                Text::new(kind.label()).class("syn-tab-label"),
            ]
    };
    let tile = DecoratedBox::new().class(class).child(content);
    let clickable = GestureDetector::new()
        .child(tile)
        .on_click(move || {
            let ctx = use_context::<SynExplorerCtx>();
            if ctx.current_tab.get_untracked() != kind {
                ctx.current_tab.set(kind);
            }
        });
    Tooltip::new(clickable, tooltip)
}
