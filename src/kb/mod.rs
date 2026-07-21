//! Knowledge base — RAG-подсистема synthos.
//!
//! Архитектура:
//! ```text
//!  ┌──────────────────────────────────────────────────────────────┐
//!  │ Settings UI (pages/settings/knowledge_base)                  │
//!  │   ↕ KbCtx (signals)                                          │
//!  ├──────────────────────────────────────────────────────────────┤
//!  │ Ingest pipeline (kb::ingest)                                  │
//!  │   DocSource → parse → chunk → embed → store                  │
//!  ├──────────────────────────────────────────────────────────────┤
//!  │ Hybrid search (kb::search) — BM25 ⊕ cosine через RRF          │
//!  │   ↕                                                           │
//!  │ Store (kb::store) — rusqlite + FTS5                          │
//!  └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Один файл `~/.config/synthos/kb/<collection_id>.sqlite` = одна коллекция.
//! Регистрация коллекций — на лету через скан каталога; в config.json не
//! пишем (устойчиво к ручным удалениям).
//!
//! Доступ из чата — через tool `kb_search` (см. `chat/tools/catalog.rs`).
//! Опциональный auto-augment — фаза в `chat/session.rs::start_agent_turn`.

pub mod augment;
pub mod collection;
pub mod ctx;
pub mod ingest;
pub mod loader;
pub mod runner;
pub mod search;
pub mod store;
pub mod vector_index;

pub use collection::{CollectionMeta, CollectionRegistry};
pub use ctx::KbCtx;
pub use search::{hybrid_search, SearchHit};
pub use store::{
    init_sqlite_vec, is_sqlite_vec_registered, ChunkRow, ChunkWithDoc, DocumentRow, KbStats,
    Store, StoreError, DEFAULT_VECTOR_INDEX_THRESHOLD,
};
pub use vector_index::{VectorIndex, VectorIndexKind};
