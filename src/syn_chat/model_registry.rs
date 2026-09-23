use std::path::PathBuf;
use std::sync::Arc;

use synaptix_core::device::Device;
use syngui::prelude::*;
use synaptix::facade::llm::{load_llm_with_policy, Llm, LlmTokenizer, QuantPolicy, SamplingProfile};

pub struct LoadedSynModel {
    pub model: Llm,
    /// Под `Arc`, чтобы счётчики токенов (`tools::budget::model_counter`)
    /// держали только токенизатор, а не модель: клон `Arc<LoadedSynModel>`
    /// в бюджете хода переживал выгрузку перед `pipelines run` — веса
    /// оставались на карте, и нодовая модель прогона падала в OOM.
    pub tokenizer: Arc<LlmTokenizer>,
    pub path: PathBuf,
    /// Кэш `Llm::supports_media()`, снятый один раз при загрузке.
    ///
    /// Спрашивать сам пайплайн из UI нельзя: `supports_media` берёт тот же
    /// `Mutex`, что и `generate_streaming`, а генерация держит его на весь
    /// ход. Реактивный блок статуса модели упирался в этот мьютекс прямо в
    /// `rebuild_if_needed` и вешал main thread на всё время генерации — окно
    /// переставало перерисовываться, композитор помечал его «не отвечает».
    /// Для загруженного бандла флаг неизменен, поэтому кэш честный.
    pub supports_media: bool,
    /// Пресеты сэмплинга и уровни размышлений модели — по ним режим
    /// `default` карточки Sampling собирает параметры хода.
    pub sampling: SamplingProfile,
}

fn select_device() -> Device {
    Device::Cuda(0)
}

pub fn vram_free_mb() -> usize {
    synaptix_core::device::cuda::mem_info(0)
        .map(|(free, _total)| free / (1024 * 1024))
        .unwrap_or(0)
}

/// Сколько VRAM реально доступно под новые аллокации: свободное по драйверу
/// плюс свободные блоки, которые уже держит пул АКТИВАЦИЙ.
///
/// `cuMemGetInfo` их не показывает, а пул — переиспользует: после генерации
/// его `reserved` вырастает на размер KV-ринга и обратно не падает (трим не
/// возвращает сегменты, в которых остались живые блоки), но следующий ринг
/// садится ровно в эти же блоки, не занимая у драйвера ни байта. Считать по
/// «свободно» значит после первого же ответа урезать себе контекст втрое на
/// ровном месте: бюджет ринга схлопывается, `max_new_tokens` обрезается до
/// сотни токенов, и ответ рвётся на полуслове.
///
/// Ринг, кэш префикс-KV и активации живут в ОТДЕЛЬНОМ пуле (см.
/// `synaptix_core::device::cuda::activations_pool`), поэтому его слабина —
/// честная часть бюджета: она уйдёт под следующий ринг тех же размеров.
/// Слабину default-пула (веса) и staging-пула (загрузка) НЕ учитываем:
/// раньше учитывали в расчёте «трим на OOM вернёт её драйверу», но на деле
/// их сегменты прошиты живыми блоками весов и трим возвращает крохи
/// (наблюдали +32 MB при 3.4 GB насчитанной слабины) — бюджет выходил
/// фантомным, планировщик раздувал кэш префикс-KV до размеров, при которых
/// активациям forward'а не оставалось ничего, и ход падал в OOM на
/// аллокации в мегабайты.
/// К свободной памяти прибавляем и ту, что держат ОТДАВАЕМЫЕ кэши: кэш
/// экспертов MoE перечитывается из бандла, и уступить гигабайт ему стоит
/// миллисекунды подкачки — против секунд полного префилла, которыми платит
/// урезанный контекст. Без этой добавки планировщик видел «по VRAM влезает
/// 0 ток» рядом с десятью гигабайтами кэша экспертов и выдавал сессию
/// префикс-KV впритык под текущий промпт; следующий ход её перерастал, и
/// кэш пересоздавался вместо переиспользования.
pub fn vram_available_mb() -> usize {
    vram_free_mb() + act_pool_slack_mb() + reclaimable_mb()
}

/// Сколько VRAM отдадут кэши, если попросить (кэш экспертов MoE и пустые
/// slab'ы его арены).
pub fn reclaimable_mb() -> usize {
    synaptix::facade::llm::cuda_reclaimable_mb(0)
}

