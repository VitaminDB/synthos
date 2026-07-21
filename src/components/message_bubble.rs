//! Рендер одного сообщения в ленте. Варианты:
//!
//! - `User`      — правый бабл `msg-bubble-out`, `MarkdownView` (класс
//!   `.msg-bubble-out-md` поверх `.msg-bubble-md` переопределяет цвета под
//!   primary-фон).
//! - `Assistant` (`kind=Text`) — левый бабл `msg-bubble-in`, `MarkdownView`
//!   либо пульсирующие точки, если `is_typing`.
//! - `Assistant` (`kind=ToolCall`) — мини-карточка с иконкой инструмента,
//!   именем и pretty-JSON аргументов.
//! - `System` (`kind=Text`) — центрированная плашка `.msg-system`.
//! - `System` (`kind=ToolResult`) — карточка tool-result, отдельный
//!   `.tool-result` / `.tool-result-error` стиль.
//!
//! Классы `avatar-*`, `msg-*` и keyframe `bubble-pop-in` переиспользуются
//! без правок.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::ImageFit;

use crate::chat::tools::Tool;
use crate::chat::{blobs, session, ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment};
use crate::context::AppCtx;
use crate::icons::{MI_AUTORENEW, MI_CHECK, MI_CONTENT_COPY, MI_EXPAND_LESS, MI_EXPAND_MORE, MI_PLAY_ARROW, MI_PSYCHOLOGY, MI_REPORT, MI_TERMINAL};

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
            // "full" и любой неизвестный ключ → полный рендер.
            _ => Box::new(tool_call_row(msg, msg_idx, tool_name, is_typing, false)),
        },
        ChatMsgKind::ToolResult {
            tool_name, error, ..
        } => match tool_mode {
            "hidden" => Box::new(DecoratedBox::new()),
            "minimal" => Box::new(tool_result_row(msg, msg_idx, tool_name, *error, true)),
            _ => Box::new(tool_result_row(msg, msg_idx, tool_name, *error, false)),
        },
        ChatMsgKind::Text => match msg.role {
            ChatMsgRole::System => Box::new(system_line(&msg.body, msg.error)),
            ChatMsgRole::User => Box::new(chat_row(msg, msg_idx, true, false, false)),
            ChatMsgRole::Assistant => {
                Box::new(chat_row(msg, msg_idx, false, is_typing, is_last_assistant))
            }
        },
        // CompactionMarker отрисовывается отдельным компонентом из
        // `message_area` (там есть доступ к `compaction_open` и pre-pass
        // мапе iter→marker_idx). Сюда `view` вызывается только для свёрнутых
        // сообщений внутри развёрнутого маркера; маркер-сообщение сюда не
        // попадает. Если всё же попало — рендер пустой плашкой, не падаем.
        ChatMsgKind::CompactionMarker { .. } => Box::new(DecoratedBox::new()),
    }
}

