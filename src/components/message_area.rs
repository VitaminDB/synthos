//! Лента сообщений чата: реактивный список поверх точечной подложки.
//!
//! Источник данных — `AppCtx.chat.messages`. Содержимое переписывается
//! инкрементально во время стриминга токенов; реактивное замыкание внутри
//! `ScrollView` подписано на сигнал и автоматически пересобирает `Column`
//! при любом изменении. Дополнительно слушаем `pending`, чтобы показать
//! «печатает…» плейсхолдер в пустом assistant-сообщении.

use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;

use crate::chat::{time::format_date_today, ChatMsg, ChatMsgKind, ChatMsgRole};
use crate::context::AppCtx;

use super::{compaction_marker, date_divider, message_bubble, tool_group};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("message-area").child(
        Stack::new()
            .fit(StackFit::Expand)
            .child(dot_pattern())
            .child(scroll_list()),
    )
}

fn dot_pattern() -> impl Widget {
    Canvas::new(|ctx, _t| {
        let size = ctx.size();
        // Цвет точек берём из MSS `color` (темой задаётся `--bg-chat-dots`),
        // с запасным значением на случай отсутствия стиля.
        let color = ctx.mss_color().unwrap_or(Color::from_srgb(230, 230, 238, 0.9));
        let step = 16.0_f32;
        let radius = 1.2_f32;
        let mut y = step * 0.5;
        while y < size.height {
            let mut x = step * 0.5;
            while x < size.width {
                ctx.set_color(color);
                ctx.fill_circle(x, y, radius);
                x += step;
            }
            y += step;
        }
    })
    .animated(false)
    .class("dot-pattern")
}

/// Реактивный скролл: `move || { ... }` пересобирает Column при изменении
/// `chat.messages` / `chat.pending` / `general.tool_display_mode`.
/// Пустая лента → hero-блок.
fn scroll_list() -> impl Widget {
    ScrollView::new().vertical().child(move || {
        let app = use_context::<AppCtx>();
        let chat = app.chat.clone();
        let has_active = chat.active_chat_id.get().is_some();
        let msgs = chat.messages.get();
        let pending = chat.pending.get();
        // Подписка: при смене режима отображения tool-карточек
        // лента пересобирается без участия отдельного эффекта.
        let tool_mode = app.general.tool_display_mode.get();

        let body: Box<dyn Widget> = if !has_active {
            Box::new(no_chat_hero())
        } else if msgs.is_empty() {
            Box::new(empty_hero())
        } else {
            Box::new(populated(msgs, pending, &tool_mode))
        };

        DecoratedBox::new()
            .class("message-area-scroll-wrap")
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![body]),
            )
    })
}

/// Блок «активного чата нет» — показываем, пока пользователь не создал
/// или не выбрал чат слева.
fn no_chat_hero() -> impl Widget {
    DecoratedBox::new()
        .class("msg-hero")
        .child(mgui! {
            Center::new() => [
                Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new("Нет активного чата").class("msg-hero-title"),
                    Text::new("Создайте чат слева (кнопка +) или выберите существующий")
                        .class("msg-hero-sub"),
                ]
            ]
        })
}

