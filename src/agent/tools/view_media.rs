//! Инструмент `view_media` — модель сама смотрит локальный файл по пути.
//!
//! Вложение к сообщению видит только то, что прикрепил пользователь; с этим
//! инструментом модель находит файлы сама (папку листает `bash`-ем) и
//! получает их тем же путём, что и вложения: картинка и видео — эмбеддингами
//! своей vision-башни, звук — транскриптом загруженной ASR-модели, документ —
//! текстом. Чего модель не умеет, приходит строкой-заглушкой с причиной.
//!
//! Исполняет инструмент agent-loop чата (`syn_chat::session`): кодированию
//! нужны загруженная модель, её башня и место в VRAM, которое держит кэш
//! префикс-KV хода. Здесь — разбор аргументов, проверка путей и тело
//! результата; [`run`] отвечает только вызову вне цикла (субагент).

use std::path::{Path, PathBuf};

use crate::agent::state::{AttachmentKind, MsgAttachment};

use super::executor::ToolError;

/// Сколько файлов за один вызов. Каждая картинка — до тысяч vision-токенов,
/// а остаток окна считается уже после кодирования: дюжина фото за раз
/// съедала бы окно целиком, прежде чем бюджет успел бы сказать «хватит».
pub const MAX_FILES: usize = 8;

