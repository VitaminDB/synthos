//! Central column: search bar + header + message area + input panel.

use syngui::mgui;
use syngui::prelude::*;

use super::{chat_header, input_panel, message_area, search_bar};

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
