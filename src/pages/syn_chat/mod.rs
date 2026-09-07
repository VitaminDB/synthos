//! Маршрут `syn_chat` — чат с in-process Qwen3.6 inference через
//! [`crate::syn_chat`] (без llama-server).
//!
//! Компоновка — общий трёхпанельный каркас ([`workspace_frame`]): у каждой
//! колонки свой заголовок одной высоты. Инструменты и скилы слева, лента +
//! ввод в центре (заголовок — [`chat_header`]: аватар, название, поиск,
//! действия), параметры модели и метрики справа. Списка чатов на странице
//! больше нет — чаты живут плитками в нав-рейле (`crate::rail`).
//!
//! ```text
//! Stack [
//!   workspace_frame [
//!     SplitView (left_split_ratio) [
//!       [▤ Инструменты      ] [аватар · название · поиск · действия] [Параметры|Детали ▥]
//!       left_panel::body      chat_pane                              right_panel::body
//!     ]
//!   ]
//!   tool_confirm                           // Portal с диалогом подтверждения
//!   media_viewer                           // Portal полноэкранного просмотра
//! ]
//! ```
//!
//! Положения разделителей живут в `SynChatCtx.{left,right}_split_ratio`
//! и persist'ятся в `AppConfig.syn_chat_{left,right}_split_ratio`;
//! видимость панелей — `AppCtx.panels.syn_chat` → `AppConfig.panels`.
//! Диалог архива (`archive_dialog`) смонтирован в shell'е — плитку чата
//! закрывают с любой страницы.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::workspace_frame::{self, FrameSpec, Pane};
use crate::context::AppCtx;

pub mod archive_dialog;
pub mod attachments;
pub mod chat_header;
pub mod chat_pane;
pub mod clear_dialog;
pub mod compaction_marker;
pub mod input_panel;
pub mod left_panel;
pub mod media_audio;
pub mod media_inline;
pub mod media_viewer;
pub mod message_area;
pub mod message_bubble;
pub mod prompt_window;
pub mod right_panel;
pub mod tool_group;

pub fn view() -> impl Widget {
    // Авто-загрузки модели тут нет: открытие чата не должно занимать VRAM.
    // Модель поднимает пользователь — кнопкой «Загрузить модель» (последний
    // бандл из `AppConfig.last_syn_model`) или файловым диалогом.
    let app = use_context::<AppCtx>();
    let ctx = use_context::<crate::syn_chat::SynChatCtx>();
    let (left_visible, right_visible) = app.panels.syn_chat;

    let spec = FrameSpec::new(
        "syn-chat-h-split",
        || Box::new(chat_header::center()),
        || Box::new(chat_pane::view()),
    )
    .left(Pane::new(
        left_visible,
        ctx.left_split_ratio,
        200.0,
        || Box::new(left_panel::header()),
        || Box::new(left_panel::body()),
    ))
    .right(Pane::new(
        right_visible,
        ctx.right_split_ratio,
        240.0,
        || Box::new(right_panel::header()),
        || Box::new(right_panel::body()),
    ));
    let frame = workspace_frame::view(spec);

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            frame,
            // Portal-диалог подтверждения tool-call'ов
            // (источник — AppCtx.tools.pending_approval).
            crate::components::tool_confirm::view(),
            // Portal полноэкранного просмотра вложений
            // (источник — SynChatCtx.viewer).
            media_viewer::view(),
            // Плавающее окно редактора системного промпта и диалоги
            // библиотеки пресетов (источник — SynChatCtx.prompt_window_open /
            // prompt_dialog).
            prompt_window::window(),
            prompt_window::dialog(),
        ]
    }
}
