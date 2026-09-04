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
    KEY_AUTOSKILL, KEY_BASH, KEY_KB_SEARCH, KEY_NOTES, KEY_PIPELINES, KEY_SUBAGENT, KEY_SYSTEM,
    KEY_WEB,
};

/// Верхняя граница длины вывода одного инструмента (в байтах). Всё, что
/// длиннее, обрезается по char-boundary и помечается `…(truncated)`.
///
/// 64 KB — компромисс под tool `web` (action=read): после Readability+htmd
/// типичная Wikipedia/article-страница даёт 30-80 KB markdown'а. `bash`
/// и `kb_search` укладываются в эти границы с большим запасом.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Предел для `autoskill`: скил — это не выхлоп команды, а инструкция,
/// которую модель обязана выполнить целиком. Обрезка на середине забирает
/// ровно ту часть, ради которой скил и подключали (рукописный скил на
/// 90 KB терял четверть текста). Вчетверо больше общего предела хватает
/// на большой скил и всё ещё страхует от патологического файла.
pub const MAX_SKILL_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES * 4;

/// Предел вывода конкретного инструмента.
fn output_limit(tool: &str) -> usize {
    if tool == KEY_AUTOSKILL {
        MAX_SKILL_OUTPUT_BYTES
    } else {
        MAX_OUTPUT_BYTES
    }
}

/// Ошибки парсинга/исполнения инструмента. Не панические — маппятся в
/// `ToolOutcome { error: true }` и уходят в LLM как обычный tool-result.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Unknown tool: {0}. Call one of: bash, kb_search, web, autoskill, subagent, system, pipelines, notes")]
    Unknown(String),
    #[error("Invalid arguments JSON: {0}")]
    BadArgs(String),
    #[error("Missing required field \"{0}\"")]
    MissingField(&'static str),
    #[error("Failed to spawn process: {0}")]
    Spawn(String),
    /// Инструмент запустился, но упал по ходу дела (у субагента — ошибка
    /// генерации). Текст уходит модели как есть: «Invalid arguments JSON»
    /// поверх OOM'а уводил её чинить аргументы вместо задачи.
    #[error("{0}")]
    Runtime(String),
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
    /// `true` — до исполнения не дошло: вызов не прошёл разбор аргументов
    /// (битый JSON, нет обязательного поля, неизвестный инструмент).
    /// Agent-loop считает такие ходы подряд: модель, которая трижды не
    /// смогла собрать вызов, уже не соберёт его и на десятый раз.
    pub invalid_args: bool,
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

/// Все ключи каталога — для канонизации имени вызова и для подсказки в
/// тексте ошибки о неизвестном инструменте.
pub(crate) const TOOL_KEYS: [&str; 8] = [
    KEY_BASH,
    KEY_KB_SEARCH,
    KEY_WEB,
    KEY_AUTOSKILL,
    KEY_SUBAGENT,
    KEY_SYSTEM,
    KEY_PIPELINES,
    KEY_NOTES,
];

/// Приводит имя вызова к ключу каталога.
///
/// Канальные шаблоны (Muse Glimmer) объявляют инструменты пространствами
/// имён — chat-шаблон рендерит `# Valid recipients: "self", "notes.*",
/// "bash.*", "user".`, то есть сам разрешает модели любой суффикс внутри
/// пространства. Она этим пользуется: `to=notes.action`, `to=bash.run`,
/// а иногда пишет и `bash.bash` или harmony-стиль `functions.web`. Ключи
/// каталога плоские, поэтому ключом считаем первый сегмент имени, который
/// в каталоге есть, — 04.09.2026 `notes.action` уходил в «Unknown tool», и
/// ход умирал на трёх одинаковых вызовах подряд.
pub(crate) fn canonical_tool_name(name: &str) -> &str {
    if TOOL_KEYS.contains(&name) {
        return name;
    }
    name.split('.')
        .find(|seg| TOOL_KEYS.contains(seg))
        .unwrap_or(name)
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
        KEY_NOTES => super::notes::run(args).await,
        other => Err(ToolError::Unknown(other.to_string())),
    };

    match result {
        Ok(content) => ToolOutcome {
            tool_call_id: call.id.clone(),
            content: truncate_output(&content, output_limit(&name)),
            name,
            error: false,
            invalid_args: false,
        },
        Err(e) => ToolOutcome {
            tool_call_id: call.id.clone(),
            name,
            invalid_args: matches!(
                e,
                ToolError::BadArgs(_) | ToolError::MissingField(_) | ToolError::Unknown(_)
            ),
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

/// Обрезает строку до `limit` по char-boundary. Если укладывается —
/// возвращает как есть. Если нет — усечение + `…(truncated N bytes)`.
fn truncate_output(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
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
        // Шаблон Muse объявляет пространство `notes.*` — модель дописывает
        // в имя действие; ключ каталога сидит в голове, а не в хвосте.
        assert_eq!(canonical_tool_name("notes.action"), "notes");
        assert_eq!(canonical_tool_name("bash.run"), "bash");
        assert_eq!(canonical_tool_name("functions.notes.create"), "notes");
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
        let t = truncate_output(&long, MAX_OUTPUT_BYTES);
        assert!(t.ends_with(" bytes)"));
        // Убедимся, что не сломали UTF-8.
        let _ = t.chars().count();
    }

    #[test]
    fn truncate_noop_when_short() {
        assert_eq!(truncate_output("ok", MAX_OUTPUT_BYTES), "ok");
    }

    #[test]
    fn skill_body_survives_common_limit() {
        // Рукописный скил на ~90 KB не должен обрезаться: у `autoskill`
        // свой предел, иначе модель получает инструкцию без хвоста.
        let skill = "я".repeat(48 * 1024); // 96 KB в байтах
        assert!(skill.len() > MAX_OUTPUT_BYTES);
        assert_eq!(output_limit(KEY_AUTOSKILL), MAX_SKILL_OUTPUT_BYTES);
        assert_eq!(truncate_output(&skill, output_limit(KEY_AUTOSKILL)), skill);
        assert!(truncate_output(&skill, output_limit(KEY_BASH)).ends_with(" bytes)"));
    }
}
