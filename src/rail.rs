//! Единый список плиток нав-рейла.
//!
//! Раньше рейл показывал только code-сессии, а графы нодового редактора
//! жили во вкладках на своей странице, чаты — в колонке слева от ленты.
//! Теперь всё это — плитки одного списка: code-сессия (папка), граф
//! (хаб), чат (аватар) и разделитель, добавленный пользователем через «+».
//! Порядок — по времени появления (`created_at`), вперемешку по типам:
//! новое встаёт в конец, как вкладка в браузере. Перетаскиванием плитки
//! можно переставить (`move_before` / `move_to_end`) — тогда порядок
//! фиксируется списком ключей `AppCtx.rail_order`, а разделители позволяют
//! группировать плитки как удобно.
//!
//! Группы (`AppCtx.rail_groups`): в рейле группа — одна плитка со своей
//! иконкой и названием, её плитки показывает выезжающая панель
//! (`components::nav_rail::group_flyout`). Плитка в группу попадает
//! перетаскиванием на плитку группы или в её панель, либо из меню
//! «В группу». Порядок плиток группы — её список ключей `members`.
//!
//! Источники данных остаются на своих местах (`CodeEditorCtx`,
//! `EditorWorkspace`, `SynChatCtx`, `AppCtx.rail_separators`) — модуль лишь
//! собирает их в один отсортированный список и знает, что значит «открыть»
//! и «закрыть» плитку каждого типа.

use std::path::PathBuf;

use syngui::prelude::*;

use crate::agent::state::ChatMeta;
use crate::config::RailGroupConfig;
use crate::context::{AppCtx, RailGroupEdit};
use crate::pages::code_editor::state::{CodeEditorCtx, CodeSession};
use crate::pages::node_editor::tabs::{EditorWorkspace, OpenTab};
use crate::pages::notes::NotesCtx;
use crate::syn_chat::{registry, SynChatCtx};

/// Одна плитка рейла.
#[derive(Clone)]
pub enum RailEntry {
    Code(CodeSession),
    Graph(OpenTab),
    Chat(ChatMeta),
    /// Открытый проект «Заметок» — плитка на каждый файл `.syn`.
    Notes { path: PathBuf, opened_at: u64 },
    /// Тонкая линия между плитками; число — штамп создания (он же ключ
    /// для удаления).
    Separator(u64),
    /// Группа: одна плитка в рейле, `members` — её плитки по порядку
    /// (уже без исчезнувших).
    Group { group: RailGroupConfig, members: Vec<RailEntry> },
}

impl RailEntry {
    /// Стабильный ключ плитки — для `rail_order` и payload'а перетаскивания.
    /// У code-сессий runtime-id переназначаются при загрузке, поэтому ключ —
    /// штамп создания (уникален: миграция раздаёт их по индексу).
    pub fn key(&self) -> String {
        match self {
            RailEntry::Code(s) => format!("code:{}", s.created_at),
            RailEntry::Graph(t) => format!("graph:{}", t.id.0),
            RailEntry::Chat(m) => format!("chat:{}", m.id),
            RailEntry::Notes { path, .. } => notes_key(path),
            RailEntry::Separator(ts) => format!("sep:{ts}"),
            RailEntry::Group { group, .. } => group_key(group.id),
        }
    }

    /// Плитку можно положить в группу: разделители и сами группы — нет.
    pub fn groupable(&self) -> bool {
        !matches!(self, RailEntry::Separator(_) | RailEntry::Group { .. })
    }

    /// Unix-миллисекунды появления в рейле.
    fn created_at(&self) -> u64 {
        match self {
            RailEntry::Code(s) => s.created_at,
            RailEntry::Graph(t) => t.created_at.get_untracked(),
            // Чаты хранят секунды.
            RailEntry::Chat(m) => m.created_at.saturating_mul(1000),
            RailEntry::Notes { opened_at, .. } => *opened_at,
            RailEntry::Separator(ts) => *ts,
            RailEntry::Group { group, .. } => group.id,
        }
    }

