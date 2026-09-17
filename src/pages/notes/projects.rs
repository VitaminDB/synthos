//! Открытые проекты заметок: несколько `.syn` одновременно, плитка рейла на
//! каждый.
//!
//! `NotesCtx` один, и его сигналы держат **активный** проект — всё, что
//! рисуют страница заметок и видит агент. Остальные открытые проекты лежат
//! стешем ([`ProjectStash`]): те же ручки страниц и объектов, поэтому
//! переключение плиткой не теряет ни правок, ни истории undo. UI ручки
//! неактивного проекта не трогает, а перед сменой всё грязное уходит в
//! очередь автосейва с путём своего файла ([`autosave::enqueue_dirty`]) —
//! запись догоняет уже без сигналов.
//!
//! Агент видит все открытые проекты, и UI при этом не переключается:
//! неактивный проект он правит в отдельном контексте
//! ([`NotesCtx::with_project`]) над теми же ручками стеша.
//!
//! Сохранение — прежний автосейв; «Сохранить» лишь пишет очередь сразу, а
//! «Сохранить как» кладёт уплотнённую копию в новый файл и переводит на неё
//! плитку (старый файл остаётся как был).

use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use syngui::prelude::*;

use crate::config::{now_millis, AppConfig, NotesProjectConfig};

use super::activity::ActivityLogHandle;
use super::autosave;
use super::calendar::CalendarStoreHandle;
use super::index::VaultIndex;
use super::project::{self, PageNode, ProbeError, ProjectTree};
use super::reminders::ReminderStateHandle;
use super::state::{LiveObject, LivePage, NotesCtx};

/// Сколько недавних проектов помнить.
pub const RECENT_MAX: usize = 10;

thread_local! {
    /// Контекст, в котором агент работает с неактивным проектом
    /// ([`NotesCtx::with_project`]). Слоты сигналов syngui не
    /// освобождаются — контекст заводится один раз на поток и
    /// переиспользуется; на время работы он вынут из ячейки, и вложенный
    /// вызов получит свой.
    static DETACHED: Cell<Option<NotesCtx>> = const { Cell::new(None) };
}

/// Состояние неактивного открытого проекта — всё, что `NotesCtx` держит в
/// сигналах про проект.
#[derive(Clone)]
pub struct ProjectStash {
    pub tree: Arc<ProjectTree>,
    pub tree_rev: u64,
    pub expanded: HashSet<String>,
    pub active: Option<String>,
    pub show_graph: bool,
    pub pages: Vec<LivePage>,
    pub objects: Vec<LiveObject>,
    pub calendar: Option<CalendarStoreHandle>,
    pub activity: Option<ActivityLogHandle>,
    pub reminder_state: Option<ReminderStateHandle>,
}

/// Открытый проект (плитка рейла).
#[derive(Clone)]
pub struct OpenProject {
    /// Нормализованный путь файла ([`project::normalize_path`]).
    pub path: PathBuf,
    /// Штамп появления плитки (порядок рейла).
    pub opened_at: u64,
    /// Где в проекте остановились — из конфига, пока проект в этой сессии
    /// ещё не загружали.
    pub saved_active: Option<String>,
    pub saved_expanded: Vec<String>,
    /// Стеш неактивного загруженного проекта; у активного — `None`: его
    /// состояние в сигналах контекста.
    pub stash: Option<Box<ProjectStash>>,
}

impl OpenProject {
    pub fn new(path: PathBuf, opened_at: u64) -> Self {
        Self { path, opened_at, saved_active: None, saved_expanded: Vec::new(), stash: None }
    }

    pub fn title(&self) -> String {
        project::project_title(&self.path)
    }
}

/// Почему операция над проектом не удалась — текст для тоста.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectError {
    Missing(PathBuf),
    NotNotes(PathBuf),
    AlreadyOpen(PathBuf),
    NoProject,
    Io(String),
}

impl ProjectError {
    pub fn message(&self) -> String {
        let name = |p: &Path| p.display().to_string();
        match self {
            ProjectError::Missing(p) => tr!("notes.project.error.missing", path = name(p)),
            ProjectError::NotNotes(p) => tr!("notes.project.error.not_notes", path = name(p)),
            ProjectError::AlreadyOpen(p) => tr!("notes.project.error.already_open", path = name(p)),
            ProjectError::NoProject => tr!("notes.project.error.no_project"),
            ProjectError::Io(e) => tr!("notes.project.error.io", error = e.clone()),
        }
    }

