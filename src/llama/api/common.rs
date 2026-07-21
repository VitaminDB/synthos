//! Общие DTO: сообщения чата, usage, timings, вероятности токенов.

use serde::{Deserialize, Serialize};

// ───────────────────────────────────────────────────────────────────────────
// Chat messages
// ───────────────────────────────────────────────────────────────────────────

/// Роль сообщения в чате.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
    /// Не OpenAI-стандарт, но встречается в Anthropic-совместимых запросах.
    Developer,
}

/// Сообщение чата. Поле `content` — либо простая строка, либо массив
/// частей (для multimodal / tool-result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatContent>,

    /// `name` — опциональное имя участника (для tool/assistant).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Идентификатор tool-вызова, на который этот message отвечает
    /// (`role: tool`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Если ассистент сгенерировал tool-вызовы — они приходят здесь.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<super::chat::ChatToolCall>>,

    /// Содержимое reasoning (Deepseek/Qwen и совместимые), если сервер
    /// настроен его выдавать.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self::simple(ChatRole::System, text)
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::simple(ChatRole::User, text)
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::simple(ChatRole::Assistant, text)
    }

    pub fn tool(text: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: Some(ChatContent::Text(text.into())),
            name: None,
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
            reasoning_content: None,
        }
    }

    pub fn simple(role: ChatRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(ChatContent::Text(text.into())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }

    /// Собрать message с массивом частей (multimodal / mixed).
    pub fn multipart(role: ChatRole, parts: Vec<ChatContentPart>) -> Self {
        Self {
            role,
            content: Some(ChatContent::Parts(parts)),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
}

/// Содержимое сообщения: строка или массив частей.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

impl ChatContent {
    /// Если это простая строка — вернуть ссылку на неё.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ChatContent::Text(t) => Some(t),
            ChatContent::Parts(_) => None,
        }
    }
}

/// Часть multipart-сообщения (OpenAI-совместимо).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
    InputAudio {
        input_audio: ChatInputAudio,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImageUrl {
    pub url: String,
    /// `"low" | "high" | "auto"` — не всегда поддерживается, клиент может
    /// передать как подсказку.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatInputAudio {
    pub data: String,
    pub format: String,
}

// ───────────────────────────────────────────────────────────────────────────
// Usage / Timings / Token probs
// ───────────────────────────────────────────────────────────────────────────

/// Статистика токенов (OpenAI-совместимо).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

/// Тайминги llama-server (секция `timings` в ответах и слотах).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_n: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_n: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_per_token_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_per_second: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_n: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_per_token_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_per_second: Option<f64>,
}

/// Вероятности одного сгенерированного токена (`completion_probabilities[]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenProb {
    pub content: String,
    #[serde(default)]
    pub tokens: Vec<i32>,
    /// Массив вариантов — элементы либо `logprob`-формы, либо `prob`-формы,
    /// смотря по `post_sampling_probs`.
    #[serde(default)]
    pub probs: Vec<TokenProbSample>,
}

/// Запись вероятности одного варианта. `logprob` / `prob` — взаимоисключающие.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenProbSample {
    pub id: i32,
    pub token: String,
    #[serde(default)]
    pub bytes: Vec<u8>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprob: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prob: Option<f32>,

    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "top_logprobs")]
    pub top_logprobs: Vec<TokenProbSample>,

    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "top_probs")]
    pub top_probs: Vec<TokenProbSample>,
}

// ───────────────────────────────────────────────────────────────────────────
// Тесты
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chat_message_serializes_simple_text() {
        let m = ChatMessage::user("Привет");
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"], "Привет");
    }

    #[test]
    fn chat_message_serializes_multipart() {
        let m = ChatMessage::multipart(
            ChatRole::User,
            vec![
                ChatContentPart::Text {
                    text: "опиши".into(),
                },
                ChatContentPart::ImageUrl {
                    image_url: ChatImageUrl {
                        url: "data:image/jpeg;base64,AAA".into(),
                        detail: None,
                    },
                },
            ],
        );
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][1]["type"], "image_url");
    }

    #[test]
    fn usage_round_trip() {
        let u: Usage =
            serde_json::from_value(json!({"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}))
                .unwrap();
        assert_eq!(u.prompt_tokens, Some(10));
        assert_eq!(u.total_tokens, Some(13));
    }

    #[test]
    fn timings_accept_partial_fields() {
        let t: Timings = serde_json::from_value(
            json!({"prompt_n": 1, "prompt_ms": 30.9, "predicted_n": 35, "predicted_ms": 661.1}),
        )
        .unwrap();
        assert_eq!(t.prompt_n, Some(1));
        assert!((t.predicted_ms.unwrap() - 661.1).abs() < 1e-6);
        assert!(t.cache_n.is_none());
    }
}
