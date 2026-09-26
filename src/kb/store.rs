//! Хранилище одной коллекции — обёртка над rusqlite + FTS5.
//!
//! Одна `<id>.sqlite` БД на коллекцию. Схема:
//! - `collections (id, name, created_at, embedding_model, embedding_dim, …)`
//!   — однострочная meta-таблица; именно здесь живут параметры коллекции.
//! - `documents (id, source_kind, source_path, sha256, title, bytes, indexed_at)`
//!   — список проиндексированных источников с UNIQUE(kind, path) для
//!   дедупликации.
//! - `chunks (id, document_id, ord, text, start_byte, end_byte, token_count, embedding BLOB)`
//!   — собственно чанки. `embedding` — `f32[dim]` LE-байты, всегда
//!   L2-нормализованный (для cosine = dot product).
//! - `chunks_fts` — внешний FTS5 индекс над `text`. Триггеры синхронизируют
//!   insert/update/delete.
//! - `chunks_vec` *(опционально, feature `kb-sqlite-vec`)* — vec0 virtual
//!   table из crate `sqlite-vec`. Подключается через `init_sqlite_vec()`
//!   (one-shot, регистрирует extension'а через `sqlite3_auto_extension`).
//!
//! Vector search: BF (full table scan) + cosine — до 50-100k чанков это
//! < 50ms. При большем масштабе автоматически переключается на vec0 KNN
//! (cм. [`super::vector_index`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use rusqlite::{params, Connection, OptionalExtension};

use super::collection::CollectionMeta;
use super::vector_index::{VectorIndex, VectorIndexKind};

/// Дефолтный порог для `VectorIndexKind::Auto`: при `count(chunks) ≥ 100_000`
/// vector_search автоматически использует vec0 KNN.
pub const DEFAULT_VECTOR_INDEX_THRESHOLD: usize = 100_000;

/// Режим и порог векторного индекса из `KbConfig` (`vector_index_kind`,
/// `vector_index_threshold`) для [`Store::open`]. Ставится при старте из
/// конфига; `Store::open` зовут и рабочие потоки, где контекста приложения нет.
static VECTOR_INDEX_DEFAULTS: std::sync::RwLock<(VectorIndexKind, usize)> =
    std::sync::RwLock::new((VectorIndexKind::Auto, DEFAULT_VECTOR_INDEX_THRESHOLD));

pub fn set_vector_index_defaults(kind: VectorIndexKind, threshold: usize) {
    let threshold = if threshold == 0 { DEFAULT_VECTOR_INDEX_THRESHOLD } else { threshold };
    *VECTOR_INDEX_DEFAULTS.write().unwrap_or_else(|e| e.into_inner()) = (kind, threshold);
}

fn vector_index_defaults() -> (VectorIndexKind, usize) {
    *VECTOR_INDEX_DEFAULTS.read().unwrap_or_else(|e| e.into_inner())
}

static SQLITE_VEC_REGISTERED: AtomicBool = AtomicBool::new(false);
static SQLITE_VEC_INIT: OnceLock<()> = OnceLock::new();

