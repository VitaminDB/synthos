//! `LtxSamplerStage1` — AvDit distilled stage1: joint A/V денойз (8 шагов,
//! `DISTILLED_SIGMAS`) на ПОЛОВИННОЙ сетке (`stage1_grid`; upscaler ×2
//! восстановит целевую). `(model, v_enc, a_enc, nag?) → (video_latent,
//! audio_tokens)`.
//!
//! width/height — ЦЕЛЕВОЙ выход; нода сама считает hp1/wp1 (инкапсуляция
//! CLI-логики). Латент несёт метаданные сетки — downstream не пересчитывает
//! их из UI-полей. AvDit после прогона удерживается `shared::hold_avdit`
//! (Stage2 возьмёт тот же инстанс); Decode/Save снимают hold перед VAE.

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{
    denoise_av, frames_for_duration, latent_grid, stage1_grid, DenoiseHooks, DISTILLED_SIGMAS,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, LtxBlob, LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::shared;
use super::{
    current_input_audio_encoding, current_input_image_cond, current_input_model, current_input_nag,
    current_input_video_encoding, device_from_idx, fps_from_idx, make_fps_dropdown, progress_row,
};

pub struct SamplerStage1Exec;

impl NodeExecutor for SamplerStage1Exec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let _ = ctx.read_input("nag");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxSamplerStage1 {
                    v_out,
                    a_out,
                    output_version,
                    ..
                } => publish_latents(track, v_out, a_out, output_version),
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("video_latent", v_pv);
        ctx.write_output("audio_tokens", a_pv);
    }
}

