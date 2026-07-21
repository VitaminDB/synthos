//! Tool `subagent` — делегировать подзадачу вложенному агенту.
//!
//! Идея: основной агент тратит контекст на длинные исследования (читает
//! 5 веб-страниц, чтобы извлечь 1-2 факта; обходит проект через серию
//! `bash find/grep`). Если такие шаги делать в основном цикле, история
//! раздувается тулрезультатами и быстро вытесняет полезные сообщения.
//!
//! `subagent` запускает свой собственный `chat_completions`-цикл (без UI)
//! с явно ограниченным набором тулов и заданной задачей, выполняет нужные
//! шаги и возвращает родителю **только финальный текст**. Промежуточные
//! tool-вызовы внутри субагента в основную ленту не попадают — только в
//! tracing-логи.
//!
//! Безопасность:
//! - Сам вызов `subagent` проходит через стандартный approval-диалог в
//!   основном цикле (см. `chat::session::await_decision_on_tool_call`).
//! - **Внутри** субагента вложенные тулы исполняются auto-allow — без
//!   диалога. Это сознательный trade-off (пользователь подтверждает
//!   только сам вызов subagent с его описанием task).
//! - Вложенный subagent (subagent → subagent) запрещён через
//!   `tokio::task_local!` счётчик глубины + явная фильтрация по имени
//!   при диспатче.
//! - Default-набор тулов = все активные в текущем чате минус `subagent`.
//!   Если пользователь отключил `bash` глобально, субагент его тоже не
//!   получит.

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::context::AppCtx;
use crate::llama::api::{
    ChatMessage as ApiChatMessage, ChatRequest, ChatRole, ChatTool, LlamaClient, SamplingParams,
};

use super::catalog::{KEY_AUTOSKILL, KEY_SUBAGENT};
use super::descriptor::Tool;
use super::executor::ToolError;

// Лимит tool-turn'ов читается из `AppCtx.general.subagent_max_turns` в
// `snapshot_from_main`. После их исчерпания делается ОДИН финальный turn
// без tools (`force_final_summary_turn`) — модель обязана сжать прогресс
// в текстовый итог. Поэтому реальный верхний предел запросов к LLM =
// `subagent_max_turns + 1`. Дефолт — `config::default_subagent_max_turns()`.

/// Sampling temperature субагента — тот же дефолт, что в main-cycle. В
/// будущем можно занизить до 0.3 для более «дисциплинированных» подзадач,
/// но сейчас держим консистентно с родителем.
const SUBAGENT_TEMPERATURE: f32 = 0.7;

tokio::task_local! {
    /// Глубина вложенности subagent-ов в рамках одной tokio-задачи.
    /// `thread_local!` тут не подходит: tokio-задачи мигрируют между
    /// потоками через `await`, и TLS не сохраняется.
    static SUBAGENT_DEPTH: u32;
}

// ─────────────────────────────────────────────────────────────────────────────
// Args
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawArgs {
    task: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
}

#[derive(Debug)]
struct SubagentArgs {
    task: String,
    system_prompt: Option<String>,
    /// `None` — использовать дефолт (все активные тулы кроме subagent).
    /// `Some(v)` — явно заданный whitelist (после фильтрации).
    tools: Option<Vec<String>>,
}

fn parse_args(args_json: &str) -> Result<SubagentArgs, ToolError> {
    let raw: RawArgs =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;

    let task = raw
        .task
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(ToolError::MissingField("task"))?;

    let system_prompt = raw
        .system_prompt
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let tools = match raw.tools {
        None => None,
        Some(v) => {
            if v.iter().any(|k| k == KEY_SUBAGENT) {
                return Err(ToolError::BadArgs(
                    "вложенный subagent запрещён: убери \"subagent\" из tools".to_string(),
                ));
            }
            Some(v.into_iter().map(|s| s.trim().to_string()).collect())
        }
    };

    Ok(SubagentArgs {
        task,
        system_prompt,
        tools,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot
// ─────────────────────────────────────────────────────────────────────────────

/// Состояние, снятое с main-потока в начале вызова `subagent::run`.
///
/// Subagent живёт в tokio-таске и не имеет прямого доступа к
/// `RwSignal`-сигналам (они привязаны к thread-local runtime syngui).
struct SubagentSnapshot {
    host: String,
    port: u16,
    default_system_prompt: String,
    /// Готовые ChatTool-дескрипторы всех активных в чате тулов кроме `subagent`.
    /// Динамически собранный (см. `build_active_tools_for_subagent`).
    active_tools: Vec<ChatTool>,
    /// Лимит tool-turn'ов цикла субагента — пользовательская настройка
    /// `general.subagent_max_turns`, склампленная к `>= 1`.
    max_turns: usize,
    /// Тот же abort, что у parent run_agent. Если пользователь нажмёт Stop —
    /// инкрементится в `chat::session::abort()`, и мы между turn-ами это видим.
    abort: Arc<AtomicU64>,
    abort_baseline: u64,
}

async fn snapshot_from_main() -> Result<SubagentSnapshot, ToolError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        let host = app.general.server_host.get_untracked();
        let port = app.general.server_port.get_untracked();
        let default_system_prompt = app.chat.system_prompt.get_untracked();
        let active_tools = build_active_tools_for_subagent(&app);
        // Снимаем лимит turn'ов из настроек. `0` (при ручной правке
        // конфига) приведёт к мгновенному forced-summary без шанса
        // что-либо сделать — клампим к `>= 1`.
        let max_turns = app.general.subagent_max_turns.get_untracked().max(1) as usize;
        let abort = app.chat.abort.clone();
        let abort_baseline = abort.load(Ordering::Relaxed);
        let _ = tx.send(SubagentSnapshot {
            host,
            port,
            default_system_prompt,
            active_tools,
            max_turns,
            abort,
            abort_baseline,
        });
    });
    rx.await.map_err(|e| ToolError::Spawn(e.to_string()))
}

