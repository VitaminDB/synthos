//! Вложения чата: приём файлов, CAS на диске, подготовка к промпту.
//!
//! ```text
//! пользователь                                     модель
//!     │ файловый диалог / drag&drop                    ▲
//!     ▼                                                │
//!  ingest ──► blobs/ (CAS: оригинал, derived, thumbs)  │
//!     │                                                │
//!     ▼                                                │
//!  ctx.pending_attachments ──► ChatMsg.attachments ──► prompt::*
//! ```
//!
//! - [`blobs`]  — раскладка content-addressed хранилища;
//! - [`ingest`] — детект типа, метаданные, превью, конвертация под модель;
//! - [`prompt`] — превращение вложений в куски промпта (vision-эмбеддинги,
//!   инлайн текста документов, транскрипт аудио).

pub mod blobs;
pub mod ingest;
pub mod media_cache;
pub mod prompt;

use std::path::PathBuf;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;

use crate::agent::state::MsgAttachment;

use super::state::SynChatCtx;

/// Открывает системный диалог и прикрепляет выбранные файлы.
///
/// Диалог и последующий ingest уходят на отдельный поток: rfd блокирует, а
/// хеширование/ffmpeg занимают секунды. UI на это время показывает счётчик
/// «готовится N файлов» (`ctx.attach_busy`).
pub fn pick_and_attach() {
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Прикрепить файлы")
            .pick_files();
        if let Some(paths) = picked {
            ingest_paths_blocking(paths);
        }
    });
}

/// Прикрепляет уже известный список путей (drag&drop из файлового менеджера).
pub fn attach_paths(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let ctx = use_context::<SynChatCtx>();
    ctx.attach_busy.update(|n| *n += paths.len());
    std::thread::spawn(move || ingest_paths_inner(paths));
}

/// Вариант для потока, где `use_context` недоступен: счётчик занятости
/// поднимается уже внутри `run_on_main_thread`.
fn ingest_paths_blocking(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let n = paths.len();
    run_on_main_thread(move || {
        use_context::<SynChatCtx>().attach_busy.update(|v| *v += n);
    });
    ingest_paths_inner(paths);
}

fn ingest_paths_inner(paths: Vec<PathBuf>) {
    for path in paths {
        let result = ingest::ingest(&path);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("файл")
            .to_string();
        run_on_main_thread(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.attach_busy.update(|n| *n = n.saturating_sub(1));
            match result {
                Ok(a) => {
                    ctx.pending_attachments.update(|list| {
                        // Один и тот же файл дважды в одном сообщении не нужен.
                        if !list.iter().any(|x| x.sha256 == a.sha256) {
                            list.push(a);
                        }
                    });
                }
                Err(e) => {
                    log::warn!("[attach] {name}: {e}");
                    ctx.error.set(Some(format!("Не удалось прикрепить {name}: {e}")));
                }
            }
        });
    }
}

/// Убирает вложение из черновика по его хешу.
pub fn remove_pending(sha: &str) {
    let ctx = use_context::<SynChatCtx>();
    let sha = sha.to_string();
    ctx.pending_attachments
        .update(|list| list.retain(|a| a.sha256 != sha));
}

