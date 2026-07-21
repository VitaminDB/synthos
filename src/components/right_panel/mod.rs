//! Правый сайдбар страницы «Чаты» — два настоящих таба.
//!
//! Табы управляются сигналом `AppCtx.right_panel_tab` (общий с остальным
//! контекстом; сохраняет выбранный таб при навигации между маршрутами).
//! Таб 0 — «Ламма контроль» (`llama_control`), таб 1 — «Customer details»
//! (`customer_details`). Переключение — через встроенный `TabBar`.

pub mod details;
pub mod llama_control;
pub mod tools_panel;

use syngui::mgui;
use syngui::prelude::*;

use crate::context::{AppCtx, RIGHT_PANEL_DETAILS, RIGHT_PANEL_LLAMA};
use crate::icons::*;

pub fn view() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let tab = ctx.right_panel_tab;

    let tabbar = TabBar::new()
        .tab(
            Tab::new("Ламма", RIGHT_PANEL_LLAMA, &tab)
                .icon(MI_MEMORY),
        )
        .tab(
            Tab::new("Детали", RIGHT_PANEL_DETAILS, &tab)
                .icon(MI_SPEED),
        )
        .class("right-panel-tabbar-inner");

    let body = DecoratedBox::new().class("right-panel-body").child(move || {
        let child: Box<dyn Widget> = match tab.get() {
            RIGHT_PANEL_DETAILS => Box::new(details::view()),
            _ => Box::new(llama_control::view()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    });

    DecoratedBox::new().class("right-panel").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("right-panel-tabbar").child(tabbar),
            body,
        ]
    })
}
