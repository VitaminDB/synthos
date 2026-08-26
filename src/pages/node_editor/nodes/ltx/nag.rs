//! `LtxNagPrompt` — NAG (Normalized Attention Guidance) negative-prompt.
//! `(model, text?) → nag: Data(Ltx::Nag)` — encoding той же Gemma (общий
//! Weak-кэш с Text Encoder → один инстанс при параллельном запуске) через
//! видео-коннектор + параметры scale/alpha/tau.
//!
//! Пустой промпт → выход Empty (NAG выключен), как `--nag-prompt ""` в CLI.

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::text_encoder::VideoTextConditioner;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, LtxBlob, LtxModelHandle, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_slider_row, make_text_field, status_row};
use super::shared::{self, GEMMA_CTX};
use super::{compute_from_idx, current_input_model, current_input_text, device_from_idx};

pub struct NagPromptExec;

impl NodeExecutor for NagPromptExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _text = ctx.read_input("text");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxNagPrompt {
                    scale,
                    alpha,
                    tau,
                    out,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    let (s, a, t) = if track {
                        (scale.get(), alpha.get(), tau.get())
                    } else {
                        (scale.get_untracked(), alpha.get_untracked(), tau.get_untracked())
                    };
                    match out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(enc) => PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::Nag {
                                encoding: enc.clone(),
                                scale: s,
                                alpha: a,
                                tau: t,
                            }))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("nag", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxNagPrompt { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxNagPrompt {
                prompt_field,
                scale,
                alpha,
                tau,
                running,
                error,
                ..
            } => Some((*prompt_field, *scale, *alpha, *tau, *running, *error)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((prompt_field, scale, alpha, tau, running, error)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxNagPrompt")).class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("node.ltx_nag.field.prompt_label"),
            make_text_field(prompt_field, tr!("node.ltx_nag.field.prompt_placeholder")),
        ),
        field_row("Scale", make_slider_row(scale, 1.0, 20.0, 0.5, 1)),
        field_row("Alpha", make_slider_row(alpha, 0.0, 1.0, 0.05, 2)),
        field_row("Tau", make_slider_row(tau, 1.0, 5.0, 0.1, 1)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, "NAG encode…", "ltx-node-running"),
        ),
    ];
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
            NodeRuntime::LtxNagPrompt {
                prompt_field,
                running,
                error,
                out,
                output_version,
                ..
            } => Some((*prompt_field, *running, *error, out.clone(), *output_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((prompt_field, running, error, out, output_version)) = snapshot else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    let prompt = current_input_text(ctx, node.id, "text")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| prompt_field.get_untracked());
    if running.get_untracked() {
        return;
    }
    if prompt.trim().is_empty() {
        if let Ok(mut g) = out.lock() {
            *g = None;
        }
        output_version.update(|v| *v = v.wrapping_add(1));
        error.set(None);
        return;
    }
    running.set(true);
    error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-ltx-nag".into())
        .spawn(move || {
            match worker(&handle, &prompt, &out) {
                Ok(()) => {
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

fn worker(
    handle: &LtxModelHandle,
    prompt: &str,
    out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let compute = compute_from_idx(handle.compute_idx);
    synaptix_core::device::cuda::set_offload_pinned(true);
    let result = (|| -> std::result::Result<(), String> {
        let (states, mask) = {
            let gemma = shared::load_gemma(handle)?;
            gemma
                .encode_for_ltx(prompt, GEMMA_CTX, dev)
                .map_err(|e| format!("Gemma encode nag: {e}"))?
        };
        let ckpt = shared::load_ckpt(handle)?;
        let ckpt_gpu = ckpt.view_on(dev);
        let enc = VideoTextConditioner::load(&ckpt_gpu, dev, compute)
            .map_err(|e| tr!("node.ltx.common.video_connector", error = e))?
            .forward(&states, &mask)
            .map_err(|e| tr!("node.ltx.common.video_connector_forward", error = e))?;
        if let Ok(mut g) = out.lock() {
            *g = Some(enc);
        }
        Ok(())
    })();
    synaptix_core::device::cuda::set_offload_pinned(false);
    result
}
