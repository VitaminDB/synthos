use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_video_minimax_h3 as h3;

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
        h.model_dir.display(),
        h.encoder_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
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

fn get_or_load<T>(
    cache: &Cache<T>,
    key: &str,
    loader: impl FnOnce() -> std::result::Result<T, String>,
) -> std::result::Result<Arc<T>, String> {
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| "кэш моделей отравлен".to_string())?;
    g.retain(|_, v| v.strong_count() > 0);
    if let Some(existing) = g.get(key).and_then(|w| w.upgrade()) {
        return Ok(existing);
    }
    let arc = Arc::new(loader()?);
    g.insert(key.to_string(), Arc::downgrade(&arc));
    Ok(arc)
}

pub fn paths_of(handle: &H3ModelHandle) -> std::result::Result<h3::H3Paths, String> {
    h3::H3Paths::open_variant(&handle.model_dir, variant_of(handle.variant_idx))
        .map_err(|e| e.to_string())
}

pub fn load_dit(handle: &H3ModelHandle) -> std::result::Result<Arc<DitShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(&DIT, &key, move || {
        let device = device_of(handle.device_idx);
        h3::memory::trim_pool(device);
        let compute = compute_of(handle.compute_idx);
        let quant = quant_dit_of(handle.quant_dit_idx, compute);
        memory_mode_of(handle.memory_mode_idx).install();

        let paths = paths_of(handle)?;
        let mut ckpt = h3::H3Checkpoint::open(paths, device, compute).map_err(|e| e.to_string())?;
        if let Some(lp) = &handle.lora_path {
            let lw = h3::LoraWeights::open(lp, device, handle.lora_strength)
                .map_err(|e| e.to_string())?;
            ckpt = ckpt.with_lora(Arc::new(lw));
        }
        let dit = h3::dit::H3Dit::load(&ckpt, device, compute, quant).map_err(|e| e.to_string())?;
        Ok(DitShared { dit, ckpt, device, compute })
    })
}

pub fn load_encoder(handle: &H3ModelHandle) -> std::result::Result<Arc<EncoderShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(&ENCODER, &key, move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let quant = quant_enc_of(handle.quant_enc_idx, compute);
        let paths = paths_of(handle)?;
        let dir = handle
            .encoder_dir
            .clone()
            .unwrap_or_else(|| paths.text_encoder_dir());
        let encoder = h3::text_encoder::EncoderHandle::load(&dir, device, compute, quant)
            .map_err(|e| e.to_string())?;
        Ok(EncoderShared { encoder })
    })
}

pub fn load_vae(handle: &H3ModelHandle) -> std::result::Result<Arc<VaeShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(&VAE, &key, move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let paths = paths_of(handle)?;
        let cfg = h3::config::VaeConfig::from_dir(&paths.root).map_err(|e| e.to_string())?;
        let w = h3::loader::ComponentLoader::open_file(paths.video_vae_file(), device)
            .map_err(|e| e.to_string())?;
        let decoder =
            h3::vae::VaeDecoder::load(&w, cfg, device, compute).map_err(|e| e.to_string())?;
        Ok(VaeShared { decoder })
    })
}

pub fn load_audio_vae(handle: &H3ModelHandle) -> std::result::Result<Arc<AudioVaeShared>, String> {
    ensure_kernels_registered();
    let key = cache_key(handle);
    get_or_load(&AUDIO_VAE, &key, move || {
        let device = device_of(handle.device_idx);
        let compute = compute_of(handle.compute_idx);
        let paths = paths_of(handle)?;
        let cfg = h3::config::AudioVaeConfig::from_dir(&paths.root).map_err(|e| e.to_string())?;
        let w = h3::loader::ComponentLoader::open_file(paths.audio_vae_file(), device)
            .map_err(|e| e.to_string())?;
        let decoder = h3::audio_vae::AudioVae::load_decoder(&w, cfg, device, compute)
            .map_err(|e| e.to_string())?;
        Ok(AudioVaeShared { decoder })
    })
}

pub fn hold_dit(shared: Arc<DitShared>) {
    let h = HOLD.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = h.lock() {
        g.push(shared);
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
