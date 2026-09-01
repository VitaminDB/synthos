//! Медиа-адаптер заметок: CAS-блобы ↔ DocumentEditor.
//!
//! Вложения заметок живут в общем content-addressed хранилище чатов
//! (`~/.config/synthos/blobs`); в markdown — ссылки `blob:<sha256>.<ext>`.
//! Резолвер отдаёт редактору локальный файл, PCM-бины волны для аудио
//! (маленькие файлы декодируются symphonia синхронно и кэшируются);
//! ingest при дропе — `blobs::store_file` в фоновом потоке с последующим
//! `handle.patch_media` на main.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use syngui::widgets::input::document_editor::{
    DocMediaResolver, MediaKind, ResolvedMedia,
};

use crate::syn_chat::attach::blobs;

use super::state::NotesCtx;
use super::storage;

/// Потолок размера аудио-файла для синхронного декода волны.
const MAX_WAVEFORM_BYTES: u64 = 12 * 1024 * 1024;

pub struct NotesMediaResolver {
    /// Корень vault'а — для относительных путей в md.
    pub vault_root: PathBuf,
}

pub fn resolver(ctx: NotesCtx) -> Arc<NotesMediaResolver> {
    Arc::new(NotesMediaResolver { vault_root: ctx.vault_path.get_untracked() })
}

/// `blob:<sha>.<ext>` → (sha, ext).
fn parse_blob_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("blob:")?;
    let (sha, ext) = rest.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then_some((sha, ext))
}

impl DocMediaResolver for NotesMediaResolver {
    fn resolve(&self, url: &str) -> Option<ResolvedMedia> {
        let path = if let Some((sha, ext)) = parse_blob_url(url) {
            blobs::blob_path(sha, ext)
        } else if url.contains("://") || url.starts_with("pending:") {
            return None;
        } else {
            // Относительный путь внутри vault'а либо абсолютный.
            let p = Path::new(url);
            if p.is_absolute() { p.to_path_buf() } else { self.vault_root.join(url) }
        };
        path.is_file().then(|| ResolvedMedia {
            kind: MediaKind::detect(url, &Default::default()),
            path,
        })
    }

    fn pcm_bins(&self, url: &str, bins: usize) -> Option<Vec<f32>> {
        let resolved = self.resolve(url)?;
        // Кэш процессный: декодировать файл на каждый mount дорого.
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
            let buf = crate::pages::node_editor::nodes::decode::decode_file(&resolved.path)
                .ok()?;
            Some(syngui::audio::compute_rms_bins(&buf.pcm, buf.channels, bins))
        })();
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(url.to_string(), computed.clone());
        computed
    }
}

/// Дроп файла в редактор: фоновый ingest в CAS + patch pending-блока.
pub fn ingest_dropped_file(ctx: NotesCtx, note_path: String, file: PathBuf, token: String) {
    let handle = ctx
        .open
        .get_untracked()
        .into_iter()
        .find(|n| n.path == note_path)
        .and_then(|n| n.page_handle().cloned());
    let Some(handle) = handle else { return };
    std::thread::spawn(move || match blobs::store_file(&file) {
        Ok((sha, _size, ext)) => {
            let url = format!("blob:{sha}.{ext}");
            syngui::async_runtime::run_on_main_thread(move || {
                if handle.patch_media(&token, &url) {
                    // Перестройка блоков редактора без репарса.
                    ctx.media_epoch.set(ctx.media_epoch.get_untracked() + 1);
                }
            });
        }
        Err(e) => log::warn!("notes: ingest {} не удался: {e}", file.display()),
    });
}

/// Все sha256, на которые ссылаются страницы vault'а — для GC блобов.
pub fn collect_blob_refs(referenced: &mut HashSet<String>) {
    let cfg = crate::config::AppConfig::load();
    let root = storage::resolve_vault_path(&cfg.notes_vault_path);
    for entry in storage::scan(&root) {
        if entry.kind != storage::VaultEntryKind::Page {
            continue;
        }
        let Ok(content) = storage::load(&root, &entry.rel) else { continue };
        collect_blob_refs_in(&content, referenced);
    }
}

/// `blob:<sha>` в одном md-тексте.
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
    fn blob_url_parsing() {
        let sha = "a".repeat(64);
        let url = format!("blob:{sha}.mp4");
        let (s, e) = parse_blob_url(&url).unwrap();
        assert_eq!(s, sha);
        assert_eq!(e, "mp4");
        assert!(parse_blob_url("blob:short.mp4").is_none());
        assert!(parse_blob_url("https://x/y.mp4").is_none());
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
