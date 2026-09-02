//! Рендер одного сообщения в Syn-ленте: текстовые пузырьки ассистента и
//! пользователя + карточки tool-call / tool-result (работа агента).
//!
//! Tool-сообщения (`ChatMsgKind::ToolCall` / `ToolResult`) рендерятся не
//! plaintext'ом, а карточками `.tool-call-card` / `.tool-result-card` —
//! иконка инструмента, имя, время, статус и тело в моноширинной плашке.
//! Режим отображения задаётся `GeneralCtx.tool_display_mode`
//! (`full` / `minimal` / `hidden`), см. Settings → Общие.
//!
//! У текстовых пузырьков (и своих, и ассистента) есть панель действий:
//! копировать / править / удалить (+ «перегенерировать» у последнего ответа).
//! Правка — in-place: `SynChatCtx::editing_msg` переключает пузырёк на
//! поле ввода (`edit_box`), сохранение — `session::edit_message`.

use syngui::animation::Easing;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::{AnimatedSize, AnimationAxis, MultilineTextEdit, Reactive};

use crate::agent::tools::Tool;
use crate::context::AppCtx;
use crate::icons::{
    MI_ACCOUNT_TREE, MI_AUTORENEW, MI_CHECK, MI_CLOSE, MI_CONTENT_COPY, MI_DELETE, MI_EDIT,
    MI_EXPAND_LESS, MI_EXPAND_MORE, MI_PSYCHOLOGY, MI_REPORT, MI_TERMINAL,
};
use crate::pages::node_editor::run_controls;
use crate::pages::node_editor::tabs::{EditorWorkspace, RunState};
use crate::pages::node_editor::timing;
use crate::syn_chat::session;
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, ChatMsgRole, SynChatCtx};

/// Сколько строк tool-результата показывать в свёрнутом виде. Длинный
/// stdout (web-поиск отдаёт 10 результатов ≈ 60 строк) иначе выдавливает
/// ответ ассистента далеко вниз по ленте.
pub const TOOL_RESULT_PREVIEW_LINES: usize = 12;

