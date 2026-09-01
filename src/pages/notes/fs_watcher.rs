//! Watcher vault-папки заметок (по образцу `code_editor::fs_watcher`).
//!
//! Внешние правки (другой редактор, синк) отражаются в UI:
//! - структурные события (создание/удаление/переименование) → перескан
//!   дерева;
//! - Modify открытой страницы: если несохранённых правок нет — тихая
//!   перезагрузка модели; есть — конфликт-баннер в редакторе
//!   («перечитать / перезаписать»);
//! - собственные записи автосейва отфильтровываются окном
//!   [`autosave::is_recent_own_save`].
//!
//! notify-события дрейнятся в std-потоке с дебаунсом 300мс и применяются
//! к сигналам через `run_on_main_thread` (signal-runtime — только main).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind};
use syngui::prelude::*;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::state::NotesCtx;
use super::{autosave, storage};

pub struct NotesWatcher {
    /// RAII: drop = стоп слежения.
    _watcher: RecommendedWatcher,
}

fn holder() -> &'static Mutex<Option<NotesWatcher>> {
    static H: OnceLock<Mutex<Option<NotesWatcher>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

/// Запустить (или перезапустить) слежение за vault'ом. Зовётся из lib.rs
/// после `provide_context(NotesCtx)`.
pub fn install_notes_watcher() {
    let ctx = use_context::<NotesCtx>();
    let root = ctx.vault_path.get_untracked();
    match start(ctx, root) {
        Ok(w) => {
            *holder().lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
        }
        Err(e) => log::warn!("notes: watcher не запустился: {e}"),
    }
}

fn start(ctx: NotesCtx, root: PathBuf) -> std::result::Result<NotesWatcher, notify::Error> {
    storage::ensure_vault(&root);
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    std::thread::Builder::new()
        .name("synthos-notes-watcher".to_string())
        .spawn(move || dispatch_loop(ctx, rx, root))
        .ok();

    Ok(NotesWatcher { _watcher: watcher })
}

enum FsEvent {
    Modified(PathBuf),
    Structure,
}

fn translate(res: notify::Result<notify::Event>, root: &Path) -> Vec<FsEvent> {
    let ev = match res {
        Ok(e) => e,
        Err(e) => {
            log::warn!("notes: notify error: {e}");
            return Vec::new();
        }
    };
    let Some(path) = ev.paths.into_iter().next() else { return Vec::new() };
    let Ok(rel) = path.strip_prefix(root) else { return Vec::new() };
    // Скрытые файлы и наши tmp-файлы атомарной записи не интересны.
    let hidden_or_tmp = rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || s.ends_with(".tmp~")
    });
    if hidden_or_tmp {
        return Vec::new();
    }

    let mut out = Vec::new();
    match ev.kind {
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Any) => {
            out.push(FsEvent::Modified(path));
        }
        EventKind::Modify(ModifyKind::Name(_)) | EventKind::Create(_) => {
            out.push(FsEvent::Structure);
            // Atomic-rename (наш же save_atomic и внешние редакторы):
            // содержимое целевого файла сменилось.
            if path.is_file() {
                out.push(FsEvent::Modified(path));
            }
        }
        EventKind::Remove(_) => out.push(FsEvent::Structure),
        _ => {}
    }
    out
}

fn dispatch_loop(
    ctx: NotesCtx,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    root: PathBuf,
) {
    let debounce = Duration::from_millis(300);
    loop {
        let raw = match rx.recv() {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut modified: HashSet<PathBuf> = HashSet::new();
        let mut structure = false;
        let mut acc = |ev: FsEvent| match ev {
            FsEvent::Modified(p) => {
                modified.insert(p);
            }
            FsEvent::Structure => structure = true,
        };
        translate(raw, &root).into_iter().for_each(&mut acc);

        let deadline = Instant::now() + debounce;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                break;
            }
            match rx.recv_timeout(timeout) {
                Ok(raw) => translate(raw, &root).into_iter().for_each(&mut acc),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        if modified.is_empty() && !structure {
            continue;
        }
        let root_clone = root.clone();
        syngui::async_runtime::run_on_main_thread(move || {
            apply_batch(ctx, modified, structure, root_clone);
        });
    }
}

fn apply_batch(ctx: NotesCtx, modified: HashSet<PathBuf>, structure: bool, root: PathBuf) {
    if structure {
        ctx.rescan();
    }
    for abs in modified {
        // Наша собственная запись в 2-секундном окне — не событие.
        if autosave::is_recent_own_save(&abs) {
            continue;
        }
        let Ok(rel) = abs.strip_prefix(&root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let is_open = ctx.open.get_untracked().iter().any(|n| n.path == rel);
        if !is_open {
            continue;
        }
        let Ok(new_content) = std::fs::read_to_string(&abs) else { continue };
        ctx.apply_external_change(&rel, new_content);
    }
}
