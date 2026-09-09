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
//! Источники данных остаются на своих местах (`CodeEditorCtx`,
//! `EditorWorkspace`, `SynChatCtx`, `AppCtx.rail_separators`) — модуль лишь
//! собирает их в один отсортированный список и знает, что значит «открыть»
//! и «закрыть» плитку каждого типа.

use syngui::prelude::*;

use crate::agent::state::ChatMeta;
use crate::context::AppCtx;
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
    /// Проект «Заметок» — одна плитка на проект; число — штамп открытия.
    Notes(u64),
    /// Тонкая линия между плитками; число — штамп создания (он же ключ
    /// для удаления).
    Separator(u64),
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
            RailEntry::Notes(_) => "notes".to_string(),
            RailEntry::Separator(ts) => format!("sep:{ts}"),
        }
    }

    /// Unix-миллисекунды появления в рейле.
    fn created_at(&self) -> u64 {
        match self {
            RailEntry::Code(s) => s.created_at,
            RailEntry::Graph(t) => t.created_at.get_untracked(),
            // Чаты хранят секунды.
            RailEntry::Chat(m) => m.created_at.saturating_mul(1000),
            RailEntry::Notes(ts) => *ts,
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
            RailEntry::Notes(_) => (4, String::new()),
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
    if let Some(ts) = notes.tile_opened_at.get() {
        // Подпись плитки — имя проекта; `.get()` перерисует при смене.
        let _ = notes.project_title.get();
        out.push(RailEntry::Notes(ts));
    }
    for ts in app.rail_separators.get() {
        out.push(RailEntry::Separator(ts));
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
        let rank = |e: &RailEntry| {
            let k = e.key();
            order.iter().position(|o| *o == k).unwrap_or(usize::MAX)
        };
        out.sort_by_key(rank);
    }
    out
}

/// Перетаскивание: поставить плитку `src` перед `target`. Список ключей
/// пересобирается из текущего порядка, так что ключи исчезнувших плиток
/// отбрасываются сами.
pub fn move_before(src: &str, target: &str) {
    if src == target {
        return;
    }
    let mut keys: Vec<String> = entries().iter().map(RailEntry::key).collect();
    let Some(from) = keys.iter().position(|k| k == src) else { return };
    let moved = keys.remove(from);
    let Some(to) = keys.iter().position(|k| k == target) else {
        keys.insert(from.min(keys.len()), moved);
        return;
    };
    keys.insert(to, moved);
    use_context::<AppCtx>().rail_order.set(keys);
}

/// Перетаскивание на «+» (или в пустое место под плитками) — в конец.
pub fn move_to_end(src: &str) {
    let mut keys: Vec<String> = entries().iter().map(RailEntry::key).collect();
    let Some(from) = keys.iter().position(|k| k == src) else { return };
    let moved = keys.remove(from);
    keys.push(moved);
    use_context::<AppCtx>().rail_order.set(keys);
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
            // Оторванный чат живёт в плавающем окне поверх любой страницы:
            // плитка переключает окно на этот чат и разворачивает его, не
            // уводя со страницы, где работает пользователь.
            if chat.chat_detached.get_untracked() {
                chat.chat_window_minimized.set(false);
            } else {
                navigate("syn_chat");
            }
        }
        RailEntry::Notes(_) => navigate("notes"),
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
        // Проект остаётся на диске (хвост автосейва дописывается) — плитка
        // просто закрывается; с самой страницы уходим.
        RailEntry::Notes(_) => {
            use_context::<NotesCtx>().close_tile();
            if use_context::<AppCtx>().current_route.get_untracked() == "notes" {
                navigate("syn_chat");
            }
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
            let chat = use_context::<SynChatCtx>();
            let same = chat.active_chat_id.get().as_deref() == Some(m.id.as_str());
            // В плавающем окне чат «открыт» с любой страницы.
            same && (chat.chat_detached.get() || route == "syn_chat")
        }
        RailEntry::Notes(_) => route == "notes",
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

/// Заметки из меню «+»: показать плитку проекта и режим; в пустом
/// проекте сразу создаётся первая страница.
pub fn new_note() {
    let notes = use_context::<NotesCtx>();
    notes.open_tile();
    if notes.tree.get_untracked().is_empty() {
        notes.create_page(None, &tr!("notes.untitled"));
    }
    navigate("notes");
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