pub fn view(
    msg: &ChatMsg,
    msg_idx: usize,
    is_typing: bool,
    is_last_assistant: bool,
    tool_mode: &str,
) -> Box<dyn Widget> {
    match &msg.kind {
        ChatMsgKind::ToolCall { tool_name } => match tool_mode {
            "hidden" => Box::new(DecoratedBox::new()),
            "minimal" => Box::new(tool_call_row(msg, msg_idx, tool_name, is_typing, true)),
            // `full` и любой неизвестный ключ → полный рендер.
            _ => Box::new(tool_call_row(msg, msg_idx, tool_name, is_typing, false)),
        },
        ChatMsgKind::ToolResult {
            tool_name, error, ..
        } => match tool_mode {
            // Медиа-результаты пайплайна показываем даже в hidden-режиме —
            // иначе сгенерированное видео просто исчезает из чата.
            "hidden" if msg.attachments.is_empty() => Box::new(DecoratedBox::new()),
            "hidden" => Box::new(attachments_only_row(msg)),
            "minimal" => Box::new(tool_result_row(msg, msg_idx, tool_name, *error, true)),
            _ => Box::new(tool_result_row(msg, msg_idx, tool_name, *error, false)),
        },
        ChatMsgKind::Text => match msg.role {
            // Медиа-результат прогона: тела нет, есть вложения — рисуем один
            // ряд карточек без пустого пузыря с аватаром.
            ChatMsgRole::Assistant if msg.body.trim().is_empty() && !msg.attachments.is_empty() => {
                Box::new(attachments_only_row(msg))
            }
            ChatMsgRole::System => Box::new(system_line(&msg.body, msg.error)),
            ChatMsgRole::User => Box::new(chat_row(msg, msg_idx, true, false, false)),
            ChatMsgRole::Assistant => {
                Box::new(chat_row(msg, msg_idx, false, is_typing, is_last_assistant))
            }
        },
        // Маркер компактификации рендерится на уровне ленты
        // (`message_area` → `compaction_marker::view`), сюда не попадает.
        // Защитный fallback, чтобы не падать на чужих JSON'ах.
        ChatMsgKind::CompactionMarker { .. } => Box::new(DecoratedBox::new()),
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

    // Правка in-place: подписка на `editing_msg` идёт в scope ленты
    // (`message_area::scroll_list`), так что смена режима её пересобирает.
    let editing =
        !is_typing && use_context::<SynChatCtx>().editing_msg.get() == Some(msg_idx);

    let avatar = Avatar::new()
        .text(msg.initials.clone())
        .size(32.0)
        .class(msg.tone_class.clone());

    let bubble_child: Box<dyn Widget> = if editing {
        Box::new(edit_box(msg_idx, body.clone()))
    } else if outgoing {
        Box::new(bubble_markdown(&body, "msg-bubble-md msg-bubble-out-md"))
    } else if is_last_assistant {
        let initial_body = body.clone();
        Box::new(syngui::widgets::containers::reactive::Reactive::new(
            move || -> Vec<Box<dyn Widget>> {
                let ctx = use_context::<SynChatCtx>();
                let pending = ctx.pending.get();
                let (tail, tool_tail) = if pending {
                    (ctx.streaming_body.get(), ctx.streaming_tool.get())
                } else {
                    (String::new(), String::new())
                };
                let merged = if tail.is_empty() {
                    initial_body.clone()
                } else if initial_body.is_empty() {
                    tail
                } else {
                    format!("{initial_body}{tail}")
                };
                let mut children: Vec<Box<dyn Widget>> = Vec::new();
                if !merged.is_empty() {
                    children.push(Box::new(bubble_markdown(&merged, "msg-bubble-md")));
                }
                if !tool_tail.is_empty() {
                    children.push(streaming_tool_preview(&tool_tail));
                }
                if children.is_empty() {
                    if pending {
                        return vec![Box::new(Text::new("•••").class("msg-typing"))];
                    }
                    children.push(Box::new(bubble_markdown(&merged, "msg-bubble-md")));
                }
                // Reactive — Loose-контейнер (дети в одной точке), поэтому
                // текст и превью складываем в явный Column.
                vec![Box::new(
                    Column::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(children),
                )]
            },
        ))
    } else if is_typing {
        Box::new(Text::new("•••").class("msg-typing"))
    } else {
        Box::new(bubble_markdown(&msg.body, "msg-bubble-md"))
    };

    let mut bubble_children: Vec<Box<dyn Widget>> = Vec::new();
    // Вложения идут первой строкой пузырька — так же, как их видит модель
    // (блоки-заполнители в промпте стоят перед текстом реплики).
    if !msg.attachments.is_empty() {
        bubble_children.push(super::attachments::bubble_grid(&msg.attachments));
    }
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
    let text_is_redundant = outgoing && body.trim().is_empty() && !msg.attachments.is_empty();
    if !text_is_redundant || editing {
        bubble_children.push(bubble_child);
    }

    // В режиме правки пузырёк растягивается: поле ввода должно быть
    // широким независимо от длины исходного текста.
    let bubble_class = if editing {
        format!("{bubble_class} msg-bubble-editing")
    } else {
        bubble_class.to_string()
    };
    let bubble = DecoratedBox::new().class(bubble_class).child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(bubble_children),
    );

    // Панель действий есть у всех текстовых пузырьков; у исходящих она
    // стоит слева от пузырька (лента выровнена вправо). В режиме правки
    // прячется — её место занимают «Сохранить»/«Отмена» в самом пузырьке.
    let show_actions = !editing && (!is_typing || is_last_assistant);
    let bubble_row: Box<dyn Widget> = if show_actions {
        let actions =
            actions_widget(Some(msg_idx), body.clone(), is_last_assistant && !outgoing, None);
        let bubble: Box<dyn Widget> = Box::new(bubble);
        let (children, align) = if outgoing {
            (vec![actions, bubble], MainAxisAlignment::End)
        } else {
            (vec![bubble, actions], MainAxisAlignment::Start)
        };
        Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .main_axis_alignment(align)
                .children(children),
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

/// Панель действий пузырька. `msg_idx = Some(..)` — текстовое сообщение
/// ленты: копировать / править / удалить (+ regen). `tool_idx = Some(..)` —
/// tool-карточка: копировать + удалить работу инструмента (вызов вместе с
/// результатом; правке tool-сообщения не подлежат). Оба `None` — только
/// копирование (медиа-ряды).
fn actions_widget(
    msg_idx: Option<usize>,
    body: String,
    regen_allowed: bool,
    tool_idx: Option<usize>,
) -> Box<dyn Widget> {
    syngui::widgets::containers::reactive::IntoWidget::into_widget(actions_row(
        msg_idx,
        body,
        regen_allowed,
        tool_idx,
    ))
}

fn actions_row(
    msg_idx: Option<usize>,
    body: String,
    regen_allowed: bool,
    tool_idx: Option<usize>,
) -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let pending = use_context::<SynChatCtx>().pending.get();
        if pending {
            return DecoratedBox::new().class("msg-actions-empty");
        }
        let body_for_copy = body.clone();
        let copy = ToolButton::new(MI_CONTENT_COPY)
            .tooltip(tr!("chat.msg.actions.copy.tooltip"))
            .on_click(move || {
                let plain = syngui::widgets::visual::markdown_view::linearize_markdown_source(
                    &body_for_copy,
                );
                syngui::clipboard::copy(&plain);
            })
            .class("msg-action-copy");
        let mut buttons: Vec<Box<dyn Widget>> = vec![Box::new(copy)];
        if let Some(idx) = msg_idx {
            buttons.push(Box::new(
                ToolButton::new(MI_EDIT)
                    .tooltip(tr!("chat.msg.actions.edit.tooltip"))
                    .on_click(move || use_context::<SynChatCtx>().editing_msg.set(Some(idx)))
                    .class("msg-action-edit"),
            ));
        }
        if regen_allowed {
            buttons.push(Box::new(
                ToolButton::new(MI_AUTORENEW)
                    .tooltip(tr!("chat.regenerate.tooltip"))
                    .on_click(session::regenerate_last)
                    .class("msg-action-regen"),
            ));
        }
        if let Some(idx) = msg_idx {
            buttons.push(Box::new(
                ToolButton::new(MI_DELETE)
                    .tooltip(tr!("chat.msg.actions.delete.tooltip"))
                    .on_click(move || session::delete_message(idx))
                    .class("msg-action-delete"),
            ));
        }
        if let Some(idx) = tool_idx {
            buttons.push(Box::new(
                ToolButton::new(MI_DELETE)
                    .tooltip(tr!("chat.msg.actions.delete_tool.tooltip"))
                    .on_click(move || session::delete_tool_work(idx))
                    .class("msg-action-delete"),
            ));
        }
        DecoratedBox::new().class("msg-actions").child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(buttons),
        )
    }
}

