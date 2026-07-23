//! OpenAI-совместимые типы схемы chat/tool, общие для обоих чат-движков.
//!
//! Раньше жили в `llama::api::chat`; вынесены в нейтральный модуль, потому что
//! их использует и нативный `syn_chat` (in-process synaptix), и подсистема
//! инструментов `chat::tools`, а слой llama-server HTTP-клиента удалён.
//!
//! Это чистые сериализуемые структуры без привязки к транспорту: описывают
//! `tool` (function-calling schema) и структурированный `tool_call` ассистента.

use serde::{Deserialize, Serialize};

/// `tool` (OAI function calling) — дескриптор инструмента для модели.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionSchema,
}

/// Описание функции инструмента: имя + JSON-schema параметров.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_call_roundtrips() {
        let raw = json!({
            "id": "call_1",
            "type": "function",
            "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}
        });
        let tc: ChatToolCall = serde_json::from_value(raw).unwrap();
        assert_eq!(tc.id, "call_1");
        assert_eq!(tc.function.name.as_deref(), Some("bash"));
        assert_eq!(tc.function.arguments.as_deref(), Some("{\"command\":\"ls\"}"));
    }

    #[test]
    fn tool_schema_skips_none_fields() {
        let t = ChatTool {
            kind: "function".into(),
            function: ToolFunctionSchema {
                name: "web".into(),
                description: None,
                parameters: None,
                strict: None,
            },
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "web");
        assert!(v["function"].get("description").is_none());
    }
}
