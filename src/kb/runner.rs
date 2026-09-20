//! Высокоуровневый запуск ingest job'а из UI / команд.
//!
//! Связывает реактивный [`KbCtx`] с pipeline'ом из `kb::ingest`:
//! - Берёт эмбеддер через `loader::embedder_ready` — не загружен, значит
//!   грузится здесь же; пользователь не обязан помнить про кнопку.
//! - Читает tokenizer.json модели (из бандла или каталога), открывает
//!   [`Store`] коллекции.
//! - Спавнит async pipeline через `tokio::task::spawn_blocking` и
//!   публикует прогресс в `KbCtx.ingest_progress` через `run_on_main_thread`.
//! - Сбрасывает `cancel_flag`, чтобы предыдущая отмена не аффектила новый job.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::widgets::feedback::NotificationCtx;
use syngui::{tr, trn};
use synaptix_rag::doc::ChunkConfig;

use crate::kb::ctx::KbCtx;
use crate::kb::ingest::pipeline::{self, IngestJob, IngestOutcome, IngestProgress, IngestStage};
use crate::kb::loader::{self, LoadPlan};
use crate::kb::models::{self, ModelKind};
use crate::kb::ingest::source::DocSource;
use crate::kb::store::Store;

static JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Стартовать ingest. `false` — job не запущен (нет источников, коллекции
/// или уже идёт другой). Только с главного потока.
pub fn start(
    kb: KbCtx,
    notifications: NotificationCtx,
    plan: LoadPlan,
    collection_id: String,
    sources: Vec<DocSource>,
) -> bool {
    if kb.ingest_progress.get_untracked().is_some() {
        notifications.warning(tr!("kb.runner.busy"));
        return false;
    }
    if sources.is_empty() {
        notifications.info(tr!("kb.runner.no_sources"));
        return false;
    }

    let registry_snapshot = kb.registry.get_untracked();
    let collection_meta = match registry_snapshot.get(&collection_id) {
        Some(m) => m.clone(),
        None => {
            notifications.error(tr!("kb.runner.collection_not_found"));
            return false;
        }
    };
    let db_path = collection_meta.db_path(&registry_snapshot.kb_dir);

    // Сбрасываем cancel-флаг под новый job.
    kb.ingest_cancel.store(false, Ordering::Relaxed);
    let cancel_flag = kb.ingest_cancel.clone();

    let job_id = JOB_ID.fetch_add(1, Ordering::Relaxed);
    let chunk_cfg = ChunkConfig {
        target_tokens: plan.cfg.chunk_target_tokens,
        overlap_tokens: plan.cfg.chunk_overlap_tokens,
        min_tokens: 32,
    };

    // Карточка прогресса появляется сразу, а не после загрузки модели.
    kb.ingest_progress.set(Some(IngestProgress {
        job_id,
        collection_id: collection_id.clone(),
        stage: IngestStage::Preparing,
        current: 0,
        total: 0,
        current_file: None,
        error: None,
    }));

    let kb_for_progress = kb.clone();
    let notifications_for_progress = notifications.clone();

    spawn(async move {
        // Любой ранний выход обязан снять карточку прогресса.
        let fail = |msg: String| {
            let kb = kb_for_progress.clone();
            let n = notifications_for_progress.clone();
            run_on_main_thread(move || {
                kb.ingest_progress.set(None);
                n.error(msg);
            });
        };

        let embedder = match loader::embedder_ready(&kb_for_progress, &plan).await {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let embedding_dim = embedder.dim();

        // Tokenizer нужен chunker'у — тот же tokenizer.json, что у эмбеддера.
        let plan_tok = plan.clone();
        let tokenizer = tokio::task::spawn_blocking(move || {
            let found = models::find(ModelKind::Embedder, &plan_tok.cfg, &plan_tok.dirs)
                .ok_or_else(|| "model file disappeared".to_string())?;
            let bytes = models::read_tokenizer_json(&found.path)?;
            tokenizers::Tokenizer::from_bytes(&bytes).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        let tokenizer = match tokenizer {
            Ok(t) => t,
            Err(e) => return fail(tr!("kb.runner.tokenizer_error", error = e)),
        };

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => return fail(tr!("kb.runner.store_open_error", error = e)),
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

        // Прогресс только рисует карточку: итог подводится по-настоящему
        // ниже, из `IngestOutcome`. Раньше здесь же говорилось «Готово:
        // проиндексировано N» по числу найденных файлов — даже если ни один
        // из них не дошёл до БД.
        let kb_pp = kb_for_progress.clone();
        let on_progress = move |p: pipeline::IngestProgress| {
            let kb_inner = kb_pp.clone();
            run_on_main_thread(move || kb_inner.ingest_progress.set(Some(p)));
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
            kb_done.documents_rev.update(|r| *r += 1);
        });

        let n_end = notifications_for_progress.clone();
        match result {
            Ok(outcome) => run_on_main_thread(move || report_outcome(&n_end, &outcome)),
            Err(e) => run_on_main_thread(move || {
                n_end.error(tr!("kb.runner.ingest_error", error = e));
            }),
        }
    });
    true
}

/// Сказать пользователю, чем кончилась индексация. Ошибки источников —
/// отдельным сообщением: молчание про них и оставляло пустые коллекции,
/// про которые никто не знал.
fn report_outcome(notifications: &NotificationCtx, outcome: &IngestOutcome) {
    if outcome.cancelled {
        notifications.info(tr!("kb.runner.ingest_cancelled"));
        return;
    }
    if outcome.indexed > 0 {
        notifications.success(tr!(
            "kb.runner.ingest_done",
            sources = outcome.indexed,
            chunks = outcome.chunks
        ));
    } else if outcome.skipped > 0 && outcome.failed.is_empty() {
        notifications.info(trn!("kb.runner.ingest_unchanged", outcome.skipped));
    }
    if let Some(first) = outcome.failed.first() {
        let text = if outcome.failed.len() == 1 {
            tr!(
                "kb.runner.ingest_failed_one",
                source = short_source(&first.source),
                error = first.error
            )
        } else {
            tr!(
                "kb.runner.ingest_failed_many",
                n = outcome.failed.len(),
                source = short_source(&first.source),
                error = first.error
            )
        };
        notifications.error(text);
    }
}

/// Имя файла вместо полного пути — в узкий snackbar путь не влезает.
fn short_source(source: &str) -> String {
    if source.starts_with("http://") || source.starts_with("https://") {
        return source.to_string();
    }
    source
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(source)
        .to_string()
}

/// Отменить активный job.
pub fn cancel(kb: &KbCtx) {
    kb.ingest_cancel.store(true, Ordering::Relaxed);
}
