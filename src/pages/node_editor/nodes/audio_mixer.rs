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
//! Несовпадающие sample_rate / channel-count: формат задаёт первый
//! источник, остальные приводятся к нему — каналы (моно ↔ N), частота
//! (офлайн — windowed-sinc, поток — линейная интерполяция с состоянием между
//! чанками). Поток сводится через очереди по входам, а не «чанк на чанк»:
//! чанки разной частоты разной длительности, иначе входы расходятся во
//! времени. Если источник остановился (RecvError) — дальше он даёт тишину.

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
                        let mut sources: Vec<StreamSource> = Vec::new();
                        let mut sr_opt: Option<u32> = None;
                        let mut ch_opt: Option<u16> = None;
                        for i in 0..n {
                            if let PortValue::AudioStream(s) = &input_pvs[i] {
                                if let Some(rx) = s.take_receiver() {
                                    if sr_opt.is_none() {
                                        sr_opt = Some(s.sample_rate);
                                        ch_opt = Some(s.channels.max(1));
                                    }
                                    let (sr, ch) = (sr_opt.unwrap_or(s.sample_rate), ch_opt.unwrap_or(1));
                                    sources.push(StreamSource {
                                        rx,
                                        gain_idx: i,
                                        conv: StreamConverter::new(s.sample_rate, s.channels, sr, ch),
                                    });
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
                                    .spawn(move || worker_loop(sources, ch as usize, tx, live))
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
/// приводятся к его формату ([`convert_buffer`]), добиваются нулями или
/// обрезаются. Каждый buf умножается на свой linear-gain.
fn mix_buffers(bufs: &[(usize, Arc<AudioBuffer>, f32)]) -> Arc<AudioBuffer> {
    let (_, first, _) = &bufs[0];
    let sr = first.sample_rate;
    let ch = first.channels.max(1);
    let len = first.pcm.len();
    let mut out = vec![0.0_f32; len];
    for (_, b, gain) in bufs {
        let converted;
        let pcm: &[f32] = if b.sample_rate == sr && b.channels.max(1) == ch {
            &b.pcm
        } else {
            converted = convert_buffer(&b.pcm, b.sample_rate, b.channels, sr, ch);
            &converted
        };
        let n = out.len().min(pcm.len());
        for i in 0..n {
            out[i] += pcm[i] * *gain;
        }
    }
    Arc::new(AudioBuffer::new(
        Arc::from(out.into_boxed_slice()),
        sr,
        ch,
    ))
}

/// Каналы кадра `frame` (src_ch) → `dst_ch`: моно размножается, в моно —
/// среднее, иначе общие каналы как есть, лишние — тишина.
fn map_channels(frame: &[f32], dst_ch: usize, out: &mut Vec<f32>) {
    let src_ch = frame.len();
    if src_ch == dst_ch {
        out.extend_from_slice(frame);
    } else if src_ch == 1 {
        out.extend(std::iter::repeat_n(frame[0], dst_ch));
    } else if dst_ch == 1 {
        out.push(frame.iter().sum::<f32>() / src_ch as f32);
    } else {
        out.extend((0..dst_ch).map(|c| frame.get(c).copied().unwrap_or(0.0)));
    }
}

/// Буфер целиком в формат (`dst_sr`, `dst_ch`): каналы, затем sinc-ресемплинг
/// по каждому каналу.
fn convert_buffer(pcm: &[f32], src_sr: u32, src_ch: u16, dst_sr: u32, dst_ch: u16) -> Vec<f32> {
    let (sc, dc) = (src_ch.max(1) as usize, dst_ch.max(1) as usize);
    let mut mapped = Vec::with_capacity(pcm.len() / sc * dc);
    for frame in pcm.chunks_exact(sc) {
        map_channels(frame, dc, &mut mapped);
    }
    if src_sr == dst_sr || src_sr == 0 || dst_sr == 0 {
        return mapped;
    }
    let planes: Vec<Vec<f32>> = (0..dc)
        .map(|c| {
            let plane: Vec<f32> = mapped.iter().skip(c).step_by(dc).copied().collect();
            synaptix_audio::resample::resample(&plane, src_sr, dst_sr).unwrap_or_default()
        })
        .collect();
    let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
    let mut out = Vec::with_capacity(frames * dc);
    for f in 0..frames {
        for plane in &planes {
            out.push(plane[f]);
        }
    }
    out
}

/// Потоковое приведение чанков к формату выхода: каналы и частота. Частота —
/// линейная интерполяция; последний кадр и дробная позиция переживают
/// границу чанка, поэтому стыков нет.
struct StreamConverter {
    src_ch: usize,
    dst_ch: usize,
    /// Шаг по входу на один выходной кадр (`src_sr / dst_sr`).
    step: f64,
    /// Позиция следующего выходного кадра относительно `prev` (0 — сам `prev`).
    pos: f64,
    /// Последний кадр предыдущего чанка (уже в `dst_ch`).
    prev: Option<Vec<f32>>,
}

impl StreamConverter {
    fn new(src_sr: u32, src_ch: u16, dst_sr: u32, dst_ch: u16) -> Self {
        let step = if src_sr == 0 || dst_sr == 0 { 1.0 } else { src_sr as f64 / dst_sr as f64 };
        Self {
            src_ch: src_ch.max(1) as usize,
            dst_ch: dst_ch.max(1) as usize,
            step,
            pos: 0.0,
            prev: None,
        }
    }

    fn push(&mut self, chunk: &[f32], out: &mut std::collections::VecDeque<f32>) {
        let mut frames = Vec::with_capacity(chunk.len() / self.src_ch * self.dst_ch);
        for frame in chunk.chunks_exact(self.src_ch) {
            map_channels(frame, self.dst_ch, &mut frames);
        }
        let dc = self.dst_ch;
        if self.step == 1.0 {
            out.extend(frames);
            return;
        }
        // Кадры: [prev?] + frames; индекс 0 — prev, если он есть.
        let prev = self.prev.take();
        let offset = usize::from(prev.is_some());
        let total = offset + frames.len() / dc;
        if total == 0 {
            return;
        }
        let frame = |k: usize| -> &[f32] {
            match (&prev, k) {
                (Some(p), 0) => p,
                _ => &frames[(k - offset) * dc..(k - offset + 1) * dc],
            }
        };
        while self.pos + 1.0 < total as f64 {
            let k = self.pos.floor() as usize;
            let t = (self.pos - k as f64) as f32;
            let (a, b) = (frame(k), frame(k + 1));
            for c in 0..dc {
                out.push_back(a[c] + (b[c] - a[c]) * t);
            }
            self.pos += self.step;
        }
        // Последний кадр — опора следующего чанка.
        self.prev = Some(frame(total - 1).to_vec());
        self.pos -= (total - 1) as f64;
    }
}

struct StreamSource {
    rx: mpsc::Receiver<Vec<f32>>,
    /// Индекс входа — какой `live_gains` к нему относится.
    gain_idx: usize,
    conv: StreamConverter,
}

/// Worker-поток: у каждого источника своя очередь сэмплов в формате выхода.
/// Тик: дочитать чанк у тех живых, чья очередь короче всех, и выдать столько
/// кадров, сколько есть у всех живых (у закончившихся — тишина). Так входы с
/// разной частотой/длиной чанка не расходятся во времени. Завершается, когда
/// все источники вернули RecvError и очереди выбраны.
fn worker_loop(
    mut sources: Vec<StreamSource>,
    channels: usize,
    tx: mpsc::Sender<Vec<f32>>,
    live_gains: Vec<Arc<AtomicU32>>,
) {
    use std::collections::VecDeque;
    let ch = channels.max(1);
    let mut queues: Vec<VecDeque<f32>> = (0..sources.len()).map(|_| VecDeque::new()).collect();
    let mut alive: Vec<bool> = vec![true; sources.len()];
    loop {
        let min_alive = (0..sources.len())
            .filter(|&i| alive[i])
            .map(|i| queues[i].len())
            .min();
        if let Some(min_len) = min_alive {
            for i in 0..sources.len() {
                if !alive[i] || queues[i].len() != min_len {
                    continue;
                }
                match sources[i].rx.recv() {
                    Ok(c) => {
                        let src = &mut sources[i];
                        src.conv.push(&c, &mut queues[i]);
                    }
                    Err(_) => alive[i] = false,
                }
            }
        }
        let any_alive = alive.iter().any(|a| *a);
        // Сколько кадров готово: минимум по живым; когда живых нет — всё,
        // что осталось в очередях.
        let ready = if any_alive {
            (0..sources.len()).filter(|&i| alive[i]).map(|i| queues[i].len()).min().unwrap_or(0)
        } else {
            queues.iter().map(VecDeque::len).max().unwrap_or(0)
        };
        let ready = ready / ch * ch;
        if ready > 0 {
            let mut out = vec![0.0_f32; ready];
            for (i, q) in queues.iter_mut().enumerate() {
                let gain = LinearGain::new(f32::from_bits(
                    live_gains[sources[i].gain_idx].load(Ordering::Relaxed),
                ));
                let take = ready.min(q.len());
                for (o, x) in out.iter_mut().zip(q.drain(..take)) {
                    *o += x * gain.linear;
                }
            }
            if tx.send(out).is_err() {
                break;
            }
        }
        if !any_alive {
            break;
        }
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;

    #[test]
    fn offline_mix_resamples_and_upmixes() {
        // Стерео 48 кГц (1 с тишины) + моно 24 кГц константа 0.5.
        let a = Arc::new(AudioBuffer::new(Arc::from(vec![0.0f32; 96_000]), 48_000, 2));
        let b = Arc::new(AudioBuffer::new(Arc::from(vec![0.5f32; 24_000]), 24_000, 1));
        let out = mix_buffers(&[(0, a, 1.0), (1, b, 1.0)]);
        assert_eq!((out.sample_rate, out.channels, out.pcm.len()), (48_000, 2, 96_000));
        // Середина — 0.5 в обоих каналах (раньше буфер выбрасывался → 0).
        assert!((out.pcm[48_000] - 0.5).abs() < 1e-3);
        assert!((out.pcm[48_001] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn stream_converter_keeps_duration_across_chunks() {
        let mut conv = StreamConverter::new(44_100, 1, 48_000, 2);
        let mut q = std::collections::VecDeque::new();
        // 1 с моно 44.1 кГц чанками по 441 → ≈ 48 000 стерео-кадров.
        for _ in 0..100 {
            conv.push(&[0.25f32; 441], &mut q);
        }
        let frames = q.len() / 2;
        assert!((47_990..=48_000).contains(&frames), "{frames}");
        assert!(q.iter().all(|x| (x - 0.25).abs() < 1e-6));
    }

    #[test]
    fn worker_aligns_sources_with_different_rates() {
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();
        // A: 48 кГц моно чанки по 480 (10 мс) × 10; B: 24 кГц чанки по 480 (20 мс) × 5.
        for _ in 0..10 {
            tx_a.send(vec![0.1f32; 480]).unwrap();
        }
        for _ in 0..5 {
            tx_b.send(vec![0.2f32; 480]).unwrap();
        }
        drop((tx_a, tx_b));
        let sources = vec![
            StreamSource { rx: rx_a, gain_idx: 0, conv: StreamConverter::new(48_000, 1, 48_000, 1) },
            StreamSource { rx: rx_b, gain_idx: 1, conv: StreamConverter::new(24_000, 1, 48_000, 1) },
        ];
        let gains = vec![
            Arc::new(AtomicU32::new(1.0f32.to_bits())),
            Arc::new(AtomicU32::new(1.0f32.to_bits())),
        ];
        let (tx, rx) = mpsc::channel();
        worker_loop(sources, 1, tx, gains);
        let out: Vec<f32> = rx.iter().flatten().collect();
        // Оба длятся 100 мс = 4800 кадров; в середине сумма 0.3.
        assert!((4790..=4800).contains(&out.len()), "{}", out.len());
        assert!((out[2400] - 0.3).abs() < 1e-5);
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
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "Mixer")),
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

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}