/// Слабина пула активаций (reserved − used), МБ: свободные блоки, которые
/// пул отдаст под следующий KV-ринг/сессию, не занимая память у драйвера.
pub fn act_pool_slack_mb() -> usize {
    synaptix_core::device::cuda::activations_pool_stats(0)
        .map(|(reserved, used)| reserved.saturating_sub(used) as usize / (1024 * 1024))
        .unwrap_or(0)
}

/// Возвращает ОС всё, что держат кэши ядер и пул аллокатора. Отдаёт
/// `(сколько MB освободилось, сколько записей выкинуто из кэша)`.
///
/// Порядок важен: TMA-дескрипторы кэшируются по АДРЕСУ тензора, поэтому
/// после каждой генерации в кэше оседают мёртвые записи на адреса KV-ринга
/// и активаций. Сами по себе они крошечные (128 Б), но живыми аллокациями
/// рассыпаны по сегментам mempool'а и не дают триму вернуть драйверу
/// зарезервированное — за пару ходов на 24 ГБ так утекает больше гигабайта,
/// ровно той VRAM, которой потом не хватает vision-башне. Чистим ДО трима;
/// дескрипторы восстанавливаются лениво на первом же вызове ядра.
pub fn reclaim_vram(device: Device) -> (u64, usize) {
    let Device::Cuda(ordinal) = device else {
        return (0, 0);
    };
    let (descs, _scratch) = synaptix::facade::llm::cuda_release_kernel_caches();
    let freed = synaptix::facade::llm::cuda_trim_pool(ordinal as i32);
    (freed, descs)
}

fn ensure_kernels_registered() {
    use std::sync::OnceLock;
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
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
}

