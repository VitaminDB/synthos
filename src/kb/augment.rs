//! Auto-augment system prompt'а перед agent-turn'ом.
//!
//! Вызывается из `chat::session::start_agent_turn` как async pre-step:
//! snapshot активных коллекций + эмбеддера на main-потоке (через
//! `run_on_main_thread` + `oneshot`), затем `tokio::task::spawn_blocking`
//! для SQLite I/O и cosine-вычислений. Результат — готовый текстовый
//! блок, который `build_history_from_slice_at` подмешивает в system
//! prompt между ACTION_RULE и user_system_prompt.
//!
//! Контракт: безопасен относительно abort (caller сверяет abort_snapshot
//! до и после `compute`); идемпотентен (одни и те же входы → одинаковый
//! выход); не паникует (все StoreError/EmbedError превращаются в Err);
//! при отсутствии хитов возвращает пустой текст без ошибки.
//!
//! См. также `chat::tools::kb_search` — основной канал интеграции с
//! LLM через tool calls. Augment отличается тем, что подмешивается
//! автоматически без явного вызова модели.

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;

use crate::context::AppCtx;
use crate::kb::search::{hybrid_search_with_rerank, SearchHit, DEFAULT_RERANK_MULTIPLIER};

/// Ошибка augment-шага. Все варианты — degradable: caller игнорирует
/// результат и идёт без RAG-блока. См. `user_message()` для текста,
/// который стоит показать в `notifications.warning`.
#[derive(Debug, thiserror::Error)]
pub enum AugmentError {
    /// Эмбеддер не удалось поднять; внутри — готовый текст причины
    /// (файл не найден / ошибка загрузки).
    #[error("эмбеддер недоступен: {0}")]
    NoEmbedder(String),
    #[error("нет активных коллекций в чате")]
    NoCollections,
    #[error("spawn_blocking panic: {0}")]
    Spawn(String),
}

impl AugmentError {
    /// Текст для `notifications.warning` или `None`, если уведомление
    /// не нужно (например, отсутствие коллекций пользователь и сам видит
    /// по KB-чипу в input-панели).
    pub fn user_message(&self) -> Option<String> {
        match self {
            AugmentError::NoEmbedder(why) => {
                Some(tr!("kb.augment.no_embedder_warning", error = why))
            }
            AugmentError::NoCollections => None,
            AugmentError::Spawn(_) => Some(tr!("kb.augment.spawn_panic_warning")),
        }
    }
}

/// Снимок данных, нужных worker'у. Берётся на main-потоке, использует
/// blocking worker — `use_context` доступен только на main.
struct AugmentSnapshot {
    kb: crate::kb::KbCtx,
    /// Конфиг и каталоги поиска — модели грузятся по первой надобности.
    plan: crate::kb::loader::LoadPlan,
    /// `(id, name)` активных коллекций в порядке `active_in_chat_ids`.
    /// `name` нужен для подписи источника в формате блока.
    collections: Vec<(String, String)>,
    kb_dir: std::path::PathBuf,
}

/// Главный entrypoint: query (последний user-Text) → готовый блок текста
/// или Err'a. Пустой query → `Ok("")` (нечего искать).
///
/// `top_k` — финальный размер top-K после объединения коллекций.
/// `token_budget` — мягкий лимит длины выходного текста (~4 char/token).
pub async fn compute(
    query: &str,
    top_k: usize,
    token_budget: usize,
) -> Result<String, AugmentError> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(String::new());
    }

    let snapshot = snapshot_from_main().await?;
    let embedder = crate::kb::loader::embedder_ready(&snapshot.kb, &snapshot.plan)
        .await
        .map_err(AugmentError::NoEmbedder)?;
    // Реранкер — улучшение, а не условие: не поднялся — ищем без него.
    let reranker = crate::kb::loader::reranker_ready(&snapshot.kb, &snapshot.plan)
        .await
        .unwrap_or_else(|e| {
            log::warn!("kb augment: reranker: {e}");
            None
        });

    let query_clone = q.to_string();
    let per_collection_k: usize = 5;
    let total_top_k = top_k.max(1);

    let hits = tokio::task::spawn_blocking(move || {
        merge_hits_across_collections(
            &snapshot.kb_dir,
            embedder.as_ref(),
            reranker.as_deref(),
            &snapshot.collections,
            &query_clone,
            per_collection_k,
            total_top_k,
        )
    })
    .await
    .map_err(|e| AugmentError::Spawn(e.to_string()))?;

    Ok(format_augment_text(&hits, token_budget))
}

