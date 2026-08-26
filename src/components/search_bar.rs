//! Search bar across the top of the chat area.

use syngui::mgui;
use syngui::prelude::*;

use crate::icons::*;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("search-bar").child(mgui! {
        Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
            search_field(),
            actions(),
        ]
    })
}

fn search_field() -> impl Widget {
    DecoratedBox::new().class("search-field").child(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_SEARCH).class("search-icon"),
            DecoratedBox::new().class("grow").child(Text::new(tr!("chat.search.placeholder")).class("search-hint")),
            DecoratedBox::new().class("shortcut-chip").child(mgui! {
                Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_KEYBOARD_COMMAND_KEY).class("shortcut-icon"),
                    Text::new("S").class("shortcut-text"),
                ]
            }),
        ]
    })
}

fn actions() -> impl Widget {
    mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_HELP_OUTLINE).class("search-bar-icon"),
            DecoratedBox::new().class("plan-button").child(mgui! {
                Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_AUTORENEW).class("plan-button-icon"),
                    Text::new(tr!("chat.search.plan")).class("plan-button-text"),
                    Icon::new(MI_EXPAND_MORE).class("plan-button-chev"),
                ]
            }),
        ]
    }
}
