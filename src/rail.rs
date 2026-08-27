//! Единый список плиток нав-рейла.
//!
//! Раньше рейл показывал только code-сессии, а графы нодового редактора
//! жили во вкладках на своей странице, чаты — в колонке слева от ленты.
//! Теперь всё это — плитки одного списка: code-сессия (папка), граф
//! (хаб), чат (аватар) и разделитель, добавленный пользователем через «+».
//! Порядок — по времени появления (`created_at`), вперемешку по типам:
//! новое встаёт в конец, как вкладка в браузере.
//!
//! Источники данных остаются на своих местах (`CodeEditorCtx`,
//! `EditorWorkspace`, `SynChatCtx`, `AppCtx.rail_separators`) — модуль лишь
//! собирает их в один отсортированный список и знает, что значит «открыть»
//! и «закрыть» плитку каждого типа.

use syngui::prelude::*;

use crate::agent::state::ChatMeta;
use crate::context::AppCtx;
use crate::pages::code_editor::state::{CodeEditorCtx, CodeSession};
use crate::pages::node_editor::tabs::{EditorWorkspace, OpenTab};
use crate::syn_chat::{registry, SynChatCtx};

/// Одна плитка рейла.
#[derive(Clone)]
pub enum RailEntry {
    Code(CodeSession),
    Graph(OpenTab),
    Chat(ChatMeta),
    /// Тонкая линия между плитками; число — штамп создания (он же ключ
    /// для удаления).
    Separator(u64),
}

impl RailEntry {
    /// Unix-миллисекунды появления в рейле.
    fn created_at(&self) -> u64 {
        match self {
            RailEntry::Code(s) => s.created_at,
            RailEntry::Graph(t) => t.created_at.get_untracked(),
            // Чаты хранят секунды.
            RailEntry::Chat(m) => m.created_at.saturating_mul(1000),
            RailEntry::Separator(ts) => *ts,
        }
    }

    /// Стабильный tie-break для одинаковых штампов: тип, затем id.
    fn tie_key(&self) -> (u8, String) {
        match self {
            RailEntry::Separator(ts) => (0, ts.to_string()),
            RailEntry::Code(s) => (1, format!("{:020}", s.id)),
            RailEntry::Graph(t) => (2, format!("{:020}", t.id.0)),
            RailEntry::Chat(m) => (3, m.id.clone()),
        }
    }
}

/// Собрать список плиток. Читает сигналы через `.get()`, так что вызов
/// внутри `Reactive` перестраивает рейл при любом изменении источников.
pub fn entries() -> Vec<RailEntry> {
    let app = use_context::<AppCtx>();
    let code = use_context::<CodeEditorCtx>();
    let ws = use_context::<EditorWorkspace>();
    let chat = use_context::<SynChatCtx>();
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
            navigate("syn_chat");
        }
        RailEntry::Separator(_) => {}
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
        RailEntry::Separator(ts) => remove_separator(*ts),
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
            route == "syn_chat"
                && use_context::<SynChatCtx>().active_chat_id.get().as_deref()
                    == Some(m.id.as_str())
        }
        RailEntry::Separator(_) => false,
    }
}

// ─────────────────────────── Меню «+» ───────────────────────────

/// Новая code-сессия: пустая, сразу активная, страница «code».
pub fn new_code_session() {
    let _ = use_context::<CodeEditorCtx>().create_empty();
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
