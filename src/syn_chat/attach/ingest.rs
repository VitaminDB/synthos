//! Приём файла во вложения: детект модальности, метаданные, thumbnail и
//! конвертация «под модель».
//!
//! Работает синхронно и блокирующе — вызывать только с worker-потока
//! (см. [`super::pick_and_attach`]): хеширование гигабайтного видео и
//! вызовы ffmpeg занимают секунды.
//!
//! Что делается для каждой модальности:
//!
//! | тип       | метаданные            | thumbnail          | под модель            |
//! |-----------|-----------------------|--------------------|-----------------------|
//! | картинка  | `image` (или ffprobe) | `image` / ffmpeg   | PNG, если формат чужой|
//! | видео     | ffprobe               | кадр через ffmpeg  | оригинал (ffmpeg сам) |
//! | аудио     | ffprobe               | —                  | WAV 16 кГц моно (ASR) |
//! | документ  | —                     | —                  | текст при сборке промпта |
//!
//! «Чужой формат» для картинки — всё, кроме PNG/JPEG/WebP: ровно эти три
//! декодирует `synaptix-io`, который стоит за vision-башней.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use crate::agent::state::{AttachmentKind, MsgAttachment};

use super::blobs::{self, THUMB_MAX_SIDE};

/// Форматы картинок, которые `synaptix-io` читает сам — конвертация под
/// модель не нужна.
const MODEL_READY_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Форматы, которые умеет декодер картинок syngui. Всё остальное нужно
/// заранее превратить в PNG, иначе превью и просмотрщик покажут пустоту
/// (у syngui собраны png/jpeg/gif/bmp/ico — WebP там нет).
const UI_READY_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "ico"];

/// Кадр для постера видео: 1 секунда от начала — первый кадр часто чёрный.
const VIDEO_POSTER_SEC: f64 = 1.0;

/// Прикрепляет файл: кладёт в CAS, снимает метаданные, готовит превью и
/// модельную копию.
pub fn ingest(path: &Path) -> Result<MsgAttachment, String> {
    if !path.is_file() {
        return Err(format!("{} — не файл", path.display()));
    }
    let (sha256, size_bytes, ext) = blobs::store_file(path)?;
    let original_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let (kind, mime) = classify(&ext, &blobs::blob_path(&sha256, &ext));

    let mut a = MsgAttachment {
        sha256,
        mime,
        original_name,
        width: 0,
        height: 0,
        size_bytes,
        kind,
        ext,
        duration_ms: 0,
        model_ext: String::new(),
        ui_ext: String::new(),
        has_thumb: false,
        share_path: false,
    };

    let blob = blobs::source_path(&a);
    match kind {
        AttachmentKind::Image => enrich_image(&mut a, &blob),
        AttachmentKind::Video => enrich_video(&mut a, &blob),
        AttachmentKind::Audio => enrich_audio(&mut a, &blob),
        AttachmentKind::Document | AttachmentKind::Other => {}
    }
    Ok(a)
}

// ─────────────────────────────────────────────────────────────────────────────
// Классификация
// ─────────────────────────────────────────────────────────────────────────────

/// Модальность + MIME по расширению; для файлов без расширения — по
/// сигнатуре первых байт.
fn classify(ext: &str, path: &Path) -> (AttachmentKind, String) {
    // `.ts` — это и TypeScript, и MPEG transport stream. Расширение не
    // различает, а первый байт различает: у TS-контейнера это sync 0x47.
    if ext == "ts" && first_byte(path) == Some(0x47) {
        return (AttachmentKind::Video, "video/mp2t".into());
    }
    if let Some(hit) = classify_ext(ext) {
        return hit;
    }
    match sniff_magic(path) {
        Some(hit) => hit,
        None => (AttachmentKind::Other, "application/octet-stream".into()),
    }
}

