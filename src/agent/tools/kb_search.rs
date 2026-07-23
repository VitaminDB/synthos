//! Tool `kb_search` — поиск по активным коллекциям knowledge base.
//!
//! Контракт: получает JSON `{ "query": string, "top_k"?: number }`,
//! возвращает текстовый envelope с топ-K фрагментами. Формат envelope
//! параллелен `web_read` — секционный plain-текст с `--- ... ---` разрывами,
//! читаемый и LLM, и пользователем в tool-bubble.
//!
//! Использует:
//! - `KbCtx::active_in_chat_ids` — какие коллекции опрашивать;
//! - `KbCtx::registry` — открытие .sqlite;
//! - `kb::search::hybrid_search` — собственно RRF-гибрид;
//! - `KbCtx::embedder` — lazy-загруженный эмбеддер. Если не загружен —
//!   возвращает понятную ошибку (без авто-загрузки: модель грузится
//!   секунды-минуты, tool-call синхронный).
//!
//! Не паникует. Все ошибки → `ToolError::*` или текст-error в envelope.

use syngui::context_provider::use_context;
use syngui::async_runtime::run_on_main_thread;

use crate::context::AppCtx;
use crate::kb::search::{hybrid_search_with_rerank, SearchHit, DEFAULT_RERANK_MULTIPLIER};

use super::executor::{ToolError, MAX_OUTPUT_BYTES};

/// Параметры из JSON-аргументов.
#[derive(Debug)]
struct Args {
    query: String,
    top_k: usize,
}

fn parse_args(args_json: &str) -> Result<Args, ToolError> {
    let v: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let query = v
        .get("query")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("query"))?
        .trim()
        .to_string();
    if query.is_empty() {
        return Err(ToolError::MissingField("query"));
    }
    let top_k = v
        .get("top_k")
        .and_then(|x| x.as_u64())
        .map(|x| x as usize)
        .unwrap_or(5)
        .clamp(1, 20);
    Ok(Args { query, top_k })
}

/// Snapshot-данные, собранные на main-потоке.
struct KbSnapshot {
    active_ids: Vec<String>,
    embedder: Option<std::sync::Arc<dyn synaptix::facade::embedding::Embedder + Send + Sync>>,
    reranker: Option<std::sync::Arc<dyn synaptix::facade::rerank::Reranker + Send + Sync>>,
    collections: Vec<(String, String)>,
    kb_dir: std::path::PathBuf,
}

