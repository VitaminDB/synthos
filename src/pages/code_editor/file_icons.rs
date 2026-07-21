//! Material Icon helper для дерева файлов и таб-списка.
//!
//! Вместо того чтобы рисовать все файлы одной generic-иконкой
//! [`MI_DESCRIPTION`], мы группируем их по типам — пользователь визуально
//! отличает `.rs`/`.toml`/`.json`/`.md`/lock-файлы и т. п. Цвета по
//! группам остаются в MSS (`.file-icon-code` и т. д., `icon-color`).
//!
//! API намеренно простой:
//! - [`icon_for_path`] → codepoint Material Icons строкой,
//! - [`class_for_path`] → MSS-класс с цветом по группе.
//!
//! Используется в `fs_ops::read_dir_to_nodes` (узлы TreeView) и
//! `open_files::list_widget` (вкладки в правой панели).

use std::path::Path;

use crate::icons::{
    MI_ARCHIVE, MI_AUDIOTRACK, MI_CODE, MI_DATA_OBJECT, MI_DESCRIPTION, MI_FOLDER,
    MI_FOLDER_OPEN_FILLED, MI_FONT_DOWNLOAD, MI_IMAGE_ICON, MI_INTEGRATION_INSTRUCTIONS,
    MI_LOCK, MI_MOVIE, MI_PICTURE_AS_PDF, MI_TERMINAL, MI_TUNE,
};

/// Семантическая группа файла. Используется и для иконки, и для класса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    FolderOpen,
    Code,
    Json,
    Toml,
    Markdown,
    Text,
    Image,
    Video,
    Audio,
    Font,
    Pdf,
    Archive,
    Lock,
    Shell,
    Dotfile,
    Other,
}

impl FileKind {
    /// Codepoint Material Icons (строка из одного UTF-8 char'а).
    pub fn icon(self) -> &'static str {
        match self {
            FileKind::Folder => MI_FOLDER,
            FileKind::FolderOpen => MI_FOLDER_OPEN_FILLED,
            FileKind::Code => MI_CODE,
            FileKind::Json => MI_DATA_OBJECT,
            FileKind::Toml => MI_TUNE,
            FileKind::Markdown => MI_DESCRIPTION,
            FileKind::Text => MI_DESCRIPTION,
            FileKind::Image => MI_IMAGE_ICON,
            FileKind::Video => MI_MOVIE,
            FileKind::Audio => MI_AUDIOTRACK,
            FileKind::Font => MI_FONT_DOWNLOAD,
            FileKind::Pdf => MI_PICTURE_AS_PDF,
            FileKind::Archive => MI_ARCHIVE,
            FileKind::Lock => MI_LOCK,
            FileKind::Shell => MI_TERMINAL,
            FileKind::Dotfile => MI_INTEGRATION_INSTRUCTIONS,
            FileKind::Other => MI_DESCRIPTION,
        }
    }

    /// MSS-класс группы. Цвета определяются в `code_editor.mss`
    /// через `icon-color` (см. memory `icon_color_mss_2026.md`).
    pub fn css_class(self) -> &'static str {
        match self {
            FileKind::Folder | FileKind::FolderOpen => "file-icon-folder",
            FileKind::Code => "file-icon-code",
            FileKind::Json => "file-icon-json",
            FileKind::Toml => "file-icon-toml",
            FileKind::Markdown => "file-icon-markdown",
            FileKind::Text => "file-icon-text",
            FileKind::Image => "file-icon-image",
            FileKind::Video => "file-icon-video",
            FileKind::Audio => "file-icon-audio",
            FileKind::Font => "file-icon-font",
            FileKind::Pdf => "file-icon-pdf",
            FileKind::Archive => "file-icon-archive",
            FileKind::Lock => "file-icon-lock",
            FileKind::Shell => "file-icon-shell",
            FileKind::Dotfile => "file-icon-dotfile",
            FileKind::Other => "file-icon-other",
        }
    }
}

/// Определить группу пути. Папки разделены на свёрнутые/раскрытые —
/// `is_dir=true, is_expanded=true` → [`FileKind::FolderOpen`], иначе
/// [`FileKind::Folder`]. Для файлов сначала проверяем точное имя
/// (lock-файлы / Cargo.toml / Makefile), потом расширение.
pub fn kind_for(path: &Path, is_dir: bool, is_expanded: bool) -> FileKind {
    if is_dir {
        return if is_expanded { FileKind::FolderOpen } else { FileKind::Folder };
    }
    kind_for_file(path)
}

