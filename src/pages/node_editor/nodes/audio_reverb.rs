//! Body для NodeKind::Reverb — Schroeder-реверб над AudioStream.
//!
//! Layout:
//!   [in] [Mix slider] [mix readout] [Room slider] [room readout] [out]
//!
//! Live-параметры (mix + room) в атомиках, worker-поток применяет их
//! перед каждым чанком (cheap CAS-flag `params_dirty`).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::{AudioBuffer, AudioStream, SchroederReverb};
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{DecoratedBox, Reactive, Row, Slider};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime, PortValue, PrevSource};

/// Executor для Reverb-ноды.
pub struct ReverbExec;

impl NodeExecutor for ReverbExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
            _ => PrevSource::None,
        };

        let track = ctx.track;
        let (mix_now, room_now) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Reverb { mix, room, .. } => {
                    let m = if track { mix.get() } else { mix.get_untracked() };
                    let r = if track { room.get() } else { room.get_untracked() };
                    (m, r)
                }
                _ => (0.3, 0.5),
            },
            Err(_) => (0.3, 0.5),
        };

        let mut out_pv = PortValue::Empty;
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::Reverb {
                last_input_kind,
                live_mix,
                live_room,
                params_dirty,
                reverb,
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
                            if let Ok(mut r) = reverb.lock() {
                                *r = SchroederReverb::new(s.sample_rate);
                            }
                            params_dirty.store(true, Ordering::Relaxed);
                            let h = thread::Builder::new()
                                .name("synthos-reverb-worker".into())
                                .spawn({
                                    let live_mix = live_mix.clone();
                                    let live_room = live_room.clone();
                                    let dirty = params_dirty.clone();
                                    let reverb = reverb.clone();
                                    move || worker_loop(rx, tx, live_mix, live_room, dirty, reverb)
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
                        let params = vec![mix_now, room_now];
                        let processed = super::audio_gain::process_buffer_cached(
                            buffer_cache,
                            input_ptr,
                            &params,
                            || process_buffer_reverb(buf, mix_now, room_now),
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

/// Offline-реверб над буфером. Отдельный реверб-инстанс на канал —
/// чтобы стерео не сливалось.
fn process_buffer_reverb(buf: &AudioBuffer, mix: f32, room: f32) -> Arc<AudioBuffer> {
    let ch = buf.channels.max(1) as usize;
    let frames = buf.pcm.len() / ch;
    let sr = buf.sample_rate.max(1);

    let mut revs: Vec<SchroederReverb> = (0..ch)
        .map(|_| {
            let mut r = SchroederReverb::new(sr);
            r.set_mix(mix);
            r.set_room(room);
            r
        })
        .collect();

    let mut out: Vec<f32> = Vec::with_capacity(buf.pcm.len());
    for f in 0..frames {
        for c in 0..ch {
            let sample = buf.pcm[f * ch + c];
            out.push(revs[c].process(sample));
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
    live_mix: Arc<AtomicU32>,
    live_room: Arc<AtomicU32>,
    dirty: Arc<AtomicBool>,
    reverb: Arc<Mutex<SchroederReverb>>,
) {
    while let Ok(mut chunk) = rx.recv() {
        if dirty.swap(false, Ordering::Relaxed) {
            let mix = f32::from_bits(live_mix.load(Ordering::Relaxed));
            let room = f32::from_bits(live_room.load(Ordering::Relaxed));
            if let Ok(mut r) = reverb.lock() {
                r.set_mix(mix);
                r.set_room(room);
            }
        }
        if let Ok(mut r) = reverb.lock() {
            r.process_slice(&mut chunk);
        }
        if tx.send(chunk).is_err() {
            break;
        }
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (mix_sig, room_sig, live_mix, live_room, dirty) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Reverb {
                mix,
                room,
                live_mix,
                live_room,
                params_dirty,
                ..
            } => (
                *mix,
                *room,
                live_mix.clone(),
                live_room.clone(),
                params_dirty.clone(),
            ),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "Reverb")),
        },
        Err(_) => return error_widget("Reverb: lock error"),
    };

    let live_mix_for_slider = live_mix.clone();
    let dirty_for_mix = dirty.clone();
    let mix_slider = Slider::new()
        .range(0.0, 1.0)
        .step(0.01)
        .value(mix_sig.get_untracked())
        .on_change(move |v| {
            let v = (v * 100.0).round() / 100.0;
            mix_sig.set(v);
            live_mix_for_slider.store(v.to_bits(), Ordering::Relaxed);
            dirty_for_mix.store(true, Ordering::Relaxed);
        })
        .class("reverb-node-slider");

    let live_room_for_slider = live_room.clone();
    let dirty_for_room = dirty.clone();
    let room_slider = Slider::new()
        .range(0.1, 0.95)
        .step(0.01)
        .value(room_sig.get_untracked())
        .on_change(move |v| {
            let v = (v * 100.0).round() / 100.0;
            room_sig.set(v);
            live_room_for_slider.store(v.to_bits(), Ordering::Relaxed);
            dirty_for_room.store(true, Ordering::Relaxed);
        })
        .class("reverb-node-slider");

    let mix_readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = mix_sig.get();
        vec![Box::new(
            Text::new(format!("mix {:.0}%", v * 100.0)).class("audio-node-timecode"),
        ) as Box<dyn Widget>]
    });
    let room_readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = room_sig.get();
        vec![Box::new(
            Text::new(format!("room {:.0}%", v * 100.0)).class("audio-node-timecode"),
        ) as Box<dyn Widget>]
    });

    let row = mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                mix_slider,
                DecoratedBox::new().child(mix_readout).class("audio-node-slot-text"),
                room_slider,
                DecoratedBox::new().child(room_readout).class("audio-node-slot-text"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-reverb-host"),
    )
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}
