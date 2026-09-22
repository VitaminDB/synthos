//! Qwen-Image 2.1 Text Encoder: промпт (и, если подключены, референсы со
//! входа `references` — те же, что идут в сэмплер) → скрытые состояния
//! Qwen3-VL до финальной нормы. Негатив для true CFG — со входа `negative`;
//! пустой — CFG выключен (так модель и задумана).

use std::result::Result;
use std::sync::Arc;
use std::thread;

use synaptix_image_qwen21::{Qwen21Conditioning, RgbaImage};
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, NodeInstance, NodeRuntime, PortValue, QwenImage21Blob, QwenImage21ModelHandle, QwenImage21Refs,
};
use super::super::acestep::{field_row, status_row};
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_model, current_input_references, current_input_text, shared};

pub struct TextEncoderExec;

impl NodeExecutor for TextEncoderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("prompt");
        let _ = ctx.read_input("negative");
        let _ = ctx.read_input("references");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::QwenImage21TextEncoder { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(c) => PortValue::Data(Arc::new(DataBlob::QwenImage21(QwenImage21Blob::Conditioning(c)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("conditioning", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21TextEncoder { running, error, loaded_name, out, output_version } => {
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
        error.set(Some(tr!("node.qwen_image21.common.connect_checkpoint")));
        return;
    };
    let Some(prompt) = current_input_text(ctx, node.id, "prompt").filter(|p| !p.trim().is_empty()) else {
        error.set(Some(tr!("node.flux_text_encoder.connect_prompt")));
        return;
    };
    let negative = current_input_text(ctx, node.id, "negative").unwrap_or_default();
    let refs = current_input_references(ctx, node.id, "references");

    running.set(true);
    error.set(None);
    loaded_name.set(None);

    let _ = thread::Builder::new().name("synthos-qwen-image21-encode".into()).spawn(move || {
        let started = log_worker_start(
            "qwen-image21-text-encoder",
            &format!("промпт {} симв., картинок {}", prompt.chars().count(), refs.as_ref().map(|r| r.images.len()).unwrap_or(0)),
        );
        let res = worker(&handle, &prompt, &negative, refs.as_deref());
        log_worker_done("qwen-image21-text-encoder", started, &res.as_ref().map(|(_, n)| n.clone()));
        match res {
            Ok((cond, name)) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(cond);
                }
                error.set(None);
                loaded_name.set(Some(name));
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

/// `(кондиционирование, подпись статуса)`.
fn worker(
    handle: &QwenImage21ModelHandle,
    prompt: &str,
    negative: &str,
    refs: Option<&QwenImage21Refs>,
) -> Result<(Arc<Qwen21Conditioning>, String), String> {
    let model = shared::load_model(handle)?;
    let m = &model.model;
    let images: Vec<RgbaImage> = match refs {
        Some(r) => r.images.iter().map(|i| shared::rgba_of(i)).collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let keys: Vec<String> = refs.map(|r| r.images.iter().map(|i| i.key.clone()).collect()).unwrap_or_default();
    // Пустой негатив — без CFG, как `negative_prompt=None`.
    let neg = (!negative.trim().is_empty()).then_some(negative);
    let key = shared::cond_key(handle, prompt, neg, &keys);
    if let Some(c) = shared::cached_conditioning(&key) {
        return Ok((c, tr!("node.flux_text_encoder.cached")));
    }
    let dev = m.device();
    let res = shared::resolution(handle);
    let cond = shared::with_vram_retry(dev, || m.encode_prompt(prompt, neg, &images, res).map_err(|e| e.to_string()))?;
    shared::trim_pool(dev);
    let tokens = cond.embeds.dims()[1];
    let cond = Arc::new(cond);
    shared::remember_conditioning(key, cond.clone());
    Ok((cond, tr!("node.qwen_image21_text_encoder.done", tokens = tokens, images = images.len())))
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21TextEncoder { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::QwenImage21TextEncoder { running, error, loaded_name, .. } => Some((*running, *error, *loaded_name)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        Box::new(Text::new(tr!("node.qwen_image21_text_encoder.hint")).class("flux-node-info")),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_text_encoder.busy"), "flux-node-running"),
        ),
    ]))
}
