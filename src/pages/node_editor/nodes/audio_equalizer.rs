//! Body + executor для нод-эквалайзеров (6/10/20/30 полос).
//!
//! Каждая полоса — peaking-Biquad с независимым `gain_db` в [-18..+18].
//! Q константный по числу полос (constant-Q дизайн): шире полос — у́же Q.
//! Live-обновление коэффициентов через `Arc<AtomicU32>` + `coeffs_dirty`-CAS.
//!
//! Layout body:
//!   [in_dot] [reset_btn] [<col_band1: gain_label / vSlider / freq_label> ...] [out_dot]
//!
//! Source-агностично: при `PortValue::AudioStream` worker stream'ит chunks
//! поканально через каскад biquad'ов; при `PortValue::Audio(buf)` —
//! offline-обработка с buffer-кэшем (как в Gain/Filter/Reverb).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::{AudioBuffer, AudioStream, Biquad, BiquadMode};
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{Column, DecoratedBox, Reactive, Row, Slider, ToolButton};

use crate::icons::MI_AUTORENEW;

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::registry;
use super::super::types::{NodeInstance, NodeRuntime, PortValue, PrevSource};

const GAIN_RANGE: f32 = 18.0; // ±18 dB

/// Executor для всех 4 EQ-нод. Логика идентична — отличается только n_bands.
pub struct EqualizerExec;

