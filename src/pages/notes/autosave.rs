//! Автосейв заметок.
//!
//! UI-эффект подписан на сигналы ревизий всех открытых страниц
//! (`DocumentEditorHandle::revision`) и складывает «грязные» страницы в
//! очередь; фоновый поток раз в 250мс сбрасывает на диск те, что не
//! редактировались ~700мс (дебаунс набора). Сериализация трогает только
//! Mutex модели — сигналов в фоновом потоке нет.
//!
//! Каждая своя запись регистрируется в [`mark_recent_save`]-карте: watcher
//! игнорирует Modify-события в 2-секундном окне после неё — иначе
//! автосейв и слежение зациклились бы (риск №4 плана).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use super::base::BaseHandle;
use super::canvas::CanvasHandle;
use super::state::{NotePayload, NotesCtx};
use super::storage;

/// Источник содержимого для записи (сериализация без сигналов —
/// безопасна в фоновом потоке).
#[derive(Clone)]
enum SaveSource {
    Doc(DocumentEditorHandle),
    Base(BaseHandle),
    Canvas(CanvasHandle),
}

impl SaveSource {
    fn serialize(&self) -> String {
        match self {
            SaveSource::Doc(h) => h.serialize(),
            SaveSource::Base(h) => h.serialize(),
            SaveSource::Canvas(h) => h.serialize(),
        }
    }
}

/// Ждущая записи страница.
struct PendingSave {
    source: SaveSource,
    abs: PathBuf,
    rev: u64,
    queued: Instant,
}

/// Дебаунс: столько тишины должно пройти после последней правки.
const DEBOUNCE: Duration = Duration::from_millis(700);
/// Окно, в котором watcher считает Modify нашей собственной записью.
const RECENT_WINDOW: Duration = Duration::from_secs(2);

fn pending() -> &'static Mutex<HashMap<String, PendingSave>> {
    static P: OnceLock<Mutex<HashMap<String, PendingSave>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn saved_revs() -> &'static Mutex<HashMap<String, u64>> {
    static S: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recent_saves() -> &'static Mutex<HashMap<PathBuf, Instant>> {
    static R: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Была ли по этому абсолютному пути наша запись в последние 2 секунды.
pub fn is_recent_own_save(abs: &std::path::Path) -> bool {
    let map = recent_saves().lock().unwrap_or_else(|e| e.into_inner());
    map.get(abs)
        .map(|t| t.elapsed() < RECENT_WINDOW)
        .unwrap_or(false)
}

fn mark_recent_save(abs: PathBuf) {
    let mut map = recent_saves().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    map.retain(|_, t| t.elapsed() < RECENT_WINDOW * 4);
    map.insert(abs, now);
}

/// Ревизия, которую мы считаем сохранённой (для детекта конфликтов).
pub fn saved_rev(path: &str) -> u64 {
    saved_revs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .copied()
        .unwrap_or(0)
}

/// Пометить текущее состояние страницы сохранённым (открытие, перечитка).
pub fn mark_saved(path: &str, rev: u64) {
    saved_revs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string(), rev);
    pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
}

/// Забыть страницу (плитка закрыта / файл удалён).
pub fn forget(path: &str) {
    saved_revs().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
}

fn write_now(path: &str, save: &PendingSave) {
    let content = save.source.serialize();
    mark_recent_save(save.abs.clone());
    match storage::save_atomic_abs(&save.abs, &content) {
        Ok(()) => {
            saved_revs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.to_string(), save.rev);
            // Индекс связей — на main-потоке (сигналы thread-local).
            let rel = path.to_string();
            if let Some(ctx) = reindex_ctx() {
                syngui::async_runtime::run_on_main_thread(move || {
                    ctx.reindex_page(&rel, &content);
                });
            }
        }
        Err(e) => log::warn!("notes: не удалось сохранить {path}: {e}"),
    }
}

/// NotesCtx для переиндексации после записи (Copy, Send).
fn reindex_slot() -> &'static Mutex<Option<NotesCtx>> {
    static C: OnceLock<Mutex<Option<NotesCtx>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn set_reindex_ctx(ctx: NotesCtx) {
    *reindex_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
}

fn reindex_ctx() -> Option<NotesCtx> {
    *reindex_slot().lock().unwrap_or_else(|e| e.into_inner())
}

/// Немедленно сбросить страницу на диск, если по ней есть очередь
/// (закрытие плитки, кнопка «Перезаписать» в конфликте).
pub fn flush_now(path: &str) {
    let entry = pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    if let Some(save) = entry {
        write_now(path, &save);
    }
}

/// Принудительная запись текущей модели (конфликт → «Перезаписать»),
/// независимо от очереди.
pub fn force_save(ctx: &NotesCtx, path: &str) {
    let Some(note) = ctx.open.get_untracked().into_iter().find(|n| n.path == path) else {
        return;
    };
    let source = match &note.payload {
        NotePayload::Page { handle, .. } => SaveSource::Doc(handle.clone()),
        NotePayload::Base(h) => SaveSource::Base(h.clone()),
        NotePayload::Canvas(h) => SaveSource::Canvas(h.clone()),
        NotePayload::Raw => return,
    };
    let root = ctx.vault_path.get_untracked();
    let save = PendingSave {
        abs: storage::abs_path(&root, path),
        rev: note.revision(),
        source,
        queued: Instant::now(),
    };
    pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    write_now(path, &save);
}

/// Эффект подписки + фоновый поток-писатель. Зовётся один раз из lib.rs.
pub fn install_notes_autosave() {
    let ctx = use_context::<NotesCtx>();
    set_reindex_ctx(ctx);

    create_effect(move || {
        let open = ctx.open.get();
        let root = ctx.vault_path.get();
        for n in open.iter() {
            // `.get()` — подписка эффекта на каждую правку страницы.
            let (rev, source) = match &n.payload {
                NotePayload::Page { handle, .. } => {
                    (handle.revision().get(), SaveSource::Doc(handle.clone()))
                }
                NotePayload::Base(h) => (h.revision.get(), SaveSource::Base(h.clone())),
                NotePayload::Canvas(h) => {
                    (h.revision.get(), SaveSource::Canvas(h.clone()))
                }
                NotePayload::Raw => continue,
            };
            if rev <= saved_rev(&n.path) {
                continue;
            }
            let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(
                n.path.clone(),
                PendingSave {
                    source,
                    abs: storage::abs_path(&root, &n.path),
                    rev,
                    queued: Instant::now(),
                },
            );
        }
    });

    // Поток-писатель: один на процесс.
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("synthos-notes-autosave".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(250));
                let due: Vec<(String, PendingSave)> = {
                    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
                    let keys: Vec<String> = map
                        .iter()
                        .filter(|(_, s)| s.queued.elapsed() >= DEBOUNCE)
                        .map(|(k, _)| k.clone())
                        .collect();
                    keys.into_iter()
                        .filter_map(|k| map.remove(&k).map(|s| (k, s)))
                        .collect()
                };
                for (path, save) in due {
                    write_now(&path, &save);
                }
            })
            .ok();
    });
}
