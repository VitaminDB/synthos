//! Сворачиваемая группа подряд идущих tool-вызовов одного типа.
//!
//! В режиме `tool_display_mode == "minimal"` цепочки вида
//! `ToolCall(web) + ToolResult(web) + ToolCall(web) + ToolResult(web) + …`
//! (≥ 2 одинаковых пары) сжимаются в одну карточку «Веб ×N» с chevron'ом.
//! Обнаружение групп — в [`super::message_area::build_lane`].
//!
//! Состояние раскрытия живёт в `SynChatCtx.tool_group_open` — ключ —
//! индекс первого `ChatMsg` группы в текущей ленте, дефолт закрыто.
//! Эфемерное, не persist'ится и сбрасывается при переключении чата.

use syngui::animation::Easing;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{AnimatedSize, AnimationAxis, Reactive};

use crate::agent::tools::Tool;
use crate::icons::{MI_EXPAND_LESS, MI_EXPAND_MORE, MI_REPORT, MI_TERMINAL};
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, SynChatCtx};

use super::message_bubble::{tool_call_card_only, tool_result_card_only};

/// Метаданные группы для рендера. Создаётся в `message_area::build_lane`,
/// потребляется один раз функцией [`view`]. `items` хранит снимки сообщений
/// (clone), чтобы reactive-closure владел данными `'static`-долго.
pub struct ToolGroup {
    /// Индекс первого `ChatMsg` группы в ленте — ключ для `tool_group_open`.
    pub start_idx: usize,
    /// Количество ПАР `(call, result)` в группе (минимум 2).
    pub count: usize,
    /// Ключ инструмента (`web`, `bash`, `kb_search`, …).
    pub tool_name: String,
    /// `true`, если хотя бы один результат в группе — ошибка.
    pub has_error: bool,
    /// Снимки сообщений группы: `(msg_idx, ChatMsg)`. Длина — `count * 2`.
    pub items: Vec<(usize, ChatMsg)>,
}

/// Рендер группы: Row(avatar, meta-column) — тот же layout, что у обычных
/// tool-карточек, чтобы лента не «прыгала» при смене режима отображения.
pub fn view(group: ToolGroup) -> impl Widget {
    let ToolGroup {
        start_idx,
        count,
        tool_name,
        has_error,
        items,
    } = group;

    let card_class = if has_error {
        "tool-group-card tool-group-card-with-error"
    } else {
        "tool-group-card"
    };

    let tool_icon = Tool::by_key(&tool_name)
        .map(|t| t.icon.to_string())
        .unwrap_or_else(|| MI_TERMINAL.to_string());
    let tool_label = Tool::by_key(&tool_name)
        .map(|t| t.label.to_string())
        .unwrap_or_else(|| tool_name.clone());

    let chevron_reactive = move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .tool_group_open
            .get()
            .get(&start_idx)
            .copied()
            .unwrap_or(false);
        let icon = if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE };
        Icon::new(icon).class("tool-group-chevron")
    };

    let mut header_children: Vec<Box<dyn Widget>> = Vec::with_capacity(6);
    header_children.push(Box::new(Icon::new(tool_icon).class("tool-group-icon")));
    header_children.push(Box::new(Text::new(tool_label).class("tool-group-name")));
    header_children.push(Box::new(
        Text::new(format!("×{count}")).class("tool-group-count"),
    ));
    if has_error {
        header_children.push(Box::new(
            Icon::new(MI_REPORT).class("tool-group-status-error"),
        ));
    }
    header_children.push(Box::new(DecoratedBox::new().class("grow")));
    header_children.push(
        syngui::widgets::containers::reactive::IntoWidget::into_widget(chevron_reactive),
    );

    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(header_children);

    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.tool_group_open.update(|m| {
                let cur = m.get(&start_idx).copied().unwrap_or(false);
                m.insert(start_idx, !cur);
            });
        })
        .child(header);

    // Body: реактивное содержимое в обёртке AnimatedSize. Закрыто —
    // zero-size DecoratedBox; открыто — Column с компактными card-only
    // карточками. AnimatedSize плавно tween'ит высоту.
    let items_for_body = items.clone();
    let body_reactive = Reactive::new(move || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx
            .tool_group_open
            .get()
            .get(&start_idx)
            .copied()
            .unwrap_or(false);
        if !open {
            return vec![Box::new(DecoratedBox::new()) as Box<dyn Widget>];
        }
        let mut cards: Vec<Box<dyn Widget>> = Vec::with_capacity(items_for_body.len());
        for (idx, msg) in &items_for_body {
            let card: Box<dyn Widget> = match &msg.kind {
                ChatMsgKind::ToolCall { tool_name } => {
                    Box::new(tool_call_card_only(msg, *idx, tool_name, false, true))
                }
                ChatMsgKind::ToolResult {
                    tool_name, error, ..
                } => Box::new(tool_result_card_only(msg, *idx, tool_name, *error, true)),
                // По дизайну в группе только Tool*-сообщения; защитный fallback.
                _ => Box::new(DecoratedBox::new()),
            };
            cards.push(card);
        }
        let col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(cards);
        vec![Box::new(DecoratedBox::new().class("tool-group-children").child(col)) as Box<dyn Widget>]
    });

    let body = AnimatedSize::new(body_reactive)
        .axis(AnimationAxis::Height)
        .duration_ms(200)
        .easing(Easing::EaseOutCubic);

    let card = DecoratedBox::new().class(card_class).child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(header_clickable) as Box<dyn Widget>,
                Box::new(body) as Box<dyn Widget>,
            ]),
    );

    let avatar = Avatar::new()
        .text("AI".to_string())
        .size(32.0)
        .class("avatar-blue".to_string());

    let meta_header = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new("Ассистент".to_string()).class("msg-author"),
            Text::new(format!("→ цепочка из {count} вызовов")).class("tool-call-hint"),
        ]
    };

    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![
            Box::new(meta_header) as Box<dyn Widget>,
            Box::new(card) as Box<dyn Widget>,
        ]);

    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar,
            meta,
        ]
    }
}