/// Главный entrypoint, который вызывается из `executor::execute`.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let args = parse_args(args_json)?;

    // Собираем snapshot на main-потоке через канал — use_context доступен
    // только там. Сам поиск будет на worker-потоке через spawn_blocking.
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        let kb = app.kb.clone();
        let active_ids = kb.active_collection_ids();

        let snapshot = if active_ids.is_empty() {
            // Возвращаем пустой snapshot — результат решим по active_ids
            KbSnapshot {
                active_ids,
                embedder: None,
                reranker: None,
                collections: Vec::new(),
                kb_dir: kb.registry.get_untracked().kb_dir.clone(),
            }
        } else {
            let embedder = kb.get_embedder();
            if embedder.is_none() {
                KbSnapshot {
                    active_ids,
                    embedder: None,
                    reranker: None,
                    collections: Vec::new(),
                    kb_dir: kb.registry.get_untracked().kb_dir.clone(),
                }
            } else {
                let registry = kb.registry.get_untracked();
                let collections: Vec<(String, String)> = active_ids
                    .iter()
                    .filter_map(|id| registry.get(id).map(|m| (m.id.clone(), m.name.clone())))
                    .collect();
                KbSnapshot {
                    active_ids,
                    embedder,
                    reranker: kb.get_reranker(),
                    collections,
                    kb_dir: registry.kb_dir.clone(),
                }
            }
        };

        let _ = tx.send(snapshot);
    });

    let snapshot = rx.await.map_err(|e| ToolError::Spawn(e.to_string()))?;

    if snapshot.active_ids.is_empty() {
        return Ok(format_envelope(
            &args.query,
            &[],
            "Нет активных баз знаний для этого чата. Включи коллекцию через \
             chip над полем ввода (Settings → Базы знаний для управления).",
        ));
    }

    let embedder = match snapshot.embedder {
        Some(e) => e,
        None => {
            return Ok(format_envelope(
                &args.query,
                &[],
                "Эмбеддер не загружен. Settings → Базы знаний → \
                 «Загрузить модель эмбеддера» / убедись, что модель скачана \
                 в указанный каталог (по умолчанию ~/models/bge-m3).",
            ));
        }
    };

    let collections = snapshot.collections;
    if collections.is_empty() {
        return Ok(format_envelope(
            &args.query,
            &[],
            "Активные коллекции не найдены в registry (возможно, удалены).",
        ));
    }

    let kb_dir = snapshot.kb_dir;
    let reranker = snapshot.reranker;
    let query_for_worker = args.query.clone();
    let top_k = args.top_k;

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<SearchHit>, String> {
        let mut all_hits: Vec<SearchHit> = Vec::new();
        for (id, _name) in &collections {
            let path = kb_dir.join(format!("{}.sqlite", id));
            let store = match crate::kb::store::Store::open(&path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("kb_search: open {}: {e}", path.display());
                    continue;
                }
            };
            match hybrid_search_with_rerank(
                &store,
                embedder.as_ref(),
                reranker.as_deref(),
                &query_for_worker,
                top_k,
                DEFAULT_RERANK_MULTIPLIER,
            ) {
                Ok(mut h) => all_hits.append(&mut h),
                Err(e) => {
                    log::warn!("kb_search: hybrid {}: {e}", path.display());
                }
            }
        }
        // Сортируем все объединённые hits и режем до top_k.
        all_hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        all_hits.truncate(top_k);
        Ok(all_hits)
    })
    .await
    .map_err(|e| ToolError::Spawn(e.to_string()))?;

    let hits = match result {
        Ok(h) => h,
        Err(e) => {
            return Ok(format_envelope(
                &args.query,
                &[],
                &format!("ошибка поиска: {e}"),
            ));
        }
    };

    // Сохраняем для UI отладки (если хотим показать панель «последний поиск»).
    let hits_for_ui = hits.clone();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        app.kb.last_search_hits.set(hits_for_ui);
    });

    let body = format_hits_body(&hits);
    Ok(format_envelope(&args.query, &hits, &body))
}

fn format_envelope(query: &str, hits: &[SearchHit], body: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("kb_search query: {query}\n"));
    out.push_str(&format!("hits: {}\n", hits.len()));
    out.push_str("--- результаты ---\n");
    out.push_str(body.trim_end());
    out.push('\n');
    truncate_envelope(out)
}

fn format_hits_body(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "(ничего не найдено)".to_string();
    }
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        let title = h
            .chunk
            .doc_title
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&h.chunk.source_path);
        out.push_str(&format!(
            "[{}] source: {} ({}) | score: {:.4}",
            i + 1,
            title,
            h.chunk.source_path,
            h.score
        ));
        if let Some(b) = h.bm25_rank {
            out.push_str(&format!(" | bm25 #{b}"));
        }
        if let Some(c) = h.cosine_rank {
            out.push_str(&format!(" | vec #{c}"));
        }
        out.push('\n');
        // snippet: до 600 байт текста чанка, иначе склеим в одну строку.
        let snippet: String = h.chunk.text.chars().take(600).collect();
        let snippet = snippet.trim();
        out.push_str(snippet);
        out.push_str("\n\n");
    }
    out
}

/// Урезает результат до MAX_OUTPUT_BYTES по char-boundary.
fn truncate_envelope(s: String) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s;
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 32);
    out.push_str(&s[..end]);
    out.push_str("\n…(truncated)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_validates_query() {
        let a = parse_args(r#"{"query":"hello","top_k":3}"#).unwrap();
        assert_eq!(a.query, "hello");
        assert_eq!(a.top_k, 3);

        let a = parse_args(r#"{"query":"hello"}"#).unwrap();
        assert_eq!(a.top_k, 5, "default top_k");

        let a = parse_args(r#"{"query":"hi","top_k":99}"#).unwrap();
        assert_eq!(a.top_k, 20, "clamped to 20");

        assert!(parse_args(r#"{}"#).is_err());
        assert!(parse_args(r#"{"query":"  "}"#).is_err());
        assert!(parse_args(r#"not json"#).is_err());
    }

    #[test]
    fn format_envelope_no_hits() {
        let s = format_envelope("foo", &[], "(ничего не найдено)");
        assert!(s.contains("kb_search query: foo"));
        assert!(s.contains("hits: 0"));
        assert!(s.contains("(ничего не найдено)"));
    }
}