/// Поле правки текста сообщения внутри пузырька. Enter — сохранить,
/// Shift+Enter — перенос строки; «Отмена» возвращает исходный текст.
/// Черновик живёт в `Arc<Mutex<String>>`, а не в сигнале: сигнал в scope
/// ленты пересоздавался бы при каждом её ребилде.
fn edit_box(msg_idx: usize, body: String) -> impl Widget {
    use std::sync::{Arc, Mutex};
    let draft = Arc::new(Mutex::new(body.clone()));
    let draft_change = draft.clone();
    let draft_save = draft.clone();
    let editor = MultilineTextEdit::new()
        .text(body)
        .rows(2)
        .max_rows(16)
        .auto_height(true)
        .submit_on_enter(true)
        .on_change(move |s| {
            if let Ok(mut d) = draft_change.lock() {
                *d = s.to_string();
            }
        })
        .on_submit(move |s| session::edit_message(msg_idx, s.to_string()))
        .class("msg-edit-field");
    let save = Button::new(tr!("chat.msg.edit.save"))
        .leading_icon(MI_CHECK)
        .on_click(move || {
            let text = draft_save.lock().map(|d| d.clone()).unwrap_or_default();
            session::edit_message(msg_idx, text);
        })
        .class("code-editor-dialog-btn-primary");
    let cancel = Button::new(tr!("app.cancel"))
        .leading_icon(MI_CLOSE)
        .on_click(|| use_context::<SynChatCtx>().editing_msg.set(None))
        .class("code-editor-dialog-btn-secondary");
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("msg-edit-box") => [
                editor,
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::End) => [
                        Text::new(tr!("chat.msg.edit.hint")).class("msg-edit-hint"),
                        cancel,
                        save,
                    ],
            ]
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
            Text::new(tr!("chat.msg.thinking.title")).class("msg-thinking-title"),
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
            Text::new(tr!("chat.msg.thinking.title")).class("msg-thinking-title"),
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

// ─────────────────────────────────────────────────────────────────────────────
// Tool-карточки — работа агента (вызов инструмента и его результат)
// ─────────────────────────────────────────────────────────────────────────────

