//! Маршрут `syn_chat` — чат с in-process Qwen3.6 inference через
//! [`crate::syn_chat`] (без llama-server).
//!
//! Компоновка — три колонки на IDE-шных drag-разделителях: список чатов,
//! центральная лента + ввод, правая панель управления моделью и sampling.
//!
//! ```text
//! Stack [
//!   SplitView Horizontal (left_split_ratio) [
//!     chats_column
//!     SplitView Horizontal (right_split_ratio) [
//!       chat_pane
//!       right_panel
//!     ]
//!   ]
//!   tool_confirm                          // Portal с диалогом подтверждения
//!   media_viewer                          // Portal полноэкранного просмотра
//! ]
//! ```
//!
//! Положения разделителей живут в `SynChatCtx.{left,right}_split_ratio`
//! и persist'ятся в `AppConfig.syn_chat_{left,right}_split_ratio` через
//! `install_config_autosave` — после перезапуска ширины восстанавливаются.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

pub mod attachments;
pub mod chat_header;
pub mod chat_pane;
pub mod chats_column;
pub mod input_panel;
pub mod media_viewer;
pub mod message_area;
pub mod message_bubble;
pub mod right_panel;
pub mod tool_group;

pub fn view() -> impl Widget {
    // Lazy auto-load последней модели — выполняется один раз при первом
    // открытии страницы. См. `syn_chat::model_registry::auto_load_attempted`.
    crate::syn_chat::model_registry::ensure_auto_load_last_model();

    // Разделители читают своё положение из сигналов через `get_untracked`
    // (см. `SplitView::create_element`), а при drag пишут обратно — поэтому
    // подписываться здесь не нужно: страница пересобирается при смене
    // маршрута и подхватывает актуальные ширины.
    let ctx = use_context::<crate::syn_chat::SynChatCtx>();

    let center_with_right = SplitView::new(chat_pane::view(), right_panel::view())
        .class("syn-chat-h-split")
        .direction(SplitDirection::Horizontal)
        .ratio_signal(ctx.right_split_ratio)
        .min_size(240.0)
        .divider_width(6.0);

    let split = SplitView::new(chats_column::view(), center_with_right)
        .class("syn-chat-h-split")
        .direction(SplitDirection::Horizontal)
        .ratio_signal(ctx.left_split_ratio)
        .min_size(200.0)
        .divider_width(6.0);

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            split,
            // Portal-диалог подтверждения tool-call'ов
            // (источник — AppCtx.tools.pending_approval).
            crate::components::tool_confirm::view(),
            // Portal полноэкранного просмотра вложений
            // (источник — SynChatCtx.viewer).
            media_viewer::view(),
        ]
    }
}
