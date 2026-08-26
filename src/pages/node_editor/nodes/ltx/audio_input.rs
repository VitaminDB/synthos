//! `LtxAudioInput` — источник аудио (речь) для lipdub. `→ audio:
//! Data(AudioInput(path))`. Загрузка (16k→mel→audio-VAE encode) делает
//! нода-потребитель.

use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{DataBlob, LtxBlob, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::field_row;
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct AudioInputExec;

impl NodeExecutor for AudioInputExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxAudioInput { audio_path } => {
                    let path = if track { audio_path.get() } else { audio_path.get_untracked() };
                    match path {
                        Some(p) => PortValue::Data(Arc::new(DataBlob::Ltx(LtxBlob::AudioInput(p)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("audio", pv);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let audio_path = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxAudioInput { audio_path } => Some(*audio_path),
            _ => None,
        },
        Err(_) => None,
    };
    let Some(audio_path) = audio_path else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxAudioInput"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![field_row(
        &tr!("node.ltx_audio_input.field.audio"),
        node_file_picker(
            tr!("node.ltx_audio_input.picker.audio"),
            audio_path,
            &[("nodes.filter.audio", &["wav", "mp3", "m4a", "flac", "ogg", "opus"])],
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
