//! `POST /completion`, `POST /completions` — native llama.cpp completion.
//!
//! Для OAI-совместимого `POST /v1/completions` здесь же реализован
//! `v1_completion`/`v1_completion_stream` (тип запроса общий, используется
//! другое серверное преобразование chat template).

use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use super::common::{Timings, TokenProb, Usage};
use super::error::LlamaError;
use super::sampling::SamplingParams;
use super::sse::SseStream;
use super::LlamaClient;

// ───────────────────────────────────────────────────────────────────────────
// Prompt
// ───────────────────────────────────────────────────────────────────────────

/// Вход для `/completion`. Сервер принимает множество форм — этот enum
/// нормализует их через `#[serde(untagged)]`, чтобы и сериализация, и
/// десериализация работали.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionPrompt {
    /// Просто строка.
    Text(String),
    /// Массив id токенов.
    Tokens(Vec<i32>),
    /// Массив элементов: строки / id токенов / multimodal-объекты.
    Mixed(Vec<CompletionPromptPart>),
    /// Multimodal prompt как объект.
    Multimodal(MultimodalPrompt),
    /// Массив отдельных prompts (сервер вернёт массив completion-ов).
    Batch(Vec<CompletionPrompt>),
}

/// Элемент смешанного prompt-массива.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionPromptPart {
    Text(String),
    Token(i32),
    Multimodal(MultimodalPrompt),
}

/// Мультимодальный prompt-объект: строка с маркерами + массив base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalPrompt {
    pub prompt_string: String,
    pub multimodal_data: Vec<String>,
}

impl MultimodalPrompt {
    pub fn new(prompt: impl Into<String>, data: Vec<String>) -> Self {
        Self {
            prompt_string: prompt.into(),
            multimodal_data: data,
        }
    }
}

