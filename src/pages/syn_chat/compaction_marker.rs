//! Раскрываемый маркер autocompact-итерации в ленте Syn-чата.
//!
//! Рендерится в `message_area::populated` для сообщений с
//! `kind=CompactionMarker`. Свёрнутая плашка показывает счётчик сообщений и
//! токены before→after; по клику разворачивается, обнажая сам summary
//! (`MarkdownView`) и список свёрнутых под этим маркером сообщений (через
//! стандартный `message_bubble::view` с wrap-классом `.compaction-collapsed-item`).
//!
//! Состояние развёрнутости — в `SynChatCtx.compaction_open` (key =
//! `iteration`, стабильный u32). Эфемерное, не persist'ится. Дефолт — закрыт.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::MarkdownView;

use crate::icons::{MI_COMPRESS, MI_EXPAND_LESS, MI_EXPAND_MORE};
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, SynChatCtx};

use super::message_bubble;

/// Человекочитаемое число с пробелом-разделителем тысяч (`12 480`).
fn fmt_thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push('\u{202F}'); // narrow no-break space
        }
        out.push(*b as char);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Главный view маркера. `marker_msg` — это `ChatMsg::compaction_marker(...)`,
/// `compacted` — свёрнутые под этим маркером сообщения вместе с их
/// оригинальными индексами в `ctx.messages` (для ключей open-state).
pub fn view(
    marker_msg: &ChatMsg,
    compacted: Vec<(usize, ChatMsg)>,
    tool_mode: &str,
) -> Box<dyn Widget> {
    let (iteration, compacted_count, tokens_before, tokens_after, summary) = match &marker_msg.kind
    {
        ChatMsgKind::CompactionMarker {
            iteration,
            compacted_count,
            tokens_before,
            tokens_after,
            summary,
        } => (
            *iteration,
            *compacted_count,
            *tokens_before,
            *tokens_after,
            summary.clone(),
        ),
        // Не должно случаться: вызывающая сторона уже отфильтровала kind.
        _ => return Box::new(DecoratedBox::new()),
    };

    let header = build_header(
        iteration,
        compacted_count,
        tokens_before,
        tokens_after,
        marker_msg.time.clone(),
    );

    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.compaction_open.update(|m| {
                let cur = m.get(&iteration).copied().unwrap_or(false);
                m.insert(iteration, !cur);
            });
        })
        .child(header);

    // Реактивное тело: при `open=false` — пустой контейнер (zero-size, не
    // занимает места в layout), при `open=true` — summary + свёрнутые
    // сообщения.
    let tool_mode_owned = tool_mode.to_string();
    let body_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .compaction_open
            .get()
            .get(&iteration)
            .copied()
            .unwrap_or(false);
        let body: Box<dyn Widget> = if open {
            let mut children: Vec<Box<dyn Widget>> = Vec::new();
            children.push(Box::new(
                MarkdownView::new(summary.clone()).class("compaction-summary"),
            ));
            // Свёрнутые сообщения — через стандартный bubble-рендер, с
            // wrap'ером для приглушённого стиля.
            for (real_idx, msg) in &compacted {
                let bubble = message_bubble::view(msg, *real_idx, false, false, &tool_mode_owned);
                let wrap = Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![bubble])
                    .class("compaction-collapsed-item");
                children.push(Box::new(wrap));
            }
            Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children)
                    .class("compaction-body"),
            )
        } else {
            Box::new(DecoratedBox::new())
        };
        DecoratedBox::new().child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![body]),
        )
    };

    Box::new(
        DecoratedBox::new().class("compaction-marker").child(mgui! {
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header_clickable,
                body_reactive,
            ]
        }),
    )
}

fn build_header(
    iteration: u32,
    compacted_count: usize,
    tokens_before: i64,
    tokens_after: i64,
    time: String,
) -> impl Widget {
    let tokens_label = if tokens_before > 0 {
        tr!(
            "chat.compaction.tokens_range",
            before = fmt_thousands(tokens_before),
            after = fmt_thousands(tokens_after)
        )
    } else {
        // Manual-триггер без снимка usage — показываем только размер summary.
        tr!("chat.compaction.tokens_approx", after = fmt_thousands(tokens_after))
    };
    let primary = tr!(
        "chat.compaction.title",
        iteration = iteration,
        compacted_count = compacted_count,
        tokens_label = tokens_label
    );

    let chevron_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .compaction_open
            .get()
            .get(&iteration)
            .copied()
            .unwrap_or(false);
        let icon = if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE };
        Icon::new(icon).class("compaction-chevron")
    };

    // Время может быть пустым (старый JSON) — тогда плашку не добавляем.
    if time.is_empty() {
        mgui! {
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_COMPRESS).class("compaction-icon"),
                Text::new(primary).class("compaction-title"),
                DecoratedBox::new().class("grow"),
                chevron_reactive,
            ]
        }
    } else {
        mgui! {
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_COMPRESS).class("compaction-icon"),
                Text::new(primary).class("compaction-title"),
                Text::new(time).class("compaction-time"),
                DecoratedBox::new().class("grow"),
                chevron_reactive,
            ]
        }
    }
}
