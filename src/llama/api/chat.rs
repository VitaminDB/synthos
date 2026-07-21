//! `POST /chat/completions`, `POST /v1/chat/completions` (OpenAI chat API).
//!
//! Поддерживает streaming, tool-calls, reasoning, response_format, multimodal
//! (через `ChatContentPart`). Принимает все llama-specific sampling-параметры
//! через `SamplingParams`.

use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use super::common::{ChatMessage, Timings, Usage};
use super::error::LlamaError;
use super::sampling::SamplingParams;
use super::sse::SseStream;
use super::LlamaClient;

// ───────────────────────────────────────────────────────────────────────────
// Request
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,

    /// Идентификатор модели; для llama-server обычно любая строка. `"llamacpp"`
    /// по умолчанию.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatTool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ChatToolChoice>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_tool_calls: Option<bool>,

    /// `logprobs` — top-K вероятности (OAI). `top_logprobs` — сколько
    /// альтернатив показывать.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i32>,

    /// `n` из OAI — сколько completion’ов генерировать.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<i32>,

    // llama-specific поля верхнего уровня.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_in_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_prompt: Option<String>,

    #[serde(flatten)]
    pub sampling: SamplingParams,
}

impl ChatRequest {
    pub fn new<I: IntoIterator<Item = ChatMessage>>(messages: I) -> Self {
        Self {
            messages: messages.into_iter().collect(),
            model: None,
            stream: None,
            stream_options: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            parse_tool_calls: None,
            logprobs: None,
            top_logprobs: None,
            n: None,
            chat_template_kwargs: None,
            reasoning_format: None,
            reasoning_in_content: None,
            generation_prompt: None,
            sampling: SamplingParams::default(),
        }
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = Some(m.into());
        self
    }
    pub fn with_stream(mut self, v: bool) -> Self {
        self.stream = Some(v);
        self
    }
    pub fn with_stream_options(mut self, opts: StreamOptions) -> Self {
        self.stream_options = Some(opts);
        self
    }
    pub fn with_response_format(mut self, fmt: ResponseFormat) -> Self {
        self.response_format = Some(fmt);
        self
    }
    pub fn with_tools<I: IntoIterator<Item = ChatTool>>(mut self, tools: I) -> Self {
        self.tools = Some(tools.into_iter().collect());
        self
    }
    pub fn with_tool_choice(mut self, choice: ChatToolChoice) -> Self {
        self.tool_choice = Some(choice);
        self
    }
    /// Явно запретить параллельные tool_calls — модель будет возвращать их
    /// по одному за turn. Упрощает оркестрацию confirm-диалога и executor’а.
    pub fn with_parallel_tool_calls_disabled(mut self) -> Self {
        self.parallel_tool_calls = Some(false);
        self
    }
    pub fn with_sampling(mut self, s: SamplingParams) -> Self {
        self.sampling = s;
        self
    }
    pub fn with_reasoning_format(mut self, f: impl Into<String>) -> Self {
        self.reasoning_format = Some(f.into());
        self
    }
    pub fn with_chat_template_kwargs(mut self, v: serde_json::Value) -> Self {
        self.chat_template_kwargs = Some(v);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    #[serde(default)]
    pub include_usage: bool,
}

/// `response_format`: OpenAI + llama-совместимые варианты.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },
    JsonSchema {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },
}

/// `tool` (OAI function calling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatToolChoice {
    Simple(String),
    Forced {
        #[serde(rename = "type")]
        kind: String,
        function: ForcedFunction,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForcedFunction {
    pub name: String,
}

/// Структурированный tool-call ассистента.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChatToolCallFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Сырой JSON-строка аргументов. Клиент сам парсит.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
// Response
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub timings: Option<Timings>,
}

impl ChatCompletionResponse {
    /// Кратко: текст первого choice.
    pub fn first_text(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.message.content.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatChoice {
    pub index: i32,
    pub message: ChatResponseMessage,
    #[serde(default)]
    pub finish_reason: Option<FinishReason>,
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponseMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub refusal: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
// Streaming
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ChatStreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChatChunkChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub timings: Option<Timings>,
}

impl ChatStreamChunk {
    /// Быстрый доступ к `choices[0].delta.content` — типичный случай при
    /// стриминге.
    pub fn first_delta_content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.delta.content.as_deref())
    }

