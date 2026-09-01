//! Поле ввода Syn-чата: полоса вложений + editor + token counter + continue +
//! regen + send/stop + pending-hint. Без mic/kb_chip/tools/audio.
//!
//! Вложения добавляются кнопкой-скрепкой (системный диалог) или
//! перетаскиванием файлов на панель: вся панель обёрнута в [`DropArea`] с
//! `accept_types=["file"]`, winit шлёт по событию на каждый файл.

use std::path::PathBuf;

use syngui::input::DragData;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::MultilineTextEdit;

use crate::icons::*;
use crate::syn_chat::attach;
use crate::syn_chat::state::{ChatMsgKind, ChatMsgRole, SynChatCtx};
use crate::syn_chat::{session, SynModelRegistry};

use super::attachments;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("input-panel-wrap").child(
        DropArea::new()
            .accept_types(vec![DragData::TYPE_FILE.to_string()])
            .on_drop(|data| {
                // winit отдаёт по одному пути на событие; ingest сам
                // отфильтрует каталоги и нечитаемые файлы.
                attach::attach_paths(vec![PathBuf::from(data.payload)]);
            })
            .child(panel_body()),
    )
}

fn panel_body() -> impl Widget {
    DecoratedBox::new().class("input-panel").child(mgui! {
        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            attachments::strip(),
            editor_reactive(),
            DecoratedBox::new().class("input-divider"),
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("input-toolbar-left") => [
                        attach_button(),
                        pending_hint_reactive(),
                    ],
                    Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        token_counter_reactive(),
                        continue_button_reactive(),
                        regen_button_reactive(),
                        send_or_stop_reactive(),
                    ],
                ],
        ]
    })
}

/// Скрепка: открывает системный диалог выбора файлов. Подсказка меняется
/// в зависимости от того, видит ли загруженная модель картинки.
fn attach_button() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let registry = use_context::<SynModelRegistry>();
        let vision = registry
            .current
            .get()
            .map(|m| m.supports_media)
            .unwrap_or(false);
        let tooltip = if vision {
            tr!("chat.input.attach.tooltip_vision")
        } else {
            tr!("chat.input.attach.tooltip_text_only")
        };
        DecoratedBox::new().class("input-attach-wrap").child(
            ToolButton::new(MI_ATTACH_FILE)
                .tooltip(tooltip)
                .on_click(attach::pick_and_attach)
                .class("input-attach"),
        )
    }
}

fn editor_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let _gen = ctx.input_gen.get();
        let initial = ctx.input.get_untracked();
        let editor = MultilineTextEdit::new()
            .text(initial)
            .placeholder(tr!("chat.input.placeholder"))
            .rows(2)
            .max_rows(8)
            .auto_height(true)
            .submit_on_enter(true)
            .on_change(move |s| {
                let ctx = use_context::<SynChatCtx>();
                ctx.input.set_always(s.to_string());
                session::schedule_tokenize();
            })
            .on_submit(move |s| {
                // Пустой текст при наличии вложений — валидная отправка
                // («что на картинке?» можно и не писать), эту проверку
                // делает сам `send_message`.
                session::send_message(s.to_string());
            })
            .class("chat-input-edit");
        DecoratedBox::new().class("chat-input-field").child(editor)
    }
}

fn token_counter_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let n = ctx.input_tokens.get();
        if n == 0 {
            return DecoratedBox::new().class("input-token-chip-empty");
        }
        DecoratedBox::new()
            .class("input-token-chip")
            .child(mgui! {
                Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_BOLT).class("input-token-chip-icon"),
                    Text::new(tr!("chat.input.token_chip", n = n)).class("input-token-chip-text"),
                ]
            })
    }
}

