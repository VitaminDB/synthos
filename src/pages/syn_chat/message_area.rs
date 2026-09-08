//! Лента Syn-чата: точечная подложка + реактивный список пузырьков.
//!
//! Кроме текстовых сообщений лента рендерит работу агента — карточки
//! tool-call / tool-result (`super::message_bubble`), в `minimal`-режиме
//! свёрнутые группы одинаковых вызовов (`super::tool_group`) и маркеры
//! компактификации (`super::compaction_marker`): сжатые autocompact'ом
//! сообщения в основную ленту не идут — они прячутся под свой маркер.

use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;

use crate::agent::time::format_date_today;
use crate::components::date_divider;
use crate::context::AppCtx;
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, ChatMsgRole, SynChatCtx};

use super::{compaction_marker, message_bubble, tool_group};

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

/// Лента — `ScrollView` в режиме `follow_end`: при показе прокручена к
/// последнему сообщению, во время стрима держится внизу, а стоит
/// пользователю пролистать вверх — новые токены её не дёргают, пока он не
/// вернётся к низу сам. Смена чата пересоздаёт `ScrollView` (внешний
/// `Reactive` по `active_chat_id`): свежий элемент снова открывается внизу,
/// а не там, где остановились в предыдущем чате.
fn scroll_list() -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = use_context::<SynChatCtx>().active_chat_id.get();
        vec![Box::new(scroll_list_for_chat())]
    })
}

