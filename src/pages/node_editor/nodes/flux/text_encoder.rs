use std::result::Result;
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{DataBlob, FluxBlob, FluxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, status_row};
use crate::pages::node_editor::controls::dropdown_field::node_dropdown_field;
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_model, current_input_text, seq_len_of, shared, SEQ_LEN_OPTIONS};

pub struct TextEncoderExec;

impl NodeExecutor for TextEncoderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("prompt");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::FluxTextEncoder { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(c) => PortValue::Data(Arc::new(DataBlob::Flux(FluxBlob::Conditioning(c)))),
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
            NodeRuntime::FluxTextEncoder { seq_len_idx, running, error, loaded_name, out, output_version } => {
                Some((*seq_len_idx, *running, *error, *loaded_name, out.clone(), *output_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((seq_len_idx, running, error, loaded_name, out, output_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.flux.common.connect_checkpoint")));
        return;
    };
    let Some(prompt) = current_input_text(ctx, node.id, "prompt").filter(|p| !p.trim().is_empty()) else {
        error.set(Some(tr!("node.flux_text_encoder.connect_prompt")));
        return;
    };
    let seq_idx = seq_len_idx.get_untracked();

    running.set(true);
    error.set(None);
    loaded_name.set(None);

    let _ = thread::Builder::new().name("synthos-flux-encode".into()).spawn(move || {
        let started = log_worker_start(
            "flux-text-encoder",
            &format!("промпт {} симв., T5 {}", prompt.chars().count(), SEQ_LEN_OPTIONS[seq_idx.min(2)]),
        );
        let res = worker(&handle, &prompt, seq_idx);
        log_worker_done("flux-text-encoder", started, &res.as_ref().map(|(_, cached)| *cached));
        match res {
            Ok((cond, cached)) => {
                if let Ok(mut g) = out.lock() {
                    *g = Some(cond);
                }
                error.set(None);
                loaded_name.set(Some(if cached {
                    tr!("node.flux_text_encoder.cached")
                } else {
                    "CLIP-L + T5-XXL".into()
                }));
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

/// `(кондиционирование, взято_из_кэша)`.
fn worker(
    handle: &FluxModelHandle,
    prompt: &str,
    seq_idx: usize,
) -> Result<(Arc<synaptix_image_flux::FluxConditioning>, bool), String> {
    let model = shared::load_model(handle)?;
    let seq = seq_len_of(seq_idx, model.model.default_max_seq_len());
    let key = shared::cond_key(handle, prompt, seq);
    if let Some(c) = shared::cached_conditioning(&key) {
        return Ok((c, true));
    }
    let cond = model.model.encode_prompt(prompt, seq).map_err(|e| e.to_string())?;
    shared::trim_pool(model.model.device());
    let cond = Arc::new(cond);
    shared::remember_conditioning(key, cond.clone());
    Ok((cond, false))
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxTextEncoder { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxTextEncoder { seq_len_idx, running, error, loaded_name, .. } => {
                Some((*seq_len_idx, *running, *error, *loaded_name))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((seq_len_idx, running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row(&tr!("node.flux_text_encoder.seq_len"), node_dropdown_field(SEQ_LEN_OPTIONS, seq_len_idx)),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(running, error, loaded_name, tr!("node.flux_text_encoder.busy"), "flux-node-running"),
                ),
            ]),
    )
}
