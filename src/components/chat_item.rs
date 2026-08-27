//! Одна карточка чата в левой колонке.
//!
//! Использует ту же MSS-стилистику, что `conversation_item`:
//! `.conversation-item`, `.conv-name`, `.conv-preview`. Разница — данные из
//! реактивной `ChatMeta` (вместо статичного `Contact`) плюс trailing-кнопка
//! удаления, появляющаяся на hover (`.chat-item-trailing-delete`).
//! Удаление также доступно через `ContextMenu` на правый клик.
//!
//! Клик по карточке ловится через `GestureDetector` — родной «контейнер
//! без визуала для ввода» в syngui. `DecoratedBox` сам on_click не даёт,
//! поэтому оборачиваем карточку в детектор.

use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;

use crate::agent::state::ChatMeta;
use crate::icons::*;

/// Рендерит карточку чата. Callback'и select/delete передаются явно —
/// Syn-чат подставляет `syn_chat::registry::{select,delete}`. Используется
/// в Syn-чате с `syn_chat::registry::{select,delete}`.
pub fn row_generic(
    meta: &ChatMeta,
    selected: bool,
    on_select: Arc<dyn Fn(&str) + Send + Sync>,
    on_delete: Arc<dyn Fn(&str) + Send + Sync>,
) -> Box<dyn Widget> {
    let id = meta.id.clone();
    let class = if selected {
        "conversation-item selected"
    } else {
        "conversation-item"
    };

    let initials = initials_from_title(&meta.title);
    let tone = tone_for(&meta.id);

    let meta_col = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(Text::new(display_title(&meta.title)).class("conv-name"))
        .child(Text::new(display_preview(&meta.preview)).class("conv-preview"));

    let avatar = Avatar::new().text(initials).size(36.0).class(tone);

    // Trailing-кнопка удаления. На hover по карточке её opacity → 1
    // (см. `chat_list.mss`). ToolButton перехватывает click раньше
    // родительского `GestureDetector`, так что нажатие на корзину не
    // активирует чат.
    let delete_id = id.clone();
    let on_delete_btn = on_delete.clone();
    let trailing = ToolButton::new(MI_DELETE)
        .tooltip(tr!("chat.item.delete"))
        .on_click(move || on_delete_btn(&delete_id))
        .class("chat-item-trailing-delete");

    let inner_row = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(avatar)
        .child(DecoratedBox::new().class("grow").child(meta_col))
        .child(trailing);

    let card = DecoratedBox::new().class(class).child(inner_row);

    // Клик по карточке → активируем чат.
    let select_id = id.clone();
    let on_select_click = on_select.clone();
    let clickable = GestureDetector::new()
        .on_click(move || on_select_click(&select_id))
        .child(card);

    // Правый клик — через ContextMenu с тем же «Удалить».
    let menu_id = id.clone();
    let on_delete_menu = on_delete.clone();
    Box::new(
        ContextMenu::new()
            .items(vec![MenuItem::new("delete", tr!("chat.item.delete")).icon(MI_DELETE)])
            .on_select(move |action| {
                if action == "delete" {
                    on_delete_menu(&menu_id);
                }
            })
            .child(clickable),
    )
}

pub fn display_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        tr!("chat.item.untitled")
    } else {
        t.to_string()
    }
}

fn display_preview(preview: &str) -> String {
    let p = preview.trim();
    if p.is_empty() {
        return tr!("chat.item.no_messages");
    }
    // Превью узкое (~180px полезной ширины при колонке 300px), а исходный
    // текст часто содержит реальные \n и литералы "\n" (если в чат прилетел
    // JSON tool-call). Без нормализации каждый перенос рвёт колонку на
    // десятки коротких строк. Схлопываем все пробельные последовательности
    // (включая \n, \t, литералы "\\n") в один пробел — Text сам сделает
    // word-wrap, и при `font-size: 13px` 80-символьная строка превью
    // уляжется примерно в 3 строки.
    let mut out = String::with_capacity(p.len());
    let mut prev_ws = false;
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        // Литерал `\n` или `\t` (бэкслэш + буква) → пробел.
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == 'n' || next == 't' || next == 'r' {
                    chars.next();
                    if !prev_ws {
                        out.push(' ');
                        prev_ws = true;
                    }
                    continue;
                }
            }
        }
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

pub fn initials_from_title(title: &str) -> String {
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

/// Тон аватара — стабильный хэш от id, чтобы карточки были разноцветными,
/// но один и тот же чат всегда рендерился в одном цвете между запусками.
pub fn tone_for(id: &str) -> &'static str {
    const TONES: &[&str] = &[
        "avatar-orange",
        "avatar-green",
        "avatar-blue",
        "avatar-violet",
        "avatar-rose",
        "avatar-slate",
    ];
    let mut hash: u32 = 0;
    for b in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    TONES[(hash as usize) % TONES.len()]
}
