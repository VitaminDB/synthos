//! Persistence-слой загрузок: запись текущего состояния
//! `HuggingFaceCtx.downloads` + `download_queue` в
//! `~/.config/synthos/hf_downloads.json` и автоматическое восстановление
//! очереди при следующем запуске приложения.
//!
//! Паттерн `save/load` — копия `AppConfig::save/load` (см. `config.rs`):
//! ошибки логируются, дефолт — пустая структура. Эффект автосейва пишет
//! только при реальном изменении (fingerprint-skip), чтобы не давить
//! диск на каждом keystroke прогресса.

use std::collections::HashMap;
use std::path::PathBuf;

use syngui::prelude::*;
use serde::{Deserialize, Serialize};

use super::state::{DlStatus, DownloadState, HuggingFaceCtx, SegmentProgress, VerifyStatus};

/// Сериализуемый снимок per-file загрузки. Поля субсет `DownloadState`
/// (без in-memory-only: `speed_*`, `last_meta_flush_at`, `retry_count`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedDownload {
    pub repo_id: String,
    pub filename: String,
    pub dest_path: String,
    pub bytes_done: u64,
    pub total: u64,
    pub status: PersistedStatus,
    #[serde(default)]
    pub segments: Vec<PersistedSegment>,
    /// Ожидаемый SHA-256 файла (из HF LFS); сохраняется чтобы после
    /// рестарта ручная проверка не требовала перезагрузки model_details.
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Сохраняем только финальные исходы верификации (Match/Mismatch),
    /// чтобы после перезапуска UI показывал бейдж без перерасчёта.
    /// Computing/Unknown/Error не персистим.
    #[serde(default)]
    pub verified_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSegment {
    pub from: u64,
    pub to: u64,
    pub bytes_done: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "msg")]
pub enum PersistedStatus {
    Pending,
    /// Active на момент крэша — после рестарта станет Pending и попадёт в очередь.
    Active,
    /// Приостановлено пользователем — после рестарта остаётся Paused (не
    /// автостартует), `.part` ждёт ручного резюма.
    Paused,
    /// Остановлено пользователем — после рестарта остаётся Stopped.
    Stopped,
    Done,
    Error(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedDownloads {
    #[serde(default)]
    pub items: Vec<PersistedDownload>,
    /// FIFO-очередь — порядок, в котором юзер кликнул «Скачать всё».
    /// При рестарте восстанавливается в `ctx.download_queue`.
    #[serde(default)]
    pub queue: Vec<String>,
}

/// `$HOME/.config/synthos/hf_downloads.json`.
pub fn path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/hf_downloads.json")
}

pub fn load() -> PersistedDownloads {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(s) => match serde_json::from_str::<PersistedDownloads>(&s) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[synthos hf] Не удалось распарсить {p:?}: {e}");
                PersistedDownloads::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PersistedDownloads::default(),
        Err(e) => {
            eprintln!("[synthos hf] Не удалось прочитать {p:?}: {e}");
            PersistedDownloads::default()
        }
    }
}

pub fn save(p: &PersistedDownloads) {
    let path = path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[synthos hf] Не удалось создать {parent:?}: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(p) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                eprintln!("[synthos hf] Не удалось записать {path:?}: {e}");
            }
        }
        Err(e) => eprintln!("[synthos hf] Ошибка сериализации hf_downloads: {e}"),
    }
}

/// Снимок `ctx.downloads` + `download_queue` в сериализуемой форме.
pub fn snapshot_from_ctx(ctx: &HuggingFaceCtx) -> PersistedDownloads {
    let downloads = ctx.downloads.get_untracked();
    let queue = ctx.download_queue.get_untracked();
    let mut items = Vec::with_capacity(downloads.len());
    for d in downloads.values() {
        items.push(PersistedDownload {
            repo_id: d.repo_id.clone(),
            filename: d.filename.clone(),
            dest_path: d.dest_path.display().to_string(),
            bytes_done: d.bytes_done,
            total: d.total,
            status: match &d.status {
                DlStatus::Pending => PersistedStatus::Pending,
                DlStatus::Active => PersistedStatus::Active,
                DlStatus::Paused => PersistedStatus::Paused,
                DlStatus::Stopped => PersistedStatus::Stopped,
                DlStatus::Done => PersistedStatus::Done,
                DlStatus::Error(m) => PersistedStatus::Error(m.clone()),
            },
            segments: d
                .segments
                .iter()
                .map(|s| PersistedSegment { from: s.from, to: s.to, bytes_done: s.bytes_done })
                .collect(),
            expected_sha256: d.expected_sha256.clone(),
            verified_sha256: match &d.verify {
                VerifyStatus::Match { sha256, .. } => Some(sha256.clone()),
                _ => None,
            },
        });
    }
    // Стабильный порядок для fingerprint'а.
    items.sort_by(|a, b| {
        a.repo_id
            .cmp(&b.repo_id)
            .then_with(|| a.filename.cmp(&b.filename))
    });
    PersistedDownloads { items, queue }
}