impl From<String> for CompletionPrompt {
    fn from(s: String) -> Self {
        CompletionPrompt::Text(s)
    }
}
impl From<&str> for CompletionPrompt {
    fn from(s: &str) -> Self {
        CompletionPrompt::Text(s.to_string())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Request
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CompletionRequest {
    pub prompt: CompletionPrompt,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Для OAI-совместимости при вызове `/v1/completions`. Серверу нужно
    /// *что-то* вместо model — `"llamacpp"` работает всегда.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(flatten)]
    pub sampling: SamplingParams,
}

impl CompletionRequest {
    pub fn new(prompt: impl Into<CompletionPrompt>) -> Self {
        Self {
            prompt: prompt.into(),
            stream: None,
            model: None,
            sampling: SamplingParams::default(),
        }
    }

    pub fn with_sampling(mut self, s: SamplingParams) -> Self {
        self.sampling = s;
        self
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = Some(m.into());
        self
    }

    pub fn with_stream(mut self, v: bool) -> Self {
        self.stream = Some(v);
        self
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Response
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopType {
    None,
    Eos,
    Limit,
    Word,
}

/// Финальный JSON `/completion` (non-stream или последний SSE-чанк).
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionFinal {
    #[serde(default)]
    pub content: String,

    #[serde(default)]
    pub tokens: Vec<i32>,

    #[serde(default)]
    pub stop: bool,

    #[serde(default)]
    pub stop_type: Option<StopType>,

    #[serde(default)]
    pub stopping_word: Option<String>,

    #[serde(default)]
    pub generation_settings: Option<serde_json::Value>,

    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub prompt: Option<serde_json::Value>,

    #[serde(default)]
    pub timings: Option<Timings>,

    #[serde(default)]
    pub tokens_cached: Option<i64>,
    #[serde(default)]
    pub tokens_evaluated: Option<i64>,
    #[serde(default)]
    pub truncated: Option<bool>,

    #[serde(default)]
    pub completion_probabilities: Option<Vec<TokenProb>>,

    /// Все поля, которые появятся в будущих версиях llama.cpp.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Чанк стрима `/completion`.
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionStreamChunk {
    #[serde(default)]
    pub content: String,

    #[serde(default)]
    pub tokens: Vec<i32>,

    #[serde(default)]
    pub stop: bool,

    #[serde(default)]
    pub stop_type: Option<StopType>,

    #[serde(default)]
    pub timings: Option<Timings>,

    #[serde(default)]
    pub prompt_progress: Option<PromptProgress>,

    /// В последнем чанке (`stop: true`) приходят все финальные поля — чтобы
    /// клиент мог собрать их, здесь доступен весь финальный объект.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl CompletionStreamChunk {
    /// Попытаться собрать из чанка `CompletionFinal` (когда `stop: true`).
    pub fn try_into_final(&self) -> Option<CompletionFinal> {
        if !self.stop {
            return None;
        }
        serde_json::from_value::<CompletionFinal>(serde_json::to_value(self).ok()?).ok()
    }
}

impl Serialize for CompletionStreamChunk {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Не основной путь — serialize используется только для утилит вроде
        // `try_into_final`, поэтому формально складываем в один Value.
        let mut v = serde_json::Map::new();
        v.insert("content".into(), serde_json::json!(&self.content));
        v.insert("tokens".into(), serde_json::json!(&self.tokens));
        v.insert("stop".into(), serde_json::json!(self.stop));
        if let Some(st) = self.stop_type {
            v.insert("stop_type".into(), serde_json::json!(st));
        }
        if let Some(t) = &self.timings {
            v.insert("timings".into(), serde_json::to_value(t).unwrap());
        }
        if let Some(pp) = &self.prompt_progress {
            v.insert("prompt_progress".into(), serde_json::to_value(pp).unwrap());
        }
        for (k, val) in &self.extra {
            v.insert(k.clone(), val.clone());
        }
        serde_json::Value::Object(v).serialize(s)
    }
}

/// Прогресс обработки prompt’а (`return_progress: true`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptProgress {
    pub total: i64,
    pub cache: i64,
    pub processed: i64,
    pub time_ms: f64,
}

/// Элементы массива вероятностей для финального ответа (алиас ради удобства).
pub type CompletionProbabilities = Vec<TokenProb>;

// ───────────────────────────────────────────────────────────────────────────
// OAI /v1/completions
// ───────────────────────────────────────────────────────────────────────────

/// Полный объект ответа `/v1/completions` (non-stream).
#[derive(Debug, Clone, Deserialize)]
pub struct V1CompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    pub choices: Vec<V1CompletionChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub timings: Option<Timings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct V1CompletionChoice {
    pub index: i32,
    pub text: String,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

/// Чанк стрима `/v1/completions`.
#[derive(Debug, Clone, Deserialize)]
pub struct V1CompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<V1CompletionChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub timings: Option<Timings>,
}

// ───────────────────────────────────────────────────────────────────────────
// Client methods
// ───────────────────────────────────────────────────────────────────────────

impl LlamaClient {
    /// `POST /completion` (non-stream).
    pub async fn completion(
        &self,
        req: &CompletionRequest,
    ) -> Result<CompletionFinal, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(false);
        self.post_json("/completion", &body).await
    }

    /// `POST /completion` (stream).
    pub async fn completion_stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<impl Stream<Item = Result<CompletionStreamChunk, LlamaError>>, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(true);
        let sse = self.post_sse("/completion", &body).await?;
        Ok(decode_completion_stream(sse))
    }

    /// `POST /v1/completions` (non-stream).
    pub async fn v1_completions(
        &self,
        req: &CompletionRequest,
    ) -> Result<V1CompletionResponse, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(false);
        if body.model.is_none() {
            body.model = Some("llamacpp".into());
        }
        self.post_json("/v1/completions", &body).await
    }

    /// `POST /v1/completions` (stream).
    pub async fn v1_completions_stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<impl Stream<Item = Result<V1CompletionChunk, LlamaError>>, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(true);
        if body.model.is_none() {
            body.model = Some("llamacpp".into());
        }
        let sse = self.post_sse("/v1/completions", &body).await?;
        Ok(decode_v1_completion_stream(sse))
    }
}

