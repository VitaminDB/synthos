//! Оркестрация скачивания: очередь с лимитом одновременных загрузок,
//! сегментированная загрузка через HTTP-Range, нотификации по завершению.
//!
//! Поведение:
//! - `start_download` → проверка cache-dir → `enqueue_download`.
//! - `enqueue_download` ставит файл в очередь со статусом `Pending` и дёргает
//!   `try_drain_queue`.
//! - `try_drain_queue` стартует ровно столько новых задач, чтобы суммарное
//!   число активных не превысило `concurrent_limit`. Любая завершившаяся
//!   задача (success/error) снова зовёт `try_drain_queue`, чтобы из очереди
//!   взялось следующее.
//! - `download_all_for_repo` enqueue'ит все siblings выбранной модели,
//!   пропуская уже Done.

use std::collections::HashSet;
use std::time::Duration;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::tr;
use syngui::widgets::feedback::NotificationCtx;

use crate::config;

use super::api;
use super::api::{AbortKind, HfError};
use super::control;
use super::persist::{self, PersistedDownloads};
use super::state::{DlStatus, DownloadState, HfSibling, HuggingFaceCtx};

const MAX_RETRIES: u32 = 3;

/// HF API в некоторых ответах возвращает `oid` со схемой `sha256:HEX` —
/// убираем префикс, чтобы хранить чистый hex для сравнения.
pub(super) fn strip_sha256_prefix(s: &str) -> &str {
    s.strip_prefix("sha256:").unwrap_or(s)
}

/// Exponential backoff: 1s, 4s, 16s. Capped на 60s.
fn retry_backoff(n: u32) -> Duration {
    let secs = (1u64.checked_shl(2 * n).unwrap_or(64)).min(60);
    Duration::from_secs(secs)
}

/// «Транзиентная» ошибка — имеет смысл повторить. Постоянные (4xx кроме
/// 408/429) сразу превращаются в Error.
fn is_transient(e: &HfError) -> bool {
    match e {
        HfError::Transport(_) | HfError::Io(_) => true,
        HfError::Status(code, _) => *code >= 500 || *code == 408 || *code == 429,
        HfError::Decode(_) => false,
        // Прерывание пользователем — намеренное, не ретраим.
        HfError::Aborted(_) => false,
    }
}

/// Публичная точка входа. Если cache-dir не задан — открывает Portal-диалог
/// (см. `dialogs.rs`) и сохраняет file в `pending_download`, чтобы после
/// accept_default / pick_folder автоматически возобновить скачивание.
pub fn start_download(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    repo_id: String,
    filename: String,
    expected_sha256: Option<String>,
) {
    let dir_raw = ctx.cache_dir.get_untracked();
    if dir_raw.trim().is_empty() {
        ctx.pending_download.update(|v| v.push((repo_id, filename)));
        ctx.cache_dir_dialog_open.set(true);
        return;
    }
    enqueue_download(ctx, notif, repo_id, filename, expected_sha256);
}

