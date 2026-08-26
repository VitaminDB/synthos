//! Левая колонка страницы настроек — список разделов.
//!
//! Весь блок обёрнут одним реактивным замыканием (подписывается на
//! `selected_settings_tab`). Внутри — статическая Column с 3 пунктами,
//! что гарантирует одинаковый layout всех элементов (без коллизий
//! Reactive-соседей в одной колонке). На этой странице
//! `MainAxisAlignment::Start` — размер колонки может быть по контенту.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::context::AppCtx;
use crate::icons::*;

#[derive(Clone, Copy)]
struct Tab {
    key: &'static str,
    icon: &'static str,
}

const TABS: &[Tab] = &[
    Tab { key: "general", icon: MI_TUNE },
    Tab { key: "themes", icon: MI_PALETTE },
    Tab { key: "skills", icon: MI_PSYCHOLOGY },
    Tab { key: "audio_models", icon: MI_HEADSET_MIC },
    Tab { key: "ai_models", icon: MI_AUTO_AWESOME },
    Tab { key: "knowledge_base", icon: MI_MENU_BOOK },
    Tab { key: "terminal", icon: MI_TERMINAL },
    Tab { key: "about", icon: MI_INFO },
];

pub fn view() -> impl Widget {
    DecoratedBox::new().class("settings-sidebar").child(move || {
        let ctx = use_context::<AppCtx>();
        let active = ctx.selected_settings_tab.get();

        let mut items: Vec<Box<dyn Widget>> = Vec::with_capacity(TABS.len() + 1);
        items.push(Box::new(Text::new(tr!("settings.title")).class("settings-sidebar-title")));
        for t in TABS {
            items.push(Box::new(tab_row(*t, active == t.key)));
        }

        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(items)
    })
}

fn tab_row(t: Tab, is_active: bool) -> impl Widget {
    let class = if is_active {
        "settings-tab-item selected"
    } else {
        "settings-tab-item"
    };
    let key = t.key;

    GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<AppCtx>();
            if ctx.selected_settings_tab.get_untracked() == key {
                return;
            }
            ctx.settings_router.lock().unwrap().navigate(key);
            ctx.selected_settings_tab.set(key.to_string());
        })
        .child(DecoratedBox::new().class(class).child(mgui! {
            Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(t.icon).class("settings-tab-icon"),
                DecoratedBox::new().class("grow").child(mgui! {
                    Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Text::new(tr!(&format!("settings.tabs.{}.title", t.key))).class("settings-tab-title"),
                        Text::new(tr!(&format!("settings.tabs.{}.subtitle", t.key))).class("settings-tab-subtitle"),
                    ]
                }),
            ]
        }))
}