/// Reactive-обёртка над `actions_row(msg_idx, body, ...)` для встраивания
/// в Row через `Vec<Box<dyn Widget>>`. Используется и в `chat_row`, и в
/// tool-bubble'ах. `body` нужен Copy-кнопке (она копирует plain-text);
/// для tool-call/tool-result передаётся аргументы/результат — пользователь
/// получит то же, что видит на экране. `continue_allowed=true` добавляет
/// кнопку «Продолжить» (только под последним assistant-text-bubble).
fn actions_widget(
    msg_idx: usize,
    body: String,
    regen_allowed: bool,
    continue_allowed: bool,
) -> Box<dyn Widget> {
    syngui::widgets::containers::reactive::IntoWidget::into_widget(actions_row(
        msg_idx,
        body,
        regen_allowed,
        continue_allowed,
    ))
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
        // User-bubble: сначала прикреплённые картинки (если есть), потом
        // markdown текста. Если тело пустое (отправили только картинки) —
        // markdown-блок не добавляем.
        if msg.attachments.is_empty() {
            Box::new(bubble_markdown(&body, "msg-bubble-md msg-bubble-out-md"))
        } else {
            let mut parts: Vec<Box<dyn Widget>> = Vec::new();
            parts.push(Box::new(attachments_block(&msg.attachments)));
            if !body.is_empty() {
                parts.push(Box::new(bubble_markdown(
                    &body,
                    "msg-bubble-md msg-bubble-out-md",
                )));
            }
            Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(parts),
            )
        }
    } else if is_last_assistant {
        // Последний assistant-text bubble: подписан на `pending` +
        // `streaming_body`. Пока pending — склеивает initial body со
        // стрим-хвостом; не pending — рисует статичный markdown. Только
        // этот один bubble пересобирается на токенах, лента целиком
        // не страдает.
        let initial_body = body.clone();
        Box::new(syngui::widgets::containers::reactive::Reactive::new(
            move || -> Vec<Box<dyn Widget>> {
                let chat = use_context::<AppCtx>().chat.clone();
                let pending = chat.pending.get();
                let tail = if pending {
                    chat.streaming_body.get()
                } else {
                    String::new()
                };
                if pending && initial_body.is_empty() && tail.is_empty() {
                    // Ни одного токена ещё не было — typing-индикатор.
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

    // Thinking-блок (chain-of-thought reasoning-моделей) — только у
    // ассистента и только если действительно есть reasoning-текст.
    // Default-раскрытие: открыт, пока body пустой (модель ещё думает);
    // свёрнут, когда уже пошёл ответ. Пользователь может toggle'ить.
    let mut bubble_children: Vec<Box<dyn Widget>> = Vec::new();
    if !outgoing {
        let default_open = msg.body.is_empty();
        let initial_thinking = msg.thinking.clone();
        if is_last_assistant {
            // Последний bubble: реактивный thinking подписан на
            // `streaming_thinking`. На !pending он сам схлопывается в
            // zero-size, если суммарный thinking пуст.
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

    // bubble + actions в одной горизонтальной строке. cross-axis end —
    // кнопка прижимается к нижнему краю bubble. На user-bubble — без
    // кнопок. На последнем assistant-bubble actions показываются всегда:
    // их Reactive-обёртка сама гасит блок при `pending=true`.
    let show_actions = !outgoing && (!is_typing || is_last_assistant);
    let bubble_row: Box<dyn Widget> = if show_actions {
        Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .main_axis_alignment(MainAxisAlignment::Start)
                .children(vec![
                    Box::new(bubble) as Box<dyn Widget>,
                    actions_widget(
                        msg_idx,
                        body.clone(),
                        msg.compacted_iter.is_none(),
                        is_last_assistant,
                    ),
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

/// Реактивный action-row под ассистент-bubble. Кнопки: «Скопировать»
/// (целое сообщение через `linearize_markdown_source`) и «Сгенерировать
/// заново». Скрывается реактивно когда `chat.pending=true` — клик в
/// процессе стрима всё равно отбрасывается `regenerate_from`'ом, но
/// визуально лучше не показывать кликабельный элемент.
fn actions_row(
    msg_idx: usize,
    body: String,
    regen_allowed: bool,
    continue_allowed: bool,
) -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let pending = use_context::<AppCtx>().chat.pending.get();
        if pending {
            return DecoratedBox::new().class("msg-actions-empty");
        }
        let body_for_copy = body.clone();
        let copy = ToolButton::new(MI_CONTENT_COPY)
            .tooltip("Скопировать сообщение")
            .on_click(move || {
                // `linearize_markdown_source` отдаёт plain-text без
                // markdown-разметки (`**`, `` ` ``, `[…](…)` пропадают),
                // что и ожидает пользователь от «копировать сообщение».
                let plain = syngui::widgets::visual::markdown_view::linearize_markdown_source(
                    &body_for_copy,
                );
                syngui::clipboard::copy(&plain);
            })
            .class("msg-action-copy");
        // Continue показываем только под последним assistant-bubble и
        // только если у него уже есть body (плейсхолдер «•••» продолжать
        // нечего). Кейс: модель оборвала ответ на `finish_reason=Stop`
        // посреди фразы — пользователь жмёт и догенерируем поверх.
        let continue_btn: Option<_> = if continue_allowed && !body.is_empty() {
            Some(
                ToolButton::new(MI_PLAY_ARROW)
                    .tooltip("Продолжить ответ")
                    .on_click(session::continue_assistant)
                    .class("msg-action-continue"),
            )
        } else {
            None
        };
        // Regenerate-кнопку не показываем для свёрнутых autocompact-маркером
        // сообщений: `regenerate_from(idx)` обрезает ленту до ближайшего
        // user-Text и затёр бы маркер вместе со всеми свёрнутыми, что
        // потеряло бы ранее сжатый контекст.
        let regen: Option<_> = if regen_allowed {
            Some(
                ToolButton::new(MI_AUTORENEW)
                    .tooltip("Сгенерировать заново")
                    .on_click(move || session::regenerate_from(msg_idx))
                    .class("msg-action-regen"),
            )
        } else {
            None
        };
        let mut buttons: Vec<Box<dyn Widget>> = Vec::new();
        buttons.push(Box::new(copy));
        if let Some(c) = continue_btn {
            buttons.push(Box::new(c));
        }
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

/// `MarkdownView` полностью стилизуется через MSS. Базовый класс —
/// `.msg-bubble-md`; для исходящих сообщений добавляется второй класс
/// `.msg-bubble-out-md`, переопределяющий `--md-*` под фон `--primary`.
/// Нормализация (`•` → `-`, схлопывание blank-линий внутри списков) —
/// общая для обеих сторон: пользователь тоже может вставить bullet-точку
/// или «loose»-список из другого редактора.
fn bubble_markdown(body: &str, class: &'static str) -> impl Widget {
    MarkdownView::new(normalize_assistant_markdown(body))
        // Светлая тема syntect — фон code-блока в баблах задан через
        // `--md-code-block-bg = var(--bg-window)` (светлый), а дефолтная
        // `base16-ocean.dark` рассчитана на тёмный фон → текст бледный.
        .with_syntax_theme("InspiredGitHub")
        .with_copy_code(true)
        .class(class)
}

/// Колонка с прикреплёнными картинками внутри user-bubble. Каждая картинка
/// — `Image::Contain` в `DecoratedBox.msg-attachment-thumb`, шириной до
/// `--bubble-max-width` (через MSS), сохраняет aspect через ImageFit::Contain.
/// Сами картинки лежат в blob-CAS (`~/.config/synthos/blobs/`).
fn attachments_block(atts: &[MsgAttachment]) -> impl Widget {
    let cards: Vec<Box<dyn Widget>> = atts
        .iter()
        .map(|a| {
            let path = blobs::full_path(a).display().to_string();
            let widget: Box<dyn Widget> = Box::new(
                DecoratedBox::new()
                    .class("msg-attachment-thumb")
                    .child(Image::new(path).fit(ImageFit::Contain)),
            );
            widget
        })
        .collect();
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(cards)
}

/// Collapsible-блок с reasoning-текстом ассистента. Шапка (иконка `psychology`,
/// заголовок «Размышления», chevron) кликабельна — toggle'ит видимость тела.
/// Состояние открыт/закрыт — в `chat.thinking_open` (HashMap по индексу
/// сообщения), `default_open` применяется когда записи в Map нет (первое
/// отображение). Тело — реактивная функция, рендерит MarkdownView только
/// когда блок раскрыт; иначе zero-size DecoratedBox (бесплатно).
fn thinking_block(msg_idx: usize, thinking: String, default_open: bool) -> impl Widget {
    // Header: реактивный chevron меняется по состоянию open.
    let header_left = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_PSYCHOLOGY).class("msg-thinking-icon"),
            Text::new("Размышления").class("msg-thinking-title"),
        ]
    };
    let chevron_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
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
            header_left,
            DecoratedBox::new().class("grow"),
            chevron_reactive,
        ]
    };

    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let chat = use_context::<AppCtx>().chat.clone();
            chat.thinking_open.update(|m| {
                let cur = m.get(&msg_idx).copied().unwrap_or(default_open);
                m.insert(msg_idx, !cur);
            });
        })
        .child(header);

    // Тело: переcобирается реактивно при смене open. Закрытое — пустой
    // zero-size контейнер (не рендерится в layout).
    let body_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
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

/// Версия [`thinking_block`] для активного стрима: тело реактивно склеивает
/// `initial_thinking` (то, что уже закоммичено в `msg.thinking`) с
/// `chat.streaming_thinking.get()` — incrementальным хвостом текущего
/// turn'а. Если суммарный thinking пуст — возвращает zero-size контейнер,
/// чтобы блок не занимал высоту до первого reasoning-токена.
fn streaming_thinking_block(
    msg_idx: usize,
    initial_thinking: String,
    default_open: bool,
) -> impl Widget {
    let header_left = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_PSYCHOLOGY).class("msg-thinking-icon"),
            Text::new("Размышления").class("msg-thinking-title"),
        ]
    };
    let chevron_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let open = chat
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
            header_left,
            DecoratedBox::new().class("grow"),
            chevron_reactive,
        ]
    };
    let header_clickable = GestureDetector::new()
        .on_click(move || {
            let chat = use_context::<AppCtx>().chat.clone();
            chat.thinking_open.update(|m| {
                let cur = m.get(&msg_idx).copied().unwrap_or(default_open);
                m.insert(msg_idx, !cur);
            });
        })
        .child(header);

    let initial_for_body = initial_thinking.clone();
    let body_reactive = move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let tail = chat.streaming_thinking.get();
        let open = chat
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

    // initial может быть пустым в момент рендера, но тело реактивное —
    // блок всё равно добавляем, чтобы при первом reasoning-токене не
    // потребовался ребилд родителя. Header отрисуется сразу.
    let _ = initial_thinking;

    DecoratedBox::new().class("msg-thinking").child(mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            header_clickable,
            body_reactive,
        ]
    })
}