impl NodeExecutor for EqualizerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
            _ => PrevSource::None,
        };

        // Snapshot gains_db для buffer-кэша + n_bands.
        let track = ctx.track;
        let (n_bands_now, gains_now): (usize, Vec<f32>) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Equalizer { n_bands, gains_db, .. } => {
                    let snap: Vec<f32> = gains_db
                        .iter()
                        .map(|sig| if track { sig.get() } else { sig.get_untracked() })
                        .collect();
                    (*n_bands, snap)
                }
                _ => (10, Vec::new()),
            },
            Err(_) => (10, Vec::new()),
        };

        let freqs = registry::equalizer_freqs(n_bands_now);
        let q = registry::equalizer_q(n_bands_now);

        let mut out_pv = PortValue::Empty;
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::Equalizer {
                n_bands,
                live_gains,
                coeffs_dirty,
                biquads,
                last_input_kind,
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

                            // Аллоцируем biquad'ы под фактический channel-count.
                            if let Ok(mut bqs) = biquads.lock() {
                                *bqs = (0..*n_bands)
                                    .map(|_| {
                                        (0..s.channels.max(1) as usize)
                                            .map(|_| Biquad::new())
                                            .collect()
                                    })
                                    .collect();
                            }
                            // Стартуем «грязным» — пересчёт под актуальный sr.
                            coeffs_dirty.store(true, Ordering::Relaxed);

                            let live_gains = live_gains.clone();
                            let dirty = coeffs_dirty.clone();
                            let bqs_arc = biquads.clone();
                            let n = *n_bands;
                            let sr = s.sample_rate;
                            let ch = s.channels.max(1);
                            let h = thread::Builder::new()
                                .name("synthos-eq-worker".into())
                                .spawn(move || {
                                    worker_loop(
                                        rx, tx, live_gains, dirty, bqs_arc, n, ch, sr, freqs, q,
                                    )
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
                        // params для кэша: [n_bands, ...gains_db].
                        let mut params = Vec::with_capacity(1 + gains_now.len());
                        params.push(n_bands_now as f32);
                        params.extend_from_slice(&gains_now);
                        let processed = super::audio_gain::process_buffer_cached(
                            buffer_cache,
                            input_ptr,
                            &params,
                            || process_buffer_eq(buf, n_bands_now, freqs, q, &gains_now),
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

fn worker_loop(
    rx: mpsc::Receiver<Vec<f32>>,
    tx: mpsc::Sender<Vec<f32>>,
    live_gains: Vec<Arc<AtomicU32>>,
    dirty: Arc<AtomicBool>,
    biquads: Arc<Mutex<Vec<Vec<Biquad>>>>,
    n_bands: usize,
    channels: u16,
    sample_rate: u32,
    freqs: &'static [f32],
    q: f32,
) {
    let ch = channels.max(1) as usize;
    while let Ok(mut chunk) = rx.recv() {
        // Пересчёт коэффициентов только при изменении UI (cheap CAS-flag).
        if dirty.swap(false, Ordering::Relaxed) {
            if let Ok(mut bqs) = biquads.lock() {
                for band in 0..n_bands {
                    let gain_db =
                        f32::from_bits(live_gains[band].load(Ordering::Relaxed));
                    let f0 = freqs[band];
                    for c in 0..ch {
                        bqs[band][c].update_coeffs(BiquadMode::Peaking, sample_rate, f0, q, gain_db);
                    }
                }
            }
        }
        if let Ok(mut bqs) = biquads.lock() {
            let frames = chunk.len() / ch;
            for f in 0..frames {
                for c in 0..ch {
                    let idx = f * ch + c;
                    let mut s = chunk[idx];
                    for band in 0..n_bands {
                        s = bqs[band][c].process(s);
                    }
                    chunk[idx] = s;
                }
            }
        }
        if tx.send(chunk).is_err() {
            break;
        }
    }
}

/// Offline-обработка буфера: каскад peaking-Biquad'ов, отдельное state per channel.
fn process_buffer_eq(
    buf: &AudioBuffer,
    n_bands: usize,
    freqs: &[f32],
    q: f32,
    gains_db: &[f32],
) -> Arc<AudioBuffer> {
    let ch = buf.channels.max(1) as usize;
    let frames = buf.pcm.len() / ch;
    let sr = buf.sample_rate.max(1);

    // [band][channel] biquad'ов с предрасчитанными коэффициентами.
    let mut bqs: Vec<Vec<Biquad>> = (0..n_bands)
        .map(|band| {
            let f0 = freqs[band];
            let gain = gains_db.get(band).copied().unwrap_or(0.0);
            (0..ch)
                .map(|_| {
                    let mut b = Biquad::new();
                    b.update_coeffs(BiquadMode::Peaking, sr, f0, q, gain);
                    b
                })
                .collect()
        })
        .collect();

    let mut out: Vec<f32> = Vec::with_capacity(buf.pcm.len());
    for f in 0..frames {
        for c in 0..ch {
            let mut s = buf.pcm[f * ch + c];
            for band in 0..n_bands {
                s = bqs[band][c].process(s);
            }
            out.push(s);
        }
    }
    Arc::new(AudioBuffer::new(
        Arc::from(out.into_boxed_slice()),
        buf.sample_rate,
        buf.channels,
    ))
}

// ──────────────────────────────────────────────────────────────────────────
// UI body
// ──────────────────────────────────────────────────────────────────────────

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (n_bands, gains_db, live_gains, dirty) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Equalizer {
                n_bands,
                gains_db,
                live_gains,
                coeffs_dirty,
                ..
            } => (
                *n_bands,
                gains_db.clone(),
                live_gains.clone(),
                coeffs_dirty.clone(),
            ),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "Equalizer")),
        },
        Err(_) => return error_widget("Equalizer: lock error"),
    };

    let freqs = registry::equalizer_freqs(n_bands);

    // Reset-кнопка: обнуляет все signals + atomics + dirty.
    let reset_gains = gains_db.clone();
    let reset_live = live_gains.clone();
    let reset_dirty = dirty.clone();
    let reset_btn = DecoratedBox::new()
        .child(
            ToolButton::new(MI_AUTORENEW)
                .tooltip(tr!("node.audio_equalizer.reset_bands"))
                .on_click(move || {
                    for sig in &reset_gains {
                        sig.set(0.0);
                    }
                    for live in &reset_live {
                        live.store(0.0_f32.to_bits(), Ordering::Relaxed);
                    }
                    reset_dirty.store(true, Ordering::Relaxed);
                }),
        )
        .class("equalizer-reset-btn");

    // Столбцы по полосам.
    let mut columns: Vec<Box<dyn Widget>> = Vec::with_capacity(n_bands);
    for band in 0..n_bands {
        let gain_sig = gains_db[band];
        let live = live_gains[band].clone();
        let dirty_for_slider = dirty.clone();

        // Текст с текущим gain'ом — реактивный.
        let gain_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let v = gain_sig.get();
            let txt = if v.abs() < 0.05 {
                "0 dB".to_string()
            } else if v > 0.0 {
                format!("+{:.1} dB", v)
            } else {
                format!("{:.1} dB", v)
            };
            vec![Box::new(Text::new(txt).class("equalizer-band-gain-label")) as Box<dyn Widget>]
        });

        let slider = Slider::new()
            .vertical()
            .bipolar()
            .range(-GAIN_RANGE, GAIN_RANGE)
            .step(0.1)
            .value(gain_sig.get_untracked())
            .on_change(move |v| {
                gain_sig.set(v);
                live.store(v.to_bits(), Ordering::Relaxed);
                dirty_for_slider.store(true, Ordering::Relaxed);
            })
            .class("equalizer-band-slider");

        let freq_label = Text::new(format_freq(freqs[band])).class("equalizer-band-freq-label");

        let col = mgui! {
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    DecoratedBox::new().child(gain_label).class("equalizer-band-gain-host"),
                    slider,
                    DecoratedBox::new().child(freq_label).class("equalizer-band-freq-host"),
                ]
        };
        columns.push(Box::new(
            DecoratedBox::new()
                .child(col)
                .class("equalizer-band-column"),
        ));
    }

    let mut row_children: Vec<Box<dyn Widget>> = Vec::with_capacity(n_bands + 2);
    row_children.push(Box::new(reset_btn));
    row_children.extend(columns);

    let row = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(row_children);

    let host_class = format!("equalizer-node-host eq-{n_bands}");
    Box::new(DecoratedBox::new().child(row).class(host_class))
}