/// Live-превью tool-вызова: модель ещё дописывает команду, а её частичный
/// текст уже стримится в карточку — тем же стилем, что настоящая
/// `.tool-call-card`, которая заменит превью по завершении вызова.
fn streaming_tool_preview(raw: &str) -> Box<dyn Widget> {
    let (icon, label) = match extract_streaming_tool_name(raw) {
        Some(name) => tool_visuals(&name),
        None => (MI_TERMINAL.to_string(), tr!("chat.msg.tool.unknown")),
    };
    let header = mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(icon).class("tool-call-icon"),
            Text::new(label).class("tool-call-name"),
            DecoratedBox::new().class("grow"),
            Text::new(tr!("chat.msg.tool.writing")).class("tool-call-hint"),
        ]
    };
    let text = raw.trim_start();
    let body: Box<dyn Widget> = if text.is_empty() {
        Box::new(Text::new("•••").class("msg-typing"))
    } else {
        Box::new(DecoratedBox::new().class("tool-call-args-wrap").child(
            MarkdownView::new(fence_plain_text(text, "json"))
                .with_copy_code(false)
                .with_syntax_theme("InspiredGitHub")
                .class("tool-call-args"),
        ))
    };
    Box::new(DecoratedBox::new().class("tool-call-card").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![Box::new(header) as Box<dyn Widget>, body]),
    ))
}

/// Имя инструмента из частичного текста вызова, пока он ещё пишется.
/// Поддерживает Qwen3 JSON (`"name": "bash"`), ATEM (`name="bash"`) и
/// Anthropic-XML (`<function=bash>`, включая глюк `<functionfunction=`).
fn extract_streaming_tool_name(raw: &str) -> Option<String> {
    fn qwen_json(raw: &str) -> Option<String> {
        let pos = raw.find("\"name\"")?;
        let rest = raw[pos + "\"name\"".len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }
    fn atem_attr(raw: &str) -> Option<String> {
        let pos = raw.find("name=\"")?;
        let rest = &raw[pos + "name=\"".len()..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }
    fn xml_function(raw: &str) -> Option<String> {
        let pos = raw.find("function=")?;
        let rest = &raw[pos + "function=".len()..];
        let end = rest.find('>')?;
        Some(rest[..end].trim().to_string())
    }
    qwen_json(raw)
        .or_else(|| atem_attr(raw))
        .or_else(|| xml_function(raw))
        .filter(|s| !s.is_empty())
}

/// Иконка + человекочитаемое имя инструмента по его ключу. Неизвестный
/// ключ (скил или tool из будущей версии) деградирует до терминала.
fn tool_visuals(tool_name: &str) -> (String, String) {
    let icon = Tool::by_key(tool_name)
        .map(|t| t.icon.to_string())
        .unwrap_or_else(|| MI_TERMINAL.to_string());
    let label = Tool::by_key(tool_name)
        .map(crate::i18n::tool_label)
        .unwrap_or_else(|| tool_name.to_string());
    (icon, label)
}

/// Раскрыто ли тело карточки `msg_idx` (`SynChatCtx.tool_body_open`).
fn body_open(msg_idx: usize) -> bool {
    use_context::<SynChatCtx>()
        .tool_body_open
        .get()
        .get(&msg_idx)
        .copied()
        .unwrap_or(false)
}

/// Переключает раскрытие тела карточки `msg_idx`.
fn toggle_body(msg_idx: usize) {
    let ctx = use_context::<SynChatCtx>();
    ctx.tool_body_open.update(|m| {
        let cur = m.get(&msg_idx).copied().unwrap_or(false);
        m.insert(msg_idx, !cur);
    });
}

/// Chevron, отражающий состояние раскрытия карточки. Показывается только
/// в compact-режиме, где шапка работает как кнопка.
fn body_chevron(msg_idx: usize, class: &'static str) -> Box<dyn Widget> {
    syngui::widgets::containers::reactive::IntoWidget::into_widget(move || {
        let icon = if body_open(msg_idx) {
            MI_EXPAND_LESS
        } else {
            MI_EXPAND_MORE
        };
        Icon::new(icon).class(class)
    })
}

/// Оборачивает шапку карточки в клик-зону разворота (compact-режим).
fn clickable_header(msg_idx: usize, header: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(
        GestureDetector::new()
            .on_click(move || toggle_body(msg_idx))
            .child(header),
    )
}

/// Только карточка `.tool-call-card`, без обрамляющего avatar/meta-row.
/// Используется и в [`tool_call_row`], и из [`super::tool_group::view`],
/// где аватар с заголовком дублировать не нужно.
///
/// `compact` (режим `minimal`) прячет аргументы под клик по шапке —
/// цепочка из 5–7 вызовов не превращается в простыню, но содержимое
/// остаётся в одном клике.
pub(super) fn tool_call_card_only(
    msg: &ChatMsg,
    msg_idx: usize,
    tool_name: &str,
    is_typing: bool,
    compact: bool,
) -> impl Widget {
    let (tool_icon, tool_label) = tool_visuals(tool_name);

    let mut header_children: Vec<Box<dyn Widget>> = Vec::with_capacity(5);
    header_children.push(Box::new(Icon::new(tool_icon).class("tool-call-icon")));
    header_children.push(Box::new(Text::new(tool_label).class("tool-call-name")));
    header_children.push(Box::new(DecoratedBox::new().class("grow")));
    header_children.push(Box::new(Text::new(msg.time.clone()).class("msg-time")));
    if compact {
        header_children.push(body_chevron(msg_idx, "tool-call-chevron"));
    }
    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(header_children);

    // Аргументы: pretty-JSON в ```json-fence. Fence нужен, потому что модель
    // часто пакует в аргумент многострочный текст с `#`-комментариями и
    // **bold** — без него MarkdownView трактовал бы это как разметку.
    let args_body = msg.body.clone();
    let typing_placeholder = is_typing && args_body.trim().is_empty();
    let args_widget = move || -> Box<dyn Widget> {
        if typing_placeholder {
            return Box::new(Text::new("•••").class("msg-typing"));
        }
        let md = if args_body.trim().is_empty() {
            tr!("chat.msg.tool.no_args")
        } else {
            fence_plain_text(&unescape_persisted_json_newlines(&args_body), "json")
        };
        Box::new(DecoratedBox::new().class("tool-call-args-wrap").child(
            MarkdownView::new(md)
                .selectable(true)
                .with_copy_code(false)
                .with_syntax_theme("InspiredGitHub")
                .class("tool-call-args"),
        ))
    };

    let card_children: Vec<Box<dyn Widget>> = if compact {
        vec![
            clickable_header(msg_idx, header),
            collapsible(msg_idx, args_widget),
        ]
    } else {
        vec![Box::new(header), args_widget()]
    };

    DecoratedBox::new().class("tool-call-card").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(card_children),
    )
}

/// Тело карточки, живущее под chevron'ом: пусто пока свёрнуто, содержимое
/// `build` — когда раскрыто. `AnimatedSize` сглаживает смену высоты.
fn collapsible<F>(msg_idx: usize, build: F) -> Box<dyn Widget>
where
    F: Fn() -> Box<dyn Widget> + Send + Sync + 'static,
{
    let reactive = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if body_open(msg_idx) {
            vec![build()]
        } else {
            vec![Box::new(DecoratedBox::new())]
        }
    });
    Box::new(
        AnimatedSize::new(reactive)
            .axis(AnimationAxis::Height)
            .duration_ms(200)
            .easing(Easing::EaseOutCubic),
    )
}

