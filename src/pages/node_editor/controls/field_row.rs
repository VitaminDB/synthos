use syngui::prelude::*;
use syngui::widgets::{DecoratedBox, Row};

pub fn node_field_row(label: &str, control: Box<dyn Widget>) -> Box<dyn Widget> {
    let label_cell = DecoratedBox::new()
        .child(Text::new(label).class("node-card-field-label"))
        .class("node-field-label-cell");
    let mut control_cell_inner = DecoratedBox::new();
    control_cell_inner.child = Some(control);
    let control_cell = control_cell_inner.class("node-field-control-cell");
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-field-row")
            .children(vec![
                Box::new(label_cell) as Box<dyn Widget>,
                Box::new(control_cell),
            ]),
    )
}
