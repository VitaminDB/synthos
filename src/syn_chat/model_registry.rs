//! Глобальный handle загруженной Qwen3.6-модели.
//!
//! Модель грузится один раз (5-15 секунд + ~12 GB VRAM), потом используется
//! всеми чатами. Загрузка спавнится в фоновый std::thread, состояние выставляется
//! через `RwSignal::set()` — он сам маршализуется в main thread.

use std::path::PathBuf;
use std::sync::Arc;

use synaptix_core::device::Device;
use syngui::prelude::*;
use synaptix::facade::llm::{load_llm_with_policy, Llm, LlmTokenizer, QuantPolicy};

pub struct LoadedSynModel {
    pub model: Llm,
    pub tokenizer: LlmTokenizer,
    pub path: PathBuf,
}

#[cfg(feature = "cuda")]
fn select_device() -> Device {
    Device::Cuda(0)
}

#[cfg(not(feature = "cuda"))]
fn select_device() -> Device {
    Device::Cpu
}

#[derive(Clone, Copy)]
pub struct SynModelRegistry {
    pub current: RwSignal<Option<Arc<LoadedSynModel>>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Был ли уже сделан lazy auto-load `AppConfig.last_syn_model` за сессию.
    /// Проверяется в `view()` страницы syn_chat; меняется на `true` после
    /// первой попытки (успех/ошибка — не важно).
    pub auto_load_attempted: RwSignal<bool>,
}

impl SynModelRegistry {
    pub fn new() -> Self {
        Self {
            current: use_signal(None),
            loading: use_signal(false),
            error: use_signal(None),
            auto_load_attempted: use_signal(false),
        }
    }

    /// Асинхронно загружает модель из `.syn`-bundle с указанной `policy`
    /// квантования. Идемпотентен: повторный вызов во время идущей загрузки
    /// игнорируется. По окончании `current` получает `Some(Arc<...>)` либо
    /// `error` — текст ошибки.
    ///
    /// `policy` обычно приходит из `AppCtx.syn_chat_quant.get_untracked().to_policy()`.
    /// Caller сам читает signal (signal-runtime thread-local, в spawn thread
    /// недоступен).
    pub fn load(&self, path: PathBuf, policy: QuantPolicy) {
        if self.loading.get_untracked() {
            return;
        }
        // Тот же путь уже загружен — нечего делать.
        if let Some(loaded) = self.current.get_untracked() {
            if loaded.path == path {
                return;
            }
        }

        let registry = *self;
        registry.loading.set(true);
        registry.error.set(None);

        std::thread::spawn(move || {
            let device = select_device();
            eprintln!(
                "[syn_chat] загрузка модели {:?} (device={:?}, preset={})",
                path, device, policy.preset_name
            );
            let t0 = std::time::Instant::now();

            match load_llm_with_policy(&path, policy, &device) {
                Ok((model, tokenizer)) => {
                    eprintln!(
                        "[syn_chat] модель загружена за {:?}, vocab={}",
                        t0.elapsed(),
                        model.vocab_size()
                    );
                    // Persist путь в config — для lazy auto-load в следующей сессии.
                    let mut cfg = crate::config::AppConfig::load();
                    let new_path_str = path.display().to_string();
                    if cfg.last_syn_model.as_deref() != Some(&new_path_str) {
                        cfg.last_syn_model = Some(new_path_str);
                        cfg.save();
                    }
                    let loaded = Arc::new(LoadedSynModel { model, tokenizer, path });
                    registry.current.set_always(Some(loaded));
                }
                Err(e) => {
                    eprintln!("[syn_chat] ошибка загрузки модели: {e:#}");
                    registry.error.set(Some(format!("{e:#}")));
                }
            }
            registry.loading.set(false);
        });
    }

    /// Выгружает текущую модель и стирает `last_syn_model` из конфига,
    /// чтобы при следующем запуске приложения auto-load не сработал.
    /// Сам `Arc<LoadedSynModel>` может оставаться живым во worker-thread
    /// до завершения генерации; вызывающий обязан вызвать `abort` перед
    /// `unload()`, если `pending == true` (см. right_panel.rs).
    pub fn unload(&self) {
        self.current.set_always(None);
        self.error.set(None);
        let mut cfg = crate::config::AppConfig::load();
        if cfg.last_syn_model.is_some() {
            cfg.last_syn_model = None;
            cfg.save();
        }
        // Сбрасываем guard — пользователь сможет выбрать новый .syn
        // без перезапуска приложения, и lazy-auto-load не сработает.
        self.auto_load_attempted.set_always(true);
    }
}

impl Default for SynModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Lazy auto-load `AppConfig.last_syn_model` при первом входе на /syn_chat.
///
/// Вызывается из `pages::syn_chat::view()`. Идемпотентен: после первой
/// попытки выставляет флаг и больше ничего не делает за сессию. Если модель
/// уже загружена другим путём (например, пользователь нажал «Выбрать .syn»)
/// — auto-load пропускается.
pub fn ensure_auto_load_last_model() {
    use syngui::prelude::use_context;
    let reg = use_context::<SynModelRegistry>();
    if reg.auto_load_attempted.get_untracked() {
        return;
    }
    reg.auto_load_attempted.set_always(true);
    if reg.current.get_untracked().is_some() || reg.loading.get_untracked() {
        return;
    }
    let cfg = crate::config::AppConfig::load();
    let Some(path_str) = cfg.last_syn_model.clone() else {
        return;
    };
    let path = PathBuf::from(path_str);
    if !path.exists() {
        eprintln!("[syn_chat] last_syn_model {:?} не существует — пропускаем", path);
        return;
    }
    let app_ctx = use_context::<crate::context::AppCtx>();
    let policy = app_ctx.syn_chat_quant.get_untracked().to_policy();
    reg.load(path, policy);
}
