//! Конвейер ingestion: DocSource → parsed → chunks → embeddings → store.
//!
//! Текущая реализация — синхронный `run_blocking` (вызывается из
//! `tokio::task::spawn_blocking` в обёртке `run`). Прогресс публикуется
//! через коллбек `report_progress`, чтобы не было жёсткой связи с
//! syngui::async_runtime в этом модуле.
//!
//! Два правила, на которых держится честность коллекции:
//! - документ попадает в БД только вместе со своими чанками
//!   ([`Store::write_document`]) — оборванная индексация не оставляет после
//!   себя строку без единого фрагмента, по которой поиск молча ничего не
//!   находит, а повторный запуск пропускает файл по совпавшему sha;
//! - ни одна ошибка источника не теряется: она попадает в
//!   [`IngestOutcome::failed`], и вызывающий показывает её пользователю,
//!   а не только в лог.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use syngui::tr;
use synaptix_rag::doc::{chunk, parse, ChunkConfig, SourceKind};
use synaptix::facade::embedding::Embedder;

use super::source::DocSource;
use crate::kb::store::{ChunkRow, DocumentRow, Store};

/// Высокоуровневый стейдж — для UI ProgressBar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestStage {
    /// Job принят, но конвейер ещё не пошёл: грузится эмбеддер/токенайзер.
    Preparing,
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

/// Чем кончилась индексация. Возвращается наружу, чтобы вызывающий сказал
/// пользователю правду: сколько источников легло в коллекцию, сколько было
/// уже проиндексировано и что именно не получилось.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestOutcome {
    /// Источники, чьи чанки записаны в БД в этом прогоне.
    pub indexed: usize,
    /// Источники, пропущенные как неизменившиеся (sha тот же, чанки на месте).
    pub skipped: usize,
    /// Сколько чанков записано.
    pub chunks: usize,
    /// Источники, которые не удалось проиндексировать.
    pub failed: Vec<IngestFailure>,
    /// Прогон оборвали кнопкой «Отменить».
    pub cancelled: bool,
}

/// Источник и причина, по которой он не попал в коллекцию.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestFailure {
    pub source: String,
    pub error: String,
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
) -> Result<IngestOutcome, IngestError> {
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
    let mut outcome = IngestOutcome::default();

    // 1. Discovery — раскручиваем все sources в плоский список (path/url, kind, bytes_loader).
    emit(IngestStage::Discovering, 0, 0, None, None);

    let mut tasks: Vec<DiscoveredTask> = Vec::new();
    for src in &job.sources {
        match src {
            DocSource::File(p) => match SourceKind::from_path(p) {
                Some(kind) => tasks.push(DiscoveredTask::File {
                    path: p.clone(),
                    kind,
                }),
                // Формат неизвестен: раньше такой файл просто исчезал из
                // задания — пользователь видел «готово» и пустую коллекцию.
                None => outcome.failed.push(IngestFailure {
                    source: p.display().to_string(),
                    error: tr!("kb.ingest.unsupported_format"),
                }),
            },
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
        return Ok(outcome);
    }

    // 2. Per-task: read → parse → chunk → embed → store.
    for (idx, task) in tasks.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            emit(IngestStage::Cancelled, idx, total, None, None);
            return Ok(outcome);
        }

        let task_label = task.display();
        // Ошибка одного источника не останавливает остальные, но и не
        // теряется: `?` внутри замыкания → запись в `outcome.failed`.
        match ingest_one(
            task,
            store,
            embedder,
            chunk_cfg,
            tokenizer,
            embedding_dim,
            fetch_url,
            |stage| emit(stage, idx, total, Some(task_label.clone()), None),
        ) {
            Ok(TaskResult::Indexed { chunks }) => {
                outcome.indexed += 1;
                outcome.chunks += chunks;
            }
            Ok(TaskResult::Skipped) => outcome.skipped += 1,
            Err(error) => {
                log::warn!("kb ingest: {task_label}: {error}");
                outcome.failed.push(IngestFailure {
                    source: task_label.clone(),
                    error,
                });
            }
        }
    }

    emit(
        IngestStage::Done,
        total,
        total,
        None,
        outcome.failed.first().map(|f| f.error.clone()),
    );
    Ok(outcome)
}

/// Что стало с одним источником.
enum TaskResult {
    Indexed { chunks: usize },
    Skipped,
}

