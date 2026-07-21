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
//! channel; tokio::task дрейнит его, дебаунсит 200мс батчами и применяет.
//!
//! Watcher хранится в [`CodeSession::watcher`] (не `RwSignal`, а обычный
//! `Arc<Mutex<Option<FsWatcher>>>` — он не Reactive, держит RAII handle).
//! Drop сессии или смена `root_folder` через [`set_root_folder`] —
//! `_watcher` дроп'ается, события прекращаются.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
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
    /// RAII handle. Drop = stop watching.
    _watcher: RecommendedWatcher,
    /// Корневой путь, чтобы фильтровать игнорируемые префиксы.
    root: PathBuf,
}

impl FsWatcher {
    /// Создать watcher на `root` рекурсивно. Запускает std-поток-диспетчер,
    /// который преобразует raw notify-события в действия над `session`.
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
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let root_for_thread = root.clone();
        std::thread::Builder::new()
            .name("synthos-fs-watcher".to_string())
            .spawn(move || dispatch_loop(session, raw_rx, root_for_thread))
            .ok();

        Ok(Self {
            _watcher: watcher,
            root,
        })
    }

    /// Корень, на который установлен watcher (для UI-доков / диагностики).
    pub fn root(&self) -> &Path {
        &self.root
    }
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
        return vec![FsEvent::GitMetaChanged];
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
            if std::fs::metadata(&path)
                .map(|m| m.is_file())
                .unwrap_or(false)
            {
                out.push(FsEvent::Modified(path.clone()));
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
        for ev in translate(raw, &root) {
            accumulate(ev, &mut modified, &mut structure, &mut git_meta_dirty);
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
                        accumulate(ev, &mut modified, &mut structure, &mut git_meta_dirty);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
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
