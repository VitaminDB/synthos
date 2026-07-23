//! Tool `autoskill` — отдаёт модели полный markdown-контент скила по id.
//!
//! Список доступных скилов модель получает в description tool'а
//! (динамически собирается в `chat::session::collect_active_tools` через
//! `build_autoskill_chat_tool`). Здесь — только executor: парсим `id`,
//! читаем файл с диска, возвращаем `# {name}\n\n{content}`.
//!
//! Чтение делается синхронно (`crate::skills::load_one`) внутри
//! `tokio::task::spawn_blocking`, чтобы не блокировать reactor на
//! медленной FS.

use serde_json::Value;

use super::executor::ToolError;

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: Value = serde_json::from_str(args_json)
        .map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("id"))?
        .to_string();

    let id_for_blocking = id.clone();
    let skill = tokio::task::spawn_blocking(move || crate::skills::load_one(&id_for_blocking))
        .await
        .map_err(|e| ToolError::BadArgs(format!("join error: {e}")))?;
    let Some(skill) = skill else {
        return Err(ToolError::BadArgs(format!("скил «{id}» не найден")));
    };
    Ok(format!("# {}\n\n{}", skill.name, skill.content))
}