/// Зарегистрировать sqlite-vec как auto-extension. Безопасно вызывается
/// несколько раз — реальная регистрация выполняется один раз на процесс.
///
/// Без feature `kb-sqlite-vec` — no-op (флаг остаётся `false`,
/// все [`VectorIndex`] деградируют в `Scan`).
pub fn init_sqlite_vec() {
    SQLITE_VEC_INIT.get_or_init(|| {
        #[cfg(feature = "kb-sqlite-vec")]
        unsafe {
            use rusqlite::ffi;
            // rusqlite::ffi::sqlite3_auto_extension ожидает конкретную
            // SQLite-extension-init сигнатуру `Option<unsafe extern "C"
            // fn(db, pzErrMsg, pApi) -> c_int>`. `sqlite-vec` экспортирует
            // entry-point ровно с этой сигнатурой, но его типы C-структур
            // приходят из его собственного `libsqlite3-sys` reexport'а.
            // Перед передачей делаем `transmute` указателя — обе обёртки
            // в итоге линкуются к одному `libsqlite3-sys` через `links =
            // "sqlite3"`, и ABI совпадает.
            type AutoExtFn = unsafe extern "C" fn(
                *mut ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;
            let entry: AutoExtFn =
                std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
            let rc = ffi::sqlite3_auto_extension(Some(entry));
            if rc == ffi::SQLITE_OK {
                SQLITE_VEC_REGISTERED.store(true, Ordering::SeqCst);
                log::info!("kb: sqlite-vec extension зарегистрирован");
            } else {
                log::warn!("kb: sqlite3_auto_extension вернул rc={rc}");
            }
        }
    });
}

/// Был ли `init_sqlite_vec()` успешно завершён в этом процессе. Не учитывает
/// версию SQLite — только тот факт, что регистрация прошла без ошибок.
pub fn is_sqlite_vec_registered() -> bool {
    SQLITE_VEC_REGISTERED.load(Ordering::SeqCst)
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("schema: collection meta отсутствует")]
    NoMeta,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("dim mismatch: ожидалось {expected}, получили {got}")]
    DimMismatch { expected: usize, got: usize },
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
    /// Желаемый режим vector-index'а. Реальный режим определяется в
    /// [`Store::try_init_vector_index`] после того, как известен `dim`.
    vec_kind: VectorIndexKind,
    /// Порог для `Auto` (см. [`VectorIndex::open`]).
    vec_threshold: usize,
    /// Lazy-инициализируемый индекс. Появляется как только известна meta
    /// коллекции (т.е. сразу после [`Store::upsert_collection`] либо
    /// при открытии существующей БД с meta).
    vec_index: Option<VectorIndex>,
}