fn populated(msgs: Vec<ChatMsg>, pending: bool, tool_mode: &str) -> impl Widget {
    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    items.push(Box::new(date_divider::view(&format_date_today())));

    let last_idx = msgs.len().saturating_sub(1);

    // Pre-pass: сообщения с `compacted_iter=Some(N)` принадлежат маркеру
    // итерации N — собираем их в Vec для передачи в `compaction_marker::view`,
    // когда обходчик дойдёт до самого маркера. Свёрнутые сообщения в основной
    // ленте НЕ рендерим (они появятся внутри маркера, если пользователь его
    // развернёт).
    let mut compacted_by_iter: std::collections::HashMap<u32, Vec<(usize, ChatMsg)>> =
        std::collections::HashMap::new();
    for (idx, m) in msgs.iter().enumerate() {
        if let Some(iter) = m.compacted_iter {
            compacted_by_iter
                .entry(iter)
                .or_default()
                .push((idx, m.clone()));
        }
    }

    // В minimal-режиме сжимаем подряд идущие пары `(ToolCall, ToolResult)`
    // одного `tool_name` (≥ 2 пары) в одну сворачиваемую карточку. В full
    // и hidden — поведение прежнее (`build_lane` возвращает плоский список
    // Single-элементов).
    for entry in build_lane(&msgs, tool_mode) {
        match entry {
            LaneEntry::Single(idx) => {
                let msg = &msgs[idx];
                // Свёрнутые сообщения скрываем — они появятся внутри маркера.
                if msg.compacted_iter.is_some() {
                    continue;
                }
                // CompactionMarker — отдельный компонент со своим collapsable.
                if let ChatMsgKind::CompactionMarker { iteration, .. } = msg.kind {
                    let compacted = compacted_by_iter.remove(&iteration).unwrap_or_default();
                    items.push(compaction_marker::view(msg, idx, compacted, tool_mode));
                    continue;
                }
                // is_last_assistant — статический флаг «это последний
                // assistant-text bubble в ленте». Не зависит от pending:
                // нужен и во время стрима (Reactive подписан на
                // streaming_body), и после (кнопка «Продолжить»).
                // Reactive внутри bubble сам читает pending и решает,
                // показывать ли typing-индикатор / стрим-хвост / actions.
                // is_typing_placeholder — служебный флаг для bubble'ов,
                // которые не последние (никогда не должен срабатывать
                // в текущей логике, оставлен для обратной совместимости
                // call-сайтов).
                let is_last_assistant = idx == last_idx
                    && msg.role == ChatMsgRole::Assistant
                    && matches!(msg.kind, ChatMsgKind::Text);
                let is_typing_placeholder = pending
                    && is_last_assistant
                    && msg.body.is_empty();
                items.push(message_bubble::view(
                    msg,
                    idx,
                    is_typing_placeholder,
                    is_last_assistant,
                    tool_mode,
                ));
            }
            LaneEntry::Group(g) => {
                items.push(Box::new(tool_group::view(g)));
            }
        }
    }

    Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(items)
}

// ─────────────────────────────────────────────────────────────────────────────
// Группировка подряд идущих tool-вызовов одного типа (только для minimal-режима)
// ─────────────────────────────────────────────────────────────────────────────

enum LaneEntry {
    Single(usize),
    Group(tool_group::ToolGroup),
}

fn build_lane(msgs: &[ChatMsg], tool_mode: &str) -> Vec<LaneEntry> {
    if tool_mode != "minimal" {
        return (0..msgs.len()).map(LaneEntry::Single).collect();
    }
    let mut out: Vec<LaneEntry> = Vec::with_capacity(msgs.len());
    let mut i = 0usize;
    while i < msgs.len() {
        if let Some((name, _)) = pair_at(msgs, i) {
            // Считаем сколько последовательных пар того же tool_name.
            let mut j = i;
            let mut pairs = 0usize;
            let mut has_error = false;
            while let Some((n, err)) = pair_at(msgs, j) {
                if n != name {
                    break;
                }
                pairs += 1;
                has_error |= err;
                j += 2;
            }
            if pairs >= 2 {
                let items: Vec<(usize, ChatMsg)> =
                    (i..j).map(|k| (k, msgs[k].clone())).collect();
                out.push(LaneEntry::Group(tool_group::ToolGroup {
                    start_idx: i,
                    count: pairs,
                    tool_name: name.to_string(),
                    has_error,
                    items,
                }));
                i = j;
                continue;
            }
        }
        out.push(LaneEntry::Single(i));
        i += 1;
    }
    out
}

