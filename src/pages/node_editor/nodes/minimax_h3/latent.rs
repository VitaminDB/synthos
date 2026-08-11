use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_minimax_h3 as h3;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{
    DataBlob, H3Blob, H3Geometry, H3Keyframe, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_int_slider_row, make_slider_row};
use crate::pages::node_editor::controls::keyframe_slot::KeyframeSlot;

pub fn geometry_of(width: u32, height: u32, seconds: f32) -> H3Geometry {
    let g = h3::pipeline::Geometry::from_duration(width as usize, height as usize, seconds as f64);
    H3Geometry {
        width: g.width,
        height: g.height,
        frame_count: g.frame_count,
        latent_t: g.latent_t,
        latent_h: g.latent_h,
        latent_w: g.latent_w,
        audio_t: g.audio_t,
    }
}

pub struct EmptyLatentExec;

impl NodeExecutor for EmptyLatentExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds } => {
                    let (w, h, d) = if track {
                        (width.get(), height.get(), duration_seconds.get())
                    } else {
                        (
                            width.get_untracked(),
                            height.get_untracked(),
                            duration_seconds.get_untracked(),
                        )
                    };
                    PortValue::Data(Arc::new(DataBlob::H3(H3Blob::AvLatent(geometry_of(w, h, d)))))
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("av_latent", pv);
    }
}

pub fn empty_latent_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds } => {
                Some((*width, *height, *duration_seconds))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration)) = snapshot else {
        return Box::new(Column::new());
    };
    let info = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let g = geometry_of(width.get(), height.get(), duration.get());
        vec![Box::new(
            Text::new(format!(
                "{} кадров @24fps, латент {}×{}×{}, аудио {}",
                g.frame_count, g.latent_t, g.latent_h, g.latent_w, g.audio_t
            ))
            .class("h3-node-info"),
        )]
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row("Ширина", make_int_slider_row(width, 256, 1920, 32)),
                field_row("Высота", make_int_slider_row(height, 256, 1088, 32)),
                field_row("Длительность, с", make_slider_row(duration, 1.0, 15.0, 0.5, 1)),
                Box::new(info),
            ]),
    )
}

pub struct KeyframeExec;

impl NodeExecutor for KeyframeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3Keyframe { image, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match image.lock().ok().and_then(|g| g.clone()) {
                        Some(kf) => PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Keyframe(kf)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("keyframe", pv);
    }
}

pub fn keyframe_on_run(node: &NodeInstance, _ctx: &super::super::super::state::NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Keyframe {
                path,
                frame_slot_idx,
                image,
                error,
                output_version,
                ..
            } => Some((*path, *frame_slot_idx, image.clone(), *error, *output_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, slot, image, error, output_version)) = snapshot else {
        return;
    };
    let Some(p) = path.get_untracked() else {
        error.set(Some("выберите изображение".into()));
        return;
    };
    match synaptix_io::image::png::load_image(&p, synaptix_core::device::Device::Cpu) {
        Ok(img) => {
            let frame_index = if slot.get_untracked() == 1 { usize::MAX } else { 0 };
            if let Ok(mut g) = image.lock() {
                *g = Some(Arc::new(H3Keyframe { image: img, frame_index }));
            }
            error.set(None);
            output_version.update(|v| *v = v.wrapping_add(1));
        }
        Err(e) => error.set(Some(format!("не удалось открыть {}: {e}", p.display()))),
    }
}

pub fn keyframe_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Keyframe { path, frame_slot_idx, resize_idx, error, .. } => {
                Some((*path, *frame_slot_idx, *resize_idx, *error))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, frame_slot_idx, resize_idx, error)) = snapshot else {
        return Box::new(Column::new());
    };
    KeyframeSlot::new(path, frame_slot_idx, resize_idx)
        .error(error)
        .build()
}
