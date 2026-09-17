//! Автосейв проектов заметок в `.syn`-бандлы.
//!
//! UI-эффект подписан на сигналы ревизий живых страниц/объектов и на
//! ревизию дерева активного проекта; «грязные» ресурсы складываются в
//! очередь. Открытых проектов может быть несколько (плитка рейла на каждый),
//! а пути внутри бандла у них совпадают (`notes/tree.json`,
//! `notes/calendar.json`, …) — поэтому и очередь, и сохранённые ревизии
//! ключуются парой «файл проекта + путь в бандле». Без проекта в ключе
//! дерево второго проекта с ревизией ниже, чем у первого, считалось бы уже
//! записанным.
//!
//! Фоновый поток раз в 250 мс собирает всё, что не менялось ~700 мс,
//! сериализует (только Mutex модели — сигналов в фоне нет) и применяет через
//! [`project::apply_ops`] — один commit на проект. Удаления и вложения идут
//! той же очередью.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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

/// Файл проекта + путь внутри бандла.
type Key = (PathBuf, String);

struct PendingSave {
    source: SaveSource,
    rev: u64,
    queued: Instant,
    /// Страница — после записи переиндексировать ссылки.
    page_id: Option<String>,
}

/// Столько тишины должно пройти после последней правки.
const DEBOUNCE: Duration = Duration::from_millis(700);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn pending() -> &'static Mutex<HashMap<Key, PendingSave>> {
    static P: OnceLock<Mutex<HashMap<Key, PendingSave>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn saved_revs() -> &'static Mutex<HashMap<Key, u64>> {
    static S: OnceLock<Mutex<HashMap<Key, u64>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Запись в бандлы идёт под этим замком: фоновый поток, сброс перед
/// закрытием проекта и копия «Сохранить как» не пересекаются — копия не
/// читает файл посреди commit'а, а закрытый проект не получает записей
/// после своего сброса.
fn write_lock() -> &'static Mutex<()> {
    static W: OnceLock<Mutex<()>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(()))
}

thread_local! {
    /// Проект, в который уходят вызовы без явного файла (`queue_bytes`,
    /// `mark_saved`, …). Все они в приложении зовутся с main-потока, где
    /// путь ставит `NotesCtx` при смене проекта; у тестов — свой поток и
    /// свой проект, и параллельные тесты не пишут в чужие файлы.
    static CURRENT: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}

/// Активный проект для вызовов без явного файла; пустой путь — проекта нет.
pub fn set_project_path(path: PathBuf) {
    CURRENT.with(|c| *c.borrow_mut() = path);
}

/// Проект, куда сейчас уходят вызовы без явного файла.
pub fn project_path() -> PathBuf {
    current()
}

fn current() -> PathBuf {
    CURRENT.with(|c| c.borrow().clone())
}

fn key(path: &str) -> Key {
    (current(), path.to_string())
}

/// Ревизия, которую мы считаем сохранённой.
pub fn saved_rev(path: &str) -> u64 {
    lock(saved_revs()).get(&key(path)).copied().unwrap_or(0)
}

/// Пометить текущее состояние ресурса сохранённым (загрузка из бандла).
pub fn mark_saved(path: &str, rev: u64) {
    let k = key(path);
    lock(saved_revs()).insert(k.clone(), rev);
    lock(pending()).remove(&k);
}

/// Забыть ресурс (удалён из проекта).
pub fn forget(path: &str) {
    let k = key(path);
    lock(saved_revs()).remove(&k);
    lock(pending()).remove(&k);
}

fn enqueue_to(project: PathBuf, path: &str, source: SaveSource, rev: u64, page_id: Option<String>, soon: bool) {
    if project.as_os_str().is_empty() {
        log::warn!("notes: запись {path} без открытого проекта пропущена");
        return;
    }
    let queued = if soon { Instant::now() - DEBOUNCE } else { Instant::now() };
    lock(pending()).insert((project, path.to_string()), PendingSave { source, rev, queued, page_id });
}

