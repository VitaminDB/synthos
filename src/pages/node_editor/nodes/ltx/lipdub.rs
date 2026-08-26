//! `LtxLipdub` — синхрон губ под речь: reference-видео даёт лицо/сцену, речь
//! управляет артикуляцией. `(model, v_enc, a_enc, ref_video, audio) →
//! (video_latent, audio_tokens)`. Двухстадийно (stage1 A/V append ref-видео +
//! ref-аудио → upscale → stage2 video-refine). LipDub IC-LoRA через lora_path
//! Checkpoint-ноды. Прогресс грубый (generate_lipdub_latents без шаг-хуков).

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{
    frames_for_duration, generate_lipdub_latents, latent_grid, out_frame_count, stage1_grid,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::sampler_stage1::publish_latents;
use super::shared;
use super::{
    current_input_audio_encoding, current_input_audio_input, current_input_model,
    current_input_video_encoding, current_input_video_input, device_from_idx, fps_from_idx,
    make_fps_dropdown, progress_row,
};

pub struct LipdubExec;

impl NodeExecutor for LipdubExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let _ = ctx.read_input("ref_video");
        let _ = ctx.read_input("audio");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxLipdub { v_out, a_out, output_version, .. } => {
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
            NodeRuntime::LtxLipdub { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxLipdub {
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
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxLipdub")).class("node-card-field-error"),
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
            status_row(running, error, loaded_name, "Lipdub denoise…", "ltx-node-running"),
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
            NodeRuntime::LtxLipdub {
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
        error.set(Some(tr!("node.ltx_lipdub.err.requires_upscaler")));
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
    let Some(ref_path) = current_input_video_input(ctx, node.id, "ref_video") else {
        error.set(Some(tr!("node.ltx.common.connect_video_input_ref_video")));
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
    let _ = seed.get_untracked();

    let _ = thread::Builder::new()
        .name("synthos-ltx-lipdub".into())
        .spawn(move || {
            // прогресс грубый: 0 → 0.5 (после ref/audio encode) → 1.0 (после denoise)
            run_on_main_thread(move || progress_pct.set(0.1));
            let r = worker(&handle, &v_enc, &a_enc, &ref_path, &audio_path, w, h, dur, fps, progress_pct, &v_out, &a_out);
            match r {
                Ok(()) => {
                    run_on_main_thread(move || {
                        progress_pct.set(1.0);
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
    ref_path: &std::path::Path,
    audio_path: &std::path::Path,
    width: usize,
    height: usize,
    duration: f64,
    fps: f64,
    progress_pct: RwSignal<f32>,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let (hp, wp) = latent_grid(width, height);
    let fp = frames_for_duration(duration, fps);
    let out_frames = out_frame_count(fp);
    let (hp1, wp1) = stage1_grid(hp, wp);

    // ref-видео на сетках обеих стадий + ref-аудио (речь).
    let (ref1, ref1_pos) = shared::video_ref_tokens(handle, ref_path, hp1, wp1, out_frames, fps)?;
    let (ref2, ref2_pos) = shared::video_ref_tokens(handle, ref_path, hp1 * 2, wp1 * 2, out_frames, fps)?;
    let audio_ref = shared::audio_ref_tokens(handle, audio_path)?;
    let up = shared::load_upsampler(handle)?;
    run_on_main_thread(move || progress_pct.set(0.5));

    let tv_max = fp * hp * wp;
    let dit = shared::load_avdit(handle, tv_max)?;
    let (latent, a_tok) = generate_lipdub_latents(
        &dit.dit, &up, v_enc, a_enc, &ref1, &ref1_pos, &ref2, &ref2_pos, &audio_ref,
        fp, hp1, wp1, fps, dev,
    )
    .map_err(|e| format!("lipdub denoise: {e}"))?;
    synaptix_core::tensor::ops::conv_filter_cache_clear();

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        // latent — full-res (hp1·2 = hp)
        *g = Some(LtxVideoLatent { tensor: latent, fp, hp, wp, fps });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a_tok);
    }
    Ok(())
}