fn kind_for_file(path: &Path) -> FileKind {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    // Lock-файлы — узнаваемая группа, специфичная иконка.
    if matches!(
        name.as_str(),
        "cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "poetry.lock" | "uv.lock" | "gemfile.lock"
    ) {
        return FileKind::Lock;
    }

    // Известные конфиги без расширения / с особым именем.
    if matches!(
        name.as_str(),
        "makefile" | "dockerfile" | "rakefile" | "gemfile" | "procfile"
    ) {
        return FileKind::Code;
    }

    // Dotfiles (`.env`, `.gitignore`, `.editorconfig` и т. д.).
    if name.starts_with('.') && !name.contains(".rs") {
        // Исключаем случаи вроде `.config.rs` — но они редкие;
        // для простоты trip wire по `.rs`. Остальные dotfiles → Dotfile.
        return FileKind::Dotfile;
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        // Source code — общая иконка `code`. Различение по цвету через MSS.
        "rs" | "go" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "java" | "kt"
        | "kts" | "swift" | "scala" | "py" | "pyx" | "rb" | "ts" | "tsx" | "js" | "jsx"
        | "mjs" | "cjs" | "vue" | "svelte" | "dart" | "lua" | "php" | "pl" | "ex" | "exs"
        | "clj" | "cljs" | "hs" | "ml" | "erl" | "zig" | "nim" | "v" | "cs" | "fs"
        | "html" | "htm" | "css" | "scss" | "sass" | "less" | "sql" => FileKind::Code,

        // JSON / data — отдельный значок.
        "json" | "json5" | "jsonc" | "ndjson" | "geojson" | "yaml" | "yml" => FileKind::Json,

        // Конфиги — `tune` (slider).
        "toml" | "ini" | "cfg" | "conf" | "config" | "properties" | "xml" | "plist" => {
            FileKind::Toml
        }

        // Markdown — отдельная подгруппа в Text для будущих стилей.
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "asciidoc" => FileKind::Markdown,

        // Plain text / log.
        "txt" | "log" | "csv" | "tsv" => FileKind::Text,

        // Изображения.
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff"
        | "avif" | "heic" => FileKind::Image,

        // Видео.
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "flv" | "m4v" | "ogv" | "wmv" => {
            FileKind::Video
        }

        // Аудио.
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "aiff" | "wma" => {
            FileKind::Audio
        }

        // Шрифты.
        "ttf" | "otf" | "woff" | "woff2" | "eot" => FileKind::Font,

        "pdf" => FileKind::Pdf,

        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "lz4" => {
            FileKind::Archive
        }

        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => FileKind::Shell,

        _ => FileKind::Other,
    }
}

/// Codepoint Material Icons для пути. Convenience-обёртка над
/// [`kind_for`] + [`FileKind::icon`].
pub fn icon_for_path(path: &Path, is_dir: bool, is_expanded: bool) -> &'static str {
    kind_for(path, is_dir, is_expanded).icon()
}

/// MSS-класс по типу пути. Convenience-обёртка над [`kind_for`] +
/// [`FileKind::css_class`].
pub fn class_for_path(path: &Path, is_dir: bool, is_expanded: bool) -> &'static str {
    kind_for(path, is_dir, is_expanded).css_class()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rust_files_classified_as_code() {
        let p = PathBuf::from("src/main.rs");
        assert_eq!(kind_for(&p, false, false), FileKind::Code);
    }

    #[test]
    fn json_files_get_data_object_icon() {
        let p = PathBuf::from("config.json");
        assert_eq!(kind_for(&p, false, false), FileKind::Json);
        assert_eq!(icon_for_path(&p, false, false), MI_DATA_OBJECT);
    }

    #[test]
    fn cargo_lock_recognized() {
        let p = PathBuf::from("Cargo.lock");
        assert_eq!(kind_for(&p, false, false), FileKind::Lock);
    }

    #[test]
    fn dotfiles_recognized() {
        let p = PathBuf::from(".gitignore");
        assert_eq!(kind_for(&p, false, false), FileKind::Dotfile);
    }

    #[test]
    fn folders_distinct_open_closed() {
        let p = PathBuf::from("src");
        assert_eq!(kind_for(&p, true, false), FileKind::Folder);
        assert_eq!(kind_for(&p, true, true), FileKind::FolderOpen);
    }

    #[test]
    fn unknown_extension_falls_back_to_other() {
        let p = PathBuf::from("data.xyz");
        assert_eq!(kind_for(&p, false, false), FileKind::Other);
    }
}
