//! EditorWorkspace — контекст открытых графов редактора нод.
//!
//! Каждый открытый граф («вкладка», исторически) владеет собственным
//! [`NodeEditorCtx`] (граф, pan/zoom, menu-state). Полосы вкладок больше
//! нет: графы показываются плитками в нав-рейле вперемешку с code-сессиями
//! и чатами (см. `crate::rail`). Workspace оркестрирует список открытых
//! графов и активный; здесь же живёт глобальный `RunState` (Run/Pause/Stop)
//! и флаг окна выбора шаблонов.
//!
//! Чистые helpers (поиск/имя «Untitled N») вынесены отдельно для тестов.

use syngui::core::Point;
use syngui::prelude::*;

use super::persist::{self, TabState, WorkspaceState};
use super::state::NodeEditorCtx;
use super::timing::Stopwatch;
use crate::templates::Template;
use crate::templates::convert::{apply_to_ctx, load_into_ctx};
use crate::templates::model::TemplateKind;

/// Идентификатор вкладки. Монотонный, не реюзается даже при закрытии.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

#[derive(Clone, Copy)]
pub struct OpenTab {
    pub id: TabId,
    pub title: RwSignal<String>,
    pub source: RwSignal<Option<String>>,
    pub dirty: RwSignal<bool>,
    pub last_saved_fp: RwSignal<u64>,
    /// `Some(chat_id)` — служебная вкладка агента Syn-чата: граф в ней
    /// собирает и запускает инструмент `pipelines`, по одной на чат.
    pub agent_chat: RwSignal<Option<String>>,
    /// Скрыта из рейла. Агентские графы рождаются скрытыми и
    /// показываются переходом из чата ([`EditorWorkspace::reveal`]).
    pub hidden: RwSignal<bool>,
    /// Unix-миллисекунды появления графа в рейле — порядок плитки среди
    /// code-сессий и чатов. Сигнал, а не число: `reveal` переставляет
    /// агентский граф в конец, как только что созданный.
    pub created_at: RwSignal<u64>,
    pub ctx: NodeEditorCtx,
}

impl PartialEq for OpenTab {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Состояние выполнения графа. Сейчас декоративно (UI Run/Pause/Stop в
/// правом верхнем углу canvas). При появлении time-based нод в `eval.rs`
/// будут гейты по `Stopped`/`Paused`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RunState {
    #[default]
    Stopped,
    Running,
    Paused,
}

impl RunState {
    pub fn is_running(self) -> bool {
        matches!(self, RunState::Running)
    }
    pub fn label(self) -> &'static str {
        match self {
            RunState::Stopped => "Stopped",
            RunState::Running => "Running",
            RunState::Paused => "Paused",
        }
    }
}

/// Workspace — provided через `provide_context` один раз при старте app.
#[derive(Clone, Copy)]
pub struct EditorWorkspace {
    /// Список открытых вкладок.
    pub tabs: RwSignal<Vec<OpenTab>>,
    /// id активного графа. `None` — открытых графов нет (или остались одни
    /// скрытые агентские): страница нод показывает заглушку «нажмите +».
    pub active: RwSignal<Option<TabId>>,
    /// Run/Pause/Stop pill справа сверху на canvas.
    pub run_state: RwSignal<RunState>,
    /// Секундомер всего прогона: стартует по Run, финиширует когда очередь
    /// опустела (или по Stop). Показывается в Run-pill рядом с кнопками.
    pub run_timer: Stopwatch,
    /// Сколько on_run-нод текущего прогона уже завершились.
    pub run_done: RwSignal<usize>,
    /// Сколько on_run-нод всего в текущем прогоне. `0` — прогонов не было.
    pub run_total: RwSignal<usize>,
    /// Открыто ли всплывающее окно выбора шаблонов (см.
    /// [`crate::components::template_picker`]). Всегда стартует закрытым —
    /// не персистится.
    pub template_picker_open: RwSignal<bool>,
    /// Auto-increment id для новых вкладок.
    pub next_tab_id: RwSignal<u64>,
    /// Граф, для которого открыт диалог «закрыть с несохранёнными
    /// изменениями» (см. `components::graph_close_dialog`). `None` — закрыт.
    pub pending_close: RwSignal<Option<TabId>>,
}

