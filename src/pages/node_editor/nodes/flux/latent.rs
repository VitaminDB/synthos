//! FLUX Empty Latent — размер картинки. Стороны кратны 16 (VAE ×8 и упаковка
//! 2×2), по умолчанию 1024×1024; FLUX.1 обучен примерно до 2 Мп.

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::input::Slider;
use syngui::widgets::{Column, Dropdown, DropdownItem, Reactive};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, FluxBlob, FluxLatent, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, idx_in};
use crate::pages::node_editor::controls::utils::option_label;

pub use super::super::minimax_h3::latent::{aspect_of, ASPECT_OPTIONS};

pub const DIM_MIN: u32 = 256;
pub const DIM_MAX: u32 = 2048;
pub const DIM_STEP: u32 = 16;
pub const DEFAULT_SIDE: u32 = 1024;

fn snap(v: f32) -> u32 {
    (((v / DIM_STEP as f32).round().max(0.0) as u32) * DIM_STEP).clamp(DIM_MIN, DIM_MAX)
}

pub fn other_side(v: u32, (rw, rh): (u32, u32), from_width: bool) -> u32 {
    if from_width {
        snap(v as f32 * rh as f32 / rw as f32)
    } else {
        snap(v as f32 * rw as f32 / rh as f32)
    }
}

/// Диапазон стороны при активной пропорции — чтобы парная не вылетела за
/// [DIM_MIN, DIM_MAX].
pub fn side_range(aspect: Option<(u32, u32)>, is_width: bool) -> (u32, u32) {
    let Some((rw, rh)) = aspect else { return (DIM_MIN, DIM_MAX) };
    let k = if is_width { rw as f32 / rh as f32 } else { rh as f32 / rw as f32 };
    let lo = ((DIM_MIN as f32 * k) / DIM_STEP as f32).ceil() as u32 * DIM_STEP;
    let hi = ((DIM_MAX as f32 * k) / DIM_STEP as f32).floor() as u32 * DIM_STEP;
    (lo.clamp(DIM_MIN, DIM_MAX), hi.clamp(DIM_MIN, DIM_MAX))
}

pub struct EmptyLatentExec;

impl NodeExecutor for EmptyLatentExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::FluxEmptyLatent { width, height, .. } => {
                    let (w, h) = if track {
                        (width.get(), height.get())
                    } else {
                        (width.get_untracked(), height.get_untracked())
                    };
                    let l = FluxLatent {
                        width: synaptix_image_flux::model::snap_side(w as usize),
                        height: synaptix_image_flux::model::snap_side(h as usize),
                        tensor: None,
                    };
                    PortValue::Data(Arc::new(DataBlob::Flux(FluxBlob::Latent(Arc::new(l)))))
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("latent", pv);
    }
}

fn dim_slider(sig: RwSignal<u32>, other: RwSignal<u32>, aspect_idx: RwSignal<usize>, is_width: bool) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let aspect = aspect_of(aspect_idx.get());
        let (min, max) = side_range(aspect, is_width);
        vec![Box::new(
            Slider::new()
                .value(sig.get().clamp(min, max) as f32)
                .range(min as f32, max as f32)
                .step(DIM_STEP as f32)
                .show_value(0)
                .on_change(move |v| {
                    let v = snap(v);
                    sig.set(v);
                    if let Some(r) = aspect_of(aspect_idx.get_untracked()) {
                        other.set(other_side(v, r, is_width));
                    }
                })
                .class("node-input-slider node-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}

fn aspect_dropdown(aspect_idx: RwSignal<usize>, width: RwSignal<u32>, height: RwSignal<u32>) -> Box<dyn Widget> {
    let items: Vec<DropdownItem> = ASPECT_OPTIONS.iter().map(|s| DropdownItem::new(*s, option_label(s))).collect();
    let current = ASPECT_OPTIONS.get(aspect_idx.get_untracked()).copied().unwrap_or(ASPECT_OPTIONS[0]).to_string();
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let Some(i) = idx_in(ASPECT_OPTIONS, s) else { return };
                aspect_idx.set(i);
                if let Some(r) = aspect_of(i) {
                    let (lo, hi) = side_range(Some(r), true);
                    let w = width.get_untracked().clamp(lo, hi);
                    width.set(w);
                    height.set(other_side(w, r, true));
                }
            })
            .class("node-input-dropdown"),
    )
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::FluxEmptyLatent { width, height, aspect_idx } => Some((*width, *height, *aspect_idx)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, aspect_idx)) = snapshot else {
        return Box::new(Column::new());
    };
    let info = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let (w, h) = (width.get() as usize, height.get() as usize);
        let mp = (w * h) as f32 / 1_000_000.0;
        let tokens = (w / 16) * (h / 16);
        vec![Box::new(
            Text::new(tr!("node.flux_empty_latent.info", mp = format!("{mp:.2}"), tokens = tokens))
                .class("flux-node-info"),
        )]
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row(&tr!("node.minimax_h3_latent.aspect_ratio"), aspect_dropdown(aspect_idx, width, height)),
                field_row(&tr!("nodes.common.field_width"), dim_slider(width, height, aspect_idx, true)),
                field_row(&tr!("nodes.common.field_height"), dim_slider(height, width, aspect_idx, false)),
                Box::new(info),
            ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_keeps_both_sides_in_range() {
        for (i, name) in ASPECT_OPTIONS.iter().enumerate().skip(1) {
            let r = aspect_of(i).unwrap();
            let (lo, hi) = side_range(Some(r), true);
            assert!(lo <= hi, "{name}");
            for w in [lo, hi, (lo + hi) / 2] {
                let h = other_side(w, r, true);
                assert!((DIM_MIN..=DIM_MAX).contains(&h), "{name}: {w} → {h}");
                assert_eq!(h % DIM_STEP, 0);
            }
        }
    }

    #[test]
    fn square_1024_is_default_friendly() {
        assert_eq!(other_side(1024, (1, 1), true), 1024);
        assert_eq!(other_side(1344, (16, 9), true), 752);
    }
}
