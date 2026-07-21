//! Типизированный async HTTP-клиент для `llama-server`.
//!
//! Покрывает весь публичный API llama-server (OpenAI-совместимые + родные
//! endpoints). Построен на `reqwest` (rustls) + `tokio`; стриминг реализован
//! поверх `futures_util::Stream` через модуль [`sse`].
//!
//! Точки входа — методы [`LlamaClient`], сгруппированные по файлам:
//! `health`, `props`, `slots`, `metrics`, `models`, `completion`, `chat`,
//! `embedding`, `rerank`, `infill`, `tokenize`, `lora`.
//!
//! Документацию по самому HTTP API см. `app/synthos/docs/llama-server-api.md`.
//!
//! # Пример
//!
//! ```no_run
//! use synthos::llama::api::{LlamaClient, ChatMessage, ChatRole, ChatRequest};
//! use futures_util::StreamExt;
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let client = LlamaClient::new("127.0.0.1", 8080);
//!
//! // health
//! assert!(client.health().await?.ok);
//!
//! // стриминг chat
//! let req = ChatRequest::new([
//!     ChatMessage::system("Будь краток."),
//!     ChatMessage::user("Привет."),
//! ]);
//! let mut stream = client.chat_completions_stream(&req).await?;
//! while let Some(ev) = stream.next().await {
//!     let chunk = ev?;
//!     if let Some(delta) = chunk.first_delta_content() {
//!         print!("{}", delta);
//!     }
//! }
//! # Ok(()) }
//! ```

mod error;
mod sse;
mod sampling;
mod common;

mod health;
mod props;
mod slots;
mod metrics;
mod models;
mod tokenize;
mod embedding;
mod rerank;
mod infill;
mod completion;
mod chat;
mod lora;

pub use error::{ApiError, ApiErrorPayload, LlamaError};
pub use sse::{SseEvent, SseStream};
pub use sampling::{LogitBias, LogitBiasEntry, LoraPatch, SamplerOrder, SamplingParams};
pub use common::{
    ChatContent, ChatContentPart, ChatImageUrl, ChatMessage, ChatRole, Timings, TokenProb,
    TokenProbSample, Usage,
};

pub use health::HealthStatus;
pub use props::{ChatTemplateCaps, Modalities, ServerProps};
pub use slots::{SlotInfo, SlotNextToken, SlotParams, SlotSaveResponse, SlotRestoreResponse, SlotEraseResponse};
pub use models::{ModelInfo, ModelList, ModelMeta};
pub use tokenize::{
    ApplyTemplateRequest, ApplyTemplateResponse, DetokenizeRequest, DetokenizeResponse,
    TokenEntry, TokenizeRequest, TokenizeResponse,
};
pub use embedding::{
    EmbeddingData, EmbeddingRequest, EmbeddingResponse, NativeEmbeddingItem, NativeEmbeddingRequest,
    NativeEmbeddingResponse,
};
pub use rerank::{RerankRequest, RerankResponse, RerankResult};
pub use infill::{InfillExtra, InfillRequest};
pub use completion::{
    CompletionFinal, CompletionPrompt, CompletionRequest, CompletionStreamChunk,
    CompletionProbabilities, MultimodalPrompt, StopType,
};
pub use chat::{
    ChatChoice, ChatChunkChoice, ChatChunkDelta, ChatChunkToolCall, ChatCompletionResponse,
    ChatRequest, ChatResponseMessage, ChatStreamChunk, ChatToolCall, ChatToolCallFunction,
    ChatToolChoice, ChatTool, FinishReason, ResponseFormat, StreamOptions, ToolFunctionSchema,
};
pub use lora::{LoraAdapter, LoraUpdate};

use std::sync::Arc;
use std::time::Duration;

/// Асинхронный клиент `llama-server`.
///
/// Держит внутри переиспользуемый [`reqwest::Client`], поэтому `.clone()`
/// дешёвый — можно расшаривать между задачами. Все методы неблокирующие
/// (`async fn`).
#[derive(Clone)]
pub struct LlamaClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl LlamaClient {
    /// Создать клиент по host/port (`http://host:port`).
    pub fn new(host: impl AsRef<str>, port: u16) -> Self {
        Self::with_base_url(format!("http://{}:{}", host.as_ref(), port))
    }

