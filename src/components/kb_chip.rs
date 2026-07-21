//! KB chip + popover для chat input panel.
//!
//! Чип в left-toolbar показывает «Базы: N» (multi-select коллекций
//! знаний для текущего чата). Клик открывает Portal-popover со списком
//! всех доступных коллекций и тогглом auto-augment.
//!
//! Хранение состояния — все сигналы в [`KbCtx`](crate::kb::ctx::KbCtx):
//! - `active_in_chat_ids: RwSignal<Vec<String>>` — multi-select для
//!   tools (`kb_search`) и augment'а;
//! - `auto_augment: RwSignal<bool>` — флаг подмеса RAG в system prompt;
//! - `kb_chip_open: RwSignal<bool>` — popover открыт.
//!
//! Popover монтируется один раз в [`crate::pages::chat::view`] рядом с
//! `tool_confirm::view()`, поэтому overlay-Z-order и backdrop работают
//! корректно (Portal требует Stack-родителя для полноценного modal).

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{Checkbox, GestureDetector};

use crate::context::AppCtx;
use crate::icons::*;

/// Реактивный chip-кнопка для left-toolbar input-панели.
///
/// Используется как `kb_chip::view()` в `mgui!{ Row => [...] }`.
/// mgui-макрос автоматически оборачивает возвращаемое замыкание в
/// `Reactive`, поэтому подписки на `active_in_chat_ids` / `registry`
/// внутри будут пересобирать chip при изменениях.
pub fn view() -> impl Fn() -> GestureDetector + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let active = app.kb.active_in_chat_ids.get();
        let registry = app.kb.registry.get();

        // Если коллекций вообще нет в registry — приглушённый стиль
        // подсказывает «зайди в Settings → Базы знаний и создай». Если
        // коллекции есть, но ни одна не активирована для чата — тоже
        // приглушённый, чтобы пользователь видел разницу между
        // «настроено» и «выбрано».
        let class = if active.is_empty() {
            "input-kb-chip input-kb-chip-empty"
        } else {
            "input-kb-chip"
        };
        let label = if active.is_empty() {
            if registry.items.is_empty() {
                "Базы знаний".to_string()
            } else {
                "Базы знаний".to_string()
            }
        } else {
            format!("Базы: {}", active.len())
        };

        GestureDetector::new()
            .on_click(toggle_chip_open)
            .child(DecoratedBox::new().class(class).child(mgui! {
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_MENU_BOOK).class("input-kb-chip-icon"),
                    Text::new(label.clone()).class("input-kb-chip-text"),
                ]
            }))
            .class("input-kb-chip-wrap")
    }
}

fn toggle_chip_open() {
    let app = use_context::<AppCtx>();
    app.kb.kb_chip_open.update(|v| *v = !*v);
}

fn close_popover() {
    use_context::<AppCtx>().kb.kb_chip_open.set(false);
}

/// Portal-popover со списком коллекций + переключателем auto-augment.
/// Монтируется один раз в `pages::chat::view()` (родитель — Stack с
/// fit=Expand, иначе backdrop не покрывает экран).
pub fn popover() -> impl Widget {
    let is_open = {
        let app = use_context::<AppCtx>();
        app.kb.kb_chip_open
    };
    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        // PortalAnchor не поддерживает «относительно элемента», поэтому
        // используем viewport-абсолютный BottomStart с margins,
        // подобранными визуально под обычное расположение chip'а в
        // toolbar'е chat input. Если в будущем потребуется точная
        // привязка к chip'у — см. TODO в styles/components/kb_chip.mss.
        .anchor(PortalAnchor::BottomStart {
            margin_bottom: 96.0,
            margin_left: 280.0,
        })
        .width(380.0)
        .on_close(close_popover)
        .child(card_reactive())
}

fn card_reactive() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let app = use_context::<AppCtx>();
        // Не подписываемся на is_open — Portal сам управляет видимостью.
        let registry = app.kb.registry.get();
        let active = app.kb.active_in_chat_ids.get();
        let auto = app.kb.auto_augment.get();

        let body: Box<dyn Widget> = if registry.items.is_empty() {
            Box::new(empty_state())
        } else {
            Box::new(card_body(registry.items.clone(), active, auto))
        };
        vec![body]
    })
}

