//! `POST /embedding`, `POST /embeddings`, `POST /v1/embeddings`.

use serde::{Deserialize, Serialize};

use super::common::Usage;
use super::error::LlamaError;
use super::LlamaClient;

// ───────────────────────────────────────────────────────────────────────────
// Legacy / native llama.cpp — /embedding
// ───────────────────────────────────────────────────────────────────────────

/// Запрос `POST /embedding` (native, не OAI).
#[derive(Debug, Clone, Serialize)]
pub struct NativeEmbeddingRequest {
    /// Вход: строка, массив токенов или смешанный.
    pub content: serde_json::Value,

    /// Нормализация (см. README): `-1`, `0`, `1`, `2`, `>2`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embd_normalize: Option<i32>,
}

impl NativeEmbeddingRequest {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            content: serde_json::Value::String(text.into()),
            embd_normalize: None,
        }
    }

    pub fn from_tokens(tokens: &[i32]) -> Self {
        Self {
            content: serde_json::Value::Array(
                tokens.iter().map(|t| serde_json::json!(*t)).collect(),
            ),
            embd_normalize: None,
        }
    }

    pub fn with_normalize(mut self, v: i32) -> Self {
        self.embd_normalize = Some(v);
        self
    }
}

/// Ответ `POST /embedding`.
///
/// Структура зависит от пулинга: при `--pooling none` — per-token массив
/// векторов; иначе — один вектор. Храним обе формы.
#[derive(Debug, Clone, Deserialize)]
pub struct NativeEmbeddingResponse {
    /// Либо `Vec<f32>` (pooled), либо `Vec<Vec<f32>>` (per-token).
    pub embedding: NativeEmbeddingValue,
    #[serde(default)]
    pub tokens_evaluated: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum NativeEmbeddingValue {
    Pooled(Vec<f32>),
    PerToken(Vec<Vec<f32>>),
}

// ───────────────────────────────────────────────────────────────────────────
// /embeddings — расширенный (многовходовой), per-token если --pooling none
// ───────────────────────────────────────────────────────────────────────────

/// Один элемент ответа `POST /embeddings` (не OAI).
#[derive(Debug, Clone, Deserialize)]
pub struct NativeEmbeddingItem {
    pub index: i32,
    pub embedding: NativeEmbeddingValue,
}

// ───────────────────────────────────────────────────────────────────────────
// /v1/embeddings — OpenAI-совместимо
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingRequest {
    /// `input`: `string | string[] | int[] | int[][]`.
    pub input: serde_json::Value,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// `"float"` (default) или `"base64"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl EmbeddingRequest {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            input: serde_json::Value::String(text.into()),
            model: None,
            encoding_format: None,
            dimensions: None,
            user: None,
        }
    }

    pub fn from_texts<I, S>(texts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            input: serde_json::Value::Array(
                texts
                    .into_iter()
                    .map(|s| serde_json::Value::String(s.into()))
                    .collect(),
            ),
            model: None,
            encoding_format: None,
            dimensions: None,
            user: None,
        }
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = Some(m.into());
        self
    }
    pub fn with_encoding_format(mut self, v: impl Into<String>) -> Self {
        self.encoding_format = Some(v.into());
        self
    }
    pub fn with_dimensions(mut self, d: i32) -> Self {
        self.dimensions = Some(d);
        self
    }
    pub fn with_user(mut self, u: impl Into<String>) -> Self {
        self.user = Some(u.into());
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingData>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingData {
    pub object: String,
    pub index: i32,
    /// Обычный float-вектор; если сервер вернул base64 — клиент должен
    /// декодировать сам (см. `encoding_format`).
    pub embedding: EmbeddingValue,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingValue {
    Floats(Vec<f32>),
    Base64(String),
}

impl LlamaClient {
    /// `POST /embedding`.
    pub async fn embedding_native(
        &self,
        req: &NativeEmbeddingRequest,
    ) -> Result<NativeEmbeddingResponse, LlamaError> {
        self.post_json("/embedding", req).await
    }

    /// `POST /embeddings` (расширенный — принимает массив).
    pub async fn embeddings_native_batch(
        &self,
        req: &EmbeddingRequest,
    ) -> Result<Vec<NativeEmbeddingItem>, LlamaError> {
        self.post_json("/embeddings", req).await
    }

    /// `POST /v1/embeddings`.
    pub async fn embeddings(
        &self,
        req: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse, LlamaError> {
        self.post_json("/v1/embeddings", req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_request_serializes_minimal() {
        let r = EmbeddingRequest::from_text("hello").with_model("bge");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"input":"hello","model":"bge"}));
    }

    #[test]
    fn openai_response_parses_float_vec() {
        let raw = json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}],
            "model": "bge",
            "usage": {"prompt_tokens": 2, "total_tokens": 2}
        });
        let r: EmbeddingResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.data.len(), 1);
        match &r.data[0].embedding {
            EmbeddingValue::Floats(v) => assert_eq!(v, &vec![0.1f32, 0.2]),
            _ => panic!("expected floats"),
        }
    }

    #[test]
    fn native_response_parses_pooled() {
        let raw = json!({"embedding": [0.5, 0.25, 0.125], "tokens_evaluated": 3});
        let r: NativeEmbeddingResponse = serde_json::from_value(raw).unwrap();
        match r.embedding {
            NativeEmbeddingValue::Pooled(v) => assert_eq!(v, vec![0.5, 0.25, 0.125]),
            _ => panic!("expected pooled"),
        }
    }

    #[test]
    fn native_response_parses_per_token() {
        let raw = json!({"embedding": [[0.1, 0.2], [0.3, 0.4]]});
        let r: NativeEmbeddingResponse = serde_json::from_value(raw).unwrap();
        match r.embedding {
            NativeEmbeddingValue::PerToken(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected per-token"),
        }
    }
}
