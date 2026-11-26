//! Сообщение истории чата для службы нейросетей: роль, текст, имя
//! инструмента, размышления и вызовы. Промпт по шаблону модели собирает
//! служба (`syn_chat::daemon::to_service` переводит в `synaptixd::Message`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Имя инструмента для `role = "tool"`: шаблоны подписывают им блок
    /// результата, без него модель не понимает, чей это вывод.
    pub name: Option<String>,
    /// Размышления реплики ассистента — шаблон рендерит их сам. `None` — без
    /// блока размышлений.
    pub reasoning: Option<String>,
    /// Вызовы инструментов реплики ассистента: (имя, JSON аргументов) —
    /// шаблон рендерит их в своём родном формате.
    pub tool_calls: Vec<(String, String)>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain("system", content, None)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain("user", content, None)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain("assistant", content, None)
    }
    pub fn tool(content: impl Into<String>) -> Self {
        Self::plain("tool", content, None)
    }

    /// Результат инструмента с именем вызванной функции.
    pub fn tool_named(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self::plain("tool", content, Some(name.into()))
    }

    /// Реплика ассистента целиком: текст, размышления и структурные вызовы.
    pub fn assistant_turn(content: impl Into<String>, reasoning: Option<String>, tool_calls: Vec<(String, String)>) -> Self {
        Self { role: "assistant".into(), content: content.into(), name: None, reasoning, tool_calls }
    }

    fn plain(role: &str, content: impl Into<String>, name: Option<String>) -> Self {
        Self { role: role.into(), content: content.into(), name, reasoning: None, tool_calls: Vec::new() }
    }
}
