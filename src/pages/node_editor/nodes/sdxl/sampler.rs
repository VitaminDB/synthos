//! SDXL Sampler: Euler с CFG. Вход `latent` — FLUX Empty Latent (только
//! размер, стороны округляются вниз до кратного 64) или латент SDXL VAE
//! Encode (img2img с `denoise`).

use std::result::Result;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use synaptix_core::tensor::Tensor;
use synaptix_image_sdxl::{SdxlConditioning, SdxlError, SdxlSampleParams};
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use tracing::{debug, info};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, FluxLatent, FluxModelHandle, NodeInstance, NodeRuntime, PortValue, SdxlBlob};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::super::ltx::{cancel_button, progress_row};
use super::super::{log_worker_done, log_worker_start, WORKER_LOG};
use super::{current_input_conditioning, current_input_latent, current_input_model, shared};

/// Как у примеров SDXL-base: 30 шагов, guidance 5.
pub const DEFAULT_STEPS: u32 = 30;
pub const DEFAULT_GUIDANCE: f32 = 5.0;
/// img2img: заметная правка при сохранённой композиции.
pub const DEFAULT_DENOISE: f32 = 0.6;

pub struct SamplerExec;

impl NodeExecutor for SamplerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("conditioning");
        let _ = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::SdxlSampler { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(l) => PortValue::Data(Arc::new(DataBlob::Sdxl(SdxlBlob::Latent(l)))),
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
            NodeRuntime::SdxlSampler {
                steps,
                guidance,
                seed,
                denoise,
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
                *denoise,
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
    let Some((steps, guidance, seed, denoise, running, error, loaded_name, progress_pct, cancel, out, output_version)) =
        snapshot
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.sdxl.common.connect_checkpoint")));
        return;
    };
    let Some(cond) = current_input_conditioning(ctx, node.id, "conditioning") else {
        error.set(Some(tr!("node.sdxl_sampler.connect_text_encoder")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent") else {
        error.set(Some(tr!("node.sdxl_sampler.connect_latent")));
        return;
    };
    let init = latent.tensor.clone();
    let p = SdxlSampleParams {
        width: latent.width,
        height: latent.height,
        steps: (steps.get_untracked() as usize).max(1),
        guidance: guidance.get_untracked(),
        seed: seed.get_untracked(),
        denoise: if init.is_some() { denoise.get_untracked().clamp(0.0, 1.0) } else { 1.0 },
    };

    running.set(true);
    error.set(None);
    loaded_name.set(None);
    progress_pct.set(0.0);
    cancel.store(false, Ordering::Relaxed);

    let _ = thread::Builder::new().name("synthos-sdxl-sampler".into()).spawn(move || {
        let started = log_worker_start(
            "sdxl-sampler",
            &format!(
                "{}x{}, шаги {}, guidance {}, seed {}, img2img {}",
                p.width,
                p.height,
                p.steps,
                p.guidance,
                p.seed,
                if init.is_some() { format!("denoise {}", p.denoise) } else { "нет".into() }
            ),
        );
        let res = worker(&handle, &cond, init.as_ref(), p, progress_pct, &cancel);
        log_worker_done("sdxl-sampler", started, &res.as_ref().map(|(_, s)| s.clone()));
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

fn worker(
    handle: &FluxModelHandle,
    cond: &SdxlConditioning,
    init: Option<&Tensor>,
    p: SdxlSampleParams,
    progress_pct: RwSignal<f32>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(FluxLatent, String), String> {
    let model = shared::load_model(handle)?;
    if let Some(t) = init {
        if t.dims().get(1) != Some(&4) {
            return Err(tr!("node.sdxl_sampler.wrong_latent"));
        }
    }
    let t_load = std::time::Instant::now();
    let unet = shared::load_unet(handle, &model)?;
    info!(
        target: WORKER_LOG,
        node = "sdxl-sampler",
        elapsed_ms = t_load.elapsed().as_millis() as u64,
        "UNet готов (загрузка или попадание в кэш)"
    );
    let mut clock = std::time::Instant::now();
    let mut progress = |i: usize, n: usize| -> bool {
        let step_ms = clock.elapsed().as_millis() as u64;
        clock = std::time::Instant::now();
        debug!(target: WORKER_LOG, node = "sdxl-sampler", step = i, total = n, step_ms, "шаг денойза");
        let pct = i as f32 / n.max(1) as f32;
        run_on_main_thread(move || progress_pct.set(pct));
        !cancel.load(Ordering::Relaxed)
    };
    let t0 = std::time::Instant::now();
    let res = model.model.sample(&unet.unet, cond, init, &p, &mut progress);
    let secs = t0.elapsed().as_secs_f64();
    if handle.resident {
        shared::hold(unet);
    } else {
        drop(unet);
        shared::trim_pool(model.model.device());
    }
    let tensor = res.map_err(|e| match e {
        SdxlError::Cancelled => tr!("node.flux_sampler.cancelled"),
        other => other.to_string(),
    })?;
    let d = tensor.dims().to_vec();
    let summary = tr!("node.sdxl_sampler.done", steps = p.steps, secs = format!("{secs:.1}"));
    Ok((FluxLatent { width: d[3] * 8, height: d[2] * 8, tensor: Some(tensor) }, summary))
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlSampler { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlSampler {
                steps, guidance, seed, denoise, running, error, loaded_name, progress_pct, cancel, ..
            } => Some((
                *steps,
                *guidance,
                *seed,
                *denoise,
                *running,
                *error,
                *loaded_name,
                *progress_pct,
                cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((steps, guidance, seed, denoise, running, error, loaded_name, progress_pct, cancel)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        field_row(&tr!("node.sdxl_sampler.steps"), make_int_slider_row(steps, 1, 100, 1)),
        field_row(&tr!("node.flux_sampler.guidance"), make_slider_row(guidance, 1.0, 15.0, 0.5, 1)),
        field_row("Seed", make_seed_slider(seed)),
        field_row(&tr!("node.flux_sampler.denoise"), make_slider_row(denoise, 0.05, 1.0, 0.05, 2)),
        Box::new(Text::new(tr!("node.sdxl_sampler.hint")).class("flux-node-info")),
        field_row(&tr!("nodes.common.progress"), progress_row(running, progress_pct)),
        field_row(&tr!("app.cancel"), cancel_button(running, cancel)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_sampler.busy"), "flux-node-running"),
        ),
    ]))
}
