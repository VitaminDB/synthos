//! IO для скилов: чтение/запись `.md` файлов с TOML-frontmatter.
//!
//! Формат:
//! ```text
//! +++
//! name = "Приветствие"
//! description = "Тон и формат первого сообщения"
//! +++
//!
//! # Тело скила в Markdown…
//! ```
//!
//! Парсер frontmatter — ручной (одна функция [`split_frontmatter`]), без
//! сторонних крейтов помимо уже подключённого `toml`. Любая ошибка
//! IO/парсинга — не паническая: логируется и пропускается, чтобы битый
//! файл не уронил приложение целиком.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Skill, make_slug};

/// Ошибки CRUD-операций над скилами. Не паника — попадают в UI как Snackbar
/// или текст ошибки в диалоге.
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("Скил с id «{0}» не найден")]
    NotFound(String),
    #[error("Имя не должно быть пустым")]
    EmptyName,
    #[error("Ошибка ввода-вывода: {0}")]
    Io(#[from] io::Error),
    #[error("Ошибка сериализации frontmatter: {0}")]
    Serialize(String),
}

/// `~/.config/synthos/skills/`. Создаётся лениво при первой записи.
pub fn skills_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/skills")
}

/// Загружает все скилы из директории. Сортирует по `name` (lowercase),
/// чтобы порядок был стабильным между запусками. Битые файлы логируются
/// и пропускаются.
pub fn load_all() -> Vec<Skill> {
    let dir = skills_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(it) => it,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "не удалось прочитать каталог скилов");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        match load_one(&id) {
            Some(skill) => out.push(skill),
            None => tracing::warn!(file = %path.display(), "битый skill файл, пропускаю"),
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Загружает один скил по id (имя файла без расширения). `None` — файла
/// нет либо frontmatter битый.
pub fn load_one(id: &str) -> Option<Skill> {
    let path = skills_dir().join(format!("{id}.md"));
    let raw = fs::read_to_string(&path).ok()?;
    let (front, body) = split_frontmatter(&raw);
    let meta: SkillFrontmatter = toml::from_str(front.unwrap_or_default()).ok()?;
    Some(Skill {
        id: id.to_string(),
        name: if meta.name.trim().is_empty() {
            id.to_string()
        } else {
            meta.name
        },
        description: meta.description,
        content: body.to_string(),
    })
}

/// Создаёт новый скил. Slug выбирается через [`make_slug`] с учётом
/// уже занятых id'ов на диске. Возвращает финальный объект (id может
/// отличаться от ожидаемого при коллизии).
pub fn create(name: &str, description: &str, content: &str) -> Result<Skill, SkillError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SkillError::EmptyName);
    }
    let dir = skills_dir();
    fs::create_dir_all(&dir)?;

    let taken = existing_ids(&dir);
    let id = make_slug(trimmed, &taken);
    let skill = Skill {
        id,
        name: trimmed.to_string(),
        description: description.trim().to_string(),
        content: content.to_string(),
    };
    write_file(&skill)?;
    Ok(skill)
}

/// Перезаписывает файл (id не меняется).
pub fn save(s: &Skill) -> Result<(), SkillError> {
    fs::create_dir_all(skills_dir())?;
    write_file(s)
}

/// Обновляет `name` и `description` скила. Если slug нового имени отличается
/// от старого — файл переименовывается (старый удаляется). Контент не теряется.
/// Возвращает обновлённый `Skill` (с возможно новым id).
pub fn update_meta(
    old_id: &str,
    new_name: &str,
    new_description: &str,
) -> Result<Skill, SkillError> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(SkillError::EmptyName);
    }
    let mut skill = load_one(old_id).ok_or_else(|| SkillError::NotFound(old_id.to_string()))?;
    skill.name = trimmed.to_string();
    skill.description = new_description.trim().to_string();

    let dir = skills_dir();
    let mut taken = existing_ids(&dir);
    taken.remove(old_id);

    let new_id = make_slug(trimmed, &taken);
    if new_id != old_id {
        skill.id = new_id;
        write_file(&skill)?;
        let _ = fs::remove_file(dir.join(format!("{old_id}.md")));
    } else {
        write_file(&skill)?;
    }
    Ok(skill)
}

