//! Загрузка моделей KB: эмбеддера (BGE-M3) и cross-encoder реранкера.
//!
//! Модели тяжёлые (по ~2,3 ГБ на диске), поэтому грузятся лениво, по первой
//! надобности: кнопкой в настройках, первым ingest-job'ом, первым
//! `kb_search` или auto-augment'ом. Все пути сходятся в [`embedder_ready`] /
//! [`reranker_ready`] — один поток грузит, остальные ждут на замке и
//! получают тот же `Arc`.
//!
//! Файл модели ищет [`crate::kb::models`]: `.syn`-бандл в каталоге из
//! «AI модели», путь в `KbConfig` — лишь ручное переопределение.
//!
//! Состояние для UI — сигналы `KbCtx.{embedder,reranker}_state`; их пишет
//! только главный поток (`run_on_main_thread`).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::prelude::use_context;
use syngui::tr;
use syngui::widgets::feedback::NotificationCtx;
use synaptix::facade::embedding::{self as embeddings, Embedder, EmbedderConfig};
use synaptix::facade::rerank::{self as rerank, Reranker, RerankerConfig};

use crate::config::KbConfig;
use crate::context::AppCtx;
use crate::kb::ctx::{KbCtx, ModelState};
use crate::kb::models::{self, FoundModel, ModelKind, ModelPaths};

/// Всё, что нужно загрузке с рабочего потока. Собирается на главном:
/// каталоги поиска живут в сигналах.
#[derive(Debug, Clone)]
pub struct LoadPlan {
    pub cfg: KbConfig,
    pub dirs: Vec<PathBuf>,
}

/// Снимок конфига и каталогов поиска. Только с главного потока.
pub fn plan() -> LoadPlan {
    LoadPlan {
        cfg: crate::config::AppConfig::load().kb,
        dirs: search_dirs(),
    }
}

