//! SDXL VAE: Encode (картинка → латент для img2img) и Decode (латент
//! сэмплера → картинка). VAE (~160 МБ) грузится на время стадии, считается в
//! BF16 (в F16 декодер SDXL переполняется).

use std::result::Result;
use std::sync::Arc;
use std::thread;

use synaptix_image_sdxl::stages::snap_side;
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, FluxLatent, FluxModelHandle, ImageData, NodeInstance, NodeRuntime, PortValue, SdxlBlob,
};
use super::super::acestep::{field_row, status_row};
use super::super::image::image_preview;
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_image, current_input_latent, current_input_model, shared};
use crate::pages::node_editor::controls::dropdown_field::node_dropdown_field;
use crate::pages::node_editor::controls::RESIZE_MODES;

/// Родная площадь SDXL — 1 Мп: крупнее UNet начинает дублировать объекты.
pub const MAX_AUTO_PIXELS: usize = 1024 * 1024;

/// Размер без входа `size`: картинка, уменьшенная до 1 Мп, стороны кратны 64.
pub fn auto_size(w: usize, h: usize) -> (usize, usize) {
    let scale = ((MAX_AUTO_PIXELS as f64) / (w * h).max(1) as f64).sqrt().min(1.0);
    (snap_side((w as f64 * scale).round() as usize), snap_side((h as f64 * scale).round() as usize))
}

// ── Encode ────────────────────────────────────────────────────────────────

pub struct VaeEncodeExec;

impl NodeExecutor for VaeEncodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("image");
        let _ = ctx.read_input("size");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::SdxlVaeEncode { out, output_version, .. } => {
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

pub fn encode_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlVaeEncode { resize_idx, running, error, out, output_version } => {
                Some((*resize_idx, *running, *error, out.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((resize_idx, running, error, out, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.sdxl.common.connect_checkpoint")));
        return;
    };
    let Some(image) = current_input_image(ctx, node.id, "image") else {
        error.set(Some(tr!("node.flux_vae_encode.connect_image")));
        return;
    };
    let target = current_input_latent(ctx, node.id, "size").map(|l| (snap_side(l.width), snap_side(l.height)));
    let center_crop = resize_idx.get_untracked() == 1;

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-sdxl-vae-encode".into()).spawn(move || {
        let started = log_worker_start("sdxl-vae-encode", &format!("{}x{} → {:?}", image.width, image.height, target));
        let res = encode_worker(&handle, &image, target, center_crop);
        log_worker_done("sdxl-vae-encode", started, &res);
        match res {
            Ok(l) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(l));
                }
                error.set(None);
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

fn encode_worker(
    handle: &FluxModelHandle,
    image: &ImageData,
    target: Option<(usize, usize)>,
    center_crop: bool,
) -> Result<FluxLatent, String> {
    let (w, h) = target.unwrap_or_else(|| auto_size(image.width as usize, image.height as usize));
    let fitted = super::super::minimax_h3::latent::fit_keyframe(&image.tensor, w, h, center_crop)?;
    let model = shared::load_model(handle)?;
    let dev = model.model.device();
    let latent = shared::with_vram_retry(dev, || model.model.encode_image(&fitted).map_err(|e| e.to_string()))?;
    shared::trim_pool(dev);
    Ok(FluxLatent { width: w, height: h, tensor: Some(latent) })
}

pub fn encode_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlVaeEncode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn encode_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlVaeEncode { resize_idx, running, error, .. } => Some((*resize_idx, *running, *error)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((resize_idx, running, error)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        field_row(&tr!("nodes.keyframe.resize"), node_dropdown_field(RESIZE_MODES, resize_idx)),
        Box::new(Text::new(tr!("node.sdxl_vae_encode.hint")).class("flux-node-info")),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_vae_encode.busy"), "flux-node-running"),
        ),
    ]))
}

// ── Decode ────────────────────────────────────────────────────────────────

pub struct VaeDecodeExec;

impl NodeExecutor for VaeDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::SdxlVaeDecode { out, output_version, .. } => {
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
            NodeRuntime::SdxlVaeDecode { running, error, out, output_version } => {
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
        error.set(Some(tr!("node.sdxl.common.connect_checkpoint")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent").filter(|l| l.tensor.is_some()) else {
        error.set(Some(tr!("node.sdxl_vae_decode.connect_latent")));
        return;
    };

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-sdxl-vae-decode".into()).spawn(move || {
        let started = log_worker_start("sdxl-vae-decode", &format!("{}x{}", latent.width, latent.height));
        let res = decode_worker(&handle, &latent);
        log_worker_done("sdxl-vae-decode", started, &res);
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

fn decode_worker(handle: &FluxModelHandle, latent: &FluxLatent) -> Result<ImageData, String> {
    let Some(t) = &latent.tensor else {
        return Err(tr!("node.sdxl_vae_decode.connect_latent"));
    };
    let model = shared::load_model(handle)?;
    let dev = model.model.device();
    let rgb = shared::with_vram_retry(dev, || model.model.decode(t).map_err(|e| e.to_string()))?;
    shared::trim_pool(dev);
    ImageData::from_tensor(rgb, None)
}

pub fn decode_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlVaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn decode_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SdxlVaeDecode { running, error, out, output_version } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_size_caps_at_one_megapixel() {
        assert_eq!(auto_size(1024, 1024), (1024, 1024));
        let (w, h) = auto_size(4032, 3024);
        assert!(w * h <= MAX_AUTO_PIXELS && w % 64 == 0 && h % 64 == 0, "{w}x{h}");
        assert_eq!(auto_size(500, 300), (448, 256));
    }
}
