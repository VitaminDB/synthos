//! `GET /models`, `GET /v1/models`, `POST /models/load`, `POST /models/unload`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::error::LlamaError;
use super::LlamaClient;

/// Ответ `GET /v1/models`.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

/// Информация о модели.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,

    #[serde(default)]
    pub object: Option<String>,

    #[serde(default)]
    pub created: Option<i64>,

    #[serde(default)]
    pub owned_by: Option<String>,

    /// OpenAI v1: `meta` — дополнительные метаданные. В router mode
    /// присутствуют `in_cache`, `path`, `status` (см. `extra`).
    #[serde(default)]
    pub meta: Option<ModelMeta>,

    /// Прочие поля (`in_cache`, `path`, `status` — для `GET /models`
    /// в router mode).
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    #[serde(default)]
    pub vocab_type: Option<i32>,
    #[serde(default)]
    pub n_vocab: Option<i32>,
    #[serde(default)]
    pub n_ctx_train: Option<i32>,
    #[serde(default)]
    pub n_embd: Option<i32>,
    #[serde(default)]
    pub n_params: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct ModelLoadReq<'a> {
    model: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelLoadResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl LlamaClient {
    /// `GET /v1/models`.
    pub async fn v1_models(&self) -> Result<ModelList, LlamaError> {
        self.get_json("/v1/models").await
    }

    /// `GET /models`. В single-model mode — синоним `/v1/models`. В router
    /// mode структура ответа немного другая (нет `object: "list"` на корне),
    /// но `data` присутствует всегда.
    pub async fn models(&self) -> Result<ModelList, LlamaError> {
        // router mode возвращает `{"data":[...]}` без `"object": "list"`.
        // Обрабатываем оба варианта.
        let value: serde_json::Value = self.get_json("/models").await?;
        let object = value
            .get("object")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_string();
        let data = value.get("data").cloned().unwrap_or(serde_json::Value::Null);
        let data_vec: Vec<ModelInfo> = match data {
            serde_json::Value::Array(_) => {
                serde_json::from_value(data).map_err(|e| super::error::LlamaError::Decode {
                    source: e,
                    body_hint: "/models data".into(),
                })?
            }
            _ => Vec::new(),
        };
        Ok(ModelList { object, data: data_vec })
    }

    /// `POST /models/load` — только в router mode.
    pub async fn model_load(&self, model_id: &str) -> Result<ModelLoadResponse, LlamaError> {
        self.post_json("/models/load", &ModelLoadReq { model: model_id })
            .await
    }

    /// `POST /models/unload` — только в router mode.
    pub async fn model_unload(&self, model_id: &str) -> Result<ModelLoadResponse, LlamaError> {
        self.post_json("/models/unload", &ModelLoadReq { model: model_id })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_models_parses() {
        let raw = json!({
            "object": "list",
            "data": [{
                "id": "gpt-4o-mini",
                "object": "model",
                "created": 1735142223,
                "owned_by": "llamacpp",
                "meta": {
                    "vocab_type": 2,
                    "n_vocab": 128256,
                    "n_ctx_train": 131072,
                    "n_embd": 4096,
                    "n_params": 8030261312u64,
                    "size": 4912898304u64
                }
            }]
        });
        let m: ModelList = serde_json::from_value(raw).unwrap();
        assert_eq!(m.data.len(), 1);
        assert_eq!(m.data[0].id, "gpt-4o-mini");
        assert_eq!(m.data[0].meta.as_ref().unwrap().n_vocab, Some(128256));
    }

    #[test]
    fn router_models_has_extra_fields() {
        let raw = json!({
            "data": [{
                "id": "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M",
                "in_cache": true,
                "path": "/cache/gemma.gguf",
                "status": {"value": "loaded", "args": ["llama-server"]}
            }]
        });
        let data = raw.get("data").cloned().unwrap();
        let v: Vec<ModelInfo> = serde_json::from_value(data).unwrap();
        assert_eq!(v[0].extra.get("in_cache"), Some(&json!(true)));
        assert_eq!(v[0].extra.get("path").unwrap().as_str(), Some("/cache/gemma.gguf"));
    }
}