    /// Быстрый доступ к `choices[0].delta.reasoning_content` — поле, которое
    /// llama-server присылает с reasoning-моделей при `reasoning_format=auto`
    /// или `=deepseek`. Для `=none` reasoning встроен в `content` тегами
    /// `<think>...</think>` — там разбирается отдельно через `ThinkParser`.
    pub fn first_delta_reasoning(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.delta.reasoning_content.as_deref())
    }

    /// Быстрый доступ к `choices[0].finish_reason`.
    pub fn first_finish_reason(&self) -> Option<FinishReason> {
        self.choices.first().and_then(|c| c.finish_reason)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatChunkChoice {
    pub index: i32,
    pub delta: ChatChunkDelta,
    #[serde(default)]
    pub finish_reason: Option<FinishReason>,
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChatChunkDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatChunkToolCall>>,
    #[serde(default)]
    pub refusal: Option<String>,
}

/// В streaming tool-вызовы приходят с `index` и частичными `function.arguments`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatChunkToolCall {
    pub index: i32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub function: Option<ChatToolCallFunction>,
}

// ───────────────────────────────────────────────────────────────────────────
// Methods
// ───────────────────────────────────────────────────────────────────────────

impl LlamaClient {
    /// `POST /v1/chat/completions` (non-stream).
    pub async fn chat_completions(
        &self,
        req: &ChatRequest,
    ) -> Result<ChatCompletionResponse, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(false);
        if body.model.is_none() {
            body.model = Some("llamacpp".into());
        }
        self.post_json("/v1/chat/completions", &body).await
    }

    /// `POST /v1/chat/completions` (stream).
    pub async fn chat_completions_stream(
        &self,
        req: &ChatRequest,
    ) -> Result<impl Stream<Item = Result<ChatStreamChunk, LlamaError>>, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(true);
        if body.model.is_none() {
            body.model = Some("llamacpp".into());
        }
        let sse = self.post_sse("/v1/chat/completions", &body).await?;
        Ok(decode_chat_stream(sse))
    }
}

fn decode_chat_stream(
    sse: SseStream,
) -> impl Stream<Item = Result<ChatStreamChunk, LlamaError>> {
    sse.filter_map(|ev| async move {
        match ev {
            Ok(e) if e.is_done() => None,
            Ok(e) if e.is_error() => Some(Err(LlamaError::Sse(e.data))),
            Ok(e) => Some(e.json::<ChatStreamChunk>()),
            Err(e) => Some(Err(e)),
        }
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::api::ChatRole;
    use serde_json::json;

    #[test]
    fn request_is_minimal_without_optional_fields() {
        let r = ChatRequest::new([ChatMessage::user("hi")]);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
        // нет sampling-полей
        assert!(v.get("temperature").is_none());
        assert!(v.get("stream").is_none());
    }

    #[test]
    fn sampling_fields_are_flattened() {
        let r = ChatRequest::new([ChatMessage::user("hi")])
            .with_sampling(SamplingParams::new().with_temperature(0.2).with_max_tokens(128));
        let v = serde_json::to_value(&r).unwrap();
        let temp = v["temperature"].as_f64().unwrap();
        assert!((temp - 0.2_f64).abs() < 1e-6);
        assert_eq!(v["max_tokens"], 128);
    }

    #[test]
    fn response_parses_full() {
        let raw = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "m",
            "system_fingerprint": "b1",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7},
            "timings": {"prompt_n": 5, "predicted_n": 2}
        });
        let r: ChatCompletionResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.first_text(), Some("hi"));
        assert_eq!(r.choices[0].finish_reason, Some(FinishReason::Stop));
        assert_eq!(r.usage.unwrap().total_tokens, Some(7));
    }

    #[test]
    fn chunk_parses_delta_only() {
        let raw = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "m",
            "choices": [{
                "index": 0,
                "delta": {"content": "fo"},
                "finish_reason": null
            }]
        });
        let c: ChatStreamChunk = serde_json::from_value(raw).unwrap();
        assert_eq!(c.first_delta_content(), Some("fo"));
        assert_eq!(c.first_finish_reason(), None);
    }

    #[test]
    fn tool_call_parses_both_paths() {
        let raw = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let r: ChatCompletionResponse = serde_json::from_value(raw).unwrap();
        let tc = r.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(
            tc[0].function.arguments.as_deref(),
            Some("{\"city\":\"Paris\"}")
        );
    }

    #[test]
    fn response_format_json_schema_serializes() {
        let fmt = ResponseFormat::JsonSchema {
            schema: Some(json!({"type": "object"})),
        };
        let v = serde_json::to_value(&fmt).unwrap();
        assert_eq!(v["type"], "json_schema");
        assert_eq!(v["schema"]["type"], "object");
    }

    #[test]
    fn tool_choice_simple_serializes_as_string() {
        let c = ChatToolChoice::Simple("auto".into());
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v, json!("auto"));
    }

    #[test]
    fn tool_choice_forced_serializes() {
        let c = ChatToolChoice::Forced {
            kind: "function".into(),
            function: ForcedFunction {
                name: "get_weather".into(),
            },
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "get_weather");
    }

    #[test]
    fn developer_role_can_roundtrip() {
        let m = ChatMessage::simple(ChatRole::Developer, "note");
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "developer");
    }
}
