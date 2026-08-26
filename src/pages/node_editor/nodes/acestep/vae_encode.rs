//! `AceStepVaeEncode` — VAE encoder (audio → latent `[B, 64, T_latent]`).
//!
//! Input: `audio: Audio` (AudioBuffer, любая sample_rate / mono или stereo).
//! Output: `latent: Data(Latent)` — `[1, 64, T_latent]` где T_latent =
//! ceil(T_audio_48k / 1920).
//!
//! Конвертация AudioBuffer → Tensor:
//! 1. Resample на 48 kHz через линейную интерполяцию (если нужно).
//! 2. Mono → stereo (duplicate). Stereo берётся как есть.
//! 3. Deinterleaved layout `[L0..LN, R0..RN]` shape `(1, 2, N)` — это
//!    эталонный layout VAE encoder'а (см. `pipeline.rs::encode_audio_file`).
//! 4. `MusicVae::encode_mean` → латент.

use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    AceStepBlob, AceStepLoadedCfg, DataBlob, NodeInstance, NodeRuntime, PortValue,
};
use super::shared::{load_vae, vae_bundle_path};
use super::{
    compute_from_idx, current_input_audio, device_from_idx, standard_body_no_path,
};

const TARGET_SAMPLE_RATE: u32 = 48_000;

pub struct VaeEncodeExec;

