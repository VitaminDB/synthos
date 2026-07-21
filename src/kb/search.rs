//! Гибридный поиск по коллекции: BM25 (FTS5) ⊕ cosine (vector) через
//! Reciprocal Rank Fusion.
//!
//! RRF не требует калибровки скоров между источниками — складывает
//! `1/(k+rank)` (k=60 — литературный стандарт). Это на практике даёт
//! качественнее, чем нормализация скоров и линейная комбинация, особенно
//! когда BM25 даёт «бесконечность» на редких терминах.

use std::collections::HashMap;

use synaptix::facade::embedding::Embedder;
use synaptix::facade::rerank::Reranker;

use super::store::{ChunkWithDoc, Store, StoreError};

/// Константа в знаменателе RRF. `k=60` — рекомендация авторов
/// (Cormack et al, 2009). Значения 30..100 дают близкие результаты;
/// 60 — золотая середина для top-K с K ≤ 20.
pub const RRF_K: f32 = 60.0;

/// Дефолтный множитель over-fetch'а перед опциональным reranking'ом:
/// hybrid берёт `top_k * REREANK_MULTIPLIER` кандидатов из RRF,
/// cross-encoder сортирует их в финальные top_k. См.
/// [`hybrid_search_with_rerank`].
pub const DEFAULT_RERANK_MULTIPLIER: usize = 4;

/// Один результат поиска для отдачи в UI / LLM.
///
/// `PartialEq` нужен для `RwSignal<Vec<SearchHit>>::set` — сигналы пропускают
/// «set без изменений». Используем агрегированные поля (id чанка + score)
/// — это однозначно идентифицирует результат.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub chunk: ChunkWithDoc,
    /// Финальный score (выше = лучше). При активном реранкинге это
    /// rerank-score, иначе — RRF fused score.
    pub score: f32,
    /// Score только от BM25 (если был в FTS topK).
    pub bm25: Option<f32>,
    /// Score только от cosine (если был в vector topK).
    pub cosine: Option<f32>,
    /// Ранг в FTS (1-based), если был.
    pub bm25_rank: Option<u32>,
    /// Ранг в vector (1-based), если был.
    pub cosine_rank: Option<u32>,
    /// Score от cross-encoder реранкера; `Some(..)` означает, что прошёл
    /// rerank-фазу. При отсутствии реранкера всегда `None`.
    pub rerank_score: Option<f32>,
}

impl PartialEq for SearchHit {
    fn eq(&self, other: &Self) -> bool {
        self.chunk.id == other.chunk.id && self.score.to_bits() == other.score.to_bits()
    }
}

/// Hybrid search без реранкинга — обёртка над
/// [`hybrid_search_with_rerank`] с `reranker = None`. Оставлена для
/// обратной совместимости с местами, где явный pipeline не нужен.
pub fn hybrid_search(
    store: &Store,
    embedder: &dyn Embedder,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    hybrid_search_with_rerank(store, embedder, None, query, top_k, DEFAULT_RERANK_MULTIPLIER)
}

