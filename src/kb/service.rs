//! Модели базы знаний в службе synaptixd: эмбеддер и реранкер считает она
//! (устройство и точность — её настройки модели). Хранилище, BM25 и разбиение
//! на куски остаются здесь.

use std::path::Path;

use synaptix::facade::embedding::{Embedder, EmbeddingResult};
use synaptix::facade::rerank::{RerankResult, Reranker};

fn client() -> Result<synaptixd::Client, String> {
    synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))
}

/// Эмбеддер службы. `model` — путь к файлу модели (служба находит её в
/// библиотеке) или id.
pub struct ServiceEmbedder {
    model: String,
    dim: usize,
    max_tokens: usize,
}

impl ServiceEmbedder {
    /// Открыть: пробный вызов узнаёт размерность и предел длины.
    pub fn open(path: &Path) -> Result<Self, String> {
        let model = path.display().to_string();
        let probe = client()?.embed(&model, &["ping".to_string()]).map_err(|e| format!("эмбеддер {}: {e}", path.display()))?;
        Ok(Self { model, dim: probe.dim, max_tokens: if probe.max_tokens > 0 { probe.max_tokens } else { 8192 } })
    }
}

impl Embedder for ServiceEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    fn encode(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        let texts: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
        client()?.embed(&self.model, &texts).map(|e| e.vectors).map_err(|e| e.to_string())
    }
}

/// Реранкер службы.
pub struct ServiceReranker {
    model: String,
    max_tokens: usize,
}

impl ServiceReranker {
    pub fn open(path: &Path) -> Result<Self, String> {
        let model = path.display().to_string();
        let probe = client()?.rerank_pairs(&model, &[("ping".into(), "pong".into())]).map_err(|e| format!("реранкер {}: {e}", path.display()))?;
        Ok(Self { model, max_tokens: if probe.max_tokens > 0 { probe.max_tokens } else { 512 } })
    }
}

impl Reranker for ServiceReranker {
    fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    fn rerank(&self, query: &str, docs: &[&str], top_k: usize) -> RerankResult<Vec<(usize, f32)>> {
        let pairs: Vec<(&str, &str)> = docs.iter().map(|d| (query, *d)).collect();
        let scores = self.score_pairs(&pairs)?;
        let mut idx: Vec<(usize, f32)> = scores.into_iter().enumerate().collect();
        idx.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        idx.truncate(top_k);
        Ok(idx)
    }

    fn score_pairs(&self, pairs: &[(&str, &str)]) -> RerankResult<Vec<f32>> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(q, d)| (q.to_string(), d.to_string())).collect();
        client()?.rerank_pairs(&self.model, &pairs).map(|r| r.scores).map_err(|e| e.to_string())
    }
}
