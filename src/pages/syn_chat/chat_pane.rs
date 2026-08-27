//! Центральная панель Syn-чата: верхняя панель + лента + ввод.
//!
//! Верхняя панель одна: `chat_header` держит и данные чата, и пилюлю
//! глобального поиска. Отдельной строки-заглушки с «Usage and plan» здесь
//! больше нет.

use syngui::mgui;
use syngui::prelude::*;

use super::{chat_header, input_panel, message_area};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("chat-pane-wrap").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            chat_header::view(),
            DecoratedBox::new().class("grow").child(message_area::view()),
            input_panel::view(),
        ]
    })
}
