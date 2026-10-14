//! Модель чата — языковая модель службы synaptixd: веса, KV-кэш и
//! генерация живут там (`syn_chat::daemon`). Здесь — открытая служба
//! (соединение держит модель загруженной), токенизатор и шаблон бандла для
//! подсчёта токенов и сборки истории, сведения о модели.

use std::path::PathBuf;
use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::prelude::*;
use synaptix::facade::llm::{LlmTokenizer, SamplingProfile};

pub struct LoadedSynModel {
    /// Id модели в библиотеке службы.
    pub id: String,
    /// Под `Arc`, чтобы счётчики токенов (`tools::budget::model_counter`)
    /// держали только токенизатор.
    pub tokenizer: Arc<LlmTokenizer>,
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
        })
    }
}

/// Список языковых моделей службы и модель по умолчанию; обновляется по
/// событиям службы (подписка), без службы — повтор раз в 5 с.
fn watch_service(models: RwSignal<Vec<ServiceLlm>>, default_id: RwSignal<Option<String>>, online: RwSignal<bool>) {
    let fetch = move || -> synaptixd::Result<()> {
        let mut c = synaptixd::Client::connect()?;
        let list: Vec<ServiceLlm> = c.list()?.iter().filter_map(ServiceLlm::from_entry).collect();
        let def = c.defaults().ok().and_then(|d| d.into_iter().find(|d| d.kind == synaptixd::ModelKind::Llm)).and_then(|d| d.resolved.or(d.model));
        syngui::async_runtime::run_on_main_thread(move || {
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
                        if matches!(ev, synaptixd::Event::ModelsChanged | synaptixd::Event::ModelState { .. }) {
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
            // Открыть бандл — это прочитать его каталог чанков; на старте
            // приложения это не повод ждать.
            std::thread::spawn(move || {
                let profile = synaptix::facade::llm::sampling_profile(&path);
                sampling.set(Some(Arc::new(profile)));
            });
        }
        let (models, default_id, online) = (use_signal(Vec::new()), use_signal(None), use_signal(false));
        watch_service(models, default_id, online);
        Self {
            current: use_signal(None),
            loading: use_signal(false),
            error: use_signal(None),
            last_path: use_signal(last),
            sampling,
            models,
            default_id,
            online,
        }
    }

    /// Открыть модель в службе (её профиль — в настройках модели службы).
    pub fn load(&self, path: PathBuf) {
        self.load_inner(path, None);
    }

    /// Как [`Self::load`], но по завершении (успех или ошибка) сигналит в
    /// `notify`. Нужен агентскому `pipelines run` с free_vram: worker-поток
    /// после прогона синхронно ждёт, пока модель реально встанет обратно.
    pub fn load_with_notify(
        &self,
        path: PathBuf,
        notify: tokio::sync::oneshot::Sender<std::result::Result<Arc<LoadedSynModel>, String>>,
    ) {
        self.load_inner(path, Some(notify));
    }

    fn load_inner(
        &self,
        path: PathBuf,
        notify: Option<tokio::sync::oneshot::Sender<std::result::Result<Arc<LoadedSynModel>, String>>>,
    ) {
        if self.loading.get_untracked() {
            if let Some(tx) = notify {
                let _ = tx.send(Err(tr!("chat.model_registry.error.load_in_progress")));
            }
            return;
        }
        if let Some(loaded) = self.current.get_untracked() {
            if loaded.path == path {
                if let Some(tx) = notify {
                    let _ = tx.send(Ok(loaded));
                }
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
                    let loaded = Arc::new(loaded);
                    registry.current.set_always(Some(loaded.clone()));
                    if let Some(tx) = notify {
                        let _ = tx.send(Ok(loaded));
                    }
                }
                Err(e) => {
                    log::error!("[syn_chat] модель {}: {e}", path.display());
                    registry.error.set(Some(e.clone()));
                    if let Some(tx) = notify {
                        let _ = tx.send(Err(e));
                    }
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
    let tokenizer = synaptix::facade::llm::load_llm_tokenizer(path).map_err(|e| format!("токенизатор {}: {e}", path.display()))?;
    let sampling = synaptix::facade::llm::sampling_profile(path);
    Ok(LoadedSynModel {
        id,
        tokenizer: Arc::new(tokenizer),
        path: path.to_path_buf(),
        supports_media: info.media,
        sampling,
        window: info.window,
        _hold: Mutex::new(c),
    })
}

impl Default for SynModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
