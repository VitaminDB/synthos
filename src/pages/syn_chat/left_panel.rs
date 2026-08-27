//! Левая панель Syn-чата: инструменты и скилы агента.
//!
//! Раньше секции tools/skills были первой вкладкой правой панели, а слева
//! стоял список чатов. Список переехал в нав-рейл, и его место заняли
//! инструменты — как ближайший к вводу «пульт» агента. Панель — TabBar с
//! одной вкладкой: каркас готов принять соседние вкладки (например,
//! историю прогонов), когда они появятся.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::right_panel::tools_panel;
use crate::icons::MI_AUTO_AWESOME;

pub fn view() -> impl Widget {
    let tab = use_signal(0_usize);
    let tabbar = TabBar::new()
        .tab(Tab::new(tr!("chat.left.tab.tools"), 0, &tab).icon(MI_AUTO_AWESOME))
        .class("right-panel-tabbar-inner");

    let body = DecoratedBox::new().class("right-panel-body").child(
        ScrollView::new().vertical().child(mgui! {
            Column::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    tools_panel::tools_section(),
                    tools_panel::skills_section(),
                ]
        }),
    );

    DecoratedBox::new()
        .class("right-panel syn-chat-left")
        .child(mgui! {
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("right-panel-tabbar").child(tabbar),
                body,
            ]
        })
}
