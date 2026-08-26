use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_video_minimax_h3 as h3;
use syngui::prelude::*;

use super::super::super::types::H3ModelHandle;
use super::{compute_of, device_of, memory_mode_of, quant_dit_of, quant_enc_of, variant_of};

static KERNELS: OnceLock<()> = OnceLock::new();

pub fn ensure_kernels_registered() {
    KERNELS.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
}

pub struct DitShared {
    pub dit: h3::dit::H3Dit,
    pub ckpt: h3::H3Checkpoint,
    pub device: Device,
    pub compute: DType,
}

pub struct EncoderShared {
    pub encoder: h3::text_encoder::EncoderHandle,
}

pub struct VaeShared {
    pub decoder: h3::vae::VaeDecoder,
}

pub struct AudioVaeShared {
    pub decoder: h3::audio_vae::AudioVae,
}

type Cache<T> = OnceLock<Mutex<HashMap<String, Weak<T>>>>;

fn cache_key(h: &H3ModelHandle) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        h.model_path.display(),
        h.encoder_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        h.lora_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        h.lora_strength.to_bits(),
        h.variant_idx,
        h.device_idx,
        h.quant_dit_idx,
        h.quant_enc_idx,
        h.compute_idx,
        h.memory_mode_idx
    )
}

static DIT: Cache<DitShared> = OnceLock::new();
static ENCODER: Cache<EncoderShared> = OnceLock::new();
static VAE: Cache<VaeShared> = OnceLock::new();
static AUDIO_VAE: Cache<AudioVaeShared> = OnceLock::new();
static HOLD: OnceLock<Mutex<Vec<Arc<DitShared>>>> = OnceLock::new();

/// Что показать про компонент в панели загруженных моделей и как его
/// оттуда выгрузить. `unload` — функция семейства, сбрасывающая
/// strong-ссылку (кэш держит только `Weak`, мешать не будет); для
/// компонентов без hold-слота это no-op: они умирают сами, как только
/// воркер их отпустил.
struct Reg {
    component: &'static str,
    label: String,
    device: Device,
    unload: fn(),
}

fn get_or_load<T: Send + Sync + 'static>(
    cache: &Cache<T>,
    key: &str,
    reg: Reg,
    loader: impl FnOnce() -> std::result::Result<T, String>,
) -> std::result::Result<Arc<T>, String> {
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| tr!("node.minimax_h3_shared.cache_poisoned"))?;
    g.retain(|_, v| v.strong_count() > 0);
    if let Some(existing) = g.get(key).and_then(|w| w.upgrade()) {
        return Ok(existing);
    }
    // Регистрируем только настоящую загрузку: на попадании в кэш запись
    // уже есть, и перерегистрация затёрла бы измеренный размер нулём.
    let (value, bytes) = crate::models::measure(loader)?;
    let arc = Arc::new(value);
    g.insert(key.to_string(), Arc::downgrade(&arc));
    crate::models::register_weak(
        format!("h3/{}/{key}", reg.component),
        "MiniMax-H3",
        reg.component,
        reg.label,
        reg.device,
        bytes,
        Arc::downgrade(&arc),
        reg.unload,
    );
    Ok(arc)
}

/// Источник весов: `.syn`-бандл или HF-каталог варианта. Открытие бандла —
/// это только mmap + разбор central directory, веса не читаются.
pub fn source_of(handle: &H3ModelHandle) -> std::result::Result<h3::H3Source, String> {
    h3::H3Source::open(&handle.model_path, variant_of(handle.variant_idx))
        .map_err(|e| e.to_string())
}

/// Источник энкодера: явно выбранный `.syn`/каталог либо энкодер из модели.
pub fn encoder_source_of(
    handle: &H3ModelHandle,
    model: &h3::H3Source,
) -> std::result::Result<h3::H3EncoderSource, String> {
    match &handle.encoder_path {
        Some(p) => h3::H3EncoderSource::open(p).map_err(|e| e.to_string()),
        None => h3::H3EncoderSource::from_model(model).ok_or_else(|| {
            tr!(
                "node.minimax_h3_shared.need_separate_encoder",
                path = model.path().display()
            )
        }),
    }
}

