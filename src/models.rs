//! Модели в памяти — у службы нейросетей synaptixd: своих весов Synthos не
//! держит. Здесь — снимок загруженных моделей службы с программами, которые
//! их используют, для панели нодового редактора и инструмента `system`, и
//! выгрузка через службу.
//!
//! Снимок обновляет фоновый поток по событиям службы (подписка) и раз в
//! 2 с — для программ и KV-сессий, о которых событий нет; панель читает
//! кэш, не ходя в сокет из главного потока.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use syngui::prelude::*;

/// Загруженная модель службы.
#[derive(Clone, PartialEq, Debug)]
pub struct ModelInfo {
    /// Id модели в службе.
    pub id: String,
    /// Вид: «Языковая модель», «Картинки»…
    pub family: String,
    /// Название модели.
    pub component: String,
    /// Точность весов («NVFP4», «MXFP8»…).
    pub label: String,
    /// «CUDA» / «CPU» / «NPU».
    pub device: String,
    /// Память модели на её устройстве, байт.
    pub bytes: u64,
    /// Идёт работа.
    pub busy: bool,
    /// Программы, которые держат модель или работают с ней.
    pub apps: Vec<String>,
}

static CACHE: Mutex<Vec<ModelInfo>> = Mutex::new(Vec::new());
static VERSION: AtomicU64 = AtomicU64::new(0);
static VERSION_SIGNAL: OnceLock<RwSignal<u64>> = OnceLock::new();

fn kind_label(k: synaptixd::ModelKind) -> String {
    use synaptixd::ModelKind as K;
    match k {
        K::Llm => tr!("models.kind.llm"),
        K::Image => tr!("models.kind.image"),
        K::Video => tr!("models.kind.video"),
        K::Music => tr!("models.kind.music"),
        K::Tts => tr!("models.kind.tts"),
        K::Asr | K::Ctc | K::Mfa => tr!("models.kind.asr"),
        K::Embed | K::Rerank => tr!("models.kind.embed"),
        K::Diarize => tr!("models.kind.diarize"),
    }
}

pub fn device_name(d: synaptixd::DeviceKind) -> String {
    d.id().to_uppercase()
}

fn fetch() -> synaptixd::Result<Vec<ModelInfo>> {
    let mut c = synaptixd::Client::connect()?;
    let list = c.list()?;
    let metrics = c.metrics(false).unwrap_or_default();
    Ok(list
        .into_iter()
        .filter(|m| m.state == synaptixd::LoadState::Loaded)
        .map(|m| {
            let mut apps: Vec<String> = metrics.apps.iter().filter(|u| u.models.contains(&m.id)).map(|u| u.app.title()).collect();
            apps.dedup();
            let bytes = if m.device == Some(synaptixd::DeviceKind::Cuda) && m.vram_bytes > 0 { m.vram_bytes } else { m.mem_bytes };
            ModelInfo {
                family: kind_label(m.kind),
                component: m.title.clone(),
                label: synaptixd::precision_label(&m.precision),
                device: m.device.map(device_name).unwrap_or_default(),
                bytes,
                busy: m.busy,
                apps,
                id: m.id,
            }
        })
        .collect())
}

fn refresh() -> bool {
    let Ok(list) = fetch() else { return false };
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if *cache != list {
        *cache = list;
        drop(cache);
        bump();
    }
    true
}

/// Сигнал версии и фоновое обновление снимка. Вызывается один раз на
/// старте с главного потока: `use_signal` работает только там.
pub fn install() {
    if VERSION_SIGNAL.set(use_signal(0_u64)).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("synaptixd-memory".into())
        .spawn(|| loop {
            if refresh() {
                // События службы — сразу; программы и сессии — раз в 2 с.
                let (tx, rx) = std::sync::mpsc::channel::<()>();
                let sub = std::thread::spawn(move || {
                    let _ = synaptixd::Client::connect().and_then(|c| {
                        c.subscribe(|ev| {
                            if matches!(ev, synaptixd::Event::ModelsChanged | synaptixd::Event::ModelState { .. }) {
                                return tx.send(()).is_ok();
                            }
                            true
                        })
                    });
                });
                loop {
                    match rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            if !refresh() {
                                break;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let _ = sub.join();
            }
            std::thread::sleep(Duration::from_secs(5));
        })
        .ok();
}

/// Сигнал версии: `get()` внутри `Reactive` подписывает панель на
/// появление и выгрузку моделей.
pub fn version() -> Option<RwSignal<u64>> {
    VERSION_SIGNAL.get().copied()
}

/// Дёрнуть версию. Зовётся из фонового потока: `RwSignal::set` сам уезжает
/// на главный поток, а читать сигнал из чужого нельзя — новое значение берём
/// из атомика.
fn bump() {
    let v = VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(sig) = VERSION_SIGNAL.get() {
        sig.set(v);
    }
}

/// Загруженные модели службы (последний снимок).
pub fn list() -> Vec<ModelInfo> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Выгрузить модель в службе (это root и группа wheel).
pub fn unload(id: &str) -> std::result::Result<(), String> {
    let r = synaptixd::Client::connect().and_then(|mut c| c.unload(id)).map_err(|e| e.to_string());
    refresh();
    r
}

/// Выгрузить все модели службы; сколько выгружено.
pub fn unload_all() -> usize {
    let n = list().iter().filter(|m| unload(&m.id).is_ok()).count();
    refresh();
    n
}

/// «12.4 ГБ» / «834 МБ» / «—» для нулевого размера.
pub fn human_bytes(b: u64) -> String {
    if b == 0 {
        return tr!("models.size.unknown");
    }
    const KB: f64 = 1024.0;
    let f = b as f64;
    if f >= KB * KB * KB {
        tr!("models.size.gb", value = format!("{:.1}", f / (KB * KB * KB)))
    } else if f >= KB * KB {
        tr!("models.size.mb", value = format!("{:.0}", f / (KB * KB)))
    } else {
        tr!("models.size.kb", value = format!("{:.0}", f / KB))
    }
}
