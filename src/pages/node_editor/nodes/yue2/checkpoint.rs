//! `Yue2Checkpoint` — источник хэндла чекпойнта YuE2.
//!
//! Весов не грузит: собирает пути (каталог моделей плюс опциональные
//! override'ы костяка и декодера) и точности, отдавая их порту `model`.
//! Хэндл кэшируется, пока параметры не изменились: иначе `PortValue::Data`
//! на каждом пересчёте считался бы новым значением и зря будил downstream.

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{
    DataBlob, NodeInstance, NodeRuntime, PortValue, Yue2Blob, Yue2ModelHandle,
};
use super::shared::default_bundle_name;
use super::{
    dir_picker_row, field_row, make_dropdown, make_toggle, COMPUTE_OPTIONS, DEVICE_OPTIONS,
    QUANT_OPTIONS, VAE_DTYPE_OPTIONS,
};
use crate::pages::node_editor::controls::file_picker::node_file_picker_placeholder;
use synaptix_music_yue2::pipeline::{MODEL_NAMES, VAE_NAMES};

const SYN_FILTER: &[(&str, &[&str])] = &[("Syn bundle", &["syn"])];

/// Плейсхолдер override-поля: имя бандла, которое возьмётся из текущего
/// каталога, если файл не выбран.
fn default_bundle_hint(
    models_dir: RwSignal<Option<std::path::PathBuf>>,
    names: &'static [&'static str],
) -> impl Fn() -> String + Send + Sync + 'static {
    move || {
        let dir = models_dir.get();
        let name = default_bundle_name(dir.as_deref(), names);
        tr!("node.yue2_checkpoint.default_bundle", name = name)
    }
}

pub struct CheckpointExec;

impl NodeExecutor for CheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Yue2Checkpoint {
                    models_dir,
                    model_path,
                    vae_path,
                    device_idx,
                    quant_idx,
                    compute_idx,
                    vae_dtype_idx,
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
                    let model = get(model_path);
                    let vae = get(vae_path);
                    // Хэндл валиден, если задан каталог ИЛИ оба override'а.
                    if dir.is_none() && !(model.is_some() && vae.is_some()) {
                        PortValue::Empty
                    } else {
                        let handle = Yue2ModelHandle {
                            models_dir: dir,
                            model_path: model,
                            vae_path: vae,
                            device_idx: geti(device_idx),
                            quant_idx: geti(quant_idx),
                            compute_idx: geti(compute_idx),
                            vae_dtype_idx: geti(vae_dtype_idx),
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
                        PortValue::Data(Arc::new(DataBlob::Yue2(Yue2Blob::Model(arc))))
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
            NodeRuntime::Yue2Checkpoint {
                models_dir,
                model_path,
                vae_path,
                device_idx,
                quant_idx,
                compute_idx,
                vae_dtype_idx,
                resident,
                ..
            } => Some((
                *models_dir,
                *model_path,
                *vae_path,
                *device_idx,
                *quant_idx,
                *compute_idx,
                *vae_dtype_idx,
                *resident,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        models_dir,
        model_path,
        vae_path,
        device_idx,
        quant_idx,
        compute_idx,
        vae_dtype_idx,
        resident,
    )) = snapshot
    else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "Yue2Checkpoint"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("node.yue2_checkpoint.field.models_dir"),
            dir_picker_row(tr!("node.yue2_checkpoint.tooltip.models_dir"), models_dir),
        ),
        field_row(
            &tr!("node.yue2_checkpoint.field.model"),
            node_file_picker_placeholder(
                tr!("node.yue2_checkpoint.tooltip.model_override"),
                model_path,
                SYN_FILTER,
                default_bundle_hint(models_dir, MODEL_NAMES),
                |_| {},
            ),
        ),
        field_row(
            &tr!("node.yue2_checkpoint.field.vae"),
            node_file_picker_placeholder(
                tr!("node.yue2_checkpoint.tooltip.vae_override"),
                vae_path,
                SYN_FILTER,
                default_bundle_hint(models_dir, VAE_NAMES),
                |_| {},
            ),
        ),
        field_row("Device", make_dropdown(DEVICE_OPTIONS, device_idx)),
        field_row("Quant", make_dropdown(QUANT_OPTIONS, quant_idx)),
        field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)),
        field_row(
            &tr!("node.yue2_checkpoint.field.vae_dtype"),
            make_dropdown(VAE_DTYPE_OPTIONS, vae_dtype_idx),
        ),
        field_row(&tr!("nodes.common.keep_in_memory"), make_toggle(resident)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
