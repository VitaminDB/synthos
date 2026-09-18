//! Сводный прогресс загрузок — одна арифметика для нижней панели страницы
//! ([`super::dock`]) и чипа в шапке окна (`components::titlebar`).
//!
//! Считаем по репозиториям, в которых есть незавершённые файлы: уже скачанные
//! файлы такого репозитория входят в сумму целиком. Так процент не обнуляется
//! после перезапуска приложения (докачка продолжает с тех же 40 %, а не с 0) и
//! совпадает с тем, что человек ждёт от «Скачать всё».

use std::collections::{HashMap, HashSet};

use syngui::tr;

use super::state::{DlStatus, DownloadState};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    pub done_bytes: u64,
    /// Сумма известных размеров. Файлы без размера (сервер не отдал
    /// Content-Length, а в API его не было) не входят ни сюда, ни в `done_bytes`.
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub files_done: u32,
    pub files_total: u32,
    pub active: u32,
    pub pending: u32,
    /// Paused и Stopped вместе — оба «прервано, `.part` ждёт».
    pub paused: u32,
    pub errors: u32,
}

impl Totals {
    /// Есть что показывать: хоть один файл не докачан.
    pub fn has_unfinished(&self) -> bool {
        self.active + self.pending + self.paused + self.errors > 0
    }

    /// Загрузка идёт или встанет в работу сама (очередь).
    pub fn in_flight(&self) -> bool {
        self.active + self.pending > 0
    }

    /// Доля 0..=1; `None`, пока не известен ни один размер.
    pub fn ratio(&self) -> Option<f32> {
        if self.total_bytes == 0 {
            return None;
        }
        Some((self.done_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32)
    }

    pub fn percent_text(&self) -> String {
        match self.ratio() {
            // Вниз, а не к ближайшему: «100 %» при недокачанном хвосте врёт.
            Some(r) => format!("{}%", (r * 100.0).floor() as u32),
            None => "…".to_string(),
        }
    }

    /// Секунд до конца при текущей суммарной скорости.
    pub fn eta_secs(&self) -> Option<u64> {
        if self.speed_bps < 1.0 || self.total_bytes <= self.done_bytes {
            return None;
        }
        Some(((self.total_bytes - self.done_bytes) as f64 / self.speed_bps).ceil() as u64)
    }
}

fn unfinished(status: &DlStatus) -> bool {
    !matches!(status, DlStatus::Done)
}

pub fn totals(downloads: &HashMap<String, DownloadState>) -> Totals {
    let repos: HashSet<&str> = downloads
        .values()
        .filter(|d| unfinished(&d.status))
        .map(|d| d.repo_id.as_str())
        .collect();
    let mut t = Totals::default();
    for d in downloads.values().filter(|d| repos.contains(d.repo_id.as_str())) {
        t.files_total += 1;
        if d.total > 0 {
            t.total_bytes += d.total;
            t.done_bytes += d.bytes_done.min(d.total);
        }
        match d.status {
            DlStatus::Done => t.files_done += 1,
            DlStatus::Active => {
                t.active += 1;
                t.speed_bps += d.speed_bps;
            }
            DlStatus::Pending => t.pending += 1,
            DlStatus::Paused | DlStatus::Stopped => t.paused += 1,
            DlStatus::Error(_) => t.errors += 1,
        }
    }
    t
}

pub fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = n as f64;
    if f >= GB {
        format!("{:.2} {}", f / GB, tr!("hf.unit.gb"))
    } else if f >= MB {
        format!("{:.1} {}", f / MB, tr!("hf.unit.mb"))
    } else if f >= KB {
        format!("{:.1} {}", f / KB, tr!("hf.unit.kb"))
    } else {
        format!("{} {}", n, tr!("hf.unit.b"))
    }
}

pub fn human_speed(bps: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bps >= GB {
        format!("{:.2} {}", bps / GB, tr!("hf.unit.gb_per_s"))
    } else if bps >= MB {
        format!("{:.1} {}", bps / MB, tr!("hf.unit.mb_per_s"))
    } else if bps >= KB {
        format!("{:.0} {}", bps / KB, tr!("hf.unit.kb_per_s"))
    } else {
        format!("{:.0} {}", bps, tr!("hf.unit.b_per_s"))
    }
}

/// «3 ч 15 мин» / «4 мин» / «40 с» — две старшие единицы, без секунд у часов.
pub fn human_eta(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h} {} {m} {}", tr!("hf.unit.hour"), tr!("hf.unit.min"))
    } else if m > 0 {
        format!("{m} {}", tr!("hf.unit.min"))
    } else {
        format!("{s} {}", tr!("hf.unit.sec"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::huggingface::state::VerifyStatus;
    use std::path::PathBuf;

    fn entry(repo: &str, name: &str, done: u64, total: u64, status: DlStatus, speed: f64) -> (String, DownloadState) {
        (
            format!("{repo}/{name}"),
            DownloadState {
                repo_id: repo.to_string(),
                filename: name.to_string(),
                bytes_done: done,
                total,
                status,
                dest_path: PathBuf::new(),
                segments: Vec::new(),
                speed_bps: speed,
                speed_sample_at: None,
                speed_sample_bytes: 0,
                retry_count: 0,
                last_meta_flush_at: None,
                expected_sha256: None,
                verify: VerifyStatus::Unknown,
            },
        )
    }

    #[test]
    fn counts_only_repos_with_unfinished_files() {
        let map: HashMap<_, _> = [
            entry("a/x", "1", 100, 100, DlStatus::Done, 0.0),
            entry("a/x", "2", 50, 300, DlStatus::Active, 25.0),
            entry("a/x", "3", 0, 600, DlStatus::Pending, 0.0),
            // Целиком скачанный репозиторий в общий прогресс не входит.
            entry("b/y", "1", 900, 900, DlStatus::Done, 0.0),
        ]
        .into_iter()
        .collect();
        let t = totals(&map);
        assert_eq!((t.done_bytes, t.total_bytes), (150, 1000));
        assert_eq!((t.files_done, t.files_total), (1, 3));
        assert_eq!((t.active, t.pending), (1, 1));
        assert_eq!(t.percent_text(), "15%");
        assert_eq!(t.eta_secs(), Some(34));
        assert!(t.in_flight() && t.has_unfinished());
    }

    #[test]
    fn nothing_to_show_when_everything_is_done() {
        let map: HashMap<_, _> =
            [entry("b/y", "1", 900, 900, DlStatus::Done, 0.0)].into_iter().collect();
        let t = totals(&map);
        assert!(!t.has_unfinished());
        assert_eq!(t.ratio(), None);
    }

    #[test]
    fn unknown_sizes_do_not_skew_the_ratio() {
        let map: HashMap<_, _> = [
            entry("a/x", "1", 10, 0, DlStatus::Active, 0.0),
            entry("a/x", "2", 50, 100, DlStatus::Paused, 0.0),
        ]
        .into_iter()
        .collect();
        let t = totals(&map);
        assert_eq!(t.ratio(), Some(0.5));
        assert_eq!(t.paused, 1);
        assert!(t.in_flight(), "активный файл без известного размера всё равно «в работе»");
    }
}
