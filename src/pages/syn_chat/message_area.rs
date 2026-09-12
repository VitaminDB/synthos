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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use syngui::widgets::containers::Keyed;

use crate::agent::time::format_date_today;
use crate::components::date_divider;
use crate::context::AppCtx;
use crate::syn_chat::state::{ChatMsg, QueuedMsg, SynChatCtx};

use super::lane::{self, LaneKind, LaneRow, LaneUi};
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
        let app = use_context::<AppCtx>();
        let has_active = ctx.active_chat_id.get().is_some();
        let msgs = Arc::new(ctx.messages.get());
        let pending = ctx.pending.get();
        // Подписки: смена режима показа tool-карточек и темы меняет вид
        // строк, поэтому они входят и в версию строки.
        let tool_mode = app.general.tool_display_mode.get();
        let theme_key = hash_of(&app.theme_key.get());

        // Индекс сообщения, на которое привёл глобальный поиск: пузырёк
        // обводится рамкой, пока пользователь не сменит чат.
        let highlight = ctx.highlight_msg.get();
        let editing = ctx.editing_msg.get();
        // Очередь отправки активного чата — пузырьки в хвосте ленты.
        let active = ctx.active_chat_id.get_untracked();
        let queued: Arc<Vec<QueuedMsg>> = Arc::new(
            ctx.queue
                .get()
                .into_iter()
                .filter(|m| Some(&m.chat_id) == active.as_ref())
                .collect(),
        );
        let queue_editing = ctx.queue_editing.get();

        let children: Vec<Box<dyn Widget>> = if !has_active {
            vec![Box::new(no_chat_hero())]
        } else if msgs.is_empty() && !pending && queued.is_empty() {
            vec![Box::new(empty_hero())]
        } else {
            // Состояние раскрытия входит в версию строки: клик по шеврону
            // пересобирает свою строку, а не всю ленту.
            let thinking_open = ctx.thinking_open.get();
            let tool_body_open = ctx.tool_body_open.get();
            let tool_group_open = ctx.tool_group_open.get();
            let compaction_open = ctx.compaction_open.get();
            let wizard_drafts = ctx.wizard_drafts.get();
            let ui = LaneUi {
                tool_mode: &tool_mode,
                pending,
                highlight,
                editing,
                theme_key,
                thinking_open: &thinking_open,
                tool_body_open: &tool_body_open,
                tool_group_open: &tool_group_open,
                compaction_open: &compaction_open,
                wizard_drafts: &wizard_drafts,
                queue_editing,
            };
            lane::build(&msgs, &queued, &ui)
                .into_iter()
                .map(|row| row_widget(row, &msgs, &queued, &tool_mode))
                .collect()
        };

        DecoratedBox::new().class("message-area-scroll-wrap").child(
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children),
        )
    })
}

fn hash_of(value: &str) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Строка ленты как виджет. Всё, от чего зависит её вид, уже посчитано в
/// версии ([`lane::build`]), поэтому `Keyed` при той же версии не вызывает
/// этот сборщик вовсе: пузырёк не пересобирается и не перемеряется.
fn row_widget(
    row: LaneRow,
    msgs: &Arc<Vec<ChatMsg>>,
    queued: &Arc<Vec<QueuedMsg>>,
    tool_mode: &str,
) -> Box<dyn Widget> {
    let LaneRow { key, version, kind } = row;
    let msgs = msgs.clone();
    let queued = queued.clone();
    let tool_mode = tool_mode.to_string();

    match kind {
        LaneKind::DateDivider => Box::new(Keyed::new(key, version, move || {
            Box::new(date_divider::view(&format_date_today()))
        })),

        LaneKind::Message(m) => Box::new(Keyed::new(key, version, move || {
            let msg = &msgs[m.idx];
            let bubble = message_bubble::view(msg, m.idx, m.is_typing, m.is_last_assistant, &tool_mode);
            // Обёртка стоит всегда, меняется только класс: иначе при
            // подсветке найденного сообщения менялся бы тип виджета строки
            // и её элемент пересоздавался бы вместе с состоянием.
            // Класс ставится всегда: `.class()` меняет тип виджета, а
            // строка должна оставаться того же типа, иначе её элемент
            // пересоздастся вместе с состоянием.
            let wrap = DecoratedBox::new().class(if m.highlighted {
                "msg-search-highlight"
            } else {
                "msg-row"
            });
            Box::new(
                wrap.child(
                    Column::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(vec![bubble]),
                ),
            )
        })),

        LaneKind::Group(g) => Box::new(Keyed::new(key, version, move || {
            Box::new(tool_group::view(tool_group::ToolGroup {
                start_idx: g.start_idx,
                count: g.count,
                tool_name: g.tool_name.clone(),
                err_count: g.err_count,
                items: g.items.iter().map(|&i| (i, msgs[i].clone())).collect(),
            }))
        })),

        LaneKind::Marker(m) => Box::new(Keyed::new(key, version, move || {
            let compacted: Vec<(usize, ChatMsg)> =
                m.compacted.iter().map(|&i| (i, msgs[i].clone())).collect();
            compaction_marker::view(&msgs[m.idx], compacted, &tool_mode)
        })),

        LaneKind::Queued(q) => Box::new(Keyed::new(key, version, move || {
            match queued.iter().find(|item| item.id == q.id) {
                Some(item) => Box::new(message_bubble::queued_view(item)) as Box<dyn Widget>,
                // Сообщение ушло из очереди между сборкой модели и строки.
                None => Box::new(Column::new()),
            }
        })),
    }
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