/// Нормализация Markdown-ответа ассистента перед передачей в `MarkdownView`:
/// - `• ` (типографский bullet) → `- ` (CommonMark-bullet), иначе CommonMark
///   не распознаёт список и схлопывает его в параграф с soft-break’ами;
/// - пустые строки **между** соседними пунктами списка удаляются, чтобы
///   CommonMark рендерил список «tight» (плотно) вместо «loose» (каждый
///   пункт в отдельном `<p>` с увеличенным зазором). LLM часто добавляет
///   лишние blank-линии — для UI это выглядит как обрывы.
fn normalize_assistant_markdown(body: &str) -> String {
    let de_bulleted = replace_typographic_bullets(body);
    collapse_blank_lines_between_list_items(&de_bulleted)
}

fn replace_typographic_bullets(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut at_line_start = true;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if at_line_start && c == '\u{2022}' {
            if chars.peek() == Some(&' ') {
                out.push_str("- ");
                chars.next();
                at_line_start = false;
                continue;
            }
        }
        out.push(c);
        at_line_start = c == '\n';
    }
    out
}

fn is_list_item_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    // Unordered: `- `, `* `, `+ ` (CommonMark разрешает также одиночные
    // маркеры без текста, но в нашем контексте всегда есть текст).
    if matches!(bytes[0], b'-' | b'*' | b'+') {
        return bytes.get(1).map(|c| *c == b' ' || *c == b'\t').unwrap_or(false);
    }
    // Ordered: `<digit>+<.|)> <space>`.
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    if i < bytes.len() && matches!(bytes[i], b'.' | b')') {
        return bytes.get(i + 1).map(|c| *c == b' ' || *c == b'\t').unwrap_or(false);
    }
    false
}

