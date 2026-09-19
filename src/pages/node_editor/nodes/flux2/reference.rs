//! FLUX.2 Reference — картинка для правки. VAE кодирует её в токены, и
//! сэмплер видит их рядом с генерируемой картинкой (координата `t` = 10, 20,
//! …). Ноды собираются цепочкой: вход `references` — предыдущие, выход — они
//! же плюс эта картинка. Размер — как у пайплайна BFL: площадь до 1 Мп,
//! стороны кратны 16.

use std::result::Result;
use std::sync::Arc;
use std::thread;

use synaptix_image_flux2::model::reference_size;
use synaptix_image_flux2::Flux2References;
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, Flux2Blob, FluxModelHandle, ImageData, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, status_row};
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_image, current_input_model, current_input_references, shared, MAX_REFERENCES};

pub struct ReferenceExec;

impl NodeExecutor for ReferenceExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("image");
        let _ = ctx.read_input("references");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Flux2Reference { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(r) => PortValue::Data(Arc::new(DataBlob::Flux2(Flux2Blob::References(r)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("references", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2Reference { running, error, loaded_name, out, output_version } => {
                Some((*running, *error, *loaded_name, out.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, loaded_name, out, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.flux2.common.connect_checkpoint")));
        return;
    };
    let Some(image) = current_input_image(ctx, node.id, "image") else {
        error.set(Some(tr!("node.flux_vae_encode.connect_image")));
        return;
    };
    let prev = current_input_references(ctx, node.id, "references");
    if prev.as_ref().is_some_and(|p| p.len() >= MAX_REFERENCES) {
        error.set(Some(tr!("node.flux2_reference.too_many", max = MAX_REFERENCES)));
        return;
    }

    running.set(true);
    error.set(None);
    loaded_name.set(None);
    let _ = thread::Builder::new().name("synthos-flux2-reference".into()).spawn(move || {
        let started = log_worker_start(
            "flux2-reference",
            &format!("{}x{}, до неё {}", image.width, image.height, prev.as_ref().map(|p| p.len()).unwrap_or(0)),
        );
        let res = worker(&handle, &image, prev.as_deref());
        log_worker_done("flux2-reference", started, &res.as_ref().map(|r| r.len()));
        match res {
            Ok(r) => {
                let (w, h) = r.sizes.last().copied().unwrap_or((0, 0));
                loaded_name.set(Some(tr!("node.flux2_reference.done", n = r.len(), w = w, h = h)));
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(r));
                }
                error.set(None);
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

fn worker(
    handle: &FluxModelHandle,
    image: &ImageData,
    prev: Option<&Flux2References>,
) -> Result<Flux2References, String> {
    let (w, h) = reference_size(image.width as usize, image.height as usize);
    let fitted = super::super::minimax_h3::latent::fit_keyframe(&image.tensor, w, h, true)?;
    let model = shared::load_model(handle)?;
    let dev = model.model.device();
    let one = shared::with_vram_retry(dev, || {
        model.model.encode_references(std::slice::from_ref(&fitted)).map_err(|e| e.to_string())
    })?;
    shared::trim_pool(dev);
    match prev {
        Some(p) => Flux2References::join(&[p, &one]).map_err(|e| e.to_string()),
        None => Ok(one),
    }
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2Reference { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2Reference { running, error, loaded_name, .. } => Some((*running, *error, *loaded_name)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        Box::new(Text::new(tr!("node.flux2_reference.hint")).class("flux-node-info")),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_vae_encode.busy"), "flux-node-running"),
        ),
    ]))
}
