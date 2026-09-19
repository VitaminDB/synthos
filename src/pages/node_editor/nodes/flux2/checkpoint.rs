use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, Flux2Blob, FluxModelHandle, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_toggle};
use super::{DEVICE_OPTIONS, MEMORY_MODE_OPTIONS, QUANT_OPTIONS};
use crate::pages::node_editor::controls::dropdown_field::node_dropdown_field;
use crate::pages::node_editor::controls::file_picker::node_file_picker;

/// Бандл выбирается файлом; каталог diffusers агент может вписать путём.
const SYN_FILTER: &[(&str, &[&str])] = &[("Syn bundle", &["syn"])];

pub struct CheckpointExec;

impl NodeExecutor for CheckpointExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Flux2Checkpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, handle_cache } => {
                    let get_u = |s: &RwSignal<usize>| if track { s.get() } else { s.get_untracked() };
                    let path = if track { model_path.get() } else { model_path.get_untracked() };
                    let handle = path.map(|model_path| FluxModelHandle {
                        model_path,
                        device_idx: get_u(device_idx),
                        quant_idx: get_u(quant_idx),
                        memory_mode_idx: get_u(memory_mode_idx),
                        resident: if track { resident.get() } else { resident.get_untracked() },
                    });
                    match handle {
                        Some(h) => {
                            // Тот же `Arc`, пока настройки не менялись.
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
                            if !arc.resident {
                                super::shared::release_hold();
                            }
                            PortValue::Data(Arc::new(DataBlob::Flux2(Flux2Blob::Model(arc))))
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
            NodeRuntime::Flux2Checkpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. } => {
                Some((*model_path, *device_idx, *quant_idx, *memory_mode_idx, *resident))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((model_path, device_idx, quant_idx, memory_mode_idx, resident)) = snapshot else {
        return Box::new(Column::new());
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("nodes.common.model"),
            node_file_picker(tr!("node.flux2_checkpoint.model_tooltip"), model_path, SYN_FILTER, |_| {}),
        ),
        field_row(&tr!("node.flux_checkpoint.device"), node_dropdown_field(DEVICE_OPTIONS, device_idx)),
        field_row(&tr!("node.flux_checkpoint.quant"), node_dropdown_field(QUANT_OPTIONS, quant_idx)),
        field_row(&tr!("node.flux_checkpoint.memory_mode"), node_dropdown_field(MEMORY_MODE_OPTIONS, memory_mode_idx)),
        // DiT остаётся в VRAM между прогонами.
        field_row(&tr!("nodes.common.keep_in_memory"), make_toggle(resident)),
        Box::new(Text::new(tr!("node.flux2_checkpoint.hint")).class("flux-node-info")),
    ];
    Box::new(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows))
}
