//! Превращение вложений в куски промпта.
//!
//! Каждая модальность попадает к модели своим путём:
//!
//! - **картинка / видео** — в текст сообщения встаёт маркер
//!   [`synaptixd::MEDIA_MARK`], а файл уходит в службу: её башня зрения
//!   кодирует его и ставит блок токенов ровно на место маркера (эмбеддинги
//!   служба кэширует сама);
//! - **документ** — содержимое инлайнится текстом в fenced-блок;
//! - **аудио** — транскрипт модели распознавания службы (кэшируется в
//!   `blobs/derived/<sha>.txt`), иначе — строка-заглушка;
//! - **прочее** — только имя и размер: модель хотя бы знает, что файл есть.
//!
//! С галочкой «передать модели путь к файлу» (`MsgAttachment::share_path`)
//! после блока вложения идёт строка с его путём на диске — чтобы модель
//! могла обработать файл инструментами, а не искать его.
//!
//! Если модель без зрения, картинки и видео деградируют до текстовой строки
//! — чат остаётся рабочим, просто модель их не «видит».

use std::path::Path;
#[cfg(test)]
use std::sync::Arc;

use crate::agent::state::{AttachmentKind, MsgAttachment};

use super::{blobs, format_duration, format_size};

/// Потолок инлайна документа без токенизатора (юнит-тесты, прогоны вне
/// чата): 60k символов ≈ 20k токенов. В чате действует [`DocBudget`] —
/// доля окна модели, посчитанная её токенизатором.
const DOC_INLINE_LIMIT: usize = 60_000;

/// Сколько документ-вложение может занять в промпте.
///
/// Считается при сборке промпта, когда остаток хода ещё не известен,
/// поэтому берётся от окна модели, а не от живого бюджета — и обязан быть
/// одинаковым от хода к ходу: блок вложения лежит в голове истории, и
/// плавающая граница обнуляла бы префикс-KV на каждом сообщении.
#[derive(Clone)]
pub struct DocBudget {
    /// Потолок на один документ в токенах.
    pub tokens: usize,
    /// Счётчик токенов — токенизатор загруженной модели.
    pub count: crate::agent::tools::budget::Counter,
}

/// Что модель умеет принимать в этой сессии.
#[derive(Clone)]
pub struct MediaCaps {
    /// Модель видит картинки и видео (у службы есть её башня зрения).
    pub vision: bool,
    /// Почему не видит (`None` — видит). Причина уходит в текстовую
    /// заглушку: «модель не видит картинку» без объяснения выглядит как глюк.
    pub vision_error: Option<String>,
    /// Потолок vision-токенов на картинку (`None` — как в конфиге модели).
    pub max_image_tokens: Option<usize>,
    /// Бюджет документа-вложения в токенах; `None` — потолок по символам
    /// ([`DOC_INLINE_LIMIT`]).
    pub doc_budget: Option<DocBudget>,
}

/// Готовое сообщение: текст для chat-шаблона (маркеры на местах картинок и
/// видео) + их файлы в порядке маркеров.
pub struct PreparedMessage {
    pub text: String,
    pub media: Vec<synaptixd::Media>,
}

/// Одно вложение в промпте: его кусок текста (маркер, документ, транскрипт
/// или строка-заглушка) и файл, если это картинка/видео.
pub struct Part {
    pub text: String,
    pub media: Option<synaptixd::Media>,
    /// Почему содержимое модели не досталось (`None` — досталось): в `text`
    /// тогда лежит строка-заглушка с этой же причиной.
    pub failure: Option<String>,
}

impl Part {
    fn failed(text: String, reason: impl Into<String>) -> Self {
        Self { text, media: None, failure: Some(reason.into()) }
    }
}

