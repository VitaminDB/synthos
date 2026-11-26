//! Модель чата — языковая модель службы synaptixd: веса, KV-кэш и
//! генерация живут там (`syn_chat::daemon`), токены считает её токенизатор.
//! Здесь — открытая служба (соединение держит модель загруженной) и сведения
//! о модели.

use std::path::PathBuf;
use std::sync::Arc;

use std::sync::Mutex;
use syngui::prelude::*;
use crate::syn_chat::params::SamplingProfile;

pub struct LoadedSynModel {
    /// Id модели в библиотеке службы.
    pub id: String,
    /// Канальный формат реплик (Muse Glimmer): история ассистента
    /// пересобирается ATEM-блоком (`channel_parser`).
    pub channel: bool,
    pub path: PathBuf,
    /// Модель принимает картинки и видео (у службы есть башня зрения).
    pub supports_media: bool,
    /// Пресеты сэмплинга и уровни размышлений модели — по ним режим
    /// `default` карточки Sampling собирает параметры хода.
    pub sampling: SamplingProfile,
    /// Окно контекста модели в службе, токенов.
    pub window: usize,
    /// Соединение, открывшее модель: пока оно живо, служба держит её
    /// загруженной (закрылось — модель выгружается по простою).
    _hold: Mutex<synaptixd::Client>,
}

impl LoadedSynModel {
    /// Счётчик токенов текста токенизатором модели в службе. Своё
    /// соединение, не открывшее модель: счётчик живёт в бюджете хода и не
    /// должен держать отпущенную модель загруженной.
    pub fn token_counter(&self) -> TokenCounter {
        TokenCounter { model: self.id.clone(), client: Arc::new(Mutex::new(None)) }
    }

    /// Токенов в тексте (`None` — служба не ответила).
    pub fn count_tokens(&self, text: &str) -> Option<usize> {
        self.token_counter().count(text)
    }
}

/// Подсчёт токенов в службе; соединение открывается при первом подсчёте.
#[derive(Clone)]
pub struct TokenCounter {
    model: String,
    client: Arc<Mutex<Option<synaptixd::Client>>>,
}

impl TokenCounter {
    pub fn count(&self, text: &str) -> Option<usize> {
        let mut guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = synaptixd::Client::connect().ok();
        }
        let r = guard.as_mut()?.count_tokens(&self.model, &[text.to_string()]);
        match r {
            Ok(v) => v.first().copied(),
            Err(e) => {
                log::warn!("[syn_chat] подсчёт токенов в службе: {e}");
                *guard = None;
                None
            }
        }
    }
}

/// Языковая модель из библиотеки службы — пункт списка выбора модели.
#[derive(Clone, PartialEq, Debug)]
pub struct ServiceLlm {
    pub id: String,
    pub title: String,
    /// Основной файл весов: по нему Synthos читает токенизатор и шаблон.
    pub path: PathBuf,
    pub loaded: bool,
    pub loading: bool,
    pub device: Option<synaptixd::DeviceKind>,
    pub precision: String,
    pub disk_bytes: u64,
    /// Действующий профиль движка (выверенный с правками).
    pub engine: Option<synaptixd::EngineProfile>,
    /// Ручные правки профиля в настройках модели.
    pub own: synaptixd::EngineProfile,
    /// Адаптеры LoRA модели по умолчанию (настройки службы).
    pub loras: Vec<synaptixd::LoraUse>,
    /// Архитектура (`qwen3`, `qwen3_5`, `gemma4`…).
    pub arch: String,
}

impl ServiceLlm {
    /// Учится ли адаптер LoRA на этой модели: Qwen3 и Qwen3.5/3.6/3.8 —
    /// плотные и MoE (Gemma, Qwen4Exp и прочие обучение synaptix пока не знает).
    pub fn trainable(&self) -> bool {
        matches!(self.arch.as_str(), "qwen3" | "qwen3_5" | "qwen3_5_moe")
    }

    /// MoE-модель: на токен работает малая доля весов.
    pub fn moe(&self) -> bool {
        self.arch.ends_with("_moe")
    }
}

