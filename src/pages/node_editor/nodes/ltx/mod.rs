//! Семейство нод **LTX-2.3 22B** (text→video+audio, synaptix). Категория
//! Нейро → подкатегория «LTX Video».
//!
//! Разбиение по нодам (9 шт.):
//!
//! - [`checkpoint`] — хэндл чекпойнта (пути + device/quant/compute) →
//!   `model: Data(Ltx::Model)`. ComfyUI-стиль: один пункт настройки.
//! - [`text_encoder`] — `(model, prompt) → (video_encoding, audio_encoding)`
//!   (Gemma-3-12B + перцивер-коннекторы; Gemma дропается после encode).
//! - [`nag`] — `(model, text) → nag` (NAG negative-prompt той же Gemma).
//! - [`sampler_stage1`] — `(model, v_enc, a_enc, nag?) → (video_latent,
//!   audio_tokens)` (AvDit distilled, 8 шагов, половинная сетка).
//! - [`upscale`] — `(model, video_latent) → video_latent ×2`.
//! - [`sampler_stage2`] — re-noise+refine (3 шага, без NAG).
//! - [`vae_decode`] — `(model, video_latent) → frames` + превью.
//! - [`audio_decode`] — `(model, audio_tokens) → audio` (48 kHz stereo).
//! - [`video_save`] — `(frames, audio?) → mp4` (ffmpeg mux).
//!
//! Подмодели грузятся через [`shared`]: Weak-кэш по ключу
//! [`shared::LtxModelKey`] — Sampler Stage1/Stage2 делят один AvDit,
//! TextEncoder/NAG — одну Gemma (при «держать загруженной»).

pub mod shared;

pub mod a2v;
pub mod audio_decode;
pub mod audio_input;
pub mod checkpoint;
pub mod ic_lora;
pub mod image;
pub mod lipdub;
pub mod nag;
pub mod retake;
pub mod video_input;
pub mod sampler_stage1;
pub mod sampler_stage2;
pub mod text_encoder;
pub mod upscale;
pub mod vae_decode;
pub mod video_save;

use std::path::PathBuf;
use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Reactive, Row, ToolButton};
use synaptix_core::device::Device;
use synaptix_core::dtype::DType;

use crate::icons::MI_FOLDER_OPEN;

use super::super::state::NodeEditorCtx;
use super::super::types::{Connection, LtxModelHandle, LtxVideoLatent, NodeId, PortValue};

/// Доступные устройства synaptix. 0 = CUDA, 1 = CPU (только smoke/отладка —
/// 22B на CPU нереалистичен).
pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];

/// Квант DiT-блоков. nvfp4 ~5.5GB резидентно (быстрейший), mxfp8 ~22.8GB,
/// dense = streaming-offload mmap→VRAM.
pub const QUANT_DIT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (compute)"];

/// Квант Gemma-3-12B. mxfp8 ~12GB влезает в VRAM целиком (дефолт CLI).
pub const QUANT_ENC_OPTIONS: &[&str] = &["mxfp8", "dense (compute)"];

/// Compute dtype активаций.
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f16", "f32"];

/// Подписи поддерживаемых fps (значения — `pipeline::SUPPORTED_FPS`).
pub const FPS_OPTIONS: &[&str] = &["24", "25", "48", "50"];

/// IC-LoRA control-препроцессинг ref-видео.
pub const CONTROL_OPTIONS: &[&str] = &["none", "canny", "depth"];

pub fn device_from_idx(i: usize) -> Device {
    match i {
        1 => Device::Cpu,
        _ => Device::Cuda(0),
    }
}

pub fn compute_from_idx(i: usize) -> DType {
    match COMPUTE_OPTIONS.get(i).copied() {
        Some("f16") => DType::F16,
        Some("f32") => DType::F32,
        _ => DType::BF16,
    }
}

pub fn quant_dit_from_idx(i: usize, compute: DType) -> DType {
    match QUANT_DIT_OPTIONS.get(i).copied() {
        Some("nvfp4") => DType::NVFP4,
        Some("mxfp8") => DType::MXFP8,
        _ => compute,
    }
}

pub fn quant_enc_from_idx(i: usize, compute: DType) -> DType {
    match QUANT_ENC_OPTIONS.get(i).copied() {
        Some("mxfp8") => DType::MXFP8,
        _ => compute,
    }
}

