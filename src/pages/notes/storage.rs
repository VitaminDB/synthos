//! Файловое хранилище режима «Заметки» (vault).
//!
//! Vault — обычная папка на диске (по умолчанию `~/Documents/SynthOS
//! Notes`), выбранная пользователем: `.md` — текстовые страницы, рядом
//! позже появятся `*.base.json` (базы) и `*.canvas.json` (канвасы).
//! Файлы видны любым внешним инструментам; ключ страницы всюду —
//! vault-относительный путь с расширением.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Тип записи дерева vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultEntryKind {
    Dir,
    Page,
    Base,
    Canvas,
}

/// Плоская запись дерева (DFS с глубиной — рендерится колонкой).
#[derive(Clone, Debug, PartialEq)]
pub struct VaultEntry {
    /// Vault-относительный путь (`Проекты/План.md`).
    pub rel: String,
    /// Отображаемое имя без расширения.
    pub name: String,
    pub kind: VaultEntryKind,
    pub depth: usize,
}

/// Абсолютный путь vault'а: настройка либо дефолт в `~/Documents`.
pub fn resolve_vault_path(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(shellexpand_home(configured.trim()));
    }
    home_dir().join("Documents").join("SynthOS Notes")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return home_dir().join(rest).display().to_string();
    }
    p.to_string()
}

/// Создаёт папку vault'а, если её ещё нет.
pub fn ensure_vault(root: &Path) {
    if let Err(e) = fs::create_dir_all(root) {
        log::warn!("notes: не удалось создать vault {}: {e}", root.display());
    }
}

/// Тип файла по имени.
pub fn kind_of(rel: &str) -> Option<VaultEntryKind> {
    let lower = rel.to_ascii_lowercase();
    if lower.ends_with(".base.json") {
        Some(VaultEntryKind::Base)
    } else if lower.ends_with(".canvas.json") {
        Some(VaultEntryKind::Canvas)
    } else if lower.ends_with(".md") {
        Some(VaultEntryKind::Page)
    } else {
        None
    }
}

/// Отображаемое имя: имя файла без «нашего» расширения.
pub fn title_of(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    for suffix in [".base.json", ".canvas.json", ".md"] {
        if let Some(stripped) = strip_suffix_ci(name, suffix) {
            return stripped.to_string();
        }
    }
    name.to_string()
}

fn strip_suffix_ci<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    if name.len() >= suffix.len()
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    {
        Some(&name[..name.len() - suffix.len()])
    } else {
        None
    }
}

/// Рекурсивный скан vault'а: папки первыми, алфавит без регистра, скрытые
/// (`.`-префикс) и посторонние файлы пропускаются.
pub fn scan(root: &Path) -> Vec<VaultEntry> {
    let mut out = Vec::new();
    scan_dir(root, "", 0, &mut out);
    out
}

fn scan_dir(root: &Path, rel_prefix: &str, depth: usize, out: &mut Vec<VaultEntry>) {
    let dir = root.join(rel_prefix);
    let Ok(entries) = fs::read_dir(&dir) else { return };
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => dirs.push(name),
            Ok(t) if t.is_file() => {
                if kind_of(&name).is_some() {
                    files.push(name);
                }
            }
            _ => {}
        }
    }
    let key = |s: &String| s.to_lowercase();
    dirs.sort_by_key(key);
    files.sort_by_key(key);

    for name in dirs {
        let rel = join_rel(rel_prefix, &name);
        out.push(VaultEntry {
            rel: rel.clone(),
            name,
            kind: VaultEntryKind::Dir,
            depth,
        });
        scan_dir(root, &rel, depth + 1, out);
    }
    for name in files {
        let rel = join_rel(rel_prefix, &name);
        out.push(VaultEntry {
            kind: kind_of(&name).unwrap_or(VaultEntryKind::Page),
            name: title_of(&rel),
            rel,
            depth,
        });
    }
}

fn join_rel(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

pub fn abs_path(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

pub fn load(root: &Path, rel: &str) -> io::Result<String> {
    fs::read_to_string(abs_path(root, rel))
}

/// Атомарная запись: tmp-файл рядом + rename, чтобы watcher и внешние
/// читатели не видели полузаписанных файлов.
pub fn save_atomic(root: &Path, rel: &str, content: &str) -> io::Result<()> {
    save_atomic_abs(&abs_path(root, rel), content)
}

/// То же по абсолютному пути (используется автосейвом из фонового потока).
pub fn save_atomic_abs(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp~");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

/// Новая страница с уникальным именем в корне vault'а.
/// Возвращает vault-относительный путь.
pub fn create_page(root: &Path, base_title: &str) -> io::Result<String> {
    ensure_vault(root);
    for i in 0..1000 {
        let name = if i == 0 {
            format!("{base_title}.md")
        } else {
            format!("{base_title} {}.md", i + 1)
        };
        let path = root.join(&name);
        if path.exists() {
            continue;
        }
        fs::write(&path, "")?;
        return Ok(name);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "не нашлось свободного имени"))
}

/// Удаление файла или папки (с содержимым).
pub fn delete(root: &Path, rel: &str) -> io::Result<()> {
    let path = abs_path(root, rel);
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
