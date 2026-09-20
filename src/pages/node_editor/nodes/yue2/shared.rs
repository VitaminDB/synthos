//! Общее для нод YuE2: резолв путей бандлов и кэш загруженных моделей.
//!
//! Костяк (AR + NAR) и декодер живут в кэше по ключу
//! `(путь, устройство, квант, compute)`: ноды с одинаковой настройкой делят
//! один `Arc`, а панель «Модели в памяти» видит их и умеет выгружать.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use syngui::core::sync::Mutex;
use syngui::tr;

use synaptix_core::{device::Device, dtype::DType};
use synaptix_music_yue2::ar::Yue2Ar;
use synaptix_music_yue2::nar::Yue2Nar;
use synaptix_music_yue2::pipeline::{MODEL_NAMES, VAE_NAMES};
use synaptix_music_yue2::protocol::CONTEXT;
use synaptix_music_yue2::vae::Yue2Vae;
use synaptix_music_yue2::Yue2Tokenizer;

use crate::pages::node_editor::types::Yue2ModelHandle;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub path: PathBuf,
    pub device_idx: usize,
    pub quant_idx: usize,
    pub compute_idx: usize,
}

fn get_or_load<T>(
    cache: &'static OnceLock<Mutex<HashMap<ModelKey, Weak<T>>>>,
    key: ModelKey,
    component: &'static str,
    loader: impl FnOnce(&ModelKey) -> Result<T, String>,
) -> Result<Arc<T>, String>
where
    T: Send + Sync + 'static,
{
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| "кэш моделей YuE2: mutex poisoned".to_string())?;
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
    let (value, bytes) = crate::models::measure(|| loader(&key))?;
    remember_size(component, &key, bytes);
    let arc = Arc::new(value);
    crate::models::register_weak(
        format!("yue2/{component}/{}/{}", key.path.display(), key.device_idx),
        "YuE2",
        component,
        label,
        device,
        bytes,
        Arc::downgrade(&arc),
        || {},
    );
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

/// Имена компонентов в панели «Модели в памяти» — они же ключи учёта размеров.
const AR_COMPONENT: &str = "AR (партитура и музыка)";
const NAR_COMPONENT: &str = "NAR (акустика)";
const VAE_COMPONENT: &str = "VAE (декодер)";

fn ensure_kernels() {
    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();
}

/// Сколько заняла каждая загрузка. Нужна резидентному слоту: панель «Модели в
/// памяти» иначе показывала бы ноль, даже когда в VRAM лежат гигабайты.
static SIZES: OnceLock<Mutex<HashMap<(String, ModelKey), u64>>> = OnceLock::new();

fn remember_size(component: &str, key: &ModelKey, bytes: u64) {
    let map = SIZES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut g) = map.lock() {
        g.insert((component.to_string(), key.clone()), bytes);
    }
}

fn known_size(component: &str, key: &ModelKey) -> u64 {
    SIZES
        .get()
        .and_then(|m| m.lock().ok().and_then(|g| g.get(&(component.to_string(), key.clone())).copied()))
        .unwrap_or(0)
}

static AR_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Yue2Ar>>>> = OnceLock::new();
static NAR_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Yue2Nar>>>> = OnceLock::new();
static VAE_CACHE: OnceLock<Mutex<HashMap<ModelKey, Weak<Yue2Vae>>>> = OnceLock::new();
static TOK_CACHE: OnceLock<Mutex<HashMap<PathBuf, Weak<Yue2Tokenizer>>>> = OnceLock::new();

pub fn load_ar(path: &Path, device_idx: usize, quant_idx: usize, compute_idx: usize) -> Result<Arc<Yue2Ar>, String> {
    get_or_load(
        &AR_CACHE,
        ModelKey { path: path.to_path_buf(), device_idx, quant_idx, compute_idx },
        AR_COMPONENT,
        |k| {
            ensure_kernels();
            Yue2Ar::open(
                &k.path,
                super::device_from_idx(k.device_idx),
                super::compute_from_idx(k.compute_idx),
                super::quant_from_idx(k.quant_idx),
                CONTEXT,
            )
            .map_err(|e| format!("Yue2Ar::open: {e}"))
        },
    )
}

pub fn load_nar(path: &Path, device_idx: usize, quant_idx: usize, compute_idx: usize) -> Result<Arc<Yue2Nar>, String> {
    get_or_load(
        &NAR_CACHE,
        ModelKey { path: path.to_path_buf(), device_idx, quant_idx, compute_idx },
        NAR_COMPONENT,
        |k| {
            ensure_kernels();
            Yue2Nar::open(
                &k.path,
                super::device_from_idx(k.device_idx),
                super::compute_from_idx(k.compute_idx),
                super::quant_from_idx(k.quant_idx),
            )
            .map_err(|e| format!("Yue2Nar::open: {e}"))
        },
    )
}

/// Декодер. `dtype_idx` — индекс в [`super::VAE_DTYPE_OPTIONS`]; занимает в
/// ключе место кванта, которого у VAE нет.
pub fn load_vae(path: &Path, device_idx: usize, dtype_idx: usize) -> Result<Arc<Yue2Vae>, String> {
    get_or_load(
        &VAE_CACHE,
        ModelKey { path: path.to_path_buf(), device_idx, quant_idx: dtype_idx, compute_idx: 0 },
        VAE_COMPONENT,
        |k| {
            ensure_kernels();
            Yue2Vae::open(
                &k.path,
                super::device_from_idx(k.device_idx),
                super::vae_dtype_from_idx(k.quant_idx),
                true,
            )
            .map_err(|e| format!("Yue2Vae::open: {e}"))
        },
    )
}