/// Полная версия hybrid_search с опциональным cross-encoder реранкингом.
///
/// `rerank_multiplier` — множитель over-fetch'а перед реранкингом:
/// hybrid собирает `top_k × multiplier` кандидатов RRF, cross-encoder
/// сортирует и оставляет `top_k`. При `reranker = None` over-fetch равен
/// `top_k × 4` (дефолт для RRF — не влияет на исход).
pub fn hybrid_search_with_rerank(
    store: &Store,
    embedder: &dyn Embedder,
    reranker: Option<&(dyn Reranker + Send + Sync)>,
    query: &str,
    top_k: usize,
    rerank_multiplier: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    if top_k == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let multiplier = if reranker.is_some() {
        rerank_multiplier.max(1)
    } else {
        4
    };
    let over_fetch = top_k.saturating_mul(multiplier).max(top_k);

    // 1. BM25.
    let bm25_hits = store.fts_search(query, over_fetch)?;
    // 2. Cosine.
    let q_emb = embedder
        .encode_query(query)
        .map_err(|e| SearchError::Embed(e.to_string()))?;
    let cos_hits = store.vector_search(&q_emb, over_fetch)?;

    // 3. RRF.
    let mut acc: HashMap<i64, FusedScore> = HashMap::new();
    for (rank, (id, score)) in bm25_hits.iter().enumerate() {
        let entry = acc.entry(*id).or_default();
        entry.score += 1.0 / (RRF_K + rank as f32 + 1.0);
        entry.bm25 = Some(*score);
        entry.bm25_rank = Some(rank as u32 + 1);
    }
    for (rank, (id, score)) in cos_hits.iter().enumerate() {
        let entry = acc.entry(*id).or_default();
        entry.score += 1.0 / (RRF_K + rank as f32 + 1.0);
        entry.cosine = Some(*score);
        entry.cosine_rank = Some(rank as u32 + 1);
    }

    // 4. Берём кандидатов для дальнейшей обработки.
    //    Без реранкинга — сразу top_k; с реранкингом — over_fetch (всё, что RRF собрал).
    let mut ranked: Vec<(i64, FusedScore)> = acc.into_iter().collect();
    ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
    let candidates_n = if reranker.is_some() {
        ranked.len().min(over_fetch)
    } else {
        top_k.min(ranked.len())
    };
    ranked.truncate(candidates_n);

    // 5. Поднимаем тексты чанков из БД (для реранкинга нужны тексты).
    let ids: Vec<i64> = ranked.iter().map(|(id, _)| *id).collect();
    let chunks = store.get_chunks_by_ids(&ids)?;
    let by_id: HashMap<i64, ChunkWithDoc> = chunks.into_iter().map(|c| (c.id, c)).collect();
    let mut hits = Vec::with_capacity(ranked.len());
    for (id, fs) in ranked {
        if let Some(chunk) = by_id.get(&id).cloned() {
            hits.push(SearchHit {
                chunk,
                score: fs.score,
                bm25: fs.bm25,
                cosine: fs.cosine,
                bm25_rank: fs.bm25_rank,
                cosine_rank: fs.cosine_rank,
                rerank_score: None,
            });
        }
    }

    // 6. Опциональный cross-encoder rerank над кандидатами.
    if let Some(r) = reranker {
        if !hits.is_empty() {
            let docs: Vec<&str> = hits.iter().map(|h| h.chunk.text.as_str()).collect();
            match r.rerank(query, &docs, hits.len()) {
                Ok(order) => {
                    let mut reranked: Vec<SearchHit> = Vec::with_capacity(order.len());
                    for (idx, score) in order {
                        if let Some(h) = hits.get(idx).cloned() {
                            let mut h = h;
                            h.rerank_score = Some(score);
                            h.score = score;
                            reranked.push(h);
                        }
                    }
                    hits = reranked;
                }
                Err(e) => {
                    // Не паникуем: реранкер «упал» → отдаём RRF top-K без
                    // post-фазы. Это симметрично деградации vector_search
                    // при отсутствии sqlite-vec.
                    log::warn!("kb::search: rerank упал, отдаём RRF top-K: {e}");
                }
            }
        }
    }

    hits.truncate(top_k);
    Ok(hits)
}

