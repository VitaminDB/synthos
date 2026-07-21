//! Header активного чата — реактивно отображает title и имя модели.
//!
//! Пока активного чата нет — показываем заглушку «Выберите или создайте чат».
//! Аватар хедера — те же цвета, что используются в карточках слева
//! (`chat_item::tone_for(id)`), чтобы визуально связать список и шапку.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::{MenuItem, PopupMenu};

use crate::chat::registry;
use crate::context::AppCtx;
use crate::icons::*;

use super::chat_item;

pub fn view() -> impl Widget {
    // Spacer с `flex-grow: 1` отодвигает правый блок (индикатор контекста +
    // кнопка «⋯») к правому краю. `.class("grow")` на самом
    // `header_body_reactive` не сработал бы — Row видит Reactive-обёртку,
    // а не внутренний DecoratedBox.
    DecoratedBox::new().class("chat-header").child(mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            header_body_reactive(),
            DecoratedBox::new().class("grow"),
            context_indicator_reactive(),
            more_menu_widget(),
        ]
    })
}

/// Кнопка «⋯» в правом углу шапки + popup-меню под ней.
///
/// Меню переключает `general.tool_display_mode` (см. `context.rs`) — глобальная
/// настройка отображения tool-карточек в ленте: `full` / `minimal` / `hidden`.
/// Активный пункт помечается иконкой `MI_CHECK`. Меню реактивно — пересобирается
/// при смене значения сигнала, чтобы галочка переезжала на актуальный пункт.
///
/// `menu_open` и `menu_pos` создаются ровно один раз (вне реактивного замыкания),
/// иначе на каждой пересборке они бы пересоздавались как новые сигналы.
fn more_menu_widget() -> impl Fn() -> Column + Send + Sync + 'static {
    let menu_open = use_signal(false);
    let menu_pos = use_signal(Point::zero());

    move || {
        let app = use_context::<AppCtx>();
        let mode_sig = app.general.tool_display_mode;
        // Подписка на mode → пересборка items при смене режима.
        let current = mode_sig.get();

        let mark = |key: &'static str, label: &'static str, current: &str| -> MenuItem {
            let mut item = MenuItem::new(key, label);
            if key == current {
                item = item.icon(MI_CHECK);
            }
            item
        };

        let trigger = ToolButton::new(MI_MORE_HORIZ)
            .on_click_at(move |pos| {
                menu_pos.set(pos);
                menu_open.set(true);
            })
            .class("chat-header-action");

        let menu = PopupMenu::new()
            .is_open(menu_open)
            .position(menu_pos)
            .min_width(260.0)
            .items(vec![
                mark("full",    "Полный вывод инструментов", &current),
                mark("minimal", "Минимальная информация",    &current),
                mark("hidden",  "Скрыть инструменты",        &current),
            ])
            .on_select(move |id| {
                // Принимаем только известные ключи; иначе игнорируем.
                if matches!(id, "full" | "minimal" | "hidden") {
                    mode_sig.set(id.to_string());
                }
            });

        // Column вместо Stack: PopupMenu сам всплывает в overlay,
        // оставаясь логическим соседом ToolButton по дереву.
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::End)
            .child(trigger)
            .child(menu)
    }
}

