//! Body для NodeKind::Mixer — суммирование N AudioStream'ов в один.
//!
//! Layout (для активного N):
//!   Row[
//!     Column[
//!       Row{ icon, "Mix", SpinBox N },
//!       Reactive→Column{
//!         Row{ in_1 dot, label "1", Slider g_1, readout },
//!         Row{ in_2 dot, label "2", Slider g_2, readout },
//!         ...
//!         Row{ in_N dot, label "N", Slider g_N, readout },
//!       },
//!     ],
//!     flex-spacer,
//!     out dot,    ← вертикально центрируется через CrossAxisAlignment::Center
//!   ]
//!
//! Streaming-pipeline: при detection любого изменения «сигнатуры» входов
//! (по active N + per-input PrevSource) executor пересоздаёт worker.
//! Worker делает per-chunk аккумуляцию: тянет с каждого подключённого
//! receiver'а очередной chunk → перевзвешивает по `live_gains[i]` → суммирует
//! в общий out chunk → push.
//!
//! Несовпадающие sample_rate / channel-count: берём параметры первого
//! ненулевого источника, остальные используем как есть (chunk-by-chunk
//! mix). Если источник остановился (RecvError) — он молча пропускается до
//! полного завершения worker'а.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::{AudioBuffer, AudioStream, LinearGain};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::input::SpinBox;
use syngui::widgets::{DecoratedBox, Reactive, Row, Slider};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{
    NodeInstance, NodeRuntime, PortKind, PortSchema, PortValue, PrevSource,
    MIXER_MAX_INPUTS,
};

// ─────────────────────────────────────────────────────────────────────────────
// Static port pool
// ─────────────────────────────────────────────────────────────────────────────

// `&'static str` имена портов (in_1 .. in_16). Хранятся как массив, а не
// генерируются при каждом resolve — это позволяет PortsSpec возвращать
// `&'static [PortSchema]`, а Connection.from_port/to_port остаются `&'static str`.
const fn port_name(i: usize) -> &'static str {
    // const-friendly mapping. Up to 16 inputs.
    match i {
        0 => "in_1",
        1 => "in_2",
        2 => "in_3",
        3 => "in_4",
        4 => "in_5",
        5 => "in_6",
        6 => "in_7",
        7 => "in_8",
        8 => "in_9",
        9 => "in_10",
        10 => "in_11",
        11 => "in_12",
        12 => "in_13",
        13 => "in_14",
        14 => "in_15",
        15 => "in_16",
        // Unreachable — длина пула равна MIXER_MAX_INPUTS.
        _ => "in_overflow",
    }
}

const fn port_label(i: usize) -> &'static str {
    match i {
        0 => "in 1",
        1 => "in 2",
        2 => "in 3",
        3 => "in 4",
        4 => "in 5",
        5 => "in 6",
        6 => "in 7",
        7 => "in 8",
        8 => "in 9",
        9 => "in 10",
        10 => "in 11",
        11 => "in 12",
        12 => "in 13",
        13 => "in 14",
        14 => "in 15",
        15 => "in 16",
        _ => "in N",
    }
}

const fn make_pool() -> [PortSchema; MIXER_MAX_INPUTS] {
    // const-init: 16 schema'ев с готовыми статическими именами.
    let mut out: [PortSchema; MIXER_MAX_INPUTS] = [PortSchema {
        name: "in_1",
        label: "in 1",
        kind: PortKind::Audio,
    }; MIXER_MAX_INPUTS];
    let mut i = 0;
    while i < MIXER_MAX_INPUTS {
        out[i] = PortSchema {
            name: port_name(i),
            label: port_label(i),
            kind: PortKind::Audio,
        };
        i += 1;
    }
    out
}

/// Полный pool из 16 input-схем. Активный под-набор отрезается через
/// [`mixer_inputs`] по текущему `n_inputs` ноды.
pub static MIXER_PORT_SCHEMAS_FULL: [PortSchema; MIXER_MAX_INPUTS] = make_pool();