fn collapse_blank_lines_between_list_items(body: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            // Ищем, является ли следующая непустая строка пунктом списка
            // и была ли предыдущая непустая строка тоже пунктом списка.
            let prev_is_item = out_lines
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| is_list_item_line(l))
                .unwrap_or(false);
            let next_is_item = lines[i + 1..]
                .iter()
                .find(|l| !l.trim().is_empty())
                .map(|l| is_list_item_line(l))
                .unwrap_or(false);
            if prev_is_item && next_is_item {
                // Пропускаем все blank-линии в этом «зазоре».
                while i < lines.len() && lines[i].trim().is_empty() {
                    i += 1;
                }
                continue;
            }
        }
        out_lines.push(line);
        i += 1;
    }
    out_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typographic_bullets_become_dash() {
        let got = replace_typographic_bullets("• first\n• second");
        assert_eq!(got, "- first\n- second");
    }

    #[test]
    fn blank_lines_between_list_items_are_dropped() {
        let src = "Intro.\n\n- one\n\n- two\n\n- three\n\nEnd.";
        let got = collapse_blank_lines_between_list_items(src);
        // Blank между Intro и списком — сохраняется. Blank между пунктами — нет.
        assert_eq!(got, "Intro.\n\n- one\n- two\n- three\n\nEnd.");
    }

    #[test]
    fn blank_line_before_and_after_list_preserved() {
        let src = "Заголовок\n\n- a\n- b\n\nПодвал";
        let got = collapse_blank_lines_between_list_items(src);
        assert_eq!(got, src);
    }

    #[test]
    fn detects_ordered_items() {
        assert!(is_list_item_line("1. первый"));
        assert!(is_list_item_line("12) двенадцатый"));
        assert!(!is_list_item_line("1-одно"));
        assert!(!is_list_item_line("текст без маркера"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool-call bubble — вызов инструмента от ассистента
// ─────────────────────────────────────────────────────────────────────────────

/// Только сама `.tool-call-card` без обрамляющего avatar/meta-row. Используется
/// и в обычном `tool_call_row` (с аватаром), и из `tool_group::view`, чтобы
/// внутри сворачиваемой группы каждая компактная карточка не дублировала
/// аватар + «Ассистент → вызов инструмента».
pub(super) fn tool_call_card_only(msg: &ChatMsg, tool_name: &str, is_typing: bool, compact: bool) -> impl Widget {
    let tool_icon = Tool::by_key(tool_name)
        .map(|t| t.icon.to_string())
        .unwrap_or_else(|| MI_TERMINAL.to_string());
    let tool_label = Tool::by_key(tool_name)
        .map(|t| t.label.to_string())
        .unwrap_or_else(|| tool_name.to_string());

    let header = mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(tool_icon).class("tool-call-icon"),
            Text::new(tool_label).class("tool-call-name"),
            DecoratedBox::new().class("grow"),
            Text::new(msg.time.clone()).class("msg-time"),
        ]
    };

    // В compact-режиме показываем только заголовок карточки (иконка/имя/время),
    // без секции аргументов. См. `view()` → `tool_mode == "minimal"`.
    let card_children: Vec<Box<dyn Widget>> = if compact {
        vec![Box::new(header)]
    } else {
        let args_text: Box<dyn Widget> = if is_typing && msg.body.trim().is_empty() {
            Box::new(Text::new("•••").class("msg-typing"))
        } else if msg.body.trim().is_empty() {
            Box::new(
                MarkdownView::new("(без аргументов)".to_string())
                    .selectable(true)
                    .with_copy_code(false)
                    .class("tool-call-args"),
            )
        } else {
            // Заворачиваем args_pretty в ```json fence: модель часто
            // пакует в `command` многострочный текст с # comment'ами и
            // **bold**, и MarkdownView без fence интерпретировал бы это
            // как заголовки/жирный. С fence syntect подсвечивает JSON,
            // а внутреннее содержимое — это просто строка-литерал.
            let fenced = format!("```json\n{}\n```", msg.body);
            Box::new(
                MarkdownView::new(fenced)
                    .selectable(true)
                    .with_copy_code(false)
                    .with_syntax_theme("InspiredGitHub")
                    .class("tool-call-args"),
            )
        };
        vec![
            Box::new(header),
            Box::new(DecoratedBox::new().class("tool-call-args-wrap").child(
                Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![args_text])
            )),
        ]
    };

    DecoratedBox::new().class("tool-call-card").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(card_children),
    )
}