/// Текстовый BPE из бандла. В панель памяти не попадает: 2,5 МБ таблицы рангов.
pub fn load_tokenizer(path: &Path) -> Result<Arc<Yue2Tokenizer>, String> {
    let map = TOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().map_err(|_| "кэш токенизатора YuE2: mutex poisoned".to_string())?;
    g.retain(|_, w| w.strong_count() > 0);
    if let Some(arc) = g.get(path).and_then(|w| w.upgrade()) {
        return Ok(arc);
    }
    let raw = synaptix_music_yue2::loader::read_bundle_file(path, "qwen.tiktoken")
        .map_err(|e| format!("qwen.tiktoken: {e}"))?;
    let tok = Yue2Tokenizer::from_tiktoken_bytes(&raw).map_err(|e| format!("токенизатор: {e}"))?;
    let arc = Arc::new(tok);
    g.insert(path.to_path_buf(), Arc::downgrade(&arc));
    Ok(arc)
}

/// Имя бандла, которое реально возьмётся из каталога (для плейсхолдера поля).
pub fn default_bundle_name<'a>(dir: Option<&Path>, names: &'a [&'a str]) -> &'a str {
    dir.and_then(|d| names.iter().find(|n| d.join(n).exists()).copied())
        .unwrap_or(names[0])
}

/// Резолв путей костяка и декодера: override → каталог/первое существующее имя.
/// Относительный override (агент пишет голое `yue2-3b.syn`) считается от
/// каталога моделей, а не от рабочей папки процесса.
pub fn resolve_paths(h: &Yue2ModelHandle) -> Result<(PathBuf, PathBuf), String> {
    let pick = |o: &Option<PathBuf>, names: &[&str]| -> Result<PathBuf, String> {
        if let Some(p) = o {
            if p.is_relative() {
                if let Some(d) = &h.models_dir {
                    return Ok(d.join(p));
                }
            }
            return Ok(p.clone());
        }
        match &h.models_dir {
            Some(d) => Ok(d.join(default_bundle_name(Some(d), names))),
            None => Err(tr!("node.yue2.error.missing_dir_or_override", name = names[0])),
        }
    };
    let model = pick(&h.model_path, MODEL_NAMES)?;
    let vae = pick(&h.vae_path, VAE_NAMES)?;
    for (label, p) in [("model", &model), ("vae", &vae)] {
        if !p.exists() {
            return Err(tr!(
                "node.yue2.error.bundle_not_found",
                label = label,
                path = p.display()
            ));
        }
    }
    Ok((model, vae))
}

/// Устройство и точности хэндла — в одном месте, чтобы воркеры не собирали их
/// из индексов каждый по-своему.
pub fn device_and_dtypes(h: &Yue2ModelHandle) -> (Device, DType, Option<DType>, DType) {
    (
        super::device_from_idx(h.device_idx),
        super::compute_from_idx(h.compute_idx),
        super::quant_from_idx(h.quant_idx),
        super::vae_dtype_from_idx(h.vae_dtype_idx),
    )
}

// ── Резидентность ─────────────────────────────────────────────────────────

/// Что держится в памяти между прогонами при включённом «держать в памяти».
/// Кэш моделей хранит `Weak`, поэтому без этих сильных ссылок веса
/// выгружались бы сразу после воркера, и чекбокс ничего бы не значил.
pub struct Resident {
    pub key: (PathBuf, PathBuf, usize, usize, usize, usize),
    pub ar: Arc<Yue2Ar>,
    pub nar: Arc<Yue2Nar>,
    pub vae: Option<Arc<Yue2Vae>>,
}

fn resident_slot() -> &'static Arc<Mutex<Option<Resident>>> {
    static SLOT: OnceLock<Arc<Mutex<Option<Resident>>>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// Запомнить загруженное до следующего прогона. Смена путей или точностей
/// вытесняет прежний набор — двух копий костяка в VRAM быть не должно.
pub fn keep_resident(
    model: &Path,
    vae_path: &Path,
    h: &Yue2ModelHandle,
    ar: Arc<Yue2Ar>,
    nar: Arc<Yue2Nar>,
    vae: Option<Arc<Yue2Vae>>,
) {
    let key = (
        model.to_path_buf(),
        vae_path.to_path_buf(),
        h.device_idx,
        h.quant_idx,
        h.compute_idx,
        h.vae_dtype_idx,
    );
    let slot = resident_slot();
    let fresh = {
        let Ok(mut g) = slot.lock() else { return };
        let fresh = g.as_ref().map(|r| r.key != key).unwrap_or(true);
        *g = Some(Resident { key, ar, nar, vae });
        fresh
    };
    if fresh {
        let label = model
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| model.display().to_string());
        let backbone =
            ModelKey { path: model.to_path_buf(), device_idx: h.device_idx, quant_idx: h.quant_idx, compute_idx: h.compute_idx };
        let decoder = ModelKey {
            path: vae_path.to_path_buf(),
            device_idx: h.device_idx,
            quant_idx: h.vae_dtype_idx,
            compute_idx: 0,
        };
        let bytes = known_size(AR_COMPONENT, &backbone)
            + known_size(NAR_COMPONENT, &backbone)
            + known_size(VAE_COMPONENT, &decoder);
        crate::models::register_slot(
            "yue2/resident".to_string(),
            "YuE2",
            "Resident (AR+NAR+VAE)",
            label,
            super::device_from_idx(h.device_idx),
            bytes,
            slot.clone(),
            || {},
        );
    }
}

/// Отпустить резидентные веса (чекбокс выключили или выгрузили из панели).
/// Возвращает `true`, если что-то держалось.
pub fn release_resident() -> bool {
    match resident_slot().lock() {
        Ok(mut g) => g.take().is_some(),
        Err(_) => false,
    }
}
