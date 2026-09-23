use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::{Reactive, Row, ToolButton};

use crate::icons::{MI_PAUSE, MI_PLAY_ARROW, MI_STOP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportState {
    Idle,
    Playing,
    Paused,
}

pub fn node_transport_buttons(
    state: RwSignal<TransportState>,
    on_play_pause: impl Fn() + Send + Sync + 'static,
    on_stop: impl Fn() + Send + Sync + 'static,
) -> Box<dyn Widget> {
    let on_play_pause = Arc::new(on_play_pause);
    let play_pause_btn = {
        let on_play_pause = on_play_pause.clone();
        Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let s = state.get();
            let icon = if s == TransportState::Playing { MI_PAUSE } else { MI_PLAY_ARROW };
            let tooltip = if s == TransportState::Playing {
                tr!("nodes.transport.pause")
            } else {
                tr!("nodes.transport.play")
            };
            let class = if s == TransportState::Playing {
                "node-transport-btn node-transport-pause"
            } else {
                "node-transport-btn node-transport-play"
            };
            let on_click = on_play_pause.clone();
            let btn = ToolButton::new(icon)
                .tooltip(tooltip)
                .on_click(move || on_click())
                .class(class);
            vec![Box::new(btn)]
        })
    };

    let stop_btn = ToolButton::new(MI_STOP)
        .tooltip(tr!("nodes.transport.stop"))
        .on_click(on_stop)
        .class("node-transport-btn node-transport-stop");

    Box::new(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-transport-row")
            .children(vec![Box::new(play_pause_btn) as Box<dyn Widget>, Box::new(stop_btn)]),
    )
}
