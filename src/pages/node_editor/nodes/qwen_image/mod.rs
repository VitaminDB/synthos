//! Qwen-Image-Edit (08.2025) и Qwen-Image-Edit-2509/2511 — правка картинки
//! по инструкции, стадиями как у FLUX.2: Checkpoint → Reference (картинки,
//! цепочкой) → Text Encoder (промпт + те же картинки) → Sampler → VAE Decode
//! → Image Save. Веса — `.syn`-бандл или каталог diffusers
//! (`synaptix_image_qwen::QwenImageSource`).
//!
//! Отличия от FLUX.2, из-за которых это отдельные ноды: картинки для правки
//! нужны и энкодеру (Qwen2.5-VL видит их вместе с промптом), и сэмплеру
//! (латент VAE); у сэмплера true CFG с негативом (по умолчанию `" "`).

pub mod checkpoint;
pub mod reference;
pub mod sampler;
pub mod shared;
pub mod text_encoder;
pub mod vae;

use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_image_qwen::transformer::Placement;

use super::super::state::NodeEditorCtx;
use super::super::types::{FluxLatent, FluxModelHandle, ImageData, NodeId, PortValue, QwenImageRefs};

pub use super::flux::{DEVICE_OPTIONS, MEMORY_MODE_OPTIONS};
/// Веса DiT (20 млрд параметров). Энкодер всегда в BF16, VAE — в F32.
pub const QUANT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (bf16)"];
/// MXFP8: блоки DiT без модуляций ~14 ГБ — на карте 24 ГБ целиком (пик 17,8 ГБ
/// на 1024² с картинкой), картинка чище, чем у NVFP4 (~7,7 ГБ, на 20 %
/// быстрее). Что не влезло в VRAM, стримится с хоста.
pub const DEFAULT_QUANT_IDX: usize = 1;
/// Сколько картинок принимает Edit-2509/2511 (у Qwen-Image-Edit — одна).
pub const MAX_IMAGES: usize = 4;

pub fn device_of(idx: usize) -> Device {
    super::flux::device_of(idx)
}

/// `(compute, quant)` DiT: активации BF16 (в F16 Qwen-Image переполняется),
/// на CPU кванта нет.
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
    let src = *conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    ctx.values.get_untracked().get(&(src.from_node, src.from_port)).cloned()
}

pub fn current_input_text(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<String> {
    match current_input(ctx, node_id, port)? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

pub fn current_input_model(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxModelHandle>> {
    current_input(ctx, node_id, port)?.as_qwen_image_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_qwen::QwenConditioning>> {
    current_input(ctx, node_id, port)?.as_qwen_image_conditioning()
}

pub fn current_input_references(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<QwenImageRefs>> {
    current_input(ctx, node_id, port)?.as_qwen_image_references()
}

/// Размер с FLUX Empty Latent (пустой латент — только размер).
pub fn current_input_size(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<(usize, usize)> {
    current_input(ctx, node_id, port)?.as_flux_latent().map(|l| (l.width, l.height))
}

pub fn current_input_latent(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxLatent>> {
    current_input(ctx, node_id, port)?.as_qwen_image_latent()
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
        assert_eq!(precision_of(0, Device::Cpu), (DType::F32, DType::F32));
        assert_eq!(placement_of(2), Placement::Stream);
    }
}