    /// Стабильный tie-break для одинаковых штампов: тип, затем id.
    fn tie_key(&self) -> (u8, String) {
        match self {
            RailEntry::Separator(ts) => (0, ts.to_string()),
            RailEntry::Code(s) => (1, format!("{:020}", s.id)),
            RailEntry::Graph(t) => (2, format!("{:020}", t.id.0)),
            RailEntry::Chat(m) => (3, m.id.clone()),
            RailEntry::Notes { path, .. } => (4, path.display().to_string()),
            RailEntry::Group { group, .. } => (5, format!("{:020}", group.id)),
        }
    }
}

/// Ключ плитки проекта заметок.
pub fn notes_key(path: &std::path::Path) -> String {
    format!("notes:{}", path.display())
}

/// Ключ плитки группы.
pub fn group_key(id: u64) -> String {
    format!("group:{id}")
}

/// Иконки, из которых выбирают значок группы: имя (оно хранится в
/// конфиге) → глиф Material Icons.
pub const GROUP_ICONS: &[(&str, &str)] = &[
    ("workspaces", "\u{EA0F}"),
    ("folder", crate::icons::MI_FOLDER),
    ("code", crate::icons::MI_CODE),
    ("terminal", crate::icons::MI_TERMINAL),
    ("hub", crate::icons::MI_HUB),
    ("chat", crate::icons::MI_CHAT),
    ("edit_note", crate::icons::MI_EDIT_NOTE),
    ("work", "\u{E8F9}"),
    ("science", "\u{EA4B}"),
    ("rocket", "\u{EB9B}"),
    ("build", "\u{E869}"),
    ("smart_toy", "\u{F06C}"),
    ("psychology", crate::icons::MI_PSYCHOLOGY),
    ("memory", crate::icons::MI_MEMORY),
    ("movie", crate::icons::MI_MOVIE),
    ("music", "\u{E405}"),
    ("brush", "\u{E3AE}"),
    ("photo", "\u{E412}"),
    ("school", "\u{E80C}"),
    ("book", "\u{E666}"),
    ("home", "\u{E88A}"),
    ("star", "\u{E838}"),
    ("favorite", "\u{E87D}"),
    ("fire", "\u{EF55}"),
    ("eco", "\u{EA35}"),
    ("money", "\u{E227}"),
    ("cart", "\u{E8CC}"),
    ("flight", "\u{E539}"),
    ("fitness", "\u{EB43}"),
    ("games", "\u{EA28}"),
    ("pets", "\u{E91D}"),
    ("cloud", "\u{E2BD}"),
];

/// Глиф иконки группы по имени; неизвестное имя — значок по умолчанию.
pub fn group_icon(name: &str) -> &'static str {
    GROUP_ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, g)| *g)
        .unwrap_or(GROUP_ICONS[0].1)
}

/// Все плитки, кроме групп, — по времени появления.
fn raw_entries() -> Vec<RailEntry> {
    let app = use_context::<AppCtx>();
    let code = use_context::<CodeEditorCtx>();
    let ws = use_context::<EditorWorkspace>();
    let chat = use_context::<SynChatCtx>();
    let notes = use_context::<NotesCtx>();
    let _ = code.session_gen.get();

    let mut out: Vec<RailEntry> = Vec::new();
    for s in code.sessions.get() {
        let _ = s.root_folder.get();
        out.push(RailEntry::Code(s));
    }
    for t in ws.tabs.get() {
        // Скрытые агентские графы в рейл не попадают, пока чат их не
        // раскроет; `.get()` — чтобы reveal перерисовал список.
        if t.hidden.get() {
            continue;
        }
        let _ = t.created_at.get();
        let _ = t.title.get();
        let _ = t.dirty.get();
        out.push(RailEntry::Graph(t));
    }
    for m in chat.chats.get() {
        if m.archived {
            continue;
        }
        out.push(RailEntry::Chat(m));
    }
    // Подпись плитки — имя файла; `.get()` перерисует при открытии,
    // закрытии и «Сохранить как».
    for p in notes.projects.get() {
        out.push(RailEntry::Notes { path: p.path, opened_at: p.opened_at });
    }
    for ts in app.rail_separators.get() {
        out.push(RailEntry::Separator(ts));
    }
    out.sort_by(|a, b| {
        a.created_at()
            .cmp(&b.created_at())
            .then_with(|| a.tie_key().cmp(&b.tie_key()))
    });
    out
}

