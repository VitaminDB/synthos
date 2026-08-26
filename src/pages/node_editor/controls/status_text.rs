use syngui::prelude::*;
use syngui::widgets::Reactive;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    Idle,
    Running(String),
    Error(String),
    Done(String),
}

pub fn node_status_text(state: RwSignal<NodeStatus>) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match state.get() {
            NodeStatus::Idle => Box::new(Text::new("").class("node-status-idle")),
            NodeStatus::Running(msg) => Box::new(Text::new(msg).class("node-status-running")),
            NodeStatus::Error(msg) => {
                Box::new(Text::new(tr!("nodes.status.error_prefix", msg = msg)).class("node-status-error"))
            }
            NodeStatus::Done(msg) => Box::new(Text::new(msg).class("node-status-done")),
        };
        vec![widget]
    }))
}
