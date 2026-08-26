use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::audio::AudioBuffer;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::visual::FramesView;
use syngui::widgets::Column;
use synaptix_video_minimax_h3 as h3;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, H3Blob, H3ModelHandle, H3VideoLatent, LtxFrames, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, status_row};
use super::super::{log_worker_done, log_worker_start};
use crate::pages::node_editor::controls::stereo_waveform::node_stereo_waveform;
use super::super::ltx::vae_decode::tensor_frames_to_rgba_unit;
use super::{current_input_audio_latent, current_input_model, current_input_video_latent, shared};

pub struct VaeDecodeExec;

impl NodeExecutor for VaeDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3VaeDecode { frames, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match frames.lock().ok().and_then(|g| g.clone()) {
                        Some(f) => PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Frames(f)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("frames", pv);
    }
}

pub fn vae_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VaeDecode { running, error, frames, preview_version, output_version } => {
                Some((*running, *error, frames.clone(), *preview_version, *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, frames, preview_version, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.minimax_h3.common.connect_checkpoint_model")));
        return;
    };
    let Some(latent) = current_input_video_latent(ctx, node.id, "video_latent") else {
        error.set(Some(tr!("node.minimax_h3_decode.connect_video_latent")));
        return;
    };

    running.set(true);
    error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-h3-vae".into())
        .spawn(move || {
            let started = log_worker_start(
                "h3-vae-decode",
                &format!("латент {:?}", latent.tensor.dims()),
            );
            let res = vae_worker(&handle, &latent);
            log_worker_done("h3-vae-decode", started, &res);
            match res {
                Ok(f) => {
                    if let Ok(mut g) = frames.lock() {
                        *g = Some(Arc::new(f));
                    }
                    error.set(None);
                    run_on_main_thread(move || {
                        preview_version.update(|v| *v = v.wrapping_add(1));
                        output_version.update(|v| *v = v.wrapping_add(1));
                    });
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

pub fn vae_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn vae_worker(handle: &H3ModelHandle, latent: &H3VideoLatent) -> std::result::Result<LtxFrames, String> {
    // «Держать в памяти» на Checkpoint: hold DiT переживает decode.
    if !handle.resident {
        shared::release_dit_hold();
    }
    shared::trim_pool(handle);
    let vae = shared::load_vae(handle)?;
    let rgb = vae.decoder.decode(&latent.tensor).map_err(|e| e.to_string())?;
    let frames = tensor_frames_to_rgba_unit(&rgb, h3::config::FPS, |_, _| {})?;
    shared::trim_pool(handle);
    Ok(frames)
}

pub fn vae_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VaeDecode { running, error, frames, preview_version, .. } => {
                Some((*running, *error, frames.clone(), *preview_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, frames, preview_version)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    let preview = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = preview_version.get();
        match frames.lock().ok().and_then(|g| g.clone()) {
            Some(fr) => vec![Box::new(
                FramesView::new(fr.frames.clone(), fr.fps as f32)
                    .fit(syngui::widgets::ImageFit::Contain)
                    .class("h3-preview-canvas"),
            )],
            None => vec![],
        }
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(preview),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.minimax_h3_decode.video_busy"), "h3-node-running"),
                ),
            ]),
    )
}

pub struct AudioDecodeExec;

impl NodeExecutor for AudioDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("audio_latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3AudioDecode { buffer, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match buffer.lock().ok().and_then(|g| g.clone()) {
                        Some(b) => PortValue::Audio(b),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("audio", pv);
    }
}

pub fn audio_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3AudioDecode { running, error, buffer, output_version } => {
                Some((*running, *error, buffer.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, buffer, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.minimax_h3.common.connect_checkpoint_model")));
        return;
    };
    let Some(latent) = current_input_audio_latent(ctx, node.id, "audio_latent") else {
        error.set(Some(tr!("node.minimax_h3_decode.connect_audio_latent")));
        return;
    };

    running.set(true);
    error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-h3-audio".into())
        .spawn(move || {
            let started =
                log_worker_start("h3-audio-decode", &format!("латент {:?}", latent.dims()));
            let res = audio_worker(&handle, &latent);
            log_worker_done("h3-audio-decode", started, &res);
            match res {
                Ok(b) => {
                    if let Ok(mut g) = buffer.lock() {
                        *g = Some(Arc::new(b));
                    }
                    error.set(None);
                    run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

pub fn audio_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3AudioDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn audio_worker(
    handle: &H3ModelHandle,
    latent: &synaptix_core::tensor::Tensor,
) -> std::result::Result<AudioBuffer, String> {
    if !handle.resident {
        shared::release_dit_hold();
    }
    shared::trim_pool(handle);
    let vae = shared::load_audio_vae(handle)?;
    let wave = vae.decoder.decode(latent).map_err(|e| e.to_string())?;
    let sample_rate = vae.decoder.sample_rate() as u32;
    let channels = wave.dims()[1] as u16;
    let pcm = h3::audio_vae::interleave_stereo(&wave).map_err(|e| e.to_string())?;
    shared::trim_pool(handle);
    Ok(AudioBuffer {
        pcm: pcm.into(),
        sample_rate,
        channels,
    })
}

pub fn audio_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3AudioDecode { running, error, buffer, output_version } => {
                Some((*running, *error, buffer.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, buffer, output_version)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    let wave = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = output_version.get();
        let buf = buffer.lock().ok().and_then(|g| g.clone());
        vec![node_stereo_waveform(buf, "h3-stereo-wave")]
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(wave),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.minimax_h3_decode.audio_busy"), "h3-node-running"),
                ),
            ]),
    )
}