impl Store {
    /// Открыть/создать БД. Не пишет схему — это `ensure_schema`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let (kind, threshold) = vector_index_defaults();
        Self::open_with_config(path, kind, threshold)
    }

    /// Открыть с явным выбором режима векторного индекса. Если БД уже
    /// содержит meta — индекс готов сразу; иначе ждём `upsert_collection`.
    pub fn open_with_config(
        path: &Path,
        vec_kind: VectorIndexKind,
        vec_threshold: usize,
    ) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::tune_pragmas(&conn)?;
        let mut s = Self {
            conn,
            path: path.to_path_buf(),
            vec_kind,
            vec_threshold,
            vec_index: None,
        };
        // На случай повторного открытия существующей БД — индекс надо
        // сразу подцепить, чтобы insert_chunks/vector_search видели его.
        let _ = s.try_init_vector_index();
        Ok(s)
    }

    /// Открыть в памяти — для тестов.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::tune_pragmas(&conn)?;
        Ok(Self {
            conn,
            path: PathBuf::from(":memory:"),
            vec_kind: VectorIndexKind::Auto,
            vec_threshold: DEFAULT_VECTOR_INDEX_THRESHOLD,
            vec_index: None,
        })
    }

    fn tune_pragmas(conn: &Connection) -> Result<(), StoreError> {
        // WAL — выдерживает одновременные read'ы во время write'а ingest'а.
        // synchronous=NORMAL — компромисс надёжность/скорость; для индекса
        // с лёгким revert (re-index) это ок.
        // foreign_keys — для каскадного удаления chunks при delete document.
        // busy_timeout — страница настроек перечитывает список документов из
        // той же БД, пока идёт индексация; без ожидания запись падала бы
        // мгновенным SQLITE_BUSY.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             PRAGMA temp_store=MEMORY;",
        )?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Создать (если ещё нет) все таблицы / триггеры FTS / индексы.
    pub fn ensure_schema(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch(SCHEMA_SQL)?;
        // На случай повторного `ensure_schema` на open'нутой существующей
        // БД — пробуем поднять vector_index, если meta уже есть.
        let _ = self.try_init_vector_index();
        Ok(())
    }

    pub fn upsert_collection(&mut self, meta: &CollectionMeta) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO collections \
             (id, name, created_at, embedding_model, embedding_dim, chunk_target_tokens, chunk_overlap_tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(id) DO UPDATE SET \
                name=excluded.name, \
                embedding_model=excluded.embedding_model, \
                embedding_dim=excluded.embedding_dim, \
                chunk_target_tokens=excluded.chunk_target_tokens, \
                chunk_overlap_tokens=excluded.chunk_overlap_tokens",
            params![
                meta.id,
                meta.name,
                meta.created_at,
                meta.embedding_model,
                meta.embedding_dim,
                meta.chunk_target_tokens,
                meta.chunk_overlap_tokens,
            ],
        )?;
        // Теперь, когда dim известен, можно поднять vector_index.
        let _ = self.try_init_vector_index();
        Ok(())
    }

    /// Текущий режим vector-index'а; `None` пока не было `upsert_collection`.
    pub fn vector_index_kind(&self) -> Option<VectorIndexKind> {
        self.vec_index.as_ref().map(|v| v.kind())
    }

    /// Принудительно сменить желаемый режим / порог. Реальный пересчёт
    /// (создание `chunks_vec`, reindex) произойдёт в `try_init_vector_index`.
    pub fn set_vector_index_config(&mut self, kind: VectorIndexKind, threshold: usize) {
        self.vec_kind = kind;
        self.vec_threshold = threshold;
        self.vec_index = None;
        let _ = self.try_init_vector_index();
    }

    /// Пытается инициализировать vector_index, если есть meta + dim>0. Если
    /// финальный режим — `SqliteVec` и `chunks_vec` ещё пуст при наличии
    /// embedding'ов в `chunks` — выполняет `reindex_all` (миграция старой БД).
    /// Идемпотентен: повторные вызовы — no-op после первой успешной инициализации.
    pub fn try_init_vector_index(&mut self) -> Result<(), StoreError> {
        if self.vec_index.is_some() {
            return Ok(());
        }
        let meta = match self.read_meta() {
            Ok(m) => m,
            Err(StoreError::NoMeta) => return Ok(()),
            Err(e) => return Err(e),
        };
        let dim = meta.embedding_dim.max(0) as usize;
        let idx = VectorIndex::open(&self.conn, dim, self.vec_kind, self.vec_threshold)?;
        let kind = idx.kind();
        self.vec_index = Some(idx);
        if matches!(kind, VectorIndexKind::SqliteVec) {
            // Если БД переоткрыта со старой схемой (chunks уже есть, vec0
            // только что создан) — донаполнить vec0 текущим содержимым.
            let n = self
                .vec_index
                .as_ref()
                .expect("just set")
                .reindex_all(&self.conn)?;
            if n > 0 {
                log::info!("kb: проиндексировано {n} существующих чанков в chunks_vec");
            }
        }
        Ok(())
    }

    /// Прочитать meta. Возвращает [`StoreError::NoMeta`], если строки нет
    /// (untouched БД).
    pub fn read_meta(&self) -> Result<CollectionMeta, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, created_at, embedding_model, embedding_dim, \
                        chunk_target_tokens, chunk_overlap_tokens \
                 FROM collections LIMIT 1",
                [],
                |r| {
                    Ok(CollectionMeta {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                        embedding_model: r.get(3)?,
                        embedding_dim: r.get(4)?,
                        chunk_target_tokens: r.get(5)?,
                        chunk_overlap_tokens: r.get(6)?,
                        document_count: 0,
                        chunk_count: 0,
                    })
                },
            )
            .optional()?;
        row.ok_or(StoreError::NoMeta)
    }

    /// Найти документ по (source_kind, source_path). Возвращает row id.
    pub fn find_document(
        &self,
        source_kind: &str,
        source_path: &str,
    ) -> Result<Option<i64>, StoreError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM documents WHERE source_kind=?1 AND source_path=?2",
                params![source_kind, source_path],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id)
    }

    /// Что БД уже знает про источник: id, sha и сколько у него чанков.
    ///
    /// Ноль чанков — документ «пустой»: строку записали, а эмбеддинги до неё
    /// не дошли (прошлую индексацию оборвали или она упала). Такой источник
    /// надо индексировать заново, поэтому [`super::ingest::pipeline`] смотрит
    /// не только на sha.
    pub fn document_status(
        &self,
        source_kind: &str,
        source_path: &str,
    ) -> Result<Option<DocumentStatus>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT d.id, d.sha256, (SELECT COUNT(*) FROM chunks c WHERE c.document_id = d.id) \
                 FROM documents d WHERE d.source_kind=?1 AND d.source_path=?2",
                params![source_kind, source_path],
                |r| {
                    Ok(DocumentStatus {
                        id: r.get(0)?,
                        sha256: r.get(1)?,
                        chunk_count: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Вставить или обновить документ. Возвращает row id.
    /// Если sha256 совпал с существующим — НЕ пересоздаём, возвращаем
    /// прежний id и `false` во втором поле (caller'у это сигнал
    /// «можно пропустить чанкинг + embed»).
    pub fn upsert_document(&self, doc: &DocumentRow) -> Result<(i64, bool), StoreError> {
        let existing = self.conn.query_row(
            "SELECT id, sha256 FROM documents WHERE source_kind=?1 AND source_path=?2",
            params![doc.source_kind, doc.source_path],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        ).optional()?;

        match existing {
            Some((id, sha)) if sha == doc.sha256 => Ok((id, false)),
            Some((id, _)) => {
                // Контент изменился — каскадно сносим чанки и обновляем row.
                self.conn.execute(
                    "UPDATE documents SET sha256=?1, title=?2, bytes=?3, indexed_at=?4 WHERE id=?5",
                    params![doc.sha256, doc.title, doc.bytes, doc.indexed_at, id],
                )?;
                self.conn.execute(
                    "DELETE FROM chunks WHERE document_id=?1",
                    params![id],
                )?;
                Ok((id, true))
            }
            None => {
                self.conn.execute(
                    "INSERT INTO documents (source_kind, source_path, sha256, title, bytes, indexed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![doc.source_kind, doc.source_path, doc.sha256, doc.title, doc.bytes, doc.indexed_at],
                )?;
                Ok((self.conn.last_insert_rowid(), true))
            }
        }
    }

    pub fn delete_document(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM documents WHERE id=?1", params![id])?;
        // chunks подхватываются ON DELETE CASCADE.
        Ok(())
    }

    /// Вставить пакет чанков. Все в одной транзакции. Если активен
    /// `chunks_vec` (sqlite-vec) — каждая строка одновременно идёт в vec0.
    pub fn insert_chunks(
        &mut self,
        document_id: i64,
        chunks: &[ChunkRow],
        embedding_dim: usize,
    ) -> Result<(), StoreError> {
        let vec_index_active = self
            .vec_index
            .as_ref()
            .map(|v| v.is_sqlite_vec())
            .unwrap_or(false);
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chunks \
                 (document_id, ord, text, start_byte, end_byte, token_count, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut vec_stmt = if vec_index_active {
                Some(tx.prepare(
                    "INSERT INTO chunks_vec(chunk_id, embedding) VALUES (?1, ?2)",
                )?)
            } else {
                None
            };
            for c in chunks {
                if c.embedding.len() != embedding_dim {
                    return Err(StoreError::DimMismatch {
                        expected: embedding_dim,
                        got: c.embedding.len(),
                    });
                }
                let blob = vec_f32_to_le_bytes(&c.embedding);
                stmt.execute(params![
                    document_id,
                    c.ord,
                    c.text,
                    c.start_byte as i64,
                    c.end_byte as i64,
                    c.token_count as i64,
                    blob.as_slice(),
                ])?;
                if let Some(vs) = vec_stmt.as_mut() {
                    let chunk_id = tx.last_insert_rowid();
                    vs.execute(params![chunk_id, blob.as_slice()])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Записать документ вместе с его чанками — одной транзакцией.
    ///
    /// Либо в БД появляется документ со всеми своими чанками, либо не
    /// меняется ничего. Раздельные `upsert_document` + `insert_chunks`
    /// оставляли после сбоя (или закрытия приложения между ними) строку
    /// документа без единого чанка: поиск по такой коллекции ничего не
    /// находил, а повторная индексация пропускала файл по совпавшему sha.
    pub fn write_document(
        &mut self,
        doc: &DocumentRow,
        chunks: &[ChunkRow],
        embedding_dim: usize,
    ) -> Result<i64, StoreError> {
        for c in chunks {
            if c.embedding.len() != embedding_dim {
                return Err(StoreError::DimMismatch {
                    expected: embedding_dim,
                    got: c.embedding.len(),
                });
            }
        }
        let vec_index_active = self
            .vec_index
            .as_ref()
            .map(|v| v.is_sqlite_vec())
            .unwrap_or(false);
        let tx = self.conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM documents WHERE source_kind=?1 AND source_path=?2",
                params![doc.source_kind, doc.source_path],
                |r| r.get(0),
            )
            .optional()?;
        let doc_id = match existing {
            Some(id) => {
                tx.execute(
                    "UPDATE documents SET sha256=?1, title=?2, bytes=?3, indexed_at=?4 WHERE id=?5",
                    params![doc.sha256, doc.title, doc.bytes, doc.indexed_at, id],
                )?;
                // Триггеры FTS и `chunks_vec_ad` вычистят индексы сами.
                tx.execute("DELETE FROM chunks WHERE document_id=?1", params![id])?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO documents (source_kind, source_path, sha256, title, bytes, indexed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![doc.source_kind, doc.source_path, doc.sha256, doc.title, doc.bytes, doc.indexed_at],
                )?;
                tx.last_insert_rowid()
            }
        };
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chunks \
                 (document_id, ord, text, start_byte, end_byte, token_count, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut vec_stmt = if vec_index_active {
                Some(tx.prepare("INSERT INTO chunks_vec(chunk_id, embedding) VALUES (?1, ?2)")?)
            } else {
                None
            };
            for c in chunks {
                let blob = vec_f32_to_le_bytes(&c.embedding);
                stmt.execute(params![
                    doc_id,
                    c.ord,
                    c.text,
                    c.start_byte as i64,
                    c.end_byte as i64,
                    c.token_count as i64,
                    blob.as_slice(),
                ])?;
                if let Some(vs) = vec_stmt.as_mut() {
                    let chunk_id = tx.last_insert_rowid();
                    vs.execute(params![chunk_id, blob.as_slice()])?;
                }
            }
        }
        tx.commit()?;
        Ok(doc_id)
    }

    /// FTS5 поиск по тексту. Возвращает (chunk_id, BM25-score).
    /// BM25 в SQLite возвращает «отрицательную релевантность» (чем меньше,
    /// тем лучше). Конвертируем в положительную: `score = -bm25(...)`.
    pub fn fts_search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<(i64, f32)>, StoreError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT c.id, -bm25(chunks_fts) AS score \
             FROM chunks_fts \
             JOIN chunks c ON c.id = chunks_fts.rowid \
             WHERE chunks_fts MATCH ?1 \
             ORDER BY score DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![sanitize_fts_query(query), top_k as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)? as f32))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Vector search. При активном `chunks_vec` (sqlite-vec) — KNN через
    /// vec0 (O(log N) на отсортированной структуре); иначе full-scan
    /// + cosine в Rust. Cosine считается как dot product (мы храним
    /// L2-нормализованные эмбеддинги).
    pub fn vector_search(
        &self,
        query_emb: &[f32],
        top_k: usize,
    ) -> Result<Vec<(i64, f32)>, StoreError> {
        if query_emb.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(idx) = self.vec_index.as_ref() {
            if let Some(hits) = idx.knn(&self.conn, query_emb, top_k)? {
                return Ok(hits);
            }
        }
        let mut stmt = self.conn.prepare("SELECT id, embedding FROM chunks")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;

        // BinaryHeap для top-k: keep min element on top, evict при превышении.
        // Простой Vec + sort — для < 100k чанков на одну query быстрее
        // (BinaryHeap pessimistic для short k и lock contention минимален).
        let mut scored: Vec<(i64, f32)> = Vec::with_capacity(1024);
        for row in rows.flatten() {
            let (id, blob) = row;
            let v = le_bytes_to_vec_f32(&blob);
            if v.len() != query_emb.len() {
                continue;
            }
            let s = dot_product(query_emb, &v);
            scored.push((id, s));
        }
        // Partial sort (descending) до top_k.
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(top_k);
        Ok(scored)
    }

    /// Поднять полный текст чанков по списку id. Сохраняет порядок ids.
    pub fn get_chunks_by_ids(&self, ids: &[i64]) -> Result<Vec<ChunkWithDoc>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // SQL не любит IN с динамическим длиной без re-prepare; делаем через
        // CTE с rowid'ами через параметры.
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT c.id, c.document_id, c.ord, c.text, c.start_byte, c.end_byte, c.token_count, \
                    d.source_kind, d.source_path, d.title \
             FROM chunks c \
             JOIN documents d ON d.id = c.document_id \
             WHERE c.id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let id_params: Vec<&dyn rusqlite::ToSql> = ids
            .iter()
            .map(|i| i as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(id_params.as_slice(), |r| {
            Ok(ChunkWithDoc {
                id: r.get(0)?,
                document_id: r.get(1)?,
                ord: r.get(2)?,
                text: r.get(3)?,
                start_byte: r.get::<_, i64>(4)? as usize,
                end_byte: r.get::<_, i64>(5)? as usize,
                token_count: r.get::<_, i64>(6)? as usize,
                source_kind: r.get(7)?,
                source_path: r.get(8)?,
                doc_title: r.get(9)?,
            })
        })?;
        let mut by_id: std::collections::HashMap<i64, ChunkWithDoc> = Default::default();
        for row in rows.flatten() {
            by_id.insert(row.id, row);
        }
        // Сохраняем исходный порядок:
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = by_id.remove(id) {
                out.push(c);
            }
        }
        Ok(out)
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_kind, source_path, sha256, title, bytes, indexed_at \
             FROM documents ORDER BY indexed_at DESC, source_path ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentRow {
                id: Some(r.get(0)?),
                source_kind: r.get(1)?,
                source_path: r.get(2)?,
                sha256: r.get(3)?,
                title: r.get(4)?,
                bytes: r.get(5)?,
                indexed_at: r.get(6)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Документы со счётчиком чанков — то, что показывает страница
    /// коллекции. Ноль у строки значит, что искать по этому документу
    /// нечем, и его стоит проиндексировать заново.
    pub fn list_documents_with_counts(&self) -> Result<Vec<DocumentInfo>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT d.id, d.source_kind, d.source_path, d.sha256, d.title, d.bytes, d.indexed_at, \
                    (SELECT COUNT(*) FROM chunks c WHERE c.document_id = d.id) \
             FROM documents d ORDER BY d.indexed_at DESC, d.source_path ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentInfo {
                row: DocumentRow {
                    id: Some(r.get(0)?),
                    source_kind: r.get(1)?,
                    source_path: r.get(2)?,
                    sha256: r.get(3)?,
                    title: r.get(4)?,
                    bytes: r.get(5)?,
                    indexed_at: r.get(6)?,
                },
                chunk_count: r.get(7)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn stats(&self) -> Result<KbStats, StoreError> {
        let document_count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
        let chunk_count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        Ok(KbStats {
            document_count,
            chunk_count,
        })
    }
}

/// Состояние источника в БД — для решения «индексировать или пропустить».
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentStatus {
    pub id: i64,
    pub sha256: String,
    pub chunk_count: i64,
}

/// Строка документа вместе с числом его чанков (для списка в настройках).
#[derive(Debug, Clone)]
pub struct DocumentInfo {
    pub row: DocumentRow,
    pub chunk_count: i64,
}

#[derive(Debug, Clone)]
pub struct DocumentRow {
    /// `None` при insert; устанавливается БД (last_insert_rowid).
    pub id: Option<i64>,
    pub source_kind: String,
    pub source_path: String,
    pub sha256: String,
    pub title: Option<String>,
    pub bytes: i64,
    pub indexed_at: i64,
}

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub ord: i32,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub token_count: usize,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct ChunkWithDoc {
    pub id: i64,
    pub document_id: i64,
    pub ord: i32,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub token_count: usize,
    pub source_kind: String,
    pub source_path: String,
    pub doc_title: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct KbStats {
    pub document_count: i64,
    pub chunk_count: i64,
}

// ───────────────────────────── helpers ─────────────────────────────

fn vec_f32_to_le_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn le_bytes_to_vec_f32(b: &[u8]) -> Vec<f32> {
    let n = b.len() / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * 4;
        let bytes = [b[off], b[off + 1], b[off + 2], b[off + 3]];
        out.push(f32::from_le_bytes(bytes));
    }
    out
}

fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    let mut s = 0.0f32;
    for i in 0..a.len() {
        s += a[i] * b[i];
    }
    s
}

/// Очень простая санитайзация FTS5-запроса: оборачиваем каждое слово
/// в `"…"`, чтобы пользовательские `:` / `(` / `OR` не сломали парсер.
/// Это превращает запрос в OR списка термов — что нам и нужно.
fn sanitize_fts_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 32);
    let mut first = true;
    for word in q.split_whitespace() {
        // Удаляем " внутри слова, иначе FTS5 распарсит как unterminated quote.
        let cleaned = word.replace('"', "");
        if cleaned.is_empty() {
            continue;
        }
        if !first {
            out.push_str(" OR ");
        }
        first = false;
        out.push('"');
        out.push_str(&cleaned);
        out.push('"');
    }
    if first {
        // Все слова отвалились — возвращаем что-то заведомо не-матчное.
        return "\"\"".to_string();
    }
    out
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS collections (
    id                   TEXT    PRIMARY KEY,
    name                 TEXT    NOT NULL,
    created_at           INTEGER NOT NULL,
    embedding_model      TEXT    NOT NULL,
    embedding_dim        INTEGER NOT NULL,
    chunk_target_tokens  INTEGER NOT NULL,
    chunk_overlap_tokens INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS documents (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source_kind  TEXT    NOT NULL,
    source_path  TEXT    NOT NULL,
    sha256       TEXT    NOT NULL,
    title        TEXT,
    bytes        INTEGER NOT NULL,
    indexed_at   INTEGER NOT NULL,
    UNIQUE(source_kind, source_path)
);

CREATE TABLE IF NOT EXISTS chunks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id  INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    ord          INTEGER NOT NULL,
    text         TEXT    NOT NULL,
    start_byte   INTEGER NOT NULL,
    end_byte     INTEGER NOT NULL,
    token_count  INTEGER NOT NULL,
    embedding    BLOB    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chunks_document ON chunks(document_id, ord);

-- FTS5 индекс над текстом чанков. content='chunks' / content_rowid='id'
-- даёт «contentless-external» — самим текстом FTS не дублирует, а синхро-
-- низирует через триггеры.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts
USING fts5(text, content='chunks', content_rowid='id', tokenize='unicode61 remove_diacritics 2');

CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.id, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> CollectionMeta {
        CollectionMeta {
            id: "abcd1234".into(),
            name: "Test".into(),
            created_at: 1700000000,
            embedding_model: "bge-m3".into(),
            embedding_dim: 4,
            chunk_target_tokens: 512,
            chunk_overlap_tokens: 64,
            document_count: 0,
            chunk_count: 0,
        }
    }

    fn sample_doc() -> DocumentRow {
        DocumentRow {
            id: None,
            source_kind: "file".into(),
            source_path: "/tmp/a.md".into(),
            sha256: "abc".into(),
            title: Some("A".into()),
            bytes: 100,
            indexed_at: 1700000001,
        }
    }

    fn sample_chunks() -> Vec<ChunkRow> {
        vec![
            ChunkRow {
                ord: 0,
                text: "hello world rust".into(),
                start_byte: 0,
                end_byte: 16,
                token_count: 3,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
            },
            ChunkRow {
                ord: 1,
                text: "tokio async runtime".into(),
                start_byte: 16,
                end_byte: 36,
                token_count: 3,
                embedding: vec![0.0, 1.0, 0.0, 0.0],
            },
            ChunkRow {
                ord: 2,
                text: "rust performance benchmark".into(),
                start_byte: 36,
                end_byte: 62,
                token_count: 3,
                embedding: vec![std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0],
            },
        ]
    }

    #[test]
    fn schema_round_trip() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let read = s.read_meta().unwrap();
        assert_eq!(read.id, "abcd1234");
        assert_eq!(read.embedding_dim, 4);
    }

    #[test]
    fn document_dedup_by_sha() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let (id1, fresh1) = s.upsert_document(&sample_doc()).unwrap();
        assert!(fresh1);
        let (id2, fresh2) = s.upsert_document(&sample_doc()).unwrap();
        assert_eq!(id1, id2);
        assert!(!fresh2, "повторный sha256 → fresh=false");
    }

    #[test]
    fn write_document_is_all_or_nothing_and_status_sees_empty_doc() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();

        // Строка документа без чанков — так выглядит оборванная индексация.
        let (_, _) = s.upsert_document(&sample_doc()).unwrap();
        let st = s.document_status("file", "/tmp/a.md").unwrap().unwrap();
        assert_eq!(st.sha256, "abc");
        assert_eq!(st.chunk_count, 0, "по такому документу искать нечем");

        // Запись документа вместе с чанками — одной транзакцией.
        s.write_document(&sample_doc(), &sample_chunks(), 4).unwrap();
        let st = s.document_status("file", "/tmp/a.md").unwrap().unwrap();
        assert_eq!(st.chunk_count, 3);
        assert_eq!(s.stats().unwrap().document_count, 1, "документ не задвоился");

        // Сбой на чанке не должен оставить документ в половинчатом виде.
        let mut broken = sample_chunks();
        broken[1].embedding = vec![0.0; 3];
        let doc = DocumentRow {
            sha256: "def".into(),
            ..sample_doc()
        };
        assert!(s.write_document(&doc, &broken, 4).is_err());
        let st = s.document_status("file", "/tmp/a.md").unwrap().unwrap();
        assert_eq!(st.sha256, "abc", "старая версия документа на месте");
        assert_eq!(st.chunk_count, 3, "чанки не потерялись");

        // Повторная запись того же пути заменяет чанки, а не добавляет.
        s.write_document(&doc, &sample_chunks()[..2], 4).unwrap();
        let st = s.document_status("file", "/tmp/a.md").unwrap().unwrap();
        assert_eq!(st.sha256, "def");
        assert_eq!(st.chunk_count, 2);
        let info = s.list_documents_with_counts().unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].chunk_count, 2);
    }

    #[test]
    fn fts_search_finds_term() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let (doc_id, _) = s.upsert_document(&sample_doc()).unwrap();
        s.insert_chunks(doc_id, &sample_chunks(), 4).unwrap();

        let hits = s.fts_search("tokio", 5).unwrap();
        assert_eq!(hits.len(), 1);
        // Этот chunk имеет ord=1 → id=2 (auto-increment).
        assert_eq!(hits[0].0, 2);
    }

    #[test]
    fn vector_search_ranks_by_cosine() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let (doc_id, _) = s.upsert_document(&sample_doc()).unwrap();
        s.insert_chunks(doc_id, &sample_chunks(), 4).unwrap();

        // Query вектор = первая ось → ближе всего chunks[0] = [1,0,0,0]
        // (cos=1.0), затем chunks[2] = [0.7071, 0.7071, …] (cos≈0.7071),
        // затем chunks[1] = [0,1,0,0] (cos=0.0).
        let hits = s.vector_search(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].0, 1, "первый — ось X");
        assert_eq!(hits[1].0, 3, "второй — диагональ");
        assert!(hits[0].1 > hits[1].1);
        assert!(hits[1].1 > hits[2].1);
    }

    #[test]
    fn delete_document_cascades_chunks() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let (doc_id, _) = s.upsert_document(&sample_doc()).unwrap();
        s.insert_chunks(doc_id, &sample_chunks(), 4).unwrap();
        s.delete_document(doc_id).unwrap();
        let stats = s.stats().unwrap();
        assert_eq!(stats.document_count, 0);
        assert_eq!(stats.chunk_count, 0);
    }

    #[test]
    fn dim_mismatch_errors_out() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&sample_meta()).unwrap();
        let (doc_id, _) = s.upsert_document(&sample_doc()).unwrap();
        let bad = vec![ChunkRow {
            ord: 0,
            text: "x".into(),
            start_byte: 0,
            end_byte: 1,
            token_count: 1,
            embedding: vec![1.0, 0.0], // ожидается dim=4
        }];
        let r = s.insert_chunks(doc_id, &bad, 4);
        assert!(matches!(r, Err(StoreError::DimMismatch { .. })));
    }

    #[test]
    fn vec_to_bytes_roundtrip() {
        let v = vec![0.1f32, 0.2, -0.3, 0.0];
        let b = vec_f32_to_le_bytes(&v);
        assert_eq!(b.len(), 16);
        let back = le_bytes_to_vec_f32(&b);
        assert_eq!(back.len(), 4);
        for i in 0..4 {
            assert!((back[i] - v[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn sanitize_fts_query_handles_punctuation() {
        // Двоеточие и круглые скобки — спецсимволы FTS5; должны быть скрыты.
        let q = sanitize_fts_query("foo: bar(baz)");
        assert!(q.contains("\"foo:\""), "{q}");
        assert!(q.contains("\"bar(baz)\""), "{q}");
        assert!(q.contains(" OR "), "{q}");
    }

    #[test]
    fn sanitize_fts_query_empty() {
        assert_eq!(sanitize_fts_query(""), "\"\"");
        assert_eq!(sanitize_fts_query("   "), "\"\"");
    }
}
