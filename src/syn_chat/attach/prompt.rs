//! Превращение вложений в куски промпта.
//!
//! Каждая модальность попадает к модели своим путём:
//!
//! - **картинка / видео** — vision-башня даёт эмбеддинги, а в текст
//!   сообщения встаёт блок токенов-заполнителей (`<|patch|>` / `<|video|>`);
//! - **документ** — содержимое инлайнится текстом в fenced-блок;
//! - **аудио** — транскрипт загруженной ASR-модели (кэшируется в
//!   `blobs/derived/<sha>.txt`), иначе — строка-заглушка;
//! - **прочее** — только имя и размер: модель хотя бы знает, что файл есть.
//!
//! Если модель без vision-башни, картинки и видео тоже деградируют до
//! текстовой строки — чат остаётся рабочим, просто модель их не «видит».

use std::path::Path;
use std::sync::Arc;

use syngui::core::sync::Mutex;

use synaptix::facade::asr::Transcriber;
use synaptix::facade::llm::{Llm, MediaEmbedding, MediaKind};

use crate::agent::state::{AttachmentKind, MsgAttachment};

use super::{blobs, format_duration, format_size, media_cache};

/// Потолок инлайна документа. 60k символов ≈ 20k токенов — дальше вложение
/// вытесняет из контекста саму переписку.
const DOC_INLINE_LIMIT: usize = 60_000;

/// Что модель умеет принимать в этой сессии.
#[derive(Clone)]
pub struct MediaCaps {
    /// Есть ли у загруженной модели vision-башня.
    pub vision: bool,
    /// Потолок vision-токенов на картинку (`None` — как в конфиге модели).
    pub max_image_tokens: Option<usize>,
    /// ASR-модель для расшифровки аудио-вложений; `None` — не загружена.
    pub asr: Option<Arc<Mutex<Option<Transcriber>>>>,
}

/// Готовое user-сообщение: текст для chat-шаблона + медиа в порядке
/// появления блоков в этом тексте.
pub struct PreparedMessage {
    pub text: String,
    pub media: Vec<MediaEmbedding>,
}

/// Собирает содержимое user-сообщения из вложений и текста.
///
/// Порядок важен: блоки вложений идут перед текстом реплики (так их видят
/// и HF-процессоры), а `media` возвращается ровно в том порядке, в каком
/// заполнители встречаются в `text` — на этом порядке держится разбор
/// эмбеддингов в `generate_streaming_media`.
pub fn prepare_user_message(
    body: &str,
    attachments: &[MsgAttachment],
    model: &Llm,
    caps: &MediaCaps,
) -> PreparedMessage {
    let mut text = String::new();
    let mut media: Vec<MediaEmbedding> = Vec::new();

    for a in attachments {
        match a.kind {
            AttachmentKind::Image | AttachmentKind::Video if caps.vision => {
                match encode_media(a, model, caps) {
                    Ok(emb) => {
                        text.push_str(&emb.prompt_block);
                        text.push('\n');
                        media.push(emb);
                    }
                    Err(e) => {
                        log::warn!("[attach] {}: {e}", a.original_name);
                        text.push_str(&fallback_line(a, Some(&e)));
                    }
                }
            }
            AttachmentKind::Image | AttachmentKind::Video => {
                text.push_str(&fallback_line(a, None));
            }
            AttachmentKind::Document => text.push_str(&document_block(a)),
            AttachmentKind::Audio => text.push_str(&audio_block(a, caps)),
            AttachmentKind::Other => text.push_str(&fallback_line(a, None)),
        }
    }

    if !body.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(body);
    }
    PreparedMessage { text, media }
}

/// Кодирует картинку/видео vision-башней, переиспользуя кэш.
///
/// Башню грузит вызывающий ([`ensure_tower`]) — здесь она уже должна быть
/// в памяти, иначе `encode_*` вернёт ошибку.
fn encode_media(
    a: &MsgAttachment,
    model: &Llm,
    caps: &MediaCaps,
) -> Result<MediaEmbedding, String> {
    let kind = match a.kind {
        AttachmentKind::Video => MediaKind::Video,
        _ => MediaKind::Image,
    };
    let limit = match kind {
        MediaKind::Image => caps.max_image_tokens,
        MediaKind::Video => None,
    };
    if let Some(hit) = media_cache::get(&a.sha256, kind, limit) {
        return Ok(hit);
    }
    let path = blobs::model_path(a);
    if !path.exists() {
        return Err(format!("файл вложения не найден: {}", path.display()));
    }
    let t0 = std::time::Instant::now();
    let emb = match kind {
        MediaKind::Image => model.encode_image(&path, limit),
        MediaKind::Video => model.encode_video(&path),
    }
    .map_err(|e| e.to_string())?;
    log::info!(
        "[attach] {} → {} vision-токенов за {:?}",
        a.original_name,
        emb.tokens,
        t0.elapsed()
    );
    media_cache::put(&a.sha256, kind, limit, &emb);
    Ok(emb)
}

/// Догружает vision-башню, если среди вложений есть картинки или видео.
/// Возвращает `true`, если башня в памяти и кодирование возможно.
pub fn ensure_tower(model: &Llm, needed: bool) -> bool {
    if !needed || !model.supports_media() {
        return false;
    }
    if model.media_tower_loaded() {
        return true;
    }
    let t0 = std::time::Instant::now();
    match model.ensure_media_tower() {
        Ok(true) => {
            log::info!("[attach] vision-башня загружена за {:?}", t0.elapsed());
            true
        }
        Ok(false) => {
            log::warn!("[attach] в бандле модели нет vision-башни");
            false
        }
        Err(e) => {
            log::warn!("[attach] не удалось загрузить vision-башню: {e}");
            false
        }
    }
}

