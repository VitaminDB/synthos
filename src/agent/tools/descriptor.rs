//! Дескриптор инструмента — данные, достаточные для:
//! 1) UI-отображения (иконка, лейбл, описание);
//! 2) сборки запроса к llama (`ChatTool` с JSON-schema параметров);
//! 3) ручного вызова executor’а по ключу.
//!
//! Инструменты описаны как `const`-подобные `once`-инициализируемые статики —
//! `serde_json::json!` не является `const fn`, поэтому используем `OnceLock`.

use std::sync::OnceLock;

use serde_json::Value as Json;

use crate::agent::schema::{ChatTool, ToolFunctionSchema};

/// Описание одного инструмента.
#[derive(Debug, Clone)]
pub struct Tool {
    /// Стабильный строковый идентификатор — уходит на сервер в `function.name`.
    pub key: &'static str,
    /// Человекочитаемый лейбл (для чипов и bubble-карточек).
    pub label: &'static str,
    /// Material-Icons codepoint.
    pub icon: &'static str,
    /// Короткое описание — уходит в `ToolFunctionSchema.description` и может
    /// показываться в tooltip’е чипа.
    pub description: &'static str,
    /// JSONSchema для `function.parameters`. Обязательно валидный JSON-объект.
    pub schema: Json,
}

impl Tool {
    /// Конвертирует descriptor в API-тип для отправки в `ChatRequest.tools`.
    pub fn to_chat_tool(&self) -> ChatTool {
        ChatTool {
            kind: "function".to_string(),
            function: ToolFunctionSchema {
                name: self.key.to_string(),
                description: Some(self.description.to_string()),
                parameters: Some(self.schema.clone()),
                strict: None,
            },
        }
    }

    /// Возвращает все известные инструменты — один глобальный список.
    pub fn all() -> &'static [Tool] {
        static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
        TOOLS.get_or_init(super::catalog::build_all).as_slice()
    }

    /// Находит инструмент по ключу. `O(N)` — список короткий.
    pub fn by_key(key: &str) -> Option<&'static Tool> {
        Self::all().iter().find(|t| t.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_chat_tool_shape() {
        let t = Tool::by_key("bash").expect("bash зарегистрирован");
        let ct = t.to_chat_tool();
        assert_eq!(ct.kind, "function");
        assert_eq!(ct.function.name, "bash");
        let params = ct.function.parameters.expect("схема присутствует");
        assert_eq!(params["type"], "object");
        assert!(params["properties"].is_object());
    }

    #[test]
    fn all_tools_have_unique_keys() {
        let tools = Tool::all();
        let mut keys: Vec<&str> = tools.iter().map(|t| t.key).collect();
        keys.sort();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "ключи инструментов должны быть уникальны");
    }
}
