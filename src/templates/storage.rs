//! Disk-CRUD для шаблонов графа: `~/.config/synthos/templates/<slug>.json`.
//!
//! Битый файл логируется и пропускается, чтобы не уронить приложение.
//! Slug-based filenames — id шаблона = имя файла без `.json`. Коллизии
//! на create/rename разрешаются суффиксами `-2`, `-3`, … через
//! [`crate::skills::make_slug`] (общий хелпер для skills и templates).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

use super::model::Template;
use crate::skills::make_slug;

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("Шаблон с id «{0}» не найден")]
    NotFound(String),
    #[error("Имя не должно быть пустым")]
    EmptyName,
    #[error("Builtin-шаблон «{0}» нельзя изменить или удалить")]
    BuiltinReadOnly(String),
    #[error("Ошибка ввода-вывода: {0}")]
    Io(#[from] io::Error),
    #[error("Ошибка сериализации JSON: {0}")]
    Serialize(String),
}

/// `~/.config/synthos/templates/`. Создаётся лениво при первой записи.
pub fn templates_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/templates")
}

/// Загружает все custom-шаблоны с диска. Возвращает только их (builtin
/// добавляются caller'ом из [`super::builtin::all`]). Сортирует по
/// имени (lowercase) — стабильный порядок между запусками.
pub fn load_all() -> Vec<Template> {
    let dir = templates_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(it) => it,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "не удалось прочитать каталог шаблонов");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        match load_one(&id) {
            Some(t) => out.push(t),
            None => tracing::warn!(file = %path.display(), "битый template-файл, пропускаю"),
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Загружает один шаблон по id (имя файла без `.json`).
pub fn load_one(id: &str) -> Option<Template> {
    let path = templates_dir().join(format!("{id}.json"));
    let raw = fs::read_to_string(&path).ok()?;
    let mut t: Template = serde_json::from_str(&raw).ok()?;
    t.id = id.to_string();
    t.builtin = false;
    Some(t)
}

/// Создаёт новый custom-шаблон. Slug = make_slug(name) с проверкой
/// существующих id'ов. Builtin-id'ы в `taken` НЕ включаем, потому что
/// builtin живут в коде, а не на диске — пересечение имён custom/builtin
/// допускается, они различаются `builtin: bool`.
pub fn create(mut t: Template) -> Result<Template, TemplateError> {
    if t.name.trim().is_empty() {
        return Err(TemplateError::EmptyName);
    }
    let dir = templates_dir();
    fs::create_dir_all(&dir)?;
    let taken = existing_ids(&dir);
    let id = make_slug(t.name.trim(), &taken);
    t.id = id;
    t.builtin = false;
    write_file(&t)?;
    Ok(t)
}

/// Перезаписывает файл существующего шаблона (id не меняется).
/// Builtin-шаблоны пишет в `templates_dir` как обычный custom со своим
/// id — caller обычно использует [`duplicate_to_custom`] вместо этого.
pub fn save(t: &Template) -> Result<(), TemplateError> {
    if t.builtin {
        return Err(TemplateError::BuiltinReadOnly(t.id.clone()));
    }
    if t.id.is_empty() {
        return Err(TemplateError::NotFound("<empty>".into()));
    }
    fs::create_dir_all(templates_dir())?;
    write_file(t)
}

/// Переименовывает шаблон (меняет name + при необходимости slug файла).
/// Builtin → ошибка.
pub fn rename(old_id: &str, new_name: &str) -> Result<Template, TemplateError> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(TemplateError::EmptyName);
    }
    let mut t = load_one(old_id).ok_or_else(|| TemplateError::NotFound(old_id.to_string()))?;
    t.name = trimmed.to_string();

    let dir = templates_dir();
    let mut taken = existing_ids(&dir);
    taken.remove(old_id);
    let new_id = make_slug(trimmed, &taken);
    if new_id != old_id {
        t.id = new_id;
        write_file(&t)?;
        let _ = fs::remove_file(dir.join(format!("{old_id}.json")));
    } else {
        write_file(&t)?;
    }
    Ok(t)
}

/// Дублирует custom ИЛИ builtin шаблон в новый custom-файл с
/// id = slug("<name> copy"). Возвращает свежий объект.
pub fn duplicate_to_custom(src: &Template) -> Result<Template, TemplateError> {
    let mut copy = src.clone();
    copy.builtin = false;
    copy.id = String::new();
    if !copy.name.ends_with(" copy") {
        copy.name.push_str(" copy");
    }
    create(copy)
}

/// Удаляет custom-шаблон. Builtin → ошибка. Отсутствие файла — не ошибка
/// (idempotent).
pub fn delete(id: &str, builtin: bool) -> Result<(), TemplateError> {
    if builtin {
        return Err(TemplateError::BuiltinReadOnly(id.to_string()));
    }
    let path = templates_dir().join(format!("{id}.json"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TemplateError::Io(e)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Внутреннее
// ─────────────────────────────────────────────────────────────────────────────

fn write_file(t: &Template) -> Result<(), TemplateError> {
    let path = templates_dir().join(format!("{}.json", t.id));
    let pretty = serde_json::to_string_pretty(t)
        .map_err(|e| TemplateError::Serialize(e.to_string()))?;
    fs::write(&path, pretty)?;
    Ok(())
}

fn existing_ids(dir: &std::path::Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if !stem.is_empty() {
                    ids.insert(stem.to_string());
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::super::model::TemplateKind;
    use super::*;

    #[test]
    fn create_load_delete_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("synthos_tmpl_test_{}", std::process::id()));
        std::env::set_var("HOME", &tmp);
        let _ = fs::remove_dir_all(templates_dir());

        let t = Template::empty("Мой граф", TemplateKind::Full);
        let saved = create(t).unwrap();
        assert_eq!(saved.id, "moi-graf");
        assert!(!saved.builtin);

        let loaded = load_one("moi-graf").unwrap();
        assert_eq!(loaded.name, "Мой граф");
        assert_eq!(loaded.kind, TemplateKind::Full);

        let all = load_all();
        assert_eq!(all.len(), 1);

        delete("moi-graf", false).unwrap();
        assert!(load_one("moi-graf").is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn delete_builtin_errors() {
        let res = delete("anything", true);
        assert!(matches!(res, Err(TemplateError::BuiltinReadOnly(_))));
    }
}
