//! Подстраница Settings → «Терминал».
//!
//! Управляет шрифтом VTE-терминала на странице «Редактор кода»: семейство и
//! размер. Переиспользует общий компонент [`panel::terminal_font_panel`] —
//! он же показывается в gear-popover самого терминала. Этим обеспечивается
//! единообразие: куда бы пользователь ни зашёл, он видит одни и те же
//! контролы и они синхронно бьют по одним сигналам.

pub mod panel;

use syngui::prelude::*;

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("settings-page terminal-page")
        .child(
            ScrollView::new().vertical().child(
                Padding::all(32.0).child(panel::terminal_font_panel()),
            ),
        )
}
