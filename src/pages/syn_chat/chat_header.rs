//! Центральный заголовок страницы чата ([`panel_header::center`]): аватар +
//! название (inline-rename по клику) + имя модели слева, пилюля глобального
//! поиска по центру, действия над чатом справа.
//!
//! Кнопка сжатия зовёт `syn_chat::compact::compact_now` (ручной autocompact);
//! disabled, пока идёт генерация или в ленте нет кандидатов на сжатие.
//! Корзина переносит чат в архив (Настройки → Архив) — насовсем удаляют
//! только оттуда.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::TextField;

use crate::components::chat_item::{initials_from_title, tone_for};
use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::expand;
use crate::icons::*;
use crate::syn_chat::{registry, SynChatCtx, SynModelRegistry};

pub fn center() -> impl Widget {
    let identity: Box<dyn Widget> = Box::new(DecoratedBox::new().child(identity_reactive()));
    panel_header::center(
        CenterSpec::new(identity).actions(DecoratedBox::new().child(actions_reactive())),
    )
}

/// Блок идентичности: с активным чатом — аватар/название/модель, без него
/// — подсказка «выберите или создайте чат».
fn identity_reactive() -> impl Fn() -> Stack + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let active = ctx.active_chat_id.get();
        let chats = ctx.chats.get();
        let _ = ctx.last_saved_fp.get();
        let meta = active
            .as_ref()
            .and_then(|id| chats.into_iter().find(|m| &m.id == id));
        match meta {
            Some(meta) => {
                let avatar = Avatar::new()
                    .text(initials_from_title(&meta.title))
                    .size(30.0)
                    .class(tone_for(&meta.id));
                expand(panel_header::identity(
                    avatar,
                    DecoratedBox::new().child(title_block_reactive(meta.title)),
                    DecoratedBox::new().child(subtitle_reactive()),
                ))
            }
            None => expand(panel_header::identity(
                panel_header::icon_bubble(MI_PSYCHOLOGY),
                panel_header::title_text(tr!("chat.header.empty_hint")),
                panel_header::subtitle_text(tr!("chat.header.empty_sub")),
            )),
        }
    }
}

/// Действия: сжать контекст / очистить ленту / в архив. Без активного чата
/// — пустая распорка той же ширины, чтобы пилюля поиска не прыгала.
fn actions_reactive() -> impl Fn() -> Stack + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let Some(id) = ctx.active_chat_id.get() else {
            return expand(Box::new(
                DecoratedBox::new().class("chat-header-actions-placeholder"),
            ));
        };
        // Кнопка сжатия: disabled, пока идёт генерация/сжатие ИЛИ в ленте
        // нет кандидатов (find_compact_range = None).
        let pending = ctx.pending.get();
        let msgs = ctx.messages.get();
        let can_compact =
            !pending && crate::syn_chat::compact::find_compact_range(&msgs).is_some();

        let row = mgui! {
            Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                ToolButton::new(MI_COMPRESS)
                    .tooltip(tr!("chat.header.compact.tooltip"))
                    .disabled(!can_compact)
                    .on_click(|| crate::syn_chat::compact::compact_now())
                    .class("panel-header-action"),
                panel_header::action_button(MI_CLEAR_ALL, tr!("chat.header.clear.tooltip"), || {
                    let ctx = use_context::<SynChatCtx>();
                    ctx.messages.set(Vec::new());
                    ctx.streaming_body.set(String::new());
                    ctx.streaming_thinking.set(String::new());
                }),
                ToolButton::new(MI_ARCHIVE)
                    .tooltip(tr!("chat.header.archive.tooltip"))
                    .on_click(move || {
                        // В архив — только через подтверждение
                        // (`archive_dialog`), как и «Закрыть» плитки.
                        let ctx = use_context::<SynChatCtx>();
                        let meta = ctx.chats.get_untracked().into_iter().find(|m| m.id == id);
                        ctx.pending_archive.set(meta);
                    })
                    .class("panel-header-action page-header-action-danger"),
            ]
        };
        expand(Box::new(row))
    }
}

fn title_block_reactive(initial: String) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let editing_flag = Arc::new(AtomicBool::new(false));
    let editing_flag_view = editing_flag.clone();
    move || {
        let ctx = use_context::<SynChatCtx>();
        let _gen = ctx.last_saved_fp.get(); // подписка для ребилда после rename
        let chats = ctx.chats.get();
        let id = ctx.active_chat_id.get();
        let current_title = id
            .as_ref()
            .and_then(|active| chats.iter().find(|m| &m.id == active).map(|m| m.title.clone()))
            .unwrap_or_else(|| initial.clone());

        if editing_flag_view.load(Ordering::Relaxed) {
            let flag = editing_flag_view.clone();
            let edit = TextField::new()
                .text(current_title.clone())
                .on_submit(move |s| {
                    let t = s.to_string();
                    if !t.trim().is_empty() {
                        registry::rename_active(t);
                    }
                    flag.store(false, Ordering::Relaxed);
                })
                .class("chat-header-title-edit");
            DecoratedBox::new().class("chat-header-title-wrap").child(edit)
        } else {
            let flag = editing_flag_view.clone();
            let clickable = GestureDetector::new()
                .on_click(move || {
                    flag.store(true, Ordering::Relaxed);
                })
                .child(Text::new(current_title).max_lines(1).class("panel-header-title"));
            DecoratedBox::new().class("chat-header-title-wrap").child(clickable)
        }
    }
}

fn subtitle_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let reg = use_context::<SynModelRegistry>();
        let current = reg.current.get();
        let loading = reg.loading.get();
        let err = reg.error.get();

        let text = if let Some(loaded) = current.as_ref() {
            loaded
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "—".to_string())
        } else if loading {
            tr!("chat.model.status.loading")
        } else if err.is_some() {
            tr!("chat.header.subtitle.error")
        } else {
            tr!("chat.model.not_loaded")
        };
        DecoratedBox::new()
            .child(Text::new(text).max_lines(1).class("panel-header-subtitle"))
            .class("chat-header-subtitle-wrap")
    }
}
