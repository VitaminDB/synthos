//! Body для NodeKind::Filter — biquad LP/HP/BP над AudioStream.
//!
//! Layout:
//!   [in] [Mode dropdown] [Cutoff slider 20–20000 log] [readout] [out]
//!
//! Cutoff слайдер работает в линейном диапазоне [0..1] и преобразуется
//! в Hz по логарифмической шкале (20 Гц → 20 кГц), чтобы низкие частоты
//! не «прижимались» к левому краю.
//!
//! Live-параметры (mode + cutoff) хранятся в атомиках. Worker-поток читает
//! их перед каждым чанком, при `coeffs_dirty` пересчитывает коэффициенты
//! biquad'а через `Mutex` (короткий лок: только при движении UI-контролов).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::{AudioBuffer, AudioStream, Biquad};
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::input::{Dropdown, DropdownItem};
use syngui::widgets::{DecoratedBox, Reactive, Row, Slider};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{
    FilterMode, NodeInstance, NodeRuntime, PortValue, PrevSource,
};

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// Executor для Filter-ноды.
pub struct FilterExec;

impl NodeExecutor for FilterExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
            _ => PrevSource::None,
        };

        // Snapshot params для buffer-режима.
        let track = ctx.track;
        let (mode_now, cutoff_now) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Filter { mode, cutoff_hz, .. } => {
                    let m = if track { mode.get() } else { mode.get_untracked() };
                    let c = if track { cutoff_hz.get() } else { cutoff_hz.get_untracked() };
                    (m, c)
                }
                _ => (FilterMode::LowPass, 1_000.0),
            },
            Err(_) => (FilterMode::LowPass, 1_000.0),
        };

        let mut out_pv = PortValue::Empty;
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::Filter {
                last_input_kind,
                live_mode,
                live_cutoff,
                coeffs_dirty,
                biquad,
                out_stream,
                worker,
                buffer_cache,
                ..
            } = &mut *g
            {
                if new_kind != *last_input_kind {
                    *last_input_kind = new_kind;
                    *worker = None;
                    *out_stream = None;
                    if let PortValue::AudioStream(s) = &in_pv {
                        if let Some(rx) = s.take_receiver() {
                            let (tx, out_rx) = mpsc::channel::<Vec<f32>>();
                            let stream =
                                AudioStream::from_channel(out_rx, s.sample_rate, s.channels);
                            *out_stream = Some(stream);
                            // Стартуем «грязным» — сразу пересчёт под актуальный sr.
                            coeffs_dirty.store(true, Ordering::Relaxed);
                            let h = thread::Builder::new()
                                .name("synthos-filter-worker".into())
                                .spawn({
                                    let live_mode = live_mode.clone();
                                    let live_cutoff = live_cutoff.clone();
                                    let dirty = coeffs_dirty.clone();
                                    let biquad = biquad.clone();
                                    let sr = s.sample_rate;
                                    move || {
                                        worker_loop(rx, tx, live_mode, live_cutoff, dirty, biquad, sr)
                                    }
                                })
                                .ok();
                            *worker = h;
                        }
                    }
                }

                match &in_pv {
                    PortValue::AudioStream(_) => {
                        if let Some(s) = &out_stream {
                            out_pv = PortValue::AudioStream(s.clone());
                        }
                    }
                    PortValue::Audio(buf) => {
                        let input_ptr = Arc::as_ptr(buf) as usize;
                        let params = vec![mode_now.as_index() as f32, cutoff_now];
                        let processed = super::audio_gain::process_buffer_cached(
                            buffer_cache,
                            input_ptr,
                            &params,
                            || process_buffer_filter(buf, mode_now, cutoff_now),
                        );
                        out_pv = PortValue::Audio(processed);
                    }
                    _ => {}
                }
            }
        }
        ctx.write_output("out", out_pv);
    }
}

/// Offline filter buffer — отдельный biquad-инстанс на канал.
fn process_buffer_filter(
    buf: &AudioBuffer,
    mode: FilterMode,
    cutoff_hz: f32,
) -> Arc<AudioBuffer> {
    let ch = buf.channels.max(1) as usize;
    let frames = buf.pcm.len() / ch;
    let sr = buf.sample_rate.max(1);

    // Один biquad на канал — независимое state per channel.
    let mut bqs: Vec<Biquad> = (0..ch)
        .map(|_| {
            let mut b = Biquad::new();
            b.update_coeffs(mode.to_biquad(), sr, cutoff_hz, 0.707, 0.0);
            b
        })
        .collect();

    let mut out: Vec<f32> = Vec::with_capacity(buf.pcm.len());
    for f in 0..frames {
        for c in 0..ch {
            let sample = buf.pcm[f * ch + c];
            out.push(bqs[c].process(sample));
        }
    }
    Arc::new(AudioBuffer::new(
        Arc::from(out.into_boxed_slice()),
        buf.sample_rate,
        buf.channels,
    ))
}

