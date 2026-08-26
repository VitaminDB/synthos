//! SHA-256-верификация скачанных файлов.
//!
//! HF Hub отдаёт `lfs.sha256` в `/api/models/{id}?blobs=true` для всех LFS-
//! файлов (тяжёлые веса). После успешного `download_file` мы автоматически
//! вычисляем hash целевого файла и сравниваем с `expected_sha256`. Если
//! `expected_sha256 = None` (мелкие git-blob'ы) — авто-проверка не
//! запускается, но ручная кнопка «SHA256» всё равно работает и просто
//! выводит вычисленный hex.
//!
//! Вычисление идёт в `tokio::task::spawn_blocking` — `sha2`-ядра pure-CPU,
//! tokio thread-pool обрабатывает их без блокировки event loop'а; UI
//! получает обновления через `run_on_main_thread`.

use std::path::{Path, PathBuf};

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::tr;
use syngui::widgets::feedback::NotificationCtx;

use super::state::{HuggingFaceCtx, VerifyStatus};

/// Прочитать файл блоками 1 МБ и вычислить SHA-256 hex digest.
/// Возвращает ошибку как строку — наверх передаётся в `VerifyStatus::Error`.
fn compute_sha256_sync(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Запустить SHA-256-проверку файла по ключу `{repo_id}/{filename}`.
/// Идемпотентно: если verify уже `Computing` — выходим, чтобы не запустить
/// две параллельных проверки на один файл. После завершения update'ит
/// `VerifyStatus` и (при mismatch) шлёт нотификацию.
pub fn verify_file(ctx: HuggingFaceCtx, notif: NotificationCtx, key: String) {
    let Some(state) = ctx.downloads.get_untracked().get(&key).cloned() else {
        return;
    };
    if matches!(state.verify, VerifyStatus::Computing) {
        return;
    }
    let path = state.dest_path.clone();
    let expected = state.expected_sha256.clone();
    let downloads = ctx.downloads;
    let key_set = key.clone();
    let fname_show = state.filename.clone();

    // Перевод в Computing — UI сразу показывает «считаю SHA256…».
    downloads.update(|m| {
        if let Some(d) = m.get_mut(&key_set) {
            d.verify = VerifyStatus::Computing;
        }
    });

    spawn(async move {
        let path_blocking: PathBuf = path.clone();
        let res = tokio::task::spawn_blocking(move || compute_sha256_sync(&path_blocking))
            .await
            .unwrap_or_else(|e| Err(format!("verify thread panic: {e}")));
        run_on_main_thread(move || {
            downloads.update(|m| {
                let Some(d) = m.get_mut(&key) else { return };
                match (res, &expected) {
                    (Err(msg), _) => {
                        d.verify = VerifyStatus::Error(msg.clone());
                        notif.error(tr!("hf.error.verify_failed", name = fname_show, error = msg));
                    }
                    (Ok(actual), Some(exp)) => {
                        if actual.eq_ignore_ascii_case(exp) {
                            d.verify = VerifyStatus::Match {
                                sha256: actual,
                                has_expected: true,
                            };
                        } else {
                            d.verify = VerifyStatus::Mismatch {
                                actual: actual.clone(),
                                expected: exp.clone(),
                            };
                            notif.error(tr!(
                                "hf.error.verify_mismatch",
                                name = fname_show, expected = exp, actual = actual
                            ));
                        }
                    }
                    (Ok(actual), None) => {
                        d.verify = VerifyStatus::Match {
                            sha256: actual,
                            has_expected: false,
                        };
                    }
                }
            });
        });
    });
}