fn enqueue(path: &str, source: SaveSource, rev: u64, page_id: Option<String>, soon: bool) {
    enqueue_to(current(), path, source, rev, page_id, soon);
}

/// Записать готовые байты (вложение, пустая новая страница/объект).
pub fn queue_bytes(path: &str, bytes: Vec<u8>) {
    enqueue(path, SaveSource::Bytes(Arc::new(bytes)), 0, None, true);
}

/// То же в явно указанный проект — для фоновых потоков (дроп файла).
pub fn queue_bytes_to(project: &Path, path: &str, bytes: Vec<u8>) {
    enqueue_to(project.to_path_buf(), path, SaveSource::Bytes(Arc::new(bytes)), 0, None, true);
}

/// Удалить файл из бандла.
pub fn queue_remove(path: &str) {
    lock(saved_revs()).remove(&key(path));
    enqueue(path, SaveSource::Remove, 0, None, true);
}

/// Есть ли у проекта незаписанные правки в очереди.
pub fn has_pending(project: &Path) -> bool {
    lock(pending()).keys().any(|(p, _)| p == project)
}

/// Записать всё ожидающее прямо сейчас (выход из приложения).
pub fn flush_all() -> std::result::Result<(), String> {
    let _w = lock(write_lock());
    let due: Vec<(Key, PendingSave)> = lock(pending()).drain().collect();
    write_batch(due)
}

/// Записать очередь одного проекта прямо сейчас (Ctrl+S, закрытие,
/// «Сохранить как»). Ошибка — очередь остаётся на месте.
pub fn flush_project(project: &Path) -> std::result::Result<(), String> {
    let _w = lock(write_lock());
    let due: Vec<(Key, PendingSave)> = {
        let mut map = lock(pending());
        let keys: Vec<Key> = map.keys().filter(|(p, _)| p == project).cloned().collect();
        keys.into_iter().filter_map(|k| map.remove(&k).map(|v| (k, v))).collect()
    };
    write_batch(due)
}

/// Выполнить `f`, пока никто не пишет в бандлы (копия файла проекта).
pub fn with_write_lock<T>(f: impl FnOnce() -> T) -> T {
    let _w = lock(write_lock());
    f()
}

/// Забыть всё о проекте (закрыт): сохранённые ревизии и хвост очереди.
/// При повторном открытии ревизии ручек начнутся с нуля.
pub fn forget_project(project: &Path) {
    lock(saved_revs()).retain(|(p, _), _| p != project);
    lock(pending()).retain(|(p, _), _| p != project);
}

/// Проект сохранён под новым именем: содержимое совпадает, поэтому
/// сохранённые ревизии и очередь переезжают на новый файл.
pub fn rekey_project(old: &Path, new: &Path) {
    let mut revs = lock(saved_revs());
    let moved: Vec<(Key, u64)> = revs.iter().filter(|((p, _), _)| p == old).map(|(k, v)| (k.clone(), *v)).collect();
    for ((_, path), rev) in moved {
        revs.remove(&(old.to_path_buf(), path.clone()));
        revs.insert((new.to_path_buf(), path), rev);
    }
    drop(revs);
    let mut map = lock(pending());
    let keys: Vec<Key> = map.keys().filter(|(p, _)| p == old).cloned().collect();
    for k in keys {
        if let Some(v) = map.remove(&k) {
            map.insert((new.to_path_buf(), k.1), v);
        }
    }
}

