//! Watcher файловой системы для проекта редактора кода.
//!
//! Цель: обнаружить внешние изменения файлов (git pull, другой редактор) и
//! отразить их в UI.
//! - Если буфер чистый (не dirty) — тихо обновить `disk_contents` +
//!   `file_contents`, чтобы пользователь сразу увидел свежий контент.
//! - Если буфер dirty и контент с диска отличается от старого диск-снимка
//!   и от dirty-буфера — пометить `external_changes` (RwSignal), чтобы UI
//!   показал conflict-индикатор.
//! - Создания/удаления файлов в открытых папках дерева — invalidate
//!   `loaded_dirs` для родителя → следующее раскрытие подгрузит свежее
//!   содержимое (или сразу refresh, если это корень).
//!
//! Реализация: `notify::RecommendedWatcher` (платформо-зависимый backend —
//! inotify / FSEvents / ReadDirectoryChangesW). События приходят в
//! channel; поток-диспетчер дрейнит его, дебаунсит 200мс батчами и применяет.
//!
//! Подписки ставятся по каталогам (`NonRecursive`), а не одним
//! `RecursiveMode::Recursive` на корень: рекурсивный watch notify падает
//! ЦЕЛИКОМ на первом нечитаемом каталоге (например, root-овский rootfs в
//! `devices/*/build/`), и проект оставался совсем без обновлений. Обход
//! ([`watch_tree`]) пропускает нечитаемое, [`IGNORED_DIRS`] и gitignored
//! каталоги (`cache/` на 12k папок съедал бы лимит inotify); такие каталоги
//! получают подписку, только когда их раскрывают в дереве ([`watch_dir`]).
//! Новые каталоги подписываются диспетчером по событию создания.
//!
//! Watcher хранится в [`CodeSession::watcher`] (не `RwSignal`, а обычный
//! `Arc<Mutex<Option<FsWatcher>>>` — он не Reactive, держит RAII handle).
//! Drop сессии или смена `root_folder` через [`set_root_folder`] —
//! `_watcher` дроп'ается, события прекращаются.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, warn};

use super::fs_ops;
use super::state::{CodeSession, MAX_OPEN_FILE_BYTES};

/// Список директорий, события из которых watcher игнорирует — большие
/// генерируемые папки шумят (cargo build, npm install) и не несут UX-смысла.
///
/// Заметка про `.git`: целиком блокировать `.git/` нельзя — нужны события
/// о смене HEAD, index, refs/* (для триггера git-status refresh после
/// commit/checkout/pull). Поэтому `.git/` обрабатывается отдельным
/// предикатом [`is_event_ignored`] — там whitelist «интересных» путей.
const IGNORED_DIRS: &[&str] = &["target", "node_modules", ".cache", "dist", "build"];

/// Внутри `.git/` ловим только события, которые меняют видимое
/// git-состояние: HEAD/index/MERGE_HEAD/refs/*. Всё остальное (objects,
/// logs, hooks, lfs, modules) игнорируется — оно шумит без UX-смысла.
fn is_git_meta_event(rel_to_root: &Path) -> bool {
    // rel_to_root начинается с ".git" (либо ".git/...")
    let mut comps = rel_to_root.components();
    let first = comps.next();
    let is_dotgit = first
        .map(|c| c.as_os_str().to_string_lossy() == ".git")
        .unwrap_or(false);
    if !is_dotgit {
        return false;
    }
    let rest: PathBuf = comps.collect();
    let s = rest.to_string_lossy();
    if s.is_empty() {
        return false; // событие на самой папке `.git/` — игнор.
    }
    // Whitelist «интересных» путей.
    matches!(
        s.as_ref(),
        "HEAD"
            | "index"
            | "MERGE_HEAD"
            | "CHERRY_PICK_HEAD"
            | "REBASE_HEAD"
            | "ORIG_HEAD"
            | "packed-refs"
    ) || s.starts_with("refs/")
}