/// Собирает дескрипторы активных тулов для субагента: те же, что в
/// `chat::session::collect_active_tools`, но с явным исключением
/// `subagent` (рекурсия запрещена) и с динамическим autoskill-обогащением.
fn build_active_tools_for_subagent(app: &AppCtx) -> Vec<ChatTool> {
    let keys = app.tools.active.get_untracked();
    keys.iter()
        .filter(|k| k.as_str() != KEY_SUBAGENT)
        .filter_map(|k| {
            if k == KEY_AUTOSKILL {
                Some(crate::chat::session::build_autoskill_chat_tool(app))
            } else {
                Tool::by_key(k).map(|t| t.to_chat_tool())
            }
        })
        .collect()
}

/// Применяет фильтр пользовательских `args.tools` к собранному snapshot'у.
/// Неизвестные ключи логируются в warn и отбрасываются. Сам `subagent`
/// явно убирается (двойная защита; первичная — в `parse_args`).
fn select_tools(snap_tools: &[ChatTool], requested: Option<&[String]>) -> Vec<ChatTool> {
    match requested {
        None => snap_tools.to_vec(),
        Some(req) => {
            let mut out = Vec::new();
            let known: std::collections::HashSet<&str> =
                snap_tools.iter().map(|t| t.function.name.as_str()).collect();
            for k in req {
                if k == KEY_SUBAGENT {
                    continue;
                }
                if !known.contains(k.as_str()) {
                    tracing::warn!(target: "subagent", tool = %k,
                        "запрошенный tool не активен в чате — игнорирую");
                    continue;
                }
                if let Some(t) = snap_tools.iter().find(|t| t.function.name == *k) {
                    out.push(t.clone());
                }
            }
            out
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Точка входа из `executor::execute`. Парсит JSON-аргументы, проверяет
/// recursion-limit, снимает snapshot конфига и крутит UI-less цикл.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let args = parse_args(args_json)?;

    let depth = SUBAGENT_DEPTH.try_with(|d| *d).unwrap_or(0);
    if depth >= 1 {
        return Err(ToolError::BadArgs(
            "вложенный subagent запрещён (max depth = 1)".to_string(),
        ));
    }

    let snap = snapshot_from_main().await?;

    let id = subagent_id();
    tracing::info!(
        target: "subagent",
        id = %id,
        depth = depth + 1,
        task_len = args.task.len(),
        "start"
    );

    let result = SUBAGENT_DEPTH
        .scope(depth + 1, run_subagent_loop(snap, args, id.clone()))
        .await;

    match &result {
        Ok(text) => {
            tracing::info!(
                target: "subagent",
                id = %id,
                output_len = text.len(),
                "end ok"
            );
        }
        Err(e) => {
            tracing::warn!(target: "subagent", id = %id, error = %e, "end err");
        }
    }
    result
}

fn subagent_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("sub-{:08x}", nanos)
}

// ─────────────────────────────────────────────────────────────────────────────
// Loop
// ─────────────────────────────────────────────────────────────────────────────

async fn run_subagent_loop(
    snap: SubagentSnapshot,
    args: SubagentArgs,
    id: String,
) -> Result<String, ToolError> {
    let tools_list = select_tools(&snap.active_tools, args.tools.as_deref());

    // System: префикс с датой + либо пользовательский system_prompt из args,
    // либо общий из chat config.
    let date = crate::chat::session::today_utc_iso(crate::chat::session::now_unix_secs());
    let preamble = format!("Текущая дата (UTC): {date}.");
    let user_sys = args
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| snap.default_system_prompt.trim().to_string());
    let merged_system = if user_sys.is_empty() {
        preamble
    } else {
        format!("{preamble}\n\n{user_sys}")
    };

    let max_turns = snap.max_turns;
    let mut history: Vec<ApiChatMessage> = Vec::with_capacity(4 + max_turns * 2);
    history.push(ApiChatMessage::system(merged_system));
    history.push(ApiChatMessage::user(args.task.clone()));

    let base_url = format!("http://{}:{}", snap.host, snap.port);
    let client = LlamaClient::with_base_url(base_url.clone());
    let sampling = SamplingParams::new().with_temperature(SUBAGENT_TEMPERATURE);

    for turn in 0..max_turns {
        if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
            tracing::info!(target: "subagent", id = %id, turn, "aborted by user");
            return Ok("Subagent прерван пользователем.".to_string());
        }

        tracing::debug!(
            target: "subagent",
            id = %id,
            turn,
            history_len = history.len(),
            tools = tools_list.len(),
            "request"
        );

        let mut req = ChatRequest::new(history.clone())
            .with_sampling(sampling.clone())
            .with_parallel_tool_calls_disabled();
        if !tools_list.is_empty() {
            req = req.with_tools(tools_list.clone());
        }

        let resp = client
            .chat_completions(&req)
            .await
            .map_err(|e| ToolError::BadArgs(format!("LLM error: {e}")))?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ToolError::BadArgs("LLM вернул пустой choices".to_string()))?;
        let msg = choice.message;
        let finish = choice.finish_reason;

        let assistant_content = msg.content.clone();
        let assistant_tool_calls = msg.tool_calls.clone();

        // Положим ассистент-сообщение в локальную историю для следующего turn.
        history.push(ApiChatMessage {
            role: ChatRole::Assistant,
            content: assistant_content
                .clone()
                .filter(|s| !s.is_empty())
                .map(crate::llama::api::ChatContent::Text),
            name: None,
            tool_call_id: None,
            tool_calls: assistant_tool_calls.clone(),
            reasoning_content: None,
        });

        let calls = match assistant_tool_calls {
            Some(v) if !v.is_empty() => v,
            _ => {
                let text = assistant_content.unwrap_or_default();
                tracing::debug!(
                    target: "subagent",
                    id = %id,
                    turn,
                    ?finish,
                    "final text"
                );
                return Ok(text);
            }
        };

        tracing::debug!(
            target: "subagent",
            id = %id,
            turn,
            n_tool_calls = calls.len(),
            "tool_calls"
        );

        for call in calls {
            let name = call.function.name.clone().unwrap_or_default();

            // Двойная защита от рекурсии: catalog уже исключает subagent из
            // дескрипторов, но модель всё равно может сгенерировать call с
            // таким именем. Не зовём `tools::execute` (там был бы дальнейший
            // depth-check), сразу записываем error tool-result.
            if name == KEY_SUBAGENT {
                history.push(ApiChatMessage::tool(
                    "ошибка: вложенный subagent запрещён".to_string(),
                    call.id.clone(),
                ));
                tracing::warn!(target: "subagent", id = %id, "blocked nested subagent call");
                continue;
            }

            tracing::info!(
                target: "subagent",
                id = %id,
                tool = %name,
                args_len = call.function.arguments.as_deref().map(str::len).unwrap_or(0),
                "exec"
            );

            // Auto-allow: пропускаем approval-диалог. Пользователь подтвердил
            // вызов сам subagent в основном цикле, дальше всё идёт без UI.
            // Box::pin — `executor::execute` через KEY_SUBAGENT возвращается
            // в этот же цикл; без боксинга компилятор не может вычислить
            // размер async-future. Реальной рекурсии нет (depth-чек в `run`).
            let outcome = Box::pin(super::executor::execute(&call)).await;
            history.push(ApiChatMessage::tool(outcome.content, call.id));
        }
    }

    tracing::info!(
        target: "subagent",
        id = %id,
        turns = max_turns,
        "turn limit reached, forcing final summary"
    );

    // Финальный turn — без tools, без `tool_choice`. Модель обязана
    // ответить текстом, потому что вызвать tool ей нечем. Получаем
    // сжатую сводку накопленного прогресса вместо сухой ошибки.
    force_final_summary_turn(&client, &sampling, &mut history, &id, &snap).await
}

