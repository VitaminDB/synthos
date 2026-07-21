//! `GET /health` и `GET /v1/health`.
//!
//! Endpoint публичный — не требует API-ключа.

use serde::{Deserialize, Serialize};

use super::error::LlamaError;
use super::LlamaClient;

/// Упрощённый статус сервера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthStatus {
    /// Сервер готов (HTTP 200 + `{"status":"ok"}`).
    pub ok: bool,
    /// Сырое поле `status` из ответа, если пришёл JSON.
    pub raw_status: HealthRawStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthRawStatus {
    Ok,
    Loading,
    /// Пришёл non-200, но тело распарсили как `error` — см. `LlamaError::Http`
    /// от вызывающего для деталей.
    Error,
}

#[derive(Serialize, Deserialize)]
struct HealthBody {
    status: String,
}

impl LlamaClient {
    /// `GET /health`. `503` преобразуется в `HealthStatus { ok: false, raw_status: Loading }`.
    /// Остальные не-2xx пробрасываются как `LlamaError::Http`.
    pub async fn health(&self) -> Result<HealthStatus, LlamaError> {
        let resp = self.authed(self.http().get(self.url("/health"))).send().await?;
        let status = resp.status();

        if status.as_u16() == 503 {
            // Модель ещё грузится — это не «ошибка» для клиента; тело в формате
            // OpenAI-ошибки, но смысл — «loading».
            let _ = resp.text().await; // освобождаем тело, но не анализируем подробности
            return Ok(HealthStatus {
                ok: false,
                raw_status: HealthRawStatus::Loading,
            });
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlamaError::from_http(status, body));
        }

        let bytes = resp.bytes().await?;
        match serde_json::from_slice::<HealthBody>(&bytes) {
            Ok(h) if h.status.eq_ignore_ascii_case("ok") => Ok(HealthStatus {
                ok: true,
                raw_status: HealthRawStatus::Ok,
            }),
            _ => Ok(HealthStatus {
                ok: false,
                raw_status: HealthRawStatus::Error,
            }),
        }
    }
}
