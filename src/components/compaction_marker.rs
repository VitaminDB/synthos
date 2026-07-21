//! Раскрываемый маркер autocompact-итерации в ленте чата.
//!
//! Рендерится в `message_area::populated` для сообщений с `kind=CompactionMarker`.
//! Свёрнутая плашка показывает счётчик сообщений и токены before→after; по
//! клику разворачивается, обнажая сам summary (`MarkdownView`) и список
//! свёрнутых под этим маркером сообщений (через стандартный
//! `message_bubble::view` с дополнительным MSS-классом `.compacted`).
//!
//! Состояние развёрнутости — в `chat.compaction_open` (key = `iteration`,
//! стабильный u32). Эфемерное, не persist'ится. Дефолт — закрыт.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::MarkdownView;

use crate::chat::{ChatMsg, ChatMsgKind, ChatMsgRole};
use crate::context::AppCtx;
use crate::icons::{MI_COMPRESS, MI_EXPAND_LESS, MI_EXPAND_MORE};

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
/// `marker_idx` — его индекс в `chat.messages` (нужен для рендера свёрнутых
/// сообщений с правильными msg_idx). `compacted` — диапазон свёрнутых под
/// этим маркером сообщений (вместе с их оригинальными индексами).
pub fn view(
    marker_msg: &ChatMsg,
    marker_idx: usize,
    compacted: Vec<(usize, ChatMsg)>,
    tool_mode: &str,
) -> Box<dyn Widget> {
    let (iteration, compacted_count, tokens_before, tokens_after, summary) = match &marker_msg.kind {
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
        // Не должно случаться: вызываемая сторона уже отфильтровала kind.
        _ => return Box::new(DecoratedBox::new()),
    };

    let time = marker_msg.time.clone();

    let header = build_header(
        iteration,
        compacted_count,
        tokens_before,
        tokens_after,
        time,
    );

    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let chat = use_context::<AppCtx>().chat.clone();
            chat.compaction_open.update(|m| {
                let cur = m.get(&iteration).copied().unwrap_or(false);
                m.insert(iteration, !cur);
            });
        })
        .child(header);

    // Реактивное тело: при `open=false` — пустой контейнер, при `open=true` —
    // summary + свёрнутые сообщения. Закрытое состояние zero-size, не
    // занимает места в layout.
    let summary_for_body = summary.clone();
    let compacted_for_body = compacted.clone();
    let tool_mode_owned = tool_mode.to_string();
    let body_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
            .compaction_open
            .get()
            .get(&iteration)
            .copied()
            .unwrap_or(false);
        let body: Box<dyn Widget> = if open {
            let mut children: Vec<Box<dyn Widget>> = Vec::new();
            children.push(Box::new(
                MarkdownView::new(summary_for_body.clone()).class("compaction-summary"),
            ));
            // Свёрнутые сообщения — через стандартный bubble-рендер, с
            // добавочным wrap'ером `.compacted` для приглушённого стиля.
            // Column::new() с одним child = bubble + class на Column =
            // wrapper-стиль без надобности в DecoratedBox::child<W: Widget>.
            for (real_idx, msg) in &compacted_for_body {
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

    // marker_idx сейчас не используется в layout, но передаётся снаружи —
    // оставлен для будущих интеграций (например, контекстное меню «удалить
    // маркер и развернуть все сообщения обратно»).
    let _ = marker_idx;

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
        format!(
            "{} → {} токенов",
            fmt_thousands(tokens_before),
            fmt_thousands(tokens_after)
        )
    } else {
        // Manual-триггер без снимка usage — показываем только размер summary.
        format!("≈{} токенов", fmt_thousands(tokens_after))
    };
    let primary = format!(
        "Компактификация #{iteration} \u{2022} сжато {compacted_count} \u{2022} {tokens_label}"
    );
    let secondary = if time.is_empty() {
        String::new()
    } else {
        time
    };

    let chevron_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
            .compaction_open
            .get()
            .get(&iteration)
            .copied()
            .unwrap_or(false);
        let icon = if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE };
        Icon::new(icon).class("compaction-chevron")
    };

    // Время может быть пустым (старый JSON) — тогда плашку не добавляем.
    // Удобнее всего через две отдельные ветки: с `time` и без.
    if secondary.is_empty() {
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
                Text::new(secondary).class("compaction-time"),
                DecoratedBox::new().class("grow"),
                chevron_reactive,
            ]
        }
    }
}

/// Игнорируется — добавлено только для использования role в импорте, но
/// сейчас в коде не нужно. Оставлено как safety заглушка для линтера.
#[allow(dead_code)]
fn _unused(_role: ChatMsgRole) {}
