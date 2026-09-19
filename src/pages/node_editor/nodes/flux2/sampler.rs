use std::result::Result;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use synaptix_image_flux2::{Flux2Conditioning, Flux2Error, Flux2Model, Flux2References, Flux2Variant, SampleParams};
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use tracing::{debug, info};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, FluxBlob, FluxLatent, FluxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::super::ltx::{cancel_button, progress_row};
use super::super::{log_worker_done, log_worker_start, WORKER_LOG};
use super::{current_input_conditioning, current_input_latent, current_input_model, current_input_references, shared};

/// 0 — по модели: dev 50, klein 4 (дистиллированный), klein base 50.
pub const DEFAULT_STEPS: u32 = 0;
/// dev — guidance-эмбеддинг, klein base — масштаб CFG; дистиллированный
/// klein его не использует.
pub const DEFAULT_GUIDANCE: f32 = 4.0;

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("latent");
        let _ = ctx.read_input("references");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Flux2Sampler { out, output_version, .. } => {
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
            NodeRuntime::Flux2Sampler {
                steps,
                guidance,
                seed,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                out,
                output_version,
            } => Some((
                *steps,
                *guidance,
                *seed,
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
    let Some((steps, guidance, seed, running, error, loaded_name, progress_pct, cancel, out, output_version)) = snapshot
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.flux2.common.connect_checkpoint")));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some(tr!("node.flux2_sampler.connect_text_encoder")));
        return;
    };
    let refs = current_input_references(ctx, node.id, "references");
    // Размер: из латента (FLUX Empty Latent), иначе — первого референса,
    // как у пайплайна BFL при правке без явного размера.
    let size = current_input_latent(ctx, node.id, "latent")
        .map(|l| (l.width, l.height))
        .or_else(|| refs.as_ref().and_then(|r| r.sizes.first().copied()));
    let Some((width, height)) = size else {
        error.set(Some(tr!("node.flux2_sampler.connect_latent")));
        return;
    };
    let p = SampleParams {
        width,
        height,
        steps: steps.get_untracked() as usize,
        guidance: guidance.get_untracked(),
        seed: seed.get_untracked(),
        denoise: 1.0,
    };

    running.set(true);
    error.set(None);
    loaded_name.set(None);
    progress_pct.set(0.0);
    cancel.store(false, Ordering::Relaxed);

    let _ = thread::Builder::new().name("synthos-flux2-sampler".into()).spawn(move || {
        let started = log_worker_start(
            "flux2-sampler",
            &format!(
                "{}x{}, шаги {} (0 — по модели), guidance {}, seed {}, референсов {}",
                p.width,
                p.height,
                p.steps,
                p.guidance,
                p.seed,
                refs.as_ref().map(|r| r.len()).unwrap_or(0)
            ),
        );
        let res = worker(&handle, &cond, refs.as_deref(), p, progress_pct, &cancel);
        log_worker_done("flux2-sampler", started, &res.as_ref().map(|(_, s)| s.clone()));
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

/// Шаги «0 — по модели» и подпись статуса.
fn resolve_steps(variant: Flux2Variant, steps: usize) -> usize {
    if steps == 0 {
        variant.default_steps()
    } else {
        steps
    }
}

fn worker(
    handle: &FluxModelHandle,
    cond: &Flux2Conditioning,
    refs: Option<&Flux2References>,
    mut p: SampleParams,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(FluxLatent, String), String> {
    let model = shared::load_model(handle)?;
    let variant = model.model.variant();
    p.steps = resolve_steps(variant, p.steps);
    let ref_tokens = refs.map(|r| r.num_tokens()).unwrap_or(0);
    let tokens = Flux2Model::tokens_for(p.width, p.height, ref_tokens);
    let t_load = std::time::Instant::now();
    let dit = shared::load_transformer(handle, &model, tokens)?;
    let r = dit.transformer.residency();
    info!(
        target: WORKER_LOG,
        node = "flux2-sampler",
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
        debug!(target: WORKER_LOG, node = "flux2-sampler", step = i, total = n, step_ms, "шаг денойза");
        let pct = i as f32 / n.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
        !cancel.load(Ordering::Relaxed)
    };
    let t0 = std::time::Instant::now();
    let res = model.model.sample(&dit.transformer, cond, refs, None, &p, &mut progress);
    let secs = t0.elapsed().as_secs_f64();
    if handle.resident {
        shared::hold(dit);
    } else {
        drop(dit);
        shared::trim_pool(model.model.device());
    }
    let tensor = res.map_err(|e| match e {
        Flux2Error::Cancelled => tr!("node.flux_sampler.cancelled"),
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
            NodeRuntime::Flux2Sampler { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2Sampler { steps, guidance, seed, running, error, loaded_name, progress_pct, cancel, .. } => {
                Some((*steps, *guidance, *seed, *running, *error, *loaded_name, *progress_pct, cancel.clone()))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, guidance, seed, running, error, loaded_name, progress_pct, cancel)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        field_row(&tr!("node.flux2_sampler.steps"), make_int_slider_row(steps, 0, 100, 1)),
        field_row(&tr!("node.flux_sampler.guidance"), make_slider_row(guidance, 1.0, 10.0, 0.5, 1)),
        field_row("Seed", make_seed_slider(seed)),
        Box::new(Text::new(tr!("node.flux2_sampler.hint")).class("flux-node-info")),
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
        assert_eq!(resolve_steps(Flux2Variant::KleinDistilled, 0), 4);
        assert_eq!(resolve_steps(Flux2Variant::Dev, 0), 50);
        assert_eq!(resolve_steps(Flux2Variant::Dev, 28), 28);
    }
}
