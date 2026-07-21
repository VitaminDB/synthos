//! Хранение Syn-чатов на диске: `~/.config/synthos/syn_chats/{id}.json`.
//!
//! Формат файла — общий с основным llama.cpp-чатом
//! ([`crate::chat::storage::StoredChat`]), так что переключение между чатами
//! сохраняет историю как обычно. Отличается только директория, чтобы
//! пользователь видел Syn-чаты отдельным списком.

use std::path::PathBuf;

pub use crate::chat::state::{ChatMeta, ChatMsg};
pub use crate::chat::storage::{preview_from_messages, truncate_chars, StoredChat};

/// Каталог, где лежат JSON-файлы Syn-чатов. Создаётся при первом save.
pub fn syn_chats_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/syn_chats")
}

fn chat_path(id: &str) -> PathBuf {
    syn_chats_dir().join(format!("{}.json", id))
}

pub fn list_meta() -> Vec<ChatMeta> {
    let dir = syn_chats_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("[synthos/syn_chat] не удалось прочитать {:?}: {e}", dir);
            return Vec::new();
        }
    };

    let mut out: Vec<ChatMeta> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<StoredChat>(&text) {
                Ok(chat) => out.push(chat.to_meta()),
                Err(e) => eprintln!("[synthos/syn_chat] пропускаю битый файл {:?}: {e}", path),
            },
            Err(e) => eprintln!("[synthos/syn_chat] не смог прочитать {:?}: {e}", path),
        }
    }

    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

pub fn load(id: &str) -> Option<StoredChat> {
    let path = chat_path(id);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<StoredChat>(&text) {
            Ok(chat) => Some(chat),
            Err(e) => {
                eprintln!("[synthos/syn_chat] битый JSON {:?}: {e}", path);
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("[synthos/syn_chat] не смог прочитать {:?}: {e}", path);
            None
        }
    }
}

pub fn save(chat: &StoredChat) {
    let dir = syn_chats_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[synthos/syn_chat] не удалось создать {:?}: {e}", dir);
        return;
    }
    let path = chat_path(&chat.id);
    match serde_json::to_string_pretty(chat) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                eprintln!("[synthos/syn_chat] не удалось записать {:?}: {e}", path);
            }
        }
        Err(e) => eprintln!("[synthos/syn_chat] ошибка сериализации {}: {e}", chat.id),
    }
}

pub fn delete(id: &str) {
    let path = chat_path(id);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[synthos/syn_chat] не удалось удалить {:?}: {e}", path),
    }
}