    fn from_probe(path: &Path, e: ProbeError) -> Self {
        match e {
            ProbeError::Missing => ProjectError::Missing(path.to_path_buf()),
            ProbeError::NotNotes => ProjectError::NotNotes(path.to_path_buf()),
            ProbeError::Unreadable(e) => ProjectError::Io(e),
        }
    }
}

pub type ProjectOpResult = std::result::Result<(), ProjectError>;

/// Состояние проекта, прочитанное с диска: дерево, активная страница
/// (сохранённая либо первая), раскрытые узлы и её предки.
pub fn load_stash(path: &Path, active_page: Option<String>, expanded: Vec<String>) -> ProjectStash {
    project::compact_if_needed(path);
    let tree = project::read_tree(path);
    let active = active_page.filter(|id| tree.find(id).is_some()).or_else(|| tree.first_id());
    let mut expanded: HashSet<String> = expanded.into_iter().filter(|id| tree.find(id).is_some()).collect();
    if let Some(id) = &active {
        for (pid, _) in tree.path_of(id) {
            if pid != *id {
                expanded.insert(pid);
            }
        }
    }
    ProjectStash {
        tree: Arc::new(tree),
        tree_rev: 0,
        expanded,
        active,
        show_graph: false,
        pages: Vec::new(),
        objects: Vec::new(),
        calendar: None,
        activity: None,
        reminder_state: None,
    }
}

impl NotesCtx {
    // ─── Смена активного проекта ──────────────────────────────────────────

    /// Снять состояние активного проекта с сигналов.
    fn take_stash(&self) -> ProjectStash {
        ProjectStash {
            tree: self.tree.get_untracked(),
            tree_rev: self.tree_rev.get_untracked(),
            expanded: self.expanded.get_untracked(),
            active: self.active.get_untracked(),
            show_graph: self.show_graph.get_untracked(),
            pages: self.pages.get_untracked(),
            objects: self.objects.get_untracked(),
            calendar: self.calendar.get_untracked(),
            activity: self.activity.get_untracked(),
            reminder_state: self.reminder_state.get_untracked(),
        }
    }

    /// Выложить состояние проекта в сигналы. `fresh` — прочитано с диска:
    /// сохранённая ревизия дерева обнуляется вместе с ревизией в сигнале.
    fn apply_stash(&self, path: &Path, s: ProjectStash, fresh: bool) {
        autosave::set_project_path(path.to_path_buf());
        if fresh {
            autosave::mark_saved(project::TREE_PATH, 0);
        }
        self.project_path.set(path.to_path_buf());
        self.project_title.set(project::project_title(path));
        // Ручки не сравниваются — через `update`, а не `set`.
        self.pages.update(|v| *v = s.pages);
        self.objects.update(|v| *v = s.objects);
        self.calendar.update(|v| *v = s.calendar);
        self.activity.update(|v| *v = s.activity);
        self.reminder_state.update(|v| *v = s.reminder_state);
        self.reminders.set(Arc::new(Vec::new()));
        self.expanded.set(s.expanded);
        self.show_graph.set(s.show_graph);
        self.tree_rev.set(s.tree_rev);
        self.tree.set(s.tree.clone());
        // Индекс связей строится заново: пока проект лежал в стеше, записи
        // его страниц индекс активного проекта не трогали.
        let index = VaultIndex::build(&s.tree, |id| Some(self.page_markdown(id)));
        self.index.set(Arc::new(index));
        self.active.set(s.active.clone());
        if let Some(id) = s.active {
            self.page(&id);
        }
        self.bump_doc_epoch();
    }

    /// Ни одного активного проекта: страница заметок показывает экран
    /// «создать / открыть».
    fn apply_empty(&self) {
        autosave::set_project_path(PathBuf::new());
        self.project_path.set(PathBuf::new());
        self.project_title.set(String::new());
        self.pages.update(Vec::clear);
        self.objects.update(Vec::clear);
        self.calendar.update(|v| *v = None);
        self.activity.update(|v| *v = None);
        self.reminder_state.update(|v| *v = None);
        self.reminders.set(Arc::new(Vec::new()));
        self.expanded.set(HashSet::new());
        self.show_graph.set(false);
        self.tree_rev.set(0);
        self.tree.set(Arc::new(ProjectTree::new()));
        self.index.set(Arc::new(VaultIndex::default()));
        self.active.set(None);
        self.bump_doc_epoch();
    }

