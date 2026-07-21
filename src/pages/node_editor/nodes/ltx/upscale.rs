//! `LtxUpscale` — spatial latent-upscaler ×2 (`Upsampler`).
//! `(model, video_latent) → video_latent` с hp/wp ×2. Статистики VAE
//! (mean/std-of-means) читаются из чекпойнта, веса — из `upscaler_path`
//! хэндла. После апскейла `conv_filter_cache_clear()` — krsc-копии
//! upscaler'а освобождают VRAM под stage2.

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::upscaler::Upsampler;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, LtxBlob, LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, status_row};
use super::shared;
use super::{compute_from_idx, current_input_model, current_input_video_latent, device_from_idx};

pub struct UpscaleExec;

impl NodeExecutor for UpscaleExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxUpscale {
                    out,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(l) => PortValue::Data(Arc::new(DataBlob::Ltx(
                                LtxBlob::VideoLatent(l.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("video_latent", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxUpscale { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxUpscale { running, error, .. } => Some((*running, *error)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error)) = snapshot else {
        return Box::new(Text::new("LtxUpscale: некорректный runtime").class("node-card-field-error"));
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![field_row(
        "Статус",
        status_row(running, error, loaded_name, "Upscale ×2…", "ltx-node-running"),
    )];
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
            NodeRuntime::LtxUpscale {
                running,
                error,
                out,
                output_version,
            } => Some((*running, *error, out.clone(), *output_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, out, output_version)) = snapshot else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some("Подключите LTX Checkpoint на вход model".into()));
        return;
    };
    let Some(latent) = current_input_video_latent(ctx, node.id, "video_latent") else {
        error.set(Some("Подключите video_latent от Sampler Stage1".into()));
        return;
    };
    if handle.upscaler_path.is_none() {
        error.set(Some("В Checkpoint-ноде не выбран spatial-upscaler".into()));
        return;
    }
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-ltx-upscale".into())
        .spawn(move || {
            match worker(&handle, &latent) {
                Ok(l2) => {
                    if let Ok(mut g) = out.lock() {
                        *g = Some(l2);
                    }
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

fn worker(handle: &LtxModelHandle, latent: &LtxVideoLatent) -> std::result::Result<LtxVideoLatent, String> {
    let dev = device_from_idx(handle.device_idx);
    let compute = compute_from_idx(handle.compute_idx);
    let up_path = handle
        .upscaler_path
        .as_ref()
        .ok_or("upscaler_path пуст")?;
    let ckpt = shared::load_ckpt(handle)?;
    let ckpt_gpu = ckpt.view_on(dev);
    let mean = ckpt_gpu
        .get_raw("vae.per_channel_statistics.mean-of-means")
        .map_err(|e| format!("vae mean: {e}"))?;
    let std = ckpt_gpu
        .get_raw("vae.per_channel_statistics.std-of-means")
        .map_err(|e| format!("vae std: {e}"))?;
    let up = Upsampler::load(up_path, &mean, &std, dev).map_err(|e| format!("upscaler: {e}"))?;
    let l2 = up
        .upsample(&latent.tensor)
        .map_err(|e| format!("upsample: {e}"))?
        .to_dtype(compute)
        .map_err(|e| format!("upsample dtype: {e}"))?;
    synaptix_core::tensor::ops::conv_filter_cache_clear();
    Ok(LtxVideoLatent {
        tensor: l2,
        fp: latent.fp,
        hp: latent.hp * 2,
        wp: latent.wp * 2,
        fps: latent.fps,
    })
}
