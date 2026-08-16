//! Шапка Syn-чата: avatar + title (inline-rename по двойному клику) +
//! subtitle (имя модели) + actions (rename / delete / clear).
//!
//! Visual parity с `components::chat_header`, но без context-progress-bar и
//! без compact-кнопки (autocompact в Syn-чате не реализован).

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::TextField;

use crate::components::chat_item::tone_for;
use crate::icons::*;
use crate::syn_chat::{registry, SynChatCtx, SynModelRegistry};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("chat-header").child(header_body_reactive())
}

fn header_body_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let reg = use_context::<SynModelRegistry>();
        let active = ctx.active_chat_id.get();
        let chats = ctx.chats.get();
        let _model = reg.current.get();
        let _editing = ctx.last_saved_fp.get();

        let Some(id) = active else {
            return empty_header();
        };
        let Some(meta) = chats.into_iter().find(|m| m.id == id) else {
            return empty_header();
        };
        active_header(meta.id, meta.title)
    }
}

fn empty_header() -> StyledWidget<DecoratedBox> {
    DecoratedBox::new().class("chat-header-empty").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Start) => [
                Icon::new(MI_PSYCHOLOGY).class("chat-header-empty-icon"),
                Text::new("Выберите или создайте чат").class("chat-header-empty-text"),
            ]
    })
}

fn active_header(id: String, title: String) -> StyledWidget<DecoratedBox> {
    let tone = tone_for(&id);
    let initials = initials_from_title(&title);

    let avatar = Avatar::new().text(initials).size(34.0).class(tone);
    let title_block = title_block_reactive(title.clone());
    let subtitle = subtitle_reactive();

    let info_col = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(title_block)
        .child(subtitle);

    let actions_row = mgui! {
        Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            ToolButton::new(MI_CLEAR_ALL)
                .tooltip("Очистить ленту")
                .on_click(|| {
                    let ctx = use_context::<SynChatCtx>();
                    ctx.messages.set(Vec::new());
                    ctx.streaming_body.set(String::new());
                    ctx.streaming_thinking.set(String::new());
                })
                .class("chat-header-action"),
            ToolButton::new(MI_DELETE)
                .tooltip("Удалить чат")
                .on_click(move || registry::delete(&id))
                .class("chat-header-action"),
        ]
    };

    DecoratedBox::new()
        .class("chat-header-active")
        .child(mgui! {
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            avatar,
                            info_col,
                        ],
                    actions_row,
                ]
        })
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
                .child(Text::new(current_title).class("chat-header-title"));
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
            "Загрузка модели…".to_string()
        } else if err.is_some() {
            "Ошибка загрузки".to_string()
        } else {
            "Модель не загружена".to_string()
        };
        DecoratedBox::new()
            .child(Text::new(text).max_lines(1).class("chat-header-subtitle"))
            .class("chat-header-subtitle-wrap")
    }
}

fn initials_from_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        return "AI".to_string();
    }
    let mut words = t.split_whitespace();
    let first = words
        .next()
        .and_then(|w| w.chars().next())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let second = words
        .next()
        .and_then(|w| w.chars().next())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    if second.is_empty() {
        t.chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .flat_map(|c| c.to_uppercase())
            .collect()
    } else {
        format!("{first}{second}")
    }
}
