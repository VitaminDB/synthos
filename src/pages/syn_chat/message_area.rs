//! Лента Syn-чата: точечная подложка + реактивный список пузырьков.
//!
//! Visual parity с `components::message_area`, упрощено: без tool-group, без
//! compaction_marker — у Syn-чата только Text-сообщения.

use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;

use crate::chat::time::format_date_today;
use crate::components::date_divider;
use crate::syn_chat::state::{ChatMsg, ChatMsgRole, SynChatCtx};

use super::message_bubble;

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

fn scroll_list() -> impl Widget {
    ScrollView::new().vertical().child(move || {
        let ctx = use_context::<SynChatCtx>();
        let has_active = ctx.active_chat_id.get().is_some();
        let msgs = ctx.messages.get();
        let pending = ctx.pending.get();

        let body: Box<dyn Widget> = if !has_active {
            Box::new(no_chat_hero())
        } else if msgs.is_empty() && !pending {
            Box::new(empty_hero())
        } else {
            Box::new(populated(msgs, pending))
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
                Text::new("Нет активного чата").class("msg-hero-title"),
                Text::new("Создайте чат слева (кнопка +) или выберите существующий")
                    .class("msg-hero-sub"),
            ]
        ]
    })
}

fn empty_hero() -> impl Widget {
    DecoratedBox::new().class("msg-hero").child(mgui! {
        Center::new() => [
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new("Напишите первое сообщение").class("msg-hero-title"),
                Text::new("Например: «Расскажи о себе»").class("msg-hero-sub"),
            ]
        ]
    })
}

fn populated(msgs: Vec<ChatMsg>, pending: bool) -> impl Widget {
    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    items.push(Box::new(date_divider::view(&format_date_today())));

    let last_idx = msgs.len().saturating_sub(1);
    for (idx, msg) in msgs.iter().enumerate() {
        let is_last_assistant = idx == last_idx && msg.role == ChatMsgRole::Assistant;
        let is_typing = pending && is_last_assistant && msg.body.is_empty();
        items.push(message_bubble::view(msg, idx, is_typing, is_last_assistant));
    }

    Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(items)
}