/// Поставить файл в очередь. Идемпотентно: если файл уже Active/Pending —
/// no-op; если был Done/Error — снова Pending (повторная попытка).
/// `expected_sha256` — ожидаемый hash из HF LFS; используется авто-verify
/// после Done. `None` — для не-LFS файлов или когда мы вызываем enqueue
/// без знания о sibling'е (resume_pending / start_download).
pub fn enqueue_download(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    repo_id: String,
    filename: String,
    expected_sha256: Option<String>,
) {
    let key = format!("{repo_id}/{filename}");
    let dir_raw = ctx.cache_dir.get_untracked();
    let dir = config::resolve_hf_cache_dir(&dir_raw);
    let dest = dir.join(&repo_id).join(&filename);
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            notif.error(format!("{}: {e}", tr!("hf.error.create_dir", path = parent.display())));
            return;
        }
    }

    // Идемпотентность: если уже Active/Pending/Done — выходим. Done = файл
    // на диске, повторно качать не нужно; пользователь может удалить файл
    // руками или дождаться истечения версии, после этого scan_existing_files
    // не отметит его как Done.
    // Paused/Stopped тоже блокируют пересоздание: иначе свежий Pending обнулит
    // накопленный `bytes_done`. Резюм идёт через `resume_download` (не enqueue).
    let already = ctx
        .downloads
        .get_untracked()
        .get(&key)
        .map(|d| {
            matches!(
                d.status,
                DlStatus::Active
                    | DlStatus::Pending
                    | DlStatus::Done
                    | DlStatus::Paused
                    | DlStatus::Stopped
            )
        })
        .unwrap_or(false);
    if already {
        return;
    }

    // Если scan_existing_files ещё не отработал, но файл на самом деле уже
    // лежит на диске целиком — рассинхрон in-memory map'ы. Дополнительная
    // проверка по metadata страхует от повторной закачки в edge-case'ах
    // (например, ручной клик «Скачать» до отрисовки detail-панели).
    if let Ok(meta) = std::fs::metadata(&dest) {
        if meta.is_file() && meta.len() > 0 {
            let len = meta.len();
            let prior_expected = ctx
                .downloads
                .get_untracked()
                .get(&key)
                .and_then(|d| d.expected_sha256.clone());
            let prior_verify = ctx
                .downloads
                .get_untracked()
                .get(&key)
                .map(|d| d.verify.clone())
                .unwrap_or(super::state::VerifyStatus::Unknown);
            let expected_final = expected_sha256.clone().or(prior_expected);
            ctx.downloads.update(|m| {
                m.insert(
                    key.clone(),
                    DownloadState {
                        repo_id: repo_id.clone(),
                        filename: filename.clone(),
                        bytes_done: len,
                        total: len,
                        status: DlStatus::Done,
                        dest_path: dest.clone(),
                        segments: Vec::new(),
                        speed_bps: 0.0,
                        speed_sample_at: None,
                        speed_sample_bytes: 0,
                        retry_count: 0,
                        last_meta_flush_at: None,
                        expected_sha256: expected_final,
                        verify: prior_verify,
                    },
                );
            });
            return;
        }
    }

    // expected: либо явно передали (download_all_for_repo с siblings),
    // либо переиспользуем из существующей записи (после scan_existing_files
    // он уже мог быть положен).
    let prior_expected = ctx
        .downloads
        .get_untracked()
        .get(&key)
        .and_then(|d| d.expected_sha256.clone());
    let expected_final = expected_sha256.or(prior_expected);
    let initial = DownloadState {
        repo_id: repo_id.clone(),
        filename: filename.clone(),
        bytes_done: 0,
        total: 0,
        status: DlStatus::Pending,
        dest_path: dest.clone(),
        segments: Vec::new(),
        speed_bps: 0.0,
        speed_sample_at: None,
        speed_sample_bytes: 0,
        retry_count: 0,
        last_meta_flush_at: None,
        expected_sha256: expected_final,
        verify: super::state::VerifyStatus::Unknown,
    };
    ctx.downloads.update(|m| {
        m.insert(key.clone(), initial);
    });
    ctx.download_queue.update(|q| {
        if !q.iter().any(|k| k == &key) {
            q.push(key.clone());
        }
    });
    try_drain_queue(ctx, notif);
}

/// Пока active < limit — снимаем head очереди и стартуем спавн.
pub fn try_drain_queue(ctx: HuggingFaceCtx, notif: NotificationCtx) {
    // Глобальная пауза гейтит очередь: новые задачи не стартуют, пока
    // пользователь не нажмёт «Продолжить все».
    if ctx.global_paused.get_untracked() {
        return;
    }
    let limit = ctx.concurrent_limit.get_untracked().max(1);
    loop {
        let active = ctx.active_downloads.get_untracked();
        if active >= limit {
            break;
        }
        let mut q = ctx.download_queue.get_untracked();
        if q.is_empty() {
            break;
        }
        let key = q.remove(0);
        ctx.download_queue.set(q);

        // Из ключа `{repo_id}/{filename}` достаём rid и fname. repo_id у HF
        // — `org/name`, fname может содержать `/`, поэтому split_once
        // невозможен по первому `/`. Берём из DownloadState.
        let snapshot = ctx
            .downloads
            .get_untracked()
            .get(&key)
            .cloned();
        let Some(state) = snapshot else { continue };
        let rid = state.repo_id.clone();
        let fname = state.filename.clone();
        let dest = state.dest_path.clone();

        // Переводим в Active и инкрементим счётчик ровно один раз.
        // `bytes_done`/`segments` НЕ обнуляем — sidecar/`.part` пресервнутые от
        // прошлого запуска подхватятся в api.rs и догрузка продолжится.
        ctx.downloads.update(|m| {
            if let Some(d) = m.get_mut(&key) {
                d.status = DlStatus::Active;
                d.speed_bps = 0.0;
                d.speed_sample_at = None;
                d.speed_sample_bytes = 0;
                d.retry_count = 0;
                d.last_meta_flush_at = None;
            }
        });
        ctx.active_downloads.update(|n| *n += 1);

        spawn_one(ctx, notif.clone(), key, rid, fname, dest);
    }
}

