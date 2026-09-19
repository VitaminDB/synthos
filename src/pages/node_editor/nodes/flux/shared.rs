//! Общие для нод FLUX объекты: открытая модель (конфиги + токенайзеры) и
//! трансформер. Кэш держит только `Weak`: объект живёт, пока его держит
//! воркер или hold-слот «Держать в памяти». Текстовые энкодеры и VAE живут
//! только внутри своей стадии (`FluxModel` сам их грузит и отпускает).

use std::result::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use synaptix_image_flux::{FluxConditioning, FluxModel, FluxTransformer};
use syngui::prelude::*;

use super::super::super::types::FluxModelHandle;
use super::{device_of, offload_of, precision_of};

static KERNELS: OnceLock<()> = OnceLock::new();

pub fn ensure_kernels_registered() {
    KERNELS.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
}

pub struct ModelShared {
    pub model: FluxModel,
}

pub struct TransformerShared {
    pub transformer: FluxTransformer,
    /// Под сколько токенов выбиралась резидентность плотных весов.
    pub planned_tokens: usize,
}

type Cache<T> = OnceLock<Mutex<HashMap<String, Weak<T>>>>;

static MODEL: Cache<ModelShared> = OnceLock::new();
static TRANSFORMER: Cache<TransformerShared> = OnceLock::new();
static HOLD: OnceLock<Mutex<Option<Arc<TransformerShared>>>> = OnceLock::new();
/// Последнее кондиционирование: повтор с тем же промптом (новый seed,
/// другие шаги) не перечитывает T5-XXL.
static LAST_COND: OnceLock<Mutex<Option<(String, Arc<FluxConditioning>)>>> = OnceLock::new();

pub fn cache_key(h: &FluxModelHandle) -> String {
    format!("{}|{}|{}|{}", h.model_path.display(), h.device_idx, h.quant_idx, h.memory_mode_idx)
}

fn label(h: &FluxModelHandle) -> String {
    h.model_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| h.model_path.display().to_string())
}

pub fn load_model(handle: &FluxModelHandle) -> Result<Arc<ModelShared>, String> {
    ensure_kernels_registered();
    let map = MODEL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.flux.shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    let key = cache_key(handle);
    if let Some(m) = g.get(&key).and_then(|w| w.upgrade()) {
        return Ok(m);
    }
    let device = device_of(handle.device_idx);
    let (compute, quant) = precision_of(handle.quant_idx, device);
    let model = FluxModel::open(&handle.model_path, device, compute, quant)
        .map_err(|e| e.to_string())?
        .with_offload(offload_of(handle.memory_mode_idx));
    let arc = Arc::new(ModelShared { model });
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

/// Трансформер под прогон на `tokens` токенов. Резидентный из hold-слота
/// переиспользуется; плотный, спланированный под меньший прогон, грузится
/// заново — иначе на большем разрешении не хватило бы запаса под активации.
pub fn load_transformer(
    handle: &FluxModelHandle,
    model: &ModelShared,
    tokens: usize,
) -> Result<Arc<TransformerShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    let map = TRANSFORMER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.flux.shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    let device = device_of(handle.device_idx);
    let quantized = precision_of(handle.quant_idx, device).1.is_quantized();
    if let Some(t) = g.get(&key).and_then(|w| w.upgrade()) {
        if quantized || tokens <= t.planned_tokens {
            return Ok(t);
        }
        drop(t);
        release_hold();
        g.remove(&key);
    }
    // Чужой держатель (другой чекпойнт) занимает VRAM — отпускаем.
    release_hold();
    trim_pool(device);
    let (value, bytes) = crate::models::measure(|| {
        let transformer = model.model.load_transformer(tokens).map_err(|e| e.to_string())?;
        Ok(TransformerShared { transformer, planned_tokens: tokens })
    })?;
    let arc = Arc::new(value);
    g.insert(key.clone(), Arc::downgrade(&arc));
    crate::models::register_weak(
        format!("flux/transformer/{key}"),
        "FLUX",
        "Transformer",
        label(handle),
        device,
        bytes,
        Arc::downgrade(&arc),
        release_hold,
    );
    Ok(arc)
}

/// «Держать в памяти»: трансформер переживает прогон до смены чекпойнта
/// или выгрузки из панели моделей.
pub fn hold(t: Arc<TransformerShared>) {
    let h = HOLD.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = h.lock() {
        *g = Some(t);
    }
}

pub fn release_hold() {
    let h = HOLD.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = h.lock() {
        *g = None;
    }
}

pub fn cond_key(handle: &FluxModelHandle, prompt: &str, seq_len: usize) -> String {
    format!("{}|{seq_len}|{prompt}", cache_key(handle))
}

pub fn cached_conditioning(key: &str) -> Option<Arc<FluxConditioning>> {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    let g = c.lock().ok()?;
    g.as_ref().filter(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn remember_conditioning(key: String, cond: Arc<FluxConditioning>) {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = c.lock() {
        *g = Some((key, cond));
    }
}

/// Вернуть драйверу память, отпущенную во все пулы (энкодеры, VAE, старый
/// DiT): пул активаций сам ничего не отдаёт, и после T5 в нём оставалось
/// ~10 ГБ — их не видели ни трансформер, ни видео-модели дальше по графу.
pub fn trim_pool(device: Device) {
    synaptix_image_flux::model::release_pools(device);
}
