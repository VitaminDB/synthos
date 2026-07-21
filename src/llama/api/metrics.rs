//! `GET /metrics` — Prometheus exposition format.

use super::error::LlamaError;
use super::LlamaClient;

impl LlamaClient {
    /// Сырой Prometheus-текст. Чтобы он работал, сервер должен быть запущен
    /// с `--metrics`. Если нет — вернётся `LlamaError::Http { status: 501, .. }`.
    pub async fn metrics(&self) -> Result<String, LlamaError> {
        self.get_text("/metrics").await
    }
}