/// Должно ли это событие игнорироваться? `rel` — путь относительно корня
/// проекта.
///
/// Правила:
/// - Любой компонент пути в [`IGNORED_DIRS`] → игнор.
/// - `.git/...` → игнор, **кроме** whitelist'а из [`is_git_meta_event`].
fn is_event_ignored(rel: &Path) -> bool {
    let mut comps = rel.components();
    let first_str = match comps.next() {
        Some(c) => c.as_os_str().to_string_lossy().to_string(),
        None => return false,
    };
    if first_str == ".git" {
        return !is_git_meta_event(rel);
    }
    if IGNORED_DIRS.iter().any(|ig| first_str == *ig) {
        return true;
    }
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        IGNORED_DIRS.iter().any(|ig| s == *ig)
    })
}

/// Watcher одной сессии (один `root_folder`). Drop останавливает поток
/// notify и закрывает channel.
pub struct FsWatcher {
    /// RAII handle. Drop = stop watching. Диспетчер держит только `Weak`,
    /// иначе он сам продлевал бы жизнь watcher'а и канал не закрывался бы.
    watcher: Arc<Mutex<RecommendedWatcher>>,
    /// Корневой путь, чтобы фильтровать игнорируемые префиксы.
    root: PathBuf,
}

impl FsWatcher {
    /// Создать watcher на `root`. Синхронно подписывается только на сам
    /// корень (чтобы старт сессии не ждал обхода), остальное дерево
    /// подписывает поток-диспетчер перед входом в цикл событий.
    /// Все мутации сигналов делаются через `run_on_main_thread` (signal-runtime
    /// thread-local'ный, доступен только на main).
    ///
    /// NB: используем std::thread + std::mpsc вместо tokio::spawn —
    /// `CodeSession::new` вызывается ДО запуска tokio runtime
    /// (`build_code_editor_ctx` в `lib.rs`), и `tokio::spawn` упал бы с
    /// «no reactor running». std::thread не имеет такого ограничения.
    pub fn start(session: CodeSession, root: PathBuf) -> Result<Self, notify::Error> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })?;
        watcher.watch(&root, RecursiveMode::NonRecursive)?;
        let watcher = Arc::new(Mutex::new(watcher));

        let weak = Arc::downgrade(&watcher);
        let root_for_thread = root.clone();
        std::thread::Builder::new()
            .name("synthos-fs-watcher".to_string())
            .spawn(move || {
                let started = Instant::now();
                let n = watch_tree(&weak, &root_for_thread, &root_for_thread);
                watch_git_meta(&weak, &root_for_thread);
                debug!(
                    target: "code-editor.fs_watcher",
                    root = %root_for_thread.display(),
                    dirs = n,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "watches installed"
                );
                dispatch_loop(session, raw_rx, root_for_thread, weak)
            })
            .ok();

        Ok(Self { watcher, root })
    }

    /// Корень, на который установлен watcher (для UI-доков / диагностики).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Подписаться на один каталог (без рекурсии). Ошибка — только лог:
    /// нечитаемый каталог не должен ломать остальное наблюдение.
    pub fn watch_dir(&self, dir: &Path) {
        add_watches(&Arc::downgrade(&self.watcher), std::iter::once(dir.to_path_buf()));
    }
}

/// Поставить `NonRecursive`-подписку на каждый каталог из `dirs`. Ошибки
/// (EACCES, ENOENT после гонки, исчерпанный лимит inotify) логируются и
/// пропускаются. Возвращает число успешно подписанных каталогов.
fn add_watches(
    watcher: &Weak<Mutex<RecommendedWatcher>>,
    dirs: impl IntoIterator<Item = PathBuf>,
) -> usize {
    let Some(w) = watcher.upgrade() else { return 0 };
    let Ok(mut w) = w.lock() else { return 0 };
    let mut ok = 0usize;
    for dir in dirs {
        match w.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => ok += 1,
            Err(e) => debug!(
                target: "code-editor.fs_watcher",
                dir = %dir.display(),
                error = %e,
                "watch failed, skip"
            ),
        }
    }
    ok
}