/// Общая публикация пары (video_latent, audio_tokens) из output-буферов
/// (используется обоими Sampler-нодами).
pub fn publish_latents(
    track: bool,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
    output_version: &RwSignal<u32>,
) -> (PortValue, PortValue) {
    if track {
        let _ = output_version.get();
    }
    let v = match v_out.lock() {
        Ok(b) => match b.as_ref() {
            Some(l) => PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::VideoLatent(l.clone())))),
            None => PortValue::Empty,
        },
        Err(_) => PortValue::Empty,
    };
    let a = match a_out.lock() {
        Ok(b) => match b.as_ref() {
            Some(t) => PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::AudioTokens(t.clone())))),
            None => PortValue::Empty,
        },
        Err(_) => PortValue::Empty,
    };
    (v, a)
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxSamplerStage1 { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxSamplerStage1 {
                width,
                height,
                duration_seconds,
                fps_idx,
                seed,
                running,
                error,
                progress_pct,
                cancel,
                ..
            } => Some((
                *width,
                *height,
                *duration_seconds,
                *fps_idx,
                *seed,
                *running,
                *error,
                *progress_pct,
                cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, seed, running, error, progress_pct, cancel)) =
        snapshot
    else {
        return Box::new(Text::new("LtxSamplerStage1: некорректный runtime").class("node-card-field-error"));
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row("Ширина", make_int_slider_row(width, 256, 1920, 32)),
        field_row("Высота", make_int_slider_row(height, 256, 1088, 32)),
        field_row("Длительность, с", make_slider_row(duration_seconds, 1.0, 20.0, 0.5, 1)),
        field_row("FPS", make_fps_dropdown(fps_idx)),
        field_row("Seed", make_seed_slider(seed)),
        field_row("Прогресс", progress_row(running, progress_pct)),
        field_row("Отмена", super::cancel_button(running, cancel)),
        field_row(
            "Статус",
            status_row(running, error, loaded_name, "Stage1 denoise…", "ltx-node-running"),
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
            NodeRuntime::LtxSamplerStage1 {
                width,
                height,
                duration_seconds,
                fps_idx,
                seed,
                running,
                error,
                progress_pct,
                cancel,
                v_out,
                a_out,
                output_version,
            } => Some((
                *width,
                *height,
                *duration_seconds,
                *fps_idx,
                *seed,
                *running,
                *error,
                *progress_pct,
                cancel.clone(),
                v_out.clone(),
                a_out.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        width,
        height,
        duration_seconds,
        fps_idx,
        seed,
        running,
        error,
        progress_pct,
        cancel,
        v_out,
        a_out,
        output_version,
    )) = snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some("Подключите LTX Checkpoint на вход model".into()));
        return;
    };
    let Some(v_enc) = current_input_video_encoding(ctx, node.id, "video_encoding") else {
        error.set(Some("Подключите video_encoding от Text Encoder".into()));
        return;
    };
    let Some(a_enc) = current_input_audio_encoding(ctx, node.id, "audio_encoding") else {
        error.set(Some("Подключите audio_encoding от Text Encoder".into()));
        return;
    };
    let nag = current_input_nag(ctx, node.id, "nag");
    let image_cond = current_input_image_cond(ctx, node.id, "image_cond");
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
        .name("synthos-ltx-sampler1".into())
        .spawn(move || {
            let r = worker(
                &handle, &v_enc, &a_enc, nag, image_cond, w, h, dur, fps, seed_v, progress_pct, &cancel,
                &v_out, &a_out,
            );
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
    nag: Option<(synaptix_core::tensor::Tensor, f32, f32, f32)>,
    image_cond: Option<(synaptix_core::tensor::Tensor, f32, usize)>,
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
    let tv_max = fp * hp * wp;

    // conditioning: frame_idx=0 → i2v replace кадра 0; frame_idx>0 → keyframe
    // (append с keyframe-позициями на пиксель-кадре). Токены = image, encoded
    // на stage1-сетку (hp1·wp1).
    let cond_toks = match &image_cond {
        Some((img, _, _)) => Some(shared::image_cond_tokens(handle, img, hp1, wp1)?),
        None => None,
    };
    let v_conds: Vec<(usize, synaptix_core::tensor::Tensor, f32)> = match (&image_cond, &cond_toks) {
        (Some((_, strength, 0)), Some(toks)) => vec![(0, toks.clone(), *strength)],
        _ => Vec::new(),
    };
    let kf_pos: Option<Vec<f64>> = match &image_cond {
        Some((_, _, fi)) if *fi > 0 => {
            Some(synaptix_video_ltx23::pipeline::keyframe_positions(hp1, wp1, *fi, fps))
        }
        _ => None,
    };

    let dit = shared::load_avdit(handle, tv_max)?;
    let hooks = DenoiseHooks {
        progress: Some(&|p| {
            let pct = p.step as f32 / p.total.max(1) as f32;
            run_on_main_thread(move || progress_pct.set(pct));
        }),
        cancel: Some(cancel),
    };
    let v_nag = nag.as_ref().map(|(t, s, a, tau)| (t, *s, *a, *tau));
    let seed_opt = if seed == 0 { None } else { Some(seed) };
    let cancelled = |e: synaptix_video_ltx23::LtxError| match e {
        synaptix_video_ltx23::LtxError::Cancelled => "Отменено".to_string(),
        other => format!("stage1 denoise: {other}"),
    };
    let (l1, a1) = match (&image_cond, &cond_toks, &kf_pos) {
        // keyframe append (frame_idx>0)
        (Some((_, strength, fi)), Some(toks), Some(pos)) if *fi > 0 => {
            synaptix_video_ltx23::pipeline::denoise_av_append(
                &dit.dit, v_enc, a_enc, fp, hp1, wp1, &DISTILLED_SIGMAS,
                None, None, false,
                Some((toks, pos, *strength)), None,
                fps, dev, seed_opt, &hooks,
            )
            .map_err(cancelled)?
        }
        // i2v replace (v_conds) либо чистый text2video
        _ => denoise_av(
            &dit.dit, v_enc, a_enc, fp, hp1, wp1, &DISTILLED_SIGMAS,
            None, None, fps, dev, v_nag, &v_conds, seed_opt, &hooks,
        )
        .map_err(cancelled)?,
    };

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        *g = Some(LtxVideoLatent {
            tensor: l1,
            fp,
            hp: hp1,
            wp: wp1,
            fps,
        });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a1);
    }
    Ok(())
}