/// Кусок промпта под одно вложение — каждой модальности своим путём (см.
/// шапку модуля).
pub fn attachment_part(a: &MsgAttachment, caps: &MediaCaps) -> Part {
    let mut part = match a.kind {
        AttachmentKind::Image | AttachmentKind::Video => match media_file(a, caps) {
            Ok(m) => Part { text: format!("{}\n", synaptixd::MEDIA_MARK), media: Some(m), failure: None },
            Err(e) => {
                log::warn!("[attach] {}: {e}", a.original_name);
                Part::failed(fallback_line(a, Some(&e)), e)
            }
        },
        AttachmentKind::Document => document_part(a, caps),
        AttachmentKind::Audio => audio_part(a),
        AttachmentKind::Other => {
            Part::failed(fallback_line(a, None), "содержимое модели недоступно")
        }
    };
    if a.share_path {
        part.text.push_str(&path_line(a, part.failure.is_none()));
    }
    part
}

/// Собирает содержимое user-сообщения из вложений и текста.
///
/// Порядок важен: блоки вложений идут перед текстом реплики (так их видят
/// и HF-процессоры), а `media` — ровно в порядке маркеров в `text`.
pub fn prepare_user_message(body: &str, attachments: &[MsgAttachment], caps: &MediaCaps) -> PreparedMessage {
    let mut text = String::new();
    let mut media = Vec::new();
    for a in attachments {
        let part = attachment_part(a, caps);
        text.push_str(&part.text);
        media.extend(part.media);
    }
    if !body.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(body);
    }
    PreparedMessage { text, media }
}