/// `PortsSpec::Dynamic.runtime` — возвращает первые `n_inputs` портов из пула.
pub fn mixer_inputs(node: &NodeInstance) -> &'static [PortSchema] {
    let n = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Mixer { n_inputs, .. } => n_inputs.get(),
            _ => 2,
        },
        Err(_) => 2,
    };
    let n = n.clamp(2, MIXER_MAX_INPUTS);
    &MIXER_PORT_SCHEMAS_FULL[..n]
}

pub static MIXER_OUTPUT_SCHEMAS: [PortSchema; 1] = [PortSchema {
    name: "out",
    label: "mix",
    kind: PortKind::Audio,
}];

// ─────────────────────────────────────────────────────────────────────────────
// Executor
// ─────────────────────────────────────────────────────────────────────────────

/// Executor для Mixer-ноды.
pub struct MixerExec;

impl NodeExecutor for MixerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        // Снимок n_inputs (для индексации входов и сравнения сигнатуры).
        let n = {
            match ctx.runtime().lock() {
                Ok(g) => match &*g {
                    NodeRuntime::Mixer { n_inputs, .. } => n_inputs.get_untracked(),
                    _ => 2,
                },
                Err(_) => 2,
            }
        }
        .clamp(2, MIXER_MAX_INPUTS);

        // Собираем PortValue по всем активным входам и считаем «сигнатуру».
        let mut input_pvs: Vec<PortValue> = Vec::with_capacity(n);
        let mut new_kinds: [PrevSource; MIXER_MAX_INPUTS] = [PrevSource::None; MIXER_MAX_INPUTS];
        for i in 0..n {
            let pv = ctx.read_input(port_name(i));
            new_kinds[i] = match &pv {
                PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
                PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
                _ => PrevSource::None,
            };
            input_pvs.push(pv);
        }

        let mut out_pv = PortValue::Empty;
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::Mixer {
                last_input_kinds,
                live_gains,
                out_stream,
                worker,
                buffer_cache,
                ..
            } = &mut *g
            {
                // Сменилась хотя бы одна входная сигнатура → пересоздаём worker
                // под streaming-режим. Buffer-режим не нуждается в worker'е.
                let any_stream = input_pvs
                    .iter()
                    .any(|pv| matches!(pv, PortValue::AudioStream(_)));
                let signature_changed = (0..n).any(|i| new_kinds[i] != last_input_kinds[i]);

                if signature_changed {
                    for i in 0..n {
                        last_input_kinds[i] = new_kinds[i];
                    }
                    *worker = None;
                    *out_stream = None;

                    if any_stream {
                        // Соберём (rx, sample_rate, channels) активных стримов.
                        let mut sources: Vec<mpsc::Receiver<Vec<f32>>> = Vec::new();
                        let mut indices: Vec<usize> = Vec::new();
                        let mut sr_opt: Option<u32> = None;
                        let mut ch_opt: Option<u16> = None;
                        for i in 0..n {
                            if let PortValue::AudioStream(s) = &input_pvs[i] {
                                if let Some(rx) = s.take_receiver() {
                                    if sr_opt.is_none() {
                                        sr_opt = Some(s.sample_rate);
                                        ch_opt = Some(s.channels);
                                    } else if sr_opt != Some(s.sample_rate)
                                        || ch_opt != Some(s.channels)
                                    {
                                        log::warn!(
                                            "[mixer] sr/channels mismatch on in_{}: \
                                             expected sr={:?}/ch={:?}, got sr={}/ch={}",
                                            i + 1,
                                            sr_opt,
                                            ch_opt,
                                            s.sample_rate,
                                            s.channels
                                        );
                                    }
                                    sources.push(rx);
                                    indices.push(i);
                                }
                            }
                        }
                        if let (Some(sr), Some(ch)) = (sr_opt, ch_opt) {
                            if !sources.is_empty() {
                                let (tx, out_rx) = mpsc::channel::<Vec<f32>>();
                                let stream = AudioStream::from_channel(out_rx, sr, ch);
                                *out_stream = Some(stream);
                                let live = live_gains.clone();
                                let h = thread::Builder::new()
                                    .name("synthos-mixer-worker".into())
                                    .spawn(move || worker_loop(sources, indices, tx, live))
                                    .ok();
                                *worker = h;
                            }
                        }
                    }
                }

                // Output: streaming-режим публикует out_stream; чисто-buffer
                // режим складывает offline через cache.
                if any_stream {
                    if let Some(s) = &out_stream {
                        out_pv = PortValue::AudioStream(s.clone());
                    }
                } else {
                    let bufs: Vec<(usize, Arc<AudioBuffer>, f32)> = (0..n)
                        .filter_map(|i| match &input_pvs[i] {
                            PortValue::Audio(b) => {
                                let g = f32::from_bits(live_gains[i].load(Ordering::Relaxed));
                                Some((i, b.clone(), g))
                            }
                            _ => None,
                        })
                        .collect();
                    if !bufs.is_empty() {
                        // Cache-key: (∑input_ptr) и линейные gain'ы. Сумма
                        // указателей коллидирует крайне редко, но дополнительно
                        // включаем index — это спасает от случайной коллизии.
                        let key_ptr: usize = bufs
                            .iter()
                            .fold(0usize, |acc, (i, b, _)| acc ^ (Arc::as_ptr(b) as usize) ^ *i);
                        let mut params: Vec<f32> = Vec::with_capacity(n);
                        for i in 0..n {
                            params.push(f32::from_bits(live_gains[i].load(Ordering::Relaxed)));
                        }
                        let processed = super::audio_gain::process_buffer_cached(
                            buffer_cache,
                            key_ptr,
                            &params,
                            || mix_buffers(&bufs),
                        );
                        out_pv = PortValue::Audio(processed);
                    }
                }
            }
        }
        ctx.write_output("out", out_pv);
    }
}