/// Собрать список плиток верхнего уровня: плитки групп убраны в их
/// `RailEntry::Group`. Читает сигналы через `.get()`, так что вызов
/// внутри `Reactive` перестраивает рейл при любом изменении источников.
pub fn entries() -> Vec<RailEntry> {
    let app = use_context::<AppCtx>();
    let raw = raw_entries();
    let groups = app.rail_groups.get();
    let grouped: std::collections::HashSet<&str> =
        groups.iter().flat_map(|g| g.members.iter().map(String::as_str)).collect();
    let mut by_key: std::collections::HashMap<String, RailEntry> = std::collections::HashMap::new();
    let mut out: Vec<RailEntry> = Vec::new();
    for e in raw {
        let k = e.key();
        if grouped.contains(k.as_str()) && e.groupable() {
            by_key.insert(k, e);
        } else {
            out.push(e);
        }
    }
    for g in &groups {
        let members = g.members.iter().filter_map(|k| by_key.get(k).cloned()).collect();
        out.push(RailEntry::Group { group: g.clone(), members });
    }
    out.sort_by(|a, b| {
        a.created_at()
            .cmp(&b.created_at())
            .then_with(|| a.tie_key().cmp(&b.tie_key()))
    });
    // Ручной порядок поверх хронологического: известные ключи — по списку,
    // неизвестные (новые плитки) — следом, по времени создания.
    let order = app.rail_order.get();
    if !order.is_empty() {
        // Ключ `notes` — плитка времён одного проекта: её место в ручном
        // порядке достаётся первому проекту.
        let first_notes = out.iter().find(|e| matches!(e, RailEntry::Notes { .. })).map(RailEntry::key);
        let rank = |e: &RailEntry| {
            let k = e.key();
            order
                .iter()
                .position(|o| *o == k || (o == "notes" && first_notes.as_deref() == Some(k.as_str())))
                .unwrap_or(usize::MAX)
        };
        out.sort_by_key(rank);
    }
    out
}

/// Группа, в которой лежит плитка `key` (по конфигу).
pub fn group_of(key: &str) -> Option<u64> {
    plan::group_of(&use_context::<AppCtx>().rail_groups.get_untracked(), key)
}

/// Плитка по ключу — среди плиток верхнего уровня и плиток групп.
pub fn find_entry(key: &str) -> Option<RailEntry> {
    for e in entries() {
        if e.key() == key {
            return Some(e);
        }
        if let RailEntry::Group { members, .. } = e {
            if let Some(m) = members.into_iter().find(|m| m.key() == key) {
                return Some(m);
            }
        }
    }
    None
}

/// Применить план к раскладке рейла: ключи верхнего уровня (в порядке
/// показа) и группы. Ключи исчезнувших плиток из групп отбрасываются.
fn apply(f: impl FnOnce(&mut plan::Layout)) {
    let app = use_context::<AppCtx>();
    let raw = raw_entries();
    let live: std::collections::HashSet<String> = raw.iter().map(RailEntry::key).collect();
    let groupable: std::collections::HashSet<String> =
        raw.iter().filter(|e| e.groupable()).map(RailEntry::key).collect();
    let mut layout = plan::Layout {
        top: entries().iter().map(RailEntry::key).collect(),
        groups: app.rail_groups.get_untracked(),
        groupable,
    };
    let before_groups = layout.groups.clone();
    let before_top = layout.top.clone();
    f(&mut layout);
    for g in layout.groups.iter_mut() {
        g.members.retain(|k| live.contains(k));
    }
    if layout.groups != before_groups {
        app.rail_groups.set(layout.groups);
    }
    if layout.top != before_top {
        app.rail_order.set(layout.top);
    }
}

