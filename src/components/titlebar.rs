//! Custom title bar for the frameless window.
//!
//! Left: app name + version (`Synthos v<CARGO_PKG_VERSION>`).
//! Right: window controls — either the built-in Windows-style trio or, when
//! «системные кнопки окна» is on, the buttons of the desktop's decoration
//! theme (on KDE with an Aurorae theme they are drawn from its own SVGs, so
//! they match every other window on screen).
//! The whole strip is a WindowDragRegion so the window follows a left-mouse
//! drag over empty areas; the controls sit on top of the drag region and
//! capture their own clicks.

use syngui::appearance::decorations::{read_system_decorations, SystemDecorations, TitleAlignment};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::{SystemWindowControls, WindowControl, WindowDragRegion};
use syngui::window::WindowState;

use crate::context::AppCtx;
use crate::icons::{MI_CLOSE, MI_CROP_SQUARE, MI_REMOVE};

pub fn view() -> impl Widget {
    WindowDragRegion::new().child(
        DecoratedBox::new().class("titlebar").child(move || {
            let ctx = use_context::<AppCtx>();
            if ctx.appearance.system_window_controls.get() {
                system_bar(ctx.appearance.window_state.get())
            } else {
                builtin_bar()
            }
        }),
    )
}

fn title_text() -> impl Widget {
    Text::new(concat!("Synthos v", env!("CARGO_PKG_VERSION"))).class("titlebar-title")
}

/// Встроенные кнопки — одинаковы на всех платформах.
fn builtin_bar() -> Row {
    mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Start) => [
                title_text(),
                DecoratedBox::new().class("grow"),
                control_button("min", MI_REMOVE, WindowControl::minimize()),
                control_button("zoom", MI_CROP_SQUARE, WindowControl::toggle_maximize()),
                control_button("close", MI_CLOSE, WindowControl::close()),
            ]
    }
}

/// Кнопки из системной темы декораций: раскладка, размеры и внешний вид —
/// как у остальных окон рабочего стола.
fn system_bar(window_state: WindowState) -> Row {
    // Настройки декораций читаются с диска, а титлбар пересобирается на каждый
    // тик реактивного блока — держим один снимок на процесс.
    static DECORATIONS: std::sync::OnceLock<SystemDecorations> = std::sync::OnceLock::new();
    let decorations = DECORATIONS.get_or_init(read_system_decorations).clone();
    let centered = decorations.metrics.title_alignment == TitleAlignment::Center;
    let edge_left = decorations.metrics.edge_left;
    let edge_right = decorations.metrics.edge_right;

    let controls = |side: SystemWindowControls| {
        side.decorations(decorations.clone())
            .maximized(window_state.maximized)
            .active(window_state.focused)
    };

    let mut row = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::Start)
        .child(
            Padding::only(edge_left, 0.0, 0.0, 0.0)
                .child(controls(SystemWindowControls::left())),
        );

    // При центрированном заголовке пустые «распорки» по краям держат текст в
    // середине титлбара, а не в середине оставшегося места.
    if centered {
        row = row
            .child(DecoratedBox::new().class("grow"))
            .child(title_text())
            .child(DecoratedBox::new().class("grow"));
    } else {
        row = row
            .child(Padding::only(12.0, 0.0, 0.0, 0.0).child(title_text()))
            .child(DecoratedBox::new().class("grow"));
    }

    row.child(
        Padding::only(0.0, 0.0, edge_right, 0.0)
            .child(controls(SystemWindowControls::right())),
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
