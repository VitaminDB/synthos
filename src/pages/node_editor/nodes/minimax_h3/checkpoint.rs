use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, H3Blob, H3ModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_dropdown, make_slider_row};
use super::{
    dir_picker_row, COMPUTE_OPTIONS, DEVICE_OPTIONS, MEMORY_MODE_OPTIONS, QUANT_DIT_OPTIONS,
    QUANT_ENC_OPTIONS, VARIANT_OPTIONS,
};
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct CheckpointExec;

impl NodeExecutor for CheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3Checkpoint {
                    model_dir,
                    encoder_dir,
                    lora_path,
                    lora_strength,
                    variant_idx,
                    device_idx,
                    quant_dit_idx,
                    quant_enc_idx,
                    compute_idx,
                    memory_mode_idx,
                    handle_cache,
                } => {
                    let md = if track { model_dir.get() } else { model_dir.get_untracked() };
                    let handle = md.map(|model_dir| H3ModelHandle {
                        model_dir,
                        encoder_dir: if track { encoder_dir.get() } else { encoder_dir.get_untracked() },
                        lora_path: if track { lora_path.get() } else { lora_path.get_untracked() },
                        lora_strength: if track {
                            lora_strength.get()
                        } else {
                            lora_strength.get_untracked()
                        },
                        variant_idx: if track { variant_idx.get() } else { variant_idx.get_untracked() },
                        device_idx: if track { device_idx.get() } else { device_idx.get_untracked() },
                        quant_dit_idx: if track {
                            quant_dit_idx.get()
                        } else {
                            quant_dit_idx.get_untracked()
                        },
                        quant_enc_idx: if track {
                            quant_enc_idx.get()
                        } else {
                            quant_enc_idx.get_untracked()
                        },
                        compute_idx: if track { compute_idx.get() } else { compute_idx.get_untracked() },
                        memory_mode_idx: if track {
                            memory_mode_idx.get()
                        } else {
                            memory_mode_idx.get_untracked()
                        },
                    });
                    match handle {
                        Some(h) => {
                            let arc = match handle_cache.lock() {
                                Ok(mut g) => match g.as_ref() {
                                    Some(prev) if **prev == h => prev.clone(),
                                    _ => {
                                        let a = Arc::new(h);
                                        *g = Some(a.clone());
                                        a
                                    }
                                },
                                Err(_) => Arc::new(h),
                            };
                            PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Model(arc))))
                        }
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("model", pv);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Checkpoint {
                model_dir,
                encoder_dir,
                lora_path,
                lora_strength,
                variant_idx,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                memory_mode_idx,
                ..
            } => Some((
                *model_dir,
                *encoder_dir,
                *lora_path,
                *lora_strength,
                *variant_idx,
                *device_idx,
                *quant_dit_idx,
                *quant_enc_idx,
                *compute_idx,
                *memory_mode_idx,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        model_dir,
        encoder_dir,
        lora_path,
        lora_strength,
        variant_idx,
        device_idx,
        quant_dit_idx,
        quant_enc_idx,
        compute_idx,
        memory_mode_idx,
    )) = snapshot
    else {
        return Box::new(Column::new());
    };

    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            "Каталог модели",
            dir_picker_row("Корень MiniMax-H3 или каталог FL2VA/Ref2VA", model_dir),
        ),
        field_row("Вариант", make_dropdown(VARIANT_OPTIONS, variant_idx)),
        field_row(
            "Энкодер",
            dir_picker_row("Каталог Qwen3-VL-32B (по умолчанию text_encoder)", encoder_dir),
        ),
        field_row(
            "LoRA",
            node_file_picker(
                "Turbo LoRA (.safetensors) — 4-8 шагов",
                lora_path,
                &[("Safetensors", &["safetensors"])],
                |_| {},
            ),
        ),
        field_row("Сила LoRA", make_slider_row(lora_strength, 0.0, 2.0, 0.05, 2)),
        field_row("Устройство", make_dropdown(DEVICE_OPTIONS, device_idx)),
        field_row("Квант DiT", make_dropdown(QUANT_DIT_OPTIONS, quant_dit_idx)),
        field_row("Квант энкодера", make_dropdown(QUANT_ENC_OPTIONS, quant_enc_idx)),
        field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)),
        field_row("Память", make_dropdown(MEMORY_MODE_OPTIONS, memory_mode_idx)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
