//! Qwen-Image 2.1 Sampler: денойз (KV-кэш префикса; true CFG, если у
//! энкодера есть негатив и cfg > 1). Размер — с FLUX Empty Latent (вход
//! `latent`), без него — ~разрешение чекпойнта: квадрат для t2i или
//! пропорции последнего референса, как у пайплайна diffusers.

use std::result::Result;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use synaptix_image_qwen21::{Qwen21Conditioning, QwenImage21Error, QwenImage21Model, SampleParams};
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use tracing::{debug, info};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, FluxLatent, NodeInstance, NodeRuntime, PortValue, QwenImage21Blob, QwenImage21ModelHandle, QwenImage21Refs,
};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, make_toggle, status_row};
use super::super::ltx::{cancel_button, progress_row};
use super::super::{log_worker_done, log_worker_start, WORKER_LOG};
use super::{current_input_conditioning, current_input_model, current_input_references, current_input_size, shared};

/// 0 — по модели (40).
pub const DEFAULT_STEPS: u32 = 0;
/// `true_cfg_scale`: модель идёт без CFG.
pub const DEFAULT_CFG: f32 = 1.0;

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("references");
        let _ = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::QwenImage21Sampler { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(l) => PortValue::Data(Arc::new(DataBlob::QwenImage21(QwenImage21Blob::Latent(l)))),
                        None => PortValue::Empty,
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
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21Sampler {
                steps,
                cfg,
                seed,
                kv_cache,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                out,
                output_version,
            } => Some((
                *steps,
                *cfg,
                *seed,
                *kv_cache,
                *running,
                *error,
                *loaded_name,
                *progress_pct,
                cancel.clone(),
                out.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, cfg, seed, kv_cache, running, error, loaded_name, progress_pct, cancel, out, output_version)) =
        snapshot
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.qwen_image21.common.connect_checkpoint")));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some(tr!("node.qwen_image21_sampler.connect_text_encoder")));
        return;
    };
    let refs = current_input_references(ctx, node.id, "references");
    let size = current_input_size(ctx, node.id, "latent");
    let (steps, cfg, seed, kv_cache) =
        (steps.get_untracked() as usize, cfg.get_untracked(), seed.get_untracked(), kv_cache.get_untracked());

    running.set(true);
    error.set(None);
    loaded_name.set(None);
    progress_pct.set(0.0);
    cancel.store(false, Ordering::Relaxed);

    let _ = thread::Builder::new().name("synthos-qwen-image21-sampler".into()).spawn(move || {
        let started = log_worker_start(
            "qwen-image21-sampler",
            &format!(
                "размер {:?}, шаги {steps} (0 — по модели), cfg {cfg}, seed {seed}, kv-кэш {kv_cache}, картинок {}",
                size,
                refs.as_ref().map(|r| r.images.len()).unwrap_or(0)
            ),
        );
        let res = worker(&handle, &cond, refs.as_deref(), size, steps, cfg, seed, kv_cache, progress_pct, &cancel);
        log_worker_done("qwen-image21-sampler", started, &res.as_ref().map(|(_, s)| s.clone()));
        match res {
            Ok((l, summary)) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(l));
                }
                error.set(None);
                loaded_name.set(Some(summary));
                run_on_main_thread(move || output_version.update(|x| *x = x.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

/// Шаги «0 — по модели».
pub fn resolve_steps(steps: usize) -> usize {
    if steps == 0 {
        synaptix_image_qwen21::model::DEFAULT_STEPS
    } else {
        steps
    }
}

#[allow(clippy::too_many_arguments)]
fn worker(
    handle: &QwenImage21ModelHandle,
    cond: &Qwen21Conditioning,
    refs: Option<&QwenImage21Refs>,
    size: Option<(usize, usize)>,
    steps: usize,
    cfg: f32,
    seed: u64,
    kv_cache: bool,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(FluxLatent, String), String> {
    let model = shared::load_model(handle)?;
    let m = &model.model;
    if refs.map(|r| r.images.len()).unwrap_or(0) != cond.images() {
        return Err(tr!("node.qwen_image21_sampler.refs_mismatch"));
    }
    let res = shared::resolution(handle);
    let sizes: Vec<(usize, usize)> =
        refs.map(|r| r.images.iter().map(|i| (i.width as usize, i.height as usize)).collect()).unwrap_or_default();
    let (width, height) = size.unwrap_or_else(|| QwenImage21Model::default_size(&sizes, res));
    let p = SampleParams { width, height, steps: resolve_steps(steps), cfg, seed, kv_cache };
    let latents = refs.map(|r| &r.latents);
    let ref_tokens = latents.map(|r| r.num_tokens()).unwrap_or(0);
    let txt = cond.embeds.dims()[1].max(cond.negative.as_ref().map(|n| n.0.dims()[1]).unwrap_or(0));
    let tokens = QwenImage21Model::tokens_for(p.width, p.height, ref_tokens, txt);
    let t_load = std::time::Instant::now();
    let dit = shared::load_transformer(handle, &model, tokens)?;
    let r = dit.transformer.residency();
    info!(
        target: WORKER_LOG,
        node = "qwen-image21-sampler",
        elapsed_ms = t_load.elapsed().as_millis() as u64,
        on_device = r.device,
        on_host = r.host,
        from_source = r.source,
        "DiT готов (загрузка или попадание в кэш)"
    );

    let mut clock = std::time::Instant::now();
    let mut progress = |i: usize, n: usize| -> bool {
        let step_ms = clock.elapsed().as_millis() as u64;
        clock = std::time::Instant::now();
        debug!(target: WORKER_LOG, node = "qwen-image21-sampler", step = i, total = n, step_ms, "шаг денойза");
        let pct = i as f32 / n.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
        !cancel.load(Ordering::Relaxed)
    };
    let t0 = std::time::Instant::now();
    let result = m.sample(&dit.transformer, cond, latents, &p, &mut progress);
    let secs = t0.elapsed().as_secs_f64();
    if handle.resident {
        shared::hold(dit);
    } else {
        drop(dit);
        shared::trim_pool(m.device());
    }
    let tensor = result.map_err(|e| match e {
        QwenImage21Error::Cancelled => tr!("node.flux_sampler.cancelled"),
        other => other.to_string(),
    })?;
    let d = tensor.dims().to_vec();
    let summary = tr!(
        "node.flux2_sampler.done",
        steps = p.steps,
        secs = format!("{secs:.1}"),
        device = r.device,
        total = r.device + r.host + r.source
    );
    Ok((FluxLatent { width: d[3] * 16, height: d[2] * 16, tensor: Some(tensor) }, summary))
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21Sampler { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21Sampler {
                steps,
                cfg,
                seed,
                kv_cache,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                ..
            } => Some((*steps, *cfg, *seed, *kv_cache, *running, *error, *loaded_name, *progress_pct, cancel.clone())),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, cfg, seed, kv_cache, running, error, loaded_name, progress_pct, cancel)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        field_row(&tr!("node.flux2_sampler.steps"), make_int_slider_row(steps, 0, 100, 1)),
        field_row(&tr!("node.qwen_image_sampler.cfg"), make_slider_row(cfg, 1.0, 10.0, 0.5, 1)),
        field_row("Seed", make_seed_slider(seed)),
        field_row(&tr!("node.qwen_image21_sampler.kv_cache"), make_toggle(kv_cache)),
        Box::new(Text::new(tr!("node.qwen_image21_sampler.hint")).class("flux-node-info")),
        field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
        field_row(&tr!("app.cancel"), cancel_button(running, cancel)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_sampler.busy"), "flux-node-running"),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_steps_follow_the_model() {
        assert_eq!(resolve_steps(0), 40);
        assert_eq!(resolve_steps(8), 8);
    }
}
