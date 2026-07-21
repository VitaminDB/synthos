//! Источники документов для индексации.

use std::path::PathBuf;

use synaptix_rag::doc::SourceKind;

#[derive(Debug, Clone)]
pub enum DocSource {
    /// Конкретный файл — формат определяется по расширению.
    File(PathBuf),
    /// Папка рекурсивно. `extra_excludes` — gitignore-паттерны поверх
    /// дефолтных (`target/`, `node_modules/`, …).
    Folder {
        root: PathBuf,
        extra_excludes: Vec<String>,
    },
    /// PDF-файл — отдельный вариант на случай, если расширение не `.pdf`
    /// (скачали без него, например из URL).
    Pdf(PathBuf),
    /// URL — fetch + html→md + индексация. Реализовано через тот же
    /// helper, что и tool `web_read`.
    Url(String),
}

impl DocSource {
    /// Человекочитаемое имя для UI / логов.
    pub fn display(&self) -> String {
        match self {
            Self::File(p) => p.display().to_string(),
            Self::Folder { root, .. } => format!("{}/", root.display()),
            Self::Pdf(p) => p.display().to_string(),
            Self::Url(u) => u.clone(),
        }
    }

    /// Если источник — ровно один файл, возвращает `(path, kind)`.
    /// Для Folder/Url — None.
    pub fn as_single_file(&self) -> Option<(PathBuf, SourceKind)> {
        match self {
            Self::File(p) => SourceKind::from_path(p).map(|k| (p.clone(), k)),
            Self::Pdf(p) => Some((p.clone(), SourceKind::Pdf)),
            _ => None,
        }
    }
}
