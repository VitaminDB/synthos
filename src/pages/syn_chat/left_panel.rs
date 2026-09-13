//! Левая панель Syn-чата: инструменты, пул autotools и скилы агента.
//!
//! Раньше секции tools/skills были первой вкладкой правой панели, а слева
//! стоял список чатов. Список переехал в нав-рейл, и его место заняли
//! инструменты — как ближайший к вводу «пульт» агента. Заголовок панели
//! ([`header`]) кладёт каркас `workspace_frame` в общую строку заголовков,
//! тело ([`body`]) — под ним. Секции сворачиваются, раскрытие — в
//! `SynChatCtx.cards`.

use syngui::mgui;
use syngui::prelude::*;

use crate::components::panel_header;
use crate::components::right_panel::tools_panel;
use crate::icons::MI_AUTO_AWESOME;
use crate::syn_chat::SynChatCtx;

pub fn header() -> impl Widget {
    panel_header::side_title(MI_AUTO_AWESOME, tr!("chat.left.tab.tools"))
}

pub fn body() -> impl Widget {
    let cards = use_context::<SynChatCtx>().cards;
    DecoratedBox::new().class("right-panel syn-chat-left").child(
        DecoratedBox::new().class("right-panel-body").child(
            ScrollView::new().vertical().child(mgui! {
                Column::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        tools_panel::tools_section(cards.tools),
                        tools_panel::autotools_section(cards.autotools),
                        tools_panel::skills_section(cards.skills),
                    ]
            }),
        ),
    )
}
