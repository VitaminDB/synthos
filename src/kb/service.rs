//! Модели базы знаний в службе synaptixd: эмбеддер и реранкер считает она
//! (устройство и точность — её настройки модели). Хранилище, BM25 и разбиение
//! на куски остаются здесь.

use std::path::Path;

pub type EmbeddingResult<T> = Result<T, String>;
pub type RerankResult<T> = Result<T, String>;

pub fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Эмбеддер: тексты → векторы.
pub trait Embedder {
    fn dim(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn encode(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>>;

    fn encode_query(&self, query: &str) -> EmbeddingResult<Vec<f32>> {
        let mut out = self.encode(&[query])?;
        Ok(out.pop().unwrap_or_default())
    }
}

/// Кросс-энкодер: оценки пар (запрос, документ).
pub trait Reranker {
    fn max_tokens(&self) -> usize;
    fn rerank(&self, query: &str, docs: &[&str], top_k: usize) -> RerankResult<Vec<(usize, f32)>>;
    fn score_pairs(&self, pairs: &[(&str, &str)]) -> RerankResult<Vec<f32>>;
}

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

/// Границы токенов по токенизатору эмбеддера в службе — нарезке документов
/// базы знаний. Служба без запроса `token_spans` (старая версия) — оценка по
/// словам, чтобы индексация не вставала.
pub struct ServiceTokenizer {
    model: String,
}

impl ServiceTokenizer {
    pub fn new(path: &Path) -> Self {
        Self { model: path.display().to_string() }
    }
}

impl crate::kb::doc::TokenSpans for ServiceTokenizer {
    fn spans(&self, text: &str) -> Result<Vec<(usize, usize)>, String> {
        match client()?.token_spans(&self.model, text) {
            Ok(s) => Ok(s),
            Err(e) if matches!(e.code, synaptixd::ErrorCode::Protocol) || e.message.contains("unknown variant") => {
                log::warn!("[kb] служба не считает границы токенов ({e}) — оценка по словам");
                Ok(estimate_spans(text))
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Оценка границ токенов без токенизатора: слово — кусками до 4 символов
/// (подслова sentencepiece), знак препинания — отдельно.
pub fn estimate_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut word: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            match &mut word {
                Some((_, n)) if *n < 4 => *n += 1,
                Some((start, _)) => {
                    out.push((*start, i));
                    word = Some((i, 1));
                }
                None => word = Some((i, 1)),
            }
            continue;
        }
        if let Some((start, _)) = word.take() {
            out.push((start, i));
        }
        if !c.is_whitespace() {
            out.push((i, i + c.len_utf8()));
        }
    }
    if let Some((start, _)) = word {
        out.push((start, text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::estimate_spans;

    #[test]
    fn estimate_splits_words_and_punct() {
        let t = "Привет, мир!";
        let s: Vec<&str> = estimate_spans(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(s, ["Прив", "ет", ",", "мир", "!"]);
    }
}