/// Каталоги под `start` (включая сам `start`), которые стоит наблюдать:
/// без нечитаемых, без [`IGNORED_DIRS`], без `.git` и без gitignored.
/// `.gitignore` всех уровней от `root` вниз учитывается обходчиком `ignore`.
fn watchable_dirs(start: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(start)
        .hidden(false)
        .git_global(false)
        .follow_links(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".git" && !IGNORED_DIRS.iter().any(|ig| name == *ig)
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_dir()))
        .map(ignore::DirEntry::into_path)
        .collect()
}

/// Подписать `start` и всё наблюдаемое под ним. `start` сам по себе
/// проверяется на gitignore через обход родителя на глубину 1 — корень
/// обхода `ignore` не фильтрует (новый `cache/` иначе подписался бы целиком).
fn watch_tree(watcher: &Weak<Mutex<RecommendedWatcher>>, root: &Path, start: &Path) -> usize {
    if start != root {
        let Some(parent) = start.parent() else { return 0 };
        let listed = ignore::WalkBuilder::new(parent)
            .hidden(false)
            .git_global(false)
            .max_depth(Some(1))
            .build()
            .filter_map(Result::ok)
            .any(|e| e.path() == start);
        if !listed {
            return 0;
        }
    }
    add_watches(watcher, watchable_dirs(start))
}

/// `.git/` наблюдаем точечно: сам каталог (HEAD, index, MERGE_HEAD…) и
/// `refs/` рекурсивно. `objects/`, `logs/` шумят без UX-смысла.
fn watch_git_meta(watcher: &Weak<Mutex<RecommendedWatcher>>, root: &Path) {
    let git = root.join(".git");
    if !git.is_dir() {
        return;
    }
    let mut dirs = vec![git.clone()];
    dirs.extend(
        walk_plain_dirs(&git.join("refs")),
    );
    add_watches(watcher, dirs);
}

/// Все каталоги под `dir` без каких-либо фильтров (для `.git/refs`).
fn walk_plain_dirs(dir: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(dir)
        .standard_filters(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_dir()))
        .map(ignore::DirEntry::into_path)
        .collect()
}

/// Высокоуровневое событие, переведённое из notify::EventKind.
#[derive(Debug, Clone)]
enum FsEvent {
    /// Файл изменён на диске. Если это путь из `open_files` —
    /// перечитываем `disk_contents` и решаем conflict ли это.
    Modified(PathBuf),
    /// Файл/папка появились или были удалены в открытом каталоге —
    /// обновляем дерево.
    StructureChanged(PathBuf),
    /// Изменился git-meta файл (`.git/HEAD`, `.git/index`, `.git/refs/*`).
    /// Сам путь не нужен — мы только фиксируем факт «надо перечитать
    /// git-status»; апдейт этого signal'а делает воркер.
    GitMetaChanged,
    /// Появился каталог — на него (и его наблюдаемое содержимое) надо
    /// поставить подписки, без этого его файлы были бы невидимы watcher'у.
    DirAdded(PathBuf),
}