/// Собирает хеши всех вложений во всех сохранённых чатах и подчищает CAS.
/// Вызывается после удаления чата.
pub fn gc_after_delete() {
    std::thread::spawn(|| {
        let mut referenced = std::collections::HashSet::new();
        for meta in super::storage::list_meta() {
            let Some(chat) = super::storage::load(&meta.id) else {
                continue;
            };
            for m in &chat.messages {
                for a in &m.attachments {
                    referenced.insert(a.sha256.clone());
                }
            }
        }
        // Чаты llama-эпохи лежат в соседнем каталоге и ссылаются на тот же CAS.
        for meta in crate::agent::storage::list_meta() {
            let Some(chat) = crate::agent::storage::load(&meta.id) else {
                continue;
            };
            for m in &chat.messages {
                for a in &m.attachments {
                    referenced.insert(a.sha256.clone());
                }
            }
        }
        // Заметки ссылаются на тот же CAS схемой `blob:<sha>` в markdown.
        crate::pages::notes::media::collect_blob_refs(&mut referenced);
        blobs::gc_unreferenced(&referenced);
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Форматирование для UI
// ─────────────────────────────────────────────────────────────────────────────

/// «12,4 МБ» — компактный размер файла для подписи карточки.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        return format!("{bytes} Б");
    }
    let (value, unit) = if b < KB * KB {
        (b / KB, "КБ")
    } else if b < KB * KB * KB {
        (b / (KB * KB), "МБ")
    } else {
        (b / (KB * KB * KB), "ГБ")
    };
    if value < 10.0 {
        format!("{value:.1} {unit}").replace('.', ",")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// «1:05» / «1:02:03» — длительность видео или аудио.
pub fn format_duration(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Прикидка, во сколько vision-токенов обойдётся вложение.
///
/// Именно прикидка: точное число даёт только сама vision-башня после
/// `smart_resize`, а она грузится лишь на время отправки. Здесь повторяется
/// её арифметика по метаданным — patch 14 × merge 2 = 28 px на токен (так
/// сконфигурированы Muse Glimmer и вся Qwen-VL-родня). Нужна, чтобы
/// пользователь до отправки видел, сколько контекста съедят вложения.
pub fn estimate_vision_tokens(a: &MsgAttachment, max_image_tokens: Option<usize>) -> usize {
    use crate::agent::state::AttachmentKind;
    /// patch_size × merge_size — сторона участка, дающего один токен.
    const TOKEN_PX: f64 = 28.0;
    /// `max_video_frame_tokens` из processor_config.json.
    const VIDEO_TOKENS_PER_GROUP: usize = 144;
    /// Видео сэмплируется в 2 fps и не длиннее 96 кадров, кадры сшиваются
    /// по два (`temporal_patch_size`).
    const VIDEO_FPS: f64 = 2.0;
    const VIDEO_MAX_FRAMES: f64 = 96.0;

    let grid = |w: u32, h: u32| -> usize {
        if w == 0 || h == 0 {
            return 0;
        }
        let gw = (w as f64 / TOKEN_PX).round().max(1.0);
        let gh = (h as f64 / TOKEN_PX).round().max(1.0);
        (gw * gh) as usize
    };

    match a.kind {
        AttachmentKind::Image => {
            let cap = max_image_tokens.unwrap_or(4096);
            grid(a.width, a.height).clamp(0, cap)
        }
        AttachmentKind::Video => {
            let secs = a.duration_ms as f64 / 1000.0;
            let frames = (secs * VIDEO_FPS).clamp(1.0, VIDEO_MAX_FRAMES);
            let groups = (frames / 2.0).ceil().max(1.0) as usize;
            let per_group = grid(a.width, a.height)
                .min(VIDEO_TOKENS_PER_GROUP)
                .max(1);
            groups * per_group
        }
        _ => 0,
    }
}

/// Короткая подпись под карточкой: размеры кадра или длительность + размер.
pub fn short_meta(a: &MsgAttachment) -> String {
    let size = format_size(a.size_bytes);
    match a.kind {
        crate::agent::state::AttachmentKind::Image if a.width > 0 => {
            format!("{}×{} · {size}", a.width, a.height)
        }
        crate::agent::state::AttachmentKind::Video
        | crate::agent::state::AttachmentKind::Audio
            if a.duration_ms > 0 =>
        {
            format!("{} · {size}", format_duration(a.duration_ms))
        }
        _ => size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(512), "512 Б");
        assert_eq!(format_size(2048), "2,0 КБ");
        assert_eq!(format_size(15 * 1024 * 1024), "15 МБ");
    }

    #[test]
    fn image_token_estimate_respects_cap() {
        use crate::agent::state::AttachmentKind;
        let mut a = MsgAttachment {
            sha256: "0".repeat(64),
            mime: "image/png".into(),
            original_name: "a.png".into(),
            width: 1400,
            height: 1400,
            size_bytes: 1,
            kind: AttachmentKind::Image,
            ext: "png".into(),
            duration_ms: 0,
            model_ext: String::new(),
            ui_ext: String::new(),
            has_thumb: false,
        };
        // 1400/28 = 50 по каждой стороне → 2500 токенов, но потолок ниже.
        assert_eq!(estimate_vision_tokens(&a, Some(1024)), 1024);
        a.width = 280;
        a.height = 280;
        assert_eq!(estimate_vision_tokens(&a, Some(1024)), 100);
    }

    #[test]
    fn durations_grow_to_hours() {
        assert_eq!(format_duration(65_000), "1:05");
        assert_eq!(format_duration(3_723_000), "1:02:03");
    }
}
