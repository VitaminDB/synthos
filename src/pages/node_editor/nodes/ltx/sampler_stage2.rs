//! `LtxSamplerStage2` — AvDit stage2 re-noise+refine (3 шага,
//! `STAGE2_SIGMAS`): видео-латент после Upscale ре-нойзится при σ₀ вместе с
//! аудио-токенами Stage1 и рефайнится. Без NAG (как CLI). Тот же AvDit из
//! Weak-кэша, что у Stage1 (жив благодаря `shared::hold_avdit`).

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{denoise_av, DenoiseHooks, STAGE2_SIGMAS};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_seed_slider, status_row};
use super::sampler_stage1::publish_latents;
use super::shared;
use super::{
    current_input_audio_encoding, current_input_audio_tokens, current_input_image_cond,
    current_input_model, current_input_video_encoding, current_input_video_latent, device_from_idx,
    progress_row,
};

pub struct SamplerStage2Exec;

impl NodeExecutor for SamplerStage2Exec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_latent");
        let _ = ctx.read_input("audio_tokens");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxSamplerStage2 {
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

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxSamplerStage2 { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxSamplerStage2 {
                seed,
                running,
                error,
                progress_pct,
                cancel,
                ..
            } => Some((*seed, *running, *error, *progress_pct, cancel.clone())),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((seed, running, error, progress_pct, cancel)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxSamplerStage2")).class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row("Seed", make_seed_slider(seed)),
        field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
        field_row(&tr!("app.cancel"), super::cancel_button(running, cancel)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, "Stage2 refine…", "ltx-node-running"),
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
            NodeRuntime::LtxSamplerStage2 {
                seed,
                running,
                error,
                progress_pct,
                cancel,
                v_out,
                a_out,
                output_version,
            } => Some((
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
    let Some((seed, running, error, progress_pct, cancel, v_out, a_out, output_version)) = snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    let Some(latent) = current_input_video_latent(ctx, node.id, "video_latent") else {
        error.set(Some(tr!("node.ltx_sampler_stage2.err.connect_video_latent_upscale")));
        return;
    };
    let Some(a_tok) = current_input_audio_tokens(ctx, node.id, "audio_tokens") else {
        error.set(Some(tr!("node.ltx_sampler_stage2.err.connect_audio_tokens_stage1")));
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
    let image_cond = current_input_image_cond(ctx, node.id, "image_cond");
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let seed_v = seed.get_untracked();

    let _ = thread::Builder::new()
        .name("synthos-ltx-sampler2".into())
        .spawn(move || {
            let r = worker(
                &handle, &latent, &a_tok, &v_enc, &a_enc, image_cond, seed_v, progress_pct, &cancel, &v_out,
                &a_out,
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
    latent: &LtxVideoLatent,
    a_tok: &synaptix_core::tensor::Tensor,
    v_enc: &synaptix_core::tensor::Tensor,
    a_enc: &synaptix_core::tensor::Tensor,
    image_cond: Option<(synaptix_core::tensor::Tensor, f32, usize)>,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &std::sync::atomic::AtomicBool,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let tv_max = latent.fp * latent.hp * latent.wp;
    // Conditioning рефайнится на обеих стадиях (иначе stage2 размывает якорь):
    // frame_idx=0 → i2v replace кадра 0; frame_idx>0 → keyframe append. Токены
    // энкодятся на stage2-сетку (latent.hp/wp уже ×2 после Upscale).
    let cond_toks = match &image_cond {
        Some((img, _, _)) => Some(shared::image_cond_tokens(handle, img, latent.hp, latent.wp)?),
        None => None,
    };
    let v_conds: Vec<(usize, synaptix_core::tensor::Tensor, f32)> =
        match (&image_cond, &cond_toks) {
            (Some((_, strength, 0)), Some(toks)) => vec![(0, toks.clone(), *strength)],
            _ => Vec::new(),
        };
    let kf_pos: Option<Vec<f64>> = match &image_cond {
        Some((_, _, fi)) if *fi > 0 => {
            Some(synaptix_video_ltx23::pipeline::keyframe_positions(latent.hp, latent.wp, *fi, latent.fps))
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
    let seed_opt = if seed == 0 { None } else { Some(seed) };
    let cancelled = |e: synaptix_video_ltx23::LtxError| match e {
        synaptix_video_ltx23::LtxError::Cancelled => tr!("node.ltx.common.cancelled"),
        other => format!("stage2 refine: {other}"),
    };
    let (l2, a2) = match (&image_cond, &cond_toks, &kf_pos) {
        // keyframe append refine (frame_idx>0): re-noise main + keyframe-якорь
        (Some((_, strength, fi)), Some(toks), Some(pos)) if *fi > 0 => {
            synaptix_video_ltx23::pipeline::denoise_av_append(
                &dit.dit, v_enc, a_enc, latent.fp, latent.hp, latent.wp, &STAGE2_SIGMAS,
                Some(&latent.tensor), Some(a_tok), false,
                Some((toks, pos, *strength)), None,
                latent.fps, dev, seed_opt, &hooks,
            )
            .map_err(cancelled)?
        }
        // i2v replace refine либо обычный refine
        _ => denoise_av(
            &dit.dit, v_enc, a_enc, latent.fp, latent.hp, latent.wp, &STAGE2_SIGMAS,
            Some(&latent.tensor), Some(a_tok), latent.fps, dev, None, &v_conds, seed_opt, &hooks,
        )
        .map_err(cancelled)?,
    };

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        *g = Some(LtxVideoLatent {
            tensor: l2,
            fp: latent.fp,
            hp: latent.hp,
            wp: latent.wp,
            fps: latent.fps,
        });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a2);
    }
    Ok(())
}
