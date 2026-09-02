//! Медиа-адаптер заметок: вложения внутри бандла ↔ DocumentEditor.
//!
//! Вложения лежат в проекте File-чанками `notes/assets/<sha256>.<ext>`; в
//! markdown — ссылки `asset:<sha256>.<ext>`. Плеерам и картинкам нужен
//! файл на диске, поэтому при первом обращении чанк распаковывается в
//! кэш `~/.cache/synthos/notes-assets/<проект>/`. Ingest при дропе — sha256
//! + запись в бандл через автосейв (и сразу в кэш), затем
//! `handle.patch_media` на main. Старые `blob:`-ссылки (первая волна)
//! резолвятся в общий CAS чатов.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use sha2::{Digest, Sha256};
use syngui::widgets::input::document_editor::{DocMediaResolver, MediaKind, ResolvedMedia};

use crate::syn_chat::attach::blobs;

use super::autosave;
use super::project;
use super::state::NotesCtx;

/// Потолок размера аудио-файла для синхронного декода волны.
const MAX_WAVEFORM_BYTES: u64 = 12 * 1024 * 1024;

pub struct NotesMediaResolver {
    pub project_path: PathBuf,
}

pub fn resolver(ctx: NotesCtx) -> Arc<NotesMediaResolver> {
    Arc::new(NotesMediaResolver { project_path: ctx.project_path.get_untracked() })
}

/// Папка кэша распакованных вложений проекта.
pub fn assets_cache_dir(project_path: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            home.join(".cache")
        });
    let mut h = DefaultHasher::new();
    project_path.hash(&mut h);
    let stem = project::project_title(project_path)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    base.join("synthos").join("notes-assets").join(format!("{stem}-{:08x}", h.finish() as u32))
}

/// `asset:<sha>.<ext>` → имя файла вложения.
pub fn parse_asset_url(url: &str) -> Option<&str> {
    let name = url.strip_prefix("asset:")?;
    let (sha, ext) = name.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) && !ext.is_empty() && !name.contains('/'))
        .then_some(name)
}

/// `blob:<sha>.<ext>` → (sha, ext) — наследие первой волны.
fn parse_blob_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("blob:")?;
    let (sha, ext) = rest.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then_some((sha, ext))
}

impl NotesMediaResolver {
    /// Файл вложения в кэше; распаковывается из бандла при первом обращении.
    fn extracted(&self, name: &str) -> Option<PathBuf> {
        let dir = assets_cache_dir(&self.project_path);
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
        let bytes = project::read_bytes(&self.project_path, &project::asset_path(name))?;
        std::fs::create_dir_all(&dir).ok()?;
        let tmp = dir.join(format!("{name}.tmp~"));
        std::fs::write(&tmp, bytes).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
        Some(path)
    }
}

impl DocMediaResolver for NotesMediaResolver {
    fn resolve(&self, url: &str) -> Option<ResolvedMedia> {
        let path = if let Some(name) = parse_asset_url(url) {
            self.extracted(name)?
        } else if let Some((sha, ext)) = parse_blob_url(url) {
            blobs::blob_path(sha, ext)
        } else if url.contains("://") || url.starts_with("pending:") {
            return None;
        } else {
            let p = Path::new(url);
            if !p.is_absolute() {
                return None;
            }
            p.to_path_buf()
        };
        path.is_file().then(|| ResolvedMedia {
            kind: MediaKind::detect(url, &Default::default()),
            path,
        })
    }

    fn pcm_bins(&self, url: &str, bins: usize) -> Option<Vec<f32>> {
        let resolved = self.resolve(url)?;
        static CACHE: OnceLock<Mutex<HashMap<String, Option<Vec<f32>>>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(url) {
            return hit.clone();
        }
        let computed = (|| {
            let size = std::fs::metadata(&resolved.path).ok()?.len();
            if size > MAX_WAVEFORM_BYTES {
                return None;
            }
            let buf = crate::pages::node_editor::nodes::decode::decode_file(&resolved.path).ok()?;
            Some(syngui::audio::compute_rms_bins(&buf.pcm, buf.channels, bins))
        })();
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(url.to_string(), computed.clone());
        computed
    }
}

/// Дроп файла в редактор: фоновый ingest в бандл + patch pending-блока.
pub fn ingest_dropped_file(ctx: NotesCtx, page_id: String, file: PathBuf, token: String) {
    let Some(page) = ctx.page(&page_id) else { return };
    let handle = page.handle;
    let project_path = ctx.project_path.get_untracked();
    std::thread::spawn(move || {
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("notes: ingest {} не удался: {e}", file.display());
                return;
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha = format!("{:x}", hasher.finalize());
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "bin".to_string());
        let name = format!("{sha}.{ext}");
        // Сразу в кэш — плеер увидит файл до commit'а бандла.
        let dir = assets_cache_dir(&project_path);
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join(&name), &bytes);
        }
        autosave::queue_bytes(&project::asset_path(&name), bytes);
        let url = format!("asset:{name}");
        syngui::async_runtime::run_on_main_thread(move || {
            if handle.patch_media(&token, &url) {
                ctx.bump_doc_epoch();
            }
        });
    });
}

/// Все sha256 общего CAS, на которые ещё ссылаются страницы проекта
/// (`blob:` первой волны) — чтобы GC блобов чатов их не выбросил.
pub fn collect_blob_refs(referenced: &mut HashSet<String>) {
    let cfg = crate::config::AppConfig::load();
    let path = project::resolve_project_path(&cfg.notes_project_path);
    if !path.is_file() {
        return;
    }
    let tree = project::read_tree(&path);
    for node in tree.all() {
        if let Some(content) = project::read_text(&path, &project::page_path(&node.id)) {
            collect_blob_refs_in(&content, referenced);
        }
    }
}

fn collect_blob_refs_in(content: &str, referenced: &mut HashSet<String>) {
    let mut rest = content;
    while let Some(pos) = rest.find("blob:") {
        let tail = &rest[pos + 5..];
        let sha: String = tail.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let sha_len = sha.len();
        if sha_len == 64 {
            referenced.insert(sha);
        }
        rest = &tail[sha_len.min(tail.len())..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_url_parsing() {
        let sha = "a".repeat(64);
        assert_eq!(parse_asset_url(&format!("asset:{sha}.mp4")), Some(format!("{sha}.mp4")).as_deref());
        assert!(parse_asset_url("asset:short.mp4").is_none());
        assert!(parse_asset_url(&format!("asset:{sha}")).is_none());
        assert!(parse_asset_url("https://x/y.mp4").is_none());
    }

    #[test]
    fn blob_refs_collector() {
        let sha = "b".repeat(64);
        let md = format!("![v](blob:{sha}.mp4){{loop}} и ещё blob:{}", "c".repeat(64));
        let mut set = HashSet::new();
        collect_blob_refs_in(&md, &mut set);
        assert!(set.contains(&sha));
        assert_eq!(set.len(), 2);
    }
}