/// Удаляет файл скила. Отсутствие файла — не ошибка.
pub fn delete(id: &str) -> Result<(), SkillError> {
    let path = skills_dir().join(format!("{id}.md"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SkillError::Io(e)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Внутреннее
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct SkillFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

fn write_file(s: &Skill) -> Result<(), SkillError> {
    let meta = SkillFrontmatter {
        name: s.name.clone(),
        description: s.description.clone(),
    };
    let front = toml::to_string(&meta).map_err(|e| SkillError::Serialize(e.to_string()))?;
    let mut buf = String::with_capacity(front.len() + s.content.len() + 16);
    buf.push_str("+++\n");
    buf.push_str(&front);
    if !front.ends_with('\n') {
        buf.push('\n');
    }
    buf.push_str("+++\n\n");
    buf.push_str(&s.content);
    fs::write(s.path(), buf)?;
    Ok(())
}

fn existing_ids(dir: &std::path::Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
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

/// Возвращает `(frontmatter, body)`. Если первая строка не `+++` —
/// возвращает `(None, raw)`, считая весь файл markdown'ом без frontmatter.
///
/// Body нормализуется: один разделитель-newline после закрывающего `+++`
/// съедается (это делает классический frontmatter-вид «+++…+++<blank>body»
/// идемпотентным относительно round-trip'а).
fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let stripped = raw.strip_prefix("+++\n").or_else(|| raw.strip_prefix("+++\r\n"));
    let Some(rest) = stripped else {
        return (None, raw);
    };
    let candidates = ["\n+++\n", "\n+++\r\n"];
    for sep in candidates {
        if let Some(idx) = rest.find(sep) {
            let front = &rest[..idx];
            let body_start = idx + sep.len();
            let body = &rest[body_start..];
            // Срезаем один разделитель-newline (CRLF/LF), если он есть —
            // компенсация «пустой строки» между +++ и body, которую пишет
            // `write_file`. Без этого round-trip даёт лишний `\n` спереди.
            let body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
            return (Some(front), body);
        }
    }
    if let Some(idx) = rest.rfind("\n+++") {
        let after = &rest[idx + 4..];
        if after.is_empty() {
            return (Some(&rest[..idx]), "");
        }
    }
    (None, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_no_frontmatter() {
        assert_eq!(split_frontmatter("hello"), (None, "hello"));
        assert_eq!(split_frontmatter("# Title\n\nBody"), (None, "# Title\n\nBody"));
    }

    #[test]
    fn split_basic_frontmatter() {
        let raw = "+++\nname = \"X\"\n+++\nbody\n";
        let (f, b) = split_frontmatter(raw);
        assert_eq!(f, Some("name = \"X\""));
        assert_eq!(b, "body\n");
    }

    #[test]
    fn split_crlf_frontmatter() {
        let raw = "+++\r\nname = \"X\"\r\n+++\r\nbody\r\n";
        let (f, b) = split_frontmatter(raw);
        // Front может содержать \r или нет — проверяем, что body отделено корректно.
        assert!(f.is_some());
        assert_eq!(b, "body\r\n");
    }

    #[test]
    fn write_then_load_roundtrip() {
        // Используем уникальный TMP-каталог для теста, чтобы не цеплять
        // реальный $HOME пользователя.
        let tmp = std::env::temp_dir().join(format!("synthos_skills_test_{}", std::process::id()));
        std::env::set_var("HOME", &tmp);
        let _ = fs::remove_dir_all(skills_dir());

        let s = create("Привет", "тон", "Hello body").unwrap();
        assert_eq!(s.id, "privet");
        let loaded = load_one("privet").unwrap();
        assert_eq!(loaded.name, "Привет");
        assert_eq!(loaded.description, "тон");
        assert_eq!(loaded.content, "Hello body");

        let all = load_all();
        assert_eq!(all.len(), 1);

        delete("privet").unwrap();
        assert!(load_one("privet").is_none());

        let _ = fs::remove_dir_all(&tmp);
    }
}
