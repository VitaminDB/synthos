use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::state::CodeSession;

const HISTORY_LIMIT: usize = 30;
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(1);

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize, Clone)]
pub struct DraftRecord {
    pub path: PathBuf,
    pub text: String,
    pub saved_at: u64,
}

pub struct HistoryEntry {
    pub path: PathBuf,
    pub saved_at: u64,
    pub file: PathBuf,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

pub fn drafts_dir() -> Option<PathBuf> {
    let dir = home_dir()?.join(".config/synthos/editor_drafts");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[code-editor] drafts: mkdir {:?}: {e}", dir);
        return None;
    }
    Some(dir)
}

fn history_root() -> Option<PathBuf> {
    let dir = drafts_dir()?.join("history");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[code-editor] drafts: mkdir {:?}: {e}", dir);
        return None;
    }
    Some(dir)
}

fn file_key(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn write_atomic(path: &Path, data: &str) -> std::io::Result<()> {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tmp = path.with_extension(format!("{pid}.{seq}.tmp"));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn save_draft(path: &Path, text: &str) {
    let Some(dir) = drafts_dir() else {
        return;
    };
    let record = DraftRecord {
        path: path.to_path_buf(),
        text: text.to_string(),
        saved_at: now_secs(),
    };
    let json = match serde_json::to_string(&record) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[code-editor] drafts: serialize {:?}: {e}", path);
            return;
        }
    };
    let file = dir.join(format!("{}.draft", file_key(path)));
    if let Err(e) = write_atomic(&file, &json) {
        eprintln!("[code-editor] drafts: write {:?}: {e}", file);
    }
}

pub fn clear_draft(path: &Path) {
    let Some(dir) = drafts_dir() else {
        return;
    };
    let file = dir.join(format!("{}.draft", file_key(path)));
    match std::fs::remove_file(&file) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[code-editor] drafts: remove {:?}: {e}", file),
    }
}

pub fn load_draft_record(path: &Path) -> Option<DraftRecord> {
    let dir = drafts_dir()?;
    let file = dir.join(format!("{}.draft", file_key(path)));
    let raw = std::fs::read_to_string(&file).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn disk_newer_than(path: &Path, saved_at: u64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let mtime = modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    mtime > saved_at
}

pub fn load_all_drafts() -> Vec<DraftRecord> {
    let Some(dir) = drafts_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("draft") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&p) {
            if let Ok(rec) = serde_json::from_str::<DraftRecord>(&raw) {
                out.push(rec);
            }
        }
    }
    out
}

pub fn snapshot_history(path: &Path, text: &str) {
    let Some(root) = history_root() else {
        return;
    };
    let dir = root.join(file_key(path));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[code-editor] drafts: history mkdir {:?}: {e}", dir);
        return;
    }
    if let Some(latest) = newest_snapshot(&dir) {
        if let Ok(raw) = std::fs::read_to_string(&latest) {
            if let Ok(rec) = serde_json::from_str::<DraftRecord>(&raw) {
                if rec.text == text {
                    return;
                }
            }
        }
    }
    let record = DraftRecord {
        path: path.to_path_buf(),
        text: text.to_string(),
        saved_at: now_secs(),
    };
    let json = match serde_json::to_string(&record) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[code-editor] drafts: history serialize {:?}: {e}", path);
            return;
        }
    };
    let file = dir.join(format!("{:039}.snap", now_nanos()));
    if let Err(e) = write_atomic(&file, &json) {
        eprintln!("[code-editor] drafts: history write {:?}: {e}", file);
        return;
    }
    trim_history(&dir);
}

fn newest_snapshot(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("snap"))
        .max()
}

fn trim_history(dir: &Path) {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(e) => e
            .flatten()
            .map(|x| x.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("snap"))
            .collect(),
        Err(_) => return,
    };
    if files.len() <= HISTORY_LIMIT {
        return;
    }
    files.sort();
    let remove_count = files.len() - HISTORY_LIMIT;
    for f in files.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(f);
    }
}

pub fn list_history(path: &Path) -> Vec<HistoryEntry> {
    let Some(root) = history_root() else {
        return Vec::new();
    };
    let dir = root.join(file_key(path));
    let mut entries: Vec<HistoryEntry> = Vec::new();
    let read = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };
    for e in read.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("snap") {
            continue;
        }
        let saved_at = std::fs::read_to_string(&p)
            .ok()
            .and_then(|raw| serde_json::from_str::<DraftRecord>(&raw).ok())
            .map(|r| r.saved_at)
            .unwrap_or(0);
        entries.push(HistoryEntry {
            path: path.to_path_buf(),
            saved_at,
            file: p,
        });
    }
    entries.sort_by(|a, b| b.file.cmp(&a.file));
    entries
}

pub fn read_history(entry: &HistoryEntry) -> Option<String> {
    let raw = std::fs::read_to_string(&entry.file).ok()?;
    let record: DraftRecord = serde_json::from_str(&raw).ok()?;
    Some(record.text)
}

pub fn human_age(saved_at: u64) -> String {
    let now = now_secs();
    let age = now.saturating_sub(saved_at);
    if age < 10 {
        return "только что".to_string();
    }
    if age < 60 {
        return format!("{age} с назад");
    }
    let mins = age / 60;
    if mins < 60 {
        return format!("{mins} мин назад");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours} ч назад");
    }
    let days = hours / 24;
    format!("{days} дн назад")
}

pub fn install_draft_autosave(session: CodeSession) {
    use std::sync::Arc;

    let generation = Arc::new(AtomicU64::new(0));

    syngui::signal::create_effect(move || {
        let _ = session.file_contents.get();
        let my_gen = generation.fetch_add(1, Ordering::Relaxed) + 1;
        let gen_arc = generation.clone();

        let file_contents = session.file_contents.get_untracked();
        let disk_contents = session.disk_contents.get_untracked();

        std::thread::Builder::new()
            .name("synthos-draft-save".to_string())
            .spawn(move || {
                std::thread::sleep(AUTOSAVE_DEBOUNCE);
                if gen_arc.load(Ordering::Relaxed) != my_gen {
                    return;
                }
                persist_dirty(&file_contents, &disk_contents);
            })
            .ok();
    });
}

fn persist_dirty(
    file_contents: &HashMap<PathBuf, String>,
    disk_contents: &HashMap<PathBuf, String>,
) {
    for (path, text) in file_contents {
        if disk_contents.get(path).map(|d| d == text).unwrap_or(false) {
            continue;
        }
        let disk = std::fs::read_to_string(path).ok();
        if disk.as_deref() != Some(text.as_str()) {
            save_draft(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_key_is_stable_and_distinct() {
        let a = file_key(Path::new("/home/u/a.rs"));
        let b = file_key(Path::new("/home/u/a.rs"));
        let c = file_key(Path::new("/home/u/b.rs"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn human_age_buckets() {
        let now = now_secs();
        assert_eq!(human_age(now), "только что");
        assert_eq!(human_age(now.saturating_sub(120)), "2 мин назад");
        assert_eq!(human_age(now.saturating_sub(7200)), "2 ч назад");
        assert_eq!(human_age(now.saturating_sub(172800)), "2 дн назад");
    }
}
