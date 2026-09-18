use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_minimax_h3 as h3;
use tracing::{debug, info};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, H3Blob, H3Conditioning, H3Geometry, H3Keyframe, H3ModelHandle, H3Refs,
    H3VideoLatent, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::super::{log_worker_done, log_worker_start, WORKER_LOG};
use super::{
    cancel_button, current_input_av_latent, current_input_conditioning, current_input_keyframe,
    current_input_model, current_input_refs, progress_row, shared,
};

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("negative");
        let _ = ctx.read_input("av_latent");
        let _ = ctx.read_input("keyframe");
        let _ = ctx.read_input("keyframe_last");
        let _ = ctx.read_input("refs");
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
        error.set(Some(tr!("node.minimax_h3.common.connect_checkpoint_model")));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some(tr!("node.minimax_h3_sampler.connect_text_encoder")));
        return;
    };
    let negative = current_input_conditioning(ctx, node.id, "negative");
    let Some(geometry) = current_input_av_latent(ctx, node.id, "av_latent") else {
        error.set(Some(tr!("node.minimax_h3_sampler.connect_empty_latent")));
        return;
    };
    let keyframes: Vec<Arc<H3Keyframe>> = ["keyframe", "keyframe_last"]
        .iter()
        .filter_map(|p| current_input_keyframe(ctx, node.id, p))
        .collect();
    let refs = current_input_refs(ctx, node.id, "refs");
    if let Some(r) = &refs {
        if !keyframes.is_empty() {
            error.set(Some(tr!("node.minimax_h3.common.refs_or_keyframes")));
            return;
        }
        // Видео и звук референсов обрезаны под длину, с которой их
        // декодировали; другая длина здесь — рассинхрон с кондиционированием.
        if r.frame_count != geometry.frame_count {
            error.set(Some(tr!(
                "node.minimax_h3_sampler.refs_length_mismatch",
                refs = r.frame_count,
                latent = geometry.frame_count
            )));
            return;
        }
    }

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
            let started = log_worker_start(
                "h3-sampler",
                &format!(
                    "{}x{}, {} кадров, латент {}x{}x{}, {n_steps} шагов, CFG {cfg}, seed {s}, \
                     негатив {}, keyframes {}, референсов {}",
                    geometry.width,
                    geometry.height,
                    geometry.frame_count,
                    geometry.latent_t,
                    geometry.latent_h,
                    geometry.latent_w,
                    if negative.is_some() { "есть" } else { "нет" },
                    keyframes.len(),
                    refs.as_ref().map(|r| r.media.len()).unwrap_or(0),
                ),
            );
            let res = worker(
                &handle,
                &cond,
                negative.as_deref(),
                geometry,
                &keyframes,
                refs.as_deref(),
                n_steps,
                cfg,
                s,
                progress_pct,
                &cancel,
            );
            log_worker_done("h3-sampler", started, &res);
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
    refs: Option<&H3Refs>,
    steps: usize,
    cfg_scale: f32,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> std::result::Result<(H3VideoLatent, synaptix_core::tensor::Tensor), String> {
    // Референсы кодируются до загрузки DiT: VAE на кадре 2048 px или на
    // 15-секундном видео нужна свободная карта.
    let ref_latents = match refs {
        Some(r) => Some(encode_refs(handle, r, seed).map_err(|e| format!("VAE: {e}"))?),
        None => None,
    };

    let anchor = shared::activation_anchor(handle, 13 << 29);
    let t_load = std::time::Instant::now();
    let shared_dit = shared::load_dit(handle)?;
    info!(
        target: WORKER_LOG,
        node = "h3-sampler",
        elapsed_ms = t_load.elapsed().as_millis() as u64,
        "DiT готов (загрузка или попадание в кэш)"
    );
    let dit = &shared_dit.dit;
    let ckpt = &shared_dit.ckpt;
    // H3_FORCE_REFS=1 — отладочный прогон пути референсов на весах FL2VA:
    // картинка бессмысленная, зато раскладка/VAE/денойз проверяются без Ref2VA.
    let force = std::env::var("H3_FORCE_REFS").is_ok_and(|v| v == "1");
    if ref_latents.is_some() && !ckpt.config.supports_references() && !force {
        return Err(tr!("node.minimax_h3_sampler.need_ref2va"));
    }

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
    if let Some(latents) = ref_latents {
        req.refs = latents.blocks;
        req.cond_rows = latents.cond_rows;
    }

    let t_prep = std::time::Instant::now();
    let prep = h3::pipeline::prepare(dit, &req, &sched).map_err(|e| e.to_string())?;
    info!(
        target: WORKER_LOG,
        node = "h3-sampler",
        video_tokens = g.video_tokens(dit.cfg.patch_size),
        elapsed_ms = t_prep.elapsed().as_millis() as u64,
        "prepare готов"
    );

    if !keyframes.is_empty() {
        let t_kf = std::time::Instant::now();
        let vae_cfg = ckpt.vae_config().map_err(|e| e.to_string())?;
        let w = h3::loader::ComponentLoader::open_component(
            ckpt.source(),
            h3::H3Component::VideoVae,
            shared_dit.device,
        )
        .map_err(|e| e.to_string())?;
        let enc = h3::vae::VaeEncoder::load(&w, vae_cfg, shared_dit.device, shared_dit.compute)
            .map_err(|e| e.to_string())?;
        let mut latents = Vec::with_capacity(keyframes.len());
        for kf in keyframes {
            let fitted = super::latent::fit_keyframe(
                &kf.image,
                geometry.width,
                geometry.height,
                kf.center_crop,
            )?;
            let d = fitted.dims().to_vec();
            let x = fitted
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
        info!(
            target: WORKER_LOG,
            node = "h3-sampler",
            count = keyframes.len(),
            elapsed_ms = t_kf.elapsed().as_millis() as u64,
            "keyframes закодированы VAE"
        );
    }

    drop(anchor);
    let t_cache = std::time::Instant::now();
    let cache = h3::pipeline::build_adaln_cache(dit, ckpt, &prep, shared_dit.compute)
        .map_err(|e| e.to_string())?;
    info!(
        target: WORKER_LOG,
        node = "h3-sampler",
        elapsed_ms = t_cache.elapsed().as_millis() as u64,
        "adaLN-кэш построен, начинаем денойз"
    );

    // Шаги денойза пишем в лог сами: без этого долгий прогон выглядит в
    // логе как тишина между «старт» и «готово», и по нему нельзя понять,
    // считает ли пайплайн вообще и с какой скоростью.
    let step_clock = std::sync::Mutex::new(std::time::Instant::now());
    let progress = move |p: h3::pipeline::DenoiseProgress| {
        let step_ms = match step_clock.lock() {
            Ok(mut last) => {
                let ms = last.elapsed().as_millis() as u64;
                *last = std::time::Instant::now();
                ms
            }
            Err(_) => 0,
        };
        let left = p.total.saturating_sub(p.step) as u64;
        debug!(
            target: WORKER_LOG,
            node = "h3-sampler",
            step = p.step,
            total = p.total,
            sigma = p.sigma,
            step_ms,
            eta_s = step_ms * left / 1000,
            "шаг денойза"
        );
        let pct = p.step as f32 / p.total.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
    };
    let hooks = h3::pipeline::DenoiseHooks {
        progress: Some(&progress),
        cancel: Some(cancel),
    };
    let out = h3::pipeline::denoise_av(dit, &cache, &prep, &req, &sched, &hooks).map_err(|e| {
        match e {
            h3::H3Error::Cancelled => tr!("node.minimax_h3_sampler.cancelled"),
            other => other.to_string(),
        }
    })?;

    shared::hold_dit(shared_dit.clone());
    Ok((
        H3VideoLatent { tensor: out.video_latent, geometry },
        out.audio_latent,
    ))
}

/// Латенты референсов: видео-VAE (энкодер) и, если есть звук, полный
/// audio-VAE. Оба живут только на время кодирования.
fn encode_refs(
    handle: &H3ModelHandle,
    refs: &H3Refs,
    seed: u64,
) -> std::result::Result<h3::refs::RefLatents, String> {
    shared::ensure_kernels_registered();
    let t = std::time::Instant::now();
    let device = super::device_of(handle.device_idx);
    let compute = super::compute_of(handle.compute_idx);
    let source = shared::source_of(handle)?;
    let err = |e: h3::H3Error| e.to_string();

    let vae_cfg = h3::config::VaeConfig::from_source(&source).map_err(err)?;
    let vw = h3::loader::ComponentLoader::open_component(&source, h3::H3Component::VideoVae, device)
        .map_err(err)?;
    let vae = h3::vae::VaeEncoder::load(&vw, vae_cfg, device, compute).map_err(err)?;

    let has_audio = refs.media.iter().any(|m| match m {
        h3::refs::RefMedia::Audio(_) => true,
        h3::refs::RefMedia::Video(v) => v.audio.is_some(),
        h3::refs::RefMedia::Image(_) => false,
    });
    let audio_vae = if has_audio {
        let cfg = h3::config::AudioVaeConfig::from_source(&source).map_err(err)?;
        let aw =
            h3::loader::ComponentLoader::open_component(&source, h3::H3Component::AudioVae, device)
                .map_err(err)?;
        Some(h3::audio_vae::AudioVae::load_full(&aw, cfg, device, compute).map_err(err)?)
    } else {
        None
    };

    let patch = h3::H3Config::from_source(&source).map_err(err)?.patch_size;
    let latents =
        h3::refs::encode_latents(&refs.media, &vae, audio_vae.as_ref(), patch, device, seed)
            .map_err(err)?;
    drop(vae);
    drop(audio_vae);
    shared::trim_pool(handle);
    info!(
        target: WORKER_LOG,
        node = "h3-sampler",
        count = refs.media.len(),
        elapsed_ms = t.elapsed().as_millis() as u64,
        "референсы закодированы VAE"
    );
    Ok(latents)
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
                field_row(&tr!("node.minimax_h3_sampler.steps"), make_int_slider_row(steps, 1, 40, 1)),
                field_row("CFG", make_slider_row(cfg_scale, 1.0, 12.0, 0.5, 1)),
                field_row("Seed", make_seed_slider(seed)),
                field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
                field_row(&tr!("app.cancel"), cancel_button(running, cancel)),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.minimax_h3_sampler.busy"), "h3-node-running"),
                ),
            ]),
    )
}
