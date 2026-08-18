//! Реестр моделей, которые прямо сейчас держат память.
//!
//! Зачем нужен: веса живут в разных местах и по разным правилам. Тяжёлые
//! видео/аудио-семейства (MiniMax-H3, LTX, ACE-Step) кэшируются глобально
//! через `Weak`, а живыми их держит либо активный воркер, либо явный
//! «hold» между нодами пайплайна (`hold_dit` / `hold_avdit`) — чтобы
//! Sampler → VAE Decode не перегружали DiT. Пайплайновые ноды (LLM, TTS,
//! ASR, диаризация) держат модель в собственном `NodeRuntime`. После
//! прогона всё это остаётся резидентным, и до появления панели выгрузить
//! его можно было только перезапуском приложения.
//!
//! Реестр не владеет весами — он хранит `alive`-пробу (жив ли `Weak` /
//! занят ли слот ноды) и `unload`-замыкание владельца. Поэтому [`list`]
//! показывает ровно то, что реально резидентно: запись, чей объект уже
//! дропнут, отсеивается на первом же обращении, а `unload` честно
//! сообщает, освободилась модель или её ещё держит активный воркер.
//!
//! Размер записи меряется дельтой счётчика CUDA-пула вокруг загрузки
//! ([`measure`]). Для CPU-моделей дельта нулевая — панель показывает
//! такие записи без размера, а не врёт числом.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use synaptix_core::device::Device;
use syngui::prelude::*;
use tracing::info;

/// Target логов реестра: регистрация и выгрузка моделей.
pub const MODELS_LOG: &str = "models";

type Probe = Arc<dyn Fn() -> bool + Send + Sync>;
type Unload = Arc<dyn Fn() + Send + Sync>;

struct Entry {
    id: u64,
    /// Стабильный ключ владельца: повторная регистрация с тем же ключом
    /// заменяет запись, а не плодит дубль (нода перезагрузила модель в
    /// тот же слот `NodeRuntime`).
    key: String,
    family: &'static str,
    component: &'static str,
    label: String,
    device: Device,
    bytes: u64,
    alive: Probe,
    unload: Unload,
}

/// Снимок записи для UI.
#[derive(Clone, PartialEq)]
pub struct ModelInfo {
    pub id: u64,
    /// Семейство: «MiniMax-H3», «LTX», «ACE-Step», «LLM»…
    pub family: &'static str,
    /// Компонент внутри семейства: «DiT», «Text Encoder», «VAE»…
    pub component: &'static str,
    /// Короткое имя источника весов (обычно имя файла бандла).
    pub label: String,
    /// «CUDA:0» / «CPU».
    pub device: String,
    /// Прирост CUDA-пула на загрузке; `0` — неизвестно (CPU-модель).
    pub bytes: u64,
}

fn entries() -> &'static Mutex<Vec<Entry>> {
    static ENTRIES: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(Vec::new()))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static VERSION: AtomicU64 = AtomicU64::new(0);
static VERSION_SIGNAL: OnceLock<RwSignal<u64>> = OnceLock::new();

/// Завести сигнал версии реестра. Вызывается один раз на старте с
/// main-thread'а: `use_signal` работает только там.
pub fn install() {
    let _ = VERSION_SIGNAL.set(use_signal(0_u64));
}

/// Сигнал версии: `get()` внутри `Reactive` подписывает панель на
/// появление и выгрузку моделей.
pub fn version() -> Option<RwSignal<u64>> {
    VERSION_SIGNAL.get().copied()
}

/// Дёрнуть версию. Зовётся и из воркеров: `RwSignal::set` сам уезжает на
/// main-thread, а вот читать сигнал из чужого потока нельзя — поэтому
/// новое значение берём из атомика, а не из самого сигнала.
fn bump() {
    let v = VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(sig) = VERSION_SIGNAL.get() {
        sig.set(v);
    }
}

/// Человекочитаемое имя устройства.
pub fn device_label(dev: Device) -> String {
    match dev {
        Device::Cpu => "CPU".to_string(),
        Device::Cuda(ord) => format!("CUDA:{ord}"),
        Device::Metal(ord) => format!("Metal:{ord}"),
        Device::Wgpu(ord) => format!("GPU:{ord}"),
    }
}

/// Байты, занятые CUDA-пулом synaptix прямо сейчас.
pub fn cuda_allocated() -> u64 {
    synaptix_core::memory::cuda_pool::cuda_allocated_bytes() as u64
}