fn scroll_list_for_chat() -> impl Widget {
    ScrollView::new().vertical().follow_end(true).child(move || {
        let ctx = use_context::<SynChatCtx>();
        let has_active = ctx.active_chat_id.get().is_some();
        let msgs = ctx.messages.get();
        let pending = ctx.pending.get();
        // Подписка: при смене режима отображения tool-карточек лента
        // пересобирается без отдельного эффекта.
        let tool_mode = use_context::<AppCtx>().general.tool_display_mode.get();

        // Индекс сообщения, на которое привёл глобальный поиск: пузырёк
        // обводится рамкой, пока пользователь не сменит чат.
        let highlight = ctx.highlight_msg.get();

        let body: Box<dyn Widget> = if !has_active {
            Box::new(no_chat_hero())
        } else if msgs.is_empty() && !pending {
            Box::new(empty_hero())
        } else {
            Box::new(populated(msgs, pending, &tool_mode, highlight))
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

fn no_chat_hero() -> impl Widget {
    DecoratedBox::new().class("msg-hero").child(mgui! {
        Center::new() => [
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(tr!("chat.area.no_chat.title")).class("msg-hero-title"),
                Text::new(tr!("chat.area.no_chat.hint"))
                    .class("msg-hero-sub"),
            ]
        ]
    })
}

fn empty_hero() -> impl Widget {
    DecoratedBox::new().class("msg-hero").child(mgui! {
        Center::new() => [
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(tr!("chat.area.empty.title")).class("msg-hero-title"),
                Text::new(tr!("chat.area.empty.hint")).class("msg-hero-sub"),
            ]
        ]
    })
}

fn populated(
    msgs: Vec<ChatMsg>,
    pending: bool,
    tool_mode: &str,
    highlight: Option<usize>,
) -> impl Widget {
    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    items.push(Box::new(date_divider::view(&format_date_today())));

    // Свёрнутые autocompact'ом сообщения из основной ленты исключаются (они
    // рендерятся внутри своего маркера). Лента и группировка строятся по
    // «видимым» сообщениям, а исходные индексы сохраняются для ключей
    // open-state в bubble'ах.
    let mut visible: Vec<ChatMsg> = Vec::with_capacity(msgs.len());
    let mut orig_idx: Vec<usize> = Vec::with_capacity(msgs.len());
    for (i, m) in msgs.iter().enumerate() {
        if m.compacted_iter.is_none() {
            visible.push(m.clone());
            orig_idx.push(i);
        }
    }

    let last_vis = visible.len().saturating_sub(1);
    // В minimal-режиме подряд идущие пары `(ToolCall, ToolResult)` одного
    // инструмента (≥ 2 пар) схлопываются в одну сворачиваемую карточку.
    for entry in build_lane(&visible, tool_mode) {
        match entry {
            LaneEntry::Single(vi) => {
                let idx = orig_idx[vi];
                let msg = &msgs[idx];
                if let ChatMsgKind::CompactionMarker { iteration, .. } = &msg.kind {
                    let compacted: Vec<(usize, ChatMsg)> = msgs
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.compacted_iter == Some(*iteration))
                        .map(|(i, m)| (i, m.clone()))
                        .collect();
                    items.push(compaction_marker::view(msg, compacted, tool_mode));
                    continue;
                }
                // Стрим-хвост и regen привязаны только к текстовому bubble'у
                // ассистента: tool-карточки собственного стрима не имеют.
                let is_last_assistant = vi == last_vis
                    && msg.role == ChatMsgRole::Assistant
                    && matches!(msg.kind, ChatMsgKind::Text);
                let is_typing = pending
                    && vi == last_vis
                    && msg.role == ChatMsgRole::Assistant
                    && msg.body.is_empty();
                let bubble = message_bubble::view(
                    msg,
                    idx,
                    is_typing,
                    is_last_assistant,
                    tool_mode,
                );
                items.push(if highlight == Some(idx) {
                    Box::new(
                        DecoratedBox::new().class("msg-search-highlight").child(
                            Column::new()
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .children(vec![bubble]),
                        ),
                    )
                } else {
                    bubble
                });
            }
            LaneEntry::Group(mut g) => {
                // Индексы группы — в «видимом» пространстве; для ключей
                // open-state возвращаем исходные.
                g.start_idx = orig_idx[g.start_idx];
                g.items = g
                    .items
                    .into_iter()
                    .map(|(vi, m)| (orig_idx[vi], m))
                    .collect();
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
// Группировка подряд идущих tool-вызовов одного типа (только minimal-режим)
// ─────────────────────────────────────────────────────────────────────────────

pub(super) enum LaneEntry {
    Single(usize),
    Group(tool_group::ToolGroup),
}

pub(super) fn build_lane(msgs: &[ChatMsg], tool_mode: &str) -> Vec<LaneEntry> {
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
            let mut errors = 0usize;
            while let Some((n, err)) = pair_at(msgs, j) {
                if n != name {
                    break;
                }
                pairs += 1;
                errors += usize::from(err);
                j += 2;
            }
            if pairs >= 2 {
                let items: Vec<(usize, ChatMsg)> =
                    (i..j).map(|k| (k, msgs[k].clone())).collect();
                out.push(LaneEntry::Group(tool_group::ToolGroup {
                    start_idx: i,
                    count: pairs,
                    tool_name: name.to_string(),
                    err_count: errors,
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

/// Возвращает `(tool_name, result_error)`, если `msgs[i]` — `ToolCall(name)`,
/// а `msgs[i+1]` — `ToolResult(_, name, error)` того же инструмента.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> ChatMsg {
        ChatMsg::tool_call(name, "{}", Vec::new())
    }
    fn result(name: &str, error: bool) -> ChatMsg {
        ChatMsg::tool_result("call-id", name, "ok", error)
    }
    fn text() -> ChatMsg {
        ChatMsg::user("hi")
    }

    /// `(индекс, Some((инструмент, вызовов, из них с ошибкой)))`.
    fn classify(lane: &[LaneEntry]) -> Vec<(usize, Option<(String, usize, usize)>)> {
        lane.iter()
            .map(|e| match e {
                LaneEntry::Single(i) => (*i, None),
                LaneEntry::Group(g) => {
                    (g.start_idx, Some((g.tool_name.clone(), g.count, g.err_count)))
                }
            })
            .collect()
    }

    #[test]
    fn non_minimal_mode_returns_flat_singles() {
        let msgs = vec![call("web"), result("web", false), call("web"), result("web", false)];
        let lane = build_lane(&msgs, "full");
        assert_eq!(classify(&lane), vec![(0, None), (1, None), (2, None), (3, None)]);
    }

    #[test]
    fn single_pair_does_not_group() {
        let msgs = vec![call("web"), result("web", false)];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, None), (1, None)]);
    }

    #[test]
    fn two_consecutive_pairs_group() {
        let msgs = vec![call("web"), result("web", false), call("web"), result("web", false)];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, Some(("web".to_string(), 2, 0)))]);
    }

    #[test]
    fn different_tool_names_break_group() {
        let msgs = vec![
            call("web"), result("web", false),
            call("web"), result("web", false),
            call("bash"), result("bash", false),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![
                (0, Some(("web".to_string(), 2, 0))),
                (4, None),
                (5, None),
            ]
        );
    }

    #[test]
    fn unfinished_call_at_tail_stays_outside_group() {
        let msgs = vec![
            call("web"), result("web", false),
            call("web"), result("web", false),
            call("web"),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![(0, Some(("web".to_string(), 2, 0))), (4, None)]
        );
    }

    #[test]
    fn err_count_counts_only_failed_results() {
        let msgs = vec![
            call("web"), result("web", false),
            call("web"), result("web", true),
            call("web"), result("web", false),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(classify(&lane), vec![(0, Some(("web".to_string(), 3, 1)))]);
    }

    #[test]
    fn err_count_zero_when_all_succeed_and_full_when_all_fail() {
        let ok = vec![
            call("web"), result("web", false),
            call("web"), result("web", false),
        ];
        assert_eq!(classify(&build_lane(&ok, "minimal")), vec![(0, Some(("web".to_string(), 2, 0)))]);
        let bad = vec![
            call("web"), result("web", true),
            call("web"), result("web", true),
        ];
        assert_eq!(classify(&build_lane(&bad, "minimal")), vec![(0, Some(("web".to_string(), 2, 2)))]);
    }

    #[test]
    fn text_messages_between_groups_are_singles() {
        let msgs = vec![
            text(),
            call("web"), result("web", false),
            call("web"), result("web", false),
            text(),
        ];
        let lane = build_lane(&msgs, "minimal");
        assert_eq!(
            classify(&lane),
            vec![(0, None), (1, Some(("web".to_string(), 2, 0))), (5, None)]
        );
    }

    #[test]
    fn pair_at_rejects_mismatched_names() {
        let msgs = vec![call("web"), result("bash", false)];
        assert_eq!(pair_at(&msgs, 0), None);
    }

    #[test]
    fn pair_at_returns_name_and_error_flag() {
        let msgs = vec![call("kb_search"), result("kb_search", true)];
        assert_eq!(pair_at(&msgs, 0), Some(("kb_search", true)));
    }
}