impl EditorWorkspace {
    pub fn new() -> Self {
        let ws = Self {
            tabs: use_signal(Vec::<OpenTab>::new()),
            active: use_signal(None::<TabId>),
            run_state: use_signal(RunState::default()),
            run_timer: Stopwatch::new(),
            run_done: use_signal(0_usize),
            run_total: use_signal(0_usize),
            template_picker_open: use_signal(false),
            next_tab_id: use_signal(1_u64),
            pending_close: use_signal(None),
        };
        // Стартовой Untitled-вкладки больше нет: графы живут плитками в
        // рейле, и пустой рейл — нормальное состояние (заглушка «нажмите +»).
        ws
    }

    /// Сконструировать workspace, восстановив сохранённые графы из
    /// `~/.config/synthos/workspace.json`. При отсутствии файла или его
    /// порче — пустой workspace (поведение [`Self::new`]).
    pub fn new_or_restore() -> Self {
        let Some(state) = persist::load() else {
            return Self::new();
        };
        Self::from_state(state)
    }

    /// Построить workspace по уже загруженному [`WorkspaceState`]. Пустой
    /// список графов остаётся пустым — рейл покажет только «+».
    pub fn from_state(state: WorkspaceState) -> Self {
        let ws = Self {
            tabs: use_signal(Vec::<OpenTab>::new()),
            active: use_signal(None::<TabId>),
            run_state: use_signal(RunState::default()),
            run_timer: Stopwatch::new(),
            run_done: use_signal(0_usize),
            run_total: use_signal(0_usize),
            template_picker_open: use_signal(false),
            next_tab_id: use_signal(state.next_tab_id.max(1)),
            pending_close: use_signal(None),
        };
        if state.tabs.is_empty() {
            return ws;
        }

        // Файлы до плиточного рейла не знают `created_at`: таким графам
        // даём штампы по порядку списка, чтобы прежняя расстановка
        // сохранилась и они встали раньше всего, что создадут сегодня.
        let legacy_base =
            crate::config::now_millis().saturating_sub(state.tabs.len() as u64);
        let mut tabs = Vec::with_capacity(state.tabs.len());
        for (idx, ts) in state.tabs.iter().enumerate() {
            let created_at = if ts.created_at == 0 {
                legacy_base + idx as u64
            } else {
                ts.created_at
            };
            tabs.push(make_tab_from_state(ts, created_at));
        }
        // next_tab_id должен быть больше максимального восстановленного id —
        // иначе новые `new_untitled` будут конфликтовать.
        let max_id = tabs.iter().map(|t| t.id.0).max().unwrap_or(0);
        let next = ws.next_tab_id.get_untracked().max(max_id + 1);
        ws.next_tab_id.set(next);

        // Activate: prefer сохранённую active-id (если она видимая), иначе
        // первая видимая вкладка. Скрытую агентскую активной не делаем.
        let active = state
            .active
            .and_then(|id| {
                tabs.iter()
                    .find(|t| t.id.0 == id && !t.hidden.get_untracked())
                    .map(|t| t.id)
            })
            .or_else(|| {
                tabs.iter()
                    .find(|t| !t.hidden.get_untracked())
                    .map(|t| t.id)
            });

        ws.tabs.set(tabs);
        ws.active.set(active);
        ws
    }

    /// Активный `NodeEditorCtx` (если есть открытая вкладка).
    pub fn active_ctx(&self) -> Option<NodeEditorCtx> {
        let id = self.active.get()?;
        let tabs = self.tabs.get();
        tabs.iter().find(|t| t.id == id).map(|t| t.ctx)
    }

    /// Активный ctx без подписки — для event-handler'ов.
    pub fn active_ctx_untracked(&self) -> Option<NodeEditorCtx> {
        let id = self.active.get_untracked()?;
        let tabs = self.tabs.get_untracked();
        tabs.iter().find(|t| t.id == id).map(|t| t.ctx)
    }

    /// Создать пустую `Untitled`-вкладку и сделать её активной.
    pub fn new_untitled(&self) -> TabId {
        let mut tabs = self.tabs.get_untracked();
        let title = next_untitled_name(&tabs);
        let id_n = self.next_tab_id.get_untracked();
        self.next_tab_id.set(id_n + 1);
        let id = TabId(id_n);
        let tab = OpenTab {
            id,
            title: use_signal(title),
            source: use_signal(None),
            dirty: use_signal(false),
            last_saved_fp: use_signal(0),
            agent_chat: use_signal(None),
            hidden: use_signal(false),
            created_at: use_signal(crate::config::now_millis()),
            ctx: NodeEditorCtx::new(),
        };
        persist::install_dirty_for_tab(tab);
        tabs.push(tab);
        self.tabs.set(tabs);
        self.active.set(Some(id));
        id
    }