fn decode_completion_stream(
    sse: SseStream,
) -> impl Stream<Item = Result<CompletionStreamChunk, LlamaError>> {
    sse.filter_map(|ev| async move {
        match ev {
            Ok(e) if e.is_done() => None,
            Ok(e) if e.is_error() => Some(Err(LlamaError::Sse(e.data))),
            Ok(e) => Some(e.json::<CompletionStreamChunk>()),
            Err(e) => Some(Err(e)),
        }
    })
}

fn decode_v1_completion_stream(
    sse: SseStream,
) -> impl Stream<Item = Result<V1CompletionChunk, LlamaError>> {
    sse.filter_map(|ev| async move {
        match ev {
            Ok(e) if e.is_done() => None,
            Ok(e) if e.is_error() => Some(Err(LlamaError::Sse(e.data))),
            Ok(e) => Some(e.json::<V1CompletionChunk>()),
            Err(e) => Some(Err(e)),
        }
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Тесты
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_as_string_serializes() {
        let req = CompletionRequest::new("hello");
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["prompt"], "hello");
        assert!(v.get("stream").is_none());
    }

    #[test]
    fn prompt_as_tokens_serializes() {
        let req = CompletionRequest::new(CompletionPrompt::Tokens(vec![1, 2, 3]));
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["prompt"], json!([1, 2, 3]));
    }

    #[test]
    fn prompt_multimodal_serializes() {
        let mm = MultimodalPrompt::new("опиши <__media__>", vec!["AAAA".into()]);
        let req = CompletionRequest::new(CompletionPrompt::Multimodal(mm));
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["prompt"]["prompt_string"], "опиши <__media__>");
        assert_eq!(v["prompt"]["multimodal_data"][0], "AAAA");
    }

    #[test]
    fn sampling_fields_are_flattened() {
        let req = CompletionRequest::new("x").with_sampling(
            SamplingParams::new()
                .with_temperature(0.1)
                .with_n_predict(32)
                .with_stop(["###"]),
        );
        let v = serde_json::to_value(&req).unwrap();
        let temp = v["temperature"].as_f64().unwrap();
        assert!((temp - 0.1_f64).abs() < 1e-6);
        assert_eq!(v["n_predict"], 32);
        assert_eq!(v["stop"], json!(["###"]));
    }

    #[test]
    fn stop_type_deserializes() {
        let s: StopType = serde_json::from_str("\"eos\"").unwrap();
        assert_eq!(s, StopType::Eos);
    }

    #[test]
    fn completion_final_parses_typical_response() {
        let raw = json!({
            "content": "world",
            "tokens": [1, 2],
            "stop": true,
            "stop_type": "limit",
            "stopping_word": "",
            "model": "m",
            "timings": {"prompt_n": 1, "prompt_ms": 10.0, "predicted_n": 2, "predicted_ms": 20.0},
            "tokens_cached": 0,
            "tokens_evaluated": 1,
            "truncated": false
        });
        let f: CompletionFinal = serde_json::from_value(raw).unwrap();
        assert_eq!(f.content, "world");
        assert_eq!(f.stop_type, Some(StopType::Limit));
        assert_eq!(f.timings.unwrap().predicted_n, Some(2));
    }

    #[test]
    fn completion_stream_chunk_parses() {
        let raw = json!({"content": "fo", "stop": false});
        let c: CompletionStreamChunk = serde_json::from_value(raw).unwrap();
        assert_eq!(c.content, "fo");
        assert!(!c.stop);
    }

    #[test]
    fn v1_completion_response_parses() {
        let raw = json!({
            "id": "cmpl-1",
            "object": "text_completion",
            "created": 1,
            "model": "m",
            "choices": [{"index": 0, "text": "hi", "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        });
        let r: V1CompletionResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.choices[0].text, "hi");
        assert_eq!(r.usage.unwrap().total_tokens, Some(3));
    }
}