    /// Попапы и режимы, привязанные к страницам уходящего проекта.
    fn close_overlays(&self) {
        self.icon_picker_open.set(false);
        self.icon_picker_page.set(None);
        self.doc_menu_open.set(false);
        self.reminders_open.set(false);
        self.viewer_open.set(false);
        self.renaming.set(None);
        self.tree_drop.set(None);
    }

    /// Сделать открытый проект активным (клик по плитке). `false` — такого
    /// среди открытых нет.
    pub fn switch_project(&self, path: &Path) -> bool {
        self.activate_project(path, true)
    }

    /// `refresh` — пересобрать список напоминаний под новый проект (при
    /// старте его собирает минутный таймер).
    pub(super) fn activate_project(&self, path: &Path, refresh: bool) -> bool {
        let cur = self.project_path.get_untracked();
        if cur == path {
            return true;
        }
        let Some(target) = self.projects.get_untracked().into_iter().find(|p| p.path == path) else {
            return false;
        };
        self.close_overlays();
        if self.has_project() {
            autosave::enqueue_dirty(*self);
            let stash = self.take_stash();
            self.projects.update(|v| {
                if let Some(p) = v.iter_mut().find(|p| p.path == cur) {
                    p.stash = Some(Box::new(stash));
                }
            });
        }
        let (stash, fresh) = match target.stash {
            Some(s) => (*s, false),
            None => (load_stash(path, target.saved_active, target.saved_expanded), true),
        };
        self.projects.update(|v| {
            if let Some(p) = v.iter_mut().find(|p| p.path == path) {
                p.stash = None;
            }
        });
        self.apply_stash(path, stash, fresh);
        if refresh {
            super::reminders::refresh_list(*self);
        }
        true
    }

    // ─── Открыть / создать / закрыть ──────────────────────────────────────

    /// Открыть файл проекта новой плиткой (уже открытый — просто показать).
    pub fn open_project(&self, path: &Path) -> ProjectOpResult {
        let path = self.open_project_quietly(path)?;
        self.switch_project(&path);
        self.remember_recent(&path);
        Ok(())
    }

    /// Открыть файл проекта плиткой, не показывая его: агент работает в
    /// нём, а пользователь остаётся там, где был. Показывается, только если
    /// другого открытого проекта нет. Возвращает нормализованный путь.
    pub fn open_project_quietly(&self, path: &Path) -> std::result::Result<PathBuf, ProjectError> {
        let path = project::normalize_path(path);
        if !self.is_open(&path) {
            project::probe(&path).map_err(|e| ProjectError::from_probe(&path, e))?;
            // Файл, закрытый раньше в этой сессии, мог смениться на диске:
            // ревизии прошлого открытия к нему не относятся.
            autosave::forget_project(&path);
            project::invalidate(&path);
            self.projects.update(|v| v.push(OpenProject::new(path.clone(), now_millis())));
            self.remember_recent(&path);
        }
        if !self.has_project() {
            self.switch_project(&path);
        }
        Ok(path)
    }

    /// Выполнить `f` над открытым проектом `path` так, будто он активен, не
    /// трогая того, что показывает UI. Для активного проекта это сам
    /// `self`. Неактивный выкладывается из стеша (или читается с диска) в
    /// отдельный контекст; после `f` его состояние — с правками и историей
    /// undo — возвращается в стеш, а грязное уходит в очередь автосейва
    /// своего файла, как при смене плитки. `None` — среди открытых такого
    /// нет.
    pub fn with_project<R>(&self, path: &Path, f: impl FnOnce(NotesCtx) -> R) -> Option<R> {
        if self.project_path.get_untracked() == path {
            return Some(f(*self));
        }
        let target = self.projects.get_untracked().into_iter().find(|p| p.path == path)?;
        let (stash, fresh) = match target.stash {
            Some(s) => (*s, false),
            None => (load_stash(path, target.saved_active, target.saved_expanded), true),
        };
        let home = autosave::project_path();
        let detached = DETACHED.with(Cell::take).unwrap_or_else(|| NotesCtx::blank(Vec::new(), Vec::new()));
        // Доски и диаграммы запоминают сигнал ревизии объектов контекста,
        // в котором загружены, — он должен быть тем, что слушает UI: туда
        // проект попадёт, когда пользователь переключится на его плитку.
        let ctx = NotesCtx { objects_rev: self.objects_rev, ..detached };
        ctx.apply_stash(path, stash, fresh);
        let out = f(ctx);
        autosave::enqueue_dirty(ctx);
        let stash = ctx.take_stash();
        ctx.apply_empty();
        autosave::set_project_path(home);
        DETACHED.with(|d| d.set(Some(detached)));
        self.projects.update(|v| {
            if let Some(p) = v.iter_mut().find(|p| p.path == path) {
                p.stash = Some(Box::new(stash));
            }
        });
        Some(out)
    }

