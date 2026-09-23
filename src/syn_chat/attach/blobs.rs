//! Content-addressed хранилище вложений.
//!
//! Раскладка (`~/.config/synthos/blobs/`):
//!
//! ```text
//! blobs/
//!   <sha256>.<ext>          — оригинал как его дал пользователь
//!   derived/<sha256>.<ext>  — конвертация «под модель» (PNG для картинки в
//!                             формате, который не читает synaptix-io; WAV
//!                             16 кГц моно для аудио)
//!   thumbs/<sha256>.png     — превью ≤ THUMB_MAX_SIDE px для карточек ленты
//! ```
//!
//! Имя = sha256 содержимого, поэтому один и тот же файл, прикреплённый в
//! десяти чатах, лежит на диске один раз, а JSON чата хранит только
//! метаданные ([`MsgAttachment`]).

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::agent::state::MsgAttachment;

/// Максимальная сторона thumbnail'а. 512 хватает и для карточки 120 px в
/// strip'е, и для крупной плитки в bubble на HiDPI.
pub const THUMB_MAX_SIDE: u32 = 512;

/// Корень CAS. Рядом с `chats/`, `syn_chats/`, `kb/`.
pub fn blobs_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/blobs")
}

pub fn derived_dir() -> PathBuf {
    blobs_dir().join("derived")
}

pub fn thumbs_dir() -> PathBuf {
    blobs_dir().join("thumbs")
}

fn named(dir: PathBuf, sha: &str, ext: &str) -> PathBuf {
    if ext.is_empty() {
        dir.join(sha)
    } else {
        dir.join(format!("{sha}.{ext}"))
    }
}

/// Путь к оригинальному blob'у.
pub fn blob_path(sha: &str, ext: &str) -> PathBuf {
    named(blobs_dir(), sha, ext)
}

/// Путь к конвертированной «под модель» копии.
pub fn derived_path(sha: &str, ext: &str) -> PathBuf {
    named(derived_dir(), sha, ext)
}

/// Путь к thumbnail'у (всегда PNG).
pub fn thumb_path(sha: &str) -> PathBuf {
    thumbs_dir().join(format!("{sha}.png"))
}

/// Оригинал вложения — то, что открывается в просмотрщике и проигрывается.
pub fn source_path(a: &MsgAttachment) -> PathBuf {
    blob_path(&a.sha256, &a.ext)
}

/// Файл, который скармливается модели: конвертированная копия, если она
/// создавалась при прикреплении, иначе — оригинал.
pub fn model_path(a: &MsgAttachment) -> PathBuf {
    if a.model_ext.is_empty() {
        source_path(a)
    } else {
        derived_path(&a.sha256, &a.model_ext)
    }
}

/// Полноразмерная картинка для показа в UI: PNG-копия, если декодер syngui
/// не понимает исходный формат, иначе — сам blob.
pub fn display_path(a: &MsgAttachment) -> PathBuf {
    if a.ui_ext.is_empty() {
        source_path(a)
    } else {
        derived_path(&a.sha256, &a.ui_ext)
    }
}

/// Картинка для превью-карточки: сгенерированный thumbnail, иначе (для
/// небольших картинок, где он не нужен) — полноразмерная копия. `None` —
/// превью нет, рисуем иконку по типу файла.
pub fn preview_path(a: &MsgAttachment) -> Option<PathBuf> {
    if a.has_thumb {
        let p = thumb_path(&a.sha256);
        if p.exists() {
            return Some(p);
        }
    }
    if a.kind.has_thumbnail() || is_svg(a) {
        let p = display_path(a);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// SVG формально документ (модель читает его как текст), но syngui умеет
/// его растеризовать — значит, показываем как картинку.
pub fn is_svg(a: &MsgAttachment) -> bool {
    a.ext == "svg" || a.mime == "image/svg+xml"
}

/// Кладёт файл в CAS. Возвращает `(sha256_hex, размер, расширение)`.
///
/// Копирование пропускается, если blob уже есть — контент адресуется хешем,
/// так что повторное прикрепление того же файла бесплатно.
pub fn store_file(src: &Path) -> Result<(String, u64, String), String> {
    let sha = sha256_file(src)?;
    let size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let dst = blob_path(&sha, &ext);
    if !dst.exists() {
        std::fs::create_dir_all(blobs_dir()).map_err(|e| format!("blobs dir: {e}"))?;
        // Атомарно: имя — хеш, и обрезанная копия (сбой, ENOSPC) иначе
        // навсегда сошла бы за готовый blob — `exists()` выше её пропустит.
        crate::fsutil::copy_atomic(src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
    }
    Ok((sha, size, ext))
}

/// Записывает произвольные байты как derived-файл (конвертация под модель).
pub fn write_derived(sha: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(derived_dir()).map_err(|e| format!("derived dir: {e}"))?;
    let p = derived_path(sha, ext);
    crate::fsutil::write_atomic(&p, bytes).map_err(|e| format!("write {}: {e}", p.display()))?;
    Ok(p)
}

/// Готовит каталог thumbnail'ов и возвращает путь, куда его писать.
pub fn prepare_thumb(sha: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(thumbs_dir()).map_err(|e| format!("thumbs dir: {e}"))?;
    Ok(thumb_path(sha))
}

/// Потоковый sha256 — файл может быть видео на несколько гигабайт, целиком
/// в память его тянуть незачем.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Удаляет blob'ы, на которые не ссылается ни один сохранённый чат.
///
/// Вызывается после удаления чата: сами файлы чатов маленькие, полный обход
/// стоит миллисекунды, зато CAS не растёт бесконечно. Ошибки не фатальны —
/// логируем и идём дальше.
pub fn gc_unreferenced(referenced: &HashSet<String>) {
    let mut removed = 0usize;
    let mut freed = 0u64;
    for (dir, strip_ext) in [
        (blobs_dir(), true),
        (derived_dir(), true),
        (thumbs_dir(), true),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let sha = if strip_ext {
                name.split('.').next().unwrap_or(name)
            } else {
                name
            };
            // Защита от случайных файлов: трогаем только имена-хеши.
            if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            if referenced.contains(sha) {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    removed += 1;
                    freed += size;
                }
                Err(err) => log::warn!("[attach] не удалось удалить {}: {err}", path.display()),
            }
        }
    }
    if removed > 0 {
        log::info!("[attach] GC: удалено {removed} файлов, освобождено {freed} байт");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn paths_carry_extension() {
        assert!(blob_path("abc", "png").ends_with("abc.png"));
        assert!(blob_path("abc", "").ends_with("abc"));
        assert!(thumb_path("abc").ends_with("abc.png"));
    }
}