pub(super) fn tool_call_row(msg: &ChatMsg, msg_idx: usize, tool_name: &str, is_typing: bool, compact: bool) -> impl Widget {
    let avatar = Avatar::new()
        .text(msg.initials.clone())
        .size(32.0)
        .class(msg.tone_class.clone());

    let card = tool_call_card_only(msg, tool_name, is_typing, compact);

    let author_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(msg.author.clone()).class("msg-author"),
            Text::new("→ вызов инструмента").class("tool-call-hint"),
        ]
    };

    // Regen-кнопка справа от карточки tool-call'а — позволяет перегенерить
    // ответ, когда модель оставила висеть tool-call без последующего text-
    // ответа (например, после Cancel). Скрывается во время `pending=true`
    // и пока сам tool-call ещё стримится (`is_typing`).
    let card_with_actions: Box<dyn Widget> = if !is_typing {
        Box::new(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .main_axis_alignment(MainAxisAlignment::Start)
                .children(vec![
                    Box::new(card) as Box<dyn Widget>,
                    actions_widget(msg_idx, msg.body.clone(), msg.compacted_iter.is_none(), false),
                ]),
        )
    } else {
        Box::new(card)
    };

    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![Box::new(author_row) as Box<dyn Widget>, card_with_actions]);

    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar,
            meta,
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool-result bubble — результат выполнения
// ─────────────────────────────────────────────────────────────────────────────