/// Offline-сумма буферов: первый buf задаёт sr/ch/length, остальные
/// добиваются нулями или обрезаются. Каждый buf умножается на свой
/// linear-gain.
fn mix_buffers(bufs: &[(usize, Arc<AudioBuffer>, f32)]) -> Arc<AudioBuffer> {
    let (_, first, _) = &bufs[0];
    let sr = first.sample_rate;
    let ch = first.channels.max(1);
    let len = first.pcm.len();
    let mut out = vec![0.0_f32; len];
    for (_, b, gain) in bufs {
        // Если sr/ch не совпадают — log + пропуск (в тех тестовых сценариях,
        // где все буферы пришли от одного декодера, такого не бывает).
        if b.sample_rate != sr || b.channels != ch {
            log::warn!(
                "[mixer] offline mix: dropping buf with sr={}/ch={} (expected {}/{})",
                b.sample_rate,
                b.channels,
                sr,
                ch
            );
            continue;
        }
        let n = out.len().min(b.pcm.len());
        for i in 0..n {
            out[i] += b.pcm[i] * *gain;
        }
    }
    Arc::new(AudioBuffer::new(
        Arc::from(out.into_boxed_slice()),
        sr,
        ch,
    ))
}

/// Worker-поток: на каждом тике тянет один chunk с КАЖДОГО активного
/// receiver'а (в порядке `indices`), применяет per-input gain и
/// суммирует в общий out chunk → push в downstream tx. Завершается, когда
/// все sources вернули RecvError.
fn worker_loop(
    mut sources: Vec<mpsc::Receiver<Vec<f32>>>,
    indices: Vec<usize>,
    tx: mpsc::Sender<Vec<f32>>,
    live_gains: Vec<Arc<AtomicU32>>,
) {
    // По умолчанию все источники активны. Если в какой-то итерации источник
    // вернёт RecvError — выкидываем его из дальнейшей обработки, продолжаем
    // микшировать остальные.
    let mut alive: Vec<bool> = vec![true; sources.len()];
    loop {
        let mut chunks: Vec<Option<Vec<f32>>> = (0..sources.len()).map(|_| None).collect();
        let mut any_alive = false;
        for (i, rx) in sources.iter_mut().enumerate() {
            if !alive[i] {
                continue;
            }
            match rx.recv() {
                Ok(c) => {
                    chunks[i] = Some(c);
                    any_alive = true;
                }
                Err(_) => {
                    alive[i] = false;
                }
            }
        }
        if !any_alive {
            break;
        }

        // Длина итогового chunk'а — максимум по всем входным (короткие
        // добиваются нулями неявно, через `for i in 0..len_i`).
        let max_len = chunks
            .iter()
            .filter_map(|c| c.as_ref().map(Vec::len))
            .max()
            .unwrap_or(0);
        if max_len == 0 {
            continue;
        }

        let mut out = vec![0.0_f32; max_len];
        for (slot_idx, chunk_opt) in chunks.iter().enumerate() {
            let Some(chunk) = chunk_opt else { continue };
            let live_idx = indices[slot_idx];
            let gain = LinearGain::new(f32::from_bits(
                live_gains[live_idx].load(Ordering::Relaxed),
            ));
            let n = chunk.len().min(max_len);
            for i in 0..n {
                out[i] += chunk[i] * gain.linear;
            }
        }
        if tx.send(out).is_err() {
            break;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Body widget
// ─────────────────────────────────────────────────────────────────────────────

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();
    let n_sig = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Mixer { n_inputs, .. } => *n_inputs,
            _ => return error_widget("Mixer: некорректный runtime"),
        },
        Err(_) => return error_widget("Mixer: lock error"),
    };

    let n_for_spin = n_sig;
    let spin = SpinBox::new()
        .min(2.0)
        .max(MIXER_MAX_INPUTS as f64)
        .step(1.0)
        .decimal_places(0)
        .value(n_sig.get_untracked() as f64)
        .width(64.0)
        .on_change(move |v| {
            let v = (v.round() as usize).clamp(2, MIXER_MAX_INPUTS);
            if v != n_for_spin.get_untracked() {
                n_for_spin.set(v);
            }
        })
        .class("mixer-input-count");

    let header = mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(crate::icons::MI_LIBRARY_MUSIC).class("audio-node-volume-icon"),
                Text::new("Mix").class("audio-node-timecode"),
                spin,
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(header)
            .class("audio-node-body-host audio-mixer-host"),
    )
}

