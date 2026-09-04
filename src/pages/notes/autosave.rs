//! Автосейв проекта заметок в `.syn`-бандл.
//!
//! UI-эффект подписан на сигналы ревизий живых страниц/объектов и на
//! ревизию дерева; «грязные» ресурсы складываются в очередь по пути внутри
//! бандла. Фоновый поток раз в 250 мс собирает всё, что не менялось ~700 мс,
//! сериализует (только Mutex модели — сигналов в фоне нет) и применяет одной
//! пачкой через [`project::apply_ops`] — один commit на пачку. Удаления и
//! вложения идут той же очередью.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use super::project::{self, ProjectTree, WriteOp};
use super::state::{LiveObject, NotesCtx};

/// Источник содержимого для записи (сериализация без сигналов).
#[derive(Clone)]
enum SaveSource {
    Doc(DocumentEditorHandle),
    /// Доска/диаграмма/карта/календарь: ручка сериализуется без сигналов.
    Object(LiveObject),
    /// Хранилище событий календаря.
    Calendar(super::calendar::CalendarStoreHandle),
    Tree(Arc<ProjectTree>),
    Bytes(Arc<Vec<u8>>),
    Remove,
}

impl SaveSource {
    fn to_op(&self, path: &str) -> WriteOp {
        match self {
            SaveSource::Doc(h) => WriteOp::Put { path: path.into(), bytes: h.serialize().into_bytes() },
            SaveSource::Object(o) => WriteOp::Put { path: path.into(), bytes: o.serialize().into_bytes() },
            SaveSource::Calendar(c) => WriteOp::Put { path: path.into(), bytes: c.serialize().into_bytes() },
            SaveSource::Tree(t) => WriteOp::Put { path: path.into(), bytes: t.serialize().into_bytes() },
            SaveSource::Bytes(b) => WriteOp::Put { path: path.into(), bytes: (**b).clone() },
            SaveSource::Remove => WriteOp::Remove { path: path.into() },
        }
    }
}

struct PendingSave {
    source: SaveSource,
    rev: u64,
    queued: Instant,
    /// Страница — после записи переиндексировать ссылки.
    page_id: Option<String>,
}

/// Столько тишины должно пройти после последней правки.
const DEBOUNCE: Duration = Duration::from_millis(700);

fn pending() -> &'static Mutex<HashMap<String, PendingSave>> {
    static P: OnceLock<Mutex<HashMap<String, PendingSave>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn saved_revs() -> &'static Mutex<HashMap<String, u64>> {
    static S: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn project_slot() -> &'static Mutex<Option<PathBuf>> {
    static C: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// Путь проекта для фонового писателя.
pub fn set_project_path(path: PathBuf) {
    *project_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

fn project_path() -> Option<PathBuf> {
    project_slot().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Ревизия, которую мы считаем сохранённой.
pub fn saved_rev(path: &str) -> u64 {
    saved_revs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .copied()
        .unwrap_or(0)
}

/// Пометить текущее состояние ресурса сохранённым (загрузка из бандла).
pub fn mark_saved(path: &str, rev: u64) {
    saved_revs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string(), rev);
    pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
}

/// Забыть ресурс (удалён из проекта).
pub fn forget(path: &str) {
    saved_revs().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    pending().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
}

fn enqueue(path: &str, source: SaveSource, rev: u64, page_id: Option<String>, soon: bool) {
    let queued = if soon { Instant::now() - DEBOUNCE } else { Instant::now() };
    pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string(), PendingSave { source, rev, queued, page_id });
}

/// Записать готовые байты (вложение, пустая новая страница/объект).
pub fn queue_bytes(path: &str, bytes: Vec<u8>) {
    enqueue(path, SaveSource::Bytes(Arc::new(bytes)), 0, None, true);
}

/// Удалить файл из бандла.
pub fn queue_remove(path: &str) {
    saved_revs().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    enqueue(path, SaveSource::Remove, 0, None, true);
}

/// Записать всё ожидающее прямо сейчас (закрытие плитки, выход).
pub fn flush_all() {
    let due: Vec<(String, PendingSave)> = {
        let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
        map.drain().collect()
    };
    write_batch(due);
}

