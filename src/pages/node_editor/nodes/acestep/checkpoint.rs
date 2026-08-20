//! `AceStepCheckpoint` — нода-источник хэндла чекпойнта (ComfyUI-стиль).
//!
//! Не грузит веса: собирает [`AceStepModelHandle`] (каталог моделей +
//! 4 опц. override + device/quant/compute) и публикует его в порт `model`.
//! Подмодели грузит Generate-нода через `generate_music` (sequential-drop
//! под 24GB). `handle_cache` переиспользует Arc, пока параметры не
//! изменились — иначе `PortValue::Data` (ptr_eq) считал бы значение новым
//! на каждый evaluate и зря пересчитывал downstream.

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{
    AceStepBlob, AceStepModelHandle, DataBlob, NodeInstance, NodeRuntime, PortValue,
};
use super::{
    dir_picker_row, field_row, make_dropdown, make_toggle, COMPUTE_OPTIONS, DEVICE_OPTIONS,
    QUANT_OPTIONS,
};
use crate::pages::node_editor::controls::file_picker::node_file_picker;

const SYN_FILTER: &[(&str, &[&str])] = &[("Syn bundle", &["syn"])];

pub struct CheckpointExec;

impl NodeExecutor for CheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AceStepCheckpoint {
                    models_dir,
                    lm_path,
                    text_encoder_path,
                    dit_path,
                    vae_path,
                    device_idx,
                    quant_dit_idx,
                    quant_enc_idx,
                    compute_idx,
                    resident,
                    handle_cache,
                } => {
                    let get = |s: &RwSignal<Option<std::path::PathBuf>>| {
                        if track {
                            s.get()
                        } else {
                            s.get_untracked()
                        }
                    };
                    let geti = |s: &RwSignal<usize>| {
                        if track {
                            s.get()
                        } else {
                            s.get_untracked()
                        }
                    };
                    let dir = get(models_dir);
                    let lm = get(lm_path);
                    let te = get(text_encoder_path);
                    let dit = get(dit_path);
                    let vae = get(vae_path);
                    // Хэндл валиден, если задан каталог ИЛИ все 4 override'а.
                    let all_overrides =
                        lm.is_some() && te.is_some() && dit.is_some() && vae.is_some();
                    if dir.is_none() && !all_overrides {
                        PortValue::Empty
                    } else {
                        let handle = AceStepModelHandle {
                            models_dir: dir,
                            lm_path: lm,
                            text_encoder_path: te,
                            dit_path: dit,
                            vae_path: vae,
                            device_idx: geti(device_idx),
                            quant_dit_idx: geti(quant_dit_idx),
                            quant_enc_idx: geti(quant_enc_idx),
                            compute_idx: geti(compute_idx),
                            resident: if track {
                                resident.get()
                            } else {
                                resident.get_untracked()
                            },
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
                        PortValue::Data(Arc::new(DataBlob::AceStep(AceStepBlob::Model(arc))))
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
            NodeRuntime::AceStepCheckpoint {
                models_dir,
                lm_path,
                text_encoder_path,
                dit_path,
                vae_path,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                resident,
                ..
            } => Some((
                *models_dir,
                *lm_path,
                *text_encoder_path,
                *dit_path,
                *vae_path,
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
        models_dir,
        lm_path,
        text_encoder_path,
        dit_path,
        vae_path,
        device_idx,
        quant_dit_idx,
        quant_enc_idx,
        compute_idx,
        resident,
    )) = snapshot
    else {
        return Box::new(
            Text::new("AceStepCheckpoint: некорректный runtime").class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            "Каталог моделей",
            dir_picker_row("Каталог с 4 .syn-бандлами (lm / text-enc / dit / vae)", models_dir),
        ),
        field_row(
            "LM (override)",
            node_file_picker("5Hz AR LM .syn (опц. — иначе из каталога)", lm_path, SYN_FILTER, |_| {}),
        ),
        field_row(
            "Text-enc (override)",
            node_file_picker("Text-encoder .syn (опц.)", text_encoder_path, SYN_FILTER, |_| {}),
        ),
        field_row(
            "DiT (override)",
            node_file_picker("DiT-бандл .syn (опц.)", dit_path, SYN_FILTER, |_| {}),
        ),
        field_row(
            "VAE (override)",
            node_file_picker("VAE .syn (опц.)", vae_path, SYN_FILTER, |_| {}),
        ),
        field_row("Device", make_dropdown(DEVICE_OPTIONS, device_idx)),
        field_row("Quant DiT", make_dropdown(QUANT_OPTIONS, quant_dit_idx)),
        field_row("Quant Enc", make_dropdown(QUANT_OPTIONS, quant_enc_idx)),
        field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)),
        // Generate передаёт в generate_music резидентный кэш компонентов
        // (LM/TE/DiT/VAE): повторный прогон не платит загрузку и
        // квантизацию. Выключение чекбокса освобождает кэш на следующем
        // запуске; выгрузка — и из панели «Модели в памяти».
        field_row("Держать в памяти", make_toggle(resident)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
