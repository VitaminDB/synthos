//! Custom title bar for the frameless window.
//!
//! Left: app name + version (`Synthos v<CARGO_PKG_VERSION>`).
//! Right: Windows-style minimize / maximize / close controls.
//! The whole strip is a WindowDragRegion so the window follows a left-mouse
//! drag over empty areas; the controls sit on top of the drag region and
//! capture their own clicks.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::{WindowControl, WindowDragRegion};

use crate::icons::{MI_CLOSE, MI_CROP_SQUARE, MI_REMOVE};

pub fn view() -> impl Widget {
    let title = concat!("Synthos v", env!("CARGO_PKG_VERSION"));
    WindowDragRegion::new().child(
        DecoratedBox::new().class("titlebar").child(mgui! {
            Row::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Start) => [
                    Text::new(title).class("titlebar-title"),
                    DecoratedBox::new().class("grow"),
                    control_button("min", MI_REMOVE, WindowControl::minimize()),
                    control_button("zoom", MI_CROP_SQUARE, WindowControl::toggle_maximize()),
                    control_button("close", MI_CLOSE, WindowControl::close()),
                ]
        }),
    )
}

fn control_button(variant: &'static str, icon: &'static str, control: WindowControl) -> impl Widget {
    let class = format!("window-control {}", variant);
    control.child(
        DecoratedBox::new().class(class).child(
            Center::new().child(Icon::new(icon).class("window-control-icon")),
        ),
    )
}
