//! Инструмент `notes` — из библиотеки заметок (`syn_notes::tool`): тот же
//! код, что исполняет хозяин заметок synshell. Здесь — только перевод
//! ошибок в ошибки исполнителя synthos.

use super::executor::ToolError;

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    syn_notes::tool::run(args_json).await.map_err(|e| match e {
        syn_notes::tool::ToolError::BadArgs(s) => ToolError::BadArgs(s),
        syn_notes::tool::ToolError::Args(s) => ToolError::Args(s),
        syn_notes::tool::ToolError::MissingField(k) => ToolError::MissingField(k),
        syn_notes::tool::ToolError::Spawn(s) => ToolError::Spawn(s),
    })
}