pub fn port_row_extra(node: &NodeInstance, idx: usize) -> Box<dyn Widget> {
    let (gains, live_gains) = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Mixer { gains_db, live_gains, .. } => {
                (gains_db.clone(), live_gains.clone())
            }
            _ => return Box::new(DecoratedBox::new()),
        },
        Err(_) => return Box::new(DecoratedBox::new()),
    };
    if idx >= gains.len() {
        return Box::new(DecoratedBox::new());
    }
    let gain_sig = gains[idx];
    let live = live_gains[idx].clone();
    let live_for_slider = live.clone();
    let slider = Slider::new()
        .range(-24.0, 12.0)
        .step(0.1)
        .value(gain_sig.get_untracked())
        .on_change(move |db| {
            let db = (db * 10.0).round() / 10.0;
            gain_sig.set(db);
            let linear = 10.0_f32.powf(db / 20.0);
            live_for_slider.store(linear.to_bits(), Ordering::Relaxed);
        })
        .class("mixer-channel-slider");
    let readout = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let db = gain_sig.get();
        let txt = if db.abs() < 0.05 {
            "0.0 dB".to_string()
        } else if db > 0.0 {
            format!("+{db:.1} dB")
        } else {
            format!("{db:.1} dB")
        };
        vec![Box::new(Text::new(txt).class("audio-node-timecode")) as Box<dyn Widget>]
    });
    Box::new(mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(format!("{}", idx + 1)).class("mixer-channel-index"),
                slider,
                DecoratedBox::new().child(readout).class("audio-node-slot-text"),
            ]
    })
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}
