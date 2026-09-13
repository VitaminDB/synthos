//! Общий для обоих чат-движков tool-flow: подтверждение tool-вызовов и
//! динамическая сборка дескрипторов autoskill и autotools.
//!
//! Раньше жил внутри `chat::session` (драйвер llama-server). Вынесен в
//! нейтральный модуль, потому что нативный `syn_chat` (in-process synaptix)
//! и подсистема `subagent` используют эти функции напрямую, а сам
//! llama-server-драйвер удалён.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use crate::agent::schema::{ChatTool, ChatToolCall, ToolFunctionSchema};
use crate::agent::tools::{self, catalog::KEY_AUTOSKILL, PendingApproval, Tool, ToolDecision};
use crate::context::AppCtx;

// ─────────────────────────────────────────────────────────────────────────────
// Время (без chrono/time)
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Howard Hinnant `civil_from_days` — UTC YYYY-MM-DD из unix-секунд,
/// без зависимостей от chrono/time. Корректно для всего диапазона,
/// в котором мы реально живём; отрицательные секунды округляем вниз.
pub(crate) fn today_utc_iso(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}")
}

// ─────────────────────────────────────────────────────────────────────────────
// autoskill-дескриптор
// ─────────────────────────────────────────────────────────────────────────────

/// Динамический ChatTool для `autoskill`: статическое описание + актуальный
/// список доступных скилов (читается из `AppCtx.skills`). Если скилов нет —
/// description явно сообщает об этом, enum в schema пустой → модель не
/// сможет вызвать tool, что и ожидается.
pub(crate) fn build_autoskill_chat_tool(app: &AppCtx) -> ChatTool {
    let descriptor = Tool::by_key(KEY_AUTOSKILL).expect("autoskill descriptor должен существовать");
    let skills = app.skills.get_untracked();

    let mut description = descriptor.description.to_string();
    description.push_str("\n\nAvailable skills (id — description):\n");
    if skills.is_empty() {
        description.push_str("- (empty, the user has no skills)\n");
    } else {
        for s in &skills {
            let desc = if s.description.is_empty() {
                "(no description)"
            } else {
                s.description.as_str()
            };
            description.push_str(&format!("- {} — {}\n", s.id, desc));
        }
    }

    let ids: Vec<String> = skills.iter().map(|s| s.id.clone()).collect();
    let mut schema = descriptor.schema.clone();
    if !ids.is_empty() {
        if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
            if let Some(id_field) = props.get_mut("id").and_then(|v| v.as_object_mut()) {
                id_field.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(
                        ids.into_iter().map(serde_json::Value::String).collect(),
                    ),
                );
            }
        }
    }

    ChatTool {
        kind: "function".to_string(),
        function: ToolFunctionSchema {
            name: descriptor.key.to_string(),
            description: Some(description),
            parameters: Some(schema),
            strict: None,
        },
    }
}

/// ChatTool `autotools` для текущего пула (см. [`tools::autotools`]) или
/// `None`, когда пул пуст: тогда инструмент модели не объявляется вовсе.
pub(crate) fn build_autotools_chat_tool(app: &AppCtx) -> Option<ChatTool> {
    let pool = tools::autotools::pool(
        &app.tools.active.get_untracked(),
        &app.tools.auto.get_untracked(),
    );
    (!pool.is_empty()).then(|| tools::autotools::chat_tool(&pool))
}

// ─────────────────────────────────────────────────────────────────────────────
// Подтверждение tool-вызова
// ─────────────────────────────────────────────────────────────────────────────