async fn snapshot_from_main() -> Result<AugmentSnapshot, AugmentError> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<AugmentSnapshot, AugmentError>>();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        let kb = app.kb.clone();
        let active_ids = kb.active_collection_ids();
        if active_ids.is_empty() {
            let _ = tx.send(Err(AugmentError::NoCollections));
            return;
        }
        let registry = kb.registry.get_untracked();
        let collections: Vec<(String, String)> = active_ids
            .iter()
            .filter_map(|id| registry.get(id).map(|m| (m.id.clone(), m.name.clone())))
            .collect();
        if collections.is_empty() {
            // Все active_ids оказались stale (коллекции удалены физически).
            let _ = tx.send(Err(AugmentError::NoCollections));
            return;
        }
        let _ = tx.send(Ok(AugmentSnapshot {
            plan: crate::kb::loader::plan(),
            kb,
            collections,
            kb_dir: registry.kb_dir.clone(),
        }));
    });
    rx.await
        .map_err(|e| AugmentError::Spawn(e.to_string()))?
}

/// Открывает Store для каждой активной коллекции, делает hybrid_search
/// (per_collection_k результатов с коллекции), сливает в один Vec,
/// сортирует по убыванию score, режет до `total_top_k`. Возвращает
/// пары `(имя_коллекции, hit)` — имя нужно для подписи источника.
/// Не паникует при ошибке одной коллекции — пишет `log::warn!` и идёт
/// дальше (поведение симметрично `chat::tools::kb_search`).
fn merge_hits_across_collections(
    kb_dir: &std::path::Path,
    embedder: &dyn synaptix::facade::embedding::Embedder,
    reranker: Option<&(dyn synaptix::facade::rerank::Reranker + Send + Sync)>,
    collections: &[(String, String)],
    query: &str,
    per_collection_k: usize,
    total_top_k: usize,
) -> Vec<(String, SearchHit)> {
    let mut all: Vec<(String, SearchHit)> = Vec::new();
    for (id, name) in collections {
        let path = kb_dir.join(format!("{}.sqlite", id));
        let store = match crate::kb::store::Store::open(&path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("augment: open {}: {e}", path.display());
                continue;
            }
        };
        match hybrid_search_with_rerank(
            &store,
            embedder,
            reranker,
            query,
            per_collection_k,
            DEFAULT_RERANK_MULTIPLIER,
        ) {
            Ok(hits) => {
                for h in hits {
                    all.push((name.clone(), h));
                }
            }
            Err(e) => log::warn!("augment: hybrid {}: {e}", path.display()),
        }
    }
    all.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
    all.truncate(total_top_k);
    all
}

