//! `LtxImage` — загрузка изображения для image→video conditioning.
//! `→ image_cond: Data(ImageCond)`. Грузит картинку (`synaptix_io::image`) в
//! `[3,H,W]` [0,1], отдаёт её + силу; resize до stage-сетки, VAE-encode и
//! формирование conditioning-токенов делает Sampler (он знает разрешение).
//!
//! Загрузка дёшева → выполняется прямо в evaluate (без on_run worker'а);
//! `cache` переиспользует тензор пока путь не сменился (ptr_eq стабилен).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_core::device::Device;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, LtxBlob, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_slider_row};
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct ImageExec;

impl NodeExecutor for ImageExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxImage {
                    image_path,
                    strength,
                    frame_idx,
                    error,
                    cache,
                } => {
                    let path = if track { image_path.get() } else { image_path.get_untracked() };
                    let s = if track { strength.get() } else { strength.get_untracked() };
                    let fi = if track { frame_idx.get() } else { frame_idx.get_untracked() } as usize;
                    match path {
                        Some(path) => match load_cached(&path, cache) {
                            Ok(img) => {
                                error.set(None);
                                PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::ImageCond {
                                    image: img,
                                    strength: s,
                                    frame_idx: fi,
                                })))
                            }
                            Err(e) => {
                                error.set(Some(e));
                                PortValue::Empty
                            }
                        },
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("image_cond", pv);
    }
}

fn load_cached(
    path: &PathBuf,
    cache: &Arc<syngui::core::sync::Mutex<Option<(PathBuf, synaptix_core::tensor::Tensor)>>>,
) -> std::result::Result<synaptix_core::tensor::Tensor, String> {
    if let Ok(g) = cache.lock() {
        if let Some((p, t)) = g.as_ref() {
            if p == path {
                return Ok(t.clone());
            }
        }
    }
    let img = synaptix_io::image::load_image(path, Device::Cpu)
        .map_err(|e| tr!("node.ltx_image.err.load_image", error = e))?;
    if let Ok(mut g) = cache.lock() {
        *g = Some((path.clone(), img.clone()));
    }
    Ok(img)
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxImage { image_path, strength, frame_idx, error, .. } => {
                Some((*image_path, *strength, *frame_idx, *error))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((image_path, strength, frame_idx, error)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxImage"))
                .class("node-card-field-error"),
        );
    };
    let status = syngui::widgets::Reactive::new(move || -> Vec<Box<dyn Widget>> {
        match error.get() {
            Some(e) => vec![Box::new(Text::new(tr!("nodes.common.error", error = e)).class("audio-node-error")) as Box<dyn Widget>],
            None => vec![],
        }
    });
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("node.ltx_image.field.image"),
            node_file_picker(
                tr!("node.ltx_image.picker.image"),
                image_path,
                &[("nodes.filter.images", &["png", "jpg", "jpeg", "webp", "bmp"])],
                |_| {},
            ),
        ),
        field_row(&tr!("node.ltx_image.field.strength"), make_slider_row(strength, 0.0, 1.0, 0.05, 2)),
        field_row(
            &tr!("node.ltx_image.field.frame_start"),
            super::super::acestep::make_int_slider_row(frame_idx, 0, 240, 1),
        ),
        Box::new(status),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