/// Перетаскивание: поставить плитку `src` перед `target` (см.
/// `plan::Layout::move_before`).
pub fn move_before(src: &str, target: &str) {
    apply(|l| l.move_before(src, target));
}

/// Перетаскивание на «+» (или в пустое место под плитками) — в конец
/// верхнего уровня; из группы плитка при этом выходит.
pub fn move_to_end(src: &str) {
    apply(|l| l.move_to_end(src));
}

/// Положить плитку в конец группы (сброс на плитку группы, «В группу»).
pub fn add_to_group(src: &str, gid: u64) {
    apply(|l| l.add_to_group(src, gid));
}

/// «Убрать из группы»: плитка встаёт в рейл сразу после своей группы.
pub fn remove_from_group(key: &str) {
    apply(|l| l.remove_from_group(key));
}

/// Новая группа. С `seed` она встаёт на место этой плитки и забирает её;
/// без — в конец рейла. Возвращает id.
pub fn create_group(name: &str, icon: &str, seed: Option<&str>) -> u64 {
    let id = crate::config::now_millis();
    apply(|l| l.create_group(id, name, icon, seed));
    id
}

/// Переименовать группу и сменить ей иконку.
pub fn update_group(id: u64, name: &str, icon: &str) {
    apply(|l| {
        if let Some(g) = l.groups.iter_mut().find(|g| g.id == id) {
            g.name = name.to_string();
            g.icon = icon.to_string();
        }
    });
}

/// «Разгруппировать»: плитки группы встают в рейл на её место, по порядку.
pub fn ungroup(id: u64) {
    apply(|l| l.ungroup(id));
    let fly = use_context::<AppCtx>().rail_flyout;
    if fly.group.get_untracked() == Some(id) && fly.open.get_untracked() {
        fly.open.set(false);
    }
}

/// Открыть диалог группы.
pub fn edit_group(edit: RailGroupEdit) {
    use_context::<AppCtx>().rail_flyout.edit.set(Some(edit));
}

/// Выезжающая панель группы: открыть у `anchor` (плитка группы в
/// координатах окна) или закрыть, если открыта эта же.
pub fn toggle_flyout(id: u64, anchor: syngui::prelude::Rect) {
    let fly = use_context::<AppCtx>().rail_flyout;
    if fly.open.get_untracked() && fly.group.get_untracked() == Some(id) {
        fly.open.set(false);
        return;
    }
    fly.anchor.set(anchor);
    fly.group.set(Some(id));
    fly.open.set(true);
}

pub fn close_flyout() {
    let fly = use_context::<AppCtx>().rail_flyout;
    if fly.open.get_untracked() {
        fly.open.set(false);
    }
}

/// Перейти на маршрут верхнего уровня (no-op, если уже там).
pub fn navigate(route: &str) {
    let app = use_context::<AppCtx>();
    if app.current_route.get_untracked() == route {
        return;
    }
    if let Ok(mut r) = app.router.lock() {
        r.navigate(route);
    }
    app.current_route.set(route.to_string());
}