/// Кнопка «Продолжить» — доступна, когда ходу агента есть что дописать:
/// цикл упёрся в `MAX_AGENT_TURNS`, ушёл в reasoning, закончил текстом без
/// единого вызова при активных инструментах (всё это `turn_cap_reached`) —
/// либо пользователь нажал «Прервать» и хвост ленты остался на
/// tool-результате. В отличие от regen историю не режет — цикл продолжает с
/// этого места.
fn continue_button_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let pending = ctx.pending.get();
        let cap_reached = ctx.turn_cap_reached.get();
        let msgs = ctx.messages.get();
        let has_any_user = msgs.iter().any(|m| m.role == ChatMsgRole::User);
        // Ход считаем незакончённым, пока хвост ленты — не непустой
        // текстовый ответ ассистента: tool-call/tool-result или пустой
        // плейсхолдер означают, что цикл оборвался на полпути.
        let tail_unfinished = msgs
            .last()
            .map(|m| {
                m.kind != ChatMsgKind::Text
                    || m.role != ChatMsgRole::Assistant
                    || m.body.is_empty()
            })
            .unwrap_or(false);
        if pending || !has_any_user || !(cap_reached || tail_unfinished) {
            return DecoratedBox::new().class("input-regen-empty");
        }
        let btn = ToolButton::new(MI_PLAY_ARROW)
            .tooltip(tr!("chat.input.continue.tooltip"))
            .on_click(session::continue_last)
            .class("input-regen");
        DecoratedBox::new().class("input-regen-wrap").child(btn)
    }
}

fn regen_button_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let pending = ctx.pending.get();
        let msgs = ctx.messages.get();
        let has_any_user = msgs.iter().any(|m| m.role == ChatMsgRole::User);
        if pending || !has_any_user {
            return DecoratedBox::new().class("input-regen-empty");
        }
        let btn = ToolButton::new(MI_AUTORENEW)
            .tooltip(tr!("chat.regenerate.tooltip"))
            .on_click(session::regenerate_last)
            .class("input-regen");
        DecoratedBox::new().class("input-regen-wrap").child(btn)
    }
}

fn send_or_stop_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let registry = use_context::<SynModelRegistry>();
        let pending = ctx.pending.get();
        let model_loaded = registry.current.get().is_some();

        let inner: Box<dyn Widget> = if pending {
            Box::new(
                ToolButton::new(MI_CLOSE)
                    .tooltip(tr!("chat.input.stop.tooltip"))
                    .on_click(|| session::abort_current())
                    .class("input-send input-send-stop"),
            )
        } else {
            let disabled = !model_loaded;
            let class = if disabled {
                "input-send disabled"
            } else {
                "input-send"
            };
            Box::new(
                ToolButton::new(MI_SEND)
                    .tooltip(tr!("chat.input.send.tooltip"))
                    .on_click(move || {
                        if !model_loaded {
                            return;
                        }
                        let ctx = use_context::<SynChatCtx>();
                        session::send_message(ctx.input.get_untracked());
                    })
                    .class(class),
            )
        };
        DecoratedBox::new()
            .child(Column::new().children(vec![inner]))
            .class("input-send-wrap")
    }
}

fn pending_hint_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let registry = use_context::<SynModelRegistry>();
        let err = ctx.error.get();
        let model_loaded = registry.current.get().is_some();
        let model_loading = registry.loading.get();
        let pending = ctx.pending.get();

        let (txt, class) = if let Some(e) = err.as_ref() {
            (tr!("chat.input.hint.error", error = e), "input-hint error")
        } else if model_loading {
            (tr!("chat.model.status.loading"), "input-hint info")
        } else if !model_loaded {
            (
                tr!("chat.input.hint.pick_model"),
                "input-hint info",
            )
        } else if pending {
            (tr!("chat.input.hint.generating"), "input-hint info")
        } else {
            (String::new(), "input-hint")
        };

        // max_lines — жёсткий потолок высоты подсказки: длинный текст
        // ошибки переносится (ширину даёт `flex-grow` левой группы) и
        // обрезается многоточием, а не растит панель на пол-экрана.
        DecoratedBox::new()
            .child(Text::new(txt).max_lines(3).class("input-hint-text"))
            .class(class)
    }
}
