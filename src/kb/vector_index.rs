//! Векторный индекс KB — оболочка над опциональной `sqlite-vec` (vec0
//! virtual table) с прозрачным fallback'ом на full-scan по `chunks.embedding`.
//!
//! ### Зачем
//!
//! Базовый `Store::vector_search` делает полный скан таблицы `chunks` и
//! считает cosine в Rust. Это работает быстро (<50ms) до 50–100k чанков,
//! но линейно деградирует на больших коллекциях. `sqlite-vec` (vec0) даёт
//! ANN-/exact-KNN над `FLOAT[N]` колонкой за O(log N) на одном уровне SQL.
//!
//! ### Архитектура
//!
//! - `chunks_vec` — `VIRTUAL TABLE … USING vec0(chunk_id INTEGER PRIMARY KEY, embedding FLOAT[<dim>])`.
//! - `chunk_id` совпадает с `chunks.id` (один-к-одному).
//! - Embedding хранится в формате `f32[N]` LE — тот же байт-в-байт BLOB, что
//!   уже лежит в `chunks.embedding`, так что копирование данных не нужно.
//! - Дистанция в vec0 — L2² по умолчанию; для L2-нормализованных
//!   эмбеддингов `cosine = 1 − L2²/2`, и мы конвертируем при выдаче
//!   наружу, чтобы интерфейс (`Vec<(i64, f32)>` с «больше = лучше»)
//!   совпадал с тем, что возвращает `Store::vector_search` сегодня.
//!
//! ### Режимы
//!
//! - [`VectorIndexKind::Scan`] — старый full-scan, всегда доступен.
//! - [`VectorIndexKind::SqliteVec`] — vec0 KNN; требует `kb-sqlite-vec`
//!   и зарегистрированной auto-extension.
//! - [`VectorIndexKind::Auto`] — выбирается в момент [`VectorIndex::open`]:
//!   `SqliteVec`, если расширение зарегистрировано **и** `chunks` уже
//!   превысил `threshold`; иначе `Scan`.
//!
//! Регистрация sqlite-vec — глобальная (через `sqlite3_auto_extension`),
//! делается из `synthos::main`. См. [`store::init_sqlite_vec`](super::store::init_sqlite_vec).

use rusqlite::{params, Connection};

use super::store::StoreError;

/// Способ выполнения vector-search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorIndexKind {
    /// Полный скан таблицы chunks + cosine в Rust.
    Scan,
    /// vec0 virtual table из sqlite-vec.
    SqliteVec,
    /// Авто-выбор: SqliteVec при доступной extension и достаточном размере
    /// коллекции, иначе Scan.
    Auto,
}

impl VectorIndexKind {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "scan" => Self::Scan,
            "sqlite-vec" | "sqlite_vec" | "vec0" => Self::SqliteVec,
            _ => Self::Auto,
        }
    }
}

/// Векторный индекс одной коллекции. Конкретный режим выбран в момент
/// [`VectorIndex::open`] и далее не меняется в течение жизни значения.
#[derive(Debug, Clone, Copy)]
pub struct VectorIndex {
    kind: VectorIndexKind,
    dim: usize,
}

impl VectorIndex {
    /// Доступна ли auto-extension `sqlite-vec` в этом процессе.
    /// `true` — если `init_sqlite_vec()` был вызван хотя бы один раз и
    /// собрана фича `kb-sqlite-vec`.
    pub fn sqlite_vec_available() -> bool {
        super::store::is_sqlite_vec_registered()
    }

    /// Открыть индекс для уже подключённой БД. Решает финальный режим
    /// (для Auto) и при необходимости создаёт схему `chunks_vec`.
    ///
    /// `threshold` — порог числа чанков, начиная с которого Auto
    /// переключается на SqliteVec.
    pub fn open(
        conn: &Connection,
        dim: usize,
        kind: VectorIndexKind,
        threshold: usize,
    ) -> Result<Self, StoreError> {
        if dim == 0 {
            return Ok(Self {
                kind: VectorIndexKind::Scan,
                dim,
            });
        }
        let effective = match kind {
            VectorIndexKind::Scan => VectorIndexKind::Scan,
            VectorIndexKind::SqliteVec if Self::sqlite_vec_available() => {
                VectorIndexKind::SqliteVec
            }
            VectorIndexKind::SqliteVec => {
                log::warn!(
                    "kb: kind=sqlite-vec, но extension не зарегистрирован — fallback на scan"
                );
                VectorIndexKind::Scan
            }
            VectorIndexKind::Auto => {
                if !Self::sqlite_vec_available() {
                    VectorIndexKind::Scan
                } else {
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
                        .unwrap_or(0);
                    if (count as usize) >= threshold {
                        VectorIndexKind::SqliteVec
                    } else {
                        VectorIndexKind::Scan
                    }
                }
            }
        };
        if effective == VectorIndexKind::SqliteVec {
            Self::ensure_schema(conn, dim)?;
        }
        Ok(Self {
            kind: effective,
            dim,
        })
    }