/// Формат augment-блока, который вставляется в system prompt:
/// ```text
/// === Контекст из базы знаний (источник для ответа, не пересказывай дословно) ===
///
/// [1] [Имя коллекции] Заголовок документа:
/// <snippet>
///
/// === Конец контекста ===
/// ```
/// Каждый chunk обрезается до `MAX_CHARS_PER_CHUNK`; общая длина
/// ограничивается `token_budget * 4` (грубая оценка 4 char/token).
/// Пустые `hits` → `""` (caller это интерпретирует как «augment пуст»).
fn format_augment_text(hits: &[(String, SearchHit)], token_budget: usize) -> String {
    if hits.is_empty() {
        return String::new();
    }
    const MAX_CHARS_PER_CHUNK: usize = 800;
    let total_budget = token_budget.saturating_mul(4).max(MAX_CHARS_PER_CHUNK);
    let header = "=== Контекст из базы знаний (источник для ответа, не пересказывай дословно) ===";
    let footer = "=== Конец контекста ===";

    let mut out = String::new();
    out.push_str(header);
    for (i, (col_name, h)) in hits.iter().enumerate() {
        // budget-cap: оставим место для footer + одной разделяющей строки.
        if out.len() + footer.len() + 4 > total_budget {
            break;
        }
        let title = h
            .chunk
            .doc_title
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&h.chunk.source_path);
        out.push_str(&format!("\n\n[{}] [{}] {}:\n", i + 1, col_name, title));
        let snippet: String = h.chunk.text.chars().take(MAX_CHARS_PER_CHUNK).collect();
        out.push_str(snippet.trim());
    }
    out.push_str("\n\n");
    out.push_str(footer);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::store::ChunkWithDoc;

    fn mk_hit(col_idx: i64, title: &str, text: &str, score: f32) -> (String, SearchHit) {
        let len = text.len();
        (
            "Test KB".into(),
            SearchHit {
                chunk: ChunkWithDoc {
                    id: col_idx,
                    document_id: col_idx,
                    ord: 0,
                    text: text.into(),
                    start_byte: 0,
                    end_byte: len,
                    token_count: 0,
                    doc_title: Some(title.into()),
                    source_path: format!("{title}.md"),
                    source_kind: "file".into(),
                },
                score,
                bm25: None,
                cosine: None,
                bm25_rank: None,
                cosine_rank: None,
                rerank_score: None,
            },
        )
    }

    #[test]
    fn empty_hits_returns_empty() {
        assert!(format_augment_text(&[], 1500).is_empty());
    }

    #[test]
    fn format_contains_header_footer_and_snippet() {
        let hits = vec![mk_hit(1, "doc-A", "квантовая запутанность связывает частицы", 0.9)];
        let s = format_augment_text(&hits, 1500);
        assert!(s.contains("Контекст из базы знаний"));
        assert!(s.contains("Конец контекста"));
        assert!(s.contains("doc-A"));
        assert!(s.contains("Test KB"));
        assert!(s.contains("квантовая запутанность"));
    }

    #[test]
    fn snippet_is_truncated_to_max_chars_per_chunk() {
        let long: String = "x".repeat(2000);
        let hits = vec![mk_hit(1, "long", &long, 1.0)];
        let s = format_augment_text(&hits, 1500);
        // 2000 char'ов «x» должны быть обрезаны до 800.
        let xs = s.chars().filter(|c| *c == 'x').count();
        assert!(xs <= 800, "snippet not truncated: {xs} xs");
        assert!(xs >= 700, "snippet too short: {xs} xs");
    }

    #[test]
    fn budget_caps_total_length() {
        // 10 хитов по 800 символов каждый → 8000 char суммарно, но при
        // token_budget=200 (=800 char-budget) после первого чанка стоп.
        let body = "y".repeat(900);
        let hits: Vec<(String, SearchHit)> = (0..10)
            .map(|i| mk_hit(i, &format!("d{i}"), &body, 1.0 - i as f32 * 0.01))
            .collect();
        let s = format_augment_text(&hits, 200);
        // Должен быть как минимум один чанк и не больше двух (с учётом
        // того, что MAX_CHARS_PER_CHUNK=800 — нижняя граница budget'а).
        let count = s.matches("] [Test KB]").count();
        assert!(count >= 1 && count <= 2, "expected 1-2 chunks, got {count}");
        assert!(s.contains("Конец контекста"));
    }

    #[test]
    fn utf8_truncation_is_char_safe() {
        // Длинная UTF-8 строка с многобайтовыми символами. .chars().take(N)
        // обязан резать по char-boundary — иначе String::from паникует.
        let body: String = "ё".repeat(2000);
        let hits = vec![mk_hit(1, "utf8", &body, 1.0)];
        let s = format_augment_text(&hits, 1500);
        // Проверяем, что строка валидна UTF-8 (она уже String, так что это
        // тавтология; проверяем, что число ё-символов не превышает 800).
        let cnt = s.chars().filter(|c| *c == 'ё').count();
        assert!(cnt <= 800, "char-count over MAX: {cnt}");
        assert!(cnt > 0, "no ё's at all: {s}");
    }

    #[test]
    fn user_message_for_no_embedder_is_actionable() {
        let m = AugmentError::NoEmbedder("нет файла".into()).user_message().unwrap();
        assert_eq!(m, tr!("kb.augment.no_embedder_warning", error = "нет файла"));
    }

    #[test]
    fn user_message_for_no_collections_is_silent() {
        assert!(AugmentError::NoCollections.user_message().is_none());
    }
}
