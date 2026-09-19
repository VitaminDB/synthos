//! Общие для нод SDXL объекты: открытый чекпойнт (источник + токенайзеры)
//! и UNet. Кэш держит только `Weak`: объект живёт, пока его держит воркер или
//! hold-слот «Держать в памяти». CLIP и VAE живут только внутри своей стадии.

use std::collections::HashMap;
use std::result::Result;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use synaptix_image_sdxl::{SdxlCheckpoint, SdxlConditioning, SdxlUnet};
use syngui::prelude::*;

use super::super::super::types::FluxModelHandle;
use super::{device_of, quant_of};

pub struct ModelShared {
    pub model: SdxlCheckpoint,
}

pub struct UnetShared {
    pub unet: SdxlUnet,
}

type Cache<T> = OnceLock<Mutex<HashMap<String, Weak<T>>>>;

static MODEL: Cache<ModelShared> = OnceLock::new();
static UNET: Cache<UnetShared> = OnceLock::new();
static HOLD: OnceLock<Mutex<Option<Arc<UnetShared>>>> = OnceLock::new();
/// Последнее кондиционирование: повтор с тем же промптом не гоняет CLIP.
static LAST_COND: OnceLock<Mutex<Option<(String, Arc<SdxlConditioning>)>>> = OnceLock::new();

pub fn cache_key(h: &FluxModelHandle) -> String {
    format!("{}|{}|{}", h.model_path.display(), h.device_idx, h.quant_idx)
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
    let model = SdxlCheckpoint::open(&handle.model_path, device_of(handle.device_idx)).map_err(|e| e.to_string())?;
    let arc = Arc::new(ModelShared { model });
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

/// UNet: уже загруженный (или из hold-слота) переиспользуется.
pub fn load_unet(handle: &FluxModelHandle, model: &ModelShared) -> Result<Arc<UnetShared>, String> {
    super::super::flux::shared::ensure_kernels_registered();
    let key = cache_key(handle);
    let map = UNET.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.flux.shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    if let Some(u) = g.get(&key).and_then(|w| w.upgrade()) {
        return Ok(u);
    }
    let device = device_of(handle.device_idx);
    release_hold();
    trim_pool(device);
    let quant = quant_of(handle.quant_idx, device);
    let (value, bytes) = crate::models::measure(|| {
        let unet = model.model.load_unet(quant).map_err(|e| e.to_string())?;
        Ok(UnetShared { unet })
    })?;
    let arc = Arc::new(value);
    g.insert(key.clone(), Arc::downgrade(&arc));
    crate::models::register_weak(
        format!("sdxl/unet/{key}"),
        "SDXL",
        "UNet",
        label(handle),
        device,
        bytes,
        Arc::downgrade(&arc),
        release_hold,
    );
    Ok(arc)
}

/// «Держать в памяти»: UNet переживает прогон до смены чекпойнта или
/// выгрузки из панели моделей.
pub fn hold(u: Arc<UnetShared>) {
    let h = HOLD.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = h.lock() {
        *g = Some(u);
    }
}

pub fn release_hold() {
    let h = HOLD.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = h.lock() {
        *g = None;
    }
}

pub fn cond_key(handle: &FluxModelHandle, prompt: &str, negative: &str) -> String {
    format!("{}|{}|{prompt}|{negative}", handle.model_path.display(), handle.device_idx)
}

pub fn cached_conditioning(key: &str) -> Option<Arc<SdxlConditioning>> {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    let g = c.lock().ok()?;
    g.as_ref().filter(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn remember_conditioning(key: String, cond: Arc<SdxlConditioning>) {
    let c = LAST_COND.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = c.lock() {
        *g = Some((key, cond));
    }
}

/// Вернуть драйверу память всех пулов.
pub fn trim_pool(device: Device) {
    if let Device::Cuda(ord) = device {
        let _ = synaptix_core::device::cuda::synchronize_all(ord);
        let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(ord);
    }
}

pub fn is_oom(msg: &str) -> bool {
    msg.contains("OOM") || msg.contains("OUT_OF_MEMORY")
}

/// Стадия, которой на малой карте может не хватить места рядом с UNet из
/// hold-слота: при OOM UNet отпускается и стадия повторяется один раз.
pub fn with_vram_retry<T>(device: Device, f: impl Fn() -> Result<T, String>) -> Result<T, String> {
    match f() {
        Err(e) if is_oom(&e) => {
            tracing::info!(target: super::super::WORKER_LOG, "OOM рядом с удержанным UNet SDXL — отпускаю его и повторяю стадию");
            release_hold();
            trim_pool(device);
            f()
        }
        other => other,
    }
}