fn tool_call_row(msg: &ChatMsg, msg_idx: usize, tool_name: &str, is_typing: bool, compact: bool) -> impl Widget {
    let avatar = Avatar::new()
        .text(msg.initials.clone())
        .size(32.0)
        .class(msg.tone_class.clone());

    let card = tool_call_card_only(msg, msg_idx, tool_name, is_typing, compact);

    let author_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(msg.author.clone()).class("msg-author"),
            Text::new(tr!("chat.msg.tool_call.hint")).class("tool-call-hint"),
        ]
    };

    // Действия справа от карточки: копировать аргументы + удалить работу
    // инструмента (вызов вместе с результатом). Во время стрима
    // (`is_typing`) прячем: копировать нечего, удалять рано.
    let card_with_actions: Box<dyn Widget> = if is_typing {
        Box::new(card)
    } else {
        Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .main_axis_alignment(MainAxisAlignment::Start)
                .children(vec![
                    Box::new(card) as Box<dyn Widget>,
                    actions_widget(None, msg.body.clone(), false, Some(msg_idx)),
                ]),
        )
    };

    let mut meta_children: Vec<Box<dyn Widget>> =
        vec![Box::new(author_row) as Box<dyn Widget>, card_with_actions];
    // Живой статус прогона — под последним tool-call `pipelines`: пока
    // результат не пришёл, вызов остаётся хвостом ленты.
    if tool_name == "pipelines" {
        let chat = use_context::<SynChatCtx>();
        let is_tail = chat
            .messages
            .get_untracked()
            .len()
            .saturating_sub(1)
            == msg_idx;
        if is_tail {
            meta_children.push(pipeline_live_card());
        }
    }

    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(meta_children);

    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar,
            meta,
        ]
    }
}