/// Из загруженного блоба собрать in-memory map'у `DownloadState`.
/// `Active` на момент крэша превращается в `Pending` — будет обработан
/// `try_drain_queue` после `resume_persisted` в download.rs.
pub fn into_map(p: PersistedDownloads) -> (HashMap<String, DownloadState>, Vec<String>) {
    let mut map = HashMap::with_capacity(p.items.len());
    for it in p.items {
        let key = format!("{}/{}", it.repo_id, it.filename);
        let status = match it.status {
            PersistedStatus::Pending => DlStatus::Pending,
            PersistedStatus::Active => DlStatus::Pending,
            // Paused/Stopped после рестарта НЕ автостартуют — остаются как есть,
            // пользователь резюмит вручную (≠ Active, который был прерван крэшем).
            PersistedStatus::Paused => DlStatus::Paused,
            PersistedStatus::Stopped => DlStatus::Stopped,
            PersistedStatus::Done => DlStatus::Done,
            PersistedStatus::Error(m) => DlStatus::Error(m),
        };
        map.insert(
            key,
            DownloadState {
                repo_id: it.repo_id,
                filename: it.filename,
                bytes_done: it.bytes_done,
                total: it.total,
                status,
                dest_path: PathBuf::from(it.dest_path),
                segments: it
                    .segments
                    .into_iter()
                    .map(|s| SegmentProgress {
                        from: s.from,
                        to: s.to,
                        bytes_done: s.bytes_done,
                    })
                    .collect(),
                speed_bps: 0.0,
                speed_sample_at: None,
                speed_sample_bytes: 0,
                retry_count: 0,
                last_meta_flush_at: None,
                expected_sha256: it.expected_sha256.clone(),
                verify: match it.verified_sha256 {
                    Some(s) => VerifyStatus::Match {
                        sha256: s,
                        has_expected: it.expected_sha256.is_some(),
                    },
                    None => VerifyStatus::Unknown,
                },
            },
        );
    }
    (map, p.queue)
}

/// Подписаться на `downloads` + `download_queue`. На каждое изменение —
/// сериализовать состояние и сравнить с последним сохранённым (fingerprint
/// по длине + первым байтам). Запись на диск только при реальной разнице.
pub fn install_autosave(ctx: HuggingFaceCtx) {
    let last_fp = use_signal(0u64);
    create_effect(move || {
        // Подписки — `.get()` обоих сигналов.
        let _ = ctx.downloads.get();
        let _ = ctx.download_queue.get();

        let snapshot = snapshot_from_ctx(&ctx);
        let fp = fingerprint(&snapshot);
        if last_fp.get_untracked() == fp {
            return;
        }
        save(&snapshot);
        last_fp.set(fp);
    });
}

/// Дёшево считаем fingerprint по статусам + bytes_done + queue, чтобы
/// прогрессовые тики, дающие одинаковый сериализованный blob, не выливались
/// в I/O. Идея — XOR + FNV-like rolling hash без сериализации.
fn fingerprint(p: &PersistedDownloads) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    fn mix(h: &mut u64, b: &[u8]) {
        for &x in b {
            *h ^= x as u64;
            *h = h.wrapping_mul(0x100000001b3);
        }
    }
    for it in &p.items {
        mix(&mut h, it.repo_id.as_bytes());
        mix(&mut h, it.filename.as_bytes());
        mix(&mut h, &it.bytes_done.to_le_bytes());
        mix(&mut h, &it.total.to_le_bytes());
        let s_tag: u8 = match &it.status {
            PersistedStatus::Pending => 1,
            PersistedStatus::Active => 2,
            PersistedStatus::Done => 3,
            PersistedStatus::Error(_) => 4,
            PersistedStatus::Paused => 5,
            PersistedStatus::Stopped => 6,
        };
        mix(&mut h, &[s_tag]);
        for s in &it.segments {
            mix(&mut h, &s.bytes_done.to_le_bytes());
        }
        if let Some(sha) = &it.verified_sha256 {
            mix(&mut h, sha.as_bytes());
        }
    }
    for k in &p.queue {
        mix(&mut h, k.as_bytes());
    }
    h
}
