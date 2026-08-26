use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_minimax_h3 as h3;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, H3Blob, H3Conditioning, H3Keyframe, H3ModelHandle, NodeInstance, NodeRuntime,
    PortValue,
};
use super::super::acestep::{field_row, status_row};
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_keyframe, current_input_model, current_input_text, shared};

pub struct TextEncoderExec;

impl NodeExecutor for TextEncoderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("prompt");
        let _ = ctx.read_input("keyframe");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3TextEncoder { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(c) => PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Conditioning(c)))),
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
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3TextEncoder { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3TextEncoder { running, error, loaded_name, out, output_version } => {
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
        error.set(Some(tr!("node.minimax_h3.common.connect_checkpoint_model")));
        return;
    };
    let prompt = current_input_text(ctx, node.id, "prompt").unwrap_or_default();
    let keyframes: Vec<Arc<H3Keyframe>> = ["keyframe", "keyframe_last"]
        .iter()
        .filter_map(|p| current_input_keyframe(ctx, node.id, p))
        .collect();

    running.set(true);
    error.set(None);
    loaded_name.set(None);

    let _ = thread::Builder::new()
        .name("synthos-h3-encode".into())
        .spawn(move || {
            let started = log_worker_start(
                "h3-text-encoder",
                &format!("промпт {} симв., keyframes {}", prompt.chars().count(), keyframes.len()),
            );
            let res = worker(&handle, &prompt, &keyframes);
            log_worker_done("h3-text-encoder", started, &res);
            match res {
                Ok(cond) => {
                    if let Ok(mut g) = out.lock() {
                        *g = Some(Arc::new(cond));
                    }
                    error.set(None);
                    loaded_name.set(Some("Qwen3-VL".into()));
                    run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

fn worker(
    handle: &H3ModelHandle,
    prompt: &str,
    keyframes: &[Arc<H3Keyframe>],
) -> std::result::Result<H3Conditioning, String> {
    let cond = {
        let enc = shared::load_encoder(handle)?;
        let e = &enc.encoder;

        let mut images = Vec::with_capacity(keyframes.len());
        for kf in keyframes {
            let prepared = e.prepare_image(&kf.image).map_err(|x| x.to_string())?;
            images.push(prepared);
        }
        let grids: Vec<_> = images.iter().map(|(_, g)| *g).collect();
        let presentation = if grids.is_empty() {
            h3::text_encoder::presentation_t2va(prompt)
        } else {
            h3::text_encoder::presentation_fl2va(prompt, &grids, e.merge_size())
        };
        e.encode(&presentation, &images).map_err(|x| x.to_string())?
    };
    shared::trim_pool(handle);
    Ok(H3Conditioning { hidden: cond.hidden, tags: cond.tags })
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3TextEncoder { running, error, loaded_name, .. } => {
                Some((*running, *error, *loaded_name))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![field_row(
                &tr!("nodes.common.status"),
                status_row(running, error, loaded_name, tr!("node.minimax_h3_text_encoder.busy"), "h3-node-running"),
            )]),
    )
}