    /// Служебная вкладка агента для чата `chat_id`: найти существующую или
    /// создать новую — скрытую, с пустым графом и БЕЗ смены активной
    /// вкладки (пользователь не должен терять фокус, пока агент работает).
    pub fn ensure_agent_tab(&self, chat_id: &str, title: &str) -> TabId {
        if let Some(id) = self.agent_tab_for_chat(chat_id) {
            return id;
        }
        let id_n = self.next_tab_id.get_untracked();
        self.next_tab_id.set(id_n + 1);
        let id = TabId(id_n);
        let ctx = NodeEditorCtx::new();
        // NodeEditorCtx::new() добавляет стартовую demo-ноду — агенту
        // нужен чистый граф.
        ctx.nodes.set(Vec::new());
        ctx.connections.set(Vec::new());
        let tab = OpenTab {
            id,
            title: use_signal(title.to_string()),
            source: use_signal(None),
            dirty: use_signal(false),
            last_saved_fp: use_signal(0),
            agent_chat: use_signal(Some(chat_id.to_string())),
            hidden: use_signal(true),
            created_at: use_signal(crate::config::now_millis()),
            ctx,
        };
        persist::install_dirty_for_tab(tab);
        let mut tabs = self.tabs.get_untracked();
        tabs.push(tab);
        self.tabs.set(tabs);
        id
    }

    /// Найти служебную вкладку агента для чата.
    pub fn agent_tab_for_chat(&self, chat_id: &str) -> Option<TabId> {
        self.tabs
            .get_untracked()
            .iter()
            .find(|t| t.agent_chat.get_untracked().as_deref() == Some(chat_id))
            .map(|t| t.id)
    }

    /// Показать скрытый граф плиткой в рейле и активировать — переход по
    /// ссылке «открыть граф» из чата. Плитка встаёт в конец рейла, как
    /// только что созданная: пользователь ищет её глазами именно там.
    pub fn reveal(&self, id: TabId) {
        let tabs = self.tabs.get_untracked();
        if let Some(t) = tabs.iter().find(|t| t.id == id) {
            if t.hidden.get_untracked() {
                t.hidden.set(false);
                t.created_at.set(crate::config::now_millis());
            }
        }
        self.activate(id);
    }

    /// Открыть шаблон новым графом. Всегда новая копия — даже если этот
    /// шаблон уже открыт в другой плитке: выбор в окне шаблонов означает
    /// «хочу ещё один такой», а не «покажи тот». Повторам даётся суффикс
    /// «(2)», «(3)», чтобы плитки различались подписью.
    pub fn open_template(&self, t: &Template) -> TabId {
        let tabs_now = self.tabs.get_untracked();
        let title = unique_title(&tabs_now, &crate::i18n::template_name(t));

        let id_n = self.next_tab_id.get_untracked();
        self.next_tab_id.set(id_n + 1);
        let id = TabId(id_n);
        let ctx = NodeEditorCtx::new();
        ctx.nodes.set(Vec::new());
        ctx.connections.set(Vec::new());
        load_into_ctx(&ctx, t);

        let tab = OpenTab {
            id,
            title: use_signal(title),
            source: use_signal(Some(t.id.clone())),
            dirty: use_signal(false),
            last_saved_fp: use_signal(0),
            agent_chat: use_signal(None),
            hidden: use_signal(false),
            created_at: use_signal(crate::config::now_millis()),
            ctx,
        };
        persist::install_dirty_for_tab(tab);
        let mut tabs = self.tabs.get_untracked();
        tabs.push(tab);
        self.tabs.set(tabs);
        self.active.set(Some(id));
        // Выбор шаблона в окне = открыть его и закрыть само окно.
        self.template_picker_open.set(false);
        id
    }

    /// Закрыть граф немедленно, без вопросов. Если закрыли активный —
    /// активным становится ближайший ВИДИМЫЙ слева (скрытые агентские
    /// графы пользователю не подсовываем); если видимых не осталось —
    /// `None`, и страница показывает заглушку «нажмите +».
    pub fn close(&self, id: TabId) {
        let mut tabs = self.tabs.get_untracked();
        let idx = match tabs.iter().position(|t| t.id == id) {
            Some(i) => i,
            None => return,
        };
        tabs.remove(idx);
        if self.pending_close.get_untracked() == Some(id) {
            self.pending_close.set(None);
        }
        if self.active.get_untracked() == Some(id) {
            let next_id = tabs[..idx.min(tabs.len())]
                .iter()
                .rev()
                .find(|t| !t.hidden.get_untracked())
                .or_else(|| tabs.iter().find(|t| !t.hidden.get_untracked()))
                .map(|t| t.id);
            self.active.set(next_id);
        }
        self.tabs.set(tabs);
    }

