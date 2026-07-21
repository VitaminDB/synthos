//! Центральная панель Syn-чата: search_bar + chat_header + message_area + input_panel.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::search_bar;

use super::{chat_header, input_panel, message_area};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("chat-pane-wrap").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            search_bar::view(),
            chat_header::view(),
            DecoratedBox::new().class("grow").child(message_area::view()),
            input_panel::view(),
        ]
    })
}
