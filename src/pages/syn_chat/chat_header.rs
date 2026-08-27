//! Верхняя панель Syn-чата — одна строка на всю ширину центра:
//! аватар + название (inline-rename по клику) + имя модели слева, пилюля
//! глобального поиска по центру, действия над чатом справа.
//!
//! Раньше здесь было два ряда: отдельная строка поиска-заглушки (с
//! «Usage and plan») и отдельная шапка чата. Полезного в первом ряду не
//! было ничего, а 70 логических пикселей ленты он забирал — поэтому ряды
//! слиты в один, а поиск стал настоящим (`crate::search`).
//!
//! Кнопка сжатия зовёт `syn_chat::compact::compact_now` (ручной autocompact);
//! disabled, пока идёт генерация или в ленте нет кандидатов на сжатие.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::TextField;

use crate::components::chat_item::tone_for;
use crate::icons::*;
use crate::search;
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
        // Кнопка сжатия: disabled, пока идёт генерация/сжатие ИЛИ в ленте
        // нет кандидатов (find_compact_range = None).
        let pending = ctx.pending.get();
        let msgs = ctx.messages.get();
        let can_compact =
            !pending && crate::syn_chat::compact::find_compact_range(&msgs).is_some();

        let meta = active
            .as_ref()
            .and_then(|id| chats.into_iter().find(|m| &m.id == id));
        match meta {
            Some(meta) => active_header(meta.id, meta.title, can_compact),
            None => empty_header(),
        }
    }
}

/// Ряд без активного чата: подсказка слева, поиск на своём месте — он
/// работает и когда открывать нечего.
fn empty_header() -> StyledWidget<DecoratedBox> {
    let left = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_PSYCHOLOGY).class("chat-header-empty-icon"),
                Text::new(tr!("chat.header.empty_hint")).max_lines(1).class("chat-header-empty-text"),
            ]
    };
    DecoratedBox::new().class("chat-header-empty").child(mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                left,
                search_slot(),
                DecoratedBox::new().class("chat-header-actions-placeholder"),
            ]
    })
}

fn active_header(id: String, title: String, can_compact: bool) -> StyledWidget<DecoratedBox> {
    let tone = tone_for(&id);
    let initials = initials_from_title(&title);

    let avatar = Avatar::new().text(initials).size(30.0).class(tone);
    let title_block = title_block_reactive(title.clone());
    let subtitle = subtitle_reactive();

    let info_col = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(title_block)
        .child(subtitle);

    let identity = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                avatar,
                info_col,
            ]
    };

    let actions_row = mgui! {
        Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            ToolButton::new(MI_COMPRESS)
                .tooltip(tr!("chat.header.compact.tooltip"))
                .disabled(!can_compact)
                .on_click(|| crate::syn_chat::compact::compact_now())
                .class("chat-header-action"),
            ToolButton::new(MI_CLEAR_ALL)
                .tooltip(tr!("chat.header.clear.tooltip"))
                .on_click(|| {
                    let ctx = use_context::<SynChatCtx>();
                    ctx.messages.set(Vec::new());
                    ctx.streaming_body.set(String::new());
                    ctx.streaming_thinking.set(String::new());
                })
                .class("chat-header-action"),
            ToolButton::new(MI_DELETE)
                .tooltip(tr!("chat.header.delete.tooltip"))
                .on_click(move || {
                    // Удаление необратимо — корзина только поднимает диалог
                    // подтверждения (`delete_dialog`), как и в списке чатов.
                    let ctx = use_context::<SynChatCtx>();
                    let meta = ctx.chats.get_untracked().into_iter().find(|m| m.id == id);
                    ctx.pending_delete.set(meta);
                })
                .class("chat-header-action chat-header-action-danger"),
        ]
    };

    DecoratedBox::new()
        .class("chat-header-active")
        .child(mgui! {
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    identity,
                    search_slot(),
                    actions_row,
                ]
        })
}

/// Пилюля поиска по центру свободного места шапки. Обойма растягивается,
/// сама пилюля — нет: ширина у неё своя (`.search-trigger`), а справа
/// остаётся воздух под будущие кнопки.
fn search_slot() -> impl Widget {
    DecoratedBox::new().class("grow chat-header-search").child(
        Row::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(search::trigger::view()),
    )
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
                .child(Text::new(current_title).max_lines(1).class("chat-header-title"));
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