impl ServiceLlm {
    fn from_entry(m: &synaptixd::ModelEntry) -> Option<Self> {
        (m.kind == synaptixd::ModelKind::Llm && !m.path.is_empty()).then(|| Self {
            id: m.id.clone(),
            title: m.title.clone(),
            path: PathBuf::from(&m.path),
            loaded: m.state == synaptixd::LoadState::Loaded,
            loading: m.state == synaptixd::LoadState::Loading,
            device: m.device,
            precision: m.precision.clone(),
            disk_bytes: m.disk_bytes,
            engine: m.engine.clone(),
            own: m.settings.engine.clone(),
            loras: m.settings.loras.clone(),
            arch: m.arch.clone(),
        })
    }
}

/// Список языковых моделей службы и модель по умолчанию; обновляется по
/// событиям службы (подписка), без службы — повтор раз в 5 с.
fn watch_service(models: RwSignal<Vec<ServiceLlm>>, default_id: RwSignal<Option<String>>, loras: RwSignal<Vec<synaptixd::LoraEntry>>, online: RwSignal<bool>) {
    let fetch = move || -> synaptixd::Result<()> {
        let mut c = synaptixd::Client::connect()?;
        let list: Vec<ServiceLlm> = c.list()?.iter().filter_map(ServiceLlm::from_entry).collect();
        let def = c.defaults().ok().and_then(|d| d.into_iter().find(|d| d.kind == synaptixd::ModelKind::Llm)).and_then(|d| d.resolved.or(d.model));
        let adapters = c.loras().unwrap_or_default();
        syngui::async_runtime::run_on_main_thread(move || {
            if loras.get_untracked() != adapters {
                loras.set(adapters);
            }
            if models.get_untracked() != list {
                models.set(list);
            }
            if default_id.get_untracked() != def {
                default_id.set(def);
            }
        });
        Ok(())
    };
    std::thread::Builder::new()
        .name("synaptixd-models".into())
        .spawn(move || loop {
            if fetch().is_ok() {
                syngui::async_runtime::run_on_main_thread(move || online.set(true));
                let sub = synaptixd::Client::connect().and_then(|c| {
                    c.subscribe(|ev| {
                        if matches!(ev, synaptixd::Event::ModelsChanged | synaptixd::Event::ModelState { .. } | synaptixd::Event::LorasChanged) {
                            let _ = fetch();
                        }
                        true
                    })
                });
                if let Err(e) = sub {
                    log::warn!("[syn_chat] подписка на службу нейросетей: {e}");
                }
            }
            syngui::async_runtime::run_on_main_thread(move || online.set(false));
            std::thread::sleep(std::time::Duration::from_secs(5));
        })
        .ok();
}

#[derive(Clone, Copy)]
pub struct SynModelRegistry {
    pub current: RwSignal<Option<Arc<LoadedSynModel>>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Последний загруженный бандл (`AppConfig.last_syn_model`). Модель сама
    /// НЕ поднимается — путь нужен кнопке «Загрузить модель», чтобы не гонять
    /// пользователя через файловый диалог на каждый запуск.
    pub last_path: RwSignal<Option<PathBuf>>,
    /// Пресеты сэмплинга для карточки Sampling: загруженной модели, а до
    /// загрузки — последней (`last_path`), чтобы режим `default` показывал
    /// значения модели сразу. Профиль читает только конфиги бандла, не веса.
    pub sampling: RwSignal<Option<Arc<SamplingProfile>>>,
    /// Языковые модели службы (список выбора).
    pub models: RwSignal<Vec<ServiceLlm>>,
    /// Модель чата по умолчанию в службе.
    pub default_id: RwSignal<Option<String>>,
    /// Адаптеры LoRA библиотек службы (обучает их программа «Модели»).
    pub loras: RwSignal<Vec<synaptixd::LoraEntry>>,
    /// Служба отвечает.
    pub online: RwSignal<bool>,
}

impl SynModelRegistry {
    /// Модель службы по файлу весов.
    pub fn service_model(&self, path: &std::path::Path) -> Option<ServiceLlm> {
        self.models.get().into_iter().find(|m| m.path == path)
    }