/// Результат инструмента `view_media`: файлы, которые модель попросила
/// посмотреть, — каждый со своей подписью перед блоком (модель сравнивает
/// фото между собой и должна знать, какое из них какой файл), затем тело
/// результата со списком путей.
///
/// Одна функция на оба пути — ход агента собирает текст из уже посчитанных
/// кусков ([`assemble_tool_view`]), пересборка истории из ленты — отсюда;
/// текст обязан совпасть байт в байт, иначе префикс-KV обнулится.
pub fn prepare_tool_view(body: &str, attachments: &[MsgAttachment], caps: &MediaCaps) -> PreparedMessage {
    let parts: Vec<Part> = attachments.iter().map(|a| attachment_part(a, caps)).collect();
    let items: Vec<(&MsgAttachment, &str)> = attachments.iter().zip(&parts).map(|(a, p)| (a, p.text.as_str())).collect();
    let text = assemble_tool_view(body, &items);
    PreparedMessage { text, media: parts.into_iter().filter_map(|p| p.media).collect() }
}
/// Текст результата `view_media` из готовых кусков: `[file N: имя]`, кусок
/// вложения, и после всех файлов — тело результата.
pub fn assemble_tool_view(body: &str, items: &[(&MsgAttachment, &str)]) -> String {
    let mut text = String::new();
    for (i, (a, part)) in items.iter().enumerate() {
        text.push_str(&format!("[file {}: {}]\n", i + 1, display_name(a)));
        text.push_str(part);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    text.push_str(body);
    text
}

/// Строка с путём вложения на диске — пользователь отметил «передать модели
/// путь к файлу». Картинка уходит модели эмбеддингами, и без пути просьба
/// «обработай её» превращалась в поиск файла по диску инструментами. Путь —
/// оригинал в CAS: имя там — хеш, поэтому рядом исходное имя.
///
/// Если содержимое модели досталось (`delivered`), строка прямо говорит, что
/// файл уже перед ней, а путь — только для инструментов. Голая строка «путь
/// к файлу …» рядом с системным «tools are disabled» сбивала Qwen3.8: треть
/// ответов (23.09.2026, смоук на промпте чата MyLife) была «не могу открыть
/// файл по пути, инструменты отключены» — при картинке, которую модель
/// видела.
fn path_line(a: &MsgAttachment, delivered: bool) -> String {
    let name = display_name(a);
    let path = blobs::source_path(a);
    if !delivered {
        return format!("[file «{name}» on disk: {}]\n", path.display());
    }
    let what = match a.kind {
        AttachmentKind::Image | AttachmentKind::Video => "is shown above: you already see it",
        _ => "content is above",
    };
    format!(
        "[«{name}» {what}, no need to open it. Path on disk, only for tools that take a file: {}]\n",
        path.display()
    )
}

/// Файл картинки или видео для службы; без зрения у модели — причина.
fn media_file(a: &MsgAttachment, caps: &MediaCaps) -> Result<synaptixd::Media, String> {
    if !caps.vision {
        return Err(caps.vision_error.clone().unwrap_or_else(|| "модель без зрения".into()));
    }
    let path = blobs::model_path(a);
    if !path.exists() {
        return Err(format!("файл вложения не найден: {}", path.display()));
    }
    let kind = match a.kind {
        AttachmentKind::Video => synaptixd::MediaKind::Video,
        _ => synaptixd::MediaKind::Image,
    };
    Ok(synaptixd::Media { kind, path: path.display().to_string() })
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
///
/// Что не влезает в [`DocBudget`], режется как выхлоп инструмента
/// (`budget::fit_with`): голова и хвост по строкам, в середине — пометка с
/// номерами пропущенных строк и путём к файлу, чтобы модель дочитала их
/// инструментом `bash`, а не гадала по обрыву.
fn document_part(a: &MsgAttachment, caps: &MediaCaps) -> Part {
    let path = blobs::source_path(a);
    match read_document(&path) {
        Ok(text) if !text.trim().is_empty() => {
            let body = match &caps.doc_budget {
                Some(b) => fit_document(&text, b, &path.display().to_string()),
                None => {
                    let (body, truncated) = truncate_chars(&text, DOC_INLINE_LIMIT);
                    if truncated {
                        format!("{body}\n… (документ обрезан)")
                    } else {
                        body
                    }
                }
            };
            Part {
                text: format!("[документ: {}]\n```\n{body}\n```\n", display_name(a)),
                media: None,
                failure: None,
            }
        }
        Ok(_) => Part::failed(fallback_line(a, Some("документ пуст")), "документ пуст"),
        Err(e) => Part::failed(fallback_line(a, Some(&e)), e),
    }
}

/// Укладка документа в [`DocBudget`]; `path` — откуда дочитать вырезанное.
fn fit_document(text: &str, budget: &DocBudget, path: &str) -> String {
    let count = budget.count.clone();
    crate::agent::tools::budget::fit_with(text, budget.tokens, 0, false, &*count, |c| {
        format!(
            "…[документ обрезан по окну контекста: показаны строки 1–{} и {}–{} из {} \
             (~{} токенов при потолке ~{}); пропущены строки {}–{} — если они нужны, \
             прочитайте именно этот диапазон из файла {path} инструментом bash \
             (sed -n 'A,Bp'), не перечитывая всё]…\n",
            c.omitted.0 - 1,
            c.omitted.1 + 1,
            c.lines,
            c.lines,
            c.tokens,
            c.allowed,
            c.omitted.0,
            c.omitted.1,
        )
    })
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

/// Транскрипт аудио: сначала кэш на диске, потом модель распознавания службы.
fn audio_part(a: &MsgAttachment) -> Part {
    match transcribe(a) {
        Ok(text) if !text.trim().is_empty() => Part {
            text: format!(
                "[аудио: {} · {}]\nРасшифровка:\n```\n{}\n```\n",
                display_name(a),
                format_duration(a.duration_ms),
                text.trim()
            ),
            media: None,
            failure: None,
        },
        Ok(_) => {
            let why = "в записи не распознана речь";
            Part::failed(fallback_line(a, Some(why)), why)
        }
        Err(why) => Part::failed(fallback_line(a, Some(&why)), why),
    }
}

fn transcribe(a: &MsgAttachment) -> Result<String, String> {
    let cached = blobs::derived_path(&a.sha256, "txt");
    if let Ok(text) = std::fs::read_to_string(&cached) {
        return Ok(text);
    }
    // Расшифровываем ту самую WAV 16 кГц моно, которую подготовил ingest.
    let wav_path = blobs::derived_path(&a.sha256, "wav");
    let wav = std::fs::read(&wav_path).map_err(|e| format!("{}: {e}", wav_path.display()))?;
    let pcm = wav_pcm(&wav).ok_or("WAV вложения не разобран")?;
    let t0 = std::time::Instant::now();
    let text = synaptixd::Client::connect()
        .and_then(|mut c| c.transcribe("", &pcm, 16_000, None))
        .map(|t| t.text)
        .map_err(|e| format!("распознавание речи в службе: {e}"))?;
    log::info!("[attach] расшифровка {} за {:?}", a.original_name, t0.elapsed());
    if let Err(e) = blobs::write_derived(&a.sha256, "txt", text.as_bytes()) {
        log::warn!("[attach] кэш транскрипта: {e}");
    }
    Ok(text)
}

/// Отсчёты 16-битного PCM WAV (моно) в f32.
fn wav_pcm(wav: &[u8]) -> Option<Vec<f32>> {
    let mut i = 12;
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let len = u32::from_le_bytes(wav[i + 4..i + 8].try_into().ok()?) as usize;
        if id == b"data" {
            let data = wav.get(i + 8..(i + 8 + len).min(wav.len()))?;
            return Some(data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect());
        }
        i += 8 + len + (len & 1);
    }
    None
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
            share_path: false,
        }
    }

    /// Путь — оригинал в CAS (имя-хеш), рядом исходное имя файла.
    #[test]
    fn path_line_names_blob_and_original() {
        let a = att(AttachmentKind::Image, "photo.png");
        for delivered in [true, false] {
            let line = path_line(&a, delivered);
            assert!(line.contains("photo.png"), "{line}");
            assert!(line.contains(&blobs::source_path(&a).display().to_string()), "{line}");
            assert!(line.ends_with("]\n"), "{line}");
        }
        // Доставленная картинка: модель должна понять, что файл уже перед
        // ней, а не идти его открывать.
        assert!(path_line(&a, true).contains("already see it"));
        assert!(!path_line(&a, false).contains("already see it"));
    }

    /// Результат `view_media`: у каждого файла своя подпись перед блоком,
    /// номера совпадают со списком в теле, тело — последним.
    #[test]
    fn tool_view_labels_each_file_before_its_block() {
        let a = att(AttachmentKind::Image, "me.png");
        let b = att(AttachmentKind::Image, "trip.jpg");
        let text = assemble_tool_view(
            "view_media: 2 of 2 file(s) shown above, in order.\n",
            &[(&a, "<|image|>\n"), (&b, "[картинка: trip.jpg — не удалось]\n")],
        );
        assert_eq!(
            text,
            "[file 1: me.png]\n<|image|>\n[file 2: trip.jpg]\n[картинка: trip.jpg — не удалось]\n\
             view_media: 2 of 2 file(s) shown above, in order.\n"
        );
        // Кусок без перевода строки в конце не склеивается со следующей подписью.
        let glued = assemble_tool_view("", &[(&a, "x"), (&b, "y")]);
        assert_eq!(glued, "[file 1: me.png]\nx\n[file 2: trip.jpg]\ny\n");
    }

    #[test]
    fn fallback_mentions_name_and_size() {
        let line = fallback_line(&att(AttachmentKind::Image, "photo.png"), None);
        assert!(line.contains("photo.png"));
        assert!(line.contains("1920×1080"));
        assert!(line.contains("2,0 КБ"));
    }

    /// Документ меряется токенизатором: что влезает в бюджет — целиком,
    /// что нет — голова и хвост по строкам с номерами пропущенного и путём.
    #[test]
    fn document_fits_by_tokens_and_names_the_gap() {
        let budget = DocBudget {
            tokens: 1_000,
            count: Arc::new(|s: &str| s.chars().count() / 3),
        };
        let short: String = (1..=20).map(|i| format!("строка {i}\n")).collect();
        assert_eq!(fit_document(&short, &budget, "/tmp/a.md"), short);
        let long: String = (1..=2_000).map(|i| format!("| {i:04} | строка таблицы |\n")).collect();
        let out = fit_document(&long, &budget, "/tmp/big.md");
        assert!(out.starts_with("| 0001 |"));
        assert!(out.ends_with("| 2000 | строка таблицы |\n"));
        assert!(out.contains("документ обрезан по окну контекста"));
        assert!(out.contains("/tmp/big.md"));
        assert!((budget.count)(&out) <= 1_000, "{}", (budget.count)(&out));
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
