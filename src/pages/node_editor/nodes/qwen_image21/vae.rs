//! Qwen-Image 2.1 VAE Decode: латент сэмплера → картинка RGBA (альфа
//! остаётся в превью и в PNG, если модель её нарисовала). VAE (~600 МБ)
//! грузится на время стадии и считается в F32; большие картинки — плитками.

use std::result::Result;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, FluxLatent, ImageData, NodeInstance, NodeRuntime, PortValue, QwenImage21ModelHandle,
};
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
                NodeRuntime::QwenImage21VaeDecode { out, output_version, .. } => {
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
            NodeRuntime::QwenImage21VaeDecode { running, error, out, output_version } => {
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
        error.set(Some(tr!("node.qwen_image21.common.connect_checkpoint")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent").filter(|l| l.tensor.is_some()) else {
        error.set(Some(tr!("node.qwen_image21_vae_decode.connect_latent")));
        return;
    };

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-qwen-image21-vae-decode".into()).spawn(move || {
        let started = log_worker_start("qwen-image21-vae-decode", &format!("{}x{}", latent.width, latent.height));
        let res = worker(&handle, &latent);
        log_worker_done("qwen-image21-vae-decode", started, &res);
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

fn worker(handle: &QwenImage21ModelHandle, latent: &FluxLatent) -> Result<ImageData, String> {
    let Some(t) = &latent.tensor else {
        return Err(tr!("node.qwen_image21_vae_decode.connect_latent"));
    };
    let model = shared::load_model(handle)?;
    let dev = model.model.device();
    let rgba = shared::with_vram_retry(dev, || model.model.decode(t).map_err(|e| e.to_string()))?;
    shared::trim_pool(dev);
    // У непрозрачных генераций альфа лишь шумит у 255 — отдаём чистый RGB,
    // иначе PNG получил бы полупрозрачные пиксели.
    let transparent = synaptix_image_qwen21::RgbaImage::from_tensor(&rgba).map_err(|e| e.to_string())?.has_transparency();
    if transparent {
        ImageData::from_rgba_tensor(rgba, None)
    } else {
        let rgb = rgba.narrow(0, 0, 3).and_then(|x| x.contiguous()).map_err(|e| e.to_string())?;
        ImageData::from_tensor(rgb, None)
    }
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21VaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21VaeDecode { running, error, out, output_version } => {
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