fn empty_state() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("kb-chip-card kb-chip-card-empty") => [
            Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_MENU_BOOK).class("kb-chip-empty-icon"),
                    DecoratedBox::new().class("grow").child(
                        Text::new("Нет коллекций").class("kb-chip-title"),
                    ),
                    ToolButton::new(MI_CLOSE)
                        .on_click(close_popover)
                        .class("kb-chip-close"),
                ],
                Text::new(
                    "Создайте коллекцию в Settings → Базы знаний, чтобы подключать её к чату.",
                )
                .class("kb-chip-empty-hint"),
                Row::new().gap(0.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new("Открыть настройки")
                        .leading_icon(MI_SETTINGS)
                        .on_click(open_kb_settings)
                        .class("kb-chip-footer-btn"),
                ],
            ]
        ]
    }
}

fn card_body(
    items: Vec<crate::kb::CollectionMeta>,
    active: Vec<String>,
    auto: bool,
) -> impl Widget {
    let rows: Vec<Box<dyn Widget>> = items
        .iter()
        .map(|meta| collection_row(meta, active.iter().any(|x| x == &meta.id)))
        .collect();

    let header = mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_MENU_BOOK).class("kb-chip-header-icon"),
            DecoratedBox::new().class("grow").child(
                Text::new("Базы знаний для чата").class("kb-chip-title"),
            ),
            ToolButton::new(MI_CLOSE)
                .on_click(close_popover)
                .class("kb-chip-close"),
        ]
    };

    let auto_toggle = Checkbox::checked(auto)
        .label("Auto-augment system prompt")
        .on_change(|v| {
            use_context::<AppCtx>().kb.auto_augment.set(v);
        })
        .class("kb-chip-switch");

    mgui! {
        DecoratedBox::new().class("kb-chip-card") => [
            Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header,
                DecoratedBox::new().class("kb-chip-divider"),
                DecoratedBox::new().class("kb-chip-switch-row") => [ auto_toggle ],
                DecoratedBox::new().class("kb-chip-divider"),
                DecoratedBox::new().class("kb-chip-list-wrap").child(
                    ScrollView::new().vertical().class("kb-chip-list-scroll").child(
                        Column::new()
                            .gap(2.0)
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .children(rows),
                    ),
                ),
                DecoratedBox::new().class("kb-chip-divider"),
                Row::new().gap(0.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new("Открыть настройки")
                        .leading_icon(MI_SETTINGS)
                        .on_click(open_kb_settings)
                        .class("kb-chip-footer-btn"),
                ],
            ]
        ]
    }
}

fn collection_row(meta: &crate::kb::CollectionMeta, in_chat: bool) -> Box<dyn Widget> {
    let id = meta.id.clone();
    let name = meta.name.clone();
    let subtitle = format!(
        "{} док · {} чанков",
        meta.document_count, meta.chunk_count
    );
    let class = if in_chat {
        "kb-chip-row kb-chip-row-active"
    } else {
        "kb-chip-row"
    };
    Box::new(mgui! {
        GestureDetector::new()
            .on_click({
                let id = id.clone();
                move || toggle_in_chat(&id)
            })
            .child(DecoratedBox::new().class(class).child(mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Checkbox::checked(in_chat).class("kb-chip-row-cbox"),
                    DecoratedBox::new().class("grow") => [
                        Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            Text::new(name.clone()).class("kb-chip-row-name"),
                            Text::new(subtitle.clone()).class("kb-chip-row-meta"),
                        ]
                    ],
                ]
            }))
    })
}

fn toggle_in_chat(id: &str) {
    let app = use_context::<AppCtx>();
    let id = id.to_string();
    app.kb.active_in_chat_ids.update(move |list| {
        if let Some(pos) = list.iter().position(|x| x == &id) {
            list.remove(pos);
        } else {
            list.push(id.clone());
        }
    });
}

fn open_kb_settings() {
    let app = use_context::<AppCtx>();
    // Сначала закрываем popover — иначе после смены route он остался
    // бы зависшим над settings-страницей.
    app.kb.kb_chip_open.set(false);
    // Главный route → settings, вложенный route → knowledge_base.
    // См. компонент nav_rail.rs — там используется ровно этот паттерн.
    app.current_route.set("settings".to_string());
    app.selected_settings_tab
        .set("knowledge_base".to_string());
}