/// Компактный индикатор использования контекста llama в хедере чата.
///
/// Знаменатель — `llama_slot.n_ctx` (размер окна), опрашивается в `session.rs`
/// через `/slots` при старте генерации. Если `/slots` ещё не отвечал
/// (свежий старт, сервер не запущен) — fallback на `ModelConfig.ctx_size`
/// для активной модели. Числитель приоритетно — `usage.total_tokens` из
/// финального чанка стрима (авторитет по завершению); fallback —
/// `slot.next_token.n_decoded` для живого прогресса во время генерации.
///
/// Виджет показывается ВСЕГДА: даже без usage/slot в строке стоит ProgressBar
/// со значением 0 и подписью «—». Это даёт пользователю стабильную точку
/// в шапке и место под кнопку «Compact now», которая запускает ручной
/// autocompact.
fn context_indicator_reactive(
) -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let slot = ctx.metrics.llama_slot.get();
        let usage = ctx.metrics.llama_usage.get();

        // n_ctx: live из slot → fallback на ctx_size активной модели.
        let total_live = slot
            .as_ref()
            .and_then(|s| s.n_ctx)
            .unwrap_or(0)
            .max(0) as u64;
        let total = if total_live > 0 {
            total_live
        } else {
            ctx.selected_model
                .get()
                .and_then(|name| {
                    ctx.models
                        .get()
                        .into_iter()
                        .find(|m| m.name == name)
                        .map(|m| m.ctx_size as u64)
                })
                .unwrap_or(0)
        };

        let used_from_usage = usage.as_ref().and_then(|u| {
            u.total_tokens
                .or_else(|| match (u.prompt_tokens, u.completion_tokens) {
                    (Some(p), Some(c)) => Some(p + c),
                    (Some(p), None) => Some(p),
                    (None, Some(c)) => Some(c),
                    _ => None,
                })
        });
        let used_from_slot = slot
            .as_ref()
            .and_then(|s| s.next_token.n_decoded);
        let used = used_from_usage
            .or(used_from_slot)
            .unwrap_or(0)
            .max(0) as u64;

        let (ratio, caption, bar_class) = if total > 0 {
            let r = (used as f32 / total as f32).clamp(0.0, 1.0);
            let pct = (r * 100.0).round() as u32;
            let cap = format!(
                "{}% · {} / {}",
                pct,
                fmt_thin_number(used),
                fmt_thin_number(total),
            );
            // Цвет полосы: warn (оранжевый) при ≥ порог autocompact'а —
            // дополнительный визуальный сигнал «пора сжимать».
            let threshold = ctx.general.autocompact_threshold_percent.get();
            let class = if pct >= threshold {
                "chat-header-ctx-bar chat-header-ctx-bar-warn"
            } else {
                "chat-header-ctx-bar"
            };
            (r, cap, class)
        } else {
            (0.0_f32, "— / —".to_string(), "chat-header-ctx-bar")
        };

        // Кнопка «Compact now»: видна всегда. Disabled если идёт стрим/agent-
        // loop ИЛИ в ленте нет кандидатов на сжатие (find_compact_range = None).
        let pending = ctx.chat.pending.get();
        let msgs = ctx.chat.messages.get();
        let can_compact = !pending && crate::chat::compact::find_compact_range(&msgs).is_some();

        let compact_btn = ToolButton::new(MI_COMPRESS)
            .tooltip("Сжать старые сообщения в краткое system-summary")
            .disabled(!can_compact)
            .on_click(|| crate::chat::compact::compact_now())
            .class("chat-header-ctx-compact");

        let row = mgui! {
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_DNS).class("chat-header-ctx-icon"),
                DecoratedBox::new().class("chat-header-ctx-bar-wrap").child(
                    ProgressBar::new().value(ratio).class(bar_class),
                ),
                Text::new(caption).class("chat-header-ctx-text"),
                compact_btn,
            ]
        };

        DecoratedBox::new().class("chat-header-ctx").child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![Box::new(row) as Box<dyn Widget>]),
        )
    }
}

/// Форматирование числа с узким неразрывным пробелом между тысячами.
/// Отдельно от `details::fmt_number`, чтобы не тянуть зависимости между
/// панелями (и не плодить re-export).
fn fmt_thin_number(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Реактивное тело — аватар + title/subtitle. Обновляется при смене активного
/// чата и его метаданных (title / model_name).
fn header_body_reactive() -> impl Fn() -> Column + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let _ = ctx.chat.chats.get();           // подписка на изменения meta
        let _ = ctx.chat.active_chat_id.get();  // подписка на смену активного

        let body: Box<dyn Widget> = match registry::active_meta() {
            None => Box::new(empty_header()),
            Some(meta) => Box::new(active_header(
                &meta.title,
                meta.model_name.as_deref(),
                &meta.id,
            )),
        };

        // Без обёртки-grow: растягивание к правому краю даёт внешний spacer
        // в `view()`. Здесь — просто «контейнер content-size».
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![body])
    }
}

fn active_header(title: &str, model: Option<&str>, chat_id: &str) -> impl Widget {
    let tone = chat_item::tone_for(chat_id);
    let initials = initials_from_title(title);
    let subtitle = match model {
        Some(name) if !name.trim().is_empty() => format!("Модель: {}", name),
        _ => "Модель не выбрана".to_string(),
    };

    mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Avatar::new().text(initials).size(40.0).class(tone),
            DecoratedBox::new().class("grow").child(mgui! {
                Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new(title.to_string()).class("chat-header-name"),
                    Text::new(subtitle).class("chat-header-email"),
                ]
            }),
        ]
    }
}

fn empty_header() -> impl Widget {
    mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Avatar::new().text("AI").size(40.0).class("avatar-slate"),
            DecoratedBox::new().class("grow").child(mgui! {
                Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new("Выберите или создайте чат").class("chat-header-name"),
                    Text::new("Нажмите + слева").class("chat-header-email"),
                ]
            }),
        ]
    }
}

fn initials_from_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        return "AI".to_string();
    }
    let mut words = t.split_whitespace();
    let first = words
        .next()
        .and_then(|w| w.chars().next())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let second = words
        .next()
        .and_then(|w| w.chars().next())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    if second.is_empty() {
        t.chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .flat_map(|c| c.to_uppercase())
            .collect()
    } else {
        format!("{first}{second}")
    }
}
