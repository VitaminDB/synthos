//! `LtxAudioDecode` — `(model, audio_tokens) → audio` (48 kHz stereo).
//! Audio-VAE → log-mel → вокодер BigVGAN-v2 (`decode_audio_tokens`), волна
//! `[1,2,L]` planar → interleaved `AudioBuffer` — играется существующей
//! AudioPlayer-нодой и муксится Save-нодой.

use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_core::device::Device;
use synaptix_video_ltx23::audio_vae::AudioVaeDecoder;
use synaptix_video_ltx23::pipeline::decode_audio_tokens;
use synaptix_video_ltx23::vocoder::VocoderWithBwe;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, status_row};
use super::shared;
use super::{current_input_audio_tokens, current_input_model, device_from_idx};

/// Выходная частота вокодера LTX (BigVGAN-v2 48k).
const VOCODER_SAMPLE_RATE: u32 = 48_000;

pub struct AudioDecodeExec;

impl NodeExecutor for AudioDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("audio_tokens");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxAudioDecode {
                    out,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(buf) => PortValue::Audio(buf.clone()),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("audio", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxAudioDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxAudioDecode { running, error, .. } => Some((*running, *error)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxAudioDecode"))
                .class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![field_row(
        &tr!("nodes.common.status"),
        status_row(
            running,
            error,
            loaded_name,
            tr!("node.ltx_audio_decode.status.busy"),
            "ltx-node-running",
        ),
    )];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxAudioDecode {
                running,
                error,
                out,
                output_version,
            } => Some((*running, *error, out.clone(), *output_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, out, output_version)) = snapshot else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    let Some(a_tok) = current_input_audio_tokens(ctx, node.id, "audio_tokens") else {
        error.set(Some(tr!("node.ltx_audio_decode.err.connect_audio_tokens")));
        return;
    };
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-ltx-audio-decode".into())
        .spawn(move || {
            match worker(&handle, &a_tok) {
                Ok(buf) => {
                    if let Ok(mut g) = out.lock() {
                        *g = Some(Arc::new(buf));
                    }
                    run_on_main_thread(move || {
                        output_version.update(|v| *v = v.wrapping_add(1));
                    });
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

fn worker(handle: &LtxModelHandle, a_tok: &synaptix_core::tensor::Tensor) -> std::result::Result<AudioBuffer, String> {
    let dev = device_from_idx(handle.device_idx);
    let ckpt = shared::load_ckpt(handle)?;
    let audio_vae = AudioVaeDecoder::load(&ckpt, dev).map_err(|e| format!("audio VAE: {e}"))?;
    let vocoder =
        VocoderWithBwe::load(&handle.model_path, dev).map_err(|e| format!("vocoder: {e}"))?;
    let wave = decode_audio_tokens(&audio_vae, &vocoder, a_tok)
        .map_err(|e| format!("audio decode: {e}"))?;
    let dims = wave.dims().to_vec();
    if dims.len() != 3 || dims[1] != 2 {
        return Err(tr!("node.ltx_audio_decode.err.bad_wave_dims", dims = format!("{dims:?}")));
    }
    let len = dims[2];
    let v = wave
        .to_device(Device::Cpu)
        .map_err(|e| format!("wave → CPU: {e}"))?
        .reshape(vec![2 * len])
        .map_err(|e| format!("wave reshape: {e}"))?
        .to_vec1::<f32>()
        .map_err(|e| format!("wave → vec: {e}"))?;
    let mut pcm = vec![0f32; 2 * len];
    for i in 0..len {
        pcm[i * 2] = v[i];
        pcm[i * 2 + 1] = v[len + i];
    }
    Ok(AudioBuffer {
        pcm: Arc::from(pcm.into_boxed_slice()),
        sample_rate: VOCODER_SAMPLE_RATE,
        channels: 2,
    })
}