/// Клик по плитке: активировать и показать её страницу.
pub fn open(entry: &RailEntry) {
    match entry {
        RailEntry::Code(s) => {
            use_context::<CodeEditorCtx>().switch_to(s.id);
            // Плитка «Терминал» — это просьба о терминале: если все вкладки
            // закрыты (или после перезапуска их ещё нет), открыть одну.
            if s.terminal_only && s.terminals.tabs.get_untracked().is_empty() {
                let _ = crate::pages::code_editor::state::add_terminal(*s, use_context::<AppCtx>());
            }
            navigate("code");
        }
        RailEntry::Graph(t) => {
            use_context::<EditorWorkspace>().activate(t.id);
            navigate("nodes");
        }
        RailEntry::Chat(m) => {
            let chat = use_context::<SynChatCtx>();
            if chat.active_chat_id.get_untracked().as_deref() != Some(m.id.as_str()) {
                registry::select(&m.id);
            }
            // Оторванный чат остаётся в плавающем окне: плитка переключает
            // окно на этот чат и разворачивает его. Страница чата при этом
            // открывается всё равно — на ней панели хода (токены, статус,
            // параметры), и раньше попасть к ним можно было только вернув
            // окно на страницу.
            if chat.chat_detached.get_untracked() {
                chat.chat_window_minimized.set(false);
            }
            navigate("syn_chat");
        }
        RailEntry::Notes { path, .. } => {
            let notes = use_context::<NotesCtx>();
            notes.switch_project(path);
            navigate("notes");
        }
        // Группу открывает рейл: панели нужны границы плитки.
        RailEntry::Separator(_) | RailEntry::Group { .. } => {}
    }
}

/// «Закрыть» из контекстного меню плитки. Code-сессия закрывается сразу;
/// граф с несохранёнными изменениями — через диалог
/// (`components::graph_close_dialog`); чат — через подтверждение архива
/// (`pages::syn_chat::archive_dialog`); разделитель просто удаляется.
pub fn request_close(entry: &RailEntry) {
    match entry {
        RailEntry::Code(s) => use_context::<CodeEditorCtx>().close(s.id),
        RailEntry::Graph(t) => use_context::<EditorWorkspace>().request_close(t.id),
        RailEntry::Chat(m) => {
            use_context::<SynChatCtx>().pending_archive.set(Some(m.clone()));
        }
        // Хвост автосейва дописывается, плитка уходит; последний проект
        // закрыт — со страницы заметок уходим.
        RailEntry::Notes { path, .. } => {
            let notes = use_context::<NotesCtx>();
            if let Err(e) = notes.close_project(path) {
                crate::pages::notes::project_ui::report_error(&e);
                return;
            }
            if !notes.has_project() && use_context::<AppCtx>().current_route.get_untracked() == "notes" {
                navigate("syn_chat");
            }
        }
        RailEntry::Separator(ts) => remove_separator(*ts),
        RailEntry::Group { group, .. } => ungroup(group.id),
    }
}

/// Плитка активна: её страница открыта и она выбрана внутри страницы.
pub fn is_active(entry: &RailEntry) -> bool {
    let app = use_context::<AppCtx>();
    let route = app.current_route.get();
    match entry {
        RailEntry::Code(s) => {
            route == "code" && use_context::<CodeEditorCtx>().active_id.get() == Some(s.id)
        }
        RailEntry::Graph(t) => {
            route == "nodes" && use_context::<EditorWorkspace>().active.get() == Some(t.id)
        }
        RailEntry::Chat(m) => {
            let chat = use_context::<SynChatCtx>();
            let same = chat.active_chat_id.get().as_deref() == Some(m.id.as_str());
            // В плавающем окне чат «открыт» с любой страницы.
            same && (chat.chat_detached.get() || route == "syn_chat")
        }
        RailEntry::Notes { path, .. } => {
            route == "notes" && use_context::<NotesCtx>().project_path.get() == *path
        }
        RailEntry::Separator(_) => false,
        // Группа активна, когда открыта одна из её плиток.
        RailEntry::Group { members, .. } => members.iter().any(is_active),
    }
}

// ─────────────────────────── Меню «+» ───────────────────────────

