//! Центральная панель Syn-чата: лента + ввод. Шапка — общая для всех
//! страниц (`chat_header` → `components::page_header`), она стоит над
//! всем каркасом, а не над центром.

use syngui::mgui;
use syngui::prelude::*;

use super::{input_panel, message_area};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("chat-pane-wrap").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("grow").child(message_area::view()),
            input_panel::view(),
        ]
    })
}
