//! Реактивное состояние страницы редактора кода в multi-session-варианте.
//!
//! Архитектура:
//! - [`CodeSession`] — Copy-handle одной независимой сессии (один проект):
//!   корень папки, дерево, открытые файлы, буферы, терминалы. Все поля —
//!   `RwSignal<…>` (Copy), сама структура Copy → передаётся в callback'и
//!   по значению.
//! - [`CodeEditorCtx`] — менеджер сессий. Держит `Vec<CodeSession>`,
//!   активный id, монотонный счётчик id'ов и `session_gen` (bump'ится при
//!   create/switch/close — pages подписаны на него для пересборки UI).
//!
//! Persist: список сессий и активный индекс сохраняются в
//! `AppConfig.code_sessions` / `active_code_session` (см.
//! `install_config_autosave` в `lib.rs`). Открытые файлы и активный файл
//! не персистятся — UX-выигрыш не оправдывает per-keystroke IO×N сессий.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use syngui::prelude::*;
use syngui::widgets::{TerminalConfig, TerminalSession, TreeNode};
use tracing::{debug, info, warn};

use crate::config::{
    default_code_editor_left_split_ratio, default_code_editor_right_split_ratio,
    default_code_editor_split_ratio, CodeSessionConfig,
};
use syngui::widgets::input::code_editor::EditorPersistedState;
use crate::context::AppCtx;

use super::dialogs::DialogKind;
use super::drafts;
use super::fs_ops;
use super::fs_watcher::FsWatcher;
use super::git_status::{GitStatusMap, GitStatusWorker};

/// Process-wide registry FS-watchers по `SessionId`. CodeSession Copy
/// через RwSignal-поля и не может напрямую держать `Arc<Mutex<FsWatcher>>`
/// (Mutex не PartialEq, RwSignal требует PartialEq). Реестр здесь —
/// единственное место, которое владеет watcher'ами; lifetime привязан к
/// процессу (drop при выходе через atexit). Очистка ручная: при close
/// сессии вызывается [`drop_watcher`].
fn watcher_registry() -> &'static Mutex<HashMap<SessionId, FsWatcher>> {
    static R: OnceLock<Mutex<HashMap<SessionId, FsWatcher>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Установить новый watcher для сессии. Старый, если был — дроп'ается
/// (RAII). Сама `Result` from `FsWatcher::start` обрабатывается caller'ом
/// (через `.ok()` или лог).
fn install_watcher(id: SessionId, watcher: FsWatcher) {
    if let Ok(mut g) = watcher_registry().lock() {
        g.insert(id, watcher);
    }
}

/// Дроп watcher'а для сессии (вызывается при close сессии или
/// смене root_folder перед установкой нового).
fn drop_watcher(id: SessionId) {
    if let Ok(mut g) = watcher_registry().lock() {
        g.remove(&id);
    }
}

/// Process-wide набор путей, уже восстановленных/усыновлённых какой-либо
/// сессией при старте. Гарантирует, что один и тот же черновик не будет
/// усыновлён дважды сессиями с пересекающимися/вложенными корнями.
fn adopted_registry() -> &'static Mutex<HashSet<PathBuf>> {
    static R: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

fn register_adopted(paths: &[PathBuf]) {
    if let Ok(mut g) = adopted_registry().lock() {
        for p in paths {
            g.insert(p.clone());
        }
    }
}

fn claim_adopted(path: &PathBuf) -> bool {
    match adopted_registry().lock() {
        Ok(mut g) => g.insert(path.clone()),
        Err(_) => true,
    }
}

/// Process-wide registry git-status воркеров — параллельно
/// `watcher_registry`. Воркер живёт пока сессия открыта; при
/// `set_root_folder` или `close` старый воркер дроп'ается, новый
/// устанавливается.
fn git_worker_registry() -> &'static Mutex<HashMap<SessionId, GitStatusWorker>> {
    static R: OnceLock<Mutex<HashMap<SessionId, GitStatusWorker>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn install_git_worker(id: SessionId, worker: GitStatusWorker) {
    if let Ok(mut g) = git_worker_registry().lock() {
        g.insert(id, worker);
    }
}

fn drop_git_worker(id: SessionId) {
    if let Ok(mut g) = git_worker_registry().lock() {
        g.remove(&id);
    }
}

/// Запросить refresh git-статуса для сессии. Безопасно с любого потока:
/// если воркер не зарегистрирован — no-op (некритично).
/// Вызывается из `fs_watcher::apply_batch` при любом изменении в workdir
/// или в `.git/HEAD|index|refs/`.
pub(super) fn request_git_refresh(session: CodeSession) {
    if let Ok(g) = git_worker_registry().lock() {
        if let Some(w) = g.get(&session.id) {
            w.request_refresh();
        }
    }
}

/// Стабильный runtime-id сессии. Монотонно растёт через `next_id`,
/// не перевыдаётся при close — closure-биндинги в sidebar безопасны.
/// Между запусками не сохраняется (id переинициализируются при загрузке;
/// активный индекс persist'ится позиционно через `active_code_session`).
pub type SessionId = u64;

/// Лимит размера файла для открытия в редакторе. Файлы больше этого
/// порога — не открываются (гард в [`open_file`]); пользователь видит
/// snackbar-уведомление и лог. Цель — не блокировать UI чтением
/// многосот-мегабайтных файлов и не съесть память.
pub const MAX_OPEN_FILE_BYTES: u64 = 5 * 1024 * 1024;

// ─────────────────────────────────────────────────────────────────────────────
// CodeSession — per-project state
// ─────────────────────────────────────────────────────────────────────────────