fn spawn_one(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    key: String,
    rid: String,
    fname: String,
    dest: std::path::PathBuf,
) {
    let downloads_sig = ctx.downloads;
    let n_segments = ctx.segments_per_file.get_untracked();
    let active_sig = ctx.active_downloads;
    // Регистрируем управление до спавна: сбрасывает stale-сигнал, оставшийся
    // от прошлого прогона при resume (Paused/Stopped → Pending → Active).
    let ctl = control::arm(&key);
    spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            let res = api::download_file(
                &rid,
                &fname,
                &dest,
                key.clone(),
                downloads_sig,
                n_segments,
                ctl.clone(),
            )
            .await;
            match res {
                Ok(()) => {
                    let key_f = key.clone();
                    let dest_show = dest.display().to_string();
                    let fname_show = fname.clone();
                    let ctx_done = ctx;
                    let notif_done = notif.clone();
                    let notif_async = notif.clone();
                    let notif_verify = notif.clone();
                    let key_verify = key.clone();
                    run_on_main_thread(move || {
                        downloads_sig.update(|m| {
                            if let Some(d) = m.get_mut(&key_f) {
                                d.status = DlStatus::Done;
                                d.retry_count = 0;
                                if d.total == 0 {
                                    d.total = d.bytes_done;
                                }
                            }
                        });
                        active_sig.update(|n| *n = n.saturating_sub(1));
                        control::clear(&key_f);
                        notif_async.success(tr!("hf.notify.downloaded", name = fname_show, path = dest_show));
                        // Запускаем авто-верификацию SHA-256 (если HF дал
                        // expected hash; иначе verify_file просто посчитает
                        // и положит Match-без-expected). Drain очереди — до
                        // verify: следующий файл качаем параллельно с проверкой
                        // предыдущего, проверка идёт в spawn_blocking-пуле.
                        try_drain_queue(ctx_done, notif_done);
                        super::verify::verify_file(ctx_done, notif_verify, key_verify);
                    });
                    return;
                }
                // Прерывание пользователем: ставим терминальный статус (без
                // retry), отпускаем слот, чистим управление и сливаем очередь.
                // Cancel дополнительно удаляет частичные файлы и саму запись.
                Err(HfError::Aborted(kind)) => {
                    let key_f = key.clone();
                    let dest_c = dest.clone();
                    let ctx_done = ctx;
                    let notif_done = notif.clone();
                    run_on_main_thread(move || {
                        match kind {
                            AbortKind::Pause => {
                                downloads_sig.update(|m| {
                                    if let Some(d) = m.get_mut(&key_f) {
                                        d.status = DlStatus::Paused;
                                        d.speed_bps = 0.0;
                                        d.speed_sample_at = None;
                                        d.speed_sample_bytes = 0;
                                    }
                                });
                            }
                            AbortKind::Stop => {
                                downloads_sig.update(|m| {
                                    if let Some(d) = m.get_mut(&key_f) {
                                        d.status = DlStatus::Stopped;
                                        d.speed_bps = 0.0;
                                        d.speed_sample_at = None;
                                        d.speed_sample_bytes = 0;
                                    }
                                });
                            }
                            AbortKind::Cancel => {
                                api::remove_partial(&dest_c);
                                downloads_sig.update(|m| {
                                    m.remove(&key_f);
                                });
                                ctx_done.download_queue.update(|q| q.retain(|k| k != &key_f));
                            }
                            // Глобальная пауза. Обычно → Paused (ждёт «Продолжить
                            // все»). Но если «Продолжить все» успел сняться раньше,
                            // чем эта задача дошла до своего check_abort (гонка),
                            // global_paused уже false — тогда не залипаем, а
                            // возвращаем файл в Pending+очередь; трейлинг
                            // try_drain_queue ниже его перезапустит.
                            AbortKind::GlobalPause => {
                                let resumed = !ctx_done.global_paused.get_untracked();
                                downloads_sig.update(|m| {
                                    if let Some(d) = m.get_mut(&key_f) {
                                        d.status = if resumed {
                                            DlStatus::Pending
                                        } else {
                                            DlStatus::Paused
                                        };
                                        d.speed_bps = 0.0;
                                        d.speed_sample_at = None;
                                        d.speed_sample_bytes = 0;
                                    }
                                });
                                if resumed {
                                    ctx_done.download_queue.update(|q| {
                                        if !q.iter().any(|k| k == &key_f) {
                                            q.push(key_f.clone());
                                        }
                                    });
                                }
                            }
                        }
                        active_sig.update(|n| *n = n.saturating_sub(1));
                        control::clear(&key_f);
                        try_drain_queue(ctx_done, notif_done);
                    });
                    return;
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && is_transient(&e) {
                        attempt += 1;
                        let backoff = retry_backoff(attempt);
                        // Отрисовать «попытка N/3» в UI до сна.
                        let key_r = key.clone();
                        let n_attempt = attempt;
                        run_on_main_thread(move || {
                            downloads_sig.update(|m| {
                                if let Some(d) = m.get_mut(&key_r) {
                                    d.retry_count = n_attempt;
                                    // Сбрасываем speed-sample, чтобы EMA не давала
                                    // стартовый «прыжок» после паузы.
                                    d.speed_bps = 0.0;
                                    d.speed_sample_at = None;
                                    d.speed_sample_bytes = 0;
                                }
                            });
                        });
                        tokio::time::sleep(backoff).await;
                        continue; // вторая/третья/N-я попытка через download_file → resume
                    }
                    let msg = e.to_string();
                    let msg_q = msg.clone();
                    let key_f = key.clone();
                    let fname_show = fname.clone();
                    let ctx_done = ctx;
                    let notif_done = notif.clone();
                    let notif_async = notif.clone();
                    run_on_main_thread(move || {
                        downloads_sig.update(|m| {
                            if let Some(d) = m.get_mut(&key_f) {
                                d.status = DlStatus::Error(msg.clone());
                            }
                        });
                        active_sig.update(|n| *n = n.saturating_sub(1));
                        control::clear(&key_f);
                        notif_async.error(tr!("hf.error.download_failed", name = fname_show, error = msg_q));
                        try_drain_queue(ctx_done, notif_done);
                    });
                    return;
                }
            }
        }
    });
}