/// Замерить прирост CUDA-пула на загрузке модели.
pub fn measure<T>(
    f: impl FnOnce() -> std::result::Result<T, String>,
) -> std::result::Result<(T, u64), String> {
    let before = cuda_allocated();
    let v = f()?;
    Ok((v, cuda_allocated().saturating_sub(before)))
}

/// Зарегистрировать модель из `Weak`-кэша. `unload` обязан сбросить
/// strong-ссылки владельца (hold-слоты семейства); сам кэш держит только
/// `Weak` и мешать не будет.
#[allow(clippy::too_many_arguments)]
pub fn register_weak<T: Send + Sync + 'static>(
    key: impl Into<String>,
    family: &'static str,
    component: &'static str,
    label: impl Into<String>,
    device: Device,
    bytes: u64,
    weak: Weak<T>,
    unload: impl Fn() + Send + Sync + 'static,
) -> u64 {
    let alive: Probe = Arc::new(move || weak.strong_count() > 0);
    push(
        key.into(),
        family,
        component,
        label.into(),
        device,
        bytes,
        alive,
        Arc::new(unload),
    )
}

/// Зарегистрировать модель, живущую в слоте `NodeRuntime`
/// (`Arc<Mutex<Option<T>>>`). Выгрузка — обнуление слота плюс `after`
/// для сопутствующего стейта ноды (`loaded_cfg`, подпись в карточке).
#[allow(clippy::too_many_arguments)]
pub fn register_slot<T: Send + 'static>(
    key: impl Into<String>,
    family: &'static str,
    component: &'static str,
    label: impl Into<String>,
    device: Device,
    bytes: u64,
    slot: Arc<Mutex<Option<T>>>,
    after: impl Fn() + Send + Sync + 'static,
) -> u64 {
    // Пробы и выгрузка идут с main-thread'а (панель рисуется в UI-потоке),
    // а воркер держит тот же слот всё время инференса — `lock()` здесь
    // подвесил бы интерфейс на всю генерацию. Поэтому `try_lock`:
    // «занято» трактуем как «жива», и выгрузка честно не срабатывает.
    let probe_slot = slot.clone();
    let alive: Probe = Arc::new(move || match probe_slot.try_lock() {
        Ok(g) => g.is_some(),
        Err(std::sync::TryLockError::WouldBlock) => true,
        Err(std::sync::TryLockError::Poisoned(g)) => g.into_inner().is_some(),
    });
    let unload: Unload = Arc::new(move || {
        if let Ok(mut g) = slot.try_lock() {
            *g = None;
            after();
        }
    });
    push(
        key.into(),
        family,
        component,
        label.into(),
        device,
        bytes,
        alive,
        unload,
    )
}

#[allow(clippy::too_many_arguments)]
fn push(
    key: String,
    family: &'static str,
    component: &'static str,
    label: String,
    device: Device,
    bytes: u64,
    alive: Probe,
    unload: Unload,
) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut g) = entries().lock() {
        g.retain(|e| e.key != key && (e.alive)());
        g.push(Entry {
            id,
            key,
            family,
            component,
            label: label.clone(),
            device,
            bytes,
            alive,
            unload,
        });
    }
    info!(
        target: MODELS_LOG,
        id,
        family,
        component,
        label = %label,
        device = %device_label(device),
        bytes,
        "модель загружена"
    );
    bump();
    id
}

/// Живые записи реестра. Мёртвые вычищаются здесь же — отдельный
/// GC-таймер не нужен, панель перечитывает список на каждой перерисовке.
pub fn list() -> Vec<ModelInfo> {
    let Ok(mut g) = entries().lock() else {
        return Vec::new();
    };
    let before = g.len();
    g.retain(|e| (e.alive)());
    let dropped = before - g.len();
    let out: Vec<ModelInfo> = g
        .iter()
        .map(|e| ModelInfo {
            id: e.id,
            family: e.family,
            component: e.component,
            label: e.label.clone(),
            device: device_label(e.device),
            bytes: e.bytes,
        })
        .collect();
    drop(g);
    if dropped > 0 {
        bump();
    }
    out
}

