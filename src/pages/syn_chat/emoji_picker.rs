//! Панель эмодзи над кнопкой-смайликом в панели ввода чата: ряд недавних
//! и группы из общего набора заметок (`notes::icon_picker::EMOJI_GROUPS`).
//! Клик кладёт эмодзи в очередь вставки поля ввода
//! (`SynChatCtx::input_insert` → `MultilineTextEdit::insert_queue`) — оно
//! встаёт в позицию каретки, не в конец; панель остаётся открытой для
//! следующего, закрывается кликом вне или Esc (встроено в `PopupPanel`).
//! Material-иконок здесь нет: в тексте сообщения они были бы мусорным
//! кодпоинтом для модели.

use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::{GestureDetector, PopupAnchor, PopupPanel, Reactive};

use crate::context::AppCtx;
use crate::pages::notes::icon_picker::EMOJI_GROUPS;
use crate::syn_chat::state::SynChatCtx;

const PANEL_W: f32 = 348.0;
const PANEL_H: f32 = 360.0;
const COLS: usize = 9;
/// Сколько недавних помнить.
pub const RECENT_MAX: usize = 16;

/// Недавние: свежий — первым, без дублей, не длиннее [`RECENT_MAX`].
pub fn push_recent(list: &mut Vec<String>, emoji: &str) {
    list.retain(|e| e != emoji);
    list.insert(0, emoji.to_string());
    list.truncate(RECENT_MAX);
}

/// Вставить эмодзи в поле ввода и запомнить его среди недавних.
pub fn insert(emoji: &str) {
    let ctx = use_context::<SynChatCtx>();
    if let Ok(mut q) = ctx.input_insert.lock() {
        q.push(emoji.to_string());
    }
    ctx.input_insert_gen.update(|v| *v += 1);
    use_context::<AppCtx>()
        .chat_recent_emoji
        .update(|list| push_recent(list, emoji));
}

pub fn view() -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    PopupPanel::new()
        .is_open(ctx.emoji_open)
        .anchor_rect(ctx.emoji_anchor)
        .anchor(PopupAnchor::BottomStart)
        .min_width(PANEL_W)
        .max_width(PANEL_W)
        .max_height(PANEL_H)
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            if !ctx.emoji_open.get() {
                return vec![Box::new(DecoratedBox::new())];
            }
            vec![Box::new(panel())]
        }))
        .class("notes-icon-picker chat-emoji-picker")
}

fn panel() -> impl Widget {
    let recent = use_context::<AppCtx>().chat_recent_emoji.get();
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !recent.is_empty() {
        col = col.child(Text::new(tr!("chat.input.emoji.recent")).class("notes-icon-section"));
        col = col.child(grid(recent));
    }
    for (key, list) in EMOJI_GROUPS {
        col = col.child(Text::new(tr!(key)).class("notes-icon-section"));
        col = col.child(grid(list.iter().map(|s| s.to_string()).collect()));
    }
    let body = Stack::new()
        .clip(false)
        .children(vec![Box::new(DecoratedBox::new().class("notes-icon-grid").child(col)) as Box<dyn Widget>]);
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-icon-picker-body")
        .child(
            DecoratedBox::new()
                .style("height", syngui::mss::StyleValue::px(PANEL_H - 16.0))
                .child(ScrollView::new().vertical().child(body)),
        )
}

fn grid(glyphs: Vec<String>) -> Column {
    let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start);
    for chunk in glyphs.chunks(COLS) {
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for g in chunk {
            let glyph = g.clone();
            let shown = g.clone();
            row = row.child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || insert(&glyph))
                    .child(
                        DecoratedBox::new()
                            .class("notes-icon-cell")
                            .child(Center::new().child(Text::new(shown).class("notes-icon-glyph emoji"))),
                    ),
            );
        }
        col = col.child(row);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_keeps_fresh_first_without_duplicates() {
        let mut list = Vec::new();
        push_recent(&mut list, "😀");
        push_recent(&mut list, "🔥");
        push_recent(&mut list, "😀");
        assert_eq!(list, ["😀", "🔥"]);
        for i in 0..RECENT_MAX + 5 {
            push_recent(&mut list, &format!("e{i}"));
        }
        assert_eq!(list.len(), RECENT_MAX);
        assert_eq!(list[0], format!("e{}", RECENT_MAX + 4));
    }
}
