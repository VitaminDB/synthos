//! Lazy-загрузка эмбеддера для KB.
//!
//! Эмбеддер — тяжёлая модель (BGE-M3 ≈ 2GB на диске, ~4GB в памяти на CPU).
//! Грузим лениво:
//! - При первом ingest-job'е — автоматически.
//! - При первом kb_search — НЕ автоматически (ответ пользователю «не загружен,
//!   откройте Settings»). Это сделано намеренно: модель грузится секунды-
//!   минуты, а tool-call синхронный — лучше дать UI-feedback.
//!
//! Загрузка идёт в `tokio::task::spawn_blocking`, успех/ошибка
//! материализуются в `KbCtx.embedder` и (если задан) в snackbar.

use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::widgets::feedback::NotificationCtx;
use synaptix::facade::embedding::{self as embeddings, Embedder, EmbedderConfig};
use synaptix::facade::rerank::{self as rerank, Reranker, RerankerConfig};

use crate::config::KbConfig;
use crate::kb::ctx::KbCtx;

/// Стартует загрузку эмбеддера. Если уже загружен — no-op.
pub fn ensure_loaded(kb: KbCtx, notifications: NotificationCtx, cfg: KbConfig) {
    if kb.get_embedder().is_some() {
        return;
    }
    let model_path = PathBuf::from(&cfg.embedder_model_path);
    if !model_path.is_dir() {
        notifications.error(format!(
            "Эмбеддер не найден: {}. Скачай BGE-M3 (BAAI/bge-m3) в этот каталог.",
            model_path.display()
        ));
        return;
    }
    notifications.info("Загружаю эмбеддер... (может занять минуту)");

    spawn(async move {
        let device = parse_device(&cfg.embedder_device);
        let dtype = parse_dtype(&cfg.embedder_dtype);
        let mut emb_cfg = EmbedderConfig::new(model_path)
            .with_device(device)
            .with_dtype(dtype);
        emb_cfg.batch_size = 16;
        let result = tokio::task::spawn_blocking(move || embeddings::load_embedder(emb_cfg))
            .await
            .map_err(|e| format!("spawn_blocking panic: {e}"))
            .and_then(|r| r.map_err(|e| e.to_string()));

        match result {
            Ok(boxed) => {
                let arc: Arc<dyn Embedder + Send + Sync> = Arc::from(boxed);
                let dim = arc.dim();
                let kb2 = kb.clone();
                let n2 = notifications.clone();
                run_on_main_thread(move || {
                    kb2.set_embedder(arc);
                    n2.success(format!("Эмбеддер загружен (dim={dim})"));
                });
            }
            Err(e) => {
                let n2 = notifications.clone();
                run_on_main_thread(move || {
                    n2.error(format!("Ошибка загрузки эмбеддера: {e}"));
                });
            }
        }
    });
}

/// Выгружает эмбеддер (UI-кнопка «Освободить память»).
pub fn unload(kb: &KbCtx, notifications: &NotificationCtx) {
    kb.drop_embedder();
    notifications.success("Эмбеддер выгружен.");
}

/// Стартует загрузку cross-encoder реранкера. Если уже загружен или
/// `reranker_enabled=false` / путь пуст — no-op.
///
/// Архитектурно симметрично `ensure_loaded` (embedder), но независимо
/// от него: пользователь может включить только reranker (для tool
/// `kb_search` поверх RRF) или только embedder.
pub fn ensure_reranker_loaded(kb: KbCtx, notifications: NotificationCtx, cfg: KbConfig) {
    if !cfg.reranker_enabled {
        return;
    }
    if kb.get_reranker().is_some() {
        return;
    }
    if cfg.reranker_model_path.trim().is_empty() {
        // Не настроен — молча. UI должен подсветить в Settings.
        return;
    }
    let model_path = PathBuf::from(&cfg.reranker_model_path);
    if !model_path.is_dir() {
        notifications.error(format!(
            "Реранкер не найден: {}. Скачай BAAI/bge-reranker-v2-m3 в этот каталог.",
            model_path.display()
        ));
        return;
    }
    notifications.info("Загружаю реранкер... (около 568 MB)");

    spawn(async move {
        let device = parse_device_rerank(&cfg.reranker_device);
        let dtype = parse_dtype_rerank(&cfg.reranker_dtype);
        let mut r_cfg = RerankerConfig::new(model_path)
            .with_device(device)
            .with_dtype(dtype)
            .with_max_tokens(cfg.reranker_max_tokens.max(64));
        r_cfg.batch_size = 8;
        let result = tokio::task::spawn_blocking(move || rerank::load_reranker(r_cfg))
            .await
            .map_err(|e| format!("spawn_blocking panic: {e}"))
            .and_then(|r| r.map_err(|e| e.to_string()));

        match result {
            Ok(boxed) => {
                let arc: Arc<dyn Reranker + Send + Sync> = Arc::from(boxed);
                let max = arc.max_tokens();
                let kb2 = kb.clone();
                let n2 = notifications.clone();
                run_on_main_thread(move || {
                    kb2.set_reranker(arc);
                    n2.success(format!("Реранкер загружен (max_tokens={max})"));
                });
            }
            Err(e) => {
                let n2 = notifications.clone();
                run_on_main_thread(move || {
                    n2.error(format!("Ошибка загрузки реранкера: {e}"));
                });
            }
        }
    });
}

/// Выгрузить реранкер. UI-кнопка «Освободить память» для cross-encoder'а.
pub fn unload_reranker(kb: &KbCtx, notifications: &NotificationCtx) {
    kb.drop_reranker();
    notifications.success("Реранкер выгружен.");
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