    /// Новый проект в файле `path` (расширение `.syn` дописывается) с одной
    /// пустой страницей; открывается сразу. Существующий файл заменяется —
    /// подтверждение замены спрашивает диалог сохранения.
    pub fn create_project(&self, path: &Path) -> ProjectOpResult {
        let path = project::normalize_path(&project::with_syn_extension(path));
        if self.is_open(&path) {
            return Err(ProjectError::AlreadyOpen(path));
        }
        let mut tree = ProjectTree::new();
        let page = PageNode::new(tr!("notes.untitled"));
        let files = vec![(project::page_path(&page.id), Vec::new())];
        tree.insert(None, None, page);
        project::create(&path, &tree, files).map_err(ProjectError::Io)?;
        self.open_project(&path)
    }

    /// Закрыть проект: дописать очередь в файл и убрать плитку. Активный
    /// уступает место соседней плитке, последний — экрану «создать /
    /// открыть». Не записалось — проект остаётся открытым (правки целы).
    pub fn close_project(&self, path: &Path) -> ProjectOpResult {
        let list = self.projects.get_untracked();
        let Some(idx) = list.iter().position(|p| p.path == path) else { return Ok(()) };
        let active = self.project_path.get_untracked() == path;
        if active {
            autosave::enqueue_dirty(*self);
        }
        if path.is_file() {
            autosave::flush_project(path).map_err(ProjectError::Io)?;
        } else {
            // Файл удалили снаружи — писать некуда.
            log::warn!("notes: файл проекта {} пропал, хвост очереди выброшен", path.display());
        }
        autosave::forget_project(path);
        project::invalidate(path);
        self.remember_recent(path);
        let next = list
            .get(idx + 1)
            .or_else(|| idx.checked_sub(1).and_then(|i| list.get(i)))
            .map(|p| p.path.clone());
        self.projects.update(|v| v.retain(|p| p.path != path));
        if active {
            self.close_overlays();
            self.apply_empty();
            if let Some(next) = next {
                self.switch_project(&next);
            }
        }
        Ok(())
    }

    // ─── Сохранение ───────────────────────────────────────────────────────

    /// Записать правки активного проекта прямо сейчас (Ctrl+S).
    pub fn save_now(&self) -> ProjectOpResult {
        if !self.has_project() {
            return Err(ProjectError::NoProject);
        }
        autosave::enqueue_dirty(*self);
        autosave::flush_project(&self.project_path.get_untracked()).map_err(ProjectError::Io)
    }

    /// «Сохранить как»: уплотнённая копия активного проекта в `dst`, плитка
    /// переходит на копию. Исходный файл остаётся с последними правками.
    pub fn save_project_as(&self, dst: &Path) -> ProjectOpResult {
        if !self.has_project() {
            return Err(ProjectError::NoProject);
        }
        let src = self.project_path.get_untracked();
        let dst = project::normalize_path(&project::with_syn_extension(dst));
        if dst == src {
            return self.save_now();
        }
        if self.is_open(&dst) {
            return Err(ProjectError::AlreadyOpen(dst));
        }
        autosave::enqueue_dirty(*self);
        autosave::flush_project(&src).map_err(ProjectError::Io)?;
        autosave::with_write_lock(|| project::save_copy(&src, &dst)).map_err(ProjectError::Io)?;
        // Файл до копии мог быть закрытым проектом — его ревизии не наши.
        let dst = project::normalize_path(&dst);
        autosave::forget_project(&dst);
        autosave::rekey_project(&src, &dst);
        autosave::set_project_path(dst.clone());
        self.projects.update(|v| {
            if let Some(p) = v.iter_mut().find(|p| p.path == src) {
                p.path = dst.clone();
            }
        });
        self.project_path.set(dst.clone());
        self.project_title.set(project::project_title(&dst));
        self.remember_recent(&src);
        self.remember_recent(&dst);
        // Резолвер медиа редактора держит путь файла — пересобрать.
        self.bump_doc_epoch();
        Ok(())
    }

    // ─── Прочее ───────────────────────────────────────────────────────────