/// ⏸ Пауза. Active → выставляем флаг (задача сама перейдёт в Paused, отпустит
/// слот и сольёт очередь). Pending → снимаем из очереди и ставим Paused прямо
/// (активной задачи нет). Прочие статусы игнорируем.
pub fn pause_download(ctx: HuggingFaceCtx, key: String) {
    let status = ctx.downloads.get_untracked().get(&key).map(|d| d.status.clone());
    match status {
        Some(DlStatus::Active) => control::request(&key, control::PAUSE),
        Some(DlStatus::Pending) => {
            ctx.download_queue.update(|q| q.retain(|k| k != &key));
            ctx.downloads.update(|m| {
                if let Some(d) = m.get_mut(&key) {
                    d.status = DlStatus::Paused;
                }
            });
        }
        _ => {}
    }
}

/// ■ Стоп. Семантически как `pause_download`, но переводит в Stopped (другой
/// UX-акцент: «остановлено», резюм кнопкой ⬇ Скачать). `.part` сохраняется.
pub fn stop_download(ctx: HuggingFaceCtx, key: String) {
    let status = ctx.downloads.get_untracked().get(&key).map(|d| d.status.clone());
    match status {
        Some(DlStatus::Active) => control::request(&key, control::STOP),
        Some(DlStatus::Pending) => {
            ctx.download_queue.update(|q| q.retain(|k| k != &key));
            ctx.downloads.update(|m| {
                if let Some(d) = m.get_mut(&key) {
                    d.status = DlStatus::Stopped;
                }
            });
        }
        _ => {}
    }
}

