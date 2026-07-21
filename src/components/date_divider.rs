use syngui::mgui;
use syngui::prelude::*;

pub fn view(label: impl Into<String>) -> impl Widget {
    let label = label.into();
    mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("grow").child(DecoratedBox::new().class("date-divider-line")),
            Text::new(label).class("date-divider-label"),
            DecoratedBox::new().class("grow").child(DecoratedBox::new().class("date-divider-line")),
        ]
    }
}
