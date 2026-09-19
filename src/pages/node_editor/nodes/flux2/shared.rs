//! Общие для нод FLUX.2 объекты: открытая модель (конфиги + токенайзер) и
//! DiT. Кэш держит только `Weak`: объект живёт, пока его держит воркер или
//! hold-слот «Держать в памяти». Энкодер текста и VAE живут только внутри
//! своей стадии (`Flux2Model` сам их грузит и отпускает).

use std::collections::HashMap;
use std::result::Result;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use synaptix_image_flux2::{Flux2Conditioning, Flux2Model, Flux2Transformer};
use syngui::prelude::*;

use super::super::super::types::FluxModelHandle;
use super::{device_of, placement_of, precision_of};

pub struct ModelShared {
    pub model: Flux2Model,
}

pub struct TransformerShared {
    pub transformer: Flux2Transformer,
    /// Под сколько токенов выбиралось размещение блоков.
    pub planned_tokens: usize,
}

type Cache<T> = OnceLock<Mutex<HashMap<String, Weak<T>>>>;

static MODEL: Cache<ModelShared> = OnceLock::new();
static TRANSFORMER: Cache<TransformerShared> = OnceLock::new();
static HOLD: OnceLock<Mutex<Option<Arc<TransformerShared>>>> = OnceLock::new();
/// Последнее кондиционирование: повтор с тем же промптом (другой seed,
/// референс) не гоняет LLM-энкодер заново — у dev это Mistral-24B.
static LAST_COND: OnceLock<Mutex<Option<(String, Arc<Flux2Conditioning>)>>> = OnceLock::new();

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
    super::super::flux::shared::ensure_kernels_registered();
    let map = MODEL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.flux.shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    let key = cache_key(handle);
    if let Some(m) = g.get(&key).and_then(|w| w.upgrade()) {
        return Ok(m);
    }
    let device = device_of(handle.device_idx);
    let (compute, quant) = precision_of(handle.quant_idx, device);
    let model = Flux2Model::open(&handle.model_path, device, compute, quant)
        .map_err(|e| e.to_string())?
        .with_memory(placement_of(handle.memory_mode_idx));
    let arc = Arc::new(ModelShared { model });
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

/// DiT под прогон на `tokens` токенов. Уже загруженный (или из hold-слота)
/// переиспользуется, если его размещение считалось под не меньший прогон;
/// иначе грузится заново — на большем разрешении не хватило бы запаса под
/// активации.
pub fn load_transformer(
    handle: &FluxModelHandle,
    model: &ModelShared,
    tokens: usize,
) -> Result<Arc<TransformerShared>, String> {
    super::super::flux::shared::ensure_kernels_registered();
    let key = cache_key(handle);
    let map = TRANSFORMER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.flux.shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    let device = device_of(handle.device_idx);
    if let Some(t) = g.get(&key).and_then(|w| w.upgrade()) {
        if tokens <= t.planned_tokens {
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
        format!("flux2/transformer/{key}"),
        "FLUX.2",
        "Transformer",
        label(handle),
        device,
        bytes,
        Arc::downgrade(&arc),
        release_hold,
    );
    Ok(arc)
}

/// «Держать в памяти»: DiT переживает прогон до смены чекпойнта или
/// выгрузки из панели моделей.
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

pub fn cond_key(handle: &FluxModelHandle, prompt: &str) -> String {
    format!("{}|{prompt}", handle.model_path.display())
}

pub fn cached_conditioning(key: &str) -> Option<Arc<Flux2Conditioning>> {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    let g = c.lock().ok()?;
    g.as_ref().filter(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn remember_conditioning(key: String, cond: Arc<Flux2Conditioning>) {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = c.lock() {
        *g = Some((key, cond));
    }
}

/// Вернуть драйверу память всех пулов (энкодер, VAE, старый DiT).
pub fn trim_pool(device: Device) {
    synaptix_image_flux2::model::release_pools(device);
}

/// Ошибка — нехватка VRAM (CUDA OOM после тримов пула).
pub fn is_oom(msg: &str) -> bool {
    msg.contains("OOM") || msg.contains("OUT_OF_MEMORY")
}

/// Стадия, которой на малой карте может не хватить места рядом с DiT из
/// hold-слота «Держать в памяти» (VAE на 1024² в F32 — несколько ГБ): при OOM
/// DiT отпускается, пулы отдаются драйверу, и стадия повторяется один раз.
pub fn with_vram_retry<T>(device: Device, f: impl Fn() -> Result<T, String>) -> Result<T, String> {
    match f() {
        Err(e) if is_oom(&e) => {
            tracing::info!(target: super::super::WORKER_LOG, "OOM рядом с удержанным DiT — отпускаю его и повторяю стадию");
            release_hold();
            trim_pool(device);
            f()
        }
        other => other,
    }
}
