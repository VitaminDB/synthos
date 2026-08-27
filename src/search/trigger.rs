//! Пилюля поиска в шапке чата: по клику раскрывает панель поверх себя.
//!
//! Обёртка сообщает свою геометрию в `SearchCtx.trigger_bounds` — по ней
//! панель выравнивается так, чтобы её поле ввода накрыло пилюлю.

use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;

use crate::components::event_hook::EventHook;
use crate::icons::*;

use super::SearchCtx;

pub fn view() -> impl Widget {
    let search = use_context::<SearchCtx>();
    DecoratedBox::new().child(move || {
        let open = search.open.get();
        let class = if open {
            "search-trigger search-trigger-open"
        } else {
            "search-trigger"
        };
        let opener = search.clone();
        EventHook::new()
            .report_bounds(search.trigger_bounds.clone())
            .child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || opener.open(None))
                    .child(
                        DecoratedBox::new().class(class).child(
                            Padding::only(12.0, 0.0, 8.0, 0.0).child(
                                Row::new()
                                    .gap(8.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(MI_SEARCH).class("search-trigger-icon"))
                                    .child(
                                        DecoratedBox::new().class("grow").clip(true).child(
                                            Text::new(tr!("search.trigger"))
                                                .max_lines(1)
                                                .class("search-trigger-text"),
                                        ),
                                    )
                                    .child(
                                        Row::new()
                                            .gap(3.0)
                                            .cross_axis_alignment(CrossAxisAlignment::Center)
                                            .child(kbd("Ctrl"))
                                            .child(kbd("K")),
                                    ),
                            ),
                        ),
                    ),
            )
    })
}

fn kbd(text: &str) -> impl Widget {
    DecoratedBox::new()
        .class("search-kbd")
        .child(Text::new(text).class("search-kbd-text"))
}