fn worker_loop(
    rx: mpsc::Receiver<Vec<f32>>,
    tx: mpsc::Sender<Vec<f32>>,
    live_mode: Arc<AtomicU8>,
    live_cutoff: Arc<AtomicU32>,
    dirty: Arc<AtomicBool>,
    biquad: Arc<Mutex<Biquad>>,
    sample_rate: u32,
) {
    while let Ok(mut chunk) = rx.recv() {
        // Пересчитываем коэффициенты только при изменении UI (cheap CAS-flag).
        if dirty.swap(false, Ordering::Relaxed) {
            let mode = FilterMode::from_index(live_mode.load(Ordering::Relaxed) as usize);
            let cutoff = f32::from_bits(live_cutoff.load(Ordering::Relaxed));
            if let Ok(mut bq) = biquad.lock() {
                bq.update_coeffs(mode.to_biquad(), sample_rate, cutoff, 0.707, 0.0);
            }
        }
        if let Ok(mut bq) = biquad.lock() {
            bq.process_slice(&mut chunk);
        }
        if tx.send(chunk).is_err() {
            break;
        }
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (mode_sig, cutoff_sig, live_mode, live_cutoff, dirty) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Filter {
                mode,
                cutoff_hz,
                live_mode,
                live_cutoff,
                coeffs_dirty,
                ..
            } => (
                *mode,
                *cutoff_hz,
                live_mode.clone(),
                live_cutoff.clone(),
                coeffs_dirty.clone(),
            ),
            _ => return error_widget("Filter: некорректный runtime"),
        },
        Err(_) => return error_widget("Filter: lock error"),
    };

    let live_mode_for_dd = live_mode.clone();
    let dirty_for_dd = dirty.clone();
    let dropdown = Dropdown::new()
        .items(vec![
            DropdownItem::new("lp", "LP"),
            DropdownItem::new("hp", "HP"),
            DropdownItem::new("bp", "BP"),
        ])
        .selected(match mode_sig.get_untracked() {
            FilterMode::LowPass => "lp",
            FilterMode::HighPass => "hp",
            FilterMode::BandPass => "bp",
        })
        .on_change(move |val| {
            let m = match val {
                "lp" => FilterMode::LowPass,
                "hp" => FilterMode::HighPass,
                "bp" => FilterMode::BandPass,
                _ => FilterMode::LowPass,
            };
            mode_sig.set(m);
            live_mode_for_dd.store(m.as_index() as u8, Ordering::Relaxed);
            dirty_for_dd.store(true, Ordering::Relaxed);
        })
        .class("filter-node-dropdown");

    // Slider в нормированном [0..1], cutoff = MIN * (MAX/MIN)^t.
    let live_cut_for_slider = live_cutoff.clone();
    let dirty_for_slider = dirty.clone();
    let slider = Slider::new()
        .range(0.0, 1.0)
        .step(0.001)
        .value(hz_to_norm(cutoff_sig.get_untracked()))
        .on_change(move |t| {
            let hz = norm_to_hz(t);
            cutoff_sig.set(hz);
            live_cut_for_slider.store(hz.to_bits(), Ordering::Relaxed);
            dirty_for_slider.store(true, Ordering::Relaxed);
        })
        .class("filter-node-slider");

    let readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let hz = cutoff_sig.get();
        let txt = if hz >= 1000.0 {
            format!("{:.2} kHz", hz / 1000.0)
        } else {
            format!("{hz:.0} Hz")
        };
        vec![Box::new(Text::new(txt).class("audio-node-timecode")) as Box<dyn Widget>]
    });

    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().child(dropdown).class("filter-node-mode-host"),
                slider,
                DecoratedBox::new().child(readout).class("audio-node-slot-text"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-filter-host"),
    )
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}

fn hz_to_norm(hz: f32) -> f32 {
    let h = hz.clamp(MIN_HZ, MAX_HZ);
    (h.ln() - MIN_HZ.ln()) / (MAX_HZ.ln() - MIN_HZ.ln())
}

fn norm_to_hz(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (MIN_HZ.ln() + t * (MAX_HZ.ln() - MIN_HZ.ln())).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_roundtrip_at_endpoints() {
        assert!((norm_to_hz(0.0) - MIN_HZ).abs() < 0.01);
        assert!((norm_to_hz(1.0) - MAX_HZ).abs() < 1.0);
    }

    #[test]
    fn hz_norm_roundtrip() {
        for hz in [50.0, 200.0, 1_000.0, 5_000.0, 12_000.0] {
            let t = hz_to_norm(hz);
            let back = norm_to_hz(t);
            assert!((back - hz).abs() < 0.5, "{hz} → {t} → {back}");
        }
    }
}
