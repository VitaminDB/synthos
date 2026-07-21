//! Ошибки HTTP-клиента.
//!
//! Все варианты непаникующие; HTTP-ошибки отдельно сохраняют статус и
//! (урезанный) текст тела ответа, чтобы в логах был читаемый контекст.

use reqwest::StatusCode;
use serde::Deserialize;
use thiserror::Error;

/// Максимальная длина «подсказки» тела ответа, которую мы держим в логе
/// при ошибке декодирования/HTTP. 4 KiB — достаточно, чтобы увидеть и
/// JSON-ошибку сервера, и хвост полезного payload’а.
const BODY_HINT_LIMIT: usize = 4 * 1024;

/// Унифицированная ошибка клиента.
#[derive(Debug, Error)]
pub enum LlamaError {
    /// Сеть / транспорт (таймаут, DNS, TLS, обрыв соединения).
    #[error("HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// Не удалось декодировать JSON-ответ.
    #[error("failed to decode response body as JSON: {source}; body: {body_hint}")]
    Decode {
        #[source]
        source: serde_json::Error,
        body_hint: String,
    },

    /// Сервер вернул non-2xx. Если тело распарсилось как OpenAI-ошибка —
    /// [`ApiError::payload`] содержит структурированный объект.
    #[error("server returned {status}: {payload:?}")]
    Http {
        status: StatusCode,
        payload: ApiErrorPayload,
    },

    /// Не удалось распарсить SSE-поток.
    #[error("SSE stream error: {0}")]
    Sse(String),
}

impl LlamaError {
    /// Создать `Http` из `StatusCode` + raw body. Пытается распознать
    /// OpenAI-совместимую структуру `{"error": {...}}`.
    pub fn from_http(status: StatusCode, body: String) -> Self {
        let payload = ApiErrorPayload::from_body(&body);
        LlamaError::Http { status, payload }
    }

    /// HTTP-статус, если это Http-ошибка.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            LlamaError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Структурированная ошибка сервера, если удалось распарсить.
    pub fn api_error(&self) -> Option<&ApiError> {
        match self {
            LlamaError::Http {
                payload: ApiErrorPayload::Parsed(e),
                ..
            } => Some(e),
            _ => None,
        }
    }

    /// Истекло ли время ожидания.
    pub fn is_timeout(&self) -> bool {
        matches!(self, LlamaError::Transport(e) if e.is_timeout())
    }

    /// Ошибка соединения (DNS, отказ TCP, TLS).
    pub fn is_connect(&self) -> bool {
        matches!(self, LlamaError::Transport(e) if e.is_connect())
    }
}

/// Структурированная ошибка API (OpenAI-совместимый формат).
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub code: Option<i64>,
    pub message: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
}

/// Либо распарсенная ошибка, либо сырой текст.
#[derive(Debug, Clone)]
pub enum ApiErrorPayload {
    /// Удалось распознать `{"error": {...}}`.
    Parsed(ApiError),
    /// Неизвестная форма — храним первые `BODY_HINT_LIMIT` байт тела.
    Raw(String),
}

impl ApiErrorPayload {
    pub fn from_body(body: &str) -> Self {
        #[derive(Deserialize)]
        struct Wrapper {
            error: ApiError,
        }
        match serde_json::from_str::<Wrapper>(body) {
            Ok(w) => ApiErrorPayload::Parsed(w.error),
            Err(_) => {
                let truncated = if body.len() > BODY_HINT_LIMIT {
                    let mut end = BODY_HINT_LIMIT;
                    while !body.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    format!("{}…(truncated)", &body[..end])
                } else {
                    body.to_string()
                };
                ApiErrorPayload::Raw(truncated)
            }
        }
    }
}