/// Парный к [`tool_call_card_only`] helper: карточка результата без
/// avatar/meta-row.
///
/// - `compact` (`minimal`-режим) — тело под клик по шапке;
/// - `full` — тело видно сразу, а вывод длиннее
///   [`TOOL_RESULT_PREVIEW_LINES`] усекается до превью со строкой-разворотом.
pub(super) fn tool_result_card_only(
    msg: &ChatMsg,
    msg_idx: usize,
    tool_name: &str,
    error: bool,
    compact: bool,
) -> impl Widget {
    let (tool_icon, tool_label) = tool_visuals(tool_name);

    let (card_class, status_icon, status_class) = if error {
        (
            "tool-result-card tool-result-card-error",
            MI_REPORT,
            "tool-result-status-icon tool-result-status-icon-error",
        )
    } else {
        ("tool-result-card", MI_CHECK, "tool-result-status-icon")
    };

    let display_body = unescape_persisted_json_newlines(&msg.body);
    let total_lines = display_body.lines().count();
    // В full-режиме длинный вывод показываем превью'шкой; в compact тело и
    // так скрыто целиком, поэтому по клику разворачиваем его полностью.
    let truncatable = !compact && total_lines > TOOL_RESULT_PREVIEW_LINES;

    let mut header_children: Vec<Box<dyn Widget>> = Vec::with_capacity(7);
    header_children.push(Box::new(Icon::new(tool_icon).class("tool-result-icon")));
    header_children.push(Box::new(Text::new(tool_label).class("tool-result-name")));
    if truncatable || (compact && total_lines > 1) {
        header_children.push(Box::new(
            Text::new(trn!("chat.msg.tool_result.lines_count", total_lines)).class("tool-result-lines"),
        ));
    }
    header_children.push(Box::new(DecoratedBox::new().class("grow")));
    header_children.push(Box::new(Icon::new(status_icon).class(status_class)));
    header_children.push(Box::new(Text::new(msg.time.clone()).class("msg-time")));
    if compact {
        header_children.push(body_chevron(msg_idx, "tool-result-chevron"));
    }

    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(header_children);

    let body_class = if error {
        "tool-result-body tool-result-body-error"
    } else {
        "tool-result-body"
    };

    // Тело — плоский текст инструмента: оборачиваем в код-фенс, чтобы
    // markdown-разметка внутри вывода не срабатывала (см. fence_plain_text),
    // а длинные строки переносились код-блоком.
    let body_wrap = move |text: String| -> Box<dyn Widget> {
        Box::new(DecoratedBox::new().class("tool-result-body-wrap").child(
            MarkdownView::new(fence_plain_text(&text, ""))
                .selectable(true)
                .with_copy_code(false)
                .class(body_class),
        ))
    };

    let mut card_children: Vec<Box<dyn Widget>> = if compact {
        let full_body = display_body.clone();
        vec![
            clickable_header(msg_idx, header),
            collapsible(msg_idx, move || body_wrap(full_body.clone())),
        ]
    } else if truncatable {
        let body_for_reactive = display_body.clone();
        let preview_reactive = Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let shown = if body_open(msg_idx) {
                body_for_reactive.clone()
            } else {
                let head: Vec<&str> = body_for_reactive
                    .lines()
                    .take(TOOL_RESULT_PREVIEW_LINES)
                    .collect();
                format!("{}\n…", head.join("\n"))
            };
            vec![body_wrap(shown)]
        });
        vec![
            Box::new(header),
            Box::new(
                AnimatedSize::new(preview_reactive)
                    .axis(AnimationAxis::Height)
                    .duration_ms(200)
                    .easing(Easing::EaseOutCubic),
            ),
            Box::new(tool_result_toggle(msg_idx, total_lines)),
        ]
    } else {
        vec![Box::new(header), body_wrap(display_body)]
    };

    // Медиа-результаты пайплайна (mp4/wav из save-нод) — плитками ВНЕ
    // collapsible: результат видно сразу, полноэкранный просмотр работает
    // тем же media_viewer'ом, что и у пользовательских вложений.
    if !msg.attachments.is_empty() {
        card_children.push(super::attachments::bubble_grid(&msg.attachments));
    }
    if tool_name == "pipelines" {
        card_children.push(Box::new(open_graph_link()));
    }

    DecoratedBox::new().class(card_class).child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(card_children),
    )
}

/// Строка «Результат» без карточки — для `tool_display_mode = hidden`, где
/// прячется всё, кроме медиа-плиток пайплайна.
fn attachments_only_row(msg: &ChatMsg) -> impl Widget {
    let avatar_placeholder = DecoratedBox::new()
        .class("tool-result-avatar")
        .child(Center::new().child(Icon::new(MI_CHECK).class("tool-result-avatar-icon")));
    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![super::attachments::bubble_grid(&msg.attachments)]);
    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar_placeholder,
            meta,
        ]
    }
}