pub fn fps_from_idx(i: usize) -> f64 {
    synaptix_video_ltx23::pipeline::SUPPORTED_FPS
        .get(i)
        .copied()
        .unwrap_or(synaptix_video_ltx23::pipeline::SUPPORTED_FPS[0])
}

// ── Input port helpers ────────────────────────────────────────────────────

fn current_input(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<PortValue> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    values.get(&(src.from_node, src.from_port)).cloned()
}

pub fn current_input_text(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<String> {
    match current_input(ctx, node_id, port)? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

pub fn current_input_model(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<LtxModelHandle>> {
    current_input(ctx, node_id, port)?.as_ltx_model()
}

pub fn current_input_video_encoding(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    current_input(ctx, node_id, port)?.as_ltx_video_encoding()
}

pub fn current_input_audio_encoding(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    current_input(ctx, node_id, port)?.as_ltx_audio_encoding()
}

pub fn current_input_nag(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<(synaptix_core::tensor::Tensor, f32, f32, f32)> {
    current_input(ctx, node_id, port)?.as_ltx_nag()
}

pub fn current_input_video_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<LtxVideoLatent> {
    current_input(ctx, node_id, port)?.as_ltx_video_latent()
}

pub fn current_input_audio_tokens(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    current_input(ctx, node_id, port)?.as_ltx_audio_tokens()
}

pub fn current_input_frames(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<crate::pages::node_editor::types::LtxFrames>> {
    current_input(ctx, node_id, port)?.as_ltx_frames()
}

pub fn current_input_audio(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<syngui::audio::AudioBuffer>> {
    current_input(ctx, node_id, port)?.as_audio()
}

pub fn current_input_image_cond(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<(synaptix_core::tensor::Tensor, f32, usize)> {
    current_input(ctx, node_id, port)?.as_ltx_image_cond()
}

pub fn current_input_video_input(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<std::path::PathBuf> {
    current_input(ctx, node_id, port)?.as_ltx_video_input()
}

pub fn current_input_audio_input(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<std::path::PathBuf> {
    current_input(ctx, node_id, port)?.as_ltx_audio_input()
}

// ── UI helpers ────────────────────────────────────────────────────────────

/// Picker директории (Gemma-3-12B — каталог HF-модели, не файл).
pub fn dir_picker_row(
    tooltip: impl Into<String>,
    sig: RwSignal<Option<PathBuf>>,
) -> Box<dyn Widget> {
    let tooltip = tooltip.into();
    let tooltip_for_dialog = tooltip.clone();
    let pick_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tooltip)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(&tooltip_for_dialog);
            if let Some(p) = dlg.pick_folder() {
                sig.set(Some(p));
            }
        })
        .class("node-file-picker-btn");
    let dirname_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match sig.get() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Box::new(Text::new(name).class("node-file-picker-name"))
            }
            None => Box::new(Text::new(tr!("nodes.common.no_folder_selected")).class("node-file-picker-empty")),
        };
        vec![widget]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-file-picker-row")
            .children(vec![Box::new(pick_btn) as Box<dyn Widget>, Box::new(dirname_text)]),
    )
}

pub fn make_fps_dropdown(idx: RwSignal<usize>) -> Box<dyn Widget> {
    super::acestep::make_dropdown(FPS_OPTIONS, idx)
}

/// Кнопка остановки денойза: видна только при running, ставит cancel-флаг —
/// `denoise_av` вернёт `LtxError::Cancelled` на границе шага.
pub fn cancel_button(
    running: RwSignal<bool>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !running.get() {
            return vec![];
        }
        let cancel = cancel.clone();
        vec![
            Box::new(
                ToolButton::new(crate::icons::MI_STOP)
                    .tooltip(tr!("node.ltx.common.stop_generation"))
                    .on_click(move || {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    })
                    .class("ltx-cancel-btn"),
            ) as Box<dyn Widget>,
        ]
    }))
}

/// Оценщик оставшегося времени генерации: EMA-скорость прогресса по
/// событиям изменения `progress_pct`. Первый замер появляется после
/// второго события (интервал «загрузка модели → первый шаг» намеренно
/// отброшен — он не характеризует скорость шага денойза).
#[derive(Default)]
struct EtaState {
    /// Время и pct последнего изменения прогресса.
    last: Option<(std::time::Instant, f32)>,
    /// Сглаженная скорость, прогресс-единиц (0..1) в секунду.
    rate_ema: Option<f32>,
}