pub fn load_dit(handle: &H3ModelHandle) -> std::result::Result<Arc<DitShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(
        &DIT,
        &key,
        Reg {
            component: "DiT",
            label: bundle_label(handle),
            device: device_of(handle.device_idx),
            unload: release_dit_hold,
        },
        move || {
        let device = device_of(handle.device_idx);
        h3::memory::trim_pool(device);
        let compute = compute_of(handle.compute_idx);
        let quant = quant_dit_of(handle.quant_dit_idx, compute);
        memory_mode_of(handle.memory_mode_idx).install();

        let source = source_of(handle)?;
        let mut ckpt =
            h3::H3Checkpoint::open_source(source, device, compute).map_err(|e| e.to_string())?;
        if let Some(lp) = &handle.lora_path {
            let lw = h3::LoraWeights::open(lp, device, handle.lora_strength)
                .map_err(|e| e.to_string())?;
            ckpt = ckpt.with_lora(Arc::new(lw));
        }
        let dit = h3::dit::H3Dit::load(&ckpt, device, compute, quant).map_err(|e| e.to_string())?;
        Ok(DitShared { dit, ckpt, device, compute })
        },
    )
}

pub fn load_encoder(handle: &H3ModelHandle) -> std::result::Result<Arc<EncoderShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(
        &ENCODER,
        &key,
        Reg {
            component: "Text Encoder",
            label: encoder_label(handle),
            device: device_of(handle.device_idx),
            unload: || {},
        },
        move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let quant = quant_enc_of(handle.quant_enc_idx, compute);
        let source = source_of(handle)?;
        let enc_src = encoder_source_of(handle, &source)?;
        let encoder = h3::text_encoder::EncoderHandle::load_source(&enc_src, device, compute, quant)
            .map_err(|e| e.to_string())?;
        Ok(EncoderShared { encoder })
        },
    )
}

pub fn load_vae(handle: &H3ModelHandle) -> std::result::Result<Arc<VaeShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(
        &VAE,
        &key,
        Reg {
            component: "Video VAE",
            label: bundle_label(handle),
            device: device_of(handle.device_idx),
            unload: || {},
        },
        move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let source = source_of(handle)?;
        let cfg = h3::config::VaeConfig::from_source(&source).map_err(|e| e.to_string())?;
        let w = h3::loader::ComponentLoader::open_component(&source, h3::H3Component::VideoVae, device)
            .map_err(|e| e.to_string())?;
        let decoder =
            h3::vae::VaeDecoder::load(&w, cfg, device, compute).map_err(|e| e.to_string())?;
        Ok(VaeShared { decoder })
        },
    )
}

pub fn load_audio_vae(handle: &H3ModelHandle) -> std::result::Result<Arc<AudioVaeShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(
        &AUDIO_VAE,
        &key,
        Reg {
            component: "Audio VAE",
            label: bundle_label(handle),
            device: device_of(handle.device_idx),
            unload: || {},
        },
        move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let source = source_of(handle)?;
        let cfg = h3::config::AudioVaeConfig::from_source(&source).map_err(|e| e.to_string())?;
        let w = h3::loader::ComponentLoader::open_component(&source, h3::H3Component::AudioVae, device)
            .map_err(|e| e.to_string())?;
        let decoder = h3::audio_vae::AudioVae::load_decoder(&w, cfg, device, compute)
            .map_err(|e| e.to_string())?;
        Ok(AudioVaeShared { decoder })
        },
    )
}

/// Подпись бандла для панели моделей: имя файла `.syn` плюс вариант,
/// если их в бандле несколько.
fn bundle_label(h: &H3ModelHandle) -> String {
    h.model_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| h.model_path.display().to_string())
}

/// Энкодер может лежать отдельным `.syn` — тогда в панели показываем его,
/// а не бандл модели.
fn encoder_label(h: &H3ModelHandle) -> String {
    match &h.encoder_path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => bundle_label(h),
    }
}

/// Удержать DiT между Sampler → VAE Decode (иначе Weak-кэш дропнул бы
/// его сразу после воркера сэмплера, и decode грузил бы 22B заново).
/// Снимается в decode-нодах через [`release_dit_hold`].
///
/// Дедуп по указателю: без него каждый прогон дописывал в HOLD ещё одну
/// ссылку на тот же DiT, и вектор рос от прогона к прогону.
pub fn hold_dit(shared: Arc<DitShared>) {
    let h = HOLD.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = h.lock() {
        if !g.iter().any(|x| Arc::ptr_eq(x, &shared)) {
            g.push(shared);
        }
    }
}

pub fn release_dit_hold() {
    let h = HOLD.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = h.lock() {
        g.clear();
    }
}

pub fn trim_pool(handle: &H3ModelHandle) {
    h3::memory::trim_pool(device_of(handle.device_idx));
}

pub fn activation_anchor(
    handle: &H3ModelHandle,
    bytes: usize,
) -> Option<synaptix_core::tensor::Tensor> {
    synaptix_core::tensor::Tensor::empty_uninit(
        vec![bytes],
        synaptix_core::dtype::DType::U8,
        device_of(handle.device_idx),
    )
    .ok()
}