/// ✕ Отмена. Active → флаг CANCEL (задача сама удалит `.part` и запись).
/// Прочие (Pending/Paused/Stopped/Error) → снимаем из очереди, удаляем
/// частичные файлы и саму запись из map'ы здесь же.
pub fn cancel_download(ctx: HuggingFaceCtx, key: String) {
    let snapshot = ctx.downloads.get_untracked().get(&key).cloned();
    let Some(d) = snapshot else { return };
    if matches!(d.status, DlStatus::Active) {
        control::request(&key, control::CANCEL);
        return;
    }
    ctx.download_queue.update(|q| q.retain(|k| k != &key));
    api::remove_partial(&d.dest_path);
    ctx.downloads.update(|m| {
        m.remove(&key);
    });
    control::clear(&key);
}

/// ▶ Продолжить / ⬇ Скачать (резюм). Переводит Paused/Stopped обратно в
/// Pending, сохраняя `bytes_done/total/segments/expected`, и ставит в очередь.
/// `try_drain_queue` подхватит файл, `api.rs` резюмит с `.part`/`.part.meta`.
pub fn resume_download(ctx: HuggingFaceCtx, notif: NotificationCtx, key: String) {
    if !ctx.downloads.get_untracked().contains_key(&key) {
        return;
    }
    ctx.downloads.update(|m| {
        if let Some(d) = m.get_mut(&key) {
            d.status = DlStatus::Pending;
            d.retry_count = 0;
        }
    });
    ctx.download_queue.update(|q| {
        if !q.iter().any(|k| k == &key) {
            q.push(key.clone());
        }
    });
    try_drain_queue(ctx, notif);
}

/// ⏸ Пауза ВСЕХ загрузок (кнопка тулбара). Поднимает `global_paused` (гейтит
/// `try_drain_queue`), сигналит `GLOBAL_PAUSE` активным (они сами перейдут в
/// Paused через abort-ветку `spawn_one`) и снимает Pending из очереди в Paused.
/// Флаг ставим ПЕРВЫМ: трейлинг `try_drain_queue` в abort-коллбэках упрётся в
/// гейт и ничего не перезапустит.
pub fn pause_all(ctx: HuggingFaceCtx) {
    ctx.global_paused.set(true);
    let snapshot = ctx.downloads.get_untracked();
    let mut to_pause: Vec<String> = Vec::new();
    for (key, d) in snapshot.iter() {
        match d.status {
            DlStatus::Active => control::request(key, control::GLOBAL_PAUSE),
            DlStatus::Pending => to_pause.push(key.clone()),
            _ => {}
        }
    }
    if !to_pause.is_empty() {
        ctx.download_queue
            .update(|q| q.retain(|k| !to_pause.iter().any(|p| p == k)));
        ctx.downloads.update(|m| {
            for key in &to_pause {
                if let Some(d) = m.get_mut(key) {
                    d.status = DlStatus::Paused;
                }
            }
        });
    }
}

/// ▶ Продолжить ВСЕ. Снимает `global_paused`, переводит все Paused обратно в
/// Pending+очередь и сливает очередь. Stopped/Error не трогает — это явный
/// выбор пользователя (резюм отдельной кнопкой строки).
pub fn resume_all(ctx: HuggingFaceCtx, notif: NotificationCtx) {
    ctx.global_paused.set(false);
    let snapshot = ctx.downloads.get_untracked();
    let to_resume: Vec<String> = snapshot
        .iter()
        .filter(|(_, d)| matches!(d.status, DlStatus::Paused))
        .map(|(k, _)| k.clone())
        .collect();
    if !to_resume.is_empty() {
        ctx.downloads.update(|m| {
            for key in &to_resume {
                if let Some(d) = m.get_mut(key) {
                    d.status = DlStatus::Pending;
                    d.retry_count = 0;
                }
            }
        });
        ctx.download_queue.update(|q| {
            for key in &to_resume {
                if !q.iter().any(|k| k == key) {
                    q.push(key.clone());
                }
            }
        });
    }
    try_drain_queue(ctx, notif);
}

/// Запустить отложенные загрузки (после выбора cache-dir в диалоге).
/// Очищает `pending_download`-список и enqueue'ит каждый файл.
pub fn resume_pending(ctx: HuggingFaceCtx, notif: NotificationCtx) {
    let pending = ctx.pending_download.get_untracked();
    ctx.pending_download.set(Vec::new());
    for (rid, fname) in pending {
        enqueue_download(ctx, notif.clone(), rid, fname, None);
    }
}

