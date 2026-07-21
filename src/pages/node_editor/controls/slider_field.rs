use syngui::prelude::*;
use syngui::widgets::input::Slider;
use syngui::widgets::{Reactive, Row};

pub fn node_slider_field(
    sig: RwSignal<f32>,
    min: f32,
    max: f32,
    step: f32,
    decimals: usize,
) -> Box<dyn Widget> {
    let slider = Slider::new()
        .value(sig.get_untracked())
        .range(min, max)
        .step(step)
        .on_change(move |v| sig.set(v))
        .class("node-input-slider node-slider-stretch");
    let readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = sig.get();
        vec![Box::new(Text::new(format!("{v:.*}", decimals)).class("node-slider-readout"))
            as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![Box::new(slider) as Box<dyn Widget>, Box::new(readout)]),
    )
}

pub fn node_int_slider_field(
    sig: RwSignal<u32>,
    min: u32,
    max: u32,
    step: u32,
) -> Box<dyn Widget> {
    let slider = Slider::new()
        .value(sig.get_untracked() as f32)
        .range(min as f32, max as f32)
        .step(step as f32)
        .on_change(move |v| sig.set(v.round().max(0.0) as u32))
        .class("node-input-slider node-slider-stretch");
    let readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = sig.get();
        vec![Box::new(Text::new(v.to_string()).class("node-slider-readout")) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![Box::new(slider) as Box<dyn Widget>, Box::new(readout)]),
    )
}