    /// Для агента: если ни один проект не открыт — открыть недавний, а без
    /// него дефолтный `~/Documents/SynthOS Notes.syn` (создав при нужде).
    pub fn ensure_project(&self) -> ProjectOpResult {
        if self.has_project() {
            return Ok(());
        }
        if let Some(first) = self.projects.get_untracked().first() {
            self.switch_project(&first.path.clone());
            return Ok(());
        }
        for path in self.recent.get_untracked() {
            if path.is_file() && self.open_project(&path).is_ok() {
                return Ok(());
            }
        }
        let default = project::resolve_project_path("");
        if !default.exists() {
            project::create(&default, &ProjectTree::new(), Vec::new()).map_err(ProjectError::Io)?;
        }
        self.open_project(&default)
    }

    /// Показать страницу, в каком бы из открытых проектов она ни была
    /// (ссылка из чата на страницу, созданную агентом до смены проекта).
    pub fn open_page_anywhere(&self, page_id: &str) -> bool {
        if self.tree.get_untracked().find(page_id).is_some() {
            self.activate(page_id);
            return true;
        }
        match self.projects_with_page(page_id).first() {
            Some(path) => {
                self.switch_project(path);
                self.activate(page_id);
                true
            }
            None => false,
        }
    }

    /// Неактивные открытые проекты, в дереве которых есть страница с этим
    /// id, в порядке плиток. Больше одного — у копии «Сохранить как» те же
    /// id страниц.
    pub fn projects_with_page(&self, page_id: &str) -> Vec<PathBuf> {
        let active = self.project_path.get_untracked();
        self.projects
            .get_untracked()
            .into_iter()
            .filter(|p| p.path != active)
            .filter(|p| match &p.stash {
                Some(s) => s.tree.find(page_id).is_some(),
                None => project::read_tree(&p.path).find(page_id).is_some(),
            })
            .map(|p| p.path)
            .collect()
    }

    /// То же для объекта (доски, диаграммы, карты, календаря, графика):
    /// в пуле стеша либо файлом в бандле.
    pub fn projects_with_object(&self, kind: &str, id: &str) -> Vec<PathBuf> {
        let active = self.project_path.get_untracked();
        let file = project::object_path(kind, id);
        self.projects
            .get_untracked()
            .into_iter()
            .filter(|p| p.path != active)
            .filter(|p| {
                p.stash.as_ref().is_some_and(|s| s.objects.iter().any(|o| o.id() == id))
                    || project::read_text(&p.path, &file).is_some()
            })
            .map(|p| p.path)
            .collect()
    }

    /// Есть ли объект в показанном проекте (без загрузки).
    pub fn has_object(&self, kind: &str, id: &str) -> bool {
        self.objects.get_untracked().iter().any(|o| o.id() == id)
            || project::read_text(&self.project_path.get_untracked(), &project::object_path(kind, id)).is_some()
    }

    fn remember_recent(&self, path: &Path) {
        let path = path.to_path_buf();
        self.recent.update(|v| {
            v.retain(|p| *p != path);
            v.insert(0, path);
            v.truncate(RECENT_MAX);
        });
    }

    /// Недавние проекты, которые можно открыть: не открытые сейчас и
    /// существующие на диске.
    pub fn recent_closed(&self) -> Vec<PathBuf> {
        let open: Vec<PathBuf> = self.projects.get().iter().map(|p| p.path.clone()).collect();
        self.recent.get().into_iter().filter(|p| !open.contains(p) && p.is_file()).collect()
    }

    // ─── Конфиг ───────────────────────────────────────────────────────────

    /// Снимок для конфига. `.get()` подписывает эффект автосейва конфига на
    /// смену проектов, активной страницы и раскрытых узлов.
    pub fn persist(&self) -> NotesPersist {
        let active_path = self.project_path.get();
        let sorted = |set: &HashSet<String>| {
            let mut v: Vec<String> = set.iter().cloned().collect();
            v.sort();
            v
        };
        let active_page = self.active.get();
        let expanded = sorted(&self.expanded.get());
        let projects = self
            .projects
            .get()
            .iter()
            .map(|p| {
                let (active_page, expanded) = if p.path == active_path {
                    (active_page.clone(), expanded.clone())
                } else if let Some(s) = &p.stash {
                    (s.active.clone(), sorted(&s.expanded))
                } else {
                    (p.saved_active.clone(), p.saved_expanded.clone())
                };
                NotesProjectConfig { path: p.path.display().to_string(), opened_at: p.opened_at, active_page, expanded }
            })
            .collect();
        NotesPersist {
            projects,
            active_project: active_path.display().to_string(),
            recent: self.recent.get().iter().map(|p| p.display().to_string()).collect(),
        }
    }
}