/// Пачка → один commit на проект. Вызывается под `write_lock`.
fn write_batch(batch: Vec<(Key, PendingSave)>) -> std::result::Result<(), String> {
    let mut by_project: BTreeMap<PathBuf, Vec<(String, PendingSave)>> = BTreeMap::new();
    for ((proj, path), save) in batch {
        by_project.entry(proj).or_default().push((path, save));
    }
    let mut first_error = None;
    for (proj, items) in by_project {
        if let Err(e) = write_project(&proj, items) {
            log::warn!("notes: запись проекта {} не удалась: {e}", proj.display());
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn write_project(proj: &Path, items: Vec<(String, PendingSave)>) -> std::result::Result<(), String> {
    let ops: Vec<WriteOp> = items.iter().map(|(p, s)| s.source.to_op(p)).collect();
    if let Err(e) = project::apply_ops(proj, &ops) {
        // Вернуть в очередь — следующая попытка через дебаунс; более свежая
        // правка того же ресурса, пришедшая за это время, важнее.
        let mut map = lock(pending());
        for (p, s) in items {
            map.entry((proj.to_path_buf(), p)).or_insert(s);
        }
        return Err(e);
    }
    {
        let mut revs = lock(saved_revs());
        for (p, s) in &items {
            if !matches!(s.source, SaveSource::Remove) {
                revs.insert((proj.to_path_buf(), p.clone()), s.rev);
            }
        }
    }
    // Индекс связей — на main-потоке (сигналы thread-local) и только если
    // проект всё ещё активен: у неактивного индекс пересоберётся при
    // возврате к нему.
    let pages: Vec<(String, String)> = items
        .iter()
        .zip(ops.iter())
        .filter_map(|((_, s), op)| match (&s.page_id, op) {
            (Some(id), WriteOp::Put { bytes, .. }) => Some((id.clone(), String::from_utf8_lossy(bytes).to_string())),
            _ => None,
        })
        .collect();
    if !pages.is_empty() {
        if let Some(ctx) = reindex_ctx() {
            let proj = proj.to_path_buf();
            syngui::async_runtime::run_on_main_thread(move || {
                if ctx.project_path.get_untracked() != proj {
                    return;
                }
                for (id, content) in pages {
                    ctx.reindex_page(&id, &content);
                }
            });
        }
    }
    Ok(())
}

fn reindex_slot() -> &'static Mutex<Option<NotesCtx>> {
    static C: OnceLock<Mutex<Option<NotesCtx>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn reindex_ctx() -> Option<NotesCtx> {
    *lock(reindex_slot())
}

/// Поставить в очередь всё, что в активном проекте изменилось после
/// последней записи. Тело эффекта автосейва; зовётся и напрямую — перед
/// сменой или закрытием проекта, когда эффект по свежей правке ещё не
/// успел отработать.
pub fn enqueue_dirty(ctx: NotesCtx) {
    // Дерево.
    let tree_rev = ctx.tree_rev.get();
    if tree_rev > saved_rev(project::TREE_PATH) {
        enqueue(project::TREE_PATH, SaveSource::Tree(ctx.tree.get_untracked()), tree_rev, None, false);
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
    // Журнал изменений: переписываются только грязные месяцы.
    if let Some(log) = ctx.activity.get() {
        let rev = log.revision.get();
        if rev > 0 {
            for (path, bytes) in log.take_dirty() {
                enqueue(&path, SaveSource::Bytes(Arc::new(bytes)), rev, None, false);
            }
        }
    }
    // Напоминания: настройки и что показано.
    if let Some(state) = ctx.reminder_state.get() {
        let rev = state.revision.get();
        if rev > saved_rev(project::REMINDERS_PATH) {
            enqueue(project::REMINDERS_PATH, SaveSource::Bytes(Arc::new(state.serialize().into_bytes())), rev, None, false);
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
}

/// Эффект подписки + фоновый поток-писатель. Зовётся один раз из lib.rs.
pub fn install_notes_autosave() {
    let ctx = use_context::<NotesCtx>();
    *lock(reindex_slot()) = Some(ctx);
    set_project_path(ctx.project_path.get_untracked());

    create_effect(move || {
        // Подписка на смену проекта: пути в очереди берутся из него.
        let _ = ctx.project_path.get();
        enqueue_dirty(ctx);
    });

    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("synthos-notes-autosave".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(250));
                let _w = lock(write_lock());
                let due: Vec<(Key, PendingSave)> = {
                    let mut map = lock(pending());
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
                let _ = write_batch(due);
            })
            .ok();
    });
}