/// Ссылка «Открыть граф» — раскрывает служебную вкладку агента текущего
/// чата и переключает роут на нодовый редактор.
fn open_graph_link() -> impl Widget {
    let link = mgui! {
        Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_ACCOUNT_TREE).class("pipeline-link-icon"),
            Text::new(tr!("chat.msg.pipeline.open_graph")).class("pipeline-link-text"),
        ]
    };
    GestureDetector::new()
        .on_click(|| {
            let chat = use_context::<SynChatCtx>();
            let Some(chat_id) = chat.active_chat_id.get_untracked() else {
                return;
            };
            let ws = use_context::<EditorWorkspace>();
            let Some(tab) = ws.agent_tab_for_chat(&chat_id) else {
                return;
            };
            ws.reveal(tab);
            let app = use_context::<AppCtx>();
            if let Ok(mut r) = app.router.lock() {
                r.navigate("nodes");
            }
            app.current_route.set("nodes".to_string());
        })
        .child(DecoratedBox::new().class("pipeline-link").child(link))
}

/// Живая карточка прогона пайплайна — рендерится под tool-call `pipelines`,
/// пока секвенсер работает: «нод 2/5 · LTX Sampler Stage1 43% · 3:12»,
/// ссылка на граф и отмена (abort хода → cancel-флаги нод). Сам ticker
/// держим здесь же: бейдж нодовой страницы не смонтирован, пока открыт чат,
/// и без него elapsed бы замирал.
fn pipeline_live_card() -> Box<dyn Widget> {
    let status = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ws = use_context::<EditorWorkspace>();
        if ws.run_state.get() != RunState::Running {
            return vec![];
        }
        let done = ws.run_done.get();
        let total = ws.run_total.get();
        let elapsed = ws
            .run_timer
            .display_ms()
            .map(timing::fmt_elapsed)
            .unwrap_or_default();
        let active = run_controls::active_nodes_status();
        let active_txt = active
            .iter()
            .map(|(title, pct)| match pct {
                Some(p) if *p > 0.0 => format!("{title} {:.0}%", (p * 100.0).min(100.0)),
                _ => title.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut line = tr!("chat.msg.pipeline.progress", done = done, total = total);
        if !active_txt.is_empty() {
            line.push_str(&format!(" · {active_txt}"));
        }
        if !elapsed.is_empty() {
            line.push_str(&format!(" · {elapsed}"));
        }

        let cancel = GestureDetector::new()
            .on_click(session::abort_current)
            .child(
                DecoratedBox::new()
                    .class("pipeline-live-cancel")
                    .child(Text::new(tr!("chat.msg.pipeline.cancel")).class("pipeline-live-cancel-text")),
            );

        vec![Box::new(
            DecoratedBox::new().class("pipeline-live-card").child(mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_AUTORENEW).class("pipeline-live-spinner"),
                    Text::new(line).class("pipeline-live-text"),
                    open_graph_link(),
                    cancel,
                ]
            }),
        )]
    });
    let ws = use_context::<EditorWorkspace>();
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(vec![
                Box::new(status) as Box<dyn Widget>,
                timing::ticker(ws.run_timer),
            ]),
    )
}

/// Кликабельная строка «Показать всё / Свернуть» под усечённым результатом.
fn tool_result_toggle(msg_idx: usize, total_lines: usize) -> impl Widget {
    let label_reactive = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let (icon, text) = if body_open(msg_idx) {
            (MI_EXPAND_LESS, tr!("chat.msg.tool_result.collapse"))
        } else {
            (MI_EXPAND_MORE, trn!("chat.msg.tool_result.show_all", total_lines))
        };
        vec![Box::new(DecoratedBox::new().class("tool-result-more").child(mgui! {
            Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(icon).class("tool-result-more-icon"),
                Text::new(text).class("tool-result-more-text"),
            ]
        }))]
    });

    GestureDetector::new()
        .on_click(move || toggle_body(msg_idx))
        .child(label_reactive)
}

