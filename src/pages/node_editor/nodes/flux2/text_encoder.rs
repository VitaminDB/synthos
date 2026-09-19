use std::result::Result;
use std::sync::Arc;
use std::thread;

use synaptix_image_flux2::{Flux2Conditioning, Flux2Variant};
use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, Flux2Blob, FluxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, status_row};
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_model, current_input_text, shared};

pub struct TextEncoderExec;

impl NodeExecutor for TextEncoderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("prompt");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Flux2TextEncoder { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(c) => PortValue::Data(Arc::new(DataBlob::Flux2(Flux2Blob::Conditioning(c)))),
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
            NodeRuntime::Flux2TextEncoder { running, error, loaded_name, out, output_version } => {
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
    let Some(prompt) = current_input_text(ctx, node.id, "prompt").filter(|p| !p.trim().is_empty()) else {
        error.set(Some(tr!("node.flux_text_encoder.connect_prompt")));
        return;
    };

    running.set(true);
    error.set(None);
    loaded_name.set(None);

    let _ = thread::Builder::new().name("synthos-flux2-encode".into()).spawn(move || {
        let started = log_worker_start("flux2-text-encoder", &format!("промпт {} симв.", prompt.chars().count()));
        let res = worker(&handle, &prompt);
        log_worker_done("flux2-text-encoder", started, &res.as_ref().map(|(_, n)| n.clone()));
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
fn worker(handle: &FluxModelHandle, prompt: &str) -> Result<(Arc<Flux2Conditioning>, String), String> {
    let model = shared::load_model(handle)?;
    let key = shared::cond_key(handle, prompt);
    if let Some(c) = shared::cached_conditioning(&key) {
        return Ok((c, tr!("node.flux_text_encoder.cached")));
    }
    let m = &model.model;
    // Пустой промпт для CFG нужен только недистиллированному klein.
    let with_negative = m.variant() == Flux2Variant::KleinBase;
    let cond = m.encode_prompt(prompt, with_negative).map_err(|e| e.to_string())?;
    shared::trim_pool(m.device());
    let cond = Arc::new(cond);
    shared::remember_conditioning(key, cond.clone());
    let te = m.text_encoder_config();
    let name = match te.arch {
        synaptix_image_flux2::config::TextEncoderArch::Mistral3 => "Mistral-Small-3.1".to_string(),
        synaptix_image_flux2::config::TextEncoderArch::Qwen3 => format!("Qwen3 ({} слоёв)", te.num_layers),
    };
    Ok((cond, name))
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2TextEncoder { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Flux2TextEncoder { running, error, loaded_name, .. } => Some((*running, *error, *loaded_name)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![
        Box::new(Text::new(tr!("node.flux2_text_encoder.hint")).class("flux-node-info")),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, tr!("node.flux_text_encoder.busy"), "flux-node-running"),
        ),
    ]))
}