/// Запрашивает решение по tool-call’у: если `allow_all` — сразу `Allow`,
/// иначе открывает диалог через `ctx.tools.pending_approval` и ждёт ответ.
///
/// Model-agnostic: используется из нативного `syn_chat::session` agent-loop
/// и из `subagent`. Источники политики — общие в `AppCtx.general`.
pub(crate) async fn await_decision_on_tool_call(
    call: &ChatToolCall,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
) -> ToolDecision {
    let tool_key = call.function.name.clone().unwrap_or_default();
    // Вопрос пользователю — не действие: подтверждать нечего, диалог перед
    // панелью с кнопками только мешал бы. Загрузка схемы из пула `autotools`
    // тоже ничего не делает — это чтение каталога.
    if tool_key == crate::agent::tools::catalog::KEY_WIZARD
        || tools::executor::canonical_tool_name(&tool_key) == crate::agent::tools::catalog::KEY_AUTOTOOLS
    {
        return ToolDecision::Allow;
    }

    // Быстрый путь: один main-hop читает три источника и решает,
    // нужно ли вообще показывать диалог.
    //   1) per-chat `allow_all` (legacy, in-memory, переживает только сессию чата);
    //   2) per-tool override из persistent-настроек;
    //   3) глобальный `tool_approval_default` из persistent-настроек.
    // Приоритет: chat-flag > per-tool override > global default.
    let key_for_fast = tool_key.clone();
    let (tx_fast, rx_fast) = tokio::sync::oneshot::channel::<bool>();
    run_on_main_thread(move || {
        let ctx = use_context::<AppCtx>();
        if ctx.tools.allow_all.get_untracked() {
            let _ = tx_fast.send(true);
            return;
        }
        let g = ctx.general;
        let default_mode = g.tool_approval_default.get_untracked();
        let overrides = g.tool_approval_overrides.get_untracked();
        let mode = crate::config::effective_approval_mode(
            &key_for_fast, &default_mode, &overrides,
        );
        let _ = tx_fast.send(mode == crate::config::TOOL_APPROVAL_ALWAYS);
    });
    if rx_fast.await.unwrap_or(false) {
        return ToolDecision::Allow;
    }

    // Долгий путь: кладём PendingApproval и await’им ответ.
    let (tx_decision, rx_decision) = tokio::sync::oneshot::channel::<ToolDecision>();
    let sender_slot = Arc::new(syngui::core::sync::Mutex::new(Some(tx_decision)));
    let sender_slot_for_ui = sender_slot.clone();

    let args_pretty = tools::pretty_args(call.function.arguments.as_deref());
    let descriptor = Tool::by_key(&tool_key);
    let tool_label = descriptor.map(|t| t.label.to_string()).unwrap_or_else(|| tool_key.clone());
    let tool_icon = descriptor
        .map(|t| t.icon.to_string())
        .unwrap_or_else(|| crate::icons::MI_TERMINAL.to_string());

    run_on_main_thread(move || {
        let pending = PendingApproval {
            tool_key,
            tool_label,
            tool_icon,
            args_pretty,
            sender: sender_slot_for_ui,
        };
        use_context::<AppCtx>()
            .tools
            .pending_approval
            .set(Some(Arc::new(pending)));
    });

    // Ждём решения. Если abort успел дёрнуться — сами кладём Cancel
    // в канал, чтобы не зависнуть.
    let abort_c = abort.clone();
    let poller = async move {
        loop {
            if abort_c.load(Ordering::Relaxed) != snapshot {
                if let Ok(mut slot) = sender_slot.lock() {
                    if let Some(tx) = slot.take() {
                        let _ = tx.send(ToolDecision::Cancel);
                    }
                }
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
    };

    let decision = tokio::select! {
        d = rx_decision => d.unwrap_or(ToolDecision::Cancel),
        _ = poller => ToolDecision::Cancel,
    };

    // Диалог закрыт — убираем из сигнала, чтобы Portal схлопнулся.
    run_on_main_thread(|| {
        use_context::<AppCtx>().tools.pending_approval.set(None);
    });

    decision
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_utc_iso_known_epoch() {
        // 2021-01-01T00:00:00Z = 1609459200.
        assert_eq!(today_utc_iso(1_609_459_200), "2021-01-01");
        // Unix epoch.
        assert_eq!(today_utc_iso(0), "1970-01-01");
    }

    fn make_skill(id: &str, name: &str, description: &str) -> crate::skills::Skill {
        crate::skills::Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            content: String::new(),
        }
    }

    /// Чистая копия логики `build_autoskill_chat_tool` без зависимости от
    /// `AppCtx` (он в thread-local и недоступен в обычных unit-тестах).
    fn build_autoskill_for_tests(skills: &[crate::skills::Skill]) -> ChatTool {
        let descriptor = Tool::by_key(KEY_AUTOSKILL).expect("autoskill descriptor present");
        let mut description = descriptor.description.to_string();
        description.push_str("\n\nAvailable skills (id — description):\n");
        if skills.is_empty() {
            description.push_str("- (empty, the user has no skills)\n");
        } else {
            for s in skills {
                let desc = if s.description.is_empty() {
                    "(no description)"
                } else {
                    s.description.as_str()
                };
                description.push_str(&format!("- {} — {}\n", s.id, desc));
            }
        }
        let ids: Vec<String> = skills.iter().map(|s| s.id.clone()).collect();
        let mut schema = descriptor.schema.clone();
        if !ids.is_empty() {
            if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
                if let Some(id_field) = props.get_mut("id").and_then(|v| v.as_object_mut()) {
                    id_field.insert(
                        "enum".to_string(),
                        serde_json::Value::Array(
                            ids.into_iter().map(serde_json::Value::String).collect(),
                        ),
                    );
                }
            }
        }
        ChatTool {
            kind: "function".to_string(),
            function: ToolFunctionSchema {
                name: descriptor.key.to_string(),
                description: Some(description),
                parameters: Some(schema),
                strict: None,
            },
        }
    }

    #[test]
    fn autoskill_descriptor_lists_skills_in_description() {
        let skills = vec![
            make_skill("greet", "Greeting", "Tone of the first message"),
            make_skill("apo", "Apology", ""),
        ];
        let tool = build_autoskill_for_tests(&skills);
        let desc = tool.function.description.unwrap();
        assert!(desc.contains("- greet — Tone of the first message"), "{desc}");
        assert!(desc.contains("- apo — (no description)"), "{desc}");
    }

    #[test]
    fn autoskill_descriptor_schema_enum_lists_ids() {
        let skills = vec![make_skill("a", "A", ""), make_skill("b", "B", "")];
        let tool = build_autoskill_for_tests(&skills);
        let params = tool.function.parameters.unwrap();
        let enum_arr = params
            .get("properties")
            .and_then(|p| p.get("id"))
            .and_then(|f| f.get("enum"))
            .and_then(|e| e.as_array())
            .unwrap();
        let ids: Vec<&str> = enum_arr.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn autoskill_descriptor_handles_empty_skills() {
        let skills: Vec<crate::skills::Skill> = Vec::new();
        let tool = build_autoskill_for_tests(&skills);
        let desc = tool.function.description.unwrap();
        assert!(desc.contains("(empty, the user has no skills)"));
        let params = tool.function.parameters.unwrap();
        assert!(params
            .get("properties")
            .and_then(|p| p.get("id"))
            .and_then(|f| f.get("enum"))
            .is_none());
    }
}