/// Пути из аргументов вызова: `path` (один файл) и/или `paths` (несколько).
/// Модели путают форму — строку в `paths` и массив в `path` тоже принимаем.
pub fn parse_paths(args_json: &str) -> Result<Vec<PathBuf>, ToolError> {
    let v: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let mut raw: Vec<String> = Vec::new();
    for key in ["path", "paths"] {
        match v.get(key) {
            Some(serde_json::Value::String(s)) => raw.push(s.clone()),
            Some(serde_json::Value::Array(items)) => {
                raw.extend(items.iter().filter_map(|x| x.as_str()).map(str::to_string))
            }
            _ => {}
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for s in raw.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let p = resolve(s);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.is_empty() {
        return Err(ToolError::MissingField("path"));
    }
    if out.len() > MAX_FILES {
        return Err(ToolError::Args(format!(
            "at most {MAX_FILES} files per call, got {}; view the first {MAX_FILES} \
             and call view_media again for the rest",
            out.len()
        )));
    }
    Ok(out)
}

/// `~` и `~/…` — от домашнего каталога, `file://` снимается. Относительный
/// путь остаётся относительным — от рабочего каталога процесса, как у `bash`.
pub fn resolve(raw: &str) -> PathBuf {
    let s = raw.strip_prefix("file://").unwrap_or(raw);
    if s == "~" || s.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(s.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    PathBuf::from(s)
}

/// Файл ли это вообще. Каталог — частая ошибка: модели сказали «посмотри
/// папку», и она передала папку, — ответ говорит, как её пролистать.
pub fn check_file(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Err(_) => Err("file not found".into()),
        Ok(m) if m.is_dir() => Err(
            "this is a directory — list it with bash (e.g. `find DIR -maxdepth 1 -type f`) \
             and pass the files you want to see"
                .into(),
        ),
        Ok(_) => Ok(()),
    }
}

/// Что стало с одним путём из вызова.
pub enum Status {
    /// Файл ушёл в промпт под номером `index` (с 1): содержимое досталось
    /// модели (`tokens` — сколько оно заняло) или вместо него стоит
    /// заглушка с причиной `failure`.
    Included {
        index: usize,
        attachment: MsgAttachment,
        tokens: usize,
        failure: Option<String>,
    },
    /// Файл прочитан, но в остаток окна не влез.
    Skipped { attachment: MsgAttachment, need: usize, allowed: usize },
    /// Файла нет, это каталог или его не удалось прочесть.
    Failed(String),
}

pub struct Entry {
    pub path: PathBuf,
    pub status: Status,
}

impl Entry {
    /// Содержимое файла досталось модели.
    pub fn shown(&self) -> bool {
        matches!(self.status, Status::Included { failure: None, .. })
    }
}

/// Тело результата: какой файл под каким номером, во что он превратился и
/// что не показано. Блоки самих файлов стоят в промпте перед этим телом
/// (`attach::prompt::assemble_tool_view`), в ленте — карточками.
pub fn summary(entries: &[Entry]) -> String {
    let shown = entries.iter().filter(|e| e.shown()).count();
    let mut out = if shown == 0 {
        format!("view_media: none of the {} file(s) could be shown to you.\n", entries.len())
    } else {
        format!(
            "view_media: {shown} of {} file(s) shown above, in order.\n",
            entries.len()
        )
    };
    for e in entries {
        let path = e.path.display();
        match &e.status {
            Status::Included { index, attachment, tokens, failure: None } => {
                out.push_str(&format!(
                    "[file {index}] {path} — {} → {}\n",
                    describe(attachment),
                    delivered_as(attachment.kind, *tokens)
                ));
            }
            Status::Included { index, attachment, failure: Some(why), .. } => {
                out.push_str(&format!(
                    "[file {index}] {path} — {}; NOT shown: {why}\n",
                    describe(attachment)
                ));
            }
            Status::Skipped { attachment, need, allowed } => {
                out.push_str(&format!(
                    "- {path} — {}; NOT shown: needs ~{need} tokens, only ~{allowed} left \
                     for this result in the context window\n",
                    describe(attachment)
                ));
            }
            Status::Failed(why) => out.push_str(&format!("- {path} — NOT shown: {why}\n")),
        }
    }
    out
}

/// Во что файл превратился для модели.
fn delivered_as(kind: AttachmentKind, tokens: usize) -> String {
    match kind {
        AttachmentKind::Image | AttachmentKind::Video => format!("{tokens} vision tokens"),
        AttachmentKind::Audio => format!("speech transcript, ~{tokens} tokens"),
        AttachmentKind::Document | AttachmentKind::Other => format!("text, ~{tokens} tokens"),
    }
}

/// «image 1920×1080, 2.1 MB» — вид файла и его метаданные.
fn describe(a: &MsgAttachment) -> String {
    let kind = match a.kind {
        AttachmentKind::Image => "image",
        AttachmentKind::Video => "video",
        AttachmentKind::Audio => "audio",
        AttachmentKind::Document => "document",
        AttachmentKind::Other => "file",
    };
    let mut s = kind.to_string();
    if a.width > 0 && a.height > 0 {
        s.push_str(&format!(" {}×{}", a.width, a.height));
    }
    if a.duration_ms > 0 {
        let secs = a.duration_ms / 1000;
        s.push_str(&format!(" {}:{:02}", secs / 60, secs % 60));
    }
    s.push_str(&format!(", {}", human_size(a.size_bytes)));
    s
}

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Вызов вне agent-loop чата. Субагенту инструмент не выдаётся (картинку
/// некуда деть — у его цикла нет медиа-пути), так что сюда попадает только
/// модель, придумавшая вызов сама.
pub async fn run(_args: &str) -> Result<String, ToolError> {
    Err(ToolError::Runtime(
        "view_media works only in the main chat, not inside a subagent".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(kind: AttachmentKind) -> MsgAttachment {
        MsgAttachment {
            sha256: "0".repeat(64),
            mime: "image/png".into(),
            original_name: "me.png".into(),
            width: 1920,
            height: 1080,
            size_bytes: 2 * 1024 * 1024 + 100 * 1024,
            kind,
            ext: "png".into(),
            duration_ms: 0,
            model_ext: String::new(),
            ui_ext: String::new(),
            has_thumb: false,
            share_path: false,
        }
    }

    #[test]
    fn path_and_paths_are_merged_and_deduplicated() {
        let p = parse_paths(r#"{"path":"/a.png","paths":["/b.jpg","/a.png"]}"#).unwrap();
        assert_eq!(p, [PathBuf::from("/a.png"), PathBuf::from("/b.jpg")]);
        // Модель перепутала форму: строка в `paths`, массив в `path`.
        let p = parse_paths(r#"{"paths":"/c.png"}"#).unwrap();
        assert_eq!(p, [PathBuf::from("/c.png")]);
        let p = parse_paths(r#"{"path":["/d.png","/e.png"]}"#).unwrap();
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn empty_and_oversized_calls_are_argument_errors() {
        assert!(matches!(parse_paths(r#"{}"#), Err(ToolError::MissingField("path"))));
        assert!(matches!(parse_paths(r#"{"path":"  "}"#), Err(ToolError::MissingField("path"))));
        let many: Vec<String> = (0..=MAX_FILES).map(|i| format!("\"/{i}.png\"")).collect();
        let args = format!("{{\"paths\":[{}]}}", many.join(","));
        assert!(matches!(parse_paths(&args), Err(ToolError::Args(_))));
        assert!(matches!(parse_paths("{\"path\":"), Err(ToolError::BadArgs(_))));
    }

    #[test]
    fn home_and_file_url_are_resolved() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(resolve("~/Pictures/me.jpg"), home.join("Pictures/me.jpg"));
        assert_eq!(resolve("~"), home);
        assert_eq!(resolve("file:///tmp/x.png"), PathBuf::from("/tmp/x.png"));
        assert_eq!(resolve("rel/x.png"), PathBuf::from("rel/x.png"));
    }

    #[test]
    fn directory_is_rejected_with_a_hint() {
        let dir = std::env::temp_dir();
        let why = check_file(&dir).unwrap_err();
        assert!(why.contains("directory") && why.contains("find"), "{why}");
        assert_eq!(check_file(Path::new("/nonexistent/x.png")).unwrap_err(), "file not found");
    }

    #[test]
    fn summary_numbers_files_like_the_prompt_headers() {
        let entries = vec![
            Entry {
                path: "/p/me.png".into(),
                status: Status::Included {
                    index: 1,
                    attachment: att(AttachmentKind::Image),
                    tokens: 1024,
                    failure: None,
                },
            },
            Entry {
                path: "/p/clip.mp4".into(),
                status: Status::Included {
                    index: 2,
                    attachment: att(AttachmentKind::Video),
                    tokens: 12,
                    failure: Some("архитектура не принимает видео".into()),
                },
            },
            Entry {
                path: "/p/big.png".into(),
                status: Status::Skipped {
                    attachment: att(AttachmentKind::Image),
                    need: 4096,
                    allowed: 1000,
                },
            },
            Entry { path: "/p/dir".into(), status: Status::Failed("file not found".into()) },
        ];
        let s = summary(&entries);
        assert!(s.starts_with("view_media: 1 of 4 file(s) shown above"), "{s}");
        assert!(s.contains("[file 1] /p/me.png — image 1920×1080, 2.1 MB → 1024 vision tokens"), "{s}");
        assert!(s.contains("[file 2] /p/clip.mp4 — video 1920×1080, 2.1 MB; NOT shown: "), "{s}");
        assert!(s.contains("- /p/big.png — image 1920×1080, 2.1 MB; NOT shown: needs ~4096"), "{s}");
        assert!(s.contains("- /p/dir — NOT shown: file not found"), "{s}");
        let none = summary(&entries[3..]);
        assert!(none.starts_with("view_media: none of the 1 file(s)"), "{none}");
    }
}
