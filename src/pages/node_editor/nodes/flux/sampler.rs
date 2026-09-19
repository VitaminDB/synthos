use std::result::Result;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_image_flux::{FluxConditioning, FluxError, FluxModel, SampleParams};
use tracing::{debug, info};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, FluxBlob, FluxLatent, FluxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::super::ltx::{cancel_button, progress_row};
use super::super::{log_worker_done, log_worker_start, WORKER_LOG};
use super::{current_input_conditioning, current_input_latent, current_input_model, seq_len_of, shared};

pub const DEFAULT_STEPS: u32 = 28;
pub const DEFAULT_GUIDANCE: f32 = 3.5;

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::FluxSampler { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(l) => PortValue::Data(Arc::new(DataBlob::Flux(FluxBlob::Latent(l)))),
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
            NodeRuntime::FluxSampler {
                steps,
                guidance,
                seed,
                denoise,
                running,
                error,
                progress_pct,
                cancel,
                out,
                output_version,
            } => Some((
                *steps,
                *guidance,
                *seed,
                *denoise,
                *running,
                *error,
                *progress_pct,
                cancel.clone(),
                out.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, guidance, seed, denoise, running, error, progress_pct, cancel, out, output_version)) = snapshot
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.flux.common.connect_checkpoint")));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some(tr!("node.flux_sampler.connect_text_encoder")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent") else {
        error.set(Some(tr!("node.flux_sampler.connect_latent")));
        return;
    };
    let p = SampleParams {
        width: latent.width,
        height: latent.height,
        steps: steps.get_untracked().max(1) as usize,
        guidance: guidance.get_untracked(),
        seed: seed.get_untracked(),
        denoise: denoise.get_untracked().clamp(0.0, 1.0),
    };

    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    cancel.store(false, Ordering::Relaxed);

    let _ = thread::Builder::new().name("synthos-flux-sampler".into()).spawn(move || {
        let started = log_worker_start(
            "flux-sampler",
            &format!(
                "{}x{}, {} шагов, guidance {}, seed {}, denoise {}, {}",
                p.width,
                p.height,
                p.steps,
                p.guidance,
                p.seed,
                p.denoise,
                if latent.tensor.is_some() { "img2img" } else { "txt2img" },
            ),
        );
        let res = worker(&handle, &cond, &latent, &p, progress_pct, &cancel);
        log_worker_done("flux-sampler", started, &res);
        match res {
            Ok(l) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(l));
                }
                error.set(None);
                run_on_main_thread(move || output_version.update(|x| *x = x.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

fn worker(
    handle: &FluxModelHandle,
    cond: &FluxConditioning,
    latent: &FluxLatent,
    p: &SampleParams,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<FluxLatent, String> {
    let model = shared::load_model(handle)?;
    let seq = cond.t5.dims().get(1).copied().unwrap_or_else(|| seq_len_of(0, model.model.default_max_seq_len()));
    let tokens = FluxModel::tokens_for(p.width, p.height, seq);
    let t_load = std::time::Instant::now();
    let dit = shared::load_transformer(handle, &model, tokens)?;
    info!(
        target: WORKER_LOG,
        node = "flux-sampler",
        elapsed_ms = t_load.elapsed().as_millis() as u64,
        "трансформер готов (загрузка или попадание в кэш)"
    );

    let mut clock = std::time::Instant::now();
    let mut progress = |i: usize, n: usize| -> bool {
        let step_ms = clock.elapsed().as_millis() as u64;
        clock = std::time::Instant::now();
        debug!(target: WORKER_LOG, node = "flux-sampler", step = i, total = n, step_ms, "шаг денойза");
        let pct = i as f32 / n.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
        !cancel.load(Ordering::Relaxed)
    };
    let res = model.model.sample(&dit.transformer, cond, latent.tensor.as_ref(), p, &mut progress);
    if handle.resident {
        shared::hold(dit);
    } else {
        drop(dit);
        shared::trim_pool(model.model.device());
    }
    let tensor = res.map_err(|e| match e {
        FluxError::Cancelled => tr!("node.flux_sampler.cancelled"),
        other => other.to_string(),
    })?;
    let d = tensor.dims().to_vec();
    Ok(FluxLatent { width: d[3] * 8, height: d[2] * 8, tensor: Some(tensor) })
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxSampler { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxSampler { steps, guidance, seed, denoise, running, error, progress_pct, cancel, .. } => {
                Some((*steps, *guidance, *seed, *denoise, *running, *error, *progress_pct, cancel.clone()))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, guidance, seed, denoise, running, error, progress_pct, cancel)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row(&tr!("node.flux_sampler.steps"), make_int_slider_row(steps, 1, 60, 1)),
                field_row(&tr!("node.flux_sampler.guidance"), make_slider_row(guidance, 1.0, 10.0, 0.5, 1)),
                field_row("Seed", make_seed_slider(seed)),
                field_row(&tr!("node.flux_sampler.denoise"), make_slider_row(denoise, 0.05, 1.0, 0.05, 2)),
                field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
                field_row(&tr!("app.cancel"), cancel_button(running, cancel)),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.flux_sampler.busy"), "flux-node-running"),
                ),
            ]),
    )
}
