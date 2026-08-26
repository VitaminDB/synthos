//! Body для NodeKind::Gain — линейный gain в децибелах над AudioStream.
//!
//! Layout — узкая горизонтальная карточка:
//!   [in] [Slider -24..+24 dB] [readout] [out]
//!
//! Streaming-pipeline: при detection нового AudioStream-источника executor
//! берёт receiver, создаёт собственный канал и спавнит worker-поток
//! `pull → multiply → push`. Output-port публикует свой `Arc<AudioStream>`.
//! Worker завершается естественно при RecvError (Sender upstream'а дропнут).
//!
//! Live-параметр (gain в dB) хранится в `AtomicU32` (f32 bits), UI пишет
//! при изменении slider'а, worker читает sample-by-sample без локов.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::{AudioBuffer, AudioStream, LinearGain};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{DecoratedBox, Reactive, Row, Slider};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{
    EffectBufferCache, NodeInstance, NodeRuntime, PortValue, PrevSource,
};

/// Executor для Gain-ноды.
pub struct GainExec;

impl NodeExecutor for GainExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
            _ => PrevSource::None,
        };

        // Snapshot текущего gain_db ДО lock'а на runtime — для buffer-режима.
        let track = ctx.track;
        let gain_db_now = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Gain { gain_db, .. } => {
                    if track { gain_db.get() } else { gain_db.get_untracked() }
                }
                _ => 0.0,
            },
            Err(_) => 0.0,
        };

        let mut out_pv = PortValue::Empty;
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::Gain {
                last_input_kind,
                live_gain,
                out_stream,
                worker,
                buffer_cache,
                ..
            } = &mut *g
            {
                // Streaming-режим: при detection нового AudioStream создаём worker.
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
                            let live = live_gain.clone();
                            let h = thread::Builder::new()
                                .name("synthos-gain-worker".into())
                                .spawn(move || worker_loop(rx, tx, live))
                                .ok();
                            *worker = h;
                        }
                    }
                }

                // Output: для streaming — публикуем out_stream; для buffer —
                // offline-DSP с кэшированием по (input_ptr, params).
                match &in_pv {
                    PortValue::AudioStream(_) => {
                        if let Some(s) = &out_stream {
                            out_pv = PortValue::AudioStream(s.clone());
                        }
                    }
                    PortValue::Audio(buf) => {
                        let input_ptr = Arc::as_ptr(buf) as usize;
                        let params = vec![gain_db_now];
                        let processed =
                            process_buffer_cached(buffer_cache, input_ptr, &params, || {
                                process_buffer_gain(buf, gain_db_now)
                            });
                        out_pv = PortValue::Audio(processed);
                    }
                    _ => {}
                }
            }
        }
        ctx.write_output("out", out_pv);
    }
}

/// Применить gain к копии PCM. Возвращает новый `Arc<AudioBuffer>` с теми
/// же sample_rate/channels.
fn process_buffer_gain(buf: &AudioBuffer, gain_db: f32) -> Arc<AudioBuffer> {
    let g = LinearGain::from_db(gain_db);
    let mut out: Vec<f32> = buf.pcm.iter().copied().collect();
    g.process_slice(&mut out);
    Arc::new(AudioBuffer::new(
        Arc::from(out.into_boxed_slice()),
        buf.sample_rate,
        buf.channels,
    ))
}

/// Утилитарный кэш-хелпер для всех эффект-нод. Если в кэше есть запись
/// с тем же input_ptr и params — возвращает её, иначе вызывает `compute`,
/// сохраняет результат в кэш и возвращает.
pub(super) fn process_buffer_cached<F: FnOnce() -> Arc<AudioBuffer>>(
    cache: &Arc<syngui::core::sync::Mutex<Option<EffectBufferCache>>>,
    input_ptr: usize,
    params: &[f32],
    compute: F,
) -> Arc<AudioBuffer> {
    if let Ok(mut g) = cache.lock() {
        if let Some(c) = g.as_ref() {
            if c.input_ptr == input_ptr
                && c.params.len() == params.len()
                && c.params
                    .iter()
                    .zip(params.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            {
                return c.output.clone();
            }
        }
        let out = compute();
        *g = Some(EffectBufferCache {
            input_ptr,
            params: params.to_vec(),
            output: out.clone(),
        });
        out
    } else {
        compute()
    }
}

/// Worker-поток: тянет чанки из rx, умножает на live_gain, шлёт в tx.
/// Завершается при RecvError (upstream закрыт) или SendError (downstream
/// отписался).
fn worker_loop(
    rx: mpsc::Receiver<Vec<f32>>,
    tx: mpsc::Sender<Vec<f32>>,
    live: Arc<AtomicU32>,
) {
    while let Ok(mut chunk) = rx.recv() {
        let gain = f32::from_bits(live.load(Ordering::Relaxed));
        for s in chunk.iter_mut() {
            *s *= gain;
        }
        if tx.send(chunk).is_err() {
            break;
        }
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (gain_db_sig, live_gain) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Gain { gain_db, live_gain, .. } => (*gain_db, live_gain.clone()),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "Gain")),
        },
        Err(_) => return error_widget("Gain: lock error"),
    };

    let live_for_slider = live_gain.clone();
    let slider = Slider::new()
        .range(-24.0, 24.0)
        .step(0.1)
        .value(gain_db_sig.get_untracked())
        .on_change(move |db| {
            let db = (db * 10.0).round() / 10.0;
            gain_db_sig.set(db);
            // dB → linear, в bits для AtomicU32.
            let linear = 10.0_f32.powf(db / 20.0);
            live_for_slider.store(linear.to_bits(), Ordering::Relaxed);
        })
        .class("gain-node-slider");

    let readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let db = gain_db_sig.get();
        let txt = if db.abs() < 0.05 {
            "0.0 dB".to_string()
        } else if db > 0.0 {
            format!("+{db:.1} dB")
        } else {
            format!("{db:.1} dB")
        };
        vec![Box::new(Text::new(txt).class("audio-node-timecode")) as Box<dyn Widget>]
    });

    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(crate::icons::MI_GRAPHIC_EQ).class("audio-node-volume-icon"),
                slider,
                DecoratedBox::new().child(readout).class("audio-node-slot-text"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-gain-host"),
    )
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}
