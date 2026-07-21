//! Git-status декорации для file tree.
//!
//! Поставляет три блока:
//! - [`GitStatus`] + [`GitStatusMap`] — данные о состоянии файлов/папок,
//!   собранные через `gix::Repository::status()`.
//! - [`apply_to_nodes`] — pure-функция, навешивающая
//!   [`syngui::widgets::TreeNodeDecoration`] на каждый узел дерева и
//!   инжектящая ghost-ноды для удалённых файлов (Zed-style).
//! - [`GitStatusWorker`] — фоновой std::thread с mpsc-debounce'ом 500мс,
//!   зеркалирующий паттерн `fs_watcher::dispatch_loop`. На каждое
//!   `request_refresh` рекомпьютит [`GitStatusMap`] и через
//!   `syngui::async_runtime::run_on_main_thread` обновляет
//!   [`super::state::CodeSession::git_status`].
//!
//! Никаких паник: любая git-ошибка → `eprintln!` + пустая карта (репо
//! считается «не git-projектом», UI без декораций).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use syngui::core::Color;
use syngui::widgets::{TreeNode, TreeNodeDecoration};
use tracing::{debug, warn};

use super::file_icons;
use super::fs_ops::PLACEHOLDER_PREFIX;
use super::state::CodeSession;

/// ID-префикс для ghost-нод (удалённые файлы, отображаемые в дереве).
/// `file_tree::handle_select` распознаёт префикс и не пытается открыть
/// несуществующий файл — вместо этого показывает notice.
pub const GHOST_PREFIX: &str = "__ghost_deleted__:";

// ─────────────────────────────────────────────────────────────────────────
// GitStatus + GitStatusMap
// ─────────────────────────────────────────────────────────────────────────

/// Категория git-изменения, которую мы визуализируем в TreeView.
///
/// Порядок вариантов соответствует порядку приоритета rollup'а на папки:
/// внутри папки победит самый «громкий» статус среди дочерних.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GitStatus {
    /// Merge-conflict — самый громкий статус.
    Conflict,
    /// Файл изменён в worktree относительно index/HEAD.
    Modified,
    /// Untracked файл (не добавлен в индекс).
    New,
    /// Файл удалён в worktree, но всё ещё в индексе.
    Deleted,
}

impl GitStatus {
    /// Числовой приоритет (выше = более важный).
    /// Используется при folder rollup'е: папка получает max-priority
    /// статус среди своих потомков.
    pub fn priority(self) -> u8 {
        match self {
            GitStatus::Conflict => 4,
            GitStatus::Modified => 3,
            GitStatus::New => 2,
            GitStatus::Deleted => 1,
        }
    }

    /// Слить два статуса, выбрав более приоритетный.
    /// Симметричная операция: `merge(a, b) == merge(b, a)`.
    pub fn merge(a: Self, b: Self) -> Self {
        if a.priority() >= b.priority() { a } else { b }
    }
}

/// Карта git-состояния проекта. `workdir == None` означает, что папка не
/// является git-репозиторием — UI просто пропускает декорации.
///
/// `PartialEq` нужен потому, что `RwSignal<Arc<GitStatusMap>>::set`
/// сравнивает значения и пропускает уведомление подписчикам, если карта
/// не изменилась — экономия rebuild'ов TreeView. Сравнение по содержимому
/// (HashMap, PathBuf, enum) выполняется автоматически через derive.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GitStatusMap {
    pub workdir: Option<PathBuf>,
    /// Per-file статус (абсолютные пути).
    pub files: HashMap<PathBuf, GitStatus>,
    /// Precomputed rollup на папки (абсолютные пути), workdir исключён.
    pub folders: HashMap<PathBuf, GitStatus>,
}

impl GitStatusMap {
    /// Пустая карта (репо отсутствует или ошибка чтения).
    pub fn empty() -> Self { Self::default() }

    /// Быстрая проверка: есть ли в карте хоть одна запись.
    /// Используется UI чтобы не тратить такты на rebuild дерева, если
    /// репо не git-репозиторий.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.folders.is_empty()
    }

    /// Найти статус для пути: сначала ищем как файл, потом как папку.
    pub fn lookup(&self, path: &Path) -> Option<GitStatus> {
        self.files
            .get(path)
            .copied()
            .or_else(|| self.folders.get(path).copied())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Палитра
// ─────────────────────────────────────────────────────────────────────────

/// Палитра git-decorations. Цвета зеркалят `--git-color-*` токены в
/// `styles/components/code_editor.mss` — единая Zed/VSCode-style палитра,
/// theme-agnostic (в светлых и тёмных темах одинаково читается).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GitPalette {
    pub modified: Color,
    pub new: Color,
    pub deleted: Color,
    pub conflict: Color,
}