/// Делает один финальный chat-completion БЕЗ tools, чтобы модель
/// сжала прогресс. Если ответ всё-таки пуст — отдаём fallback-сообщение.
async fn force_final_summary_turn(
    client: &LlamaClient,
    sampling: &SamplingParams,
    history: &mut Vec<ApiChatMessage>,
    id: &str,
    snap: &SubagentSnapshot,
) -> Result<String, ToolError> {
    if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
        return Ok("Subagent прерван пользователем.".to_string());
    }

    history.push(ApiChatMessage::user(
        "Ты исчерпал лимит tool-вызовов. Не вызывай больше никаких tools. \
         Сожми накопленный прогресс в краткий итог 1-3 абзаца — что узнал, \
         что осталось неясным, какие ещё шаги нужны. Текст без преамбулы."
            .to_string(),
    ));

    let req = ChatRequest::new(history.clone()).with_sampling(sampling.clone());
    // Без `with_tools` — модель не сможет ответить tool_calls'ами.

    let resp = client
        .chat_completions(&req)
        .await
        .map_err(|e| ToolError::BadArgs(format!("LLM error (final summary): {e}")))?;

    let summary = resp
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match summary {
        Some(s) => {
            tracing::info!(target: "subagent", id = %id, summary_len = s.len(), "final summary ok");
            Ok(s)
        }
        None => {
            tracing::warn!(target: "subagent", id = %id, "final summary empty");
            let limit = snap.max_turns;
            Ok(format!(
                "Subagent достиг лимита в {limit} turn-ов и не сошёлся \
                 к финальному ответу. Сформулируй задачу более узко или вызови \
                 subagent повторно с уже имеющимся прогрессом."
            ))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_minimal() {
        let a = parse_args(r#"{"task":"hello"}"#).unwrap();
        assert_eq!(a.task, "hello");
        assert!(a.system_prompt.is_none());
        assert!(a.tools.is_none());
    }

    #[test]
    fn parse_args_full() {
        let a = parse_args(
            r#"{"task":"  do X  ","system_prompt":"be careful","tools":["bash","web"]}"#,
        )
        .unwrap();
        assert_eq!(a.task, "do X");
        assert_eq!(a.system_prompt.as_deref(), Some("be careful"));
        let t = a.tools.unwrap();
        assert_eq!(t, vec!["bash".to_string(), "web".to_string()]);
    }

    #[test]
    fn parse_args_rejects_subagent_in_tools() {
        let err = parse_args(r#"{"task":"hi","tools":["subagent"]}"#).unwrap_err();
        match err {
            ToolError::BadArgs(msg) => assert!(msg.contains("subagent")),
            _ => panic!("ожидалась BadArgs, было {:?}", err),
        }
    }

    #[test]
    fn parse_args_rejects_subagent_among_others() {
        let err = parse_args(r#"{"task":"hi","tools":["bash","subagent","web"]}"#).unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }

    #[test]
    fn parse_args_missing_task() {
        let err = parse_args(r#"{}"#).unwrap_err();
        assert!(matches!(err, ToolError::MissingField("task")));

        let err = parse_args(r#"{"task":"   "}"#).unwrap_err();
        assert!(matches!(err, ToolError::MissingField("task")));
    }

    #[test]
    fn parse_args_empty_system_prompt_is_none() {
        let a = parse_args(r#"{"task":"x","system_prompt":"  "}"#).unwrap();
        assert!(a.system_prompt.is_none());
    }

    #[test]
    fn parse_args_invalid_json() {
        let err = parse_args("not json").unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }

    fn dummy_chat_tool(name: &str) -> ChatTool {
        ChatTool {
            kind: "function".into(),
            function: crate::llama::api::ToolFunctionSchema {
                name: name.into(),
                description: None,
                parameters: None,
                strict: None,
            },
        }
    }

    #[test]
    fn select_tools_default_returns_all_active() {
        let snap = vec![
            dummy_chat_tool("bash"),
            dummy_chat_tool("web"),
            dummy_chat_tool("kb_search"),
        ];
        let out = select_tools(&snap, None);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn select_tools_filters_unknown_keys() {
        let snap = vec![dummy_chat_tool("bash"), dummy_chat_tool("web")];
        let req = vec!["bash".to_string(), "totally_made_up".to_string()];
        let out = select_tools(&snap, Some(&req));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].function.name, "bash");
    }

    #[test]
    fn select_tools_drops_subagent_silently() {
        let snap = vec![dummy_chat_tool("bash"), dummy_chat_tool("web")];
        // Реально parse_args бы это уже отбил, но проверим вторую линию защиты.
        let req = vec!["bash".to_string(), KEY_SUBAGENT.to_string()];
        let out = select_tools(&snap, Some(&req));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].function.name, "bash");
    }

    #[test]
    fn select_tools_preserves_order_from_request() {
        let snap = vec![
            dummy_chat_tool("bash"),
            dummy_chat_tool("web"),
            dummy_chat_tool("kb_search"),
        ];
        let req = vec!["web".to_string(), "bash".to_string()];
        let out = select_tools(&snap, Some(&req));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].function.name, "web");
        assert_eq!(out[1].function.name, "bash");
    }
}
