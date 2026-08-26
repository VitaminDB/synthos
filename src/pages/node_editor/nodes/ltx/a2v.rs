//! `LtxA2V` — audio→video: входное аудио фиксирует звуковую дорожку, видео
//! генерируется под неё. `(model, v_enc, a_enc, audio) → (video_latent,
//! audio_tokens)`. Двухстадийно (stage1 video-denoise при frozen-аудио →
//! upscale ×2 → stage2 refine, аудио снова frozen). Аудио заморожено
//! (a_init=encoded, noise_scale=0) — собирается из denoise_av_append.

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{
    audio_token_count, denoise_av_append, frames_for_duration, latent_grid, stage1_grid,
    DenoiseHooks, DISTILLED_SIGMAS, STAGE2_SIGMAS,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::sampler_stage1::publish_latents;
use super::shared;
use super::{
    current_input_audio_encoding, current_input_audio_input, current_input_model,
    current_input_video_encoding, device_from_idx, fps_from_idx, make_fps_dropdown, progress_row,
};

pub struct A2vExec;

impl NodeExecutor for A2vExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let _ = ctx.read_input("audio");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxA2V { v_out, a_out, output_version, .. } => {
                    publish_latents(track, v_out, a_out, output_version)
                }
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("video_latent", v_pv);
        ctx.write_output("audio_tokens", a_pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxA2V { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxA2V {
                width, height, duration_seconds, fps_idx, seed, running, error, progress_pct, cancel, ..
            } => Some((*width, *height, *duration_seconds, *fps_idx, *seed, *running, *error, *progress_pct, cancel.clone())),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, seed, running, error, progress_pct, cancel)) =
        snapshot
    else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxA2V")).class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(&tr!("nodes.common.field_width"), make_int_slider_row(width, 256, 1920, 32)),
        field_row(&tr!("nodes.common.field_height"), make_int_slider_row(height, 256, 1088, 32)),
        field_row(&tr!("nodes.common.field_duration_seconds"), make_slider_row(duration_seconds, 1.0, 20.0, 0.5, 1)),
        field_row("FPS", make_fps_dropdown(fps_idx)),
        field_row("Seed", make_seed_slider(seed)),
        field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
        field_row(&tr!("app.cancel"), super::cancel_button(running, cancel)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, "A2V denoise…", "ltx-node-running"),
        ),
    ];
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
            NodeRuntime::LtxA2V {
                width, height, duration_seconds, fps_idx, seed, running, error, progress_pct, cancel,
                v_out, a_out, output_version,
            } => Some((
                *width, *height, *duration_seconds, *fps_idx, *seed, *running, *error, *progress_pct,
                cancel.clone(), v_out.clone(), a_out.clone(), *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, seed, running, error, progress_pct, cancel, v_out, a_out, output_version)) =
        snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    if handle.upscaler_path.is_none() {
        error.set(Some(tr!("node.ltx_a2v.err.requires_upscaler")));
        return;
    }
    let Some(v_enc) = current_input_video_encoding(ctx, node.id, "video_encoding") else {
        error.set(Some(tr!("node.ltx.common.connect_video_encoding")));
        return;
    };
    let Some(a_enc) = current_input_audio_encoding(ctx, node.id, "audio_encoding") else {
        error.set(Some(tr!("node.ltx.common.connect_audio_encoding")));
        return;
    };
    let Some(audio_path) = current_input_audio_input(ctx, node.id, "audio") else {
        error.set(Some(tr!("node.ltx.common.connect_audio_input_audio")));
        return;
    };
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let w = width.get_untracked() as usize;
    let h = height.get_untracked() as usize;
    let dur = duration_seconds.get_untracked() as f64;
    let fps = fps_from_idx(fps_idx.get_untracked());
    let seed_v = seed.get_untracked();

    let _ = thread::Builder::new()
        .name("synthos-ltx-a2v".into())
        .spawn(move || {
            let r = worker(&handle, &v_enc, &a_enc, &audio_path, w, h, dur, fps, seed_v, progress_pct, &cancel, &v_out, &a_out);
            match r {
                Ok(()) => {
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

#[allow(clippy::too_many_arguments)]
fn worker(
    handle: &LtxModelHandle,
    v_enc: &synaptix_core::tensor::Tensor,
    a_enc: &synaptix_core::tensor::Tensor,
    audio_path: &std::path::Path,
    width: usize,
    height: usize,
    duration: f64,
    fps: f64,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &std::sync::atomic::AtomicBool,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let (hp, wp) = latent_grid(width, height);
    let fp = frames_for_duration(duration, fps);
    let (hp1, wp1) = stage1_grid(hp, wp);
    let fa = audio_token_count(fp, fps);
    let tv_max = fp * hp * wp;

    // входное аудио → audio-латент (frozen-conditioning для обеих стадий).
    let audio_lat = shared::audio_input_latent(handle, audio_path, fa)?;
    let up = shared::load_upsampler(handle)?;
    run_on_main_thread(move || progress_pct.set(0.15));

    let dit = shared::load_avdit(handle, tv_max)?;
    let seed_opt = if seed == 0 { None } else { Some(seed) };
    let cancelled = |e: synaptix_video_ltx23::LtxError| match e {
        synaptix_video_ltx23::LtxError::Cancelled => tr!("node.ltx.common.cancelled"),
        other => format!("a2v denoise: {other}"),
    };

    // stage1: видео денойз с нуля, аудио frozen=входное (half-res).
    let hooks1 = DenoiseHooks {
        progress: Some(&|p| {
            let pct = 0.15 + 0.5 * (p.step as f32 / p.total.max(1) as f32);
            run_on_main_thread(move || progress_pct.set(pct));
        }),
        cancel: Some(cancel),
    };
    let (l1, _a1) = denoise_av_append(
        &dit.dit, v_enc, a_enc, fp, hp1, wp1, &DISTILLED_SIGMAS,
        None, Some(&audio_lat), true,
        None, None,
        fps, dev, seed_opt, &hooks1,
    )
    .map_err(cancelled)?;

    // upscale → stage2: видео refine, аудио снова frozen=входное.
    let l2 = up.upsample(&l1).map_err(|e| format!("upsample: {e}"))?
        .to_dtype(synaptix_core::dtype::DType::BF16).map_err(|e| format!("upsample dtype: {e}"))?;
    synaptix_core::tensor::ops::conv_filter_cache_clear();
    let hooks2 = DenoiseHooks {
        progress: Some(&|p| {
            let pct = 0.65 + 0.35 * (p.step as f32 / p.total.max(1) as f32);
            run_on_main_thread(move || progress_pct.set(pct));
        }),
        cancel: Some(cancel),
    };
    let (latent, a2) = denoise_av_append(
        &dit.dit, v_enc, a_enc, fp, hp1 * 2, wp1 * 2, &STAGE2_SIGMAS,
        Some(&l2), Some(&audio_lat), true,
        None, None,
        fps, dev, seed_opt, &hooks2,
    )
    .map_err(cancelled)?;

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        *g = Some(LtxVideoLatent { tensor: latent, fp, hp, wp, fps });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a2);
    }
    Ok(())
}