fn format_freq(hz: f32) -> String {
    if hz >= 10_000.0 {
        format!("{:.0}k", hz / 1000.0)
    } else if hz >= 1000.0 {
        let kv = hz / 1000.0;
        if (kv - kv.round()).abs() < 0.05 {
            format!("{}k", kv.round() as i32)
        } else {
            format!("{:.1}k", kv)
        }
    } else {
        format!("{:.0}", hz)
    }
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_freq_rounds_octaves() {
        assert_eq!(format_freq(1_000.0), "1k");
        assert_eq!(format_freq(2_000.0), "2k");
        assert_eq!(format_freq(16_000.0), "16k");
        assert_eq!(format_freq(20_000.0), "20k");
        assert_eq!(format_freq(63.0), "63");
        assert_eq!(format_freq(1_250.0), "1.2k");
    }

    #[test]
    fn process_buffer_eq_zero_gains_is_identity() {
        // Все gain'ы = 0 dB — выход ≈ вход (peaking-фильтры с unity gain).
        let pcm: Vec<f32> = (0..480).map(|i| (i as f32 * 0.01).sin()).collect();
        let buf = Arc::new(AudioBuffer::new(
            Arc::from(pcm.clone().into_boxed_slice()),
            48_000,
            1,
        ));
        let freqs = registry::equalizer_freqs(10);
        let gains = vec![0.0_f32; 10];
        let out = process_buffer_eq(&buf, 10, freqs, registry::equalizer_q(10), &gains);
        for (a, b) in out.pcm.iter().zip(pcm.iter()) {
            assert!((a - b).abs() < 1e-3, "0-gain EQ должен быть identity");
        }
    }
}