fn classify_ext(ext: &str) -> Option<(AttachmentKind, String)> {
    let img = |m: &str| Some((AttachmentKind::Image, m.to_string()));
    let vid = |m: &str| Some((AttachmentKind::Video, m.to_string()));
    let aud = |m: &str| Some((AttachmentKind::Audio, m.to_string()));
    let doc = |m: &str| Some((AttachmentKind::Document, m.to_string()));
    match ext {
        "png" => img("image/png"),
        "jpg" | "jpeg" | "jpe" => img("image/jpeg"),
        "webp" => img("image/webp"),
        "gif" => img("image/gif"),
        "bmp" => img("image/bmp"),
        "tif" | "tiff" => img("image/tiff"),
        "avif" => img("image/avif"),
        "heic" | "heif" => img("image/heic"),
        "ico" => img("image/x-icon"),
        "tga" => img("image/x-tga"),
        "ppm" | "pgm" | "pnm" => img("image/x-portable-anymap"),

        "mp4" | "m4v" => vid("video/mp4"),
        "mkv" => vid("video/x-matroska"),
        "webm" => vid("video/webm"),
        "mov" => vid("video/quicktime"),
        "avi" => vid("video/x-msvideo"),
        "mpg" | "mpeg" => vid("video/mpeg"),
        "wmv" => vid("video/x-ms-wmv"),
        "flv" => vid("video/x-flv"),
        "m2ts" | "mts" => vid("video/mp2t"),
        "ogv" => vid("video/ogg"),
        "3gp" => vid("video/3gpp"),

        "mp3" => aud("audio/mpeg"),
        "wav" => aud("audio/wav"),
        "flac" => aud("audio/flac"),
        "ogg" | "oga" => aud("audio/ogg"),
        "opus" => aud("audio/opus"),
        "m4a" => aud("audio/mp4"),
        "aac" => aud("audio/aac"),
        "wma" => aud("audio/x-ms-wma"),
        "aiff" | "aif" => aud("audio/aiff"),

        "pdf" => doc("application/pdf"),
        "md" | "markdown" => doc("text/markdown"),
        "html" | "htm" => doc("text/html"),
        "svg" => doc("image/svg+xml"),
        "txt" | "text" | "log" => doc("text/plain"),
        "json" => doc("application/json"),
        "yaml" | "yml" => doc("application/yaml"),
        "toml" => doc("application/toml"),
        "csv" => doc("text/csv"),
        "tsv" => doc("text/tab-separated-values"),
        "xml" => doc("application/xml"),
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cc" | "go"
        | "java" | "kt" | "rb" | "php" | "sh" | "bash" | "zsh" | "sql" | "ini" | "conf"
        | "cfg" | "lua" | "swift" | "scala" | "cs" | "vue" | "svelte" | "mss" | "css" => {
            doc("text/plain")
        }
        _ => None,
    }
}

fn first_byte(path: &Path) -> Option<u8> {
    use std::io::Read;
    let mut b = [0u8; 1];
    std::fs::File::open(path).ok()?.read_exact(&mut b).ok()?;
    Some(b[0])
}

