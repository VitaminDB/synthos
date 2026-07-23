//! Высокоуровневый запуск ingest job'а из UI / команд.
//!
//! Связывает реактивный [`KbCtx`] с pipeline'ом из `kb::ingest`:
//! - Проверяет, что эмбеддер уже загружен (если нет — стартует загрузку).
//! - Загружает tokenizer.json модели, открывает [`Store`] коллекции.
//! - Спавнит async pipeline через `tokio::task::spawn_blocking` и
//!   публикует прогресс в `KbCtx.ingest_progress` через `run_on_main_thread`.
//! - Сбрасывает `cancel_flag`, чтобы предыдущая отмена не аффектила новый job.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::widgets::feedback::NotificationCtx;
use synaptix_rag::doc::ChunkConfig;

use crate::config::KbConfig;
use crate::kb::ctx::KbCtx;
use crate::kb::ingest::pipeline::{self, IngestJob, IngestStage};
use crate::kb::ingest::source::DocSource;
use crate::kb::store::Store;

static JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Стартовать ingest. Если эмбеддер ещё не загружен — выдаёт snackbar и
/// возвращает `false` (UI должен сначала загрузить модель).
pub fn start(
    kb: KbCtx,
    notifications: NotificationCtx,
    cfg: KbConfig,
    collection_id: String,
    sources: Vec<DocSource>,
) -> bool {
    let embedder = match kb.get_embedder() {
        Some(e) => e,
        None => {
            notifications.warning(
                "Эмбеддер не загружен. Сначала «Загрузить» в Settings → Базы знаний.",
            );
            return false;
        }
    };
    if sources.is_empty() {
        notifications.info("Нет источников для индексации.");
        return false;
    }

    let registry_snapshot = kb.registry.get_untracked();
    let collection_meta = match registry_snapshot.get(&collection_id) {
        Some(m) => m.clone(),
        None => {
            notifications.error("Коллекция не найдена.");
            return false;
        }
    };
    let db_path = collection_meta.db_path(&registry_snapshot.kb_dir);

    // Сбрасываем cancel-флаг под новый job.
    kb.ingest_cancel.store(false, Ordering::Relaxed);
    let cancel_flag = kb.ingest_cancel.clone();

    let job_id = JOB_ID.fetch_add(1, Ordering::Relaxed);
    let chunk_cfg = ChunkConfig {
        target_tokens: cfg.chunk_target_tokens,
        overlap_tokens: cfg.chunk_overlap_tokens,
        min_tokens: 32,
    };
    let embedding_dim = embedder.dim();
    let model_path = PathBuf::from(&cfg.embedder_model_path);

    let kb_for_progress = kb.clone();
    let notifications_for_progress = notifications.clone();

    spawn(async move {
        // Загружаем tokenizer (нужен chunker'у). Тот же tokenizer.json,
        // что использует эмбеддер — единый словарь.
        let tokenizer_path = model_path.join("tokenizer.json");
        let tokenizer = match tokenizers::Tokenizer::from_file(&tokenizer_path) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("ошибка tokenizer.json: {e}");
                let n = notifications_for_progress.clone();
                run_on_main_thread(move || {
                    n.error(msg);
                });
                return;
            }
        };

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("не удалось открыть БД коллекции: {e}");
                let n = notifications_for_progress.clone();
                run_on_main_thread(move || {
                    n.error(msg);
                });
                return;
            }
        };

        // URL fetcher — общий с web_read tool.
        let fetch_arc: Arc<
            dyn Fn(&str) -> Result<(String, String), String> + Send + Sync,
        > = Arc::new(|url: &str| -> Result<(String, String), String> {
            // pipeline вызывается из spawn_blocking, нужен sync-доступ к async fetch.
            // Запускаем через tokio runtime handle. Однако внутри spawn_blocking
            // у нас нет current handle; поэтому создаём локальный.
            let url = url.to_string();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let doc = runtime
                .block_on(async {
                    crate::agent::tools::web_fetch::fetch_url(&url).await
                })
                .map_err(|e| e.to_string())?;
            let title = crate::agent::tools::web_fetch::extract_title(&doc.body)
                .unwrap_or_else(|| url.clone());
            Ok((doc.body, title))
        });

        let job = IngestJob {
            job_id,
            collection_id: collection_id.clone(),
            sources,
            cancel_flag,
        };

        let kb_pp = kb_for_progress.clone();
        let notifications_pp = notifications_for_progress.clone();
        let on_progress = move |p: pipeline::IngestProgress| {
            let stage = p.stage;
            let total = p.total;
            let kb_inner = kb_pp.clone();
            let n_inner = notifications_pp.clone();
            run_on_main_thread(move || {
                kb_inner.ingest_progress.set(Some(p));
                if stage == IngestStage::Done {
                    n_inner.success(format!(
                        "Готово: проиндексировано {total} источников"
                    ));
                } else if stage == IngestStage::Cancelled {
                    n_inner.info("Индексация отменена");
                }
            });
        };

        let result = pipeline::run(
            job,
            store,
            embedder,
            chunk_cfg,
            tokenizer,
            embedding_dim,
            on_progress,
            Some(fetch_arc),
        )
        .await;

        let kb_done = kb_for_progress.clone();
        run_on_main_thread(move || {
            // Чистим прогресс через пару секунд — чтобы UI успел показать
            // финальное «Done» на ProgressBar'е.
            // Здесь — сразу очищаем, snackbar показывает результат.
            kb_done.ingest_progress.set(None);
            // Перечитываем counts в registry.
            kb_done.registry.update(|reg| reg.scan());
        });

        if let Err(e) = result {
            let n_err = notifications_for_progress.clone();
            run_on_main_thread(move || {
                n_err.error(format!("Ошибка ingest: {e}"));
            });
        }
    });
    true
}

/// Отменить активный job.
pub fn cancel(kb: &KbCtx) {
    kb.ingest_cancel.store(true, Ordering::Relaxed);
}
