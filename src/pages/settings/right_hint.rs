//! Правая колонка на вкладках Общие / Темы — подсказка-заглушка.

use syngui::mgui;
use syngui::prelude::*;

use crate::icons::*;

pub fn view() -> impl Widget {
    mgui! {
        Center::new() => [
            Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("settings-right-hint-bubble") => [
                    Center::new().child(Icon::new(MI_LIGHTBULB).class("settings-right-hint-icon")),
                ],
                Text::new(tr!("settings.right_hint.title")).class("settings-right-hint-title"),
                Padding::symmetric(16.0, 0.0).child(
                    Text::new(tr!("settings.right_hint.text"))
                        .class("settings-right-hint-text"),
                ),
            ],
        ]
    }
}
