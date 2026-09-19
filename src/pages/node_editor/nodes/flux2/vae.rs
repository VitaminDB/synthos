//! FLUX.2 VAE Decode: латент сэмплера → картинка. VAE (~330 МБ в F32)
//! грузится на время декода.

use std::result::Result;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, FluxLatent, FluxModelHandle, ImageData, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, status_row};
use super::super::image::image_preview;
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_latent, current_input_model, shared};

pub struct VaeDecodeExec;

impl NodeExecutor for VaeDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Flux2VaeDecode { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(img) => PortValue::Data(Arc::new(DataBlob::Image(img))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("image", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2VaeDecode { running, error, out, output_version } => {
                Some((*running, *error, out.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, out, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.flux2.common.connect_checkpoint")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent").filter(|l| l.tensor.is_some()) else {
        error.set(Some(tr!("node.flux2_vae_decode.connect_latent")));
        return;
    };

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-flux2-vae-decode".into()).spawn(move || {
        let started = log_worker_start("flux2-vae-decode", &format!("{}x{}", latent.width, latent.height));
        let res = worker(&handle, &latent);
        log_worker_done("flux2-vae-decode", started, &res);
        match res {
            Ok(img) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(img));
                }
                error.set(None);
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

fn worker(handle: &FluxModelHandle, latent: &FluxLatent) -> Result<ImageData, String> {
    let Some(t) = &latent.tensor else {
        return Err(tr!("node.flux2_vae_decode.connect_latent"));
    };
    let model = shared::load_model(handle)?;
    // Латент FLUX.1 (16 каналов) сюда не подходит — явное сообщение вместо
    // ошибки формы из глубины VAE.
    let ch = model.model.config().in_channels;
    if t.dims().get(1) != Some(&ch) {
        return Err(tr!("node.flux2_vae_decode.wrong_latent"));
    }
    let dev = model.model.device();
    let rgb = shared::with_vram_retry(dev, || model.model.decode(t).map_err(|e| e.to_string()))?;
    shared::trim_pool(dev);
    ImageData::from_tensor(rgb, None)
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2VaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2VaeDecode { running, error, out, output_version } => {
                Some((*running, *error, out.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, out, output_version)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    let preview = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = output_version.get();
        vec![image_preview(out.lock().ok().and_then(|g| g.clone()))]
    });
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        Box::new(preview),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_vae_decode.busy"), "flux-node-running"),
        ),
    ]))
}