/// Выгрузить одну запись. `true` — объект действительно умер; `false` —
/// на модель ещё держится strong-ссылка (идёт прогон), запись остаётся
/// в списке, и панель честно покажет её на месте.
pub fn unload(id: u64) -> bool {
    let found = {
        let Ok(g) = entries().lock() else { return false };
        g.iter().find(|e| e.id == id).map(|e| {
            (
                e.unload.clone(),
                e.alive.clone(),
                e.family,
                e.component,
                e.label.clone(),
                e.device,
            )
        })
    };
    let Some((unload, alive, family, component, label, device)) = found else {
        return false;
    };
    unload();
    let released = !alive();
    if released {
        trim_device(device);
    }
    info!(
        target: MODELS_LOG,
        id, family, component, label = %label, released,
        "выгрузка модели"
    );
    if let Ok(mut g) = entries().lock() {
        g.retain(|e| (e.alive)());
    }
    bump();
    released
}

/// Выгрузить всё. Возвращает число освободившихся записей.
pub fn unload_all() -> usize {
    let all: Vec<u64> = {
        let Ok(g) = entries().lock() else { return 0 };
        g.iter().map(|e| e.id).collect()
    };
    all.into_iter().filter(|id| unload(*id)).count()
}

/// Вернуть драйверу незанятый резерв CUDA-пула. Веса не трогаются —
/// освобождается только то, что пул держал про запас.
pub fn trim_device(device: Device) {
    if let Device::Cuda(ord) = device {
        let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(ord);
    }
}

/// Trim по всем устройствам, на которых что-то грузилось (плюс нулевое,
/// если CUDA вообще доступна).
pub fn trim_all() {
    let mut ords: Vec<usize> = Vec::new();
    if let Ok(g) = entries().lock() {
        for e in g.iter() {
            if let Device::Cuda(o) = e.device {
                if !ords.contains(&o) {
                    ords.push(o);
                }
            }
        }
    }
    if ords.is_empty() && synaptix_core::device::cuda::mem_info(0).is_ok() {
        ords.push(0);
    }
    for o in ords {
        let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(o);
    }
    bump();
}

/// «12.4 ГБ» / «834 МБ» / «—» для нулевого размера.
pub fn human_bytes(b: u64) -> String {
    if b == 0 {
        return "—".to_string();
    }
    const KB: f64 = 1024.0;
    let f = b as f64;
    if f >= KB * KB * KB {
        format!("{:.1} ГБ", f / (KB * KB * KB))
    } else if f >= KB * KB {
        format!("{:.0} МБ", f / (KB * KB))
    } else {
        format!("{:.0} КБ", f / KB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(0), "—");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 ГБ");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5 МБ");
    }

    /// Мёртвая запись не должна показываться: панель обещает «что реально
    /// висит в памяти», а не «что когда-то грузили».
    #[test]
    fn dead_entry_disappears() {
        let arc = Arc::new(7_u32);
        let weak = Arc::downgrade(&arc);
        let id = register_weak(
            "test/dead",
            "test",
            "unit",
            "x",
            Device::Cpu,
            0,
            weak,
            || {},
        );
        assert!(list().iter().any(|m| m.id == id));
        drop(arc);
        assert!(!list().iter().any(|m| m.id == id));
    }

    /// Слот-запись жива, пока в слоте что-то лежит, и умирает от unload.
    #[test]
    fn slot_entry_unloads() {
        let slot = Arc::new(Mutex::new(Some(vec![1_u8, 2, 3])));
        let id = register_slot(
            "test/slot",
            "test",
            "slot",
            "y",
            Device::Cpu,
            0,
            slot.clone(),
            || {},
        );
        assert!(list().iter().any(|m| m.id == id));
        assert!(unload(id));
        assert!(slot.lock().unwrap().is_none());
        assert!(!list().iter().any(|m| m.id == id));
    }

    /// Повторная регистрация тем же ключом заменяет запись, а не двоит:
    /// нода, перезагрузившая модель в тот же слот, остаётся одной строкой.
    #[test]
    fn same_key_replaces() {
        let slot = Arc::new(Mutex::new(Some(1_u8)));
        let k = "test/replace";
        let first = register_slot(k, "test", "slot", "a", Device::Cpu, 0, slot.clone(), || {});
        let second = register_slot(k, "test", "slot", "b", Device::Cpu, 0, slot.clone(), || {});
        let l = list();
        assert!(!l.iter().any(|m| m.id == first));
        assert_eq!(l.iter().filter(|m| m.id == second).count(), 1);
        unload(second);
    }
}