/// Просканировать кэш на диске и проставить `DlStatus::Done` для тех файлов
/// репозитория, которые уже скачаны полностью. Вызывается после загрузки
/// `model_details`. Не трогает уже Active/Pending — переход через `match`
/// гарантирует, что мы не перезатрём активную работу.
pub fn scan_existing_files(
    ctx: HuggingFaceCtx,
    repo_id: &str,
    siblings: &[HfSibling],
) {
    let dir_raw = ctx.cache_dir.get_untracked();
    let dir = config::resolve_hf_cache_dir(&dir_raw);
    let repo_dir = dir.join(repo_id);
    if !repo_dir.exists() {
        return;
    }
    let mut updates: Vec<(String, DownloadState)> = Vec::new();
    for s in siblings {
        let dest = repo_dir.join(&s.rfilename);
        let key = format!("{}/{}", repo_id, s.rfilename);
        // Не трогаем активные/pending — текущая задача всё дописывает. Paused/
        // Stopped тоже не перетираем: их `.part` ждёт ручного резюма, а полного
        // файла на диске нет (dest-metadata всё равно бы не сработала).
        if let Some(existing) = ctx.downloads.get_untracked().get(&key) {
            if matches!(
                existing.status,
                DlStatus::Active | DlStatus::Pending | DlStatus::Paused | DlStatus::Stopped
            ) {
                continue;
            }
        }
        if let Ok(meta) = std::fs::metadata(&dest) {
            if meta.is_file() && meta.len() > 0 {
                let len = meta.len();
                // Если в API известен size и он отличается — файл повреждён/частичный,
                // не отмечаем как Done; пользователь сможет переcкачать.
                let size_matches = match s.size {
                    Some(expected) => expected == len,
                    None => true, // size неизвестен — доверяем диску
                };
                if size_matches {
                    // Подтягиваем expected_sha256 из HF API (LFS). Если он есть
                    // и in-memory verify уже Match — сохраняем (resume_persisted
                    // мог положить). Иначе Unknown → авто-проверка не запускается
                    // здесь (только после реального скачивания), но кнопка
                    // «SHA256» в UI остаётся доступной.
                    let expected = s.lfs.as_ref().and_then(|l| {
                        let raw = l.sha256.trim();
                        if raw.is_empty() {
                            None
                        } else {
                            Some(strip_sha256_prefix(raw).to_string())
                        }
                    });
                    let prior_verify = ctx
                        .downloads
                        .get_untracked()
                        .get(&key)
                        .map(|d| d.verify.clone());
                    updates.push((
                        key,
                        DownloadState {
                            repo_id: repo_id.to_string(),
                            filename: s.rfilename.clone(),
                            bytes_done: len,
                            total: len,
                            status: DlStatus::Done,
                            dest_path: dest,
                            segments: Vec::new(),
                            speed_bps: 0.0,
                            speed_sample_at: None,
                            speed_sample_bytes: 0,
                            retry_count: 0,
                            last_meta_flush_at: None,
                            expected_sha256: expected,
                            verify: prior_verify
                                .unwrap_or(super::state::VerifyStatus::Unknown),
                        },
                    ));
                }
            }
        }
    }
    if !updates.is_empty() {
        ctx.downloads.update(|m| {
            for (k, v) in updates {
                m.insert(k, v);
            }
        });
    }
}

/// «Скачать всё» для выбранного репозитория: для каждого sibling, который
/// ещё не `Done`, ставим Pending в очередь.
pub fn download_all_for_repo(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    repo_id: String,
    siblings: Vec<HfSibling>,
) {
    // Тумблер «пропускать onnx/openvino/fp32/bin» применяется только к bulk —
    // кнопка «Скачать» на отдельном файле качает что угодно.
    let skip = ctx.skip_unwanted_formats.get_untracked();
    let gguf_ok = ctx.gguf_support.get_untracked();

    let dir_raw = ctx.cache_dir.get_untracked();
    if dir_raw.trim().is_empty() {
        // Сохраняем весь набор как pending и открываем диалог cache-dir.
        // Фильтр применяем уже здесь, чтобы отложенный bulk тоже его учитывал.
        ctx.pending_download.update(|v| {
            for s in &siblings {
                if skip && super::filter::is_excluded(&s.rfilename, gguf_ok) {
                    continue;
                }
                v.push((repo_id.clone(), s.rfilename.clone()));
            }
        });
        ctx.cache_dir_dialog_open.set(true);
        return;
    }

    let downloads = ctx.downloads.get_untracked();
    for s in siblings {
        if skip && super::filter::is_excluded(&s.rfilename, gguf_ok) {
            continue;
        }
        let key = format!("{}/{}", repo_id, s.rfilename);
        if let Some(d) = downloads.get(&key) {
            if matches!(
                d.status,
                DlStatus::Done
                    | DlStatus::Active
                    | DlStatus::Pending
                    | DlStatus::Paused
                    | DlStatus::Stopped
            ) {
                continue;
            }
        }
        let expected = s.lfs.as_ref().and_then(|l| {
            let raw = l.sha256.trim();
            if raw.is_empty() {
                None
            } else {
                Some(strip_sha256_prefix(raw).to_string())
            }
        });
        enqueue_download(ctx, notif.clone(), repo_id.clone(), s.rfilename, expected);
    }
}