/// Одна независимая сессия редактора кода. Все поля — `RwSignal<T>`,
/// поэтому `Copy` через derive.
#[derive(Clone, Copy)]
pub struct CodeSession {
    pub id: SessionId,
    /// Корень проекта. `None` = пустая сессия (без выбранной папки).
    pub root_folder: RwSignal<Option<PathBuf>>,
    /// Текущее дерево файлов. Vec не реализует PartialEq для TreeNode,
    /// поэтому обновление — только через `.update(|v| *v = ...)`.
    pub tree_nodes: RwSignal<Vec<TreeNode>>,
    /// Папки, чьё содержимое уже было прочитано через `read_dir`.
    /// Lazy-expand: первое раскрытие читает с диска, остальные — переключают `expanded`.
    pub loaded_dirs: RwSignal<HashSet<PathBuf>>,
    /// Открытые файлы в порядке добавления (правая панель).
    pub open_files: RwSignal<Vec<PathBuf>>,
    /// Активный файл в редакторе. `None` = empty state.
    pub active_file: RwSignal<Option<PathBuf>>,
    /// Выделенный узел TreeView (хранится снаружи виджета — Reactive
    /// пересоздаёт TreeView при каждом обновлении nodes, и внутреннее
    /// `selected` терялось бы).
    pub selected_node: RwSignal<Option<String>>,
    /// Текущее содержимое файла (с несохранёнными правками).
    pub file_contents: RwSignal<HashMap<PathBuf, String>>,
    /// Снимок «что на диске» — для определения dirty.
    pub disk_contents: RwSignal<HashMap<PathBuf, String>>,
    /// Счётчик пересоздания CodeEditor. `MultilineTextEdit` принимает
    /// текст только в конструкторе; смена активного файла = инкремент
    /// `editor_gen` → Reactive пересоздаёт виджет.
    pub editor_gen: RwSignal<u64>,
    /// Таб-состояние терминалов сессии.
    pub terminals: TerminalsState,
    /// Карта git-статусов файлов проекта (Modified/New/Deleted/Conflict +
    /// rollup на папки). Заполняется фоновым воркером `GitStatusWorker`,
    /// сидит в process-wide `git_worker_registry`. Обёрнут в Arc, чтобы
    /// `RwSignal::set` дёргал подписчиков только при смене всей карты, а
    /// не на каждое изменение поля. Для НЕ git-репо карта пустая —
    /// декорации тогда no-op.
    pub git_status: RwSignal<Arc<GitStatusMap>>,
    /// Положение разделителя editor↔terminal (vertical), доля верхней
    /// панели 0.05..0.95. Биндится к `SplitView::ratio_signal`; drag
    /// дивайдера пишет сюда напрямую. Persist'ится через
    /// `CodeSessionConfig.split_ratio` в `install_config_autosave`.
    pub split_ratio: RwSignal<f32>,
    /// Word-wrap (soft-wrap) toggle для CodeEditor. Toggle-кнопка в
    /// header'е переключает; CodeEditor реактивно перестраивается.
    /// Persist'ится через `CodeSessionConfig.soft_wrap`.
    pub soft_wrap: RwSignal<bool>,
    /// Показан ли редактор над терминалом. Без активного файла редактор
    /// схлопывается и центр целиком отдаётся терминалу
    /// ([`install_editor_autohide`]); открытие файла возвращает его
    /// ([`activate_file`]), кнопка в шапке переключает вручную. Persist'ится
    /// через `CodeSessionConfig.editor_visible`.
    pub editor_visible: RwSignal<bool>,
    /// Положение левого разделителя file_tree↔(center+open_files)
    /// (horizontal). Аналогично [`Self::split_ratio`].
    pub left_split_ratio: RwSignal<f32>,
    /// Положение правого разделителя (file_tree+center)↔open_files
    /// (horizontal). Аналогично [`Self::split_ratio`].
    pub right_split_ratio: RwSignal<f32>,
    /// Persisted cursor + scroll позиция per-file. Ключ — путь файла, значение
    /// — `EditorPersistedState` (byte cursor + visual scroll). Заполняется
    /// editor'ом через двусторонний `state_signal`; `install_config_autosave`
    /// сериализует в `AppConfig.code_sessions[i].editor_states`.
    pub editor_states: RwSignal<HashMap<PathBuf, EditorPersistedState>>,
    /// Пути с внешним изменением, конфликтующим с dirty-буфером. Заполняется
    /// `fs_watcher::apply_batch`; UI показывает диалог разрешения по одному.
    pub conflicts: RwSignal<Vec<PathBuf>>,
    /// Unix-миллисекунды создания — порядок плитки в нав-рейле. Persist:
    /// `CodeSessionConfig.created_at`.
    pub created_at: u64,
}

