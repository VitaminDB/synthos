//! Реактивный «фасад» KB-подсистемы для UI.
//!
//! `KbCtx` живёт в `AppCtx` рядом с `chat`/`general`/`audio_models`
//! и даёт UI/командному коду общий доступ к:
//! - списку коллекций (через `CollectionRegistry`),
//! - выбранной активной коллекции в Settings (`active_collection_id`),
//! - набору активных коллекций для текущего чата (`active_in_chat_ids`),
//! - флагу auto-augment system prompt (`auto_augment`),
//! - текущему прогрессу ingestion'а (`ingest_progress`),
//! - результатам последнего поиска (`last_search_hits`) — для отладки UI.
//!
//! Embedder загружается лениво (тяжёлый — sec-минут на CPU) при первом
//! ingest/search; до этого UI показывает прогресс «Загрузка модели…».

use std::sync::{Arc, Mutex};

use synaptix::facade::embedding::Embedder;
use synaptix::facade::rerank::Reranker;
use syngui::prelude::{use_signal, RwSignal};

use super::collection::CollectionRegistry;
use super::ingest::IngestProgress;
use super::search::SearchHit;

/// Реактивный контекст KB.
#[derive(Clone)]
pub struct KbCtx {
    /// Список всех коллекций (path = `kb_dir/<id>.sqlite`). Обновляется
    /// при `scan()` и при create/delete операциях.
    pub registry: RwSignal<CollectionRegistry>,
    /// Какая коллекция выбрана в Settings UI («открыт редактор»).
    pub active_collection_id: RwSignal<Option<String>>,
    /// Какие коллекции активны для текущего чата (multi-select). Не
    /// сохраняются в config.json — это per-session состояние, привязанное
    /// к открытому окну.
    pub active_in_chat_ids: RwSignal<Vec<String>>,
    /// Флаг auto-augment system prompt'а перед каждым agent-turn'ом.
    pub auto_augment: RwSignal<bool>,
    /// Открыт ли popover KB-чипа в input-панели чата. Локальный
    /// `use_signal` в замыкании `view()` не подошёл бы: триггер (chip)
    /// и тело (Portal-overlay) монтируются в разных узлах дерева.
    pub kb_chip_open: RwSignal<bool>,
    /// `true`, пока async pre-step `kb::augment::compute` крутится в
    /// `start_agent_turn`. Используется индикатором в input-панели
    /// «Поиск в KB…» и сбрасывается в `chat::session::abort`.
    pub augment_in_progress: RwSignal<bool>,
    /// Текущий ingest job (если идёт). None между job'ами.
    pub ingest_progress: RwSignal<Option<IngestProgress>>,
    /// Stop-токен текущего ingest job'а — UI ставит в true при «Cancel».
    pub ingest_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Hits из последнего kb_search (для отладочной панели).
    pub last_search_hits: RwSignal<Vec<SearchHit>>,
    /// Lazy-загружаемый embedder. Делится через Arc, чтобы ingest и search
    /// могли работать с одним инстансом.
    pub embedder: Arc<Mutex<Option<Arc<dyn Embedder + Send + Sync>>>>,
    /// Lazy-загружаемый cross-encoder reranker. Используется как
    /// опциональная пост-фаза в `hybrid_search` поверх RRF top-K'. Если
    /// `None` — поиск ведёт себя как раньше (BM25 ⊕ cosine RRF).
    pub reranker: Arc<Mutex<Option<Arc<dyn Reranker + Send + Sync>>>>,
}

impl KbCtx {
    pub fn new(kb_dir: impl Into<std::path::PathBuf>) -> Self {
        let mut reg = CollectionRegistry::new(kb_dir);
        reg.scan();
        Self {
            registry: use_signal(reg),
            active_collection_id: use_signal(None),
            active_in_chat_ids: use_signal(Vec::new()),
            auto_augment: use_signal(false),
            kb_chip_open: use_signal(false),
            augment_in_progress: use_signal(false),
            ingest_progress: use_signal(None),
            ingest_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_search_hits: use_signal(Vec::new()),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
        }
    }

    /// Snapshot активных коллекций (без подписки) — для tool-исполнителей.
    pub fn active_collection_ids(&self) -> Vec<String> {
        self.active_in_chat_ids.get_untracked()
    }

    /// Получить (или загрузить впервые) embedder. На каждый `try_load`
    /// проверяем lock — если уже загружен, возвращаем clone Arc'а.
    pub fn get_embedder(&self) -> Option<Arc<dyn Embedder + Send + Sync>> {
        self.embedder.lock().ok().and_then(|g| g.as_ref().cloned())
    }

    pub fn set_embedder(&self, e: Arc<dyn Embedder + Send + Sync>) {
        if let Ok(mut g) = self.embedder.lock() {
            *g = Some(e);
        }
    }

    pub fn drop_embedder(&self) {
        if let Ok(mut g) = self.embedder.lock() {
            *g = None;
        }
    }

    pub fn get_reranker(&self) -> Option<Arc<dyn Reranker + Send + Sync>> {
        self.reranker.lock().ok().and_then(|g| g.as_ref().cloned())
    }

    pub fn set_reranker(&self, r: Arc<dyn Reranker + Send + Sync>) {
        if let Ok(mut g) = self.reranker.lock() {
            *g = Some(r);
        }
    }

    pub fn drop_reranker(&self) {
        if let Ok(mut g) = self.reranker.lock() {
            *g = None;
        }
    }
}
