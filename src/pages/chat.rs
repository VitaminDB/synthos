//! Маршрут `chat` — текущий дизайн мессенджера.
//!
//! Компоновка: `[chats_column, Expanded(chat_pane), customer_panel]`.
//! Это ровно то, что было в lib.rs до ввода роутера. Левый nav-rail
//! нарисован уровнем выше и не входит в этот маршрут.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::{chat_pane, chats_column, kb_chip, right_panel, tool_confirm};

pub fn view() -> impl Widget {
    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                chats_column::view(),
                DecoratedBox::new().class("grow").child(chat_pane::view()),
                right_panel::view(),
            ],
            // Overlay подтверждения tool-вызова — рендерится поверх страницы
            // через Portal. Stack нужен, чтобы Portal имел общий root и не
            // был «подвешен» внутри Row — иначе overlay-слой не будет иметь
            // полноразмерного backdrop.
            tool_confirm::view(),
            // Popover KB-чипа (multi-select коллекций знаний для чата).
            // Тоже Portal — монтируется на том же уровне, что и tool_confirm,
            // чтобы overlay-Z-order и backdrop работали единообразно.
            kb_chip::popover(),
        ]
    }
}