/// Новая code-сессия: пустая, сразу активная, страница «code».
pub fn new_code_session() {
    let _ = use_context::<CodeEditorCtx>().create_empty();
    navigate("code");
}

/// Новый терминал: сессия без проекта со своей плиткой, сразу с одной
/// вкладкой в домашнем каталоге.
pub fn new_terminal() {
    let code = use_context::<CodeEditorCtx>();
    let id = code.create_terminal();
    if let Some(s) = code.sessions.get_untracked().into_iter().find(|s| s.id == id) {
        let _ = crate::pages::code_editor::state::add_terminal(s, use_context::<AppCtx>());
    }
    navigate("code");
}

/// Новый граф: открыть окно шаблонов. Плитка появится после выбора
/// (`EditorWorkspace::open_template`), закрытие окна без выбора ничего не
/// создаёт.
pub fn new_graph() {
    use_context::<EditorWorkspace>().template_picker_open.set(true);
}

/// Новый чат: создать, активировать, показать страницу чата.
pub fn new_chat() {
    registry::create_new();
    navigate("syn_chat");
}

/// Плитка сменила ключ («Сохранить как» перевело проект на другой файл):
/// ручной порядок рейла сохраняет её место.
pub fn rename_key(old: &str, new: &str) {
    let app = use_context::<AppCtx>();
    if app.rail_groups.get_untracked().iter().any(|g| g.members.iter().any(|k| k == old)) {
        app.rail_groups.update(|groups| {
            for k in groups.iter_mut().flat_map(|g| g.members.iter_mut()) {
                if k == old {
                    *k = new.to_string();
                }
            }
        });
    }
    if app.rail_order.get_untracked().iter().any(|k| k == old) {
        app.rail_order.update(|v| {
            for k in v.iter_mut() {
                if k == old {
                    *k = new.to_string();
                }
            }
        });
    }
}

/// Разделитель со штампом «сейчас» — встаёт после последней плитки.
pub fn add_separator() {
    let app = use_context::<AppCtx>();
    let ts = crate::config::now_millis();
    app.rail_separators.update(|v| v.push(ts));
}

pub fn remove_separator(ts: u64) {
    let app = use_context::<AppCtx>();
    app.rail_separators.update(|v| v.retain(|t| *t != ts));
}

/// Перестановки рейла над голыми ключами — без контекстов приложения,
/// чтобы их можно было проверить тестами.
mod plan {
    use super::group_key;
    use crate::config::RailGroupConfig;

    pub fn group_of(groups: &[RailGroupConfig], key: &str) -> Option<u64> {
        groups.iter().find(|g| g.members.iter().any(|m| m == key)).map(|g| g.id)
    }

    /// Раскладка рейла: ключи верхнего уровня в порядке показа (группы —
    /// `group:<id>`), группы со своими ключами и множество плиток, которые
    /// можно класть в группу (не разделители и не группы).
    pub struct Layout {
        pub top: Vec<String>,
        pub groups: Vec<RailGroupConfig>,
        pub groupable: std::collections::HashSet<String>,
    }

    impl Layout {
        fn detach(&mut self, key: &str) {
            for g in self.groups.iter_mut() {
                g.members.retain(|k| k != key);
            }
        }

        /// Плитка есть в рейле: на верхнем уровне или в группе.
        fn exists(&self, key: &str) -> bool {
            self.top.iter().any(|k| k == key) || group_of(&self.groups, key).is_some()
        }

