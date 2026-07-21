//! CAS-хранилище прикреплённых пользователем картинок.
//!
//! Файлы лежат в `~/.config/synthos/blobs/<sha256>.<ext>`. Имя — это hex
//! от sha256 содержимого, поэтому повторное прикрепление одной и той же
//! картинки не дублирует файл. JSON чата хранит только лёгкие метаданные
//! ([`super::state::MsgAttachment`]), бинарь живёт здесь.
//!
//! Все ошибки безопасные: не паникуют, логируются (`log::warn!`/`error!`)
//! и возвращаются как `io::Error`. Удаление blob-файла снаружи приложения
//! не ломает рендер — пропавшая картинка показывается через `Failed`-
//! плейсхолдер syngui `Image`-виджета.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::{Digest, Sha256};

use super::state::MsgAttachment;

// ─────────────────────────────────────────────────────────────────────────────
// Пути
// ─────────────────────────────────────────────────────────────────────────────

/// Каталог CAS-хранилища: `~/.config/synthos/blobs/`. Создаётся при первом
/// `write_if_absent`. Всегда абсолютный путь, безопасен для `Image::new`.
pub fn blobs_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/blobs")
}

/// Полный путь до конкретного blob'а.
pub fn full_path(att: &MsgAttachment) -> PathBuf {
    blobs_dir().join(att.rel_path())
}

// ─────────────────────────────────────────────────────────────────────────────
// MIME / extension
// ─────────────────────────────────────────────────────────────────────────────

/// Расширение для файла на диске, зависит от MIME. Для неизвестных типов —
/// `bin`, чтобы операция write не падала.
pub fn ext_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

/// Угадать MIME по расширению пути или, если по расширению неизвестно,
/// по магическим байтам через `image::guess_format`. Возвращает строку вида
/// `"image/png"`.
pub fn sniff_mime(path: &Path, bytes: &[u8]) -> String {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "png" => return "image/png".into(),
            "jpg" | "jpeg" => return "image/jpeg".into(),
            "webp" => return "image/webp".into(),
            "gif" => return "image/gif".into(),
            "bmp" => return "image/bmp".into(),
            _ => {}
        }
    }
    if let Ok(fmt) = image::guess_format(bytes) {
        return match fmt {
            image::ImageFormat::Png => "image/png".into(),
            image::ImageFormat::Jpeg => "image/jpeg".into(),
            image::ImageFormat::WebP => "image/webp".into(),
            image::ImageFormat::Gif => "image/gif".into(),
            image::ImageFormat::Bmp => "image/bmp".into(),
            other => format!("application/{:?}", other).to_lowercase(),
        };
    }
    "application/octet-stream".into()
}

// ─────────────────────────────────────────────────────────────────────────────
// CAS-операции
// ─────────────────────────────────────────────────────────────────────────────

/// Записать байты в CAS, если ещё нет. Идемпотентно: одни и те же байты
/// дают один и тот же sha256, поэтому второй вызов с теми же данными
/// просто переиспользует существующий файл.
///
/// Возвращает [`MsgAttachment`] с заполненными `sha256`, `mime`,
/// `original_name`, `width`/`height` (декодированы один раз через
/// `image::load_from_memory` — могут быть `0/0`, если декод не удался)
/// и `size_bytes`.
pub fn write_if_absent(
    bytes: &[u8],
    mime: &str,
    original_name: &str,
) -> io::Result<MsgAttachment> {
    let dir = blobs_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }

    let sha256 = hex_sha256(bytes);
    let ext = ext_from_mime(mime);
    let path = dir.join(format!("{sha256}.{ext}"));

    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }

    let (width, height) = match image::load_from_memory(bytes) {
        Ok(img) => (img.width(), img.height()),
        Err(e) => {
            log::warn!("blob {sha256}: не удалось получить размеры: {e}");
            (0, 0)
        }
    };

    Ok(MsgAttachment {
        sha256,
        mime: mime.to_string(),
        original_name: original_name.to_string(),
        width,
        height,
        size_bytes: bytes.len() as u64,
    })
}

/// Прочитать байты blob'а с диска.
pub fn read(att: &MsgAttachment) -> io::Result<Vec<u8>> {
    let path = full_path(att);
    if !path.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("blob {} отсутствует", att.sha256),
        ));
    }
    std::fs::read(path)
}

/// Сформировать `data:` URL для отправки в multipart-сообщение к LLM.
/// Это синхронная операция: читает файл, base64-кодирует, склеивает.
/// Для очень больших картинок (>10 MB) можно подумать про потоковую
/// upload-загрузку, но пока llama-server и API ждут именно data: URL.
pub fn data_url(att: &MsgAttachment) -> io::Result<String> {
    let bytes = read(att)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", att.mime, b64))
}

/// Сформировать `file://` URL — пригодится, когда хочется сослаться на blob
/// из markdown'а ассистента (`![alt](file:///...)`) или из syngui Image.
pub fn file_url(att: &MsgAttachment) -> String {
    format!("file://{}", full_path(att).display())
}

// ─────────────────────────────────────────────────────────────────────────────
// Внутреннее
// ─────────────────────────────────────────────────────────────────────────────

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_from_mime_known() {
        assert_eq!(ext_from_mime("image/png"), "png");
        assert_eq!(ext_from_mime("image/jpeg"), "jpg");
        assert_eq!(ext_from_mime("image/webp"), "webp");
        assert_eq!(ext_from_mime("application/octet-stream"), "bin");
    }

    #[test]
    fn sniff_mime_by_extension() {
        let p = std::path::Path::new("/tmp/x.png");
        assert_eq!(sniff_mime(p, &[]), "image/png");
    }

    #[test]
    fn sha256_is_deterministic() {
        let a = hex_sha256(b"hello");
        let b = hex_sha256(b"hello");
        assert_eq!(a, b);
        // Известный hash «hello»:
        assert_eq!(a, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }
}