fn write_batch(batch: Vec<(String, PendingSave)>) {
    if batch.is_empty() {
        return;
    }
    let Some(path) = project_path() else { return };
    let ops: Vec<WriteOp> = batch.iter().map(|(p, s)| s.source.to_op(p)).collect();
    match project::apply_ops(&path, &ops) {
        Ok(()) => {
            {
                let mut revs = saved_revs().lock().unwrap_or_else(|e| e.into_inner());
                for (p, s) in &batch {
                    if !matches!(s.source, SaveSource::Remove) {
                        revs.insert(p.clone(), s.rev);
                    }
                }
            }
            // Индекс связей — на main-потоке (сигналы thread-local).
            let pages: Vec<(String, String)> = batch
                .iter()
                .zip(ops.iter())
                .filter_map(|((_, s), op)| match (&s.page_id, op) {
                    (Some(id), WriteOp::Put { bytes, .. }) => {
                        Some((id.clone(), String::from_utf8_lossy(bytes).to_string()))
                    }
                    _ => None,
                })
                .collect();
            if !pages.is_empty() {
                if let Some(ctx) = reindex_ctx() {
                    syngui::async_runtime::run_on_main_thread(move || {
                        for (id, content) in pages {
                            ctx.reindex_page(&id, &content);
                        }
                    });
                }
            }
        }
        Err(e) => {
            log::warn!("notes: запись проекта не удалась: {e}");
            // Вернуть в очередь — следующая попытка через дебаунс.
            let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
            for (p, s) in batch {
                map.entry(p).or_insert(s);
            }
        }
    }
}

fn reindex_slot() -> &'static Mutex<Option<NotesCtx>> {
    static C: OnceLock<Mutex<Option<NotesCtx>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn reindex_ctx() -> Option<NotesCtx> {
    *reindex_slot().lock().unwrap_or_else(|e| e.into_inner())
}

/// Эффект подписки + фоновый поток-писатель. Зовётся один раз из lib.rs.
pub fn install_notes_autosave() {
    let ctx = use_context::<NotesCtx>();
    *reindex_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
    set_project_path(ctx.project_path.get_untracked());

    create_effect(move || {
        // Дерево.
        let tree_rev = ctx.tree_rev.get();
        if tree_rev > saved_rev(project::TREE_PATH) {
            enqueue(
                project::TREE_PATH,
                SaveSource::Tree(ctx.tree.get_untracked()),
                tree_rev,
                None,
                false,
            );
        }
        // Страницы.
        for p in ctx.pages.get().iter() {
            let rev = p.handle.revision().get();
            let path = project::page_path(&p.id);
            if rev > saved_rev(&path) {
                enqueue(&path, SaveSource::Doc(p.handle.clone()), rev, Some(p.id.clone()), false);
            }
        }
        // Хранилище событий календаря.
        if let Some(store) = ctx.calendar.get() {
            let rev = store.revision.get();
            if rev > saved_rev(project::CALENDAR_PATH) {
                enqueue(project::CALENDAR_PATH, SaveSource::Calendar(store.clone()), rev, None, false);
            }
        }
        // Доски и диаграммы.
        for o in ctx.objects.get().iter() {
            let rev = o.revision();
            let path = o.bundle_path();
            if rev > saved_rev(&path) {
                enqueue(&path, SaveSource::Object(o.clone()), rev, None, false);
            }
        }
    });

    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("synthos-notes-autosave".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(250));
                let due: Vec<(String, PendingSave)> = {
                    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
                    // Пачка пишется целиком, когда всё в ней отлежалось:
                    // иначе полстраницы уедет в один commit, полстраницы в
                    // другой. Ждём тишины по самому свежему элементу.
                    let all_quiet = map.values().all(|s| s.queued.elapsed() >= DEBOUNCE);
                    if !all_quiet || map.is_empty() {
                        Vec::new()
                    } else {
                        map.drain().collect()
                    }
                };
                write_batch(due);
            })
            .ok();
    });
}
