//! Маршрут `syn_chat` — чат с in-process Qwen3.6 inference через
//! [`crate::syn_chat`] (без llama-server).
//!
//! Компоновка повторяет `pages::chat`: левая колонка чатов, центральная
//! лента + ввод, правая панель управления моделью и параметрами sampling.

use syngui::mgui;
use syngui::prelude::*;

pub mod chat_header;
pub mod chat_pane;
pub mod chats_column;
pub mod input_panel;
pub mod message_area;
pub mod message_bubble;
pub mod right_panel;

pub fn view() -> impl Widget {
    // Lazy auto-load последней модели — выполняется один раз при первом
    // открытии страницы. См. `syn_chat::model_registry::auto_load_attempted`.
    crate::syn_chat::model_registry::ensure_auto_load_last_model();

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                chats_column::view(),
                DecoratedBox::new().class("grow").child(chat_pane::view()),
                right_panel::view(),
            ],
            // Portal-диалог подтверждения tool-call'ов: общий для llama-чата
            // и syn-чата (источник — AppCtx.tools.pending_approval).
            crate::components::tool_confirm::view(),
        ]
    }
}
