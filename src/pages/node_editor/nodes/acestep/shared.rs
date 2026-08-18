//! Shared-Model Registry для ACE-Step нод (нативный synaptix-бэкенд).
//!
//! Кэш по `(path, device_idx, storage_idx, compute_idx)` через `Weak<T>`:
//! ноды с идентичной конфигурацией делят один `Arc<T>`. Загрузка из `.syn`
//! через `synaptix-music-acestep` (VAE/LM/DiT/encoders/FSQ/detok).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use crate::context::AppCtx;
use syngui::context_provider::use_context;
use syngui::core::sync::Mutex;

use synaptix_bundle::Bundle;
use synaptix_core::{device::Device, dtype::DType, tensor::Tensor};
use synaptix_music_acestep::cond_encoder::ConditionEncoder;
use synaptix_music_acestep::config::DitConfig;
use synaptix_music_acestep::detokenizer::Detokenizer;
use synaptix_music_acestep::dit::Dit;
use synaptix_music_acestep::fsq::Fsq;
use synaptix_music_acestep::lm::AceStepLm;
use synaptix_music_acestep::loader::CompLoader;
use synaptix_music_acestep::text_encoder::TextEncoder;
use synaptix_music_acestep::tokenizer::AceTokenizer;
use synaptix_music_acestep::vae::AceStepVae;
use synaptix_tokenizer::hf::HfTokenizer;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub path: PathBuf,
    pub device_idx: usize,
    pub storage_idx: usize,
    pub compute_idx: usize,
}

impl ModelKey {
    pub fn new(path: impl Into<PathBuf>, device_idx: usize, storage_idx: usize, compute_idx: usize) -> Self {
        Self { path: path.into(), device_idx, storage_idx, compute_idx }
    }
}

/// `component` — как компонент зовётся в панели загруженных моделей;
/// `None` для мелочи вроде токенизаторов и null-эмбеддинга: в списке
/// «что занимает память» им делать нечего. Strong-hold'ов у ACE-Step нет —
/// компоненты живут ровно столько, сколько их держит воркер, поэтому
/// `unload` здесь no-op, а панель покажет запись только пока она живая.
fn get_or_load<T>(
    cache: &'static OnceLock<Mutex<HashMap<ModelKey, Weak<T>>>>,
    key: ModelKey,
    component: Option<&'static str>,
    loader: impl FnOnce(&ModelKey) -> Result<T, String>,
) -> Result<Arc<T>, String>
where
    T: Send + Sync + 'static,
{
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| "shared-model cache mutex poisoned".to_string())?;
    g.retain(|_, w| w.strong_count() > 0);
    if let Some(arc) = g.get(&key).and_then(|w| w.upgrade()) {
        return Ok(arc);
    }
    let device = super::device_from_idx(key.device_idx);
    let label = key
        .path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| key.path.display().to_string());
    // Регистрируем только настоящую загрузку: на попадании в кэш запись
    // уже есть, и перерегистрация затёрла бы измеренный размер нулём.
    let (value, bytes) = crate::models::measure(|| loader(&key))?;
    let arc = Arc::new(value);
    if let Some(component) = component {
        crate::models::register_weak(
            format!("acestep/{component}/{}/{}", key.path.display(), key.device_idx),
            "ACE-Step",
            component,
            label,
            device,
            bytes,
            Arc::downgrade(&arc),
            || {},
        );
    }
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

fn ensure_kernels() {
    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();
}

fn dev_dtype(k: &ModelKey) -> (Device, DType) {
    ensure_kernels();
    (super::device_from_idx(k.device_idx), super::compute_from_idx(k.compute_idx))
}

fn quant_or_dense(storage_idx: usize, compute: DType) -> (DType, DType) {
    match super::quant_from_idx(storage_idx) {
        Some(qd) => (compute, qd),
        None => (compute, compute),
    }
}

/// Конфиг DiT по варианту bundle (base/turbo отличаются димами).
fn dit_config_for(path: &Path) -> DitConfig {
    match detect_xl_bundle_kind(path) {
        XlBundleKind::Turbo => DitConfig::xl_turbo(),
        _ => DitConfig::xl_base(),
    }
}

