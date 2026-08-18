use syngui::prelude::*;
use syngui::widgets::input::Slider;
use syngui::widgets::Reactive;

/// Slider со встроенным readout'ом (`Slider::show_value`): клик по числу
/// открывает текстовый инлайн-ввод точного значения (снап к step + кламп).
/// Reactive-обёртка — внешние изменения сигнала двигают ползунок.
pub fn node_slider_field(
    sig: RwSignal<f32>,
    min: f32,
    max: f32,
    step: f32,
    decimals: usize,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![Box::new(
            Slider::new()
                .value(sig.get())
                .range(min, max)
                .step(step)
                .show_value(decimals as u8)
                .on_change(move |v| sig.set(v))
                .class("node-input-slider node-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}

pub fn node_int_slider_field(
    sig: RwSignal<u32>,
    min: u32,
    max: u32,
    step: u32,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![Box::new(
            Slider::new()
                .value(sig.get() as f32)
                .range(min as f32, max as f32)
                .step(step as f32)
                .show_value(0)
                .on_change(move |v| sig.set(v.round().max(0.0) as u32))
                .class("node-input-slider node-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}