/// Мини-детект по сигнатуре — на случай файла без расширения.
fn sniff_magic(path: &Path) -> Option<(AttachmentKind, String)> {
    use std::io::Read;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .ok()?
        .read(&mut head)
        .ok()?;
    let head = &head[..n];
    let starts = |sig: &[u8]| head.len() >= sig.len() && &head[..sig.len()] == sig;
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some((AttachmentKind::Image, "image/png".into()));
    }
    if starts(b"\xff\xd8\xff") {
        return Some((AttachmentKind::Image, "image/jpeg".into()));
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some((AttachmentKind::Image, "image/gif".into()));
    }
    if starts(b"BM") {
        return Some((AttachmentKind::Image, "image/bmp".into()));
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some((AttachmentKind::Image, "image/webp".into()));
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        return Some((AttachmentKind::Audio, "audio/wav".into()));
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some((AttachmentKind::Video, "video/mp4".into()));
    }
    if starts(b"\x1a\x45\xdf\xa3") {
        return Some((AttachmentKind::Video, "video/x-matroska".into()));
    }
    if starts(b"%PDF-") {
        return Some((AttachmentKind::Document, "application/pdf".into()));
    }
    if head.is_ascii() && !head.is_empty() {
        return Some((AttachmentKind::Document, "text/plain".into()));
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Картинки
// ─────────────────────────────────────────────────────────────────────────────

fn enrich_image(a: &mut MsgAttachment, blob: &Path) {
    // Основной путь: декодируем сами. Он же даёт корректные размеры с учётом
    // EXIF-поворота и исходник для thumbnail'а.
    match decode_image(blob) {
        Ok(img) => {
            a.width = img.width();
            a.height = img.height();
            // Одна PNG-копия закрывает обе дыры: чужой формат для модели и
            // чужой формат для декодера UI.
            let need_model = !MODEL_READY_IMAGE_EXTS.contains(&a.ext.as_str());
            let need_ui = !UI_READY_IMAGE_EXTS.contains(&a.ext.as_str());
            if need_model || need_ui {
                match save_png(&img, a, "png") {
                    Ok(()) => {
                        if need_model {
                            a.model_ext = "png".into();
                        }
                        if need_ui {
                            a.ui_ext = "png".into();
                        }
                    }
                    Err(e) => log::warn!("[attach] конверсия {} в PNG: {e}", a.original_name),
                }
            }
            write_thumb(a, &img);
        }
        // Формат, который не читает `image` (HEIC/AVIF/TIFF-экзотика) —
        // прогоняем через ffmpeg в PNG и работаем уже с ним.
        Err(err) => {
            log::info!("[attach] {} не декодируется напрямую ({err}) — пробуем ffmpeg", a.original_name);
            match ffmpeg_to_png(a, blob) {
                Ok(png) => {
                    a.model_ext = "png".into();
                    a.ui_ext = "png".into();
                    if let Ok(img) = decode_image(&png) {
                        a.width = img.width();
                        a.height = img.height();
                        write_thumb(a, &img);
                    }
                }
                Err(e) => {
                    log::warn!("[attach] {}: ни image, ни ffmpeg не открыли файл: {e}", a.original_name);
                    // Остаётся вложением-файлом: превью не будет, в промпт
                    // уйдёт как «прочий файл».
                    a.kind = AttachmentKind::Other;
                }
            }
        }
    }
}

/// Декод с применением EXIF-ориентации — иначе фото с телефона висит боком.
fn decode_image(path: &Path) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("open: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("format: {e}"))?;
    let mut decoder = reader.into_decoder().map_err(|e| format!("decoder: {e}"))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img =
        image::DynamicImage::from_decoder(decoder).map_err(|e| format!("decode: {e}"))?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn save_png(img: &image::DynamicImage, a: &MsgAttachment, ext: &str) -> Result<(), String> {
    std::fs::create_dir_all(blobs::derived_dir()).map_err(|e| format!("derived dir: {e}"))?;
    let dst = blobs::derived_path(&a.sha256, ext);
    if dst.exists() {
        return Ok(());
    }
    img.save(&dst).map_err(|e| format!("save {}: {e}", dst.display()))
}

fn write_thumb(a: &mut MsgAttachment, img: &image::DynamicImage) {
    if img.width() <= THUMB_MAX_SIDE && img.height() <= THUMB_MAX_SIDE {
        // Мелкая картинка: карточка отрисует сам blob, лишний файл не нужен.
        return;
    }
    let Ok(dst) = blobs::prepare_thumb(&a.sha256) else {
        return;
    };
    if dst.exists() {
        a.has_thumb = true;
        return;
    }
    let thumb = img.thumbnail(THUMB_MAX_SIDE, THUMB_MAX_SIDE);
    match thumb.save(&dst) {
        Ok(()) => a.has_thumb = true,
        Err(e) => log::warn!("[attach] thumbnail {}: {e}", a.original_name),
    }
}

fn ffmpeg_to_png(a: &MsgAttachment, src: &Path) -> Result<std::path::PathBuf, String> {
    let dst = blobs::derived_path(&a.sha256, "png");
    if dst.exists() {
        return Ok(dst);
    }
    std::fs::create_dir_all(blobs::derived_dir()).map_err(|e| format!("derived dir: {e}"))?;
    run_ffmpeg(&[
        "-y".as_ref(),
        "-v".as_ref(),
        "error".as_ref(),
        "-i".as_ref(),
        src.as_os_str(),
        "-frames:v".as_ref(),
        "1".as_ref(),
        dst.as_os_str(),
    ])?;
    Ok(dst)
}

// ─────────────────────────────────────────────────────────────────────────────
// Видео и аудио
// ─────────────────────────────────────────────────────────────────────────────

fn enrich_video(a: &mut MsgAttachment, blob: &Path) {
    if let Some(info) = ffprobe(blob) {
        a.width = info.width;
        a.height = info.height;
        a.duration_ms = info.duration_ms;
    }
    // Постер: кадр на VIDEO_POSTER_SEC (или в начале, если ролик короче),
    // уменьшенный по длинной стороне.
    let Ok(dst) = blobs::prepare_thumb(&a.sha256) else {
        return;
    };
    if dst.exists() {
        a.has_thumb = true;
        return;
    }
    let seek = if a.duration_ms > 0 && (a.duration_ms as f64) < VIDEO_POSTER_SEC * 1000.0 {
        0.0
    } else {
        VIDEO_POSTER_SEC
    };
    let scale = format!(
        "scale='min({THUMB_MAX_SIDE},iw)':-2:force_original_aspect_ratio=decrease"
    );
    let res = run_ffmpeg(&[
        "-y".as_ref(),
        "-v".as_ref(),
        "error".as_ref(),
        "-ss".as_ref(),
        format!("{seek}").as_ref(),
        "-i".as_ref(),
        blob.as_os_str(),
        "-frames:v".as_ref(),
        "1".as_ref(),
        "-vf".as_ref(),
        scale.as_ref(),
        dst.as_os_str(),
    ]);
    match res {
        Ok(()) => a.has_thumb = dst.exists(),
        Err(e) => log::warn!("[attach] постер для {}: {e}", a.original_name),
    }
}

fn enrich_audio(a: &mut MsgAttachment, blob: &Path) {
    if let Some(info) = ffprobe(blob) {
        a.duration_ms = info.duration_ms;
    }
    // WAV 16 кГц моно — формат, который принимает ASR-фасад (`transcribe_wav`).
    let dst = blobs::derived_path(&a.sha256, "wav");
    if dst.exists() {
        a.model_ext = "wav".into();
        return;
    }
    if std::fs::create_dir_all(blobs::derived_dir()).is_err() {
        return;
    }
    let res = run_ffmpeg(&[
        "-y".as_ref(),
        "-v".as_ref(),
        "error".as_ref(),
        "-i".as_ref(),
        blob.as_os_str(),
        "-ac".as_ref(),
        "1".as_ref(),
        "-ar".as_ref(),
        "16000".as_ref(),
        "-f".as_ref(),
        "wav".as_ref(),
        dst.as_os_str(),
    ]);
    match res {
        Ok(()) if dst.exists() => a.model_ext = "wav".into(),
        Ok(()) => {}
        Err(e) => log::warn!("[attach] перекодирование {} в WAV: {e}", a.original_name),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ffmpeg / ffprobe
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) struct ProbeInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
}

/// Есть ли ffmpeg/ffprobe в PATH. Проверяется один раз за процесс.
pub fn ffmpeg_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let ok = |bin: &str| {
            Command::new(bin)
                .arg("-version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        let ok = ok("ffmpeg") && ok("ffprobe");
        if !ok {
            log::warn!(
                "[attach] ffmpeg/ffprobe не найдены — превью видео и \
                 перекодирование недоступны"
            );
        }
        ok
    })
}

fn run_ffmpeg(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    if !ffmpeg_available() {
        return Err("ffmpeg не найден в PATH".into());
    }
    let out = Command::new("ffmpeg")
        .args(args)
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Размеры и длительность через `ffprobe -of json`.
pub(crate) fn ffprobe(path: &Path) -> Option<ProbeInfo> {
    if !ffmpeg_available() {
        return None;
    }
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        log::warn!(
            "[attach] ffprobe {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let streams = v.get("streams")?.as_array()?;
    let video = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("video"));
    let (width, height) = match video {
        Some(s) => (
            s.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            s.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        ),
        None => (0, 0),
    };
    // Длительность живёт в format, а у некоторых контейнеров — только в потоке.
    let dur = v
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| {
            streams.iter().find_map(|s| {
                s.get("duration")
                    .and_then(|d| d.as_str())
                    .and_then(|d| d.parse::<f64>().ok())
            })
        })
        .unwrap_or(0.0);
    Some(ProbeInfo {
        width,
        height,
        duration_ms: (dur * 1000.0).max(0.0) as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify_ext("png").unwrap().0, AttachmentKind::Image);
        assert_eq!(classify_ext("mkv").unwrap().0, AttachmentKind::Video);
        assert_eq!(classify_ext("flac").unwrap().0, AttachmentKind::Audio);
        assert_eq!(classify_ext("rs").unwrap().0, AttachmentKind::Document);
        assert!(classify_ext("bin").is_none());
    }

    #[test]
    fn sniffs_png_without_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("noext");
        std::fs::write(&p, b"\x89PNG\r\n\x1a\n....").unwrap();
        assert_eq!(sniff_magic(&p).unwrap().0, AttachmentKind::Image);
    }

    #[test]
    fn model_ready_exts_need_no_conversion() {
        for e in MODEL_READY_IMAGE_EXTS {
            assert_eq!(classify_ext(e).unwrap().0, AttachmentKind::Image);
        }
    }
}
