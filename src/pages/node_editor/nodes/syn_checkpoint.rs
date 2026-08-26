//! `SynCheckpoint` — универсальная нода-источник модели (ComfyUI-стиль) для
//! слот-семейств: LLM, VoxCPM2, OmniVoice, ASR GigaAM, Sortformer.
//!
//! Как и семейные чекпойнты (LTX/H3/ACE-Step), весов не грузит: собирает
//! [`SynModelHandle`] (путь + предпочтения device/storage/compute +
//! резидентность) и публикует его в порт `model`. Загрузка — у ноды-
//! потребителя, которая маппит предпочтения на свои family-опции
//! (Auto = дефолт семейства).
//!
//! Чекбокс «Держать в памяти» — «слотовое» поведение: модель остаётся в
//! слоте потребителя после прогона (как слот-ноды вели себя всегда).
//! Выключен — потребитель очищает слот по завершении прогона и VRAM
//! возвращается (полезно перед тяжёлым видео-пайплайном).

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{DataBlob, NodeInstance, NodeRuntime, PortValue, SynModelHandle};
use super::acestep::{field_row, make_dropdown, make_toggle};
use super::ltx::dir_picker_row;
use crate::pages::node_editor::controls::file_picker::node_file_picker;

/// Индекс 0 везде — «Auto»: потребитель подставляет дефолт своего семейства.
pub const DEVICE_PREF_OPTIONS: &[&str] = &["Auto", "CUDA", "CPU"];
pub const STORAGE_PREF_OPTIONS: &[&str] = &["Auto", "F16", "BF16", "FP8", "NVFP4"];
pub const COMPUTE_PREF_OPTIONS: &[&str] = &["Auto", "F16", "BF16", "F32"];

pub struct SynCheckpointExec;

impl NodeExecutor for SynCheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::SynCheckpoint {
                    model_path,
                    device_idx,
                    storage_idx,
                    compute_idx,
                    resident,
                    handle_cache,
                } => {
                    let (mp, di, si, ci, res) = if track {
                        (
                            model_path.get(),
                            device_idx.get(),
                            storage_idx.get(),
                            compute_idx.get(),
                            resident.get(),
                        )
                    } else {
                        (
                            model_path.get_untracked(),
                            device_idx.get_untracked(),
                            storage_idx.get_untracked(),
                            compute_idx.get_untracked(),
                            resident.get_untracked(),
                        )
                    };
                    match mp {
                        Some(model_path) => {
                            let handle = SynModelHandle {
                                model_path,
                                device_idx: di,
                                storage_idx: si,
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
                            PortValue::Data(Arc::new(DataBlob::SynModel(arc)))
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
            NodeRuntime::SynCheckpoint {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                resident,
                ..
            } => Some((*model_path, *device_idx, *storage_idx, *compute_idx, *resident)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((model_path, device_idx, storage_idx, compute_idx, resident)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "SynCheckpoint"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("nodes.common.model"),
            node_file_picker(
                tr!("node.syn_checkpoint.picker.model_bundle"),
                model_path,
                &[("Syn bundle", &["syn"])],
                |_| {},
            ),
        ),
        field_row(
            &tr!("node.syn_checkpoint.hf_dir"),
            dir_picker_row(tr!("node.syn_checkpoint.picker.hf_dir"), model_path),
        ),
        field_row("Device", make_dropdown(DEVICE_PREF_OPTIONS, device_idx)),
        field_row("Storage", make_dropdown(STORAGE_PREF_OPTIONS, storage_idx)),
        field_row("Compute", make_dropdown(COMPUTE_PREF_OPTIONS, compute_idx)),
        // «Слотовое» поведение: держать модель в памяти между прогонами.
        field_row(&tr!("nodes.common.keep_in_memory"), make_toggle(resident)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
