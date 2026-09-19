//! FLUX.2 (dev / klein 4B / klein 9B) — картинка по тексту и правка по
//! референсам, стадиями как у FLUX.1: Checkpoint → Text Encoder → (Empty
//! Latent из FLUX) → Sampler (+ Reference) → VAE Decode → Image Save. Веса —
//! `.syn`-бандл или каталог diffusers (`synaptix_image_flux2::Flux2Source`).
//!
//! Отличия от FLUX.1, из-за которых это отдельные ноды: текст кодирует LLM
//! (Mistral-24B у dev, Qwen3 у klein), латент 128-канальный, у klein 4 шага
//! без guidance, а картинки-референсы для правки идут прямо в сэмплер.

pub mod checkpoint;
pub mod reference;
pub mod sampler;
pub mod shared;
pub mod text_encoder;
pub mod vae;

use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_image_flux2::transformer::Placement;

use super::super::state::NodeEditorCtx;
use super::super::types::{FluxLatent, FluxModelHandle, ImageData, NodeId, PortValue};

pub use super::flux::{DEVICE_OPTIONS, MEMORY_MODE_OPTIONS};
/// Веса DiT. Энкодер всегда в BF16, VAE — в F32.
pub const QUANT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (bf16)"];
/// MXFP8 почти без потери качества; что не влезло в VRAM, стримится с хоста,
/// поэтому работает и на малой карте (проверено в 7 ГБ для dev и klein).
pub const DEFAULT_QUANT_IDX: usize = 1;
/// Сколько референсов принимает сэмплер (у пайплайна BFL — до 10).
pub const MAX_REFERENCES: usize = 10;

pub fn device_of(idx: usize) -> Device {
    super::flux::device_of(idx)
}

/// `(compute, quant)` DiT. Активации — BF16 (fused-преквант FLUX.2 на BF16
/// не нужен: NVFP4-GEMM умеет BF16 нативно, MXFP8 — через F16); на CPU
/// кванта нет.
pub fn precision_of(quant_idx: usize, device: Device) -> (DType, DType) {
    if !device.is_cuda() {
        return (DType::F32, DType::F32);
    }
    match quant_idx {
        0 => (DType::BF16, DType::NVFP4),
        1 => (DType::BF16, DType::MXFP8),
        _ => (DType::BF16, DType::BF16),
    }
}

pub fn placement_of(idx: usize) -> Placement {
    match idx {
        1 => Placement::Resident,
        2 => Placement::Stream,
        _ => Placement::Auto,
    }
}

fn current_input(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<PortValue> {
    let conns = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?.clone();
    ctx.values.get_untracked().get(&(src.from_node, src.from_port)).cloned()
}

pub fn current_input_text(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<String> {
    match current_input(ctx, node_id, port)? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

pub fn current_input_model(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxModelHandle>> {
    current_input(ctx, node_id, port)?.as_flux2_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_flux2::Flux2Conditioning>> {
    current_input(ctx, node_id, port)?.as_flux2_conditioning()
}

pub fn current_input_references(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_flux2::Flux2References>> {
    current_input(ctx, node_id, port)?.as_flux2_references()
}

pub fn current_input_latent(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxLatent>> {
    current_input(ctx, node_id, port)?.as_flux_latent()
}

pub fn current_input_image(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<ImageData>> {
    current_input(ctx, node_id, port)?.as_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_is_bf16_with_selected_weights() {
        assert_eq!(precision_of(0, Device::Cuda(0)), (DType::BF16, DType::NVFP4));
        assert_eq!(precision_of(1, Device::Cuda(0)), (DType::BF16, DType::MXFP8));
        assert_eq!(precision_of(2, Device::Cuda(0)), (DType::BF16, DType::BF16));
        assert_eq!(precision_of(1, Device::Cpu), (DType::F32, DType::F32));
        assert_eq!(placement_of(2), Placement::Stream);
    }
}