fn comp_loader(path: &Path, device: Device) -> Result<CompLoader, String> {
    CompLoader::open(path, None, device).map_err(|e| e.to_string())
}

// ── Caches ────────────────────────────────────────────────────────────────
static VAE_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<AceStepVae>>>> = OnceLock::new();
static DIT_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Dit>>>> = OnceLock::new();
static COND_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<ConditionEncoder>>>> = OnceLock::new();
static FSQ_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Fsq>>>> = OnceLock::new();
static DETOK_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Detokenizer>>>> = OnceLock::new();
static NULL_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Tensor>>>> = OnceLock::new();
static TEXT_ENC_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<TextEncoder>>>> = OnceLock::new();
static TEXT_TOK_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<HfTokenizer>>>> = OnceLock::new();
static AR_LM_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<AceStepLm>>>> = OnceLock::new();
static AR_TOK_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<AceTokenizer>>>> = OnceLock::new();

// ── Loaders ─────────────────────────────────────────────────────────────────

pub fn load_vae(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<AceStepVae>, String> {
    get_or_load(&VAE_CACHE, ModelKey::new(path, di, si, ci), Some("VAE"), |k| {
        let (device, _) = dev_dtype(k);
        AceStepVae::open(&k.path, device).map_err(|e| format!("AceStepVae::open: {e}"))
    })
}

pub fn load_dit(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<Dit>, String> {
    get_or_load(&DIT_CACHE, ModelKey::new(path, di, si, ci), Some("DiT"), |k| {
        let (device, compute) = dev_dtype(k);
        // compute = выбранный (bf16/f32); квант весов attn/mlp = дропдаун Quant
        // (none → qdt==compute = Dense, бит-в-бит). compute остаётся bf16 при
        // кванте (как LTX). Денойз сам гейтит CUDA-graph off при кванте.
        let qdt = super::quant_from_idx(k.storage_idx).unwrap_or(compute);
        let ck = comp_loader(&k.path, device)?;
        Dit::load(&ck, &dit_config_for(&k.path), compute, qdt).map_err(|e| format!("Dit::load: {e}"))
    })
}

pub fn load_cond_encoder(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<ConditionEncoder>, String> {
    get_or_load(&COND_CACHE, ModelKey::new(path, di, si, ci), Some("Condition Encoder"), |k| {
        let (device, _) = dev_dtype(k);
        let ck = comp_loader(&k.path, device)?;
        ConditionEncoder::load(&ck, &dit_config_for(&k.path)).map_err(|e| format!("ConditionEncoder::load: {e}"))
    })
}

pub fn load_fsq(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<Fsq>, String> {
    get_or_load(&FSQ_CACHE, ModelKey::new(path, di, si, ci), Some("FSQ"), |k| {
        let (device, _) = dev_dtype(k);
        let ck = comp_loader(&k.path, device)?;
        Fsq::load(&ck, "tokenizer.quantizer").map_err(|e| format!("Fsq::load: {e}"))
    })
}

pub fn load_detokenizer(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<Detokenizer>, String> {
    get_or_load(&DETOK_CACHE, ModelKey::new(path, di, si, ci), Some("Detokenizer"), |k| {
        let (device, _) = dev_dtype(k);
        let ck = comp_loader(&k.path, device)?;
        Detokenizer::load(&ck, &dit_config_for(&k.path)).map_err(|e| format!("Detokenizer::load: {e}"))
    })
}

pub fn load_null_cond(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<Tensor>, String> {
    get_or_load(&NULL_CACHE, ModelKey::new(path, di, si, ci), None, |k| {
        let (device, _) = dev_dtype(k);
        let ck = comp_loader(&k.path, device)?;
        ck.f32("null_condition_emb").map_err(|e| format!("null_condition_emb: {e}"))
    })
}

pub fn load_text_encoder(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<TextEncoder>, String> {
    get_or_load(&TEXT_ENC_CACHE, ModelKey::new(path, di, si, ci), Some("Text Encoder"), |k| {
        let (device, compute) = dev_dtype(k);
        let (comp, qw) = quant_or_dense(k.storage_idx, compute);
        TextEncoder::open(&k.path, device, comp, qw, 4096).map_err(|e| format!("TextEncoder::open: {e}"))
    })
}

pub fn load_text_tokenizer(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<HfTokenizer>, String> {
    get_or_load(&TEXT_TOK_CACHE, ModelKey::new(path, di, si, ci), None, |k| {
        let bytes = read_bundle_bytes(&k.path, "tokenizer.json")?;
        HfTokenizer::from_bytes(&bytes).map_err(|e| format!("HfTokenizer: {e}"))
    })
}

pub fn load_ar_lm(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<AceStepLm>, String> {
    get_or_load(&AR_LM_CACHE, ModelKey::new(path, di, si, ci), Some("AR LM"), |k| {
        let (device, compute) = dev_dtype(k);
        let (comp, qw) = quant_or_dense(k.storage_idx, compute);
        AceStepLm::open(&k.path, device, comp, qw, 8192).map_err(|e| format!("AceStepLm::open: {e}"))
    })
}

pub fn load_ar_tokenizer(path: &Path, di: usize, si: usize, ci: usize) -> Result<Arc<AceTokenizer>, String> {
    get_or_load(&AR_TOK_CACHE, ModelKey::new(path, di, si, ci), None, |k| {
        let bytes = read_bundle_bytes(&k.path, "tokenizer.json")?;
        AceTokenizer::from_bytes(&bytes).map_err(|e| format!("AceTokenizer: {e}"))
    })
}

fn read_bundle_bytes(path: &Path, name: &str) -> Result<Vec<u8>, String> {
    synaptix_music_acestep::loader::read_bundle_file(path, name).map_err(|e| e.to_string())
}

// ── Bundle kind + AppCtx paths (без изменений по семантике) ──────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XlBundleKind {
    Base,
    Turbo,
    Sft,
    Unknown,
}

impl XlBundleKind {
    pub fn default_infer_steps(self) -> u32 {
        match self {
            XlBundleKind::Turbo => 8,
            XlBundleKind::Base | XlBundleKind::Unknown => 32,
            XlBundleKind::Sft => 50,
        }
    }
    pub fn default_cfg_scale(self) -> f32 {
        match self {
            XlBundleKind::Turbo => 1.0,
            XlBundleKind::Base | XlBundleKind::Sft | XlBundleKind::Unknown => 7.0,
        }
    }
    pub fn default_flow_match_shift(self) -> f32 {
        match self {
            XlBundleKind::Turbo | XlBundleKind::Base => 3.0,
            XlBundleKind::Sft | XlBundleKind::Unknown => 1.0,
        }
    }
}

pub fn detect_xl_bundle_kind(path: &Path) -> XlBundleKind {
    if let Ok(b) = Bundle::open(path) {
        let id = b.id().to_lowercase();
        if id.contains("turbo") {
            return XlBundleKind::Turbo;
        }
        if id.contains("sft") {
            return XlBundleKind::Sft;
        }
        if id.contains("base") {
            return XlBundleKind::Base;
        }
    }
    let name = path.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.contains("turbo") {
        XlBundleKind::Turbo
    } else if name.contains("sft") {
        XlBundleKind::Sft
    } else if name.contains("base") {
        XlBundleKind::Base
    } else {
        XlBundleKind::Unknown
    }
}

pub fn xl_bundle_path() -> Result<PathBuf, String> {
    let ctx = use_context::<AppCtx>();
    let raw = ctx
        .acestep_xl_bundle_path
        .get_untracked()
        .ok_or_else(|| "Settings → AI Models → ACE-Step: укажите xl-bundle (xl-base / xl-turbo)".to_string())?;
    let p = PathBuf::from(&raw);
    if !p.exists() {
        return Err(format!("xl-bundle не найден: {raw}"));
    }
    Ok(p)
}

pub fn vae_bundle_path() -> Result<PathBuf, String> {
    let ctx = use_context::<AppCtx>();
    let raw = ctx
        .acestep_vae_bundle_path
        .get_untracked()
        .ok_or_else(|| "Settings → AI Models → ACE-Step: укажите VAE bundle (acestep_vae.syn)".to_string())?;
    let p = PathBuf::from(&raw);
    if !p.exists() {
        return Err(format!("VAE bundle не найден: {raw}"));
    }
    Ok(p)
}
