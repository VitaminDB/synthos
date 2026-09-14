//! Инструмент `wizard`: вопрос пользователю с вариантами ответа.
//!
//! Модель вызывает `wizard` с вопросом и вариантами; в ленте появляется
//! панель с кнопками (один или несколько вариантов) и/или полем свободного
//! ответа (`pages::syn_chat::wizard`). Клик отправляет ответ обычным
//! сообщением пользователя, а ход агента на этом вызове ЗАКАНЧИВАЕТСЯ
//! (`session::run_agent_loop`): ответ придёт следующим сообщением.
//! Здесь — только разбор и проверка аргументов; результат вызова —
//! стабильная строка от аргументов (префикс-KV требует, чтобы лента и
//! промпт совпадали байт в байт).

use serde::{Deserialize, Serialize};

use super::executor::ToolError;

/// Больше вариантов панель не рисует — это уже не выбор, а список.
pub const MAX_OPTIONS: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WizardOption {
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub tooltip: String,
    /// Вариант «свой»: выбор открывает поле, ответ уходит как «label: текст».
    #[serde(default)]
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WizardSpec {
    pub question: String,
    #[serde(default)]
    pub options: Vec<WizardOption>,
    #[serde(default)]
    pub allow_multiple: bool,
    /// Поле свободного ответа рядом с вариантами (без вариантов — всегда).
    #[serde(default)]
    pub allow_free_text: bool,
    /// Без ответа панель не закрыть (кнопки «Пропустить» нет).
    #[serde(default)]
    pub required: bool,
    /// Секунды до сворачивания панели без ответа; 0/None — без таймера.
    #[serde(default)]
    pub timeout_sec: Option<u64>,
}

impl WizardSpec {
    /// Есть ли у панели поле свободного ответа (общее или у варианта).
    pub fn has_free_text(&self) -> bool {
        self.allow_free_text || self.options.iter().any(|o| o.allow_free_text)
    }
}

/// Разбор и нормализация аргументов: пустые подписи выбрасываются, без
/// вариантов панель становится полем свободного ответа.
pub fn parse_spec(args_json: &str) -> Result<WizardSpec, String> {
    let mut value: serde_json::Value = serde_json::from_str(args_json).map_err(|e| e.to_string())?;
    // Панель ленты разбирает сырые аргументы вызова, мимо исполнителя:
    // `"True"` приводится к типам схемы и здесь.
    if let Some(tool) = super::descriptor::Tool::by_key(super::catalog::KEY_WIZARD) {
        super::executor::coerce_to_schema(&mut value, &tool.schema);
    }
    let mut spec: WizardSpec = serde_json::from_value(value).map_err(|e| e.to_string())?;
    spec.question = spec.question.trim().to_string();
    if spec.question.is_empty() {
        return Err("missing \"question\"".to_string());
    }
    let options: Vec<WizardOption> = spec
        .options
        .into_iter()
        .filter_map(|o| {
            let label = o.label.trim().to_string();
            (!label.is_empty()).then(|| WizardOption {
                label,
                value: o.value.trim().to_string(),
                tooltip: o.tooltip.trim().to_string(),
                allow_free_text: o.allow_free_text,
            })
        })
        .collect();
    if options.len() > MAX_OPTIONS {
        return Err(format!("too many options: {} (max {MAX_OPTIONS})", options.len()));
    }
    spec.options = options;
    if spec.options.is_empty() {
        spec.allow_free_text = true;
    }
    if spec.timeout_sec == Some(0) {
        spec.timeout_sec = None;
    }
    Ok(spec)
}

/// Текст ответа, который уйдёт сообщением пользователя: подписи выбранных
/// вариантов через запятую; у варианта «свой» — «label: текст»; свободный
/// текст без такого варианта — отдельным пунктом.
pub fn answer_text(spec: &WizardSpec, selected: &[usize], custom: &str) -> String {
    let custom = custom.trim();
    let mut parts: Vec<String> = Vec::new();
    let mut custom_used = false;
    for &i in selected {
        let Some(o) = spec.options.get(i) else { continue };
        if o.allow_free_text && !custom.is_empty() {
            parts.push(format!("{}: {custom}", o.label));
            custom_used = true;
        } else {
            parts.push(o.label.clone());
        }
    }
    if !custom.is_empty() && !custom_used {
        parts.push(custom.to_string());
    }
    parts.join(", ")
}

/// Ответ инструмента модели. Ход на нём заканчивается — текст говорит,
/// чего ждать дальше, и повторяет варианты, чтобы модель узнала ответ.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let spec = parse_spec(args_json).map_err(ToolError::Args)?;
    let mut out = String::from(
        "The question is shown to the user as a panel with buttons. Your turn ends here: \
         the answer arrives as the user's next message",
    );
    if spec.options.is_empty() {
        out.push_str(" (free text).");
    } else {
        let labels: Vec<&str> = spec.options.iter().map(|o| o.label.as_str()).collect();
        out.push_str(if spec.allow_multiple { " — one or more of: " } else { " — one of: " });
        out.push_str(&labels.join(" | "));
        if spec.has_free_text() {
            out.push_str(", or their own text");
        }
        out.push('.');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_normalizes_options_and_free_text() {
        let spec = parse_spec(
            r#"{"question":" Какой период? ","options":[{"label":"Неделя","value":"week"},{"label":"  "},{"label":"Свой","allow_free_text":true}],"timeout_sec":0}"#,
        )
        .unwrap();
        assert_eq!(spec.question, "Какой период?");
        assert_eq!(spec.options.len(), 2);
        assert!(spec.has_free_text());
        assert_eq!(spec.timeout_sec, None);
        let free = parse_spec(r#"{"question":"Как назвать?"}"#).unwrap();
        assert!(free.allow_free_text && free.options.is_empty());
        // Булево строкой — как пишет локальная модель; панель не должна пропасть.
        let py = parse_spec(r#"{"question":"q","options":[{"label":"A","allow_free_text":"False"}],"allow_free_text":"True","required":"True"}"#).unwrap();
        assert!(py.allow_free_text && py.required && !py.options[0].allow_free_text);
    }

    #[test]
    fn parse_rejects_missing_question_and_too_many_options() {
        assert!(parse_spec(r#"{"options":[{"label":"A"}]}"#).unwrap_err().contains("question"));
        let many: Vec<String> = (0..13).map(|i| format!(r#"{{"label":"o{i}"}}"#)).collect();
        let err = parse_spec(&format!(r#"{{"question":"q","options":[{}]}}"#, many.join(","))).unwrap_err();
        assert!(err.contains("too many options"), "{err}");
        assert!(parse_spec("{").is_err());
    }

    #[test]
    fn answer_text_joins_labels_and_custom() {
        let spec = parse_spec(
            r#"{"question":"q","options":[{"label":"Неделя"},{"label":"Месяц"},{"label":"Свой","allow_free_text":true}],"allow_multiple":true}"#,
        )
        .unwrap();
        assert_eq!(answer_text(&spec, &[0], ""), "Неделя");
        assert_eq!(answer_text(&spec, &[0, 1], ""), "Неделя, Месяц");
        assert_eq!(answer_text(&spec, &[2], " 45 дней "), "Свой: 45 дней");
        assert_eq!(answer_text(&spec, &[0], "и ещё"), "Неделя, и ещё");
        assert_eq!(answer_text(&spec, &[], "только текст"), "только текст");
        assert_eq!(answer_text(&spec, &[9], ""), "");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_reports_options_and_turn_end() {
        let out = run(r#"{"question":"q","options":[{"label":"A"},{"label":"B"}]}"#).await.unwrap();
        assert!(out.contains("Your turn ends here") && out.contains("one of: A | B"), "{out}");
        let err = run(r#"{"options":[]}"#).await.unwrap_err();
        assert!(err.to_string().starts_with("Invalid arguments: missing"), "{err}");
    }
}
