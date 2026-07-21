//! Конвейер ingestion: DocSource → parsed → chunks → embeddings → store.
//!
//! Текущая реализация — синхронный `run_blocking` (вызывается из
//! `tokio::task::spawn_blocking` в обёртке `run`). Прогресс публикуется
//! через коллбек `report_progress`, чтобы не было жёсткой связи с
//! syngui::async_runtime в этом модуле.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use synaptix_rag::doc::{chunk, parse, ChunkConfig, ParsedDoc, SourceKind};
use synaptix::facade::embedding::Embedder;

use super::source::DocSource;
use crate::kb::store::{ChunkRow, DocumentRow, Store};

/// Высокоуровневый стейдж — для UI ProgressBar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestStage {
    Discovering,
    Parsing,
    Chunking,
    Embedding,
    Storing,
    Done,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngestProgress {
    pub job_id: u64,
    pub collection_id: String,
    pub stage: IngestStage,
    pub current: usize,
    pub total: usize,
    pub current_file: Option<String>,
    pub error: Option<String>,
}

/// Описание задачи. `cancel_flag` — кооперативный stop-токен;
/// caller выставляет в `true` для отмены.
pub struct IngestJob {
    pub job_id: u64,
    pub collection_id: String,
    pub sources: Vec<DocSource>,
    pub cancel_flag: Arc<AtomicBool>,
}

/// Реализация ingest'а как чистая функция: на вход — job, store, embedder,
/// колбек прогресса. Никаких глобальных синглтонов — облегчает тестирование
/// и переиспользование (одну функцию можно вызвать из CLI, тестов, UI).
///
/// `report` вызывается на каждом значимом шаге. Для главного UI он
/// должен вызывать `run_on_main_thread` + `signal.set(...)`.
pub fn run_blocking<F: FnMut(IngestProgress)>(
    job: &IngestJob,
    store: &mut Store,
    embedder: &dyn Embedder,
    chunk_cfg: &ChunkConfig,
    tokenizer: &tokenizers::Tokenizer,
    embedding_dim: usize,
    mut report: F,
    fetch_url: Option<&dyn Fn(&str) -> Result<(String, String), String>>,
) -> Result<(), IngestError> {
    let cancel_flag = job.cancel_flag.clone();
    let mut emit = |stage: IngestStage,
                    current: usize,
                    total: usize,
                    current_file: Option<String>,
                    error: Option<String>| {
        report(IngestProgress {
            job_id: job.job_id,
            collection_id: job.collection_id.clone(),
            stage,
            current,
            total,
            current_file,
            error,
        });
    };

    // 1. Discovery — раскручиваем все sources в плоский список (path/url, kind, bytes_loader).
    emit(IngestStage::Discovering, 0, 0, None, None);

    let mut tasks: Vec<DiscoveredTask> = Vec::new();
    for src in &job.sources {
        match src {
            DocSource::File(p) => {
                if let Some(kind) = SourceKind::from_path(p) {
                    tasks.push(DiscoveredTask::File {
                        path: p.clone(),
                        kind,
                    });
                }
            }
            DocSource::Pdf(p) => tasks.push(DiscoveredTask::File {
                path: p.clone(),
                kind: SourceKind::Pdf,
            }),
            DocSource::Folder {
                root,
                extra_excludes,
            } => {
                // ai/kb feature всегда тянет doc-processing-folder, поэтому
                // здесь нет cfg-gate. Если в будущем synthos соберут без kb —
                // эта строчка не скомпилируется, что и нужно.
                use synaptix_rag::doc::ignore::{walk, WalkConfig};
                let mut cfg = WalkConfig::new(root);
                cfg.extra_excludes = extra_excludes.clone();
                for f in walk(&cfg) {
                    tasks.push(DiscoveredTask::File {
                        path: f.path,
                        kind: f.kind,
                    });
                }
            }
            DocSource::Url(u) => tasks.push(DiscoveredTask::Url(u.clone())),
        }
    }
    let total = tasks.len();
    if total == 0 {
        emit(IngestStage::Done, 0, 0, None, None);
        return Ok(());
    }

    // 2. Per-task: read → parse → chunk → embed → store.
    for (idx, task) in tasks.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            emit(IngestStage::Cancelled, idx, total, None, None);
            return Ok(());
        }

        let task_label = task.display();

        // --- Parsing ---
        emit(
            IngestStage::Parsing,
            idx,
            total,
            Some(task_label.clone()),
            None,
        );
        let (bytes, source_kind, source_kind_str, source_path, doc_title): (
            Vec<u8>,
            SourceKind,
            String,
            String,
            Option<String>,
        ) = match task {
            DiscoveredTask::File { path, kind } => {
                let bs = match std::fs::read(path) {
                    Ok(b) => b,
                    Err(e) => {
                        log::warn!("kb ingest: read {}: {e}", path.display());
                        continue;
                    }
                };
                let title = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());
                (
                    bs,
                    *kind,
                    match kind {
                        SourceKind::Pdf => "pdf".into(),
                        _ => "file".into(),
                    },
                    path.display().to_string(),
                    title,
                )
            }
            DiscoveredTask::Url(u) => {
                let fetch = match fetch_url {
                    Some(f) => f,
                    None => {
                        log::warn!("kb ingest: URL '{u}' пропущен — fetch_url не задан");
                        continue;
                    }
                };
                match fetch(u) {
                    Ok((md, title)) => (
                        md.into_bytes(),
                        SourceKind::Markdown,
                        "url".into(),
                        u.clone(),
                        Some(title),
                    ),
                    Err(e) => {
                        log::warn!("kb ingest: fetch {u}: {e}");
                        continue;
                    }
                }
            }
        };

        let parsed = match parse(&bytes, source_kind) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("kb ingest: parse {task_label}: {e}");
                continue;
            }
        };
        if parsed.plain_text.is_empty() {
            continue;
        }
        // ParsedDoc::title (h1 для md, <title> для html) превалирует над именем файла.
        let final_title = parsed.title.clone().or(doc_title);

        // sha256 от исходных bytes — стабильнее, чем от parsed.plain_text
        // (markdown / html форматирование может менять чанк, но если bytes
        // те же — ингест уже актуальный).
        let sha256 = sha256_hex(&bytes);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let doc_row = DocumentRow {
            id: None,
            source_kind: source_kind_str,
            source_path,
            sha256,
            title: final_title,
            bytes: bytes.len() as i64,
            indexed_at: now,
        };

        // --- Chunking ---
        emit(
            IngestStage::Chunking,
            idx,
            total,
            Some(task_label.clone()),
            None,
        );
        let chunks = match chunk(&parsed, tokenizer, chunk_cfg) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("kb ingest: chunk {task_label}: {e}");
                continue;
            }
        };
        if chunks.is_empty() {
            continue;
        }

        // upsert_document: если sha не изменился, fresh=false → пропустим embed.
        let (doc_id, fresh) = match store.upsert_document(&doc_row) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("kb ingest: upsert_document: {e}");
                continue;
            }
        };
        if !fresh {
            // Документ не изменился — пропускаем embed/store, идём дальше.
            continue;
        }

        // --- Embedding ---
        emit(
            IngestStage::Embedding,
            idx,
            total,
            Some(task_label.clone()),
            None,
        );
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let vectors = match embedder.encode(&texts) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("kb ingest: embed {task_label}: {e}");
                continue;
            }
        };
        if vectors.len() != chunks.len() {
            log::warn!(
                "kb ingest: embedder вернул {} векторов на {} чанков — пропускаю",
                vectors.len(),
                chunks.len()
            );
            continue;
        }

        // --- Storing ---
        emit(
            IngestStage::Storing,
            idx,
            total,
            Some(task_label.clone()),
            None,
        );
        let chunk_rows: Vec<ChunkRow> = chunks
            .into_iter()
            .zip(vectors)
            .enumerate()
            .map(|(ord, (c, v))| ChunkRow {
                ord: ord as i32,
                text: c.text,
                start_byte: c.start_byte,
                end_byte: c.end_byte,
                token_count: c.token_count,
                embedding: v,
            })
            .collect();
        if let Err(e) = store.insert_chunks(doc_id, &chunk_rows, embedding_dim) {
            log::warn!("kb ingest: insert_chunks {task_label}: {e}");
            continue;
        }

        let _ = parsed; // drop heavy buffers to keep peak memory low
    }

    emit(IngestStage::Done, total, total, None, None);
    Ok(())
}

