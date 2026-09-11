//! Агентские инструменты (tools) для llama.cpp.
//!
//! Организация:
//! - [`descriptor`] — статическое описание инструмента: ключ, имя, иконка,
//!   JSON-schema параметров, адаптер в `ChatTool` API-слоя.
//! - [`catalog`]    — конкретные инструменты: `bash`, `kb_search`, `web`.
//! - [`executor`]   — async-исполнение: безопасный парсинг аргументов,
//!   запуск, truncate больших выводов, структурированный результат.
//! - [`budget`]     — сколько контекста осталось под ответ: agent-loop кладёт
//!   туда остаток окна в токенах, инструмент режет ответ по нему.
//! - [`web`]        — единый tool интернет-доступа: `action=search` (DDG
//!   html-endpoint) и `action=read` (reqwest → Readability → htmd).
//! - [`kb_search`]  — поиск по локальным базам знаний (RAG hybrid).
//! - [`web_fetch`]  — общий HTTP-helper (reqwest+htmd), используется
//!   kb-ingest pipeline'ом; tool `web` имеет свой собственный pipeline в
//!   `web::http`, чтобы получать сырой HTML до htmd для Readability.
//! - [`view_media`] — модель смотрит локальный файл по пути своим зрением
//!   (картинка/видео → vision-башня, звук → ASR); исполняет agent-loop чата.
//!
//! Политика безопасности:
//! - Каждое исполнение проходит через UI-подтверждение (кроме «Разрешить все»,
//!   действующее до конца текущего чата).
//! - `bash` запускается через `bash -lc "<cmd>"`; сырой вывод ограничен
//!   предохранителем [`executor::MAX_READ_OUTPUT_BYTES`], а в промпт и ленту
//!   уходит копия, уложенная в остаток окна хода по токенам
//!   (`syn_chat::session::fit_for_prompt` поверх [`budget::fit`]). Исключение —
//!   `autoskill`: скил не выхлоп, а инструкция, и уходит модели целиком
//!   (потолок [`executor::MAX_SKILL_OUTPUT_BYTES`], укладке не подлежит).

pub mod autoskill;
pub mod budget;
pub mod catalog;
pub mod descriptor;
pub mod executor;
pub mod kb_search;
pub mod notes;
pub mod pipelines;
pub mod subagent;
pub mod system;
pub mod view_media;
pub mod web;
pub mod web_fetch;
pub mod wizard;

use std::sync::Arc;

use syngui::core::sync::Mutex;
use tokio::sync::oneshot;

pub use descriptor::Tool;
pub use executor::{execute, ToolError, ToolOutcome};

/// Решение пользователя по диалогу подтверждения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDecision {
    /// Отказ: инструмент не выполнять, текущий tool-turn прекратить.
    Cancel,
    /// Однократное разрешение — выполнить текущий вызов.
    Allow,
    /// Разрешить все последующие tool-вызовы до конца чата без диалогов.
    AllowAll,
}

/// Запрос на подтверждение одного tool-вызова.
///
/// Живёт в `ctx.tools.pending_approval` ровно столько, сколько открыт диалог.
/// `sender` — одноразовый канал, через него UI передаёт решение оркестратору
/// из `chat::session`. После `send` слот очищается.
pub struct PendingApproval {
    pub tool_key: String,
    pub tool_label: String,
    pub tool_icon: String,
    pub args_pretty: String,
    /// `Arc<Mutex<Option<...>>>` — чтобы UI мог забрать sender по клику,
    /// а эффект abort мог независимо положить в слот `Cancel`.
    pub sender: Arc<Mutex<Option<oneshot::Sender<ToolDecision>>>>,
}

impl std::fmt::Debug for PendingApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingApproval")
            .field("tool_key", &self.tool_key)
            .field("tool_label", &self.tool_label)
            .field("args_pretty", &self.args_pretty)
            .finish()
    }
}

/// Сравнение по идентичности канала-sender’а: два разных `PendingApproval`
/// — это две разные сессии подтверждения, даже если tool_key случайно совпал.
/// Нужно, чтобы `RwSignal<Option<Arc<PendingApproval>>>::set` видел разницу
/// между «закрыт» / «открыт» / «переоткрыт», не требуя `Eq` на внутренностях.
impl PartialEq for PendingApproval {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}
impl Eq for PendingApproval {}

impl PendingApproval {
    /// Пытается отправить решение. Если канал уже использован — молчит.
    pub fn send(&self, decision: ToolDecision) {
        if let Ok(mut slot) = self.sender.lock() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(decision);
            }
        }
    }
}

/// Удобный тайп-алиас — оркестратор держит именно такой receiver.
pub type ApprovalReceiver = oneshot::Receiver<ToolDecision>;

/// Форматирует JSON-аргументы в читаемый вид для UI и для LLM-истории.
///
/// Если в `ChatToolCallFunction.arguments` лежит валидный JSON — возвращаем
/// pretty-версию; если нет (модель отдала сырую строку-подобие) — оставляем
/// как есть, просто trim’им. Пустые аргументы → пустая строка.
pub fn pretty_args(raw: Option<&str>) -> String {
    let Some(s) = raw else { return String::new(); };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string()),
        Err(_) => trimmed.to_string(),
    }
}
