//! Запись пользовательских файлов без риска оставить их обрезанными.
//!
//! `std::fs::write` пишет поверх: если процесс убьют посреди записи (OOM,
//! Ctrl+C, падение), на диске остаётся половина JSON, и при следующем
//! запуске файл не разбирается. Здесь запись идёт во временный файл рядом
//! и подменяет цель `rename`'ом — читатель видит либо старую, либо новую
//! версию целиком.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Файлы, отложенные [`quarantine`] за этот запуск: (исходный путь, копия).
/// UI забирает их через [`take_quarantined`] и показывает уведомление.
static QUARANTINED: Mutex<Vec<(PathBuf, PathBuf)>> = Mutex::new(Vec::new());

type Notifier = Box<dyn Fn(PathBuf, PathBuf) + Send + Sync>;

/// Куда сообщать об отложенных файлах после старта UI (см. [`set_notifier`]).
/// До установки события копятся в [`QUARANTINED`].
static NOTIFIER: OnceLock<Notifier> = OnceLock::new();

fn register(file: PathBuf, backup: PathBuf) {
    match NOTIFIER.get() {
        Some(notify) => notify(file, backup),
        None => QUARANTINED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((file, backup)),
    }
}

/// Подключить уведомления UI: всё, что отложено после этого вызова, сразу
/// уходит в `notify` (например, битый файл проекта, открытого позже старта).
/// Отложенное до вызова забирается через [`take_quarantined`].
pub fn set_notifier(notify: impl Fn(PathBuf, PathBuf) + Send + Sync + 'static) {
    let _ = NOTIFIER.set(Box::new(notify));
}

fn bad_suffix() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(".bad-{ts}")
}

/// Пишет `data` в `path` атомарно: временный файл в том же каталоге (тот же
/// раздел — иначе `rename` не атомарен) и `rename` поверх цели. Имя
/// временного файла уникально по процессу и вызову, так что параллельные
/// записи одного пути из разных потоков не делят tmp.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    replace_via_tmp(path, |tmp| std::fs::write(tmp, data))
}

/// Сохранение файла пользователя (редактор кода): атомарно, с правами
/// исходного файла, если цель — обычный файл без жёстких ссылок. Символьную
/// ссылку `rename` заменил бы обычным файлом, а у жёсткой ссылки оторвал бы
/// второе имя — туда, как и в каталог без права записи (tmp не создать),
/// пишем поверх, как vim/VS Code.
pub fn write_user_file(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    let data = data.as_ref();
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return write_atomic(path, data),
        Err(_) => return std::fs::write(path, data),
    };
    #[cfg(unix)]
    let single_link = std::os::unix::fs::MetadataExt::nlink(&meta) == 1;
    #[cfg(not(unix))]
    let single_link = true;
    if !meta.file_type().is_file() || !single_link {
        return std::fs::write(path, data);
    }
    let perms = meta.permissions();
    match replace_via_tmp(path, |tmp| {
        std::fs::write(tmp, data)?;
        std::fs::set_permissions(tmp, perms)
    }) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => std::fs::write(path, data),
        Err(e) => Err(e),
    }
}

/// Копирует `src` в `dst` атомарно (копия во временный файл рядом с `dst`
/// и `rename`). Для файлов, чьё имя — хеш содержимого: обрезанная копия под
/// таким именем иначе считалась бы готовой навсегда.
pub fn copy_atomic(src: &Path, dst: &Path) -> std::io::Result<()> {
    replace_via_tmp(dst, |tmp| std::fs::copy(src, tmp).map(|_| ()))
}

fn replace_via_tmp(
    path: &Path,
    fill: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{seq}.tmp", std::process::id()));
    let tmp = path.with_file_name(name);
    if let Err(e) = fill(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Откладывает в сторону файл, который не удалось разобрать:
/// `config.json` → `config.json.bad-<unix-время>`. Вызывающий после этого
/// может начать с чистого листа, не затирая данные пользователя, — их
/// можно поправить руками и вернуть.
pub fn quarantine(path: &Path) -> std::io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(bad_suffix());
    let backup = path.with_file_name(name);
    std::fs::rename(path, &backup)?;
    register(path.to_path_buf(), backup.clone());
    Ok(backup)
}

/// То же для данных, у которых нет своего файла на диске (запись внутри
/// `.syn`-бандла): сырое содержимое сохраняется рядом с `near` как
/// `<near>.<tag>.bad-<время>`. `label` — что показать пользователю как
/// источник (например, `проект.syn/notes/tree.json`).
pub fn save_aside(near: &Path, tag: &str, label: PathBuf, data: &[u8]) -> std::io::Result<PathBuf> {
    let mut name = near.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{tag}{}", bad_suffix()));
    let backup = near.with_file_name(name);
    write_atomic(&backup, data)?;
    register(label, backup.clone());
    Ok(backup)
}

/// Забрать список отложенных за запуск файлов (исходный путь, копия) —
/// один раз, для уведомления пользователю.
pub fn take_quarantined() -> Vec<(PathBuf, PathBuf)> {
    std::mem::take(&mut *QUARANTINED.lock().unwrap_or_else(|p| p.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synthos-fsutil-{tag}-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_tmp() {
        let dir = tmp_dir("write");
        let path = dir.join("config.json");
        std::fs::write(&path, "old").unwrap();
        write_atomic(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "временный файл остался: {names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_atomic_copies_and_leaves_no_tmp() {
        let dir = tmp_dir("copy");
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, b"payload").unwrap();
        copy_atomic(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"payload");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert!(copy_atomic(&dir.join("missing"), &dir.join("x")).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "tmp после ошибки удалён");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_user_file_keeps_mode_and_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("user");
        let file = dir.join("run.sh");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_user_file(&file, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        let link = dir.join("link.sh");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        write_user_file(&link, "via link").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "via link");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantine_moves_file_aside() {
        let dir = tmp_dir("quarantine");
        let path = dir.join("config.json");
        std::fs::write(&path, "{broken").unwrap();
        let backup = quarantine(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{broken");
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("config.json.bad-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
