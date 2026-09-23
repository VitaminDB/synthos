//! SDXL (Stable Diffusion XL base 1.0) — картинка по тексту и img2img,
//! стадиями как у FLUX: Checkpoint → Text Encoder (промпт + негатив) → FLUX
//! Empty Latent (размер) | SDXL VAE Encode → Sampler → VAE Decode → Image
//! Save. Веса — `.syn`-бандл или каталог diffusers
//! (`synaptix_image_sdxl::SdxlSource`).

pub mod checkpoint;
pub mod sampler;
pub mod shared;
pub mod text_encoder;
pub mod vae;

use std::sync::Arc;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;

use super::super::state::NodeEditorCtx;
use super::super::types::{FluxLatent, FluxModelHandle, ImageData, NodeId, PortValue};

pub use super::flux::DEVICE_OPTIONS;
/// Веса UNet: плотный F16 (5,1 ГБ, как у diffusers) или квант линеек
/// внимания и GEGLU. Энкодеры — F16, VAE — BF16.
pub const QUANT_OPTIONS: &[&str] = &["dense (f16)", "mxfp8", "nvfp4"];
pub const DEFAULT_QUANT_IDX: usize = 0;

pub fn device_of(idx: usize) -> Device {
    super::flux::device_of(idx)
}

/// Формат весов UNet (на CPU кванта нет).
pub fn quant_of(quant_idx: usize, device: Device) -> DType {
    if !device.is_cuda() {
        return DType::F32;
    }
    match quant_idx {
        1 => DType::MXFP8,
        2 => DType::NVFP4,
        _ => DType::F16,
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
    current_input(ctx, node_id, port)?.as_sdxl_model()
}

pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<synaptix_image_sdxl::SdxlConditioning>> {
    current_input(ctx, node_id, port)?.as_sdxl_conditioning()
}

/// Латент на входе сэмплера/декодера: SDXL (VAE Encode, Sampler) или
/// пустой FLUX Empty Latent — только размер.
pub fn current_input_latent(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<FluxLatent>> {
    let v = current_input(ctx, node_id, port)?;
    v.as_sdxl_latent().or_else(|| v.as_flux_latent().filter(|l| l.tensor.is_none()))
}

pub fn current_input_image(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<Arc<ImageData>> {
    current_input(ctx, node_id, port)?.as_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_by_index() {
        assert_eq!(quant_of(0, Device::Cuda(0)), DType::F16);
        assert_eq!(quant_of(1, Device::Cuda(0)), DType::MXFP8);
        assert_eq!(quant_of(2, Device::Cuda(0)), DType::NVFP4);
        assert_eq!(quant_of(2, Device::Cpu), DType::F32);
    }
}
