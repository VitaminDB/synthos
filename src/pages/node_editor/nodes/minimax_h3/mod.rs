pub mod checkpoint;
pub mod decode;
pub mod latent;
pub mod sampler;
pub mod save;
pub mod shared;
pub mod text_encoder;

use std::path::PathBuf;
use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use syngui::prelude::*;

use super::super::state::NodeEditorCtx;
use super::super::types::{
    H3Conditioning, H3Geometry, H3Keyframe, H3ModelHandle, H3VideoLatent, NodeId, PortValue,
};

pub const VARIANT_OPTIONS: &[&str] = &["FL2VA (t2va + first/last)", "Ref2VA (референсы)"];
pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];
pub const QUANT_DIT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (compute)"];
pub const QUANT_ENC_OPTIONS: &[&str] = &["mxfp8", "nvfp4", "dense (compute)"];
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f16", "f32"];
pub const MEMORY_MODE_OPTIONS: &[&str] =
    &["auto", "предвычисленный adaLN", "блочный оффлоад"];
pub const FRAME_SLOT_OPTIONS: &[&str] = &["первый кадр", "последний кадр"];
pub const RESIZE_OPTIONS: &[&str] = &["растянуть", "кроп по центру"];

pub fn device_of(idx: usize) -> Device {
    match idx {
        1 => Device::Cpu,
        _ => Device::Cuda(0),
    }
}

pub fn compute_of(idx: usize) -> DType {
    match idx {
        1 => DType::F16,
        2 => DType::F32,
        _ => DType::BF16,
    }
}

pub fn quant_dit_of(idx: usize, compute: DType) -> DType {
    match idx {
        0 => DType::NVFP4,
        1 => DType::MXFP8,
        _ => compute,
    }
}

pub fn quant_enc_of(idx: usize, compute: DType) -> DType {
    match idx {
        0 => DType::MXFP8,
        1 => DType::NVFP4,
        _ => compute,
    }
}

pub fn memory_mode_of(idx: usize) -> synaptix_video_minimax_h3::H3MemoryMode {
    use synaptix_video_minimax_h3::H3MemoryMode as M;
    match idx {
        1 => M::AdalnPrecomputed,
        2 => M::BlockOffload,
        _ => M::Auto,
    }
}

pub fn variant_of(idx: usize) -> synaptix_video_minimax_h3::config::H3Variant {
    use synaptix_video_minimax_h3::config::H3Variant as V;
    match idx {
        1 => V::Ref2va,
        _ => V::Fl2va,
    }
}

fn current_input(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<PortValue> {
    let conns = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?
        .clone();
    let values = ctx.values.get_untracked();
    values.get(&(src.from_node, src.from_port)).cloned()
}

pub fn current_input_text(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<String> {
    match current_input(ctx, node_id, port)? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

pub fn current_input_model(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<H3ModelHandle>> {
    current_input(ctx, node_id, port)?.as_h3_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<H3Conditioning>> {
    current_input(ctx, node_id, port)?.as_h3_conditioning()
}

pub fn current_input_av_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<H3Geometry> {
    current_input(ctx, node_id, port)?.as_h3_av_latent()
}

pub fn current_input_video_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<H3VideoLatent>> {
    current_input(ctx, node_id, port)?.as_h3_video_latent()
}

pub fn current_input_audio_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    current_input(ctx, node_id, port)?.as_h3_audio_latent()
}

pub fn current_input_keyframe(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<H3Keyframe>> {
    current_input(ctx, node_id, port)?.as_h3_keyframe()
}

pub fn current_input_frames(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<super::super::types::LtxFrames>> {
    current_input(ctx, node_id, port)?.as_h3_frames()
}

pub fn current_input_audio(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<syngui::audio::AudioBuffer>> {
    current_input(ctx, node_id, port)?.as_audio()
}

pub fn dir_picker_row(
    tooltip: &'static str,
    sig: RwSignal<Option<PathBuf>>,
) -> Box<dyn Widget> {
    super::ltx::dir_picker_row(tooltip, sig)
}

pub fn progress_row(running: RwSignal<bool>, progress_pct: RwSignal<f32>) -> Box<dyn Widget> {
    super::ltx::progress_row(running, progress_pct)
}

pub fn cancel_button(
    running: RwSignal<bool>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Box<dyn Widget> {
    super::ltx::cancel_button(running, cancel)
}