    pub fn kind(&self) -> VectorIndexKind {
        self.kind
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn is_sqlite_vec(&self) -> bool {
        matches!(self.kind, VectorIndexKind::SqliteVec)
    }

    /// Идемпотентно создать `chunks_vec` под нужный `dim`. Дополнительно
    /// вешает AFTER DELETE триггер `chunks_vec_ad`, чтобы каскадное
    /// удаление chunks (например по ON DELETE CASCADE с documents)
    /// синхронно сносило строки и в vec0.
    pub fn ensure_schema(conn: &Connection, dim: usize) -> Result<(), StoreError> {
        if dim == 0 {
            return Ok(());
        }
        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec \
             USING vec0(chunk_id INTEGER PRIMARY KEY, embedding FLOAT[{dim}]);"
        );
        conn.execute_batch(&sql)?;
        // Триггер: удаление chunks → удаление в vec0. На update embedding
        // полагаемся на явный delete+insert из кода (insert_chunks делает
        // полный re-create через ON DELETE CASCADE documents → chunks).
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS chunks_vec_ad AFTER DELETE ON chunks BEGIN \
                 DELETE FROM chunks_vec WHERE chunk_id = old.id; \
             END;",
        )?;
        Ok(())
    }

    /// Вставить одну запись. Принимает уже подготовленный LE-байтовый
    /// буфер f32[dim] — тот же формат, что лежит в `chunks.embedding`.
    pub fn insert_bytes(
        &self,
        conn: &Connection,
        chunk_id: i64,
        emb_bytes: &[u8],
    ) -> Result<(), StoreError> {
        if !self.is_sqlite_vec() {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO chunks_vec(chunk_id, embedding) VALUES (?1, ?2)",
            params![chunk_id, emb_bytes],
        )?;
        Ok(())
    }

    /// Удалить запись по `chunk_id`. Триггер `chunks_vec_ad` подхватит то же
    /// при DELETE из таблицы `chunks`; этот метод нужен для явных
    /// очисток (например в тестах).
    pub fn delete(&self, conn: &Connection, chunk_id: i64) -> Result<(), StoreError> {
        if !self.is_sqlite_vec() {
            return Ok(());
        }
        conn.execute(
            "DELETE FROM chunks_vec WHERE chunk_id = ?1",
            params![chunk_id],
        )?;
        Ok(())
    }

    /// KNN-поиск через vec0. Возвращает `(chunk_id, cosine_score)`,
    /// `cosine = 1 − L2²/2` (см. docstring модуля).
    ///
    /// При `kind != SqliteVec` возвращает `None` — caller должен
    /// fallback'нуться на [`Store::vector_search`](super::store::Store::vector_search).
    pub fn knn(
        &self,
        conn: &Connection,
        query: &[f32],
        top_k: usize,
    ) -> Result<Option<Vec<(i64, f32)>>, StoreError> {
        if !self.is_sqlite_vec() || query.is_empty() || top_k == 0 {
            return Ok(None);
        }
        if query.len() != self.dim {
            return Err(StoreError::DimMismatch {
                expected: self.dim,
                got: query.len(),
            });
        }
        let bytes = vec_f32_to_le_bytes(query);
        let mut stmt = conn.prepare(
            "SELECT chunk_id, distance FROM chunks_vec \
             WHERE embedding MATCH ?1 AND k = ?2 \
             ORDER BY distance",
        )?;
        let rows = stmt.query_map(params![bytes, top_k as i64], |r| {
            let id = r.get::<_, i64>(0)?;
            let d = r.get::<_, f64>(1)? as f32;
            // L2² для unit-векторов лежит в [0, 4]. cos = 1 - L2²/2 ∈ [-1, 1].
            // При неортогональных не-нормализованных эмбеддингах это просто
            // affine-преобразование distance; sort order сохраняется.
            let cos = 1.0 - d * 0.5;
            Ok((id, cos))
        })?;
        Ok(Some(rows.filter_map(Result::ok).collect()))
    }

    /// Перезалить все embedding'и из `chunks` в `chunks_vec`. Идемпотентно:
    /// вставляются только записи, которых ещё нет в vec0.
    /// Возвращает количество фактически добавленных строк.
    pub fn reindex_all(&self, conn: &Connection) -> Result<usize, StoreError> {
        if !self.is_sqlite_vec() {
            return Ok(0);
        }
        let mut stmt = conn.prepare(
            "SELECT c.id, c.embedding FROM chunks c \
             WHERE NOT EXISTS (SELECT 1 FROM chunks_vec v WHERE v.chunk_id = c.id)",
        )?;
        let mut insert = conn.prepare(
            "INSERT INTO chunks_vec(chunk_id, embedding) VALUES (?1, ?2)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut n = 0usize;
        for row in rows {
            let (id, blob) = row?;
            // На случай если в chunks залежался embedding не той размерности
            // (была миграция модели без полного reindex) — пропускаем.
            if blob.len() != self.dim * 4 {
                log::warn!(
                    "kb: пропускаю chunk_id={id} dim mismatch ({} vs {})",
                    blob.len() / 4,
                    self.dim
                );
                continue;
            }
            insert.execute(params![id, blob])?;
            n += 1;
        }
        Ok(n)
    }
}

