//! Семейство нод **YuE2** — песня из стиля и лирики через редактируемую
//! партитуру. Категория Нейро → подкатегория «YuE2».
//!
//! - [`checkpoint`] — пути бандлов и точности → `model: Data(Model)`;
//! - [`generate`] — `(model, style, lyrics, опц. abc) → (audio, score,
//!   latent)`: партитура (ABC), семантические токены, акустические латенты и
//!   48 кГц стерео одним прогоном;
//! - [`vae_decode`] — `(model, latent) → audio`: передекодировать латенты
//!   другим декодером, не повторяя генерацию.
//!
//! Один бандл `yue2-3b.syn` держит обе ветки модели: AR (партитура и музыка) и
//! NAR (акустика). Они грузятся как два компонента и в панели «Модели в памяти»
//! видны отдельно — NAR нужен только на стадии акустики.

pub mod checkpoint;
pub mod generate;
pub mod shared;
pub mod vae_decode;

use std::sync::Arc;

use synaptix_core::{device::Device, dtype::DType};

use super::super::state::NodeEditorCtx;
use super::super::types::{NodeId, Yue2ModelHandle};

// UI-хелперы общие для всех модельных нод (живут в `acestep`, как и у LTX,
// FLUX и H3).
pub use super::acestep::{
    dir_picker_row, field_row, idx_in, make_dropdown, make_int_slider_row, make_seed_slider,
    make_slider_row, make_toggle, status_row, COMPUTE_OPTIONS, DEVICE_OPTIONS, QUANT_OPTIONS,
};

/// Тип вычислений декодера. Релиз проверен на F32; BF16 вдвое экономнее по
/// памяти и заметно быстрее, но это уже не эталонный тракт.
pub const VAE_DTYPE_OPTIONS: &[&str] = &["f32", "bf16"];

/// Партитура: что модель пишет перед музыкой.
pub const COT_OPTIONS: &[&str] = &["full", "melody", "off"];

pub fn device_from_idx(i: usize) -> Device {
    super::acestep::device_from_idx(i)
}

pub fn compute_from_idx(i: usize) -> DType {
    super::acestep::compute_from_idx(i)
}

pub fn quant_from_idx(i: usize) -> Option<DType> {
    super::acestep::quant_from_idx(i)
}

pub fn vae_dtype_from_idx(i: usize) -> DType {
    match VAE_DTYPE_OPTIONS.get(i).copied() {
        Some("bf16") => DType::BF16,
        _ => DType::F32,
    }
}

pub fn cot_from_idx(i: usize) -> synaptix_music_yue2::protocol::Cot {
    use synaptix_music_yue2::protocol::Cot;
    match COT_OPTIONS.get(i).copied() {
        Some("melody") => Cot::Melody,
        Some("off") => Cot::Off,
        _ => Cot::Full,
    }
}

/// Каталог моделей приложения (настройка «Каталог моделей»); вне UI-контекста
/// (тесты) — `~/Storage/syn_models`.
pub fn app_models_dir() -> std::path::PathBuf {
    super::acestep::app_models_dir()
}

/// Прочитать хэндл чекпойнта YuE2 из подключённого upstream'а.
pub fn current_input_model(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<Yue2ModelHandle>> {
    let conns = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    values.get(&(src.from_node, src.from_port)).cloned()?.as_yue2_model()
}

/// Прочитать акустические латенты YuE2 (`[кадры, 64]`).
pub fn current_input_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    let conns = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    values.get(&(src.from_node, src.from_port)).cloned()?.as_yue2_latent()
}

/// Текст из подключённого upstream'а (стиль, лирика, партитура).
pub fn current_input_text(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<String> {
    super::acestep::current_input_text(ctx, node_id, port)
}