impl Default for GitPalette {
    fn default() -> Self {
        // Цвета совпадают с `--git-color-*` в code_editor.mss и опираются
        // на существующую `--file-color-*` палитру (Material-style):
        //   modified  → --file-color-code     (#FFB454, тёплый жёлтый)
        //   new       → --file-color-shell    (#6BD36B, зелёный)
        //   deleted   → --file-color-pdf      (#EE5E48, красный)
        //   conflict  → --file-color-toml     (#B07FFF, фиолетовый)
        Self {
            modified: Color::from_hex("#FFB454"),
            new: Color::from_hex("#6BD36B"),
            deleted: Color::from_hex("#EE5E48"),
            conflict: Color::from_hex("#B07FFF"),
        }
    }
}

impl GitPalette {
    pub fn color_for(&self, status: GitStatus) -> Color {
        match status {
            GitStatus::Modified => self.modified,
            GitStatus::New => self.new,
            GitStatus::Deleted => self.deleted,
            GitStatus::Conflict => self.conflict,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// compute(): запрос статусов через gix
// ─────────────────────────────────────────────────────────────────────────

/// Собрать [`GitStatusMap`] для папки `root`. Все ошибки → лог + пустая
/// карта. Никогда не паникует.
///
/// Эта функция запускается в фоновом потоке (см. [`GitStatusWorker`]),
/// потому что на больших репо `Repository::status` может занимать
/// сотни миллисекунд / секунды.
pub fn compute(root: &Path) -> GitStatusMap {
    let started = Instant::now();
    let repo = match gix::discover(root) {
        Ok(r) => r,
        Err(e) => {
            // Не git-репо — нормальный сценарий, не error-уровень.
            debug!(
                target: "code-editor.git_status",
                root = %root.display(),
                error = %e,
                "не git-репозиторий"
            );
            return GitStatusMap::empty();
        }
    };

    let workdir_raw = match repo.workdir() {
        Some(p) => p.to_path_buf(),
        None => {
            warn!(target: "code-editor.git_status", root = %root.display(), "bare-репо, пропускаем");
            return GitStatusMap::empty();
        }
    };
    // Канонизируем workdir, чтобы абсолютные пути файлов в карте
    // совпадали с теми, что приходят из `read_dir` (которое тоже
    // следует канонической форме).
    let workdir = workdir_raw.canonicalize().unwrap_or(workdir_raw);

    let mut files: HashMap<PathBuf, GitStatus> = HashMap::new();
    if let Err(e) = collect_statuses(&repo, &workdir, &mut files) {
        warn!(target: "code-editor.git_status", error = %e, "collect failed");
        return GitStatusMap::empty();
    }

    let folders = compute_folder_rollup(&files, &workdir);
    debug!(
        target: "code-editor.git_status",
        workdir = %workdir.display(),
        files = files.len(),
        folders = folders.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "compute done"
    );
    GitStatusMap {
        workdir: Some(workdir),
        files,
        folders,
    }
}

/// Низкоуровневый сборщик: вызывает `Repository::status().into_index_worktree_iter`
/// и переводит варианты gix в [`GitStatus`].
///
/// Использует `into_index_worktree_iter` (не общий `into_iter`), потому
/// что для file tree нам интересны только разногласия index↔worktree;
/// staged-vs-HEAD (TreeIndex) визуально не разделяем — все «изменено»
/// одним цветом. Это и быстрее (одна walk-сессия вместо трёх).
fn collect_statuses(
    repo: &gix::Repository,
    workdir: &Path,
    out: &mut HashMap<PathBuf, GitStatus>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use gix::bstr::{BStr, ByteSlice};
    use gix::status::index_worktree::Item as IwItem;
    // EntryStatus и Change живут в plumbing-крейте gix-status —
    // gix реэкспортит его как `gix::status::plumbing`.
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

    let platform = repo
        .status(gix::progress::Discard)?
        // По умолчанию untracked-файлы gix не возвращает.
        // Включаем — нам нужны «зелёные» (new) узлы.
        .untracked_files(gix::status::UntrackedFiles::Collapsed);

    // Пустой набор паттернов = «все файлы».
    let iter = platform.into_index_worktree_iter(Vec::<gix::bstr::BString>::new())?;

    for item in iter {
        let iw = match item {
            Ok(i) => i,
            Err(e) => {
                warn!(target: "code-editor.git_status", error = %e, "item error");
                continue;
            }
        };

        let (rel, status): (&BStr, GitStatus) = match &iw {
            IwItem::Modification {
                rela_path, status, ..
            } => {
                let s = match status {
                    EntryStatus::Conflict { .. } => GitStatus::Conflict,
                    EntryStatus::Change(change) => match change {
                        Change::Removed => GitStatus::Deleted,
                        Change::Type { .. } => GitStatus::Modified,
                        Change::Modification { .. } => GitStatus::Modified,
                        Change::SubmoduleModification(_) => GitStatus::Modified,
                    },
                    EntryStatus::IntentToAdd => GitStatus::New,
                    EntryStatus::NeedsUpdate(_) => continue,
                };
                (rela_path.as_bstr(), s)
            }
            IwItem::DirectoryContents { entry, .. } => {
                use gix::dir::entry::Status;
                match entry.status {
                    Status::Untracked => (entry.rela_path.as_bstr(), GitStatus::New),
                    // Pruned/Ignored/Tracked — не показываем (Tracked
                    // = чистый файл, Pruned/Ignored — игнорируем).
                    _ => continue,
                }
            }
            IwItem::Rewrite { .. } => {
                // Rename: gix без `rewrites` опции по умолчанию не
                // репортит rename'ы как Rewrite. Если включено — нам
                // фактически придёт delete+add отдельными элементами,
                // обработать обе стороны проще, чем парсить Rewrite.
                continue;
            }
        };

        let rel_path = match rel.to_path() {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                warn!(target: "code-editor.git_status", path = %rel, error = %e, "bad utf-8 path");
                continue;
            }
        };
        let abs = workdir.join(&rel_path);
        out.insert(abs, status);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Folder rollup
// ─────────────────────────────────────────────────────────────────────────

/// Собрать precomputed map «папка → max-priority статус потомков».
/// Сам `workdir` исключён (корень не подсвечиваем).
pub fn compute_folder_rollup(
    files: &HashMap<PathBuf, GitStatus>,
    workdir: &Path,
) -> HashMap<PathBuf, GitStatus> {
    let mut out: HashMap<PathBuf, GitStatus> = HashMap::new();
    for (file, &status) in files {
        let mut p = file.parent();
        while let Some(dir) = p {
            if !dir.starts_with(workdir) || dir == workdir {
                break;
            }
            out.entry(dir.to_path_buf())
                .and_modify(|cur| *cur = GitStatus::merge(*cur, status))
                .or_insert(status);
            p = dir.parent();
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// apply_to_nodes — навешиваем декорации + injects ghost'ы
// ─────────────────────────────────────────────────────────────────────────

/// Pure-функция: проходит по дереву и навешивает [`TreeNodeDecoration`]
/// на каждый узел в соответствии с git-статусом. Для развёрнутых папок
/// дополнительно инжектит ghost-ноды для удалённых файлов в этой
/// папке.
pub fn apply_to_nodes(
    nodes: Vec<TreeNode>,
    git: &GitStatusMap,
    palette: &GitPalette,
) -> Vec<TreeNode> {
    if git.is_empty() {
        return nodes;
    }
    let mut out = nodes;
    apply_recursive(&mut out, git, palette);
    out
}

fn apply_recursive(nodes: &mut Vec<TreeNode>, git: &GitStatusMap, palette: &GitPalette) {
    for node in nodes.iter_mut() {
        // Placeholder (свёрнутая папка с sentinel-leaf'ом) — пропускаем,
        // у него фиктивный id, нет смысла декорировать.
        if node.id.starts_with(PLACEHOLDER_PREFIX) {
            continue;
        }
        // Ghost-ноды могут попасть в `apply_recursive` при повторном
        // вызове — пропускаем, у них уже есть decoration.
        if node.id.starts_with(GHOST_PREFIX) {
            continue;
        }

        let path = PathBuf::from(&node.id);
        let has_children = !node.children.is_empty();

        if has_children {
            // Папка: только badge, label/icon не подкрашиваем.
            if let Some(st) = git.folders.get(&path).copied() {
                node.decoration = Some(TreeNodeDecoration {
                    label_color: None,
                    icon_color: None,
                    badge_color: Some(palette.color_for(st)),
                    strikethrough: false,
                });
            }
            apply_recursive(&mut node.children, git, palette);
            inject_ghost_deleted(&path, &mut node.children, git, palette);
        } else {
            // Файл: подкрашиваем label + icon целиком, strikethrough
            // для Deleted (это нужно для редкого кейса: файл всё ещё
            // на диске, но git считает его Deleted — например,
            // `git rm --cached` с восстановлением на диск).
            if let Some(st) = git.files.get(&path).copied() {
                let c = palette.color_for(st);
                node.decoration = Some(TreeNodeDecoration {
                    label_color: Some(c),
                    icon_color: Some(c),
                    badge_color: None,
                    strikethrough: st == GitStatus::Deleted,
                });
            }
        }
    }
}

/// Добавить ghost-ноды в `children` для всех `Deleted`-файлов, чьи
/// родители — `dir`, и которых нет среди `children` (на диске). Сортировка
/// alpha по label.
fn inject_ghost_deleted(
    dir: &Path,
    children: &mut Vec<TreeNode>,
    git: &GitStatusMap,
    palette: &GitPalette,
) {
    use std::collections::HashSet;
    let already: HashSet<&str> = children.iter().map(|n| n.id.as_str()).collect();
    let mut ghosts: Vec<TreeNode> = git
        .files
        .iter()
        .filter(|(p, st)| {
            **st == GitStatus::Deleted
                && p.parent() == Some(dir)
                && !already.contains(format!("{}", p.display()).as_str())
        })
        .map(|(p, _)| make_ghost_node(p, palette))
        .collect();
    if ghosts.is_empty() {
        return;
    }
    ghosts.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    children.extend(ghosts);
}

fn make_ghost_node(path: &Path, palette: &GitPalette) -> TreeNode {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let id = format!("{GHOST_PREFIX}{}", path.display());
    let icon = file_icons::icon_for_path(path, false, false);
    let red = palette.color_for(GitStatus::Deleted);
    TreeNode::leaf(id, name)
        .icon(icon)
        .decoration(TreeNodeDecoration {
            label_color: Some(red),
            icon_color: Some(red),
            badge_color: None,
            strikethrough: true,
        })
}

// ─────────────────────────────────────────────────────────────────────────
// Background worker
// ─────────────────────────────────────────────────────────────────────────

/// Фоновой воркер, который по `request_refresh` рекомпьютит
/// [`GitStatusMap`] и пишет его в `session.git_status` через
/// `syngui::async_runtime::run_on_main_thread`.
///
/// Mirrors `fs_watcher::dispatch_loop`: std::thread + mpsc + 500мс
/// debounce. Не использует tokio, потому что воркер создаётся в
/// `CodeSession::new` ДО старта tokio runtime.
pub struct GitStatusWorker {
    tx: mpsc::Sender<()>,
}

impl GitStatusWorker {
    /// Запустить воркер для папки `root`. Возвращает handle, через
    /// который можно слать `request_refresh()`. Drop handle = поток
    /// ловит Disconnected и завершается.
    pub fn start(session: CodeSession, root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<()>();
        let _ = thread::Builder::new()
            .name("synthos-git-status".to_string())
            .spawn(move || dispatch_loop(session, rx, root));
        Self { tx }
    }

    /// Запросить refresh. Безопасно вызывать с любого потока. Если канал
    /// отвалился (worker дроп'нут), просто молча игнорируем.
    pub fn request_refresh(&self) {
        let _ = self.tx.send(());
    }
}

fn dispatch_loop(session: CodeSession, rx: mpsc::Receiver<()>, root: PathBuf) {
    const DEBOUNCE: Duration = Duration::from_millis(500);
    loop {
        // Ждём первый запрос.
        if rx.recv().is_err() {
            return;
        }
        let mut coalesced = 1u32;
        // Drain последующих запросов в окне DEBOUNCE: если за 500мс
        // прилетело 100 событий (например, рекурсивный rm -rf), мы
        // сделаем ровно один git-запрос вместо 100.
        let deadline = Instant::now() + DEBOUNCE;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                break;
            }
            match rx.recv_timeout(timeout) {
                Ok(_) => {
                    coalesced += 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        debug!(
            target: "code-editor.git_status",
            session_id = session.id,
            root = %root.display(),
            coalesced,
            "refresh start"
        );
        let map = compute(&root);
        let arc = Arc::new(map);
        // RwSignal'ы thread-local — мутируем строго на main.
        let session_id = session.id;
        syngui::async_runtime::run_on_main_thread(move || {
            session.git_status.set(arc);
            debug!(target: "code-editor.git_status", session_id, "git_status applied to main");
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn priority_orders_correctly() {
        assert!(GitStatus::Conflict.priority() > GitStatus::Modified.priority());
        assert!(GitStatus::Modified.priority() > GitStatus::New.priority());
        assert!(GitStatus::New.priority() > GitStatus::Deleted.priority());
    }

    #[test]
    fn merge_picks_higher_priority_and_is_commutative() {
        let pairs = [
            (GitStatus::Modified, GitStatus::New, GitStatus::Modified),
            (GitStatus::Conflict, GitStatus::Deleted, GitStatus::Conflict),
            (GitStatus::New, GitStatus::Deleted, GitStatus::New),
            (GitStatus::Modified, GitStatus::Conflict, GitStatus::Conflict),
        ];
        for (a, b, expected) in pairs {
            assert_eq!(GitStatus::merge(a, b), expected, "merge({a:?}, {b:?})");
            assert_eq!(GitStatus::merge(b, a), expected, "merge({b:?}, {a:?})");
        }
    }

    #[test]
    fn folder_rollup_walks_all_ancestors() {
        let workdir = p("/repo");
        let mut files = HashMap::new();
        files.insert(p("/repo/src/foo/bar.rs"), GitStatus::Modified);
        files.insert(p("/repo/docs/intro.md"), GitStatus::New);

        let rollup = compute_folder_rollup(&files, &workdir);

        // /repo/src и /repo/src/foo должны получить Modified.
        assert_eq!(rollup.get(&p("/repo/src")).copied(), Some(GitStatus::Modified));
        assert_eq!(
            rollup.get(&p("/repo/src/foo")).copied(),
            Some(GitStatus::Modified)
        );
        // /repo/docs — New.
        assert_eq!(rollup.get(&p("/repo/docs")).copied(), Some(GitStatus::New));
        // Сам workdir исключён.
        assert!(rollup.get(&p("/repo")).is_none());
    }

    #[test]
    fn folder_rollup_picks_max_when_mixed_children() {
        let workdir = p("/repo");
        let mut files = HashMap::new();
        files.insert(p("/repo/src/a.rs"), GitStatus::New);
        files.insert(p("/repo/src/b.rs"), GitStatus::Modified);
        files.insert(p("/repo/src/c.rs"), GitStatus::Deleted);

        let rollup = compute_folder_rollup(&files, &workdir);
        assert_eq!(rollup.get(&p("/repo/src")).copied(), Some(GitStatus::Modified));
    }

    #[test]
    fn folder_rollup_isolates_siblings() {
        let workdir = p("/repo");
        let mut files = HashMap::new();
        files.insert(p("/repo/a/x.rs"), GitStatus::Conflict);
        files.insert(p("/repo/b/y.rs"), GitStatus::New);

        let rollup = compute_folder_rollup(&files, &workdir);
        assert_eq!(rollup.get(&p("/repo/a")).copied(), Some(GitStatus::Conflict));
        assert_eq!(rollup.get(&p("/repo/b")).copied(), Some(GitStatus::New));
    }

    #[test]
    fn apply_to_nodes_decorates_files_with_label_and_icon() {
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        git.files
            .insert(p("/repo/src/a.rs"), GitStatus::Modified);

        let nodes = vec![
            TreeNode::leaf("/repo/src/a.rs", "a.rs"),
        ];
        let out = apply_to_nodes(nodes, &git, &palette);
        let deco = out[0].decoration.as_ref().unwrap();
        assert_eq!(deco.label_color, Some(palette.modified));
        assert_eq!(deco.icon_color, Some(palette.modified));
        assert!(!deco.strikethrough);
    }

    #[test]
    fn apply_to_nodes_strikethrough_for_deleted_file_still_on_disk() {
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        git.files
            .insert(p("/repo/old.txt"), GitStatus::Deleted);

        let nodes = vec![TreeNode::leaf("/repo/old.txt", "old.txt")];
        let out = apply_to_nodes(nodes, &git, &palette);
        let deco = out[0].decoration.as_ref().unwrap();
        assert!(deco.strikethrough);
        assert_eq!(deco.label_color, Some(palette.deleted));
    }

    #[test]
    fn apply_to_nodes_folder_only_gets_badge() {
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        git.folders.insert(p("/repo/src"), GitStatus::Modified);

        // Папка с одним child'ом, чтобы has_children == true.
        let nodes = vec![TreeNode::branch(
            "/repo/src",
            "src",
            vec![TreeNode::leaf("/repo/src/foo.rs", "foo.rs")],
        )];
        let out = apply_to_nodes(nodes, &git, &palette);
        let deco = out[0].decoration.as_ref().unwrap();
        assert_eq!(deco.badge_color, Some(palette.modified));
        assert_eq!(deco.label_color, None, "у папки label не подкрашиваем");
        assert_eq!(deco.icon_color, None);
        assert!(!deco.strikethrough);
    }

    #[test]
    fn apply_to_nodes_skips_placeholder_ids() {
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        git.files
            .insert(p("/repo/src/a.rs"), GitStatus::Modified);

        let placeholder_id = format!("{PLACEHOLDER_PREFIX}/repo/src/a.rs");
        let nodes = vec![TreeNode::leaf(placeholder_id, "")];
        let out = apply_to_nodes(nodes, &git, &palette);
        assert!(out[0].decoration.is_none(), "placeholder не декорируется");
    }

    #[test]
    fn inject_ghost_only_immediate_children_alphabetical() {
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        // Два удалённых в /repo/src и один в /repo/docs (не должен попасть).
        git.files.insert(p("/repo/src/zz.rs"), GitStatus::Deleted);
        git.files.insert(p("/repo/src/aa.rs"), GitStatus::Deleted);
        git.files.insert(p("/repo/docs/x.md"), GitStatus::Deleted);
        // И один глубоко вложенный — не должен попасть как непосредственный.
        git.files
            .insert(p("/repo/src/sub/deep.rs"), GitStatus::Deleted);

        // Папка /repo/src уже expanded и имеет один живой файл.
        let nodes = vec![TreeNode::branch(
            "/repo/src",
            "src",
            vec![TreeNode::leaf("/repo/src/alive.rs", "alive.rs")],
        )];
        let out = apply_to_nodes(nodes, &git, &palette);
        let kids = &out[0].children;
        // alive + 2 ghost'а (zz, aa).
        assert_eq!(kids.len(), 3);
        // Sort: alive.rs (real, idx 0), затем ghosts по алфавиту: aa, zz.
        // apply_to_nodes сохраняет порядок реальных детей и добавляет
        // ghost'ов в конец отсортированными.
        assert_eq!(kids[0].label, "alive.rs");
        assert_eq!(kids[1].label, "aa.rs");
        assert!(kids[1].id.starts_with(GHOST_PREFIX));
        assert_eq!(kids[2].label, "zz.rs");
        assert!(kids[2].id.starts_with(GHOST_PREFIX));
    }

    #[test]
    fn inject_ghost_skips_files_already_present_on_disk() {
        // Файл всё ещё есть в дереве (read_dir вернул его), но git
        // считает его Deleted (например, `git rm --cached`). Не должны
        // дублировать ghost'ом — оригинальный узел уже декорирован
        // strikethrough в apply_recursive.
        let palette = GitPalette::default();
        let mut git = GitStatusMap::empty();
        git.workdir = Some(p("/repo"));
        git.files.insert(p("/repo/old.txt"), GitStatus::Deleted);

        let nodes = vec![TreeNode::branch(
            "/repo",
            "repo",
            vec![TreeNode::leaf("/repo/old.txt", "old.txt")],
        )];
        let out = apply_to_nodes(nodes, &git, &palette);
        // У корня нет статуса (workdir исключён из folders), но ghost
        // injecting не должен сработать для уже присутствующего файла.
        assert_eq!(out[0].children.len(), 1);
        assert_eq!(out[0].children[0].id, "/repo/old.txt");
    }
}
