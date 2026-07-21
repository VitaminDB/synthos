//! `LtxVideoInput` — источник исходного видео для v2v-режимов (retake и т.п.).
//! `→ video: Data(VideoInput(path))`. Декодирование (ffmpeg→кадры→VAE encode)
//! на нужную сетку делает нода-потребитель (она знает разрешение/кадры).

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, LtxBlob, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::field_row;
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct VideoInputExec;

impl NodeExecutor for VideoInputExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxVideoInput { video_path } => {
                    let path = if track { video_path.get() } else { video_path.get_untracked() };
                    match path {
                        Some(p) => {
                            PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::VideoInput(p))))
                        }
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("video", pv);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let video_path = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxVideoInput { video_path } => Some(*video_path),
            _ => None,
        },
        Err(_) => None,
    };
    let Some(video_path) = video_path else {
        return Box::new(Text::new("LtxVideoInput: некорректный runtime").class("node-card-field-error"));
    };
    let rows: Vec<Box<dyn Widget>> = vec![field_row(
        "Видео",
        node_file_picker(
            "Исходное видео (mp4/mkv/webm/mov)",
            video_path,
            &[("Видео", &["mp4", "mkv", "webm", "mov", "m4v", "avi"])],
            |_| {},
        ),
    )];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