fn vec_f32_to_le_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::collection::CollectionMeta;
    use crate::kb::store::{ChunkRow, DocumentRow, Store};

    /// Без зарегистрированной auto-extension Auto должен выбрать Scan,
    /// SqliteVec — деградировать в Scan с warn.
    #[test]
    fn open_falls_back_when_extension_unavailable() {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        let conn = s.conn();
        // ОК: тест не вызывает init_sqlite_vec(); кому-то другому в той же
        // тест-бинарке init мог быть вызван — на это не закладываемся.
        if !VectorIndex::sqlite_vec_available() {
            let auto = VectorIndex::open(conn, 4, VectorIndexKind::Auto, 0).unwrap();
            assert_eq!(auto.kind(), VectorIndexKind::Scan);
            let req = VectorIndex::open(conn, 4, VectorIndexKind::SqliteVec, 0).unwrap();
            assert_eq!(req.kind(), VectorIndexKind::Scan);
        }
    }

    #[cfg(feature = "kb-sqlite-vec")]
    mod with_extension {
        use super::*;

        fn ensure_init() {
            crate::kb::store::init_sqlite_vec();
        }

        fn build_store_with_chunks(n: usize, dim: usize) -> Store {
            ensure_init();
            let mut s = Store::open_in_memory().unwrap();
            s.ensure_schema().unwrap();
            s.upsert_collection(&CollectionMeta {
                id: "x".into(),
                name: "x".into(),
                created_at: 0,
                embedding_model: "test".into(),
                embedding_dim: dim as i32,
                chunk_target_tokens: 512,
                chunk_overlap_tokens: 64,
                document_count: 0,
                chunk_count: 0,
            })
            .unwrap();
            let (doc_id, _) = s
                .upsert_document(&DocumentRow {
                    id: None,
                    source_kind: "file".into(),
                    source_path: "/d.md".into(),
                    sha256: "x".into(),
                    title: None,
                    bytes: 0,
                    indexed_at: 0,
                })
                .unwrap();
            let mut rows = Vec::with_capacity(n);
            for i in 0..n {
                let mut v = vec![0f32; dim];
                v[i % dim] = 1.0;
                if dim > 1 {
                    v[(i + 1) % dim] = ((i % 7) as f32) * 0.1;
                }
                // нормализуем (для cosine = dot)
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                for x in v.iter_mut() {
                    *x /= norm;
                }
                rows.push(ChunkRow {
                    ord: i as i32,
                    text: format!("chunk #{i}"),
                    start_byte: 0,
                    end_byte: 1,
                    token_count: 1,
                    embedding: v,
                });
            }
            s.insert_chunks(doc_id, &rows, dim).unwrap();
            s
        }

        #[test]
        fn knn_matches_scan() {
            const N: usize = 64;
            const DIM: usize = 8;
            let s = build_store_with_chunks(N, DIM);
            let vi = VectorIndex::open(s.conn(), DIM, VectorIndexKind::SqliteVec, 0).unwrap();
            assert!(vi.is_sqlite_vec());
            // sync vec0 с уже залитыми chunks
            let reindexed = vi.reindex_all(s.conn()).unwrap();
            assert_eq!(reindexed, N);

            let mut q = vec![0f32; DIM];
            q[3] = 1.0;
            let scan = s.vector_search(&q, 5).unwrap();
            let knn = vi
                .knn(s.conn(), &q, 5)
                .unwrap()
                .expect("knn returns Some");
            assert_eq!(scan.len(), knn.len());
            // Тop-1 совпадает.
            assert_eq!(scan[0].0, knn[0].0);
        }

        #[test]
        fn reindex_idempotent() {
            const N: usize = 32;
            const DIM: usize = 8;
            let s = build_store_with_chunks(N, DIM);
            let vi = VectorIndex::open(s.conn(), DIM, VectorIndexKind::SqliteVec, 0).unwrap();
            let n1 = vi.reindex_all(s.conn()).unwrap();
            let n2 = vi.reindex_all(s.conn()).unwrap();
            assert_eq!(n1, N);
            assert_eq!(n2, 0, "повторный reindex ничего не добавляет");
        }

        #[test]
        fn delete_via_trigger() {
            const DIM: usize = 4;
            let s = build_store_with_chunks(3, DIM);
            let vi = VectorIndex::open(s.conn(), DIM, VectorIndexKind::SqliteVec, 0).unwrap();
            vi.reindex_all(s.conn()).unwrap();
            // Удаляем документ → CASCADE → chunks → trigger → chunks_vec
            let doc_id: i64 = s
                .conn()
                .query_row("SELECT id FROM documents LIMIT 1", [], |r| r.get(0))
                .unwrap();
            s.conn()
                .execute("DELETE FROM documents WHERE id=?1", params![doc_id])
                .unwrap();
            let remaining: i64 = s
                .conn()
                .query_row("SELECT COUNT(*) FROM chunks_vec", [], |r| r.get(0))
                .unwrap();
            assert_eq!(remaining, 0);
        }
    }
}
