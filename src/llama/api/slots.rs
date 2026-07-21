//! `GET /slots`, `POST /slots/{id}?action=save|restore|erase`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::error::LlamaError;
use super::{LlamaClient, Timings};

/// Один слот (элемент ответа `GET /slots`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotInfo {
    pub id: i32,
    #[serde(default)]
    pub id_task: Option<i64>,
    #[serde(default)]
    pub n_ctx: Option<i64>,
    #[serde(default)]
    pub speculative: Option<bool>,
    #[serde(default)]
    pub is_processing: Option<bool>,

    #[serde(default)]
    pub params: SlotParams,

    #[serde(default)]
    pub next_token: SlotNextToken,

    /// Прочие поля, которые могут появляться в новых версиях.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Параметры текущего слота. Поля — срез [`SamplingParams`](super::SamplingParams)
/// + специфические для слота; фиксированных обязательных нет, поэтому
/// храним как прозрачный `serde_json::Value` + удобные геттеры.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotParams(pub serde_json::Value);

impl SlotParams {
    /// Попытаться достать поле по имени.
    pub fn get<'a>(&'a self, key: &str) -> Option<&'a serde_json::Value> {
        self.0.get(key)
    }

    pub fn temperature(&self) -> Option<f64> {
        self.get("temperature").and_then(|v| v.as_f64())
    }
    pub fn n_predict(&self) -> Option<i64> {
        self.get("n_predict").and_then(|v| v.as_i64())
    }
    pub fn max_tokens(&self) -> Option<i64> {
        self.get("max_tokens").and_then(|v| v.as_i64())
    }
    pub fn stream(&self) -> Option<bool> {
        self.get("stream").and_then(|v| v.as_bool())
    }
    pub fn samplers(&self) -> Option<Vec<String>> {
        self.get("samplers").and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotNextToken {
    #[serde(default)]
    pub has_next_token: Option<bool>,
    #[serde(default)]
    pub has_new_line: Option<bool>,
    #[serde(default)]
    pub n_remain: Option<i64>,
    #[serde(default)]
    pub n_decoded: Option<i64>,
    #[serde(default)]
    pub stopping_word: Option<String>,
}

/// Ответ на `save` слота.
#[derive(Debug, Clone, Deserialize)]
pub struct SlotSaveResponse {
    pub id_slot: i32,
    pub filename: String,
    pub n_saved: i64,
    pub n_written: i64,
    #[serde(default)]
    pub timings: Option<Timings>,
}

/// Ответ на `restore` слота.
#[derive(Debug, Clone, Deserialize)]
pub struct SlotRestoreResponse {
    pub id_slot: i32,
    pub filename: String,
    pub n_restored: i64,
    pub n_read: i64,
    #[serde(default)]
    pub timings: Option<Timings>,
}

/// Ответ на `erase` слота.
#[derive(Debug, Clone, Deserialize)]
pub struct SlotEraseResponse {
    pub id_slot: i32,
    pub n_erased: i64,
}

#[derive(Serialize)]
struct SaveReq<'a> {
    filename: &'a str,
}

#[derive(Serialize)]
struct RestoreReq<'a> {
    filename: &'a str,
}

impl LlamaClient {
    /// `GET /slots`. При `fail_on_no_slot=true` сервер вернёт `503`,
    /// если нет свободного слота — преобразуется в `LlamaError::Http`.
    pub async fn slots(&self, fail_on_no_slot: bool) -> Result<Vec<SlotInfo>, LlamaError> {
        let path = if fail_on_no_slot {
            "/slots?fail_on_no_slot=1"
        } else {
            "/slots"
        };
        self.get_json(path).await
    }

    /// `POST /slots/{id}?action=save`. Требует `--slot-save-path`.
    pub async fn slot_save(
        &self,
        id_slot: i32,
        filename: &str,
    ) -> Result<SlotSaveResponse, LlamaError> {
        let body = SaveReq { filename };
        self.post_json(&format!("/slots/{id_slot}?action=save"), &body)
            .await
    }

    /// `POST /slots/{id}?action=restore`. Требует `--slot-save-path`.
    pub async fn slot_restore(
        &self,
        id_slot: i32,
        filename: &str,
    ) -> Result<SlotRestoreResponse, LlamaError> {
        let body = RestoreReq { filename };
        self.post_json(&format!("/slots/{id_slot}?action=restore"), &body)
            .await
    }

    /// `POST /slots/{id}?action=erase`.
    pub async fn slot_erase(&self, id_slot: i32) -> Result<SlotEraseResponse, LlamaError> {
        let body = serde_json::json!({});
        self.post_json(&format!("/slots/{id_slot}?action=erase"), &body)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slot_info_parses_readme_fixture() {
        let raw = json!([
            {
                "id": 0,
                "id_task": 135,
                "n_ctx": 65536,
                "speculative": false,
                "is_processing": true,
                "params": {
                    "n_predict": -1,
                    "temperature": 0.8,
                    "samplers": ["dry", "temperature"],
                    "stream": true
                },
                "next_token": {
                    "has_next_token": true,
                    "has_new_line": false,
                    "n_remain": -1,
                    "n_decoded": 0
                }
            }
        ]);
        let parsed: Vec<SlotInfo> = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.len(), 1);
        let s = &parsed[0];
        assert_eq!(s.id, 0);
        assert_eq!(s.n_ctx, Some(65536));
        assert_eq!(s.params.temperature(), Some(0.8));
        assert_eq!(
            s.params.samplers().unwrap(),
            vec!["dry".to_string(), "temperature".to_string()]
        );
        assert_eq!(s.next_token.n_decoded, Some(0));
    }

    #[test]
    fn save_response_parses() {
        let raw = json!({
            "id_slot": 0,
            "filename": "x.bin",
            "n_saved": 100,
            "n_written": 200,
            "timings": {"prompt_n": 10}
        });
        let r: SlotSaveResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.n_saved, 100);
        assert_eq!(r.timings.unwrap().prompt_n, Some(10));
    }
}
