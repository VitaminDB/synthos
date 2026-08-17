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

fn select_device() -> Device {
    Device::Cuda(0)
}

pub(crate) fn vram_free_mb() -> usize {
    synaptix_core::device::cuda::mem_info(0)
        .map(|(free, _total)| free / (1024 * 1024))
        .unwrap_or(0)
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
}

impl SynModelRegistry {
    pub fn new() -> Self {
        let last = crate::config::AppConfig::load()
            .last_syn_model
            .map(PathBuf::from)
            .filter(|p| p.exists());
        Self {
            current: use_signal(None),
            loading: use_signal(false),
            error: use_signal(None),
            last_path: use_signal(last),
        }
    }

    pub fn load(&self, path: PathBuf, policy: QuantPolicy) {
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
                    log::info!(
                        "[syn_chat] модель загружена за {:?}, vocab={}, max_seq_len={}; \
                         VRAM: веса {} MB, свободно {} MB",
                        t0.elapsed(),
                        model.vocab_size(),
                        model.config().max_seq_len,
                        vram_before.saturating_sub(vram_after),
                        vram_after
                    );
                    let mut cfg = crate::config::AppConfig::load();
                    let new_path_str = path.display().to_string();
                    if cfg.last_syn_model.as_deref() != Some(&new_path_str) {
                        cfg.last_syn_model = Some(new_path_str);
                        cfg.save();
                    }
                    registry.last_path.set_always(Some(path.clone()));
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

    pub fn unload(&self) {
        crate::syn_chat::attach::media_cache::clear();
        let held = self.current.get_untracked();
        let strong = held.as_ref().map(Arc::strong_count).unwrap_or(0);
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
        // Топ живых классов аллокаций — если после выгрузки пул всё ещё
        // держит сегменты, здесь видно, кто именно их пришпилил.
        let top: Vec<String> = synaptix_core::memory::cuda_pool::live_alloc_top(5)
            .into_iter()
            .map(|(bytes, count)| format!("{}x{}KB", count, bytes / 1024))
            .collect();
        log::info!(
            "[syn_chat] выгрузка модели: ссылок было {strong}, кэши ядер: {descs} TMA-деск. \
             + {} MB скретчей, trim +{freed} MB, VRAM свободно {before} -> {after} MB; \
             default-пул {dres}/{dused} MB, weights-пул {wres}/{wused} MB (reserved/used), \
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