/// Строка-заглушка: модель узнаёт о файле, даже когда прочесть его не может.
fn fallback_line(a: &MsgAttachment, err: Option<&str>) -> String {
    let name = display_name(a);
    let mut s = format!("[{}: {name}", a.kind.label().to_lowercase());
    if a.width > 0 && a.height > 0 {
        s.push_str(&format!(", {}×{}", a.width, a.height));
    }
    if a.duration_ms > 0 {
        s.push_str(&format!(", {}", format_duration(a.duration_ms)));
    }
    s.push_str(&format!(", {}", format_size(a.size_bytes)));
    match err {
        Some(e) => s.push_str(&format!(" — не удалось передать модели: {e}]\n")),
        None => s.push_str(" — содержимое модели недоступно]\n"),
    }
    s
}

fn display_name(a: &MsgAttachment) -> &str {
    if a.original_name.is_empty() {
        "без имени"
    } else {
        &a.original_name
    }
}

/// Текст документа в fenced-блоке с именем файла в заголовке.
fn document_block(a: &MsgAttachment) -> String {
    let path = blobs::source_path(a);
    match read_document(&path) {
        Ok(text) if !text.trim().is_empty() => {
            let (body, truncated) = truncate_chars(&text, DOC_INLINE_LIMIT);
            let mut s = format!("[документ: {}]\n```\n{body}", display_name(a));
            if truncated {
                s.push_str("\n… (документ обрезан)");
            }
            s.push_str("\n```\n");
            s
        }
        Ok(_) => fallback_line(a, Some("документ пуст")),
        Err(e) => fallback_line(a, Some(&e)),
    }
}

/// Парсинг документа: markdown/html/pdf разбираются синаптиксовым
/// doc-парсером, всё остальное читается как UTF-8-текст.
fn read_document(path: &Path) -> Result<String, String> {
    use synaptix_rag::doc::{parse, SourceKind};
    let bytes = std::fs::read(path).map_err(|e| format!("чтение: {e}"))?;
    match SourceKind::from_path(path) {
        Some(kind) => parse(&bytes, kind).map(|d| d.plain_text),
        None => Ok(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

/// Обрезка по границе символа. Возвращает `(текст, была_ли_обрезка)`.
fn truncate_chars(s: &str, limit: usize) -> (String, bool) {
    if s.chars().count() <= limit {
        return (s.to_string(), false);
    }
    (s.chars().take(limit).collect(), true)
}

/// Транскрипт аудио: сначала кэш на диске, потом загруженная ASR-модель.
fn audio_block(a: &MsgAttachment, caps: &MediaCaps) -> String {
    match transcribe(a, caps) {
        Some(text) if !text.trim().is_empty() => format!(
            "[аудио: {} · {}]\nРасшифровка:\n```\n{}\n```\n",
            display_name(a),
            format_duration(a.duration_ms),
            text.trim()
        ),
        Some(_) => fallback_line(a, Some("в записи не распознана речь")),
        None => fallback_line(a, Some("ASR-модель не загружена")),
    }
}

fn transcribe(a: &MsgAttachment, caps: &MediaCaps) -> Option<String> {
    let cached = blobs::derived_path(&a.sha256, "txt");
    if let Ok(text) = std::fs::read_to_string(&cached) {
        return Some(text);
    }
    let asr = caps.asr.as_ref()?;
    // Расшифровываем ту самую WAV 16 кГц моно, которую подготовил ingest.
    let wav_path = blobs::derived_path(&a.sha256, "wav");
    let wav = std::fs::read(&wav_path).ok()?;
    let mut guard = asr.lock().ok()?;
    let transcriber = guard.as_mut()?;
    let t0 = std::time::Instant::now();
    match transcriber.transcribe_wav(&wav) {
        Ok(text) => {
            log::info!("[attach] расшифровка {} за {:?}", a.original_name, t0.elapsed());
            if let Err(e) = blobs::write_derived(&a.sha256, "txt", text.as_bytes()) {
                log::warn!("[attach] кэш транскрипта: {e}");
            }
            Some(text)
        }
        Err(e) => {
            log::warn!("[attach] ASR {}: {e}", a.original_name);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(kind: AttachmentKind, name: &str) -> MsgAttachment {
        MsgAttachment {
            sha256: "0".repeat(64),
            mime: "application/octet-stream".into(),
            original_name: name.into(),
            width: 1920,
            height: 1080,
            size_bytes: 2048,
            kind,
            ext: "bin".into(),
            duration_ms: 0,
            model_ext: String::new(),
            ui_ext: String::new(),
            has_thumb: false,
        }
    }

    #[test]
    fn fallback_mentions_name_and_size() {
        let line = fallback_line(&att(AttachmentKind::Image, "photo.png"), None);
        assert!(line.contains("photo.png"));
        assert!(line.contains("1920×1080"));
        assert!(line.contains("2,0 КБ"));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let (s, cut) = truncate_chars("привет мир", 6);
        assert!(cut);
        assert_eq!(s, "привет");
        let (s, cut) = truncate_chars("abc", 10);
        assert!(!cut);
        assert_eq!(s, "abc");
    }
}