/// Возвращает 0..2 высокоуровневых события для одного notify-event.
///
/// Almost-atomic write (Edit/Write tool, vim swap-rename, `mv tmp orig`) на
/// inotify даёт `Modify(Name(From/To))` или пару `Create`/`Remove` без
/// `Modify(Data)` — содержимое целевого файла сменилось, но событие
/// семантически «структурное». Поэтому при `Modify(Name)` и `Create` мы
/// выдаём ОБА FsEvent: `StructureChanged(parent)` (для tree refresh) и
/// дополнительно `Modified(path)` если `path` указывает на существующий
/// файл (для перечитки буфера в [`apply_batch`]). Без этого открытая
/// вкладка остаётся со старым контентом при atomic-rename изменении.
fn translate(res: notify::Result<notify::Event>, root: &Path) -> Vec<FsEvent> {
    let ev = match res {
        Ok(e) => e,
        Err(e) => {
            warn!(target: "code-editor.fs_watcher", error = %e, "notify error");
            return Vec::new();
        }
    };

    let path = match ev.paths.into_iter().next() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let rel = match path.strip_prefix(root) {
        Ok(r) => r.to_path_buf(),
        Err(_) => return Vec::new(), // путь не внутри корня — пропускаем
    };

    if is_event_ignored(&rel) {
        return Vec::new();
    }

    // Если событие в `.git/...` — это git-meta change, а не file edit.
    let is_git_meta = rel
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy() == ".git")
        .unwrap_or(false);
    if is_git_meta {
        debug!(target: "code-editor.fs_watcher", rel = %rel.display(), "git-meta event");
        let mut out = vec![FsEvent::GitMetaChanged];
        // Новая ветка с «/» в имени создаёт каталог в refs/ — подписываем.
        if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_)))
            && path.is_dir()
        {
            out.push(FsEvent::DirAdded(path));
        }
        return out;
    }

    let mut out: Vec<FsEvent> = Vec::with_capacity(2);
    match ev.kind {
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Any) => {
            out.push(FsEvent::Modified(path.clone()));
        }
        EventKind::Modify(ModifyKind::Name(_)) | EventKind::Create(_) => {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or(path.clone());
            out.push(FsEvent::StructureChanged(dir));
            // Atomic-rename / write-then-rename: целевой файл существует с
            // новым контентом — открытый буфер должен перечитаться.
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => out.push(FsEvent::Modified(path.clone())),
                Ok(m) if m.is_dir() => out.push(FsEvent::DirAdded(path.clone())),
                _ => {}
            }
        }
        EventKind::Remove(_) => {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or(path.clone());
            out.push(FsEvent::StructureChanged(dir));
        }
        _ => {}
    }

    if !out.is_empty() {
        debug!(
            target: "code-editor.fs_watcher",
            path = %path.display(),
            events = ?out,
            "translated"
        );
    }
    out
}

