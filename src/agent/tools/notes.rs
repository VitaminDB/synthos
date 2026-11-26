//! Инструмент `notes` — заметки пользователя в программе syn-notes
//! (synshell). Своего режима заметок в synthos больше нет: вызов уходит
//! хозяину заметок по его сокету (`syn_notes_proto`, тот же путь, что у
//! ассистента synshell) — правки сразу видны в открытом окне заметок и
//! пишутся его автосохранением. Не запущенный хозяин поднимается в фоне.

use super::executor::ToolError;

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    if args.get("action").and_then(|a| a.as_str()).is_none() {
        return Err(ToolError::MissingField("action"));
    }
    // Клиент сокета блокирующий (ждёт ответа до трёх минут) — не на
    // рабочем потоке рантайма.
    tokio::task::spawn_blocking(move || syn_notes_proto::tool(&args))
        .await
        .map_err(|e| ToolError::Spawn(e.to_string()))?
        .map_err(ToolError::Args)
}

/// Показать страницу заметок (id из ответа инструмента) в окне syn-notes;
/// без id — просто окно. Ошибка — текстом для уведомления.
pub fn open_in_notes(page_id: Option<&str>) -> Result<(), String> {
    match page_id {
        Some(id) => syn_notes_proto::open(id),
        None => {
            syn_notes_proto::ensure_running(std::time::Duration::from_secs(20))?;
            syn_notes_proto::request(&serde_json::json!({"cmd": "show"}), std::time::Duration::from_secs(10))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}