/// Один источник: прочитать → сверить с БД → распарсить → нарезать →
/// посчитать эмбеддинги → записать документ с чанками одной транзакцией.
/// Любой сбой — `Err(текст)`, который увидит пользователь.
#[allow(clippy::too_many_arguments)]
fn ingest_one(
    task: &DiscoveredTask,
    store: &mut Store,
    embedder: &dyn Embedder,
    chunk_cfg: &ChunkConfig,
    tokenizer: &tokenizers::Tokenizer,
    embedding_dim: usize,
    fetch_url: Option<&dyn Fn(&str) -> Result<(String, String), String>>,
    mut stage: impl FnMut(IngestStage),
) -> Result<TaskResult, String> {
    stage(IngestStage::Parsing);

    let (bytes, source_kind, source_kind_str, source_path, doc_title): (
        Vec<u8>,
        SourceKind,
        String,
        String,
        Option<String>,
    ) = match task {
        DiscoveredTask::File { path, kind } => {
            let bs = std::fs::read(path).map_err(|e| e.to_string())?;
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
            let fetch = fetch_url.ok_or_else(|| tr!("kb.ingest.no_fetcher"))?;
            let (md, title) = fetch(u)?;
            (
                md.into_bytes(),
                SourceKind::Markdown,
                "url".into(),
                u.clone(),
                Some(title),
            )
        }
    };

    // sha256 от исходных bytes — стабильнее, чем от parsed.plain_text
    // (markdown / html форматирование может менять чанк, но если bytes
    // те же — ингест уже актуальный). Документ без чанков переиндексируем
    // даже при совпавшем sha: искать по нему всё равно нечего.
    let sha256 = sha256_hex(&bytes);
    let known = store
        .document_status(&source_kind_str, &source_path)
        .map_err(|e| e.to_string())?;
    if known.is_some_and(|d| d.sha256 == sha256 && d.chunk_count > 0) {
        return Ok(TaskResult::Skipped);
    }

    let parsed = parse(&bytes, source_kind).map_err(|e| e.to_string())?;
    if parsed.plain_text.trim().is_empty() {
        return Err(tr!("kb.ingest.no_text"));
    }
    // ParsedDoc::title (h1 для md, <title> для html) превалирует над именем файла.
    let final_title = parsed.title.clone().or(doc_title);

    stage(IngestStage::Chunking);
    let chunks = chunk(&parsed, tokenizer, chunk_cfg).map_err(|e| e.to_string())?;
    if chunks.is_empty() {
        return Err(tr!("kb.ingest.no_chunks"));
    }

    stage(IngestStage::Embedding);
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let vectors = embedder.encode(&texts)?;
    if vectors.len() != chunks.len() {
        return Err(tr!(
            "kb.ingest.vector_count_mismatch",
            got = vectors.len(),
            expected = chunks.len()
        ));
    }

    stage(IngestStage::Storing);
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
    let count = chunks.len();
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
    store
        .write_document(&doc_row, &chunk_rows, embedding_dim)
        .map_err(|e| e.to_string())?;
    Ok(TaskResult::Indexed { chunks: count })
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
) -> Result<IngestOutcome, IngestError>
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