/// Поля конфига про проекты заметок.
pub struct NotesPersist {
    pub projects: Vec<NotesProjectConfig>,
    pub active_project: String,
    pub recent: Vec<String>,
}

/// Что восстановить при старте.
pub struct Restored {
    pub projects: Vec<OpenProject>,
    pub active: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
}

/// Открытые проекты из конфига. Конфиг без списка (обновление со времён
/// одного проекта) даёт единственный проект: настроенный или дефолтный
/// файл, как раньше, с его активной страницей и раскрытыми узлами.
/// Пропавшие файлы пропускаются.
pub fn restore(cfg: &AppConfig) -> Restored {
    let recent: Vec<PathBuf> = cfg.notes_recent.iter().map(|p| project::normalize_path(Path::new(p))).collect();
    let mut projects: Vec<OpenProject> = Vec::new();
    let mut push = |p: OpenProject| {
        if !projects.iter().any(|o| o.path == p.path) {
            projects.push(p);
        }
    };
    match &cfg.notes_projects {
        Some(list) => {
            for p in list {
                let path = project::normalize_path(Path::new(&p.path));
                if !path.is_file() {
                    log::warn!("notes: проект {} не найден — плитка не восстановлена", path.display());
                    continue;
                }
                push(OpenProject {
                    path,
                    opened_at: p.opened_at,
                    saved_active: p.active_page.clone(),
                    saved_expanded: p.expanded.clone(),
                    stash: None,
                });
            }
        }
        None => {
            if let Some(path) = legacy_project(cfg) {
                push(OpenProject {
                    path,
                    opened_at: cfg.notes_tile_opened_at.unwrap_or_else(now_millis),
                    saved_active: cfg.notes_active.clone(),
                    saved_expanded: cfg.notes_expanded.clone(),
                    stash: None,
                });
            }
        }
    }
    let wanted = (!cfg.notes_active_project.is_empty())
        .then(|| project::normalize_path(Path::new(&cfg.notes_active_project)));
    let active = wanted
        .filter(|w| projects.iter().any(|p| p.path == *w))
        .or_else(|| projects.first().map(|p| p.path.clone()));
    Restored { projects, active, recent }
}