impl SynModelRegistry {
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
        Self {
            current: use_signal(None),
            loading: use_signal(false),
            error: use_signal(None),
            last_path: use_signal(last),
            sampling,
        }
    }

    pub fn load(&self, path: PathBuf, policy: QuantPolicy) {
        self.load_inner(path, policy, None);
    }

    /// Как [`Self::load`], но по завершении (успех или ошибка) сигналит в
    /// `notify`. Нужен агентскому `pipelines run` с free_vram: worker-поток
    /// после прогона синхронно ждёт, пока модель реально встанет обратно.
    pub fn load_with_notify(
        &self,
        path: PathBuf,
        policy: QuantPolicy,
        notify: tokio::sync::oneshot::Sender<std::result::Result<Arc<LoadedSynModel>, String>>,
    ) {
        self.load_inner(path, policy, Some(notify));
    }

    fn load_inner(
        &self,
        path: PathBuf,
        policy: QuantPolicy,
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
        crate::syn_chat::session::reset_kernel_cache_warm();
        crate::syn_chat::session::drop_kv_session();
        registry.loading.set(true);
        registry.error.set(None);
        // Эмбеддинги вложений привязаны к vision-башне прежней модели —
        // при смене модели они больше не валидны (и держат её VRAM).
        crate::syn_chat::attach::media_cache::clear();

        std::thread::spawn(move || {
            ensure_kernels_registered();
            let device = select_device();
            let vram_before = vram_free_mb();
            log::info!(
                "[syn_chat] загрузка модели {:?} (device={:?}, preset={}, VRAM свободно {} MB)",
                path, device, policy.preset_name, vram_before
            );
            let t0 = std::time::Instant::now();

            match load_llm_with_policy(&path, policy, &device) {
                Ok((model, tokenizer)) => {
                    if let Device::Cuda(ord) = device {
                        let freed = synaptix::facade::llm::cuda_trim_pool(ord as i32);
                        log::info!("[syn_chat] trim после загрузки: +{freed} MB");
                    }
                    let vram_after = vram_free_mb();
                    // Потолок контекста по памяти — то, что реально ограничивает
                    // чат: KV-ринг живёт один ход и садится в свободную VRAM.
                    // Запас (KV_RESERVE_MB в session.rs) вычитаем, иначе цифра
                    // обманывает на размер активаций префилла.
                    let kv_per_token = model.kv_bytes_per_token();
                    // Ring-окна sliding-слоёв не растут с контекстом, но VRAM
                    // держат — вычитаем их до деления на ставку «на токен».
                    let kv_fixed_mb =
                        model.kv_fixed_bytes(model.config().max_seq_len) / (1024 * 1024);
                    let ctx_ceiling = if kv_per_token > 0 {
                        (vram_available_mb().saturating_sub(1280 + kv_fixed_mb) * 1024 * 1024)
                            / kv_per_token
                    } else {
                        model.config().max_seq_len
                    };
                    log::info!(
                        "[syn_chat] модель загружена за {:?}, vocab={}, max_seq_len={}; \
                         VRAM: веса {} MB, свободно {} MB; KV {} B/ток → контекст по \
                         памяти ≈{} ток после прогрева кэшей ядер (cap модели {})",
                        t0.elapsed(),
                        model.vocab_size(),
                        model.config().max_seq_len,
                        vram_before.saturating_sub(vram_after),
                        vram_after,
                        kv_per_token,
                        ctx_ceiling,
                        model.config().max_seq_len
                    );
                    let new_path_str = path.display().to_string();
                    if crate::config::AppConfig::load().last_syn_model.as_deref()
                        != Some(&new_path_str)
                    {
                        crate::config::AppConfig::update(|cfg| {
                            cfg.last_syn_model = Some(new_path_str)
                        });
                    }
                    registry.last_path.set_always(Some(path.clone()));
                    let supports_media = model.supports_media();
                    let sampling = synaptix::facade::llm::sampling_profile(&path);
                    registry.sampling.set(Some(Arc::new(sampling.clone())));
                    let loaded = Arc::new(LoadedSynModel {
                        model,
                        tokenizer: Arc::new(tokenizer),
                        path,
                        supports_media,
                        sampling,
                    });
                    registry.current.set_always(Some(loaded.clone()));
                    if let Some(tx) = notify {
                        let _ = tx.send(Ok(loaded));
                    }
                }
                Err(e) => {
                    eprintln!("[syn_chat] ошибка загрузки модели: {e:#}");
                    registry.error.set(Some(format!("{e:#}")));
                    if let Some(tx) = notify {
                        let _ = tx.send(Err(format!("{e:#}")));
                    }
                }
            }
            registry.loading.set(false);
        });
    }

    pub fn unload(&self) {
        crate::syn_chat::session::reset_kernel_cache_warm();
        crate::syn_chat::session::drop_kv_session();
        crate::syn_chat::attach::media_cache::clear();
        let held = self.current.get_untracked();
        let strong = held.as_ref().map(Arc::strong_count).unwrap_or(0);
        // Кэши на карте (у Qwen4Exp — резидентные эксперты, гигабайты) отдаём
        // явно: `Drop` модели может задержаться, пока жива хоть одна ссылка,
        // а видеопамять нужна следующей модели сразу.
        if let Some(model) = held.as_ref() {
            model.model.release_device_caches();
        }
        drop(held);
        self.current.set_always(None);
        self.error.set(None);
        let before = vram_free_mb();
        // TMA-дескрипторы кэшируются по АДРЕСУ тензора, MXFP8-скретчи — по
        // устройству: после Drop модели записи мертвы, но живы как аллокации и
        // рассыпаны по сегментам mempool'а — trim возвращал драйверу не всё
        // (reserved 4768 MB при used 51 MB). Чистим ДО трима.
        let (descs, scratch) = synaptix::facade::llm::cuda_release_kernel_caches();
        let freed = synaptix::facade::llm::cuda_trim_pool(0);
        let after = vram_free_mb();
        let mb = |(r, u): (u64, u64)| (r / (1024 * 1024), u / (1024 * 1024));
        let (dres, dused) = synaptix_core::memory::cuda_pool::cuda_mempool_stats(0)
            .map(mb)
            .unwrap_or((0, 0));
        let (wres, wused) = synaptix_core::device::cuda::weights_pool_stats(0)
            .map(mb)
            .unwrap_or((0, 0));
        let (ares, aused) = synaptix_core::device::cuda::activations_pool_stats(0)
            .map(mb)
            .unwrap_or((0, 0));
        // Топ живых классов аллокаций — если после выгрузки пул всё ещё
        // держит сегменты, здесь видно, кто именно их пришпилил.
        let top: Vec<String> = synaptix_core::memory::cuda_pool::live_alloc_top(5)
            .into_iter()
            .map(|(bytes, count)| format!("{}x{}KB", count, bytes / 1024))
            .collect();
        log::info!(
            "[syn_chat] выгрузка модели: ссылок было {strong}, кэши ядер: {descs} TMA-деск. \
             + {} MB скретчей, trim +{freed} MB, VRAM свободно {before} -> {after} MB; \
             default-пул {dres}/{dused} MB, staging-пул {wres}/{wused} MB, \
             пул активаций {ares}/{aused} MB (reserved/used), \
             живых по нашему учёту {:.0} MB, топ: {}",
            scratch / (1024 * 1024),
            synaptix_core::memory::cuda_pool::cuda_allocated_mb(),
            top.join(", ")
        );
    }
}

impl Default for SynModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
