//! `GET /props` и `POST /props`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::error::LlamaError;
use super::LlamaClient;

/// Ответ `GET /props`. Часть полей опциональна, потому что зависит от сборки
/// сервера и режима (single-model / router).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerProps {
    #[serde(default)]
    pub default_generation_settings: serde_json::Value,

    #[serde(default)]
    pub total_slots: Option<i32>,

    #[serde(default)]
    pub model_path: Option<String>,

    #[serde(default)]
    pub chat_template: Option<String>,

    #[serde(default)]
    pub chat_template_caps: ChatTemplateCaps,

    #[serde(default)]
    pub modalities: Modalities,

    #[serde(default)]
    pub media_marker: Option<String>,

    #[serde(default)]
    pub build_info: Option<String>,

    #[serde(default)]
    pub is_sleeping: Option<bool>,

    /// Любые дополнительные поля, которые появятся в будущих версиях.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Возможности chat template (полный список зависит от сервера, поля —
/// opt-in). Сохраняем как `HashMap<String, bool>` + удобные геттеры.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatTemplateCaps(pub HashMap<String, bool>);

impl ChatTemplateCaps {
    pub fn supports(&self, key: &str) -> bool {
        *self.0.get(key).unwrap_or(&false)
    }
}

/// Поддерживаемые модальности.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub vision: bool,

    #[serde(default)]
    pub audio: bool,

    /// Прочие модальности, которые появятся в будущем.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl LlamaClient {
    /// `GET /props`.
    pub async fn props(&self) -> Result<ServerProps, LlamaError> {
        self.get_json("/props").await
    }

    /// `GET /props?model=<id>` — router mode.
    pub async fn props_for_model(&self, model: &str) -> Result<ServerProps, LlamaError> {
        let encoded = urlencoding_minimal(model);
        self.get_json(&format!("/props?model={encoded}")).await
    }

    /// `POST /props` — требует запуск с `--props`. На момент написания
    /// API без списка полей, передаём произвольный JSON и получаем
    /// обновлённые свойства.
    pub async fn update_props(&self, body: &serde_json::Value) -> Result<ServerProps, LlamaError> {
        self.post_json("/props", body).await
    }
}

/// Минимальное URL-encoding без отдельной зависимости.
///
/// Кодирует всё, что не вписывается в «unreserved» набор RFC 3986.
fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_basic() {
        assert_eq!(urlencoding_minimal("foo/bar:Q4_K_M"), "foo%2Fbar%3AQ4_K_M");
        assert_eq!(urlencoding_minimal("abc-123"), "abc-123");
    }

    #[test]
    fn props_parses_example_from_readme() {
        let raw = r#"{
          "default_generation_settings": {"id": 0},
          "total_slots": 1,
          "model_path": "/tmp/m.gguf",
          "chat_template": "...",
          "chat_template_caps": {"tool_calls": true},
          "modalities": {"vision": true},
          "media_marker": "<__m__>",
          "build_info": "b1",
          "is_sleeping": false
        }"#;
        let p: ServerProps = serde_json::from_str(raw).unwrap();
        assert_eq!(p.total_slots, Some(1));
        assert_eq!(p.model_path.as_deref(), Some("/tmp/m.gguf"));
        assert!(p.chat_template_caps.supports("tool_calls"));
        assert!(p.modalities.vision);
        assert_eq!(p.is_sleeping, Some(false));
    }
}
