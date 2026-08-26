//! Хранение чатов на диске: один чат — один JSON в `~/.config/synthos/chats/`.
//!
//! Дизайн-решения:
//! - **По файлу на чат.** Позволяет инкрементально сохранять изменения и не
//!   терять остальные при битом JSON. Список в UI строится из `list_meta()`,
//!   который читает все файлы, но в этот список кладёт только лёгкую
//!   «шапку» (без полной ленты) — типичный объём чата в несколько KB.
//! - **Без внешних зависимостей на UUID.** id = `{:016x}` от
//!   `time::unix_nanos()`. Двух одинаковых значений за один процесс
//!   получить почти невозможно, коллизия с *другим* процессом — не наш
//!   сценарий (однопользовательское десктоп-приложение).
//! - **Ошибки не паникуют.** Всё, что могло сломаться (доступ к каталогу,
//!   невалидный JSON, write failure), логируется через `eprintln!` и
//!   функция безопасно возвращает `None`/`Vec::new()` — так же, как
//!   устроен [`crate::config::AppConfig`].

use syngui::{tr, trn};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::syn_chat::params::SamplingParams;

use super::state::{ChatMeta, ChatMsg};

// ─────────────────────────────────────────────────────────────────────────────
// Типы диск-слоя
// ─────────────────────────────────────────────────────────────────────────────

/// Полная запись чата на диске. Отдельная от `ChatCtx`, чтобы спокойно
/// добавлять поля (теги, flag «архивный», summary) без ломки сигналов.
///
/// `syn_params` — per-chat override sampling-параметров для Syn-чата
/// (in-process Qwen3.6 inference). `None` означает «используй
/// `AppConfig.syn_chat_defaults`». В llama-chat поле игнорируется.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredChat {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub model_name: Option<String>,
    pub messages: Vec<ChatMsg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syn_params: Option<SamplingParams>,
}

impl Default for StoredChat {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: tr!("chat.registry.new_chat_title"),
            created_at: 0,
            updated_at: 0,
            model_name: None,
            messages: Vec::new(),
            syn_params: None,
        }
    }
}

impl StoredChat {
    /// Превращает `StoredChat` в лёгкую метаинфу для списка слева.
    pub fn to_meta(&self) -> ChatMeta {
        ChatMeta {
            id: self.id.clone(),
            title: self.title.clone(),
            preview: preview_from_messages(&self.messages),
            updated_at: self.updated_at,
            model_name: self.model_name.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Пути
// ─────────────────────────────────────────────────────────────────────────────

/// Каталог, где лежат JSON-файлы чатов. Создаётся автоматически при
/// первом `save`.
pub fn chats_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/chats")
}

fn chat_path(id: &str) -> PathBuf {
    chats_dir().join(format!("{}.json", id))
}

// ─────────────────────────────────────────────────────────────────────────────
// Операции
// ─────────────────────────────────────────────────────────────────────────────

/// Читает все JSON-файлы в [`chats_dir`] и возвращает метаданные.
///
/// Порядок: «новые сверху» — отсортировано по `updated_at` убывающе. Битые
/// файлы пропускаются с логом в stderr; отсутствие каталога — не ошибка
/// (первый запуск), возвращаем пустой вектор.
pub fn list_meta() -> Vec<ChatMeta> {
    let dir = chats_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("[synthos] Не удалось прочитать каталог {:?}: {e}", dir);
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
                Err(e) => {
                    eprintln!(
                        "[synthos] Пропускаю битый чат-файл {:?}: {e}",
                        path
                    );
                }
            },
            Err(e) => {
                eprintln!("[synthos] Не смог прочитать {:?}: {e}", path);
            }
        }
    }

    // Свежие сверху.
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

/// Загружает полный чат по id. `None` — если файла нет или он битый.
pub fn load(id: &str) -> Option<StoredChat> {
    let path = chat_path(id);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<StoredChat>(&text) {
            Ok(chat) => Some(chat),
            Err(e) => {
                eprintln!("[synthos] Битый JSON чата {:?}: {e}", path);
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("[synthos] Не смог прочитать чат {:?}: {e}", path);
            None
        }
    }
}

/// Сохраняет чат на диск. Ошибка логируется и игнорируется — UI не должен
/// падать из-за проблемы с файловой системой.
pub fn save(chat: &StoredChat) {
    let dir = chats_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[synthos] Не удалось создать {:?}: {e}", dir);
        return;
    }
    let path = chat_path(&chat.id);
    match serde_json::to_string_pretty(chat) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                eprintln!("[synthos] Не удалось записать {:?}: {e}", path);
            }
        }
        Err(e) => eprintln!("[synthos] Ошибка сериализации чата {}: {e}", chat.id),
    }
}

/// Удаляет файл чата. Отсутствие файла не считается ошибкой (операция
/// идемпотентна со стороны UI).
pub fn delete(id: &str) {
    let path = chat_path(id);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[synthos] Не удалось удалить {:?}: {e}", path),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Вспомогательные
// ─────────────────────────────────────────────────────────────────────────────

/// Превью из последнего непустого user/assistant-сообщения — до 80 символов.
/// System-плашки в превью не попадают: они бесполезны для идентификации чата.
///
/// Сообщение из одних вложений (текст пустой) описывается их количеством —
/// иначе такой чат выглядел бы в списке пустым.
pub fn preview_from_messages(messages: &[ChatMsg]) -> String {
    use super::state::ChatMsgRole;
    for m in messages.iter().rev() {
        if !matches!(m.role, ChatMsgRole::User | ChatMsgRole::Assistant) {
            continue;
        }
        let text = m.body.trim();
        if !text.is_empty() {
            return truncate_chars(text, 80);
        }
        if !m.attachments.is_empty() {
            return attachments_preview(&m.attachments);
        }
    }
    String::new()
}

/// «📎 3 вложения» — компактная подпись для чата без текста.
fn attachments_preview(attachments: &[super::state::MsgAttachment]) -> String {
    if attachments.len() == 1 {
        let a = &attachments[0];
        if !a.original_name.is_empty() {
            return truncate_chars(&a.original_name, 80);
        }
        return a.kind.label().to_string();
    }
    trn!("chat.attachments.count", attachments.len())
}

/// Обрезка строки до `max` **символов** (а не байтов) с сохранением
/// UTF-8-безопасности. Добавляет `…`, если обрезали.
pub fn truncate_chars(s: &str, max: usize) -> String {
    let mut count = 0;
    let mut end_byte = s.len();
    for (i, _) in s.char_indices() {
        if count == max {
            end_byte = i;
            break;
        }
        count += 1;
    }
    if end_byte == s.len() {
        s.to_string()
    } else {
        format!("{}…", &s[..end_byte])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_utf8_boundary() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("abcdefghij", 3), "abc…");
        // Кириллица: каждый символ — 2 байта в UTF-8.
        let s = "Привет, мир!";
        let got = truncate_chars(s, 6);
        assert_eq!(got, "Привет…");
    }

    #[test]
    fn preview_uses_latest_nonempty() {
        use super::super::state::ChatMsg;
        let msgs = vec![
            ChatMsg::user("first"),
            ChatMsg::assistant_empty(),          // body пустой — пропускаем
            ChatMsg::user("second"),
            ChatMsg::system("sys", false),       // system — пропускаем
        ];
        assert_eq!(preview_from_messages(&msgs), "second");
    }
}