    pub fn new() -> Self {
        let last = crate::config::AppConfig::load()
            .last_syn_model
            .map(PathBuf::from)
            .filter(|p| p.exists());
        let sampling = use_signal(None);
        if let Some(path) = last.clone() {
            // Сведения служба отдаёт и о невыгруженной модели (по её файлам),
            // но это запрос — на старте приложения не повод ждать.
            std::thread::spawn(move || {
                if let Some(profile) = service_info(&path) {
                    sampling.set(Some(Arc::new(profile)));
                }
            });
        }
        let (models, default_id, loras, online) = (use_signal(Vec::new()), use_signal(None), use_signal(Vec::new()), use_signal(false));
        watch_service(models, default_id, loras, online);
        Self {
            current: use_signal(None),
            loading: use_signal(false),
            error: use_signal(None),
            last_path: use_signal(last),
            sampling,
            models,
            default_id,
            loras,
            online,
        }
    }

    /// Открыть модель в службе (её профиль — в настройках модели службы).
    pub fn load(&self, path: PathBuf) {
        if self.loading.get_untracked() {
            return;
        }
        if let Some(loaded) = self.current.get_untracked() {
            if loaded.path == path {
                return;
            }
        }

        let registry = *self;
        registry.loading.set(true);
        registry.error.set(None);

        std::thread::spawn(move || {
            log::info!("[syn_chat] модель {:?} — в службе", path);
            let t0 = std::time::Instant::now();
            match open_in_service(&path) {
                Ok(loaded) => {
                    log::info!(
                        "[syn_chat] модель {} открыта в службе за {:?}: окно {} ток., зрение {}",
                        loaded.id,
                        t0.elapsed(),
                        loaded.window,
                        loaded.supports_media
                    );
                    let new_path_str = path.display().to_string();
                    if crate::config::AppConfig::load().last_syn_model.as_deref() != Some(&new_path_str) {
                        crate::config::AppConfig::update(|cfg| cfg.last_syn_model = Some(new_path_str));
                    }
                    registry.last_path.set_always(Some(path.clone()));
                    registry.sampling.set(Some(Arc::new(loaded.sampling.clone())));
                    registry.current.set_always(Some(Arc::new(loaded)));
                }
                Err(e) => {
                    log::error!("[syn_chat] модель {}: {e}", path.display());
                    registry.error.set(Some(e));
                }
            }
            registry.loading.set(false);
        });
    }

    /// Отпустить модель: служба выгрузит её по простою или когда карта
    /// понадобится другой модели.
    pub fn unload(&self) {
        let held = self.current.get_untracked();
        if let Some(m) = held.as_ref() {
            log::info!("[syn_chat] модель {} отпущена (ссылок {})", m.id, Arc::strong_count(m));
        }
        drop(held);
        self.current.set_always(None);
        self.error.set(None);
    }
}

/// Открыть модель в службе по файлу бандла: загрузка, сведения, токенизатор.
fn open_in_service(path: &std::path::Path) -> std::result::Result<LoadedSynModel, String> {
    let svc = |e: synaptixd::Error| format!("служба нейросетей: {e}");
    let mut c = synaptixd::Client::connect().map_err(svc)?;
    let file = path.display().to_string();
    let opened = c.open(&file).map_err(svc)?;
    let id = opened.entry.id.clone();
    let info = c.llm_info(&id).map_err(svc)?;
    // Канальная модель — все служебные токены формата в её словаре по одному.
    let marks: Vec<String> = crate::syn_chat::channel_parser::CHANNEL_TOKENS.iter().map(|t| t.to_string()).collect();
    let channel = c.count_tokens(&id, &marks).map(|n| n.iter().all(|&n| n == 1)).unwrap_or(false);
    Ok(LoadedSynModel {
        id,
        channel,
        path: path.to_path_buf(),
        supports_media: info.media,
        window: info.window,
        sampling: info,
        _hold: Mutex::new(c),
    })
}

/// Пресеты сэмплинга и уровни размышлений модели по её файлу — из службы,
/// без загрузки модели.
fn service_info(path: &std::path::Path) -> Option<SamplingProfile> {
    let mut c = synaptixd::Client::connect().ok()?;
    let id = c.list().ok()?.into_iter().find(|m| std::path::Path::new(&m.path) == path).map(|m| m.id)?;
    c.llm_info(&id).ok()
}

impl Default for SynModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