    /// Закрыть граф из рейла: с несохранёнными изменениями — сначала
    /// диалог (`pending_close`), чистый — сразу.
    pub fn request_close(&self, id: TabId) {
        let dirty = self
            .tabs
            .get_untracked()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.dirty.get_untracked())
            .unwrap_or(false);
        if dirty {
            self.pending_close.set(Some(id));
        } else {
            self.close(id);
        }
    }

    /// Вкладка по id.
    pub fn tab(&self, id: TabId) -> Option<OpenTab> {
        self.tabs.get_untracked().into_iter().find(|t| t.id == id)
    }

    /// Сделать вкладку активной (без перестройки списка).
    pub fn activate(&self, id: TabId) {
        // Только если реально другая — иначе лишний effect-trigger.
        if self.active.get_untracked() != Some(id) {
            self.active.set(Some(id));
        }
    }
}

impl Default for EditorWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure helpers (тестируемые без runtime)
// ─────────────────────────────────────────────────────────────────────────────

/// Найти id первой открытой вкладки, у которой `source == template_id`.
pub fn find_tab_by_template(tabs: &[OpenTab], template_id: &str) -> Option<TabId> {
    tabs.iter()
        .find(|t| t.source.get_untracked().as_deref() == Some(template_id))
        .map(|t| t.id)
}

/// Построить `OpenTab` из персистированного состояния. Создаёт fresh
/// `NodeEditorCtx`, очищает стартовую ноду (NodeEditorCtx::new
/// добавляет одну для первого UI-впечатления), применяет нод и связи
/// через `apply_to_ctx` — это же путь, которым грузятся шаблоны, поэтому
/// все per-kind `NodeStateData` корректно проставляются в свежий runtime.
fn make_tab_from_state(ts: &TabState, created_at: u64) -> OpenTab {
    let ctx = NodeEditorCtx::new();
    ctx.nodes.set(Vec::new());
    ctx.connections.set(Vec::new());

    // Шаблон-обёртка нужна потому что `apply_to_ctx` принимает `Template`.
    // `kind` тут чисто номинальное — apply пользуется только `nodes/connections`.
    let template = Template {
        id: String::new(),
        builtin: false,
        name: ts.title.clone(),
        description: String::new(),
        kind: TemplateKind::Full,
        nodes: ts.nodes.clone(),
        connections: ts.connections.clone(),
        viewport: ts.viewport,
    };
    apply_to_ctx(&ctx, &template, Point::new(0.0, 0.0));
    if let Some(v) = ts.viewport {
        ctx.pan.set(v.pan.into());
        ctx.zoom.set(v.zoom);
    }

    let tab = OpenTab {
        id: TabId(ts.id),
        title: use_signal(ts.title.clone()),
        source: use_signal(ts.source.clone()),
        dirty: use_signal(false),
        last_saved_fp: use_signal(0),
        agent_chat: use_signal(ts.agent_chat.clone()),
        hidden: use_signal(ts.hidden),
        created_at: use_signal(created_at),
        ctx,
    };
    persist::install_dirty_for_tab(tab);
    tab
}

/// Уникальная подпись для новой копии шаблона: «Имя», «Имя (2)», «Имя (3)»…
pub fn unique_title(tabs: &[OpenTab], base: &str) -> String {
    let titles: std::collections::HashSet<String> =
        tabs.iter().map(|t| t.title.get_untracked()).collect();
    if !titles.contains(base) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base} ({n})");
        if !titles.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Подобрать имя следующей `Untitled`-вкладки. Считает занятые номера,
/// возвращает первую свободную: «Untitled», «Untitled 2», «Untitled 3», …
pub fn next_untitled_name(tabs: &[OpenTab]) -> String {
    let titles: std::collections::HashSet<String> =
        tabs.iter().map(|t| t.title.get_untracked()).collect();
    let base = tr!("nodes.tabs.untitled");
    if !titles.contains(&base) {
        return base;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base} {n}");
        if !titles.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}