impl CodeSession {
    /// Создаёт пустую сессию с заданным id и опциональной папкой.
    /// При наличии существующей папки сразу подгружает топ-уровень дерева.
    /// Если путь указан, но не существует — сбрасывает `root_folder` в None
    /// (чтобы при следующем save'е конфиг очистился).
    ///
    /// `restore_open` — список путей открытых файлов (из persisted config'а),
    /// которые нужно перечитать с диска. Файлы, которых уже нет, пропускаются
    /// с warn-логом. `restore_active` определяет, какой из них становится
    /// активным после восстановления.
    ///
    /// `split_ratio_init` / `left_split_ratio_init` / `right_split_ratio_init` —
    /// стартовые значения per-session splitter ratio'ов. Вызывающий резолвит
    /// `Option<f32>` из persisted конфига в конкретные значения, подставляя
    /// `default_code_editor_*_split_ratio()` для отсутствующих/None полей.
    fn new(
        id: SessionId,
        folder: Option<PathBuf>,
        restore_open: Vec<PathBuf>,
        restore_active: Option<PathBuf>,
        split_ratio_init: f32,
        left_split_ratio_init: f32,
        right_split_ratio_init: f32,
        soft_wrap_init: bool,
        editor_visible_init: bool,
        editor_states_init: HashMap<PathBuf, EditorPersistedState>,
        created_at: u64,
    ) -> Self {
        let root_folder = use_signal(folder);
        let tree_nodes = use_signal(Vec::new());
        let loaded_dirs = use_signal(HashSet::new());
        let open_files = use_signal(Vec::new());
        let active_file = use_signal(None);
        let selected_node = use_signal(None);
        let file_contents = use_signal(HashMap::new());
        let disk_contents = use_signal(HashMap::new());
        let editor_gen = use_signal(0u64);
        let terminals = TerminalsState::new();
        let git_status = use_signal(Arc::new(GitStatusMap::empty()));
        let split_ratio = use_signal(split_ratio_init);
        let left_split_ratio = use_signal(left_split_ratio_init);
        let right_split_ratio = use_signal(right_split_ratio_init);
        let soft_wrap = use_signal(soft_wrap_init);
        let editor_states = use_signal(editor_states_init);
        let conflicts = use_signal(Vec::new());

        if let Some(path) = root_folder.get_untracked() {
            if path.is_dir() {
                let nodes = fs_ops::read_dir_to_nodes(&path);
                tree_nodes.update(|v| *v = nodes);
                loaded_dirs.update(|s| {
                    s.insert(path);
                });
            } else {
                eprintln!(
                    "[code-editor] Восстановленная папка сессии {:?} не существует — сбрасываю",
                    path
                );
                root_folder.set(None);
            }
        }

        let mut restored: Vec<PathBuf> = Vec::with_capacity(restore_open.len());
        let mut conflict_paths: Vec<PathBuf> = Vec::new();
        for path in restore_open {
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > MAX_OPEN_FILE_BYTES => {
                    eprintln!(
                        "[code-editor] restore: skip большой файл {:?} ({} байт)",
                        path,
                        meta.len()
                    );
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[code-editor] restore: skip {:?}: {e}", path);
                    continue;
                }
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let buffer = match drafts::load_draft_record(&path) {
                        Some(rec) if rec.text != content => {
                            if drafts::disk_newer_than(&path, rec.saved_at) {
                                conflict_paths.push(path.clone());
                            }
                            rec.text
                        }
                        Some(rec) => rec.text,
                        None => content.clone(),
                    };
                    file_contents.update(|m| {
                        m.insert(path.clone(), buffer);
                    });
                    disk_contents.update(|m| {
                        m.insert(path.clone(), content);
                    });
                    restored.push(path);
                }
                Err(e) => {
                    eprintln!("[code-editor] restore: skip {:?}: {e}", path);
                }
            }
        }

        register_adopted(&restored);

        if let Some(root) = root_folder.get_untracked() {
            for rec in drafts::load_all_drafts() {
                if !rec.path.starts_with(&root) || !claim_adopted(&rec.path) {
                    continue;
                }
                let disk = match std::fs::metadata(&rec.path) {
                    Ok(meta) if meta.len() > MAX_OPEN_FILE_BYTES => continue,
                    Ok(_) => match std::fs::read_to_string(&rec.path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    },
                    Err(_) => {
                        drafts::clear_draft(&rec.path);
                        continue;
                    }
                };
                if rec.text == disk {
                    drafts::clear_draft(&rec.path);
                    continue;
                }
                if drafts::disk_newer_than(&rec.path, rec.saved_at) {
                    conflict_paths.push(rec.path.clone());
                }
                eprintln!(
                    "[code-editor] restore: восстановлен черновик {:?} ({} байт)",
                    rec.path,
                    rec.text.len()
                );
                file_contents.update(|m| {
                    m.insert(rec.path.clone(), rec.text);
                });
                disk_contents.update(|m| {
                    m.insert(rec.path.clone(), disk);
                });
                restored.push(rec.path);
            }
        }

        if !conflict_paths.is_empty() {
            conflicts.set(conflict_paths);
        }

        if !restored.is_empty() {
            open_files.set(restored.clone());
            // Активный файл из persisted — только если он реально среди
            // восстановленных (на случай если файл был удалён).
            let active = restore_active.filter(|p| restored.contains(p));
            if let Some(p) = active {
                active_file.set(Some(p));
                editor_gen.update(|n| *n = n.wrapping_add(1));
            } else {
                // Fallback: первый восстановленный файл становится активным.
                if let Some(first) = restored.into_iter().next() {
                    active_file.set(Some(first));
                    editor_gen.update(|n| *n = n.wrapping_add(1));
                }
            }
        }
        // Сохранённое «показан» действует, только если есть что показывать:
        // сессия без восстановленных файлов стартует с одним терминалом.
        let editor_visible = use_signal(editor_visible_init && active_file.get_untracked().is_some());

        let session = Self {
            id,
            root_folder,
            tree_nodes,
            loaded_dirs,
            open_files,
            active_file,
            selected_node,
            file_contents,
            disk_contents,
            editor_gen,
            terminals,
            git_status,
            split_ratio,
            left_split_ratio,
            right_split_ratio,
            soft_wrap,
            editor_visible,
            editor_states,
            conflicts,
            created_at,
        };

        // Запустить FS-watcher и git-status worker, если папка валидна.
        // Watcher живёт в process-wide registry (`watcher_registry`) —
        // Mutex не реактивный. git-worker — параллельный registry.
        if let Some(path) = session.root_folder.get_untracked() {
            spawn_watcher(session, &path);
            spawn_git_worker(session, &path);
        }

        session
    }

    /// Человекочитаемое имя сессии для tooltip / accessible label.
    /// Папка → её basename; пустая сессия → fallback ("Сессия #N" вычисляется
    /// в sidebar по позиционному индексу).
    pub fn folder_name(&self) -> Option<String> {
        self.root_folder
            .get_untracked()
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CodeEditorCtx — менеджер сессий
// ─────────────────────────────────────────────────────────────────────────────

/// Глобальный реактивный контекст страницы «code» (менеджер сессий).
///
/// Все поля — `RwSignal<…>`, поэтому Copy. Создаётся один раз в
/// [`crate::run_desktop`] через `provide_context`.
#[derive(Clone, Copy)]
pub struct CodeEditorCtx {
    /// Список сессий в порядке отображения в sidebar (left-to-right /
    /// top-to-bottom). Vec НЕ Copy, но он внутри RwSignal — структура Copy.
    pub sessions: RwSignal<Vec<CodeSession>>,
    /// id активной сессии. `None` — список пуст или последняя только что закрыта.
    pub active_id: RwSignal<Option<SessionId>>,
    /// Монотонный счётчик id'ов. Никогда не сбрасывается, чтобы closure-биндинги
    /// после close оставались корректными.
    pub next_id: RwSignal<u64>,
    /// Bump-счётчик пересборки UI: triggered create/switch/close. Pages
    /// подписаны на него + `active_id` и пересоздают своё дерево виджетов.
    pub session_gen: RwSignal<u64>,
    /// Открытый модальный диалог (создание / переименование / удаление файла).
    /// `None` = диалог закрыт. Управляется через [`Self::open_dialog`] /
    /// [`Self::close_dialog`]. Portal в `code_editor::view()` подписан на
    /// `is_some()` для управления видимостью.
    pub pending_dialog: RwSignal<Option<DialogKind>>,
}

impl CodeEditorCtx {
    /// Создаёт менеджер из persist-данных. Если `sessions_cfg` пуст — список
    /// остаётся пустым (`active_id = None`); pages показывают no-session
    /// placeholder. `active_idx` валидируется по длине списка.
    pub fn new(sessions_cfg: Vec<CodeSessionConfig>, active_idx: Option<usize>) -> Self {
        let next_id = use_signal(1u64);

        let mut built: Vec<CodeSession> = Vec::with_capacity(sessions_cfg.len());
        let legacy_base = crate::config::now_millis().saturating_sub(sessions_cfg.len() as u64);
        for cfg in &sessions_cfg {
            let id = next_id.get_untracked();
            next_id.set(id.wrapping_add(1));
            let folder = cfg.root_folder.as_ref().map(PathBuf::from);
            let open: Vec<PathBuf> = cfg.open_files.iter().map(PathBuf::from).collect();
            let active = cfg.active_file.as_ref().map(PathBuf::from);
            // Резолвим persisted Option<f32> → конкретный f32. None
            // (миграция со старого конфига или конфиг до multi-session
            // splitter'ов) → fallback на хардкод-дефолт.
            let split_ratio = cfg
                .split_ratio
                .unwrap_or_else(default_code_editor_split_ratio);
            let left_split_ratio = cfg
                .left_split_ratio
                .unwrap_or_else(default_code_editor_left_split_ratio);
            let right_split_ratio = cfg
                .right_split_ratio
                .unwrap_or_else(default_code_editor_right_split_ratio);
            // Persisted позиции cursor/scroll по файлам. Конвертим
            // `EditorStateConfig` → `EditorPersistedState`. Ключ — путь как
            // PathBuf (config хранит как String → PathBuf::from).
            let editor_states: HashMap<PathBuf, EditorPersistedState> = cfg
                .editor_states
                .iter()
                .map(|(path, st)| {
                    (
                        PathBuf::from(path),
                        EditorPersistedState {
                            cursor_offset: st.cursor_offset,
                            scroll_lines: st.scroll_lines,
                            scroll_x: st.scroll_x,
                        },
                    )
                })
                .collect();
            // Конфиги до плиточного рейла не знают `created_at`: даём им
            // штампы по порядку списка, чтобы прежняя расстановка сохранилась.
            let created_at = cfg
                .created_at
                .unwrap_or_else(|| legacy_base + built.len() as u64);
            let session = CodeSession::new(
                id,
                folder,
                open,
                active,
                split_ratio,
                left_split_ratio,
                right_split_ratio,
                cfg.soft_wrap,
                cfg.editor_visible.unwrap_or(true),
                editor_states,
                created_at,
            );
            drafts::install_draft_autosave(session);
            install_editor_autohide(session);
            built.push(session);
        }

        let active = active_idx
            .filter(|&i| i < built.len())
            .map(|i| built[i].id);

        Self {
            sessions: use_signal(built),
            active_id: use_signal(active),
            next_id,
            session_gen: use_signal(0u64),
            pending_dialog: use_signal(None),
        }
    }

    /// Открыть модальный диалог. Перезаписывает предыдущий kind, если он
    /// был — обычно безопасно, диалоги монопольные (одновременно открыт
    /// один). UI рендерит соответствующий card в Portal.
    pub fn open_dialog(&self, kind: DialogKind) {
        self.pending_dialog.set(Some(kind));
    }

    /// Закрыть модальный диалог. Идемпотентно (no-op если уже закрыт).
    pub fn close_dialog(&self) {
        if self.pending_dialog.get_untracked().is_some() {
            self.pending_dialog.set(None);
        }
    }

    /// Показать info-уведомление через глобальный `AppCtx::notifications`.
    /// Раньше у CodeEditor был свой локальный snackbar; теперь все feedback'и
    /// идут через единый Notification-host (TopRight, 15s) — см. TASK.md.
    pub fn show_notice(&self, msg: impl Into<String>) {
        use syngui::context_provider::use_context;
        let app = use_context::<crate::context::AppCtx>();
        app.notifications.info(msg);
    }

    /// Реактивно (с подпиской) возвращает активную сессию, если есть.
    pub fn active_session(&self) -> Option<CodeSession> {
        let active = self.active_id.get()?;
        self.sessions.get().into_iter().find(|s| s.id == active)
    }

    /// Без подписки — для read из callback'ов и helper'ов.
    pub fn active_session_untracked(&self) -> Option<CodeSession> {
        let active = self.active_id.get_untracked()?;
        self.sessions
            .get_untracked()
            .into_iter()
            .find(|s| s.id == active)
    }

    /// Создаёт пустую сессию (без папки) и делает её активной. Возвращает id.
    pub fn create_empty(&self) -> SessionId {
        let id = self.next_id.get_untracked();
        self.next_id.set(id.wrapping_add(1));
        let session = CodeSession::new(
            id,
            None,
            Vec::new(),
            None,
            default_code_editor_split_ratio(),
            default_code_editor_left_split_ratio(),
            default_code_editor_right_split_ratio(),
            false, // soft_wrap default off — пользователь включает по желанию
            true,  // файлов нет — редактор всё равно стартует скрытым
            HashMap::new(),
            crate::config::now_millis(),
        );
        drafts::install_draft_autosave(session);
        install_editor_autohide(session);
        self.sessions.update(|v| v.push(session));
        self.active_id.set(Some(id));
        self.session_gen.update(|n| *n = n.wrapping_add(1));
        id
    }

    /// Переключает активную сессию. No-op если id не существует или уже активен.
    pub fn switch_to(&self, id: SessionId) {
        if self.active_id.get_untracked() == Some(id) {
            return;
        }
        let exists = self
            .sessions
            .get_untracked()
            .iter()
            .any(|s| s.id == id);
        if !exists {
            return;
        }
        self.active_id.set(Some(id));
        self.session_gen.update(|n| *n = n.wrapping_add(1));
    }

    /// Закрывает сессию по id. Если была активной — переключается на соседа
    /// (предыдущий, иначе следующий, иначе None). Drop удалённой `CodeSession`
    /// уносит её `Vec<TerminalTab>` → drop `TerminalSession` → kill PTY
    /// (см. `Drop for SessionShared` в syngui::widgets::Terminal).
    pub fn close(&self, id: SessionId) {
        let was_active = self.active_id.get_untracked() == Some(id);
        let new_active: Option<SessionId> = if was_active {
            let sessions = self.sessions.get_untracked();
            let idx = sessions.iter().position(|s| s.id == id);
            idx.and_then(|i| {
                if i > 0 {
                    sessions.get(i - 1).map(|s| s.id)
                } else {
                    sessions.get(i + 1).map(|s| s.id)
                }
            })
        } else {
            self.active_id.get_untracked()
        };

        self.sessions.update(|v| v.retain(|s| s.id != id));
        // Закрыли сессию — снимаем FS-watcher и git-worker, иначе они
        // продолжат держать inotify/FSEvents handle и thread'ы для
        // папки, к которой никто уже не подписан.
        drop_watcher(id);
        drop_git_worker(id);
        if was_active {
            self.active_id.set(new_active);
        }
        // Bump'аем session_gen всегда: даже если закрыли неактивную сессию,
        // sidebar должен пересобраться (исчезла иконка).
        self.session_gen.update(|n| *n = n.wrapping_add(1));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TerminalsState — таб-состояние нижней панели code editor'а
//
// Как в Zed/VSCode: список открытых терминалов с табами, переключаемых
// одним кликом. Каждый таб владеет своей [`TerminalSession`] (Arc inside),
// которая переживает route switch — RouterView дроп'ает Element'ы, но
// Arc на сессию остаётся здесь, и при возврате на страницу новый Element
// подхватывает существующий PTY/grid через [`syngui::widgets::Terminal::attach`].
// ─────────────────────────────────────────────────────────────────────────────

/// Один таб терминала. NOT Copy — содержит TerminalSession (Arc<…>).
#[derive(Clone)]
pub struct TerminalTab {
    /// Уникальный id (монотонный счётчик внутри сессии, не перевыдаётся при close).
    pub id: u32,
    /// Заголовок таба. Заполняется из OSC 0/2 терминала; pre-populate'ится
    /// дефолтом `"Terminal {id}"` для новых табов до первого title-sequence.
    pub title: RwSignal<String>,
    /// PTY-сессия. Дроп этого clone'а (через retain в close_terminal или
    /// при close сессии) — единственное место, которое финализирует процесс таба.
    pub session: TerminalSession,
}

/// Реактивная коллекция вкладок терминала. Copy через RwSignal-поля.
#[derive(Clone, Copy)]
pub struct TerminalsState {
    /// Vec табов в порядке отображения (left-to-right). Vec НЕ Copy, но он
    /// внутри RwSignal, так что вся структура Copy.
    pub tabs: RwSignal<Vec<TerminalTab>>,
    /// id активного таба (отображается в основной области терминала). None —
    /// нет ни одной вкладки (показываем empty placeholder).
    pub active_id: RwSignal<Option<u32>>,
    /// Монотонный счётчик id'ов терминалов внутри одной сессии.
    pub next_id: RwSignal<u32>,
    /// Число «занятых» терминалов сессии: вывод обновлялся в последние
    /// секунды (`revision()` менялся внутри окна активности — см.
    /// [`super::terminal_activity`], который пишет сюда раз в секунду).
    /// nav-rail по соотношению busy/total красит бейдж количества
    /// терминалов на плитке сессии.
    pub busy_count: RwSignal<usize>,
}

impl TerminalsState {
    fn new() -> Self {
        Self {
            tabs: use_signal(Vec::new()),
            active_id: use_signal(None),
            next_id: use_signal(1u32),
            busy_count: use_signal(0usize),
        }
    }
}

/// Создать новый таб терминала в указанной сессии. Сразу делает его активным.
/// Возвращает id нового таба.
///
/// cwd берётся из `session.root_folder` (корень открытого проекта). Если
/// folder не задан — session спавнит shell без cwd (наследуется от процесса).
/// font_size/family — снимок из `app.terminal_font_*`.
pub fn add_terminal(session: CodeSession, app: AppCtx) -> Option<u32> {
    let mut config = TerminalConfig::default();
    if let Some(folder) = session.root_folder.get_untracked() {
        config.cwd = Some(folder);
    }
    let family = app.terminal_font_family.get_untracked();
    if !family.is_empty() {
        config.font_family = family;
    }
    let size = app.terminal_font_size.get_untracked();
    if size > 0.0 {
        config.font_size = size;
    }

    let pty = match TerminalSession::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[code-editor] add_terminal: open session failed: {e}");
            return None;
        }
    };

    let id = next_terminal_id(&session.terminals);
    let title = use_signal(format!("Terminal {id}"));
    pty.set_title_signal(title);

    let tab = TerminalTab {
        id,
        title,
        session: pty,
    };
    session.terminals.tabs.update(|v| v.push(tab));
    session.terminals.active_id.set(Some(id));
    Some(id)
}

/// Закрыть таб по id в указанной сессии. Если был активным — переключиться на
/// соседа (предыдущий, иначе следующий, иначе None → empty placeholder).
pub fn close_terminal(session: CodeSession, id: u32) {
    let was_active = session.terminals.active_id.get_untracked() == Some(id);
    let new_active = if was_active {
        let tabs = session.terminals.tabs.get_untracked();
        let idx = tabs.iter().position(|t| t.id == id);
        idx.and_then(|i| {
            if i > 0 {
                tabs.get(i - 1).map(|t| t.id)
            } else {
                tabs.get(i + 1).map(|t| t.id)
            }
        })
    } else {
        session.terminals.active_id.get_untracked()
    };
    session.terminals.tabs.update(|v| v.retain(|t| t.id != id));
    if was_active {
        session.terminals.active_id.set(new_active);
    }
}

/// Низкоуровневый write байт в pty активного таба активной сессии. Возвращает
/// `false`, если активной сессии/таба нет или сессия мертва. Используется
/// общим helper'ом для voice-paste (с auto-newline) и drag&drop (без него).
fn write_bytes_to_active_terminal(bytes: &[u8]) -> bool {
    use syngui::context_provider::use_context;
    let mgr = use_context::<CodeEditorCtx>();
    let Some(session) = mgr.active_session_untracked() else {
        return false;
    };
    let Some(active_id) = session.terminals.active_id.get_untracked() else {
        return false;
    };
    let tabs = session.terminals.tabs.get_untracked();
    let Some(tab) = tabs.iter().find(|t| t.id == active_id).cloned() else {
        return false;
    };
    if !tab.session.is_alive() {
        return false;
    }
    tab.session.write(bytes);
    true
}

/// Записать строку в pty stdin активного таба активной сессии. Возвращает
/// `false`, если активной сессии/таба нет или сессия мертва — вызывающий
/// должен сделать fallback (см. voice_fab::actions::paste_to_target).
///
/// Для удобства voice-пасты добавляем `\n` в конце, чтобы строка сразу была
/// «введена» в shell — пользователь редко ожидает, что распознанный текст
/// останется висеть в prompt без enter.
pub fn write_to_active_terminal(text: &str) -> bool {
    let mut buf = text.as_bytes().to_vec();
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    write_bytes_to_active_terminal(&buf)
}

/// Вставить текст в активный pty без auto-newline. Используется для drag&drop
/// файлов: пользователь сам решит, нажимать Enter или продолжать редактировать
/// (например, добавить `cd ` перед путём).
pub fn paste_to_active_terminal(text: &str) -> bool {
    write_bytes_to_active_terminal(text.as_bytes())
}

/// POSIX-shell-quote: возвращает строку, безопасную для вставки одним
/// аргументом в bash/zsh/sh-prompt. Без специальных символов — as-is,
/// иначе — одинарные кавычки с escape `'` через `'\''`. Эта форма
/// работает в любом POSIX-shell без зависимости от quoting-режима.
///
/// Применяется к путям файлов, перетянутых через OS DnD: путь может
/// содержать пробелы и спецсимволы, и без quoting-а shell разобьёт его
/// на несколько аргументов.
pub fn shell_quote_posix(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s.chars().all(|c| {
        matches!(
            c,
            'a'..='z' | 'A'..='Z' | '0'..='9'
                | '_' | '-' | '.' | '/' | ':' | ',' | '=' | '+' | '@' | '%'
        )
    });
    if safe {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Сменить активный таб в указанной сессии. No-op если такого id нет.
///
/// Дополнительно сбрасываем `autofocus_consumed` на target session — это
/// явный user-action «хочу этот терминал», и после set_active_terminal
/// Reactive в `active_terminal_inner` пересоберётся с новой PTY-session;
/// без reset autofocus уже был бы consumed (предыдущий switch / создание),
/// и фокус остался бы там, где был (например, в CodeEditor).
pub fn set_active_terminal(session: CodeSession, id: u32) {
    let tabs = session.terminals.tabs.get_untracked();
    let Some(target) = tabs.iter().find(|t| t.id == id) else {
        return;
    };
    target.session.reset_autofocus();
    session.terminals.active_id.set(Some(id));
}

fn next_terminal_id(state: &TerminalsState) -> u32 {
    let id = state.next_id.get_untracked();
    state.next_id.set(id.wrapping_add(1));
    id
}

/// Текущий файл считается dirty, если его буфер отличается от
/// последнего записанного на диск снимка.
pub fn is_dirty(
    contents: &HashMap<PathBuf, String>,
    disk: &HashMap<PathBuf, String>,
    path: &PathBuf,
) -> bool {
    contents.get(path) != disk.get(path)
}

// ─────────────────────────────────────────────────────────────────────────────
// Действия над сессией
//
// Свободные функции принимают `CodeSession` (Copy) первым аргументом, чтобы
// вызывающий мог передать активную сессию без borrow self.
// ─────────────────────────────────────────────────────────────────────────────

/// Сменить корневую папку сессии. Подгружает топ-уровень дерева, сбрасывает
/// все открытые файлы и буферы (мы переключаемся между разными проектами).
/// Перезапускает FS-watcher на новый корень — старый watcher дроп'ается
/// при удалении из registry.
pub fn set_root_folder(session: CodeSession, path: PathBuf) {
    info!(
        target: "code-editor.state",
        session_id = session.id,
        root = %path.display(),
        "set_root_folder"
    );
    drop_watcher(session.id);
    drop_git_worker(session.id);

    let nodes = fs_ops::read_dir_to_nodes(&path);
    let initial = nodes.len();
    session.tree_nodes.update(|v| *v = nodes);
    session.loaded_dirs.update(|s| {
        s.clear();
        s.insert(path.clone());
    });
    session.open_files.set(Vec::new());
    session.active_file.set(None);
    session.selected_node.set(None);
    session.file_contents.set(HashMap::new());
    session.disk_contents.set(HashMap::new());
    session.editor_gen.update(|n| *n = n.wrapping_add(1));
    // Сбросить git-status в пустую карту — пока новый воркер не дотянется,
    // декорации старого проекта не должны висеть на новых файлах.
    session.git_status.set(Arc::new(GitStatusMap::empty()));
    session.root_folder.set(Some(path.clone()));

    debug!(
        target: "code-editor.state",
        session_id = session.id,
        nodes = initial,
        "root tree filled, spawning workers"
    );
    spawn_watcher(session, &path);
    spawn_git_worker(session, &path);
}

/// Запустить FS-watcher на `path` и положить его в process-wide registry.
/// Failure — лог и тихий fallback (UI продолжает работать без авто-обновлений).
fn spawn_watcher(session: CodeSession, path: &std::path::Path) {
    if !path.is_dir() {
        return;
    }
    match FsWatcher::start(session, path.to_path_buf()) {
        Ok(w) => {
            install_watcher(session.id, w);
        }
        Err(e) => {
            warn!(
                target: "code-editor.state",
                path = %path.display(),
                error = %e,
                "fs_watcher: failed to start"
            );
        }
    }
}

/// Запустить git-status воркер на `path` и положить его в registry.
/// Воркер всегда стартует — даже если папка не git-репо, он молча будет
/// возвращать пустые карты (стоимость — один спящий thread на сессию).
/// При первом запуске сразу дёргаем `request_refresh`, чтобы дерево
/// получило декорации без ожидания событий fs_watcher.
fn spawn_git_worker(session: CodeSession, path: &std::path::Path) {
    if !path.is_dir() {
        return;
    }
    let worker = GitStatusWorker::start(session, path.to_path_buf());
    worker.request_refresh();
    install_git_worker(session.id, worker);
}

/// Раскрыть/свернуть папку. На первом раскрытии — лениво подгружает
/// содержимое и кладёт в `loaded_dirs`. Вызывается из обоих TreeView-
/// callback'ов: `on_toggle` (клик по чеврону) и `on_select` (клик по
/// label папки — VSCode-like UX).
pub fn toggle_dir(session: CodeSession, path: PathBuf) {
    let id = fs_ops::id_for(&path);
    let cur_expanded = {
        let nodes = session.tree_nodes.get_untracked();
        fs_ops::find_node(&nodes, &id)
            .map(|n| n.expanded)
            .unwrap_or(false)
    };
    let need_load = !session.loaded_dirs.get_untracked().contains(&path);

    session.tree_nodes.update(|nodes| {
        if need_load {
            let kids = fs_ops::read_dir_to_nodes(&path);
            let mut taken = Some(kids);
            fs_ops::replace_children_and_expand(nodes, &id, &mut taken);
        } else {
            fs_ops::set_node_expanded(nodes, &id, !cur_expanded);
        }
    });

    if need_load {
        session.loaded_dirs.update(|s| {
            s.insert(path);
        });
    }
}

/// Открыть файл — добавить в `open_files` (если ещё не открыт), сделать
/// активным и пересоздать редактор. Ошибки чтения логируются, состояние
/// не меняется.
///
/// Защита: файлы крупнее [`MAX_OPEN_FILE_BYTES`] и бинарные (UTF-8 invalid)
/// не открываются — пользователь получает snackbar-уведомление
/// (см. [`CodeEditorCtx::show_notice`]) и `eprintln!` в лог. Это
/// предотвращает блокировку UI на чтении многосот-мегабайтных файлов
/// и не съедает память на 1GB-логах.
pub fn open_file(session: CodeSession, path: PathBuf) {
    use syngui::context_provider::use_context;

    if !session.open_files.get_untracked().contains(&path) {
        // Гард: размер файла. Файлы > MAX_OPEN_FILE_BYTES — отбрасываем,
        // не пытаясь даже открыть (read_to_string выделил бы Vec на
        // весь размер до проверки UTF-8 — слишком большой риск OOM).
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX_OPEN_FILE_BYTES => {
                let mb = meta.len() as f64 / (1024.0 * 1024.0);
                let limit_mb = MAX_OPEN_FILE_BYTES as f64 / (1024.0 * 1024.0);
                eprintln!(
                    "[code-editor] open_file: файл слишком большой {:?} ({:.1} MB, лимит {:.0} MB)",
                    path, mb, limit_mb
                );
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let ctx = use_context::<CodeEditorCtx>();
                ctx.show_notice(tr!(
                    "code.session.notice.file_too_large",
                    name = name,
                    mb = format!("{mb:.1}"),
                    limit = format!("{limit_mb:.0}")
                ));
                return;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[code-editor] open_file metadata {:?}: {e}", path);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let ctx = use_context::<CodeEditorCtx>();
                ctx.show_notice(tr!("code.session.notice.open_failed", name = name, error = e));
                return;
            }
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                session.file_contents.update(|m| {
                    m.insert(path.clone(), content.clone());
                });
                session.disk_contents.update(|m| {
                    m.insert(path.clone(), content);
                });
                session.open_files.update(|v| v.push(path.clone()));
            }
            Err(e) => {
                // read_to_string падает на не-UTF-8 → InvalidData. Пользователь
                // часто кликает в дереве на бинарник «случайно» — нужен
                // понятный диагноз, а не молчаливый отказ.
                eprintln!("[code-editor] open_file read {:?}: {e}", path);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let kind_msg = if e.kind() == std::io::ErrorKind::InvalidData {
                    tr!("code.session.error.not_text_file")
                } else {
                    e.to_string()
                };
                let ctx = use_context::<CodeEditorCtx>();
                ctx.show_notice(tr!("code.session.notice.open_failed", name = name, error = kind_msg));
                return;
            }
        }
    }
    activate_file(session, path);
}

/// Сделать открытый файл активным и показать редактор. Клик по файлу — явная
/// просьба его увидеть, поэтому редактор, скрытый кнопкой в шапке,
/// возвращается и для файла, который уже активен.
pub fn activate_file(session: CodeSession, path: PathBuf) {
    if session.active_file.get_untracked().as_ref() != Some(&path) {
        session.active_file.set(Some(path));
        session.editor_gen.update(|n| *n = n.wrapping_add(1));
    }
    if !session.editor_visible.get_untracked() {
        session.editor_visible.set(true);
    }
}

/// Редактор без файла не занимает место: когда активный файл пропадает
/// (закрыли последний, сменили папку сессии), редактор скрывается и терминал
/// получает весь центр. Смотрим только на переход «был файл → нет файла»:
/// переименование меняет путь, но не должно показывать скрытый вручную
/// редактор, а показ при открытии делает [`activate_file`].
pub fn install_editor_autohide(session: CodeSession) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let had_file = Arc::new(AtomicBool::new(session.active_file.get_untracked().is_some()));
    create_effect(move || {
        let has_file = session.active_file.get().is_some();
        let lost_file = had_file.swap(has_file, Ordering::Relaxed) && !has_file;
        if lost_file && session.editor_visible.get_untracked() {
            session.editor_visible.set(false);
        }
    });
}

