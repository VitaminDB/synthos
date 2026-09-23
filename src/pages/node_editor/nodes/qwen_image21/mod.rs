//! Qwen-Image 2.1 — одна модель на картинку по тексту, правку по референсам
//! (до 10 картинок) и прозрачные RGBA. Стадиями как у Qwen-Image: Checkpoint
//! → [Reference (цепочкой)] → Text Encoder (промпт + те же картинки) →
//! Sampler → VAE Decode → Image Save. Веса — `.syn`-бандл или каталог
//! diffusers (`synaptix_image_qwen21::QwenImage21Model`).
//!
//! Отличия от Qwen-Image-Edit, из-за которых это отдельные ноды: референсы
//! необязательны (t2i той же моделью), выход RGBA, CFG по умолчанию выключен
//! (модель обучена идти без него), KV-кэш префикса, «разрешение» — общая
//! настройка чекпойнта (`output_resolution` пайплайна): под неё приводятся
//! референсы и от неё считается размер выхода без Empty Latent.

pub mod checkpoint;
pub mod reference;
pub mod sampler;
pub mod shared;
pub mod text_encoder;
pub mod vae;

use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_image_qwen21::MemoryMode;

use super::super::state::NodeEditorCtx;
use super::super::types::{FluxLatent, ImageData, NodeId, PortValue, QwenImage21ModelHandle, QwenImage21Refs};

pub use super::flux::{DEVICE_OPTIONS, MEMORY_MODE_OPTIONS};
/// Веса DiT (7 млрд параметров). Энкодер Qwen3-VL — BF16, VAE — F32.
pub const QUANT_OPTIONS: &[&str] = &["nvfp4", "mxfp8", "dense (bf16)"];
/// MXFP8: DiT ~7,4 ГБ, косинус к F32 0,99998 — на карте 24 ГБ остаётся место
/// под KV-кэш и 2048²; dense BF16 (14 ГБ) — эталонная точность.
pub const DEFAULT_QUANT_IDX: usize = 1;
/// `output_resolution`: сторона квадрата той же площади, что у референсов и
/// выхода без Empty Latent (стороны кратны 32).
pub const RESOLUTION_OPTIONS: &[&str] = &["512", "768", "1024", "1536", "2048"];
pub const DEFAULT_RESOLUTION_IDX: usize = 2;
/// Сколько референсов принимает модель.
pub const MAX_IMAGES: usize = synaptix_image_qwen21::model::MAX_IMAGES;

pub fn device_of(idx: usize) -> Device {
    super::flux::device_of(idx)
}

/// `(compute, quant)` DiT: активации BF16, на CPU кванта нет.
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

pub fn placement_of(idx: usize) -> MemoryMode {
    match idx {
        1 => MemoryMode::Resident,
        2 => MemoryMode::Stream,
        _ => MemoryMode::Auto,
    }
}

pub fn resolution_of(idx: usize) -> usize {
    RESOLUTION_OPTIONS.get(idx).and_then(|s| s.parse().ok()).unwrap_or(1024)
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

pub fn current_input_model(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<QwenImage21ModelHandle>> {
    current_input(ctx, node_id, port)?.as_qwen_image21_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_qwen21::Qwen21Conditioning>> {
    current_input(ctx, node_id, port)?.as_qwen_image21_conditioning()
}

pub fn current_input_references(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<QwenImage21Refs>> {
    current_input(ctx, node_id, port)?.as_qwen_image21_references()
}

/// Размер с FLUX Empty Latent (пустой латент — только размер).
pub fn current_input_size(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<(usize, usize)> {
    current_input(ctx, node_id, port)?.as_flux_latent().map(|l| (l.width, l.height))
}

pub fn current_input_latent(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxLatent>> {
    current_input(ctx, node_id, port)?.as_qwen_image21_latent()
}

pub fn current_input_image(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<ImageData>> {
    current_input(ctx, node_id, port)?.as_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_and_resolution_options() {
        assert_eq!(precision_of(0, Device::Cuda(0)), (DType::BF16, DType::NVFP4));
        assert_eq!(precision_of(1, Device::Cuda(0)), (DType::BF16, DType::MXFP8));
        assert_eq!(precision_of(2, Device::Cuda(0)), (DType::BF16, DType::BF16));
        assert_eq!(precision_of(0, Device::Cpu), (DType::F32, DType::F32));
        assert_eq!(placement_of(2), MemoryMode::Stream);
        assert_eq!(resolution_of(DEFAULT_RESOLUTION_IDX), 1024);
        assert_eq!(resolution_of(4), 2048);
        assert_eq!(resolution_of(99), 1024);
    }
}