#[derive(Default)]
struct FusedScore {
    score: f32,
    bm25: Option<f32>,
    cosine: Option<f32>,
    bm25_rank: Option<u32>,
    cosine_rank: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("ошибка эмбеддера: {0}")]
    Embed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::collection::CollectionMeta;
    use crate::kb::store::{ChunkRow, DocumentRow};

    /// Stub-embedder для тестов: рандомный детерминированный по входу.
    struct StubEmbedder {
        dim: usize,
    }
    impl Embedder for StubEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn max_tokens(&self) -> usize {
            128
        }
        fn encode(
            &self,
            texts: &[&str],
        ) -> synaptix::facade::embedding::EmbeddingResult<Vec<Vec<f32>>> {
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                let mut v = vec![0f32; self.dim];
                let mut h = 0u64;
                for b in t.bytes() {
                    h = h.wrapping_mul(31).wrapping_add(b as u64);
                }
                for i in 0..self.dim {
                    let x = ((h.wrapping_add(i as u64)) % 1000) as f32 / 1000.0;
                    v[i] = x - 0.5;
                }
                let n = synaptix::facade::embedding::l2_norm(&v).max(1e-12);
                for x in v.iter_mut() {
                    *x /= n;
                }
                out.push(v);
            }
            Ok(out)
        }
    }

    fn build_store() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_schema().unwrap();
        s.upsert_collection(&CollectionMeta {
            id: "x".into(),
            name: "x".into(),
            created_at: 0,
            embedding_model: "bge-m3".into(),
            embedding_dim: 4,
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
                title: Some("D".into()),
                bytes: 100,
                indexed_at: 0,
            })
            .unwrap();
        s.insert_chunks(
            doc_id,
            &[
                ChunkRow {
                    ord: 0,
                    text: "rust async tokio".into(),
                    start_byte: 0,
                    end_byte: 16,
                    token_count: 3,
                    embedding: vec![1.0, 0.0, 0.0, 0.0],
                },
                ChunkRow {
                    ord: 1,
                    text: "python pandas pdf".into(),
                    start_byte: 17,
                    end_byte: 34,
                    token_count: 3,
                    embedding: vec![0.0, 1.0, 0.0, 0.0],
                },
            ],
            4,
        )
        .unwrap();
        s
    }

    #[test]
    fn hybrid_returns_topk_at_least_from_one_source() {
        let s = build_store();
        let emb = StubEmbedder { dim: 4 };
        let hits = hybrid_search(&s, &emb, "tokio", 2).unwrap();
        // BM25 должен поймать "tokio" — chunk 1.
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.chunk.text.contains("tokio")));
        assert!(hits.iter().all(|h| h.score > 0.0));
    }

    #[test]
    fn hybrid_empty_query_returns_nothing() {
        let s = build_store();
        let emb = StubEmbedder { dim: 4 };
        let hits = hybrid_search(&s, &emb, "", 5).unwrap();
        assert!(hits.is_empty());
    }

    /// Stub-реранкер: даёт высокий score документам, чей текст начинается
    /// на букву `p` (python pandas pdf), и низкий — остальным. Цель —
    /// проверить, что rerank-фаза реально переупорядочивает результат
    /// относительно RRF.
    struct PrefixPRerankerStub;
    impl synaptix::facade::rerank::Reranker for PrefixPRerankerStub {
        fn max_tokens(&self) -> usize {
            128
        }
        fn rerank(
            &self,
            _query: &str,
            docs: &[&str],
            top_k: usize,
        ) -> synaptix::facade::rerank::RerankResult<Vec<(usize, f32)>> {
            let mut scored: Vec<(usize, f32)> = docs
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let s = if d.trim_start().starts_with('p') { 0.99 } else { 0.01 };
                    (i, s)
                })
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(top_k);
            Ok(scored)
        }
        fn score_pairs(
            &self,
            pairs: &[(&str, &str)],
        ) -> synaptix::facade::rerank::RerankResult<Vec<f32>> {
            Ok(pairs
                .iter()
                .map(|(_, d)| if d.trim_start().starts_with('p') { 0.99 } else { 0.01 })
                .collect())
        }
    }

    #[test]
    fn rerank_reorders_topk_and_marks_score() {
        let s = build_store();
        let emb = StubEmbedder { dim: 4 };
        // Без реранкера BM25 ловит «tokio» → chunk «rust async tokio»
        // сильнее «python pandas pdf».
        let plain = hybrid_search(&s, &emb, "tokio", 2).unwrap();
        assert!(plain[0].chunk.text.starts_with("rust"));
        assert!(plain[0].rerank_score.is_none());

        // С реранкером, который любит `p…`-документы, порядок инвертируется.
        let r = PrefixPRerankerStub;
        let reranked = hybrid_search_with_rerank(
            &s,
            &emb,
            Some(&r),
            "tokio",
            2,
            DEFAULT_RERANK_MULTIPLIER,
        )
        .unwrap();
        assert_eq!(reranked.len(), 2);
        assert!(reranked[0].chunk.text.starts_with("python"));
        assert!(reranked[0].rerank_score.is_some());
        assert!((reranked[0].score - 0.99).abs() < 1e-5);
    }
}
