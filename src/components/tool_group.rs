//! Сворачиваемая группа подряд идущих tool-вызовов одного типа.
//!
//! В режиме `tool_display_mode == "minimal"` цепочки вида
//! `ToolCall(bash) + ToolResult(bash) + ToolCall(bash) + ToolResult(bash) + …`
//! (≥ 2 одинаковых пар) сжимаются в одну карточку «Bash ×N» с chevron'ом.
//! Pre-pass и обнаружение групп — в [`super::message_area::build_lane`].
//!
//! Состояние раскрытия живёт в `chat.tool_group_open: RwSignal<HashMap<usize, bool>>`
//! (см. [`crate::chat::state::ChatCtx`]) — ключом является индекс первого
//! `ChatMsg` группы в текущей ленте, дефолт — закрыто. Эфемерное состояние,
//! не persist'ится и сбрасывается при переключении чата.
//!
//! Внутри развёрнутой группы рендерятся «голые» card-only-карточки
//! ([`tool_call_card_only`] / [`tool_result_card_only`]) — без аватаров и
//! meta-row, чтобы цепочка из 5–7 пар не превращалась в шум.

use syngui::animation::Easing;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{AnimatedSize, AnimationAxis, Reactive};

use crate::chat::tools::Tool;
use crate::chat::{ChatMsg, ChatMsgKind};
use crate::context::AppCtx;
use crate::icons::{MI_EXPAND_LESS, MI_EXPAND_MORE, MI_REPORT, MI_TERMINAL};

use super::message_bubble::{tool_call_card_only, tool_result_card_only};

/// Метаданные группы для рендера. Создаётся в `message_area::build_lane`,
/// потребляется один раз функцией [`view`]. Поле `items` хранит снимки
/// сообщений (clone) — это нужно, чтобы reactive-closure внутри владел
/// данными `'static`-долго и независимо от внешнего lifetime ленты.
pub struct ToolGroup {
    /// Индекс первого `ChatMsg` группы в текущей ленте — стабильный ключ
    /// для `tool_group_open` HashMap.
    pub start_idx: usize,
    /// Количество ПАР `(call, result)` в группе (минимум 2).
    pub count: usize,
    /// Имя инструмента (`bash`, `web_read`, `kb_search` и т.п.).
    pub tool_name: String,
    /// `true`, если хотя бы один результат в группе — ошибка.
    pub has_error: bool,
    /// Снимки сообщений группы: `(msg_idx, ChatMsg)`. Длина — `count * 2`.
    pub items: Vec<(usize, ChatMsg)>,
}

/// Рендер группы. Возвращает Row(avatar, meta-column) — тот же layout,
/// что у обычных tool-карточек, чтобы лента визуально не «прыгала» при
/// переключении режимов отображения.
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

    // Header: иконка + имя + ×N + (опц. error-метка) + grow + реактивный chevron.
    let chevron_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
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
        Text::new(format!("×{}", count)).class("tool-group-count"),
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
            let chat = use_context::<AppCtx>().chat.clone();
            chat.tool_group_open.update(|m| {
                let cur = m.get(&start_idx).copied().unwrap_or(false);
                m.insert(start_idx, !cur);
            });
        })
        .child(header);

    // Body: реактивный contents в обёртке AnimatedSize. При закрытом —
    // zero-size DecoratedBox; при открытом — Column с компактными
    // card-only-карточками. AnimatedSize ловит смену natural-size и
    // плавно tween'ит высоту (200мс EaseOutCubic).
    let items_for_body = items.clone();
    let body_reactive = Reactive::new(move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
            .tool_group_open
            .get()
            .get(&start_idx)
            .copied()
            .unwrap_or(false);
        if !open {
            return vec![Box::new(DecoratedBox::new()) as Box<dyn Widget>];
        }
        let mut cards: Vec<Box<dyn Widget>> = Vec::with_capacity(items_for_body.len());
        for (_idx, msg) in &items_for_body {
            let card: Box<dyn Widget> = match &msg.kind {
                ChatMsgKind::ToolCall { tool_name } => {
                    Box::new(tool_call_card_only(msg, tool_name, false, true))
                }
                ChatMsgKind::ToolResult {
                    tool_name, error, ..
                } => Box::new(tool_result_card_only(msg, tool_name, *error, true)),
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

    // Тот же layout (avatar + meta-column), что у обычных tool-карточек.
    // Avatar — стандартный «AI» в blue-tone'е, чтобы группа визуально была
    // равнозначна assistant-цепочке.
    let avatar = Avatar::new()
        .text("AI".to_string())
        .size(32.0)
        .class("avatar-blue".to_string());

    let meta_header = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new("Ассистент".to_string()).class("msg-author"),
            Text::new(format!("→ цепочка из {} вызовов", count)).class("tool-call-hint"),
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