/// Где ищем модели: каталог из «AI модели», затем кэш HuggingFace и
/// закладки проводника бандлов. Только с главного потока.
pub fn search_dirs() -> Vec<PathBuf> {
    let app = use_context::<AppCtx>();
    let mut dirs = vec![crate::config::resolve_models_dir(&app.models_dir.get_untracked())];
    let hf = use_context::<crate::pages::huggingface::HuggingFaceCtx>();
    let mut extra = vec![crate::config::resolve_hf_cache_dir(&hf.cache_dir.get_untracked())];
    extra.extend(
        use_context::<crate::pages::syn_explorer::SynExplorerCtx>()
            .bookmarks
            .get_untracked(),
    );
    for dir in extra {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Перечитать, где лежат модели (открытие страницы, смена каталога, ручной
/// выбор файла). Диск читается на рабочем потоке.
pub fn refresh_paths(kb: KbCtx) {
    let plan = plan();
    spawn(async move {
        let found = tokio::task::spawn_blocking(move || models::discover(&plan.cfg, &plan.dirs))
            .await
            .unwrap_or_default();
        run_on_main_thread(move || {
            kb.model_paths.set(found);
            kb.model_paths_ready.set(true);
        });
    });
}

/// Кнопка «Загрузить»: обе модели в фоне, ошибки — в snackbar.
pub fn ensure_loaded(kb: KbCtx, notifications: NotificationCtx, plan: LoadPlan) {
    spawn(async move {
        if let Err(e) = embedder_ready(&kb, &plan).await {
            run_on_main_thread(move || notifications.error(e));
            return;
        }
        if let Err(e) = reranker_ready(&kb, &plan).await {
            run_on_main_thread(move || notifications.error(e));
        }
    });
}

/// Только реранкер (кнопка в его строке).
pub fn ensure_reranker_loaded(kb: KbCtx, notifications: NotificationCtx, plan: LoadPlan) {
    spawn(async move {
        if let Err(e) = reranker_ready(&kb, &plan).await {
            run_on_main_thread(move || notifications.error(e));
        }
    });
}

/// Эмбеддер, готовый к работе: уже загруженный либо загруженный сейчас.
/// Ошибка — готовый текст для пользователя/модели.
pub async fn embedder_ready(
    kb: &KbCtx,
    plan: &LoadPlan,
) -> Result<Arc<dyn Embedder + Send + Sync>, String> {
    if let Some(e) = kb.get_embedder() {
        return Ok(e);
    }
    let _guard = kb.embedder_load_lock.clone().lock_owned().await;
    // Пока ждали замок, модель мог загрузить другой поток.
    if let Some(e) = kb.get_embedder() {
        return Ok(e);
    }
    publish_state(kb, ModelKind::Embedder, ModelState::Loading);

    let plan2 = plan.clone();
    let result = tokio::task::spawn_blocking(move || {
        let found = locate(ModelKind::Embedder, &plan2)?;
        let mut cfg = EmbedderConfig::new(found.path.clone())
            .with_device(parse_device(&plan2.cfg.embedder_device))
            .with_dtype(parse_dtype(&plan2.cfg.embedder_dtype));
        cfg.batch_size = 16;
        let model = embeddings::load_embedder(cfg)
            .map_err(|e| tr!("kb.loader.embedder_load_error", error = e))?;
        Ok::<_, String>((found, model))
    })
    .await
    .map_err(|e| tr!("kb.loader.spawn_blocking_panic", error = e))
    .and_then(|r| r);

    match result {
        Ok((found, boxed)) => {
            let arc: Arc<dyn Embedder + Send + Sync> = Arc::from(boxed);
            kb.set_embedder(arc.clone());
            publish_found(kb, ModelKind::Embedder, found);
            publish_state(kb, ModelKind::Embedder, ModelState::Ready);
            Ok(arc)
        }
        Err(e) => {
            publish_state(kb, ModelKind::Embedder, ModelState::Failed(e.clone()));
            Err(e)
        }
    }
}

/// Реранкер, если он включён. `Ok(None)` — выключен в настройках; поиск
/// тогда идёт без rerank-фазы. Отсутствие файла — ошибка: пользователь
/// включил реранкер и должен узнать, что тот не работает.
pub async fn reranker_ready(
    kb: &KbCtx,
    plan: &LoadPlan,
) -> Result<Option<Arc<dyn Reranker + Send + Sync>>, String> {
    if !plan.cfg.reranker_enabled {
        return Ok(None);
    }
    if let Some(r) = kb.get_reranker() {
        return Ok(Some(r));
    }
    let _guard = kb.reranker_load_lock.clone().lock_owned().await;
    if let Some(r) = kb.get_reranker() {
        return Ok(Some(r));
    }
    publish_state(kb, ModelKind::Reranker, ModelState::Loading);

    let plan2 = plan.clone();
    let result = tokio::task::spawn_blocking(move || {
        let found = locate(ModelKind::Reranker, &plan2)?;
        let mut cfg = RerankerConfig::new(found.path.clone())
            .with_device(parse_device_rerank(&plan2.cfg.reranker_device))
            .with_dtype(parse_dtype_rerank(&plan2.cfg.reranker_dtype))
            .with_max_tokens(plan2.cfg.reranker_max_tokens.max(64));
        cfg.batch_size = 8;
        let model = rerank::load_reranker(cfg)
            .map_err(|e| tr!("kb.loader.reranker_load_error", error = e))?;
        Ok::<_, String>((found, model))
    })
    .await
    .map_err(|e| tr!("kb.loader.spawn_blocking_panic", error = e))
    .and_then(|r| r);

    match result {
        Ok((found, boxed)) => {
            let arc: Arc<dyn Reranker + Send + Sync> = Arc::from(boxed);
            kb.set_reranker(arc.clone());
            publish_found(kb, ModelKind::Reranker, found);
            publish_state(kb, ModelKind::Reranker, ModelState::Ready);
            Ok(Some(arc))
        }
        Err(e) => {
            publish_state(kb, ModelKind::Reranker, ModelState::Failed(e.clone()));
            Err(e)
        }
    }
}

/// Найти файл модели или объяснить, где его ждали.
fn locate(kind: ModelKind, plan: &LoadPlan) -> Result<FoundModel, String> {
    models::find(kind, &plan.cfg, &plan.dirs).ok_or_else(|| {
        let dir = plan
            .dirs
            .first()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let file = format!("{}.syn", kind.stem());
        let repo = kind.hf_repo();
        match kind {
            ModelKind::Embedder => {
                tr!("kb.loader.embedder_not_found", file = file, dir = dir, repo = repo)
            }
            ModelKind::Reranker => {
                tr!("kb.loader.reranker_not_found", file = file, dir = dir, repo = repo)
            }
        }
    })
}

fn publish_state(kb: &KbCtx, kind: ModelKind, state: ModelState) {
    let kb = kb.clone();
    run_on_main_thread(move || kb.model_state(kind).set(state));
}

fn publish_found(kb: &KbCtx, kind: ModelKind, found: FoundModel) {
    let kb = kb.clone();
    run_on_main_thread(move || {
        kb.model_paths.update(|paths: &mut ModelPaths| match kind {
            ModelKind::Embedder => paths.embedder = Some(found),
            ModelKind::Reranker => paths.reranker = Some(found),
        });
    });
}

/// Выгрузить модель и освободить память. Только с главного потока.
pub fn unload(kb: &KbCtx, kind: ModelKind) {
    match kind {
        ModelKind::Embedder => kb.drop_embedder(),
        ModelKind::Reranker => kb.drop_reranker(),
    }
    kb.model_state(kind).set(ModelState::Idle);
}

fn parse_device_rerank(s: &str) -> rerank::Device {
    match s {
        "cuda" => gpu_or_cpu_device(rerank::Device::Cuda(0), "kb reranker loader", "CUDA"),
        "metal" => gpu_or_cpu_device(rerank::Device::Metal(0), "kb reranker loader", "Metal"),
        _ => rerank::Device::Cpu,
    }
}

fn parse_dtype_rerank(s: &str) -> rerank::DType {
    match s {
        "f16" | "fp16" | "half" => rerank::DType::F16,
        "bf16" => rerank::DType::BF16,
        _ => rerank::DType::F32,
    }
}

fn parse_device(s: &str) -> embeddings::Device {
    match s {
        "cuda" => gpu_or_cpu_device(embeddings::Device::Cuda(0), "kb loader", "CUDA"),
        "metal" => gpu_or_cpu_device(embeddings::Device::Metal(0), "kb loader", "Metal"),
        _ => embeddings::Device::Cpu,
    }
}

fn gpu_or_cpu_device(gpu: embeddings::Device, who: &str, backend: &str) -> embeddings::Device {
    let _ = (who, backend);
    gpu
}

fn parse_dtype(s: &str) -> embeddings::DType {
    match s {
        "f16" | "fp16" | "half" => embeddings::DType::F16,
        "bf16" => embeddings::DType::BF16,
        _ => embeddings::DType::F32,
    }
}
