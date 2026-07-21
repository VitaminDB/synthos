//! Поле ввода Syn-чата: editor + token counter + regen + send/stop +
//! pending-hint. Без attach/mic/kb_chip/tools/audio.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::MultilineTextEdit;

use crate::icons::*;
use crate::syn_chat::state::{ChatMsgRole, SynChatCtx};
use crate::syn_chat::{session, SynModelRegistry};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("input-panel-wrap").child(mgui! {
        DecoratedBox::new().class("input-panel").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                editor_reactive(),
                DecoratedBox::new().class("input-divider"),
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                        pending_hint_reactive(),
                        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            token_counter_reactive(),
                            regen_button_reactive(),
                            send_or_stop_reactive(),
                        ],
                    ],
            ]
        })
    })
}

fn editor_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let _gen = ctx.input_gen.get();
        let initial = ctx.input.get_untracked();
        let editor = MultilineTextEdit::new()
            .text(initial)
            .placeholder("Сообщение для модели…")
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
                let text = s.to_string();
                if !text.trim().is_empty() {
                    session::send_message(text);
                }
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
                    Text::new(format!("{} ток.", n)).class("input-token-chip-text"),
                ]
            })
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
            .tooltip("Сгенерировать заново")
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
                    .tooltip("Прервать")
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
                    .tooltip("Отправить")
                    .on_click(move || {
                        if !model_loaded {
                            return;
                        }
                        let ctx = use_context::<SynChatCtx>();
                        let text = ctx.input.get_untracked();
                        if !text.trim().is_empty() {
                            session::send_message(text);
                        }
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
            (format!("Ошибка: {}", e), "input-hint error")
        } else if model_loading {
            ("Загрузка модели…".to_string(), "input-hint info")
        } else if !model_loaded {
            (
                "Выберите .syn в правой панели".to_string(),
                "input-hint info",
            )
        } else if pending {
            ("Генерация ответа…".to_string(), "input-hint info")
        } else {
            (String::new(), "input-hint")
        };

        DecoratedBox::new()
            .child(Text::new(txt).class("input-hint-text"))
            .class(class)
    }
}