/// Возвращает `(tool_name, result_error)`, если `msgs[i]` это `ToolCall(name)`,
/// а `msgs[i+1]` — `ToolResult(_, name, error)` с тем же `tool_name`.
/// Иначе `None` (нет полной пары для группировки).
fn pair_at(msgs: &[ChatMsg], i: usize) -> Option<(&str, bool)> {
    if i + 1 >= msgs.len() {
        return None;
    }
    let call_name = match &msgs[i].kind {
        ChatMsgKind::ToolCall { tool_name } => tool_name.as_str(),
        _ => return None,
    };
    let (result_name, err) = match &msgs[i + 1].kind {
        ChatMsgKind::ToolResult {
            tool_name, error, ..
        } => (tool_name.as_str(), *error),
        _ => return None,
    };
    if call_name != result_name {
        return None;
    }
    Some((call_name, err))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests for build_lane / pair_at — pure logic, поэтому покрываем модулем.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatMsg;

    fn call(name: &str) -> ChatMsg {
        ChatMsg::tool_call(name, "{}", Vec::new())
    }
    fn result(name: &str, error: bool) -> ChatMsg {
        ChatMsg::tool_result("call-id", name, "ok", error)
    }
    fn text() -> ChatMsg {
        ChatMsg::user("hi")
    }

    fn classify(lane: &[LaneEntry]) -> Vec<(usize, Option<(String, usize, bool)>)> {
        lane.iter()
            .map(|e| match e {
                LaneEntry::Single(i) => (*i, None),
                LaneEntry::Group(g) => (g.start_idx, Some((g.tool_name.clone(), g.count, g.has_error))),
            })
            .collect()
    }

    #[test]
    fn non_minimal_mode_returns_flat_singles() {
        let msgs = vec![call("bash"), result("bash", false), call("bash"), result("bash", false)];
        let lane = build_lane(&msgs, "full");
        assert_eq!(classify(&lane), vec![(0, None), (1, None), (2, None), (3, None)]);
    }

    #[test]
    fn single_pair_does_not_group() {
        let msgs = vec![call("bash"), result("bash", false)];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, None), (1, None)]);
    }

    #[test]
    fn two_consecutive_pairs_group() {
        let msgs = vec![call("bash"), result("bash", false), call("bash"), result("bash", false)];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, Some(("bash".to_string(), 2, false)))]);
    }

    #[test]
    fn five_consecutive_pairs_collapse_into_one_group() {
        let msgs: Vec<ChatMsg> = (0..5)
            .flat_map(|_| [call("bash"), result("bash", false)])
            .collect();
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, Some(("bash".to_string(), 5, false)))]);
    }

    #[test]
    fn different_tool_names_break_group() {
        let msgs = vec![
            call("bash"), result("bash", false),
            call("bash"), result("bash", false),
            call("web_read"), result("web_read", false),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![
                (0, Some(("bash".to_string(), 2, false))),
                (4, None),
                (5, None),
            ]
        );
    }

    #[test]
    fn unfinished_call_at_tail_stays_outside_group() {
        let msgs = vec![
            call("bash"), result("bash", false),
            call("bash"), result("bash", false),
            call("bash"), // нет ToolResult — стрим в процессе.
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![
                (0, Some(("bash".to_string(), 2, false))),
                (4, None),
            ]
        );
    }

    #[test]
    fn has_error_propagates_when_any_result_is_error() {
        let msgs = vec![
            call("bash"), result("bash", false),
            call("bash"), result("bash", true),
            call("bash"), result("bash", false),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, Some(("bash".to_string(), 3, true)))]);
    }

    #[test]
    fn text_messages_between_groups_are_singles() {
        let msgs = vec![
            text(),
            call("bash"), result("bash", false),
            call("bash"), result("bash", false),
            text(),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![
                (0, None),
                (1, Some(("bash".to_string(), 2, false))),
                (5, None),
            ]
        );
    }

    #[test]
    fn pair_at_rejects_mismatched_names() {
        let msgs = vec![call("bash"), result("web_read", false)];
        assert_eq!(pair_at(&msgs, 0), None);
    }

    #[test]
    fn pair_at_rejects_text_followed_by_text() {
        let msgs = vec![text(), text()];
        assert_eq!(pair_at(&msgs, 0), None);
    }

    #[test]
    fn pair_at_returns_name_and_error_flag() {
        let msgs = vec![call("kb_search"), result("kb_search", true)];
        assert_eq!(pair_at(&msgs, 0), Some(("kb_search", true)));
    }
}

/// Hero-блок, когда в ленте ещё нет сообщений. Сохраняем стиль чата
/// (точечный фон + мягкие цвета), добавляем через `.msg-hero*`.
fn empty_hero() -> impl Widget {
    DecoratedBox::new()
        .class("msg-hero")
        .child(mgui! {
            Center::new() => [
                Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new("Начните разговор с llama").class("msg-hero-title"),
                    Text::new("Введите сообщение ниже — ответ приходит потоково")
                        .class("msg-hero-sub"),
                ]
            ]
        })
}