/// Проект времён одного проекта. Настроенный файл создаётся, если его нет;
/// дефолтный — только если он уже есть или есть папка первой волны для
/// миграции: у свежей установки проектов нет, пока их не создадут.
fn legacy_project(cfg: &AppConfig) -> Option<PathBuf> {
    let configured = !cfg.notes_project_path.trim().is_empty();
    let path = project::resolve_project_path(&cfg.notes_project_path);
    if path.exists() {
        return Some(project::normalize_path(&path));
    }
    let migrated = project::migrate_folder(&project::legacy_vault_path(&cfg.notes_vault_path));
    if !configured && migrated.is_none() {
        return None;
    }
    let (tree, files) = migrated.unwrap_or_else(|| (ProjectTree::new(), Vec::new()));
    match project::create(&path, &tree, files) {
        Ok(()) => {
            log::info!("notes: создан проект {} ({} страниц)", path.display(), tree.all().len());
            Some(project::normalize_path(&path))
        }
        Err(e) => {
            log::error!("notes: не удалось создать проект {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("synthos-notes-projects-{}", project::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        project::normalize_path(&dir)
    }

    /// Проект с одной страницей; возвращает (файл, id страницы).
    fn make_project(dir: &Path, name: &str, text: &str) -> (PathBuf, String) {
        let path = dir.join(format!("{name}.syn"));
        let mut tree = ProjectTree::new();
        let page = PageNode::new(name);
        let id = page.id.clone();
        tree.insert(None, None, page);
        project::create(&path, &tree, vec![(project::page_path(&id), text.as_bytes().to_vec())]).unwrap();
        (project::normalize_path(&path), id)
    }

    fn cfg_with(dir: &Path, projects: &[&Path], active: &Path) -> AppConfig {
        AppConfig {
            notes_projects: Some(
                projects
                    .iter()
                    .enumerate()
                    .map(|(i, p)| NotesProjectConfig { path: p.display().to_string(), opened_at: 1000 + i as u64, ..Default::default() })
                    .collect(),
            ),
            notes_active_project: active.display().to_string(),
            notes_vault_path: dir.join("no-vault").display().to_string(),
            ..AppConfig::default()
        }
    }

    fn file_text(path: &Path, id: &str) -> String {
        project::invalidate(path);
        project::read_text(path, &project::page_path(id)).unwrap_or_default()
    }

    /// Смена плитки не теряет ни правок, ни истории: ручка страницы лежит в
    /// стеше, а грязное уходит в очередь своего файла.
    #[test]
    fn switching_keeps_edits_and_history_per_project() {
        let dir = temp_dir();
        let (a, a_page) = make_project(&dir, "A", "alpha");
        let (b, b_page) = make_project(&dir, "B", "beta");
        let ctx = NotesCtx::new_or_restore(&cfg_with(&dir, &[&a, &b], &a));
        assert_eq!(ctx.project_path.get_untracked(), a);
        assert_eq!(ctx.active.get_untracked().as_deref(), Some(a_page.as_str()));

        assert!(ctx.set_page_markdown(&a_page, "alpha 2"));
        assert!(ctx.switch_project(&b));
        assert!(autosave::has_pending(&a), "правка A уходит в очередь до смены");
        assert_eq!(ctx.active_page().unwrap().markdown().trim(), "beta");
        assert!(ctx.set_page_markdown(&b_page, "beta 2"));

        assert!(ctx.switch_project(&a));
        let page = ctx.active_page().unwrap();
        assert_eq!(page.markdown().trim(), "alpha 2");
        assert!(page.handle.history_state().get_untracked().0, "undo страницы пережил смену проекта");

        autosave::flush_all().unwrap();
        assert_eq!(file_text(&a, &a_page).trim(), "alpha 2");
        assert_eq!(file_text(&b, &b_page).trim(), "beta 2");
    }

    /// У всех проектов один путь `notes/tree.json`: ревизия дерева второго
    /// проекта ниже, чем у первого, и без проекта в ключе автосейва его
    /// правка считалась бы уже записанной.
    #[test]
    fn tree_edit_of_second_project_is_not_shadowed_by_first() {
        let dir = temp_dir();
        let (a, a_page) = make_project(&dir, "A", "");
        let (b, b_page) = make_project(&dir, "B", "");
        let ctx = NotesCtx::new_or_restore(&cfg_with(&dir, &[&a, &b], &a));
        for t in ["A1", "A2", "A3"] {
            ctx.rename_page(&a_page, t);
        }
        ctx.save_now().unwrap();
        ctx.switch_project(&b);
        ctx.rename_page(&b_page, "B1");
        ctx.save_now().unwrap();
        project::invalidate(&b);
        assert_eq!(project::read_tree(&b).title_of(&b_page).as_deref(), Some("B1"));
        project::invalidate(&a);
        assert_eq!(project::read_tree(&a).title_of(&a_page).as_deref(), Some("A3"));
    }

    #[test]
    fn close_writes_file_and_activates_neighbour() {
        let dir = temp_dir();
        let (a, a_page) = make_project(&dir, "A", "alpha");
        let (b, _) = make_project(&dir, "B", "beta");
        let ctx = NotesCtx::new_or_restore(&cfg_with(&dir, &[&a, &b], &a));
        ctx.set_page_markdown(&a_page, "alpha closed");
        ctx.close_project(&a).unwrap();
        assert_eq!(file_text(&a, &a_page).trim(), "alpha closed");
        assert_eq!(ctx.project_path.get_untracked(), b);
        assert_eq!(ctx.projects.get_untracked().len(), 1);
        assert_eq!(ctx.recent.get_untracked().first(), Some(&a));

        ctx.close_project(&b).unwrap();
        assert!(!ctx.has_project());
        assert!(ctx.tree.get_untracked().is_empty());
        assert!(ctx.active_page().is_none());
        // Недавние — закрытые, свежие первыми.
        assert_eq!(ctx.recent_closed(), vec![b.clone(), a.clone()]);

        // Повторное открытие из недавних: правка на месте.
        ctx.open_project(&a).unwrap();
        assert_eq!(ctx.active_page().unwrap().markdown().trim(), "alpha closed");
    }

    #[test]
    fn save_as_writes_copy_and_moves_tile() {
        let dir = temp_dir();
        let (a, a_page) = make_project(&dir, "A", "alpha");
        let ctx = NotesCtx::new_or_restore(&cfg_with(&dir, &[&a], &a));
        ctx.set_page_markdown(&a_page, "alpha 2");
        ctx.save_project_as(&dir.join("Копия")).unwrap();
        let copy = project::normalize_path(&dir.join("Копия.syn"));
        assert_eq!(ctx.project_path.get_untracked(), copy);
        assert_eq!(ctx.project_title.get_untracked(), "Копия");
        assert_eq!(ctx.projects.get_untracked()[0].path, copy);
        assert_eq!(file_text(&copy, &a_page).trim(), "alpha 2");
        assert_eq!(file_text(&a, &a_page).trim(), "alpha 2", "исходник получил последние правки");

        // Дальше правки идут в копию, исходник не трогается.
        ctx.set_page_markdown(&a_page, "alpha 3");
        ctx.save_now().unwrap();
        assert_eq!(file_text(&copy, &a_page).trim(), "alpha 3");
        assert_eq!(file_text(&a, &a_page).trim(), "alpha 2");
    }

    #[test]
    fn open_checks_the_file() {
        let dir = temp_dir();
        let (a, _) = make_project(&dir, "A", "");
        let ctx = NotesCtx::new_or_restore(&cfg_with(&dir, &[&a], &a));
        let missing = dir.join("нет.syn");
        assert!(matches!(ctx.open_project(&missing), Err(ProjectError::Missing(_))));
        let model = dir.join("model.syn");
        synaptix_bundle::BundleBuilder::new("m", "1")
            .purpose("llm")
            .add_file_bytes("config.json", b"{}".to_vec(), synaptix_bundle::FileTag::Doc)
            .unwrap()
            .write(&model)
            .unwrap();
        assert!(matches!(ctx.open_project(&model), Err(ProjectError::NotNotes(_))));
        assert_eq!(ctx.projects.get_untracked().len(), 1);

        // Уже открытый — второй плиткой не становится.
        ctx.open_project(&a).unwrap();
        assert_eq!(ctx.projects.get_untracked().len(), 1);
        // Новый проект: `.syn` дописывается, страница есть, плитка вторая.
        ctx.create_project(&dir.join("Новый")).unwrap();
        assert_eq!(ctx.projects.get_untracked().len(), 2);
        assert_eq!(ctx.project_title.get_untracked(), "Новый");
        assert_eq!(ctx.tree.get_untracked().all().len(), 1);
    }

    /// Конфиг времён одного проекта: проект открывается плиткой с прежней
    /// активной страницей, а снимок для конфига восстанавливает то же.
    #[test]
    fn legacy_config_migrates_and_persist_roundtrips() {
        let dir = temp_dir();
        let (a, _) = make_project(&dir, "A", "");
        let mut tree = project::read_tree(&a);
        let second = PageNode::new("Вторая");
        let second_id = second.id.clone();
        tree.insert(None, None, second);
        project::apply_ops(&a, &[project::WriteOp::Put { path: project::TREE_PATH.into(), bytes: tree.serialize().into_bytes() }]).unwrap();
        let (b, b_page) = make_project(&dir, "B", "");

        let legacy = AppConfig {
            notes_project_path: a.display().to_string(),
            notes_active: Some(second_id.clone()),
            notes_tile_opened_at: None,
            notes_vault_path: dir.join("no-vault").display().to_string(),
            ..AppConfig::default()
        };
        let ctx = NotesCtx::new_or_restore(&legacy);
        assert_eq!(ctx.projects.get_untracked().len(), 1);
        assert_eq!(ctx.active.get_untracked().as_deref(), Some(second_id.as_str()));

        ctx.open_project(&b).unwrap();
        let state = ctx.persist();
        assert_eq!(state.active_project, b.display().to_string());
        let cfg = AppConfig {
            notes_projects: Some(state.projects),
            notes_active_project: state.active_project,
            notes_recent: state.recent,
            ..legacy.clone()
        };
        let again = NotesCtx::new_or_restore(&cfg);
        assert_eq!(again.project_path.get_untracked(), b);
        assert_eq!(again.active.get_untracked().as_deref(), Some(b_page.as_str()));
        assert!(again.switch_project(&a));
        assert_eq!(again.active.get_untracked().as_deref(), Some(second_id.as_str()), "страница A из стеша попала в конфиг");
    }

    /// Свежая установка: дефолтного файла нет — проекта тоже нет, пока его
    /// не создадут (раньше пустой проект создавался молча при старте).
    #[test]
    fn fresh_install_has_no_project_but_agent_can_get_one() {
        let dir = temp_dir();
        let cfg = AppConfig { notes_vault_path: dir.join("no-vault").display().to_string(), ..AppConfig::default() };
        let restored = restore(&AppConfig { notes_project_path: String::new(), ..cfg });
        // Дефолтный файл может быть у разработчика на машине — проверяем
        // только, что без него проект не придумывается.
        if !project::resolve_project_path("").exists() {
            assert!(restored.projects.is_empty());
            assert!(restored.active.is_none());
        }
    }
}
