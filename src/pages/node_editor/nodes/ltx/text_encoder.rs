//! `LtxTextEncoder` — Gemma-3-12B (49 hidden-states) + перцивер-коннекторы.
//! `(model, prompt) → (video_encoding [1,T,4096], audio_encoding [1,T,2048])`.
//!
//! VRAM-цикл как в CLI: pinned-staging на время encode, Gemma дропается ДО
//! загрузки коннекторов (bf16-Gemma 24GB иначе не помещается вместе с ними),
//! затем sync + hard-trim пула. «Держать Gemma» продлевает жизнь инстанса в
//! Weak-кэше (`gemma_keep` strong-ref) — повторные прогоны без перезагрузки,
//! ценой VRAM под DiT.

use std::sync::Arc;
use std::thread;

use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_llm_gemma3::pipeline::GemmaPipeline;
use synaptix_video_ltx23::text_encoder::{AudioTextConditioner, VideoTextConditioner};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, LtxBlob, LtxModelHandle, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_text_field, make_toggle, status_row};
use super::shared::{self, GEMMA_CTX};
use super::{compute_from_idx, current_input_model, current_input_text, device_from_idx, progress_row};

pub struct TextEncoderExec;

impl NodeExecutor for TextEncoderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _prompt = ctx.read_input("prompt");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxTextEncoder {
                    v_out,
                    a_out,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    let v = match v_out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(t) => PortValue::Data(Arc::new(DataBlob::Ltx(
                                LtxBlob::VideoEncoding(t.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    };
                    let a = match a_out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(t) => PortValue::Data(Arc::new(DataBlob::Ltx(
                                LtxBlob::AudioEncoding(t.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    };
                    (v, a)
                }
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("video_encoding", v_pv);
        ctx.write_output("audio_encoding", a_pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxTextEncoder { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxTextEncoder {
                prompt_field,
                keep_gemma,
                running,
                error,
                progress_pct,
                ..
            } => Some((*prompt_field, *keep_gemma, *running, *error, *progress_pct)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((prompt_field, keep_gemma, running, error, progress_pct)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxTextEncoder"))
                .class("node-card-field-error"),
        );
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("node.ltx_text_encoder.field.prompt_fallback"),
            make_text_field(prompt_field, tr!("node.ltx_text_encoder.placeholder.prompt")),
        ),
        field_row(&tr!("node.ltx_text_encoder.field.keep_gemma"), make_toggle(keep_gemma)),
        field_row(&tr!("node.ltx.common.progress"), progress_row(running, progress_pct)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, loaded_name, "Gemma encode…", "ltx-node-running"),
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
            NodeRuntime::LtxTextEncoder {
                prompt_field,
                keep_gemma,
                gemma_keep,
                running,
                error,
                progress_pct,
                v_out,
                a_out,
                output_version,
            } => Some((
                *prompt_field,
                *keep_gemma,
                gemma_keep.clone(),
                *running,
                *error,
                *progress_pct,
                v_out.clone(),
                a_out.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        prompt_field,
        keep_gemma,
        gemma_keep,
        running,
        error,
        progress_pct,
        v_out,
        a_out,
        output_version,
    )) = snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.ltx.common.connect_checkpoint_model")));
        return;
    };
    let prompt = current_input_text(ctx, node.id, "prompt")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| prompt_field.get_untracked());
    if prompt.trim().is_empty() {
        error.set(Some(tr!("node.ltx_text_encoder.err.empty_prompt")));
        return;
    }
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    let keep = keep_gemma.get_untracked();

    let _ = thread::Builder::new()
        .name("synthos-ltx-text-encoder".into())
        .spawn(move || {
            let r = worker(
                &handle,
                &prompt,
                keep,
                &gemma_keep,
                progress_pct,
                &v_out,
                &a_out,
            );
            match r {
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
    keep: bool,
    gemma_keep: &Arc<Mutex<Option<Arc<GemmaPipeline>>>>,
    progress_pct: RwSignal<f32>,
    v_out: &Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
    a_out: &Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let compute = compute_from_idx(handle.compute_idx);
    let set_pct = |pct: f32| {
        run_on_main_thread(move || progress_pct.set(pct));
    };

    synaptix_core::device::cuda::set_offload_pinned(true);
    let encode_result = (|| -> std::result::Result<(Vec<synaptix_core::tensor::Tensor>, Vec<u32>), String> {
        let gemma = shared::load_gemma(handle)?;
        set_pct(0.4);
        let (states, mask) = gemma
            .encode_for_ltx(prompt, GEMMA_CTX, dev)
            .map_err(|e| format!("Gemma encode: {e}"))?;
        set_pct(0.6);
        if let Ok(mut g) = gemma_keep.lock() {
            *g = if keep { Some(gemma.clone()) } else { None };
        }
        Ok((states, mask))
    })();
    let (states, mask) = match encode_result {
        Ok(r) => r,
        Err(e) => {
            synaptix_core::device::cuda::set_offload_pinned(false);
            return Err(e);
        }
    };
    if !keep {
        shared::sync_and_trim(dev);
    }

    let result = (|| -> std::result::Result<(), String> {
        let ckpt = shared::load_ckpt(handle)?;
        let ckpt_gpu = ckpt.view_on(dev);
        // Видео-коннектор (~4,8 ГБ весов) отпускается до загрузки аудио — на
        // малой карте оба сразу не помещаются.
        let v = {
            let vtc = VideoTextConditioner::load(&ckpt_gpu, dev, compute)
                .map_err(|e| tr!("node.ltx.common.video_connector", error = e))?;
            vtc.forward(&states, &mask)
                .map_err(|e| tr!("node.ltx.common.video_connector_forward", error = e))?
        };
        shared::sync_and_trim(dev);
        set_pct(0.8);
        let a = AudioTextConditioner::load(&ckpt_gpu, dev, compute)
            .map_err(|e| tr!("node.ltx_text_encoder.err.audio_connector", error = e))?
            .forward(&states, &mask)
            .map_err(|e| tr!("node.ltx_text_encoder.err.audio_connector_forward", error = e))?;
        set_pct(1.0);
        if let Ok(mut g) = v_out.lock() {
            *g = Some(v);
        }
        if let Ok(mut g) = a_out.lock() {
            *g = Some(a);
        }
        Ok(())
    })();
    synaptix_core::device::cuda::set_offload_pinned(false);
    // Коннекторы и активации отпущены — вернуть память драйверу, иначе на
    // малой карте следующей стадии (NAG, DiT) её не хватает.
    shared::sync_and_trim(dev);
    result
}
