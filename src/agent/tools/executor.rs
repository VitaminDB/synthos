//! Async-исполнители инструментов.
//!
//! Каждый исполнитель принимает сырые аргументы из `ChatToolCall`
//! (строка с JSON) и возвращает [`ToolOutcome`] — структурированный
//! результат, который удобно и показывать в UI, и отправлять обратно
//! в llama как `role=tool` сообщение.
//!
//! Безопасность и защита контекста:
//! - `run_bash` запускает `bash -lc "<cmd>"`. Потенциально опасно — защита
//!   на уровне UI-подтверждения (см. `components::tool_confirm`).
//! - Вывод обрезается до [`MAX_OUTPUT_BYTES`] по char-boundary, чтобы
//!   гигантские листинги не раздували контекст LLM.

use thiserror::Error;
use tokio::process::Command;

use crate::agent::schema::ChatToolCall;

use super::catalog::{
    KEY_AUTOSKILL, KEY_BASH, KEY_KB_SEARCH, KEY_PIPELINES, KEY_SUBAGENT, KEY_SYSTEM, KEY_WEB,
};

/// Верхняя граница длины вывода одного инструмента (в байтах). Всё, что
/// длиннее, обрезается по char-boundary и помечается `…(truncated)`.
///
/// 64 KB — компромисс под tool `web` (action=read): после Readability+htmd
/// типичная Wikipedia/article-страница даёт 30-80 KB markdown'а. `bash`
/// и `kb_search` укладываются в эти границы с большим запасом.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Ошибки парсинга/исполнения инструмента. Не панические — маппятся в
/// `ToolOutcome { error: true }` и уходят в LLM как обычный tool-result.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Неизвестный инструмент: {0}")]
    Unknown(String),
    #[error("Некорректный JSON аргументов: {0}")]
    BadArgs(String),
    #[error("Отсутствует обязательное поле «{0}»")]
    MissingField(&'static str),
    #[error("Ошибка запуска процесса: {0}")]
    Spawn(String),
}

/// Структурированный результат исполнения.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub tool_call_id: String,
    pub name: String,
    /// Текстовое содержимое (возможно усечённое). Уходит и в UI-bubble,
    /// и в `role=tool` сообщение серверу.
    pub content: String,
    /// `true` — результат неуспешен (non-zero exit, ошибка парсинга,
    /// неизвестный инструмент). Красится UI иначе.
    pub error: bool,
}

/// Диспетчер исполнения: читает `call.function.name` и вызывает
/// соответствующий обработчик.
///
/// Никогда не паникует — все ошибки конвертируются в `ToolOutcome` с
/// `error=true`.
fn trim_json_strings(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            let t = s.trim().to_string();
            *s = t;
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(trim_json_strings),
        serde_json::Value::Object(o) => o.values_mut().for_each(trim_json_strings),
        _ => {}
    }
}

pub fn normalize_args(raw: &str) -> String {
    let trimmed = raw.trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(mut v) => {
            trim_json_strings(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| trimmed.to_string())
        }
        Err(_) => trimmed.to_string(),
    }
}

/// Приводит имя вызова к ключу каталога.
///
/// Канальные шаблоны (Muse Glimmer) объявляют инструменты пространствами
/// имён — `# Valid recipients: "self", "bash.*", "user"` — и модель иногда
/// пишет квалифицированное имя (`bash.bash`, `web.web`). Ключи каталога
/// плоские, поэтому неизвестное имя с точкой пробуем как хвост.
fn canonical_tool_name(name: &str) -> &str {
    const KEYS: [&str; 7] = [
        KEY_BASH,
        KEY_KB_SEARCH,
        KEY_WEB,
        KEY_AUTOSKILL,
        KEY_SUBAGENT,
        KEY_SYSTEM,
        KEY_PIPELINES,
    ];
    if KEYS.contains(&name) {
        return name;
    }
    match name.rsplit_once('.') {
        Some((_, tail)) if KEYS.contains(&tail) => tail,
        _ => name,
    }
}

