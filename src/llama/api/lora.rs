//! `GET /lora-adapters`, `POST /lora-adapters`.

use serde::{Deserialize, Serialize};

use super::error::LlamaError;
use super::LlamaClient;

/// Один LoRA-адаптер.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraAdapter {
    pub id: i32,
    pub path: String,
    pub scale: f32,
}

/// Элемент запроса `POST /lora-adapters`.
#[derive(Debug, Clone, Serialize)]
pub struct LoraUpdate {
    pub id: i32,
    pub scale: f32,
}

impl LoraUpdate {
    pub fn new(id: i32, scale: f32) -> Self {
        Self { id, scale }
    }
}

impl LlamaClient {
    /// `GET /lora-adapters`.
    pub async fn lora_adapters(&self) -> Result<Vec<LoraAdapter>, LlamaError> {
        self.get_json("/lora-adapters").await
    }

    /// `POST /lora-adapters` — глобальный set scale. Возвращает сырой JSON
    /// (обычно `{"success": true}`), а также обновлённый список можно
    /// получить повторным `GET /lora-adapters`.
    pub async fn set_lora_adapters<I: IntoIterator<Item = LoraUpdate>>(
        &self,
        updates: I,
    ) -> Result<serde_json::Value, LlamaError> {
        let body: Vec<LoraUpdate> = updates.into_iter().collect();
        self.post_json("/lora-adapters", &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adapter_parses() {
        let raw = json!([
            {"id": 0, "path": "a.gguf", "scale": 1.0},
            {"id": 1, "path": "b.gguf", "scale": 0.0}
        ]);
        let parsed: Vec<LoraAdapter> = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed[0].path, "a.gguf");
        assert_eq!(parsed[1].scale, 0.0);
    }

    #[test]
    fn update_request_serializes_as_array() {
        let body = vec![LoraUpdate::new(0, 0.25), LoraUpdate::new(1, 0.5)];
        let v = serde_json::to_value(&body).unwrap();
        // Выбрали float-значения, точно представимые в f32 (степени 1/2),
        // чтобы сравнение было без допусков.
        assert_eq!(v, json!([{"id": 0, "scale": 0.25}, {"id": 1, "scale": 0.5}]));
    }
}