    /// Создать клиент по произвольному `base_url` (без слеша в конце).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self::builder().base_url(base_url).build()
    }

    /// Builder для тонкой настройки.
    pub fn builder() -> LlamaClientBuilder {
        LlamaClientBuilder::default()
    }

    /// Текущий base URL (для диагностики).
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Подготовить полный URL для запроса по относительному пути.
    pub(crate) fn url(&self, path: &str) -> String {
        let trimmed = path.trim_start_matches('/');
        format!("{}/{}", self.inner.base_url.trim_end_matches('/'), trimmed)
    }

    /// Применить API key к request builder’у, если задан.
    pub(crate) fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.inner.api_key {
            rb.bearer_auth(key)
        } else {
            rb
        }
    }

    /// Получить HTTP-клиент (для endpoints).
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    /// GET JSON → десериализовать.
    pub(crate) async fn get_json<Resp>(&self, path: &str) -> Result<Resp, LlamaError>
    where
        Resp: serde::de::DeserializeOwned,
    {
        let resp = self.authed(self.inner.http.get(self.url(path))).send().await?;
        check_and_decode(resp).await
    }

    /// POST JSON → десериализовать.
    pub(crate) async fn post_json<Req, Resp>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp, LlamaError>
    where
        Req: serde::Serialize + ?Sized,
        Resp: serde::de::DeserializeOwned,
    {
        let resp = self
            .authed(self.inner.http.post(self.url(path)).json(body))
            .send()
            .await?;
        check_and_decode(resp).await
    }

    /// GET с текстовым ответом (Prometheus, ошибки и т.п.).
    pub(crate) async fn get_text(&self, path: &str) -> Result<String, LlamaError> {
        let resp = self.authed(self.inner.http.get(self.url(path))).send().await?;
        check_and_text(resp).await
    }

    /// POST JSON → SSE-поток сырых событий.
    pub(crate) async fn post_sse<Req>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<SseStream, LlamaError>
    where
        Req: serde::Serialize + ?Sized,
    {
        let resp = self
            .authed(
                self.inner
                    .http
                    .post(self.url(path))
                    .header(reqwest::header::ACCEPT, "text/event-stream")
                    .json(body),
            )
            .send()
            .await?;
        let resp = check_status(resp).await?;
        Ok(SseStream::from_response(resp))
    }
}

/// Builder клиента.
#[derive(Default)]
pub struct LlamaClientBuilder {
    base_url: Option<String>,
    api_key: Option<String>,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
    http: Option<reqwest::Client>,
    user_agent: Option<String>,
}

impl LlamaClientBuilder {
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    pub fn host_port(self, host: impl AsRef<str>, port: u16) -> Self {
        self.base_url(format!("http://{}:{}", host.as_ref(), port))
    }

    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Полный таймаут запроса. `None` по умолчанию — без лимита (стрим
    /// chat/completions может длиться минуты).
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = Some(dur);
        self
    }

    pub fn connect_timeout(mut self, dur: Duration) -> Self {
        self.connect_timeout = Some(dur);
        self
    }

    pub fn pool_max_idle_per_host(mut self, n: usize) -> Self {
        self.pool_max_idle_per_host = Some(n);
        self
    }

    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub fn build(self) -> LlamaClient {
        let base_url = self.base_url.unwrap_or_else(|| "http://127.0.0.1:8080".into());
        let http = self.http.unwrap_or_else(|| {
            let mut b = reqwest::Client::builder()
                .user_agent(self.user_agent.unwrap_or_else(|| "synthos-llama-client/0.1".into()));
            if let Some(t) = self.timeout {
                b = b.timeout(t);
            }
            if let Some(t) = self.connect_timeout {
                b = b.connect_timeout(t);
            }
            if let Some(n) = self.pool_max_idle_per_host {
                b = b.pool_max_idle_per_host(n);
            }
            // Запасной вариант, если билд клиента упадёт (не должен) — дефолтный.
            b.build().unwrap_or_else(|_| reqwest::Client::new())
        });

        LlamaClient {
            inner: Arc::new(Inner {
                http,
                base_url: base_url.trim_end_matches('/').to_string(),
                api_key: self.api_key,
            }),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Внутренние хелперы работы с ответом
// ────────────────────────────────────────────────────────────────────────────

pub(crate) async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, LlamaError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(LlamaError::from_http(status, body))
}

async fn check_and_decode<T>(resp: reqwest::Response) -> Result<T, LlamaError>
where
    T: serde::de::DeserializeOwned,
{
    let resp = check_status(resp).await?;
    let bytes = resp.bytes().await?;
    serde_json::from_slice::<T>(&bytes).map_err(|e| LlamaError::Decode {
        source: e,
        body_hint: String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned(),
    })
}

async fn check_and_text(resp: reqwest::Response) -> Result<String, LlamaError> {
    let resp = check_status(resp).await?;
    let text = resp.text().await?;
    Ok(text)
}
