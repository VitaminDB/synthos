//! Центральная панель Syn-чата: лента + ввод. Заголовок колонки
//! (`chat_header::center`) рисует каркас `components::workspace_frame`.
//!
//! Пока чат оторван в плавающее окно (`float_window`), лента и ввод живут
//! в окне, а здесь — плейсхолдер с «Вернуть»: один экземпляр ленты на
//! приложение.

use syngui::mgui;
use syngui::prelude::*;

use crate::syn_chat::SynChatCtx;

use super::{float_window, input_panel, message_area};

pub fn view() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        if use_context::<SynChatCtx>().chat_detached.get() {
            vec![Box::new(float_window::placeholder())]
        } else {
            vec![Box::new(pane())]
        }
    })
}

/// Лента + ввод — то, что переезжает в плавающее окно.
pub fn pane() -> impl Widget {
    DecoratedBox::new().class("chat-pane-wrap").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("grow").child(message_area::view()),
            input_panel::view(),
        ]
    })
}
