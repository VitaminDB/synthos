//! FLUX VAE: Encode (картинка → латент для img2img) и Decode (латент →
//! картинка). VAE — 160 МБ в F32, грузится на время стадии.

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
    DataBlob, FluxBlob, FluxLatent, FluxModelHandle, ImageData, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, status_row};
use crate::pages::node_editor::controls::dropdown_field::node_dropdown_field;
use super::super::image::image_preview;
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_image, current_input_latent, current_input_model, shared};
use crate::pages::node_editor::controls::RESIZE_MODES;

/// Без входа `size` картинка кодируется в своём размере, но не больше
/// этой площади: FLUX.1 обучен примерно до 2 Мп, а фото с телефона на
/// 12 Мп не влезли бы в VRAM.
pub const MAX_AUTO_PIXELS: usize = 2_097_152;

/// Размер латента для картинки `w×h` без явного размера: площадь не больше
/// [`MAX_AUTO_PIXELS`], стороны кратны 16.
pub fn auto_size(w: usize, h: usize) -> (usize, usize) {
    let scale = ((MAX_AUTO_PIXELS as f64) / (w * h) as f64).sqrt().min(1.0);
    let snap = |v: f64| synaptix_image_flux::model::snap_side(v.round() as usize);
    (snap(w as f64 * scale), snap(h as f64 * scale))
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
                NodeRuntime::FluxVaeEncode { out, output_version, .. } => {
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

pub fn encode_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeEncode { resize_idx, running, error, out, output_version } => {
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
        error.set(Some(tr!("node.flux.common.connect_checkpoint")));
        return;
    };
    let Some(image) = current_input_image(ctx, node.id, "image") else {
        error.set(Some(tr!("node.flux_vae_encode.connect_image")));
        return;
    };
    let target = current_input_latent(ctx, node.id, "size").map(|l| (l.width, l.height));
    let center_crop = resize_idx.get_untracked() == 1;

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-flux-vae-encode".into()).spawn(move || {
        let started = log_worker_start(
            "flux-vae-encode",
            &format!("{}x{} → {:?}", image.width, image.height, target),
        );
        let res = encode_worker(&handle, &image, target, center_crop);
        log_worker_done("flux-vae-encode", started, &res);
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
    let latent = model.model.encode_image(&fitted).map_err(|e| e.to_string())?;
    shared::trim_pool(model.model.device());
    Ok(FluxLatent { width: w, height: h, tensor: Some(latent) })
}

pub fn encode_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeEncode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn encode_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeEncode { resize_idx, running, error, .. } => Some((*resize_idx, *running, *error)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((resize_idx, running, error)) = snapshot else {
        return Box::new(Column::new());
    };
    let loaded_name = use_signal(None::<String>);
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row(&tr!("nodes.keyframe.resize"), node_dropdown_field(RESIZE_MODES, resize_idx)),
                Box::new(Text::new(tr!("node.flux_vae_encode.hint")).class("flux-node-info")),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.flux_vae_encode.busy"), "flux-node-running"),
                ),
            ]),
    )
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
                NodeRuntime::FluxVaeDecode { out, output_version, .. } => {
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

pub fn decode_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeDecode { running, error, out, output_version } => {
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
        error.set(Some(tr!("node.flux.common.connect_checkpoint")));
        return;
    };
    let Some(latent) = current_input_latent(ctx, node.id, "latent").filter(|l| l.tensor.is_some()) else {
        error.set(Some(tr!("node.flux_vae_decode.connect_latent")));
        return;
    };

    running.set(true);
    error.set(None);
    let _ = thread::Builder::new().name("synthos-flux-vae-decode".into()).spawn(move || {
        let started = log_worker_start("flux-vae-decode", &format!("{}x{}", latent.width, latent.height));
        let res = decode_worker(&handle, &latent);
        log_worker_done("flux-vae-decode", started, &res);
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
        return Err(tr!("node.flux_vae_decode.connect_latent"));
    };
    let model = shared::load_model(handle)?;
    let rgb = model.model.decode(t).map_err(|e| e.to_string())?;
    shared::trim_pool(model.model.device());
    ImageData::from_tensor(rgb, None)
}

pub fn decode_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn decode_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxVaeDecode { running, error, out, output_version } => {
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
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(preview),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.flux_vae_decode.busy"), "flux-node-running"),
                ),
            ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_size_caps_area_and_snaps() {
        assert_eq!(auto_size(1024, 768), (1024, 768));
        let (w, h) = auto_size(4032, 3024); // 12 Мп с телефона
        assert!(w * h <= MAX_AUTO_PIXELS, "{w}x{h}");
        assert_eq!((w % 16, h % 16), (0, 0));
        assert!((w as f64 / h as f64 - 4.0 / 3.0).abs() < 0.02);
        assert_eq!(auto_size(1000, 1000), (992, 992));
    }
}
