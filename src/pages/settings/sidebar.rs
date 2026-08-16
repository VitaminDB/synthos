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
    title: &'static str,
    subtitle: &'static str,
}

const TABS: &[Tab] = &[
    Tab { key: "general",      icon: MI_TUNE,         title: "Общие",        subtitle: "Уведомления и интерфейс" },
    Tab { key: "themes",       icon: MI_PALETTE,      title: "Темы",         subtitle: "Светлые и тёмные оформления" },
    Tab { key: "skills",       icon: MI_PSYCHOLOGY,   title: "Скилы",        subtitle: "Инструкции и подсказки" },
    Tab { key: "audio_models",   icon: MI_HEADSET_MIC,  title: "Аудио модели", subtitle: "Распознавание речи" },
    Tab { key: "ai_models",      icon: MI_SMART_TOY,    title: "AI модели",    subtitle: "Квантование LLM: Qwen3.6/3.8, Muse Glimmer (Syn-чат)" },
    Tab { key: "knowledge_base", icon: MI_MENU_BOOK,    title: "Базы знаний",  subtitle: "RAG-коллекции для агента" },
    Tab { key: "terminal",       icon: MI_TERMINAL,     title: "Терминал",     subtitle: "Шрифт встроенного VTE" },
    Tab { key: "about",          icon: MI_INFO,         title: "О программе",  subtitle: "Версия, авторы, лицензия" },
];

pub fn view() -> impl Widget {
    DecoratedBox::new().class("settings-sidebar").child(move || {
        let ctx = use_context::<AppCtx>();
        let active = ctx.selected_settings_tab.get();

        let mut items: Vec<Box<dyn Widget>> = Vec::with_capacity(TABS.len() + 1);
        items.push(Box::new(Text::new("Настройки").class("settings-sidebar-title")));
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
                        Text::new(t.title).class("settings-tab-title"),
                        Text::new(t.subtitle).class("settings-tab-subtitle"),
                    ]
                }),
            ]
        }))
}
