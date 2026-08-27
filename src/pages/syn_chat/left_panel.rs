//! Левая панель Syn-чата: инструменты и скилы агента.
//!
//! Раньше секции tools/skills были первой вкладкой правой панели, а слева
//! стоял список чатов. Список переехал в нав-рейл, и его место заняли
//! инструменты — как ближайший к вводу «пульт» агента. Заголовок панели
//! ([`header`]) кладёт каркас `workspace_frame` в общую строку заголовков,
//! тело ([`body`]) — под ним.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::panel_header;
use crate::components::right_panel::tools_panel;
use crate::icons::MI_AUTO_AWESOME;

pub fn header() -> impl Widget {
    panel_header::side_title(MI_AUTO_AWESOME, tr!("chat.left.tab.tools"))
}

pub fn body() -> impl Widget {
    DecoratedBox::new().class("right-panel syn-chat-left").child(
        DecoratedBox::new().class("right-panel-body").child(
            ScrollView::new().vertical().child(mgui! {
                Column::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        tools_panel::tools_section(),
                        tools_panel::skills_section(),
                    ]
            }),
        ),
    )
}