impl NodeExecutor for VaeEncodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _audio = ctx.read_input("audio");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AceStepVaeEncode {
                    output_buf,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match output_buf.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(t) => PortValue::Data(Arc::new(DataBlob::AceStep(
                                AceStepBlob::Latent(t.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("latent", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepVaeEncode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepVaeEncode {
                device_idx,
                storage_idx,
                compute_idx,
                running,
                error,
                loaded_name,
                ..
            } => Some((
                *device_idx,
                *storage_idx,
                *compute_idx,
                *running,
                *error,
                *loaded_name,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((device_idx, storage_idx, compute_idx, running, error, loaded_name)) = snapshot
    else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "AceStepVaeEncode"))
                .class("node-card-field-error"),
        );
    };
    standard_body_no_path(
        device_idx,
        storage_idx,
        compute_idx,
        false,
        running,
        error,
        loaded_name,
        "VAE encode…",
        tr!("node.acestep_vae_encode.model_hint"),
        Vec::new(),
    )
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepVaeEncode {
                device_idx,
                storage_idx,
                compute_idx,
                loaded_cfg,
                running,
                error,
                loaded_name,
                output_buf,
                output_version,
                ..
            } => Some((
                *device_idx,
                *storage_idx,
                *compute_idx,
                loaded_cfg.clone(),
                *running,
                *error,
                *loaded_name,
                output_buf.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        device_idx,
        storage_idx,
        compute_idx,
        loaded_cfg,
        running,
        error,
        loaded_name,
        output_buf,
        output_version,
    )) = snapshot
    else {
        return;
    };

    let bundle_path = match vae_bundle_path() {
        Ok(p) => p,
        Err(e) => {
            error.set(Some(e));
            return;
        }
    };
    let audio = match current_input_audio(ctx, node.id, "audio") {
        Some(a) => a,
        None => {
            error.set(Some(tr!("nodes.common.connect_audio_input")));
            return;
        }
    };
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);

    let device_i = device_idx.get_untracked();
    let storage_i = storage_idx.get_untracked();
    let compute_i = compute_idx.get_untracked();
    let cfg = AceStepLoadedCfg {
        model_path: bundle_path.clone(),
        device_idx: device_i,
        storage_idx: storage_i,
        compute_idx: compute_i,
    };

    let _ = thread::Builder::new()
        .name("synthos-acestep-vae-encode".into())
        .spawn(move || {
            vae_encode_worker(
                bundle_path,
                device_i,
                storage_i,
                compute_i,
                cfg,
                audio,
                loaded_cfg,
                running,
                error,
                loaded_name,
                output_buf,
                output_version,
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn vae_encode_worker(
    bundle_path: std::path::PathBuf,
    device_idx: usize,
    storage_idx: usize,
    compute_idx: usize,
    cfg: AceStepLoadedCfg,
    audio: Arc<AudioBuffer>,
    loaded_cfg: Arc<Mutex<Option<AceStepLoadedCfg>>>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_buf: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
    output_version: RwSignal<u32>,
) {
    let vae = match load_vae(&bundle_path, device_idx, storage_idx, compute_idx) {
        Ok(v) => v,
        Err(e) => {
            error.set(Some(tr!("node.acestep_vae_encode.error.load_vae", error = e)));
            running.set(false);
            return;
        }
    };
    if let Ok(mut g) = loaded_cfg.lock() {
        *g = Some(cfg.clone());
    }
    let name = bundle_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| bundle_path.display().to_string());
    loaded_name.set(Some(name));

    let device = device_from_idx(device_idx);
    let dtype = compute_from_idx(compute_idx);

    // 1. AudioBuffer → stereo @ 48 kHz через линейную интерполяцию.
    let stereo_48k = match audio_to_stereo_48k(&audio) {
        Ok(s) => s,
        Err(e) => {
            error.set(Some(format!("Resample: {e}")));
            running.set(false);
            return;
        }
    };
    let n = stereo_48k.len();
    if n == 0 {
        error.set(Some(tr!("node.acestep_vae_encode.error.empty_audio_buffer")));
        running.set(false);
        return;
    }
    // 2. Deinterleave: [L0..LN, R0..RN] для shape (1, 2, N).
    let mut flat: Vec<f32> = Vec::with_capacity(2 * n);
    flat.extend(stereo_48k.iter().map(|(l, _)| *l));
    flat.extend(stereo_48k.iter().map(|(_, r)| *r));
    let tensor = match synaptix_core::tensor::Tensor::from_vec(flat, vec![1, 2, n], device) {
        Ok(t) => t,
        Err(e) => {
            error.set(Some(format!("Tensor::from_vec: {e}")));
            running.set(false);
            return;
        }
    };
    let tensor = match tensor.to_dtype(dtype) {
        Ok(t) => t,
        Err(e) => {
            error.set(Some(format!("to_dtype: {e}")));
            running.set(false);
            return;
        }
    };

    // 3. VAE encode_mean → [1, 64, T_latent].
    match vae.encode_mean(&tensor) {
        Ok(latent) => {
            if let Ok(mut g) = output_buf.lock() {
                *g = Some(latent);
            }
            run_on_main_thread(move || {
                output_version.update(|v| *v = v.wrapping_add(1));
            });
            error.set(None);
        }
        Err(e) => {
            error.set(Some(format!("vae.encode_mean: {e}")));
        }
    }

    running.set(false);
}

/// Конвертация `AudioBuffer` → `Vec<(L, R)>` @ 48 kHz. Mono дублируется
/// в стерео; ресэмпл через линейную интерполяцию.
fn audio_to_stereo_48k(buf: &AudioBuffer) -> std::result::Result<Vec<(f32, f32)>, String> {
    let in_sr = buf.sample_rate;
    let ch = buf.channels.max(1) as usize;
    let frames_in = buf.pcm.len() / ch;
    if frames_in == 0 {
        return Ok(Vec::new());
    }

    // Сначала собираем стерео-исходник.
    let mut stereo_in: Vec<(f32, f32)> = Vec::with_capacity(frames_in);
    for i in 0..frames_in {
        let off = i * ch;
        let l = buf.pcm[off];
        let r = if ch >= 2 { buf.pcm[off + 1] } else { l };
        stereo_in.push((l, r));
    }

    if in_sr == TARGET_SAMPLE_RATE {
        return Ok(stereo_in);
    }
    let ratio = TARGET_SAMPLE_RATE as f64 / in_sr as f64;
    let frames_out = (frames_in as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(frames_out);
    for j in 0..frames_out {
        let src = j as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(frames_in - 1);
        let frac = src - i0 as f64;
        let (l0, r0) = stereo_in[i0];
        let (l1, r1) = stereo_in[i1];
        out.push((
            (l0 as f64 * (1.0 - frac) + l1 as f64 * frac) as f32,
            (r0 as f64 * (1.0 - frac) + r1 as f64 * frac) as f32,
        ));
    }
    Ok(out)
}