/// «Скачать выбранные»: enqueue'ит только siblings, чьи ключи
/// `{repo_id}/{filename}` есть в `selected`. Формат-фильтр НЕ применяется —
/// выбор ручной. Поведение при отсутствии cache-dir — как у
/// `download_all_for_repo` (откладываем в `pending_download` + диалог).
/// Снимает галочки после постановки в очередь (выбор = временная «корзина»).
pub fn download_selected(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    repo_id: String,
    siblings: Vec<HfSibling>,
    selected: HashSet<String>,
) {
    let dir_raw = ctx.cache_dir.get_untracked();
    if dir_raw.trim().is_empty() {
        ctx.pending_download.update(|v| {
            for s in &siblings {
                let key = format!("{}/{}", repo_id, s.rfilename);
                if selected.contains(&key) {
                    v.push((repo_id.clone(), s.rfilename.clone()));
                }
            }
        });
        ctx.cache_dir_dialog_open.set(true);
        return;
    }

    let downloads = ctx.downloads.get_untracked();
    for s in siblings {
        let key = format!("{}/{}", repo_id, s.rfilename);
        if !selected.contains(&key) {
            continue;
        }
        if let Some(d) = downloads.get(&key) {
            if matches!(
                d.status,
                DlStatus::Done
                    | DlStatus::Active
                    | DlStatus::Pending
                    | DlStatus::Paused
                    | DlStatus::Stopped
            ) {
                continue;
            }
        }
        let expected = s.lfs.as_ref().and_then(|l| {
            let raw = l.sha256.trim();
            if raw.is_empty() {
                None
            } else {
                Some(strip_sha256_prefix(raw).to_string())
            }
        });
        enqueue_download(ctx, notif.clone(), repo_id.clone(), s.rfilename, expected);
    }
    ctx.selected_files.update(|s| s.clear());
}

/// Восстановление загрузок с прошлого запуска. Вызывается единожды из
/// `lib.rs::run_desktop` после `HuggingFaceCtx::new + provide_context`.
/// - `Done`-записи кладутся в `downloads` (UI показывает чек сразу).
/// - `Pending` и бывшие `Active` → Pending + в конец `download_queue`,
///   `try_drain_queue` запускает первые `concurrent_limit` файлов.
///   `api.rs` сам подберёт `.part`/`.part.meta` и продолжит с того места.
/// - `Error` сохраняем как Error: пользователь решает retry'ить или нет.
pub fn resume_persisted(
    ctx: HuggingFaceCtx,
    notif: NotificationCtx,
    persisted: PersistedDownloads,
) {
    if persisted.items.is_empty() && persisted.queue.is_empty() {
        return;
    }
    let (map, mut queue) = persist::into_map(persisted);
    let mut to_enqueue = Vec::new();
    for (key, d) in map.iter() {
        if matches!(d.status, DlStatus::Pending) && !queue.iter().any(|k| k == key) {
            to_enqueue.push(key.clone());
        }
    }
    // Сначала ставим то, что было в FIFO queue до крэша, потом — остальные
    // Pending (теоретически могут возникнуть при ручной правке файла).
    queue.extend(to_enqueue);

    ctx.downloads.update(|m| {
        for (k, v) in map {
            // Не перетираем уже существующие записи (resume идёт один раз
            // на старте, но защищаемся от двойного вызова).
            m.entry(k).or_insert(v);
        }
    });
    ctx.download_queue.set(queue);
    ctx.active_downloads.set(0);
    try_drain_queue(ctx, notif);
}