/// Сохранить активный файл сессии на диск.
pub fn save_active(session: CodeSession) {
    let Some(path) = session.active_file.get_untracked() else {
        return;
    };
    let text = match session.file_contents.get_untracked().get(&path).cloned() {
        Some(t) => t,
        None => return,
    };
    if let Err(e) = crate::fsutil::write_user_file(&path, &text) {
        // Черновик остаётся (текст не потерян), но пользователь должен
        // знать, что Ctrl+S не сработал.
        eprintln!("[code-editor] save {:?}: {e}", path);
        syngui::context_provider::use_context::<crate::context::AppCtx>()
            .notifications
            .error(tr!(
                "code.save.error",
                path = crate::paths::pretty(&path),
                error = e.to_string()
            ));
        return;
    }
    drafts::snapshot_history(&path, &text);
    drafts::clear_draft(&path);
    session.conflicts.update(|v| v.retain(|p| p != &path));
    session.disk_contents.update(|m| {
        m.insert(path, text);
    });
}

/// Закрыть файл в сессии. Если был активным — переключиться на соседа
/// (предыдущий, потом следующий, иначе `None`).
pub fn close_file(session: CodeSession, path: PathBuf) {
    let was_active = session.active_file.get_untracked().as_ref() == Some(&path);
    let new_active: Option<PathBuf> = if was_active {
        let files = session.open_files.get_untracked();
        let idx = files.iter().position(|p| p == &path);
        idx.and_then(|i| {
            if i > 0 {
                files.get(i - 1).cloned()
            } else {
                files.get(i + 1).cloned()
            }
        })
        .filter(|p| p != &path)
    } else {
        None
    };

    let was_dirty = is_dirty(
        &session.file_contents.get_untracked(),
        &session.disk_contents.get_untracked(),
        &path,
    );

    session.open_files.update(|v| v.retain(|p| p != &path));
    session.file_contents.update(|m| {
        m.remove(&path);
    });
    session.disk_contents.update(|m| {
        m.remove(&path);
    });
    session.conflicts.update(|v| v.retain(|p| p != &path));
    if !was_dirty {
        drafts::clear_draft(&path);
    }

    if was_active {
        session.active_file.set(new_active);
        session.editor_gen.update(|n| *n = n.wrapping_add(1));
    }
}