        /// Поставить `src` перед `target`. Если `target` лежит в группе,
        /// `src` переезжает в ту же группу; разделитель или группа в группу
        /// не кладутся — встают перед самой группой. Плитка из группы,
        /// сброшенная на плитку верхнего уровня, выходит из группы.
        pub fn move_before(&mut self, src: &str, target: &str) {
            if src == target || !self.exists(src) {
                return;
            }
            if let Some(gid) = group_of(&self.groups, target) {
                if !self.groupable.contains(src) {
                    return self.move_before(src, &group_key(gid));
                }
                self.top.retain(|k| k != src);
                self.detach(src);
                if let Some(g) = self.groups.iter_mut().find(|g| g.id == gid) {
                    let at = g.members.iter().position(|k| k == target).unwrap_or(g.members.len());
                    g.members.insert(at, src.to_string());
                }
                return;
            }
            let Some(_) = self.top.iter().position(|k| k == target) else { return };
            self.top.retain(|k| k != src);
            self.detach(src);
            let to = self.top.iter().position(|k| k == target).unwrap_or(self.top.len());
            self.top.insert(to, src.to_string());
        }

        pub fn move_to_end(&mut self, src: &str) {
            if !self.exists(src) {
                return;
            }
            self.top.retain(|k| k != src);
            self.detach(src);
            self.top.push(src.to_string());
        }

        /// В конец группы `gid`. Разделитель или группа встают перед ней.
        pub fn add_to_group(&mut self, src: &str, gid: u64) {
            if !self.groups.iter().any(|g| g.id == gid) || !self.exists(src) {
                return;
            }
            if !self.groupable.contains(src) {
                return self.move_before(src, &group_key(gid));
            }
            self.top.retain(|k| k != src);
            self.detach(src);
            if let Some(g) = self.groups.iter_mut().find(|g| g.id == gid) {
                g.members.push(src.to_string());
            }
        }

        /// Из группы — в рейл сразу после неё.
        pub fn remove_from_group(&mut self, key: &str) {
            let Some(gid) = group_of(&self.groups, key) else { return };
            self.detach(key);
            let gk = group_key(gid);
            let at = self.top.iter().position(|k| *k == gk).map_or(self.top.len(), |i| i + 1);
            self.top.insert(at, key.to_string());
        }

        /// Новая группа: на месте `seed` (если тот в группе — сразу после
        /// его группы) и с ним внутри, иначе пустая в конце рейла.
        pub fn create_group(&mut self, id: u64, name: &str, icon: &str, seed: Option<&str>) {
            let seed = seed.filter(|k| self.groupable.contains(*k) && self.exists(k));
            let gk = group_key(id);
            let at = seed.and_then(|k| match group_of(&self.groups, k) {
                Some(old) => self.top.iter().position(|x| *x == group_key(old)).map(|i| i + 1),
                None => self.top.iter().position(|x| x == k),
            });
            if let Some(k) = seed {
                self.top.retain(|x| x != k);
                self.detach(k);
            }
            match at {
                Some(i) => self.top.insert(i.min(self.top.len()), gk),
                None => self.top.push(gk),
            }
            self.groups.push(RailGroupConfig {
                id,
                name: name.to_string(),
                icon: icon.to_string(),
                members: seed.map(|k| vec![k.to_string()]).unwrap_or_default(),
            });
        }

