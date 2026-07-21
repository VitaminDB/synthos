//! Рендер одного сообщения в Syn-ленте: упрощённая копия
//! `components::message_bubble`. Только Text-варианты, без tool-calls
//! и attachments.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::MarkdownView;

use crate::icons::{
    MI_AUTORENEW, MI_CONTENT_COPY, MI_EXPAND_LESS, MI_EXPAND_MORE, MI_PSYCHOLOGY,
};
use crate::syn_chat::session;
use crate::syn_chat::state::{ChatMsg, ChatMsgRole, SynChatCtx};

pub fn view(msg: &ChatMsg, msg_idx: usize, is_typing: bool, is_last_assistant: bool) -> Box<dyn Widget> {
    match msg.role {
        ChatMsgRole::System => Box::new(system_line(&msg.body, msg.error)),
        ChatMsgRole::User => Box::new(chat_row(msg, msg_idx, true, false, false)),
        ChatMsgRole::Assistant => {
            Box::new(chat_row(msg, msg_idx, false, is_typing, is_last_assistant))
        }
    }
}

fn system_line(body: &str, error: bool) -> impl Widget {
    let class = if error {
        "msg-system msg-system-error"
    } else {
        "msg-system"
    };
    mgui! {
        Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center) => [
            Text::new(body.to_string()).class(class),
        ]
    }
}

fn chat_row(
    msg: &ChatMsg,
    msg_idx: usize,
    outgoing: bool,
    is_typing: bool,
    is_last_assistant: bool,
) -> impl Widget {
    let bubble_class = if outgoing {
        "msg-bubble msg-bubble-out"
    } else {
        "msg-bubble msg-bubble-in"
    };
    let author_class = if outgoing {
        "msg-author msg-author-right"
    } else {
        "msg-author"
    };

    let time = msg.time.clone();
    let author = msg.author.clone();
    let body = msg.body.clone();

    let avatar = Avatar::new()
        .text(msg.initials.clone())
        .size(32.0)
        .class(msg.tone_class.clone());

    let bubble_child: Box<dyn Widget> = if outgoing {
        Box::new(bubble_markdown(&body, "msg-bubble-md msg-bubble-out-md"))
    } else if is_last_assistant {
        let initial_body = body.clone();
        Box::new(syngui::widgets::containers::reactive::Reactive::new(
            move || -> Vec<Box<dyn Widget>> {
                let ctx = use_context::<SynChatCtx>();
                let pending = ctx.pending.get();
                let tail = if pending {
                    ctx.streaming_body.get()
                } else {
                    String::new()
                };
                if pending && initial_body.is_empty() && tail.is_empty() {
                    return vec![Box::new(Text::new("•••").class("msg-typing"))];
                }
                let merged = if tail.is_empty() {
                    initial_body.clone()
                } else if initial_body.is_empty() {
                    tail
                } else {
                    format!("{initial_body}{tail}")
                };
                vec![Box::new(bubble_markdown(&merged, "msg-bubble-md"))]
            },
        ))
    } else if is_typing {
        Box::new(Text::new("•••").class("msg-typing"))
    } else {
        Box::new(bubble_markdown(&msg.body, "msg-bubble-md"))
    };

    let mut bubble_children: Vec<Box<dyn Widget>> = Vec::new();
    if !outgoing {
        let default_open = msg.body.is_empty();
        let initial_thinking = msg.thinking.clone();
        if is_last_assistant {
            bubble_children.push(Box::new(streaming_thinking_block(
                msg_idx,
                initial_thinking,
                default_open,
            )));
        } else if !initial_thinking.is_empty() {
            bubble_children.push(Box::new(thinking_block(
                msg_idx,
                initial_thinking,
                default_open,
            )));
        }
    }
    bubble_children.push(bubble_child);

    let bubble = DecoratedBox::new().class(bubble_class).child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(bubble_children),
    );

    let show_actions = !outgoing && (!is_typing || is_last_assistant);
    let bubble_row: Box<dyn Widget> = if show_actions {
        Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .main_axis_alignment(MainAxisAlignment::Start)
                .children(vec![
                    Box::new(bubble) as Box<dyn Widget>,
                    actions_widget(body.clone(), is_last_assistant),
                ]),
        )
    } else {
        Box::new(bubble)
    };

    let header_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(author).class(author_class),
            Text::new(time).class("msg-time"),
        ]
    };

    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(if outgoing { CrossAxisAlignment::End } else { CrossAxisAlignment::Start })
        .children(vec![Box::new(header_row) as Box<dyn Widget>, bubble_row]);

    if outgoing {
        mgui! {
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::End).main_axis_alignment(MainAxisAlignment::End) => [
                meta,
                avatar,
            ]
        }
    } else {
        mgui! {
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::End).main_axis_alignment(MainAxisAlignment::Start) => [
                avatar,
                meta,
            ]
        }
    }
}