/// Главный цикл (sync, в отдельном std::thread): дрейнит raw notify-события,
/// фильтрует/переводит в `FsEvent`, дебаунсит 200мс батчами и применяет к
/// сессии через `run_on_main_thread`.
fn dispatch_loop(
    session: CodeSession,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    root: PathBuf,
    watcher: Weak<Mutex<RecommendedWatcher>>,
) {
    let debounce = Duration::from_millis(200);
    loop {
        // Ждём первое событие. Disconnected = watcher дроп'нут — выходим.
        let raw = match rx.recv() {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut modified: HashSet<PathBuf> = HashSet::new();
        let mut structure: HashSet<PathBuf> = HashSet::new();
        let mut git_meta_dirty = false;
        let mut added: HashSet<PathBuf> = HashSet::new();
        for ev in translate(raw, &root) {
            accumulate(ev, &mut modified, &mut structure, &mut git_meta_dirty, &mut added);
        }

        // Дебаунс: ждём 200мс с recv_timeout. Любое новое событие
        // продлевает окно (мы не пересчитываем deadline — debounce
        // фиксированный с момента первого события).
        let deadline = Instant::now() + debounce;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                break;
            }
            match rx.recv_timeout(timeout) {
                Ok(raw) => {
                    for ev in translate(raw, &root) {
                        accumulate(
                            ev,
                            &mut modified,
                            &mut structure,
                            &mut git_meta_dirty,
                            &mut added,
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        // Подписки на новые каталоги ставим здесь, в фоне: обход может
        // быть длинным (распаковали архив). `.git/refs/*` — без фильтров.
        for dir in added {
            if dir.starts_with(root.join(".git")) {
                add_watches(&watcher, walk_plain_dirs(&dir));
            } else {
                watch_tree(&watcher, &root, &dir);
            }
        }

        if modified.is_empty() && structure.is_empty() && !git_meta_dirty {
            continue;
        }

        debug!(
            target: "code-editor.fs_watcher",
            modified = modified.len(),
            structure = structure.len(),
            git_meta = git_meta_dirty,
            "batch ready, dispatch to main"
        );

        // Применить на main thread — RwSignal'ы thread-local.
        syngui::async_runtime::run_on_main_thread(move || {
            apply_batch(session, modified, structure, git_meta_dirty);
        });
    }
}

fn accumulate(
    ev: FsEvent,
    modified: &mut HashSet<PathBuf>,
    structure: &mut HashSet<PathBuf>,
    git_meta_dirty: &mut bool,
    added: &mut HashSet<PathBuf>,
) {
    match ev {
        FsEvent::Modified(p) => {
            modified.insert(p);
        }
        FsEvent::StructureChanged(p) => {
            structure.insert(p);
        }
        FsEvent::GitMetaChanged => {
            *git_meta_dirty = true;
        }
        FsEvent::DirAdded(p) => {
            added.insert(p);
        }
    }
}

fn apply_batch(
    session: CodeSession,
    modified: HashSet<PathBuf>,
    structure: HashSet<PathBuf>,
    git_meta_dirty: bool,
) {
    // Любое изменение в workdir или git-meta триггерит refresh
    // git-status. Запрос через registry (request_git_refresh ничего
    // не делает, если воркер не зарегистрирован — например, root
    // не git-репо).
    let need_git_refresh =
        !modified.is_empty() || !structure.is_empty() || git_meta_dirty;
    if need_git_refresh {
        super::state::request_git_refresh(session);
    }
    // Modified: обновить disk_contents для каждого открытого файла.
    if !modified.is_empty() {
        let open: HashSet<PathBuf> = session.open_files.get_untracked().into_iter().collect();
        for path in &modified {
            if !open.contains(path) {
                continue;
            }
            // Защита от больших файлов (если внешне дописали 50 MB —
            // не блокируем UI). Применяем тот же гард, что в open_file.
            match std::fs::metadata(path) {
                Ok(meta) if meta.len() > MAX_OPEN_FILE_BYTES => {
                    warn!(
                        target: "code-editor.fs_watcher",
                        path = %path.display(),
                        size = meta.len(),
                        "file too large for auto-reload"
                    );
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(target: "code-editor.fs_watcher", path = %path.display(), error = %e, "metadata failed");
                    continue;
                }
            }
            let new_disk = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    warn!(target: "code-editor.fs_watcher", path = %path.display(), error = %e, "read failed");
                    continue;
                }
            };

            let buf_now = session
                .file_contents
                .get_untracked()
                .get(path)
                .cloned()
                .unwrap_or_default();
            let disk_old = session
                .disk_contents
                .get_untracked()
                .get(path)
                .cloned()
                .unwrap_or_default();

            // Случай A: буфер чистый (== старого disk). Тихо обновляем
            // оба значения — пользователь сразу видит свежий контент через
            // EditorCommand::Reload (effect в editor_pane.rs подписан на
            // `file_contents` и пушит Reload в `command_signal`, не
            // пересоздавая виджет — курсор/скролл сохраняются).
            if buf_now == disk_old {
                session.file_contents.update(|m| {
                    m.insert(path.clone(), new_disk.clone());
                });
                session.disk_contents.update(|m| {
                    m.insert(path.clone(), new_disk);
                });
                continue;
            }

            // Случай B: буфер dirty И диск изменился — конфликт. Сначала
            // страхуемся черновиком (несохранённый буфер на диск), затем
            // обновляем снимок диска и ставим путь в очередь конфликтов —
            // UI покажет диалог «оставить моё / перезагрузить / diff».
            warn!(
                target: "code-editor.fs_watcher",
                path = %path.display(),
                "внешнее изменение конфликтует с dirty-буфером"
            );
            super::drafts::save_draft(path, &buf_now);
            session.disk_contents.update(|m| {
                m.insert(path.clone(), new_disk);
            });
            session.conflicts.update(|v| {
                if !v.contains(path) {
                    v.push(path.clone());
                }
            });
        }
    }

    // Structure: перечитать затронутые ЗАГРУЖЕННЫЕ каталоги и отразить
    // изменения в дереве. Работаем на клоне `tree_nodes` и пишем сигнал
    // ТОЛЬКО если что-то реально изменилось — иначе `TreeView`
    // пересоздавался бы на каждое событие, сбрасывая скролл и выделение.
    // root читаем ОДИН раз ДО мутаций (клон + set вместо update-замыкания
    // снимает риск "RefCell already mutably borrowed", signal.rs:196).
    let had_structure = !structure.is_empty();
    if had_structure {
        let loaded = session.loaded_dirs.get_untracked();
        let root_opt = session.root_folder.get_untracked();
        let mut current = session.tree_nodes.get_untracked();
        let mut any_changed = false;
        for dir in structure {
            if !loaded.contains(&dir) {
                debug!(
                    target: "code-editor.fs_watcher",
                    dir = %dir.display(),
                    "structure event for non-loaded dir, skip"
                );
                continue;
            }
            // Fail-closed: транзиентную ошибку чтения (EMFILE, гонка с
            // git checkout) НЕ принимаем за истину — пропускаем каталог,
            // дерево остаётся как есть. Реальное удаление подхватит
            // reconcile_loaded_dirs по ENOENT.
            let new_kids = match fs_ops::read_dir_to_nodes_checked(&dir) {
                Ok(k) => k,
                Err(e) => {
                    debug!(
                        target: "code-editor.fs_watcher",
                        dir = %dir.display(),
                        error = %e,
                        "read failed, skip (tree untouched)"
                    );
                    continue;
                }
            };
            let id = fs_ops::id_for(&dir);
            let is_root = root_opt.as_ref().map(|r| dir == *r).unwrap_or(false);
            let changed = if is_root {
                fs_ops::merge_level_preserve_state(&mut current, new_kids)
            } else {
                match fs_ops::refresh_children_preserve_state(
                    &mut current,
                    &id,
                    &mut Some(new_kids),
                ) {
                    Some(c) => c,
                    None => {
                        warn!(
                            target: "code-editor.fs_watcher",
                            dir = %dir.display(),
                            id,
                            "structure event: parent node not found in tree"
                        );
                        false
                    }
                }
            };
            any_changed |= changed;
        }
        // Vec<TreeNode> не PartialEq → set недоступен, пишем через update
        // (он уведомляет подписчиков всегда, поэтому зовём лишь при changed).
        if any_changed {
            session.tree_nodes.update(move |nodes| *nodes = current);
        }
    }

    // Safety net: при любом structure-событии реконсилируем tree против
    // диска. Компенсирует пропуски inotify (overflow на массовых
    // операциях: git checkout, cargo build, переименование крейтов) —
    // без этого stale-папки жили бы до ручного refresh. Тоже
    // set-if-changed: на no-op скролл/раскрытость не трогаются.
    if had_structure {
        reconcile_loaded_dirs(session);
    }
}

/// Сверить `loaded_dirs` с диском: отсутствующие каталоги удалить из
/// дерева (вместе с поддеревом) и из `loaded_dirs`; существующие —
/// перечитать и смержить с текущим состоянием. Восстанавливает
/// согласованность после пропущенных notify-событий.
///
/// Дешёвая операция: read_dir на каждый раскрытый каталог. Запускается
/// после каждого batch'а со структурным событием.
fn reconcile_loaded_dirs(session: CodeSession) {
    use std::io::ErrorKind;
    let loaded_snapshot: Vec<PathBuf> =
        session.loaded_dirs.get_untracked().into_iter().collect();
    let root_opt = session.root_folder.get_untracked();

    let mut missing: Vec<PathBuf> = Vec::new();
    let mut reads: Vec<(PathBuf, Vec<syngui::widgets::TreeNode>)> = Vec::new();
    for dir in loaded_snapshot {
        match fs_ops::read_dir_to_nodes_checked(&dir) {
            Ok(kids) => reads.push((dir, kids)),
            // Только ПОДТВЕРЖДЁННОЕ отсутствие (ENOENT) удаляет узлы.
            Err(e) if e.kind() == ErrorKind::NotFound => missing.push(dir),
            // Транзиентная ошибка (EMFILE, гонка чтения) — НЕ удаляем:
            // иначе временный сбой стёр бы живую ветку из дерева.
            Err(e) => {
                debug!(
                    target: "code-editor.fs_watcher",
                    dir = %dir.display(),
                    error = %e,
                    "reconcile: transient read error, keeping subtree"
                );
            }
        }
    }

    if missing.is_empty() && reads.is_empty() {
        return;
    }

    if !missing.is_empty() {
        warn!(
            target: "code-editor.fs_watcher",
            count = missing.len(),
            paths = ?missing,
            "reconcile: dropping stale loaded dirs (deleted on disk)"
        );
    }

    // Работаем на клоне; сигнал пишем только при реальном изменении —
    // на no-op скролл/раскрытость дерева сохраняются.
    let mut current = session.tree_nodes.get_untracked();
    let mut changed = false;
    // 1) Удалить подтверждённо пропавшие dirs вместе с поддеревом.
    for dir in &missing {
        if fs_ops::remove_node(&mut current, &fs_ops::id_for(dir)) {
            changed = true;
        }
    }
    // 2) Перемержить существующие dirs, СОХРАНЯЯ раскрытость (root
    // отдельно — он сам корень дерева, не его children).
    for (dir, kids) in reads {
        let id = fs_ops::id_for(&dir);
        let is_root = root_opt.as_ref().map(|r| &dir == r).unwrap_or(false);
        let c = if is_root {
            fs_ops::merge_level_preserve_state(&mut current, kids)
        } else {
            fs_ops::refresh_children_preserve_state(&mut current, &id, &mut Some(kids))
                .unwrap_or(false)
        };
        changed |= c;
    }
    // Vec<TreeNode> не PartialEq → только update (зовём при реальном changed).
    if changed {
        session.tree_nodes.update(move |nodes| *nodes = current);
    }

    if !missing.is_empty() {
        session.loaded_dirs.update(|s| {
            s.retain(|p| !missing.iter().any(|m| p == m || p.starts_with(m)));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Нечитаемый каталог (root-овский rootfs) не должен лишать проект
    /// наблюдения: раньше рекурсивный watch падал целиком с EACCES.
    #[test]
    fn unreadable_and_ignored_dirs_do_not_break_watching() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::write(root.join(".gitignore"), "/cache/\n").unwrap();
        // .gitignore учитывается обходчиком только внутри git-репо.
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        for d in ["docs", "cache/deep", "build/x", "locked/inner", "src/a"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000))
            .unwrap();

        let dirs = watchable_dirs(&root);
        let has = |d: &str| dirs.contains(&root.join(d));
        assert!(has("docs") && has("src/a") && dirs.contains(&root));
        assert!(!has("cache") && !has("cache/deep"), "gitignored: {dirs:?}");
        assert!(!has("build/x") && !has(".git"), "ignored: {dirs:?}");

        let (tx, rx) = mpsc::channel();
        let w = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .unwrap();
        let w = Arc::new(Mutex::new(w));
        let weak = Arc::downgrade(&w);
        // `locked` виден в листинге родителя, но подписка на него (если мы
        // не root) падает с EACCES — остальные каталоги подписываются.
        assert!(has("locked"));
        assert!(add_watches(&weak, dirs.clone()) >= dirs.len() - 1);

        // Новый gitignored каталог по событию не подписывается целиком.
        assert_eq!(watch_tree(&weak, &root, &root.join("cache")), 0);
        assert!(watch_tree(&weak, &root, &root.join("src")) >= 2);

        std::fs::write(root.join("docs/22-camera.md"), "x").unwrap();
        let seen = std::iter::from_fn(|| rx.recv_timeout(Duration::from_millis(500)).ok())
            .filter_map(Result::ok)
            .flat_map(|e| e.paths)
            .collect::<Vec<_>>();
        assert!(seen.iter().any(|p| p.ends_with("docs/22-camera.md")), "{seen:?}");

        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
}