        /// Плитки группы — в рейл на её место, по порядку; группа удаляется.
        pub fn ungroup(&mut self, id: u64) {
            let Some(pos) = self.groups.iter().position(|g| g.id == id) else { return };
            let g = self.groups.remove(pos);
            let gk = group_key(id);
            match self.top.iter().position(|k| *k == gk) {
                Some(at) => {
                    self.top.splice(at..at + 1, g.members);
                }
                None => self.top.extend(g.members),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn layout(top: &[&str], groups: &[(u64, &[&str])]) -> Layout {
            let mut groupable: std::collections::HashSet<String> = top
                .iter()
                .filter(|k| !k.starts_with("sep:") && !k.starts_with("group:"))
                .map(|k| k.to_string())
                .collect();
            let groups = groups
                .iter()
                .map(|(id, m)| {
                    groupable.extend(m.iter().map(|k| k.to_string()));
                    RailGroupConfig {
                        id: *id,
                        name: format!("g{id}"),
                        icon: String::new(),
                        members: m.iter().map(|k| k.to_string()).collect(),
                    }
                })
                .collect();
            Layout { top: top.iter().map(|k| k.to_string()).collect(), groups, groupable }
        }

        fn members(l: &Layout, id: u64) -> Vec<&str> {
            l.groups.iter().find(|g| g.id == id).unwrap().members.iter().map(String::as_str).collect()
        }

        #[test]
        fn drop_on_group_member_joins_group() {
            let mut l = layout(&["code:1", "group:9", "chat:a"], &[(9, &["code:2", "code:3"])]);
            l.move_before("chat:a", "code:3");
            assert_eq!(l.top, ["code:1", "group:9"]);
            assert_eq!(members(&l, 9), ["code:2", "chat:a", "code:3"]);
        }

        #[test]
        fn member_dropped_on_top_tile_leaves_group() {
            let mut l = layout(&["code:1", "group:9"], &[(9, &["code:2", "code:3"])]);
            l.move_before("code:3", "code:1");
            assert_eq!(l.top, ["code:3", "code:1", "group:9"]);
            assert_eq!(members(&l, 9), ["code:2"]);
        }

        #[test]
        fn separator_and_group_never_nest() {
            let mut l = layout(&["sep:5", "code:1", "group:9", "group:8"], &[(9, &["code:2"]), (8, &[])]);
            l.add_to_group("sep:5", 9);
            assert_eq!(l.top, ["code:1", "sep:5", "group:9", "group:8"]);
            l.move_before("group:8", "code:2");
            assert_eq!(l.top, ["code:1", "sep:5", "group:8", "group:9"]);
            assert_eq!(members(&l, 9), ["code:2"]);
            assert!(members(&l, 8).is_empty());
        }

        #[test]
        fn add_moves_between_groups() {
            let mut l = layout(&["group:1", "group:2"], &[(1, &["code:a"]), (2, &["code:b"])]);
            l.add_to_group("code:a", 2);
            assert!(members(&l, 1).is_empty());
            assert_eq!(members(&l, 2), ["code:b", "code:a"]);
        }

        #[test]
        fn remove_from_group_lands_after_it() {
            let mut l = layout(&["code:1", "group:9", "chat:z"], &[(9, &["code:2", "code:3"])]);
            l.remove_from_group("code:2");
            assert_eq!(l.top, ["code:1", "group:9", "code:2", "chat:z"]);
            assert_eq!(members(&l, 9), ["code:3"]);
        }

        #[test]
        fn create_group_takes_seed_place() {
            let mut l = layout(&["code:1", "code:2", "chat:z"], &[]);
            l.create_group(7, "стек", "code", Some("code:2"));
            assert_eq!(l.top, ["code:1", "group:7", "chat:z"]);
            assert_eq!(members(&l, 7), ["code:2"]);
            // Из чужой группы: новая встаёт сразу после старой.
            l.create_group(8, "x", "", Some("code:2"));
            assert_eq!(l.top, ["code:1", "group:7", "group:8", "chat:z"]);
            assert!(members(&l, 7).is_empty());
            // Без плитки — пустая в конец.
            l.create_group(10, "y", "", None);
            assert_eq!(l.top.last().unwrap(), "group:10");
        }

        #[test]
        fn ungroup_spills_members_in_place() {
            let mut l = layout(&["code:1", "group:9", "chat:z"], &[(9, &["code:2", "code:3"])]);
            l.ungroup(9);
            assert_eq!(l.top, ["code:1", "code:2", "code:3", "chat:z"]);
            assert!(l.groups.is_empty());
        }

        #[test]
        fn move_to_end_pulls_out_of_group() {
            let mut l = layout(&["group:9", "code:1"], &[(9, &["code:2"])]);
            l.move_to_end("code:2");
            assert_eq!(l.top, ["group:9", "code:1", "code:2"]);
            assert!(members(&l, 9).is_empty());
        }
    }
}
