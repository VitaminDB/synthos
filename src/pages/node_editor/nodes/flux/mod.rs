//! FLUX.1 (dev / schnell) — картинка по тексту и img2img, стадиями как у
//! LTX/H3: Checkpoint → Text Encoder → Empty Latent | VAE Encode → Sampler →
//! VAE Decode → Image Save. Веса — `.syn`-бандл или каталог diffusers
//! (`synaptix_image_flux::FluxSource`).

pub mod checkpoint;
pub mod latent;
pub mod sampler;
pub mod shared;
pub mod text_encoder;
pub mod vae;

use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_image_flux::OffloadMode;

use super::super::state::NodeEditorCtx;
use super::super::types::{FluxLatent, FluxModelHandle, ImageData, NodeId, PortValue};

pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];
/// Веса трансформера. Энкодеры всегда в BF16 (T5 в F16 переполняется),
/// VAE — в F32.
pub const QUANT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (bf16)"];
/// MXFP8: 12 ГБ и почти без потери качества; NVFP4 — 6 ГБ; плотный BF16 —
/// 23 ГБ. Что не влезло в VRAM, стримится с хоста (проверено в 7 ГБ на всех
/// трёх).
pub const DEFAULT_QUANT_IDX: usize = 1;
/// Где держать блоки DiT: `auto` — сколько влезло с запасом под активации,
/// остальное стримится с хоста; `resident` — всё на карте;
/// `block_offload` — всё с хоста. T5-XXL на малой карте стримит блоки сам.
pub const MEMORY_MODE_OPTIONS: &[&str] = &["auto", "resident", "block_offload"];
/// Длина T5: `auto` — по модели (dev 512, schnell 256).
pub const SEQ_LEN_OPTIONS: &[&str] = &["auto", "256", "512"];

pub fn device_of(idx: usize) -> Device {
    match idx {
        1 => Device::Cpu,
        _ => Device::Cuda(0),
    }
}

/// `(compute, quant)` трансформера. Квантованные ядра считают в F16; на CPU
/// кванта нет — всё в F32.
pub fn precision_of(quant_idx: usize, device: Device) -> (DType, DType) {
    if !device.is_cuda() {
        return (DType::F32, DType::F32);
    }
    match quant_idx {
        0 => (DType::F16, DType::NVFP4),
        1 => (DType::F16, DType::MXFP8),
        _ => (DType::BF16, DType::BF16),
    }
}

pub fn offload_of(idx: usize) -> OffloadMode {
    match idx {
        1 => OffloadMode::Resident,
        2 => OffloadMode::Stream,
        _ => OffloadMode::Auto,
    }
}

/// Длина T5 по индексу дропдауна; `auto` — рекомендованная моделью.
pub fn seq_len_of(idx: usize, model_default: usize) -> usize {
    match idx {
        1 => 256,
        2 => 512,
        _ => model_default,
    }
}

fn current_input(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<PortValue> {
    let conns = ctx.connections.get_untracked();
    let src = *conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    ctx.values.get_untracked().get(&(src.from_node, src.from_port)).cloned()
}

pub fn current_input_text(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<String> {
    match current_input(ctx, node_id, port)? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

pub fn current_input_model(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<FluxModelHandle>> {
    current_input(ctx, node_id, port)?.as_flux_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_flux::FluxConditioning>> {
    current_input(ctx, node_id, port)?.as_flux_conditioning()
}

pub fn current_input_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<FluxLatent>> {
    current_input(ctx, node_id, port)?.as_flux_latent()
}

pub fn current_input_image(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<ImageData>> {
    current_input(ctx, node_id, port)?.as_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_follows_quant_and_device() {
        assert_eq!(precision_of(0, Device::Cuda(0)), (DType::F16, DType::NVFP4));
        assert_eq!(precision_of(1, Device::Cuda(0)), (DType::F16, DType::MXFP8));
        assert_eq!(precision_of(2, Device::Cuda(0)), (DType::BF16, DType::BF16));
        assert_eq!(precision_of(0, Device::Cpu), (DType::F32, DType::F32));
    }

    #[test]
    fn seq_len_auto_uses_model_default() {
        assert_eq!(seq_len_of(0, 512), 512);
        assert_eq!(seq_len_of(0, 256), 256);
        assert_eq!(seq_len_of(1, 512), 256);
        assert_eq!(seq_len_of(2, 256), 512);
    }
}