/// Только сама `.tool-result-card` без обрамляющего avatar/meta-row. Парный
/// helper к [`tool_call_card_only`] — используется в `tool_result_row` (полный
/// макет с плашкой-аватаром «✓») и в `tool_group::view` для компактного
/// рендера внутри развёрнутой группы.
pub(super) fn tool_result_card_only(msg: &ChatMsg, tool_name: &str, error: bool, compact: bool) -> impl Widget {
    let tool_icon = Tool::by_key(tool_name)
        .map(|t| t.icon.to_string())
        .unwrap_or_else(|| MI_TERMINAL.to_string());
    let tool_label = Tool::by_key(tool_name)
        .map(|t| t.label.to_string())
        .unwrap_or_else(|| tool_name.to_string());

    let (card_class, status_icon, status_class) = if error {
        (
            "tool-result-card tool-result-card-error",
            MI_REPORT,
            "tool-result-status-icon tool-result-status-icon-error",
        )
    } else {
        (
            "tool-result-card",
            MI_CHECK,
            "tool-result-status-icon",
        )
    };

    let header = mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(tool_icon).class("tool-result-icon"),
            Text::new(tool_label).class("tool-result-name"),
            DecoratedBox::new().class("grow"),
            Icon::new(status_icon).class(status_class),
            Text::new(msg.time.clone()).class("msg-time"),
        ]
    };

    let body_class = if error {
        "tool-result-body tool-result-body-error"
    } else {
        "tool-result-body"
    };

    let card_children: Vec<Box<dyn Widget>> = if compact {
        vec![Box::new(header)]
    } else {
        let display_body = unescape_persisted_json_newlines(&msg.body);
        vec![
            Box::new(header),
            Box::new(DecoratedBox::new().class("tool-result-body-wrap").child(
                MarkdownView::new(display_body)
                    .selectable(true)
                    .with_copy_code(false)
                    .class(body_class),
            )),
        ]
    };

    DecoratedBox::new().class(card_class).child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(card_children),
    )
}

pub(super) fn tool_result_row(msg: &ChatMsg, msg_idx: usize, tool_name: &str, error: bool, compact: bool) -> impl Widget {
    let card = tool_result_card_only(msg, tool_name, error, compact);

    // Layout идентичен assistant-row (avatar + meta-column): так карточка
    // корректно наследует bounded-width от внешнего Row.Scroll и `max-width:
    // 90%` на карточке реально ограничивает ширину. Плашка «Результат» вместо
    // аватара подсказывает пользователю, что это не ответ ассистента.
    let avatar_placeholder = DecoratedBox::new().class("tool-result-avatar")
        .child(Center::new().child(Icon::new(MI_CHECK).class("tool-result-avatar-icon")));

    // Та же regen-кнопка, что и у tool-call: в случае «застрявшей» цепочки
    // (tool_result без последующего assistant-text) пользователь может
    // перегенерировать ответ от ближайшего предыдущего user-сообщения.
    let card_with_actions: Box<dyn Widget> = Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::End)
            .main_axis_alignment(MainAxisAlignment::Start)
            .children(vec![
                Box::new(card) as Box<dyn Widget>,
                actions_widget(msg_idx, msg.body.clone(), msg.compacted_iter.is_none(), false),
            ]),
    );

    let header_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new("Результат").class("msg-author"),
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

/// Бывшие tool-результаты сохранены как pretty-JSON, где `\n` экранирован
/// как два символа `\` + `n`. Разворачиваем обратно, чтобы старые чаты не
/// отображались одной бесконечной строкой. Новые (plain-text) проходят
/// без изменений, потому что там нет последовательности `\n` из двух
/// ASCII-символов подряд — реальный перевод строки это символ `U+000A`.
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
