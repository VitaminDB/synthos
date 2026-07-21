//! Левая колонка «Chats» — реактивный список чатов с CRUD.
//!
//! Структура (сохраняем визуал скриншота):
//! ```
//! Chats                               [+]  ← шапка с кнопкой создания
//! [ Filter (3) • ]                         ← фильтр-чип (декоративный)
//! Разговоры (N)                        ⌄   ← группа со счётчиком
//!   <карточки чатов>                       ← из ChatCtx.chats
//! ```
//!
//! Данные — из `AppCtx.chat.chats` (реактивный сигнал). Пустое состояние —
//! подсказка «Нажмите +, чтобы создать чат». Никакого mock — полный путь
//! от диска до UI живёт в `chat::storage` / `chat::registry`.

use syngui::mgui;
use syngui::prelude::*;

use crate::chat::registry;
use crate::context::AppCtx;
use crate::icons::*;

use super::chat_item;

pub fn view() -> impl Widget {
    let content = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header_row())
        .child(filter_row())
        .child(group_header_reactive())
        .child(chats_list_reactive());

    let scroll = ScrollView::new().vertical().child(content);

    DecoratedBox::new().class("chats-column").child(scroll)
}

// ─────────────────────────────────────────────────────────────────────────────
// Верх — заголовок + кнопка «+»
// ─────────────────────────────────────────────────────────────────────────────

fn header_row() -> impl Widget {
    DecoratedBox::new().class("chats-header-row").child(mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new("Chats").class("chats-column-title"),
                ToolButton::new(MI_ADD)
                    .tooltip("Новый чат")
                    .on_click(|| { registry::create_new(); })
                    .class("chats-header-add"),
            ]
    })
}

fn filter_row() -> impl Widget {
    DecoratedBox::new().class("chats-filter-wrap").child(mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("filter-chip").child(mgui! {
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_FILTER_LIST).class("filter-chip-icon"),
                    Text::new("Filter (3)").class("filter-chip-text"),
                    DecoratedBox::new().class("filter-chip-dot"),
                ]
            }),
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Заголовок группы «Разговоры (N)»
// ─────────────────────────────────────────────────────────────────────────────

fn group_header_reactive() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let count = ctx.chat.chats.get().len();
        let text = if count == 0 {
            "Разговоры".to_string()
        } else {
            format!("Разговоры ({})", count)
        };
        DecoratedBox::new().class("chats-group-wrap").child(mgui! {
            Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(text).class("chats-group-label"),
                Icon::new(MI_EXPAND_MORE).class("chats-group-chev"),
            ]
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Реактивный список чатов
// ─────────────────────────────────────────────────────────────────────────────

fn chats_list_reactive() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let chats = ctx.chat.chats.get();
        let active = ctx.chat.active_chat_id.get();

        // Лента карточек обёрнута в `.chats-list-wrap` — тонкий горизонтальный
        // padding, чтобы карточки не упирались в боковую кромку панели и
        // оставалось место для рамок/hover-фона.
        let inner: Column = if chats.is_empty() {
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(empty_hint())
        } else {
            let mut items: Vec<Box<dyn Widget>> = Vec::with_capacity(chats.len());
            for meta in chats.iter() {
                let selected = active.as_deref() == Some(&meta.id);
                items.push(chat_item::row(meta, selected));
            }
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(items)
        };

        DecoratedBox::new().class("chats-list-wrap").child(inner)
    }
}

fn empty_hint() -> impl Widget {
    DecoratedBox::new()
        .class("chats-empty-hint")
        .child(Text::new("Нажмите +, чтобы создать чат").class("chats-empty-hint-text"))
}