#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use synaptix::facade::embedding::EmbeddingResult;

    use super::*;
    use crate::kb::collection::CollectionMeta;

    const DIM: usize = 8;

    /// Эмбеддер, которым можно управлять из теста: считает «хэш-векторы»,
    /// а при `fail` падает — как настоящий на нехватке VRAM.
    struct StubEmbedder {
        fail: bool,
    }

    impl Embedder for StubEmbedder {
        fn dim(&self) -> usize {
            DIM
        }
        fn max_tokens(&self) -> usize {
            512
        }
        fn encode(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
            if self.fail {
                return Err("CUDA out of memory".to_string());
            }
            Ok(texts
                .iter()
                .map(|t| {
                    let seed = t.len() as f32;
                    let mut v = vec![0.0f32; DIM];
                    for (i, x) in v.iter_mut().enumerate() {
                        *x = ((seed + i as f32) % 7.0) / 7.0;
                    }
                    let n = synaptix::facade::embedding::l2_norm(&v).max(1e-12);
                    for x in v.iter_mut() {
                        *x /= n;
                    }
                    v
                })
                .collect())
        }
    }

    /// Пословный токенайзер: чанкеру нужны только offset'ы токенов.
    fn tokenizer() -> tokenizers::Tokenizer {
        const JSON: &str = r#"{
            "version": "1.0",
            "truncation": null,
            "padding": null,
            "added_tokens": [],
            "normalizer": null,
            "pre_tokenizer": { "type": "Whitespace" },
            "post_processor": null,
            "decoder": null,
            "model": { "type": "WordLevel", "vocab": { "[UNK]": 0 }, "unk_token": "[UNK]" }
        }"#;
        tokenizers::Tokenizer::from_bytes(JSON.as_bytes()).unwrap()
    }

    fn store() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&CollectionMeta {
            id: "test".into(),
            name: "test".into(),
            created_at: 0,
            embedding_model: "stub".into(),
            embedding_dim: DIM as i32,
            chunk_target_tokens: 16,
            chunk_overlap_tokens: 4,
            document_count: 0,
            chunk_count: 0,
        })
        .unwrap();
        s
    }

    fn job(paths: Vec<std::path::PathBuf>) -> IngestJob {
        IngestJob {
            job_id: 1,
            collection_id: "test".into(),
            sources: paths.into_iter().map(DocSource::File).collect(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    fn run(job: &IngestJob, store: &mut Store, fail: bool) -> IngestOutcome {
        let cfg = ChunkConfig {
            target_tokens: 16,
            overlap_tokens: 4,
            min_tokens: 2,
        };
        run_blocking(
            job,
            store,
            &StubEmbedder { fail },
            &cfg,
            &tokenizer(),
            DIM,
            |_| {},
            None,
        )
        .unwrap()
    }

    fn temp_md(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("synthos-kb-pipeline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

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

    /// Сбой эмбеддинга не должен оставлять документ в БД: раньше строка
    /// записывалась до векторов, и коллекция с виду была полной, а поиск
    /// по ней не находил ничего — навсегда, потому что повторная
    /// индексация пропускала файл по совпавшему sha.
    #[test]
    fn failed_embedding_leaves_nothing_and_next_run_retries() {
        let path = temp_md("family.md", "Брачный возраст восемнадцать лет для мужчин и женщин.");
        let job = job(vec![path.clone()]);
        let mut store = store();

        let outcome = run(&job, &mut store, true);
        assert_eq!(outcome.indexed, 0);
        assert_eq!(outcome.failed.len(), 1, "ошибка источника видна наружу");
        assert!(outcome.failed[0].error.contains("out of memory"), "{outcome:?}");
        let stats = store.stats().unwrap();
        assert_eq!(stats.document_count, 0, "пустых документов в БД не остаётся");
        assert_eq!(stats.chunk_count, 0);

        let outcome = run(&job, &mut store, false);
        assert_eq!(outcome.indexed, 1, "следующий запуск берётся за файл снова");
        assert!(outcome.failed.is_empty());
        assert!(outcome.chunks > 0);
        let stats = store.stats().unwrap();
        assert_eq!(stats.document_count, 1);
        assert!(stats.chunk_count > 0);

        // Тот же файл без изменений — повторно не считаем.
        let outcome = run(&job, &mut store, false);
        assert_eq!(outcome.indexed, 0);
        assert_eq!(outcome.skipped, 1);
        let _ = std::fs::remove_file(path);
    }

    /// Документ без чанков (наследство старой версии) переиндексируется,
    /// хотя sha файла не менялся.
    #[test]
    fn document_without_chunks_is_reindexed() {
        let path = temp_md("orphan.md", "Опека и попечительство устанавливаются судом.");
        let job = job(vec![path.clone()]);
        let mut store = store();
        let bytes = std::fs::read(&path).unwrap();
        store
            .upsert_document(&DocumentRow {
                id: None,
                source_kind: "file".into(),
                source_path: path.display().to_string(),
                sha256: sha256_hex(&bytes),
                title: None,
                bytes: bytes.len() as i64,
                indexed_at: 0,
            })
            .unwrap();
        assert_eq!(store.stats().unwrap().chunk_count, 0);

        let outcome = run(&job, &mut store, false);
        assert_eq!(outcome.indexed, 1, "пустой документ не считается свежим");
        assert!(store.stats().unwrap().chunk_count > 0);
        let _ = std::fs::remove_file(path);
    }

    /// Файл неизвестного формата раньше молча исчезал из задания.
    #[test]
    fn unsupported_format_is_reported() {
        let path = temp_md("archive.bin", "не текст");
        let job = job(vec![path.clone()]);
        let mut store = store();
        let outcome = run(&job, &mut store, false);
        assert_eq!(outcome.indexed, 0);
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].source, path.display().to_string());
        let _ = std::fs::remove_file(path);
    }
}
