//! `LtxRetake` — регенерация временного региона `[start,end]` исходного видео
//! (одностадийно, полная сетка): `(model, v_enc, a_enc, video) →
//! (video_latent, audio_tokens)`. Видео декодится (ffmpeg) → VAE encode →
//! denoise_av_retake (регион денойзится, остальное frozen к исходнику;
//! аудио генерируется заново). Латент идёт прямо в VAE Decode (без upscale).

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{
    denoise_av_retake, frames_for_duration, latent_grid, out_frame_count, DenoiseHooks,
    DISTILLED_SIGMAS,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::sampler_stage1::publish_latents;
use super::shared;
use super::{
    current_input_audio_encoding, current_input_model, current_input_video_encoding,
    current_input_video_input, device_from_idx, fps_from_idx, make_fps_dropdown, progress_row,
};

pub struct RetakeExec;

impl NodeExecutor for RetakeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let _ = ctx.read_input("video");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxRetake { v_out, a_out, output_version, .. } => {
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
            NodeRuntime::LtxRetake { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxRetake {
                width, height, duration_seconds, fps_idx, retake_start, retake_end, seed,
                running, error, progress_pct, cancel, ..
            } => Some((
                *width, *height, *duration_seconds, *fps_idx, *retake_start, *retake_end, *seed,
                *running, *error, *progress_pct, cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, retake_start, retake_end, seed, running, error, progress_pct, cancel)) =
        snapshot
    else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxRetake")).class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(&tr!("nodes.common.field_width"), make_int_slider_row(width, 256, 1920, 32)),
        field_row(&tr!("nodes.common.field_height"), make_int_slider_row(height, 256, 1088, 32)),
        field_row(&tr!("nodes.common.field_duration_seconds"), make_slider_row(duration_seconds, 1.0, 20.0, 0.5, 1)),
        field_row("FPS", make_fps_dropdown(fps_idx)),
        field_row(&tr!("node.ltx_retake.field.region_start"), make_slider_row(retake_start, 0.0, 20.0, 0.1, 1)),
        field_row(&tr!("node.ltx_retake.field.region_end"), make_slider_row(retake_end, 0.0, 20.0, 0.1, 1)),
        field_row("Seed", make_seed_slider(seed)),
        field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
        field_row(&tr!("app.cancel"), super::cancel_button(running, cancel)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, "Retake denoise…", "ltx-node-running"),
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
            NodeRuntime::LtxRetake {
                width, height, duration_seconds, fps_idx, retake_start, retake_end, seed,
                running, error, progress_pct, cancel, v_out, a_out, output_version,
            } => Some((
                *width, *height, *duration_seconds, *fps_idx, *retake_start, *retake_end, *seed,
                *running, *error, *progress_pct, cancel.clone(), v_out.clone(), a_out.clone(), *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, retake_start, retake_end, seed, running, error, progress_pct, cancel, v_out, a_out, output_version)) =
        snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    let Some(v_enc) = current_input_video_encoding(ctx, node.id, "video_encoding") else {
        error.set(Some(tr!("node.ltx.common.connect_video_encoding")));
        return;
    };
    let Some(a_enc) = current_input_audio_encoding(ctx, node.id, "audio_encoding") else {
        error.set(Some(tr!("node.ltx.common.connect_audio_encoding")));
        return;
    };
    let Some(video_path) = current_input_video_input(ctx, node.id, "video") else {
        error.set(Some(tr!("node.ltx_retake.err.connect_video_input")));
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
    let (rs, re) = (retake_start.get_untracked() as f64, retake_end.get_untracked() as f64);
    let seed_v = seed.get_untracked();

    let _ = thread::Builder::new()
        .name("synthos-ltx-retake".into())
        .spawn(move || {
            let r = worker(&handle, &v_enc, &a_enc, &video_path, w, h, dur, fps, rs, re, seed_v, progress_pct, &cancel, &v_out, &a_out);
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
    video_path: &std::path::Path,
    width: usize,
    height: usize,
    duration: f64,
    fps: f64,
    retake_start: f64,
    retake_end: f64,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &std::sync::atomic::AtomicBool,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let (hp_t, wp_t) = latent_grid(width, height);
    let out_frames = out_frame_count(frames_for_duration(duration, fps));

    // исходное видео → кадры (на целевом разрешении) → VAE encode → source-латент.
    // Сетка retake берётся из РЕАЛЬНОГО латента (VAE temporal-компрессия может
    // дать иное число латент-кадров, чем frames_for_duration).
    let frames = shared::load_video_frames(video_path, hp_t * 32, wp_t * 32, out_frames, dev)?;
    let encoder = shared::load_vae_encoder(handle)?;
    let source = encoder.encode(&frames).map_err(|e| format!("VAE encode: {e}"))?;
    let sd = source.dims();
    let (fp, hp, wp) = (sd[2], sd[3], sd[4]);
    let tv_max = fp * hp * wp;

    let dit = shared::load_avdit(handle, tv_max)?;
    let hooks = DenoiseHooks {
        progress: Some(&|p| {
            let pct = p.step as f32 / p.total.max(1) as f32;
            run_on_main_thread(move || progress_pct.set(pct));
        }),
        cancel: Some(cancel),
    };
    let seed_opt = if seed == 0 { None } else { Some(seed) };
    let (l, a) = denoise_av_retake(
        &dit.dit, v_enc, a_enc, fp, hp, wp, &DISTILLED_SIGMAS,
        &source, retake_start, retake_end, fps, dev, seed_opt, &hooks,
    )
    .map_err(|e| match e {
        synaptix_video_ltx23::LtxError::Cancelled => tr!("node.ltx.common.cancelled"),
        other => format!("retake denoise: {other}"),
    })?;

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        *g = Some(LtxVideoLatent { tensor: l, fp, hp, wp, fps });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a);
    }
    Ok(())
}