/// Подменить текст активного файла из `on_change` редактора. НЕ
/// инкрементирует `editor_gen` — иначе курсор будет сбрасываться при
/// каждом нажатии клавиши.
pub fn update_active_text(session: CodeSession, text: &str) {
    let Some(path) = session.active_file.get_untracked() else {
        return;
    };
    session.file_contents.update(|m| {
        m.insert(path, text.to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "synthos-editor-autohide-{}-{tag}.txt",
            std::process::id()
        ));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        path
    }

    /// `set` только ставит эффекты в очередь; в приложении её разбирает цикл
    /// кадра, здесь — вручную после каждого шага.
    fn frame() {
        syngui::signal::drain_and_run_effects();
    }

    #[test]
    fn editor_follows_active_file_and_manual_toggle() {
        let ctx = CodeEditorCtx::new(Vec::new(), None);
        ctx.create_empty();
        frame();
        let session = ctx.active_session_untracked().unwrap();
        assert!(!session.editor_visible.get_untracked(), "без файла редактор скрыт");

        let a = temp_file("a");
        let b = temp_file("b");
        open_file(session, a.clone());
        frame();
        assert!(session.editor_visible.get_untracked(), "открытие файла показывает редактор");

        session.editor_visible.set(false);
        frame();
        open_file(session, a.clone());
        frame();
        assert!(
            session.editor_visible.get_untracked(),
            "повторный клик по активному файлу возвращает скрытый редактор"
        );

        open_file(session, b.clone());
        frame();
        close_file(session, b.clone());
        frame();
        assert!(
            session.editor_visible.get_untracked(),
            "закрытие не последнего файла редактор не прячет"
        );

        close_file(session, a.clone());
        frame();
        assert!(
            !session.editor_visible.get_untracked(),
            "закрыли последний файл — редактор скрыт"
        );

        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    fn restored_visibility_needs_a_file() {
        let file = temp_file("restore");
        let cfg = |open: Vec<String>, visible: Option<bool>| CodeSessionConfig {
            root_folder: None,
            open_files: open.clone(),
            active_file: open.first().cloned(),
            split_ratio: None,
            left_split_ratio: None,
            right_split_ratio: None,
            soft_wrap: false,
            editor_visible: visible,
            editor_states: HashMap::new(),
            created_at: None,
        };
        let path = file.display().to_string();
        let ctx = CodeEditorCtx::new(
            vec![
                cfg(vec![path.clone()], None),
                cfg(vec![path.clone()], Some(false)),
                cfg(Vec::new(), Some(true)),
            ],
            Some(0),
        );
        let visible: Vec<bool> = ctx
            .sessions
            .get_untracked()
            .iter()
            .map(|s| s.editor_visible.get_untracked())
            .collect();
        assert_eq!(visible, vec![true, false, false]);

        let _ = std::fs::remove_file(file);
    }
}
