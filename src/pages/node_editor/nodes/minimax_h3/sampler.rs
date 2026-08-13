use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_minimax_h3 as h3;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, H3Blob, H3Conditioning, H3Geometry, H3Keyframe, H3ModelHandle, H3VideoLatent,
    NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::{
    cancel_button, current_input_av_latent, current_input_conditioning, current_input_keyframe,
    current_input_model, progress_row, shared,
};

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("negative");
        let _ = ctx.read_input("av_latent");
        let _ = ctx.read_input("keyframe");
        let track = ctx.track;
        let (v, a) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3Sampler { v_out, a_out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    let v = v_out
                        .lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .map(|t| PortValue::Data(Arc::new(DataBlob::H3(H3Blob::VideoLatent(t)))))
                        .unwrap_or(PortValue::Empty);
                    let a = a_out
                        .lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .map(|t| PortValue::Data(Arc::new(DataBlob::H3(H3Blob::AudioLatent(t)))))
                        .unwrap_or(PortValue::Empty);
                    (v, a)
                }
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("video_latent", v);
        ctx.write_output("audio_latent", a);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Sampler { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

#[allow(clippy::type_complexity)]
fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Sampler {
                steps,
                cfg_scale,
                seed,
                running,
                error,
                progress_pct,
                cancel,
                v_out,
                a_out,
                output_version,
            } => Some((
                *steps,
                *cfg_scale,
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
        steps,
        cfg_scale,
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
    if running.get_untracked() {
        return;
    }

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some("подключите H3 Checkpoint на вход model".into()));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some("подключите H3 Text Encoder на вход conditioning".into()));
        return;
    };
    let negative = current_input_conditioning(ctx, node.id, "negative");
    let Some(geometry) = current_input_av_latent(ctx, node.id, "av_latent") else {
        error.set(Some("подключите H3 Empty AV Latent на вход av_latent".into()));
        return;
    };
    let keyframes: Vec<Arc<H3Keyframe>> = ["keyframe", "keyframe_last"]
        .iter()
        .filter_map(|p| current_input_keyframe(ctx, node.id, p))
        .collect();

    let n_steps = steps.get_untracked().max(1) as usize;
    let cfg = cfg_scale.get_untracked();
    let s = seed.get_untracked();

    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    cancel.store(false, Ordering::Relaxed);

    let _ = thread::Builder::new()
        .name("synthos-h3-sampler".into())
        .spawn(move || {
            let res = worker(
                &handle,
                &cond,
                negative.as_deref(),
                geometry,
                &keyframes,
                n_steps,
                cfg,
                s,
                progress_pct,
                &cancel,
            );
            match res {
                Ok((v, a)) => {
                    if let Ok(mut g) = v_out.lock() {
                        *g = Some(Arc::new(v));
                    }
                    if let Ok(mut g) = a_out.lock() {
                        *g = Some(a);
                    }
                    error.set(None);
                    run_on_main_thread(move || output_version.update(|x| *x = x.wrapping_add(1)));
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

#[allow(clippy::too_many_arguments)]
fn worker(
    handle: &H3ModelHandle,
    cond: &H3Conditioning,
    negative: Option<&H3Conditioning>,
    geometry: H3Geometry,
    keyframes: &[Arc<H3Keyframe>],
    steps: usize,
    cfg_scale: f32,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> std::result::Result<(H3VideoLatent, synaptix_core::tensor::Tensor), String> {
    let anchor = shared::activation_anchor(handle, 13 << 29);
    let shared_dit = shared::load_dit(handle)?;
    let dit = &shared_dit.dit;
    let ckpt = &shared_dit.ckpt;

    let g = h3::pipeline::Geometry::new(geometry.width, geometry.height, geometry.frame_count);
    let sched = h3::H3Scheduler::new(
        steps,
        ckpt.config.sigma_shift_video as f64,
        ckpt.config.sigma_shift_audio as f64,
    );

    let conditioning = h3::pipeline::Conditioning {
        context: cond.hidden.clone(),
        text_tags: cond.tags.clone(),
    };
    let negative_cond = negative.map(|n| h3::pipeline::Conditioning {
        context: n.hidden.clone(),
        text_tags: n.tags.clone(),
    });
    let mut req = h3::pipeline::DenoiseRequest::new(g, &conditioning);
    req.seed = Some(seed);
    if cfg_scale > 1.0 {
        if let Some(n) = negative_cond.as_ref() {
            req.guider = h3::guider::GuiderParams::cfg(cfg_scale);
            req.negative = Some(n);
        }
    }
    req.keyframes = keyframes
        .iter()
        .map(|kf| h3::layout::Keyframe {
            resolved_frame_index: if kf.frame_index == usize::MAX {
                g.frame_count - 1
            } else {
                kf.frame_index
            },
        })
        .collect();

    let prep = h3::pipeline::prepare(dit, &req, &sched).map_err(|e| e.to_string())?;

    if !keyframes.is_empty() {
        let vae_cfg = ckpt.vae_config().map_err(|e| e.to_string())?;
        let paths = shared::paths_of(handle)?;
        let w = h3::loader::ComponentLoader::open_file(paths.video_vae_file(), shared_dit.device)
            .map_err(|e| e.to_string())?;
        let enc = h3::vae::VaeEncoder::load(&w, vae_cfg, shared_dit.device, shared_dit.compute)
            .map_err(|e| e.to_string())?;
        let mut latents = Vec::with_capacity(keyframes.len());
        for kf in keyframes {
            let d = kf.image.dims().to_vec();
            let x = kf
                .image
                .to_device(shared_dit.device)
                .and_then(|t| t.reshape(vec![1, d[0], 1, d[1], d[2]]))
                .and_then(|t| t.mul_scalar(2.0))
                .and_then(|t| t.add_scalar(-1.0))
                .map_err(|e| e.to_string())?;
            latents.push(enc.encode(&x).map_err(|e| e.to_string())?);
        }
        drop(enc);
        shared::trim_pool(handle);
        req.cond_rows.video =
            h3::pipeline::cond_rows_from_keyframe_latents(&latents, dit.cfg.patch_size, None, seed)
                .map_err(|e| e.to_string())?;
    }

    drop(anchor);
    let cache = h3::pipeline::build_adaln_cache(dit, ckpt, &prep, shared_dit.compute)
        .map_err(|e| e.to_string())?;

    let progress = move |p: h3::pipeline::DenoiseProgress| {
        let pct = p.step as f32 / p.total.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
    };
    let hooks = h3::pipeline::DenoiseHooks {
        progress: Some(&progress),
        cancel: Some(cancel),
    };
    let out = h3::pipeline::denoise_av(dit, &cache, &prep, &req, &sched, &hooks).map_err(|e| {
        match e {
            h3::H3Error::Cancelled => "отменено".to_string(),
            other => other.to_string(),
        }
    })?;

    shared::hold_dit(shared_dit.clone());
    Ok((
        H3VideoLatent { tensor: out.video_latent, geometry },
        out.audio_latent,
    ))
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Sampler {
                steps,
                cfg_scale,
                seed,
                running,
                error,
                progress_pct,
                cancel,
                ..
            } => Some((*steps, *cfg_scale, *seed, *running, *error, *progress_pct, cancel.clone())),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, cfg_scale, seed, running, error, progress_pct, cancel)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row("Шагов", make_int_slider_row(steps, 1, 40, 1)),
                field_row("CFG", make_slider_row(cfg_scale, 1.0, 12.0, 0.5, 1)),
                field_row("Seed", make_seed_slider(seed)),
                field_row("Прогресс", progress_row(running, progress_pct)),
                field_row("Отмена", cancel_button(running, cancel)),
                field_row(
                    "Статус",
                    status_row(running, error, loaded_name, "денойзинг…", "h3-node-running"),
                ),
            ]),
    )
}