fn tool_result_row(
    msg: &ChatMsg,
    msg_idx: usize,
    tool_name: &str,
    error: bool,
    compact: bool,
) -> impl Widget {
    let card = tool_result_card_only(msg, msg_idx, tool_name, error, compact);

    // Layout идентичен assistant-row (avatar + meta-column): так карточка
    // наследует bounded-width от внешнего Row и `max-width: 90%` реально
    // ограничивает ширину. Плашка «✓» вместо аватара подсказывает, что это
    // не ответ ассистента, а вывод инструмента.
    let avatar_placeholder = DecoratedBox::new()
        .class("tool-result-avatar")
        .child(Center::new().child(Icon::new(MI_CHECK).class("tool-result-avatar-icon")));

    let card_with_actions: Box<dyn Widget> = Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::End)
            .main_axis_alignment(MainAxisAlignment::Start)
            .children(vec![
                Box::new(card) as Box<dyn Widget>,
                actions_widget(None, msg.body.clone(), false, Some(msg_idx)),
            ]),
    );

    let header_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(tr!("chat.msg.tool_result.author")).class("msg-author"),
            Text::new(msg.time.clone()).class("msg-time"),
        ]
    };

    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![Box::new(header_row) as Box<dyn Widget>, card_with_actions]);

    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar_placeholder,
            meta,
        ]
    }
}

/// Оборачивает плоский текст в код-фенс, выбирая ограждение длиннее самой
/// длинной серии бэктиков внутри (иначе ``` в содержимом рвёт блок).
///
/// Вывод инструмента — не markdown, и парсить его как markdown нельзя:
/// `echo "---"` в stdout превращал весь текст выше в setext-заголовок H2
/// (крупный жирный шрифт с линией-подчёркиванием), `#` становился
/// заголовком, отступы — код-блоками, `*` — курсивом.
fn fence_plain_text(text: &str, lang: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}{lang}\n{text}\n{fence}")
}

/// Tool-результаты, сохранённые как pretty-JSON, содержат `\n` в виде двух
/// символов `\` + `n`. Разворачиваем обратно, чтобы старые чаты не
/// показывались одной бесконечной строкой. Новые (plain-text) проходят
/// без изменений: реальный перевод строки — это `U+000A`, не пара ASCII.
fn unescape_persisted_json_newlines(s: &str) -> String {
    if !s.contains("\\n") && !s.contains("\\t") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => { chars.next(); out.push('\n'); continue; }
                Some('t') => { chars.next(); out.push('\t'); continue; }
                Some('r') => { chars.next(); out.push('\r'); continue; }
                Some('\\') => { chars.next(); out.push('\\'); continue; }
                Some('"') => { chars.next(); out.push('"'); continue; }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_restores_newlines_and_tabs() {
        assert_eq!(unescape_persisted_json_newlines("a\\nb\\tc"), "a\nb\tc");
    }

    #[test]
    fn unescape_is_noop_for_plain_text() {
        let s = "WEB_SEARCH query=\"rust\"\nresults-count: 10";
        assert_eq!(unescape_persisted_json_newlines(s), s);
    }

    #[test]
    fn fence_wraps_plain_text() {
        assert_eq!(fence_plain_text("a\nb", ""), "```\na\nb\n```");
        assert_eq!(fence_plain_text("{}", "json"), "```json\n{}\n```");
    }

    #[test]
    fn fence_grows_past_backtick_runs_inside() {
        // ``` внутри текста не должен закрывать блок — ограждение длиннее.
        let out = fence_plain_text("код:\n```rust\nfn main() {}\n```", "");
        assert!(out.starts_with("````\n"), "{out}");
        assert!(out.ends_with("\n````"), "{out}");
    }

    #[test]
    fn markdown_structure_in_tool_output_stays_literal() {
        // Регрессия: echo "---" в stdout превращал текст выше в setext-H2.
        let body = "$ echo \"---\"\nexit: 0\n--- stdout ---\n---";
        let fenced = fence_plain_text(body, "");
        let blocks = syngui::widgets::visual::markdown_view::parse_markdown(&fenced);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert!(
            matches!(
                &blocks[0],
                syngui::widgets::visual::markdown_view::MdBlock::CodeBlock { code, .. }
                if code.contains("--- stdout ---")
            ),
            "{blocks:?}"
        );
    }

    #[test]
    fn extract_tool_name_from_partial_streams() {
        assert_eq!(
            extract_streaming_tool_name(r#"{"name": "bash", "argu"#).as_deref(),
            Some("bash")
        );
        // Двоеточие ещё не приехало — имени пока нет.
        assert_eq!(extract_streaming_tool_name(r#"{"name""#), None);
        assert_eq!(
            extract_streaming_tool_name("<atem:invoke name=\"web\">").as_deref(),
            Some("web")
        );
        assert_eq!(
            extract_streaming_tool_name("<functionfunction=kb_search><param").as_deref(),
            Some("kb_search")
        );
        assert_eq!(extract_streaming_tool_name("просто текст"), None);
    }
}
