//! Ingest pipeline (заполнение коллекции документами).
//!
//! Эта модулька — оркестратор. Тяжёлая работа делится между:
//! - `kb::store` (sqlite/FTS),
//! - `synaptix_rag::doc` (parsing + chunking),
//! - `synaptix::facade::embedding` (encode batches).
//!
//! `IngestJob` — описание одной запущенной задачи, состоит из перечня
//! [`source::DocSource`] и target collection_id. Прогресс публикуется
//! через [`KbCtx::ingest_progress`] (см. `kb::ctx`).
//!
//! ВАЖНО: на одну коллекцию допускается одновременно один активный job.
//! Запуск нового job'а во время активного — отклоняется.

pub mod pipeline;
pub mod source;

pub use pipeline::{run, run_blocking, IngestJob, IngestProgress, IngestStage};
pub use source::DocSource;