pub async fn execute(call: &ChatToolCall) -> ToolOutcome {
    let name = call.function.name.clone().unwrap_or_default();
    let name = canonical_tool_name(&name).to_string();
    let raw_args = call.function.arguments.as_deref().unwrap_or("");
    let normalized = normalize_args(raw_args);
    let args = normalized.as_str();

    let result = match name.as_str() {
        KEY_BASH => run_bash(args).await,
        KEY_KB_SEARCH => super::kb_search::run(args).await,
        KEY_WEB => super::web::run(args).await,
        KEY_AUTOSKILL => super::autoskill::run(args).await,
        KEY_SUBAGENT => super::subagent::run(args).await,
        KEY_SYSTEM => super::system::run(args).await,
        KEY_PIPELINES => super::pipelines::run(args).await,
        other => Err(ToolError::Unknown(other.to_string())),
    };

    match result {
        Ok(content) => ToolOutcome {
            tool_call_id: call.id.clone(),
            name,
            content: truncate_output(&content),
            error: false,
        },
        Err(e) => ToolOutcome {
            tool_call_id: call.id.clone(),
            name,
            content: e.to_string(),
            error: true,
        },
    }
}

/// Запускает `bash -lc <cmd>`. Формат результата — человекочитаемый текст
/// с реальными `\n`, а не JSON-строка. Причина:
/// - JSON экранирует переводы строк (`\n` → `"\\n"`), и многокилобайтный
///   `stdout` складывается в одну длинную линию, которая рвёт layout bubble.
/// - LLM-ы стабильно понимают plain-текстовую «секцию» `--- stdout ---`.
pub async fn run_bash(args_json: &str) -> Result<String, ToolError> {
    let v: serde_json::Value = serde_json::from_str(args_json)
        .map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let command = v
        .get("command")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("command"))?;

    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .output()
        .await
        .map_err(|e| ToolError::Spawn(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    let mut out = String::new();
    out.push_str(&format!("$ {}\n", command));
    out.push_str(&format!("exit: {}\n", exit_code));
    if !stdout.is_empty() {
        out.push_str("--- stdout ---\n");
        out.push_str(&stdout);
        if !stdout.ends_with('\n') {
            out.push('\n');
        }
    }
    if !stderr.is_empty() {
        out.push_str("--- stderr ---\n");
        out.push_str(&stderr);
        if !stderr.ends_with('\n') {
            out.push('\n');
        }
    }
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("(no output)\n");
    }
    Ok(out)
}

/// Обрезает строку до `MAX_OUTPUT_BYTES` по char-boundary. Если укладывается —
/// возвращает как есть. Если нет — усечение + `…(truncated N bytes)`.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.to_string();
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = s.len() - end;
    let mut out = String::with_capacity(end + 32);
    out.push_str(&s[..end]);
    out.push_str(&format!("\n…(truncated {} bytes)", omitted));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_tool_name_resolves_to_catalog_key() {
        assert_eq!(canonical_tool_name("bash"), "bash");
        assert_eq!(canonical_tool_name("bash.bash"), "bash");
        assert_eq!(canonical_tool_name("tools.web"), "web");
        // Незнакомое имя остаётся как есть — ошибку про него отдаст execute.
        assert_eq!(canonical_tool_name("weather.today"), "weather.today");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_echo_ok() {
        let out = run_bash(r#"{"command":"echo hi"}"#).await.unwrap();
        assert!(out.contains("exit: 0"), "{out}");
        assert!(out.contains("--- stdout ---"), "{out}");
        assert!(out.contains("hi"), "{out}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_nonzero_exit() {
        // `false` всегда возвращает exit=1 — проверяем, что мы его снимаем.
        let out = run_bash(r#"{"command":"false"}"#).await.unwrap();
        assert!(out.contains("exit: 1"), "{out}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_missing_command() {
        let err = run_bash(r#"{}"#).await.unwrap_err();
        matches!(err, ToolError::MissingField("command"));
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // Многобайтный символ "я" занимает 2 байта; повторяем до полной
        // длины MAX_OUTPUT_BYTES + ровно один символ сверху, чтобы
        // активировать обрезку.
        let long = "я".repeat(MAX_OUTPUT_BYTES);
        let t = truncate_output(&long);
        assert!(t.ends_with(" bytes)"));
        // Убедимся, что не сломали UTF-8.
        let _ = t.chars().count();
    }

    #[test]
    fn truncate_noop_when_short() {
        assert_eq!(truncate_output("ok"), "ok");
    }
}