impl EtaState {
    fn observe(&mut self, now: std::time::Instant, pct: f32) {
        match self.last {
            None => {
                if pct > 0.0 {
                    self.last = Some((now, pct));
                }
            }
            Some((t0, p0)) => {
                if pct < p0 {
                    // Рестарт генерации — старые замеры невалидны.
                    self.rate_ema = None;
                    self.last = if pct > 0.0 { Some((now, pct)) } else { None };
                } else if pct > p0 {
                    let dt = (now - t0).as_secs_f32();
                    if dt > 0.0 {
                        let inst = (pct - p0) / dt;
                        self.rate_ema = Some(match self.rate_ema {
                            Some(ema) => ema * 0.6 + inst * 0.4,
                            None => inst,
                        });
                    }
                    self.last = Some((now, pct));
                }
            }
        }
    }

    /// Секунд до конца; между шагами убывает по часам (countdown), не
    /// уходя ниже нуля.
    fn remaining_secs(&self, now: std::time::Instant, pct: f32) -> Option<f32> {
        let rate = self.rate_ema?;
        let (t0, _) = self.last?;
        if rate <= f32::EPSILON || pct >= 1.0 {
            return None;
        }
        Some(((1.0 - pct) / rate - (now - t0).as_secs_f32()).max(0.0))
    }
}

/// `95 с` → `"1:35"`, `3723 с` → `"1:02:03"`.
fn fmt_eta(secs: f32) -> String {
    let s = secs.round().max(0.0) as u64;
    if s < 60 {
        tr!("nodes.common.eta_seconds", n = s)
    } else if s < 3600 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    }
}

/// Секундный тик для countdown'а ETA: пока нода работает, бампает сигнал
/// `tick` раз в секунду (Reactive-строка на него подписана), между шагами
/// денойза оценка «тает» по часам, а не замирает до следующего callback'а.
struct EtaTicker {
    running: RwSignal<bool>,
    tick: RwSignal<u64>,
    acc: syngui::core::sync::Mutex<std::time::Duration>,
}

impl super::super::controls::ProgressTicker for EtaTicker {
    fn tick(&self, dt: std::time::Duration) -> bool {
        if !self.running.get_untracked() {
            return false;
        }
        let due = match self.acc.lock() {
            Ok(mut acc) => {
                *acc += dt;
                if *acc >= std::time::Duration::from_secs(1) {
                    *acc = std::time::Duration::ZERO;
                    true
                } else {
                    false
                }
            }
            Err(_) => true,
        };
        if due {
            self.tick.update(|v| *v = v.wrapping_add(1));
        }
        true
    }
}

/// Прогресс-бар ноды: [`syngui::widgets::ProgressBar`] (MSS-класс
/// `.ltx-progress`) + строка «47% · ≈ 5:32» (процент и оценка оставшегося
/// времени). Виден только при running.
pub fn progress_row(running: RwSignal<bool>, progress_pct: RwSignal<f32>) -> Box<dyn Widget> {
    let eta = Arc::new(syngui::core::sync::Mutex::new(EtaState::default()));
    let tick = use_signal(0_u64);
    let ticker = super::super::controls::node_progress_animator(Arc::new(EtaTicker {
        running,
        tick,
        acc: syngui::core::sync::Mutex::new(std::time::Duration::ZERO),
    }));
    let row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !running.get() {
            if let Ok(mut st) = eta.lock() {
                *st = EtaState::default();
            }
            return vec![];
        }
        let _ = tick.get(); // подписка на секундный countdown-тик
        let pct = progress_pct.get().clamp(0.0, 1.0);
        let now = std::time::Instant::now();
        let label = match eta.lock() {
            Ok(mut st) => {
                st.observe(now, pct);
                match st.remaining_secs(now, pct) {
                    Some(rem) => format!("{}% · ≈ {}", (pct * 100.0) as i32, fmt_eta(rem)),
                    None => format!("{}%", (pct * 100.0) as i32),
                }
            }
            Err(_) => format!("{}%", (pct * 100.0) as i32),
        };
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("ltx-progress-row")
                .children(vec![
                    Box::new(syngui::widgets::ProgressBar::with_value(pct).class("ltx-progress"))
                        as Box<dyn Widget>,
                    Box::new(Text::new(label).class("ltx-progress-label")),
                ]),
        ) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("ltx-progress-row")
            .children(vec![ticker, Box::new(row) as Box<dyn Widget>]),
    )
}
