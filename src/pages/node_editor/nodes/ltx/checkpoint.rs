//! `LtxCheckpoint` — нода-источник хэндла чекпойнта (ComfyUI-стиль).
//!
//! Не грузит веса: собирает [`LtxModelHandle`] (пути + device/quant/compute)
//! и публикует его в порт `model`. Подмодели грузят ноды-потребители через
//! `shared`-кэш по ключу из хэндла. `handle_cache` переиспользует Arc, пока
//! параметры не изменились — иначе `PortValue::Data` (ptr_eq) считал бы
//! значение новым на каждый evaluate и зря пересчитывал downstream.

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, LtxBlob, LtxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_dropdown, make_slider_row, make_toggle};
use super::{dir_picker_row, COMPUTE_OPTIONS, DEVICE_OPTIONS, QUANT_DIT_OPTIONS, QUANT_ENC_OPTIONS};
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct CheckpointExec;

impl NodeExecutor for CheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxCheckpoint {
                    model_path,
                    gemma_dir,
                    upscaler_path,
                    lora_path,
                    lora_strength,
                    device_idx,
                    quant_dit_idx,
                    quant_enc_idx,
                    compute_idx,
                    resident,
                    handle_cache,
                } => {
                    let (mp, gd) = if track {
                        (model_path.get(), gemma_dir.get())
                    } else {
                        (model_path.get_untracked(), gemma_dir.get_untracked())
                    };
                    let (up, lp, ls, di, qd, qe, ci, res) = if track {
                        (
                            upscaler_path.get(),
                            lora_path.get(),
                            lora_strength.get(),
                            device_idx.get(),
                            quant_dit_idx.get(),
                            quant_enc_idx.get(),
                            compute_idx.get(),
                            resident.get(),
                        )
                    } else {
                        (
                            upscaler_path.get_untracked(),
                            lora_path.get_untracked(),
                            lora_strength.get_untracked(),
                            device_idx.get_untracked(),
                            quant_dit_idx.get_untracked(),
                            quant_enc_idx.get_untracked(),
                            compute_idx.get_untracked(),
                            resident.get_untracked(),
                        )
                    };
                    match (mp, gd) {
                        (Some(model_path), Some(gemma_dir)) => {
                            let handle = LtxModelHandle {
                                model_path,
                                gemma_dir,
                                upscaler_path: up,
                                lora_path: lp,
                                lora_strength: ls,
                                device_idx: di,
                                quant_dit_idx: qd,
                                quant_enc_idx: qe,
                                compute_idx: ci,
                                resident: res,
                            };
                            let arc = match handle_cache.lock() {
                                Ok(mut g) => match g.as_ref() {
                                    Some(prev) if **prev == handle => prev.clone(),
                                    _ => {
                                        let a = Arc::new(handle);
                                        *g = Some(a.clone());
                                        a
                                    }
                                },
                                Err(_) => Arc::new(handle),
                            };
                            PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::Model(arc))))
                        }
                        _ => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("model", pv);
    }
}

/// Предупреждение о несовпадении чекпойнта и расписания.
///
/// Стадии идут по `DISTILLED_SIGMAS` (8 шагов stage1 + 3 stage2) — числа
/// шагов у ноды нет. На `ltx-2.3-22b-dev` восьми шагов не хватает, и видео
/// выходит размытым; по одному имени файла в пикере это не очевидно.
fn distilled_hint(model_path: RwSignal<Option<std::path::PathBuf>>) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(p) = model_path.get() else {
            return vec![];
        };
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.contains("distilled") {
            return vec![];
        }
        vec![Box::new(
            Text::new(
                "Стадии рассчитаны на distilled-чекпойнт (8+3 шага). \
                 С dev-моделью видео выйдет размытым.",
            )
            .class("node-card-field-error"),
        )]
    }))
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxCheckpoint {
                model_path,
                gemma_dir,
                upscaler_path,
                lora_path,
                lora_strength,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                resident,
                ..
            } => Some((
                *model_path,
                *gemma_dir,
                *upscaler_path,
                *lora_path,
                *lora_strength,
                *device_idx,
                *quant_dit_idx,
                *quant_enc_idx,
                *compute_idx,
                *resident,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        model_path,
        gemma_dir,
        upscaler_path,
        lora_path,
        lora_strength,
        device_idx,
        quant_dit_idx,
        quant_enc_idx,
        compute_idx,
        resident,
    )) = snapshot
    else {
        return Box::new(Text::new("LtxCheckpoint: некорректный runtime").class("node-card-field-error"));
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            "Чекпойнт",
            node_file_picker(
                "LTX-2.3 .syn-бандл или .safetensors (DiT+VAE+vocoder+коннекторы)",
                model_path,
                &[("Syn bundle", &["syn"]), ("Safetensors", &["safetensors"])],
                |_| {},
            ),
        ),
        field_row(
            "Gemma",
            node_file_picker(
                "Gemma-3-12B (текст-энкодер) — .syn-бандл",
                gemma_dir,
                &[("Syn bundle", &["syn"])],
                |_| {},
            ),
        ),
        field_row(
            "Gemma HF-каталог",
            dir_picker_row("Или HF-каталог Gemma-3-12B (config.json + safetensors)", gemma_dir),
        ),
        field_row(
            "Upscaler",
            node_file_picker(
                "Spatial-upscaler ×2 — .syn-бандл или .safetensors (для two-stage)",
                upscaler_path,
                &[("Syn bundle", &["syn"]), ("Safetensors", &["safetensors"])],
                |_| {},
            ),
        ),
        field_row(
            "LoRA",
            node_file_picker(
                "LoRA-адаптер для мерджа в DiT — .syn или .safetensors (опционально)",
                lora_path,
                &[("Syn bundle", &["syn"]), ("Safetensors", &["safetensors"])],
                |_| {},
            ),
        ),
        field_row("LoRA strength", make_slider_row(lora_strength, 0.0, 2.0, 0.05, 2)),
        distilled_hint(model_path),
        field_row("Device", make_dropdown(DEVICE_OPTIONS, device_idx)),
        field_row("Quant DiT", make_dropdown(QUANT_DIT_OPTIONS, quant_dit_idx)),
        field_row("Quant Gemma", make_dropdown(QUANT_ENC_OPTIONS, quant_enc_idx)),
        field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)),
        // Держать DiT в VRAM после прогона: на 24 ГБ рядом с VAE-decode
        // может не хватить памяти — осознанный опт-ин.
        field_row("Держать в памяти", make_toggle(resident)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
