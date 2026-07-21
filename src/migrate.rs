//! Одноразовая миграция пользовательских директорий с `aichat` на `synthos`.
//!
//! Вызывается первой строкой в `main` — до инициализации логгера и любого
//! чтения конфига. Если соответствующая `synthos`-директория ещё не создана,
//! а `aichat`-версия существует — переносим её одним `rename`.
//!
//! Миграция идемпотентна: повторный запуск ничего не делает (старого пути
//! нет → пропуск).

use std::path::PathBuf;

/// Список (старая_корневая_дир, новая_корневая_дир) внутри `$HOME`.
/// Соответствует фактическим путям, которые проект использует
/// (см. `config.rs`, `logging.rs`, `chat/storage.rs`, `chat/blobs.rs`,
/// `skills/storage.rs`, `kb/collection.rs`,
/// `pages/code_editor/drafts.rs`).
const PAIRS: &[(&str, &str)] = &[
    (".config/aichat", ".config/synthos"),
    (".local/state/aichat", ".local/state/synthos"),
    (".local/share/aichat", ".local/share/synthos"),
    (".cache/aichat", ".cache/synthos"),
];

pub fn migrate_user_data() {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    for (old_rel, new_rel) in PAIRS {
        let old = home.join(old_rel);
        let new = home.join(new_rel);
        if !old.exists() || new.exists() {
            continue;
        }
        if let Some(parent) = new.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(&old, &new) {
            Ok(()) => {
                eprintln!(
                    "[synthos/migrate] moved {} → {}",
                    old.display(),
                    new.display()
                );
            }
            Err(e) => {
                eprintln!(
                    "[synthos/migrate] failed to move {} → {}: {e}",
                    old.display(),
                    new.display()
                );
            }
        }
    }
}