/// Удобный async-launcher, который вызывает `run_blocking` в spawn_blocking.
/// Используется UI-кодом (Settings → Базы знаний → «Index»).
pub async fn run<F>(
    job: IngestJob,
    store: Store,
    embedder: Arc<dyn Embedder + Send + Sync>,
    chunk_cfg: ChunkConfig,
    tokenizer: tokenizers::Tokenizer,
    embedding_dim: usize,
    on_progress: F,
    fetch_url: Option<Arc<dyn Fn(&str) -> Result<(String, String), String> + Send + Sync>>,
) -> Result<(), IngestError>
where
    F: Fn(IngestProgress) + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut store = store;
        run_blocking(
            &job,
            &mut store,
            embedder.as_ref(),
            &chunk_cfg,
            &tokenizer,
            embedding_dim,
            on_progress,
            fetch_url.as_deref().map(|f| f as _),
        )
    })
    .await
    .map_err(|e| IngestError::Join(e.to_string()))?
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("неподдерживаемая операция: {0}")]
    Unsupported(String),
    #[error("spawn_blocking panic: {0}")]
    Join(String),
}

enum DiscoveredTask {
    File { path: std::path::PathBuf, kind: SourceKind },
    Url(String),
}

impl DiscoveredTask {
    fn display(&self) -> String {
        match self {
            Self::File { path, .. } => path.display().to_string(),
            Self::Url(u) => u.clone(),
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    let res = h.finalize();
    let mut s = String::with_capacity(64);
    for b in res {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

// Используем let _ = ParsedDoc; чтобы shut up warning'и про unused import,
// если parsed выпадает в no-op ветке. Сам тип нужен из публичного API.
#[allow(dead_code)]
fn _type_anchor() -> Option<ParsedDoc> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_stable() {
        let s = sha256_hex(b"hello");
        assert_eq!(s.len(), 64);
        assert_eq!(s, sha256_hex(b"hello"));
        assert_ne!(s, sha256_hex(b"hellow"));
    }

    #[test]
    fn ingest_stage_default_progression() {
        // Smoke: enum derive Eq/Copy.
        let s: IngestStage = IngestStage::Discovering;
        assert_eq!(s, IngestStage::Discovering);
        assert_ne!(s, IngestStage::Done);
    }
}