fn actions_widget(body: String, regen_allowed: bool) -> Box<dyn Widget> {
    syngui::widgets::containers::reactive::IntoWidget::into_widget(actions_row(body, regen_allowed))
}

fn actions_row(body: String, regen_allowed: bool) -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let pending = use_context::<SynChatCtx>().pending.get();
        if pending {
            return DecoratedBox::new().class("msg-actions-empty");
        }
        let body_for_copy = body.clone();
        let copy = ToolButton::new(MI_CONTENT_COPY)
            .tooltip("Скопировать сообщение")
            .on_click(move || {
                let plain = syngui::widgets::visual::markdown_view::linearize_markdown_source(
                    &body_for_copy,
                );
                syngui::clipboard::copy(&plain);
            })
            .class("msg-action-copy");

        let regen: Option<_> = if regen_allowed {
            Some(
                ToolButton::new(MI_AUTORENEW)
                    .tooltip("Сгенерировать заново")
                    .on_click(session::regenerate_last)
                    .class("msg-action-regen"),
            )
        } else {
            None
        };

        let mut buttons: Vec<Box<dyn Widget>> = Vec::new();
        buttons.push(Box::new(copy));
        if let Some(r) = regen {
            buttons.push(Box::new(r));
        }
        DecoratedBox::new().class("msg-actions").child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(buttons),
        )
    }
}

fn bubble_markdown(body: &str, class: &'static str) -> impl Widget {
    let normalized = normalize_assistant_markdown(body);
    MarkdownView::new(normalized)
        .with_syntax_theme("InspiredGitHub")
        .with_copy_code(true)
        .class(class)
}

fn normalize_assistant_markdown(body: &str) -> String {
    // Заменить типографские bullet'ы (• ) на CommonMark `- `.
    let mut out = String::with_capacity(body.len());
    let mut at_line_start = true;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if at_line_start && c == '\u{2022}' && chars.peek() == Some(&' ') {
            out.push_str("- ");
            chars.next();
            at_line_start = false;
            continue;
        }
        out.push(c);
        at_line_start = c == '\n';
    }
    out
}

fn thinking_block(msg_idx: usize, thinking: String, default_open: bool) -> impl Widget {
    let chevron_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .thinking_open
            .get()
            .get(&msg_idx)
            .copied()
            .unwrap_or(default_open);
        let icon = if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE };
        Icon::new(icon).class("msg-thinking-chevron")
    };
    let header = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_PSYCHOLOGY).class("msg-thinking-icon"),
            Text::new("Размышления").class("msg-thinking-title"),
            DecoratedBox::new().class("grow"),
            chevron_reactive,
        ]
    };
    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.thinking_open.update(|m| {
                let cur = m.get(&msg_idx).copied().unwrap_or(default_open);
                m.insert(msg_idx, !cur);
            });
        })
        .child(header);

    let body_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .thinking_open
            .get()
            .get(&msg_idx)
            .copied()
            .unwrap_or(default_open);
        let body: Box<dyn Widget> = if open {
            Box::new(MarkdownView::new(thinking.clone()).class("msg-thinking-body"))
        } else {
            Box::new(DecoratedBox::new())
        };
        DecoratedBox::new().child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![body]),
        )
    };

    DecoratedBox::new().class("msg-thinking").child(mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            header_clickable,
            body_reactive,
        ]
    })
}

fn streaming_thinking_block(msg_idx: usize, initial_thinking: String, default_open: bool) -> impl Widget {
    let chevron_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .thinking_open
            .get()
            .get(&msg_idx)
            .copied()
            .unwrap_or(default_open);
        let icon = if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE };
        Icon::new(icon).class("msg-thinking-chevron")
    };
    let header = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_PSYCHOLOGY).class("msg-thinking-icon"),
            Text::new("Размышления").class("msg-thinking-title"),
            DecoratedBox::new().class("grow"),
            chevron_reactive,
        ]
    };
    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.thinking_open.update(|m| {
                let cur = m.get(&msg_idx).copied().unwrap_or(default_open);
                m.insert(msg_idx, !cur);
            });
        })
        .child(header);

    let initial_for_body = initial_thinking.clone();
    let body_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let tail = ctx.streaming_thinking.get();
        let open = ctx
            .thinking_open
            .get()
            .get(&msg_idx)
            .copied()
            .unwrap_or(default_open);
        let merged = if tail.is_empty() {
            initial_for_body.clone()
        } else if initial_for_body.is_empty() {
            tail
        } else {
            format!("{initial_for_body}{tail}")
        };
        let body: Box<dyn Widget> = if !open || merged.is_empty() {
            Box::new(DecoratedBox::new())
        } else {
            Box::new(MarkdownView::new(merged).class("msg-thinking-body"))
        };
        DecoratedBox::new().child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![body]),
        )
    };

    let _ = initial_thinking;

    DecoratedBox::new().class("msg-thinking").child(mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            header_clickable,
            body_reactive,
        ]
    })
}
