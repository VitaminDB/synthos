//! Tool `subagent` — делегировать подзадачу вложенному агенту.
//!
//! Идея: основной агент тратит контекст на длинные исследования (читает
//! 5 веб-страниц, чтобы извлечь 1-2 факта; обходит проект через серию
//! `bash find/grep`). Если такие шаги делать в основном цикле, история
//! раздувается тулрезультатами и быстро вытесняет полезные сообщения.
//!
//! `subagent` запускает свой собственный agent-цикл (без UI) поверх той же
//! нативной Qwen3.6-модели (`SynModelRegistry`), что и основной Syn-чат,
//! с явно ограниченным набором тулов и заданной задачей, выполняет нужные
//! шаги и возвращает родителю **только финальный текст**. Промежуточные
//! tool-вызовы внутри субагента в основную ленту не попадают — только в
//! tracing-логи.
//!
//! Безопасность:
//! - Сам вызов `subagent` проходит через стандартный approval-диалог в
//!   основном цикле (см. `agent::tool_flow::await_decision_on_tool_call`).
//! - **Внутри** субагента вложенные тулы исполняются auto-allow — без
//!   диалога. Это сознательный trade-off (пользователь подтверждает
//!   только сам вызов subagent с его описанием task).
//! - Вложенный subagent (subagent → subagent) запрещён через
//!   `tokio::task_local!` счётчик глубины + явная фильтрация по имени
//!   при диспатче.
//! - Default-набор тулов = все активные в текущем чате минус `subagent`.
//!   Если пользователь отключил `bash` глобально, субагент его тоже не
//!   получит.

use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use synaptix::facade::llm::{LlmGeneration, Message};

use crate::agent::schema::{ChatTool, ChatToolCall, ChatToolCallFunction};
use crate::context::AppCtx;
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{SynChatCtx, ThinkParser};
use crate::syn_chat::tool_parser::{RawToolCall, ToolCallParser};

use super::catalog::{KEY_AUTOSKILL, KEY_SUBAGENT};
use super::descriptor::Tool;
use super::executor::ToolError;

// Лимит tool-turn'ов читается из `AppCtx.general.subagent_max_turns` в
// `snapshot_from_main`. После их исчерпания делается ОДИН финальный turn
// без tools (`force_final_summary_turn`) — модель обязана сжать прогресс
// в текстовый итог. Поэтому реальный верхний предел запросов к LLM =
// `subagent_max_turns + 1`. Дефолт — `config::default_subagent_max_turns()`.

/// Sampling temperature субагента — тот же дефолт, что в main-cycle.
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
    /// Загруженная нативная модель — тот же handle, что у основного чата.
    model: Arc<LoadedSynModel>,
    /// Sampling-параметры основного Syn-чата (temperature переопределяется).
    params: SamplingParams,
    default_system_prompt: String,
    /// Готовые ChatTool-дескрипторы всех активных в чате тулов кроме `subagent`.
    active_tools: Vec<ChatTool>,
    /// Лимит tool-turn'ов цикла субагента — пользовательская настройка
    /// `general.subagent_max_turns`, склампленная к `>= 1`.
    max_turns: usize,
    /// Тот же abort, что у parent run_agent_loop (`SynChatCtx.abort`). Если
    /// пользователь нажмёт Stop — инкрементится, и мы между turn-ами это видим.
    abort: Arc<AtomicU64>,
    abort_baseline: u64,
}

async fn snapshot_from_main() -> Result<SubagentSnapshot, ToolError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        let syn = use_context::<SynChatCtx>();
        let reg = use_context::<SynModelRegistry>();
        // Модель может быть не загружена — тогда снимок невозможен.
        let snap = reg.current.get_untracked().map(|model| {
            let abort = syn.abort.clone();
            let abort_baseline = abort.load(Ordering::Relaxed);
            SubagentSnapshot {
                model,
                params: syn.params.get_untracked(),
                default_system_prompt: syn.system_prompt.get_untracked(),
                active_tools: build_active_tools_for_subagent(&app),
                max_turns: app.general.subagent_max_turns.get_untracked().max(1) as usize,
                abort,
                abort_baseline,
            }
        });
        let _ = tx.send(snap);
    });
    let snap = rx.await.map_err(|e| ToolError::Spawn(e.to_string()))?;
    snap.ok_or_else(|| ToolError::Spawn("модель не загружена".to_string()))
}

/// Собирает дескрипторы активных тулов для субагента: те же активные ключи,
/// что в основном чате, но с явным исключением `subagent` (рекурсия
/// запрещена) и с динамическим autoskill-обогащением.
fn build_active_tools_for_subagent(app: &AppCtx) -> Vec<ChatTool> {
    let keys = app.tools.active.get_untracked();
    keys.iter()
        .filter(|k| k.as_str() != KEY_SUBAGENT)
        .filter_map(|k| {
            if k == KEY_AUTOSKILL {
                Some(crate::agent::tool_flow::build_autoskill_chat_tool(app))
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
    let tool_schemas: Vec<serde_json::Value> = tools_list
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    // System: префикс с датой + либо пользовательский system_prompt из args,
    // либо общий из Syn-чата.
    let date = crate::agent::tool_flow::today_utc_iso(crate::agent::tool_flow::now_unix_secs());
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

    let mut history: Vec<Message> = Vec::with_capacity(4 + snap.max_turns * 2);
    history.push(Message::system(merged_system));
    history.push(Message::user(args.task.clone()));

    // Субагенту reasoning-шум в summary не нужен — thinking выключаем.
    let mut params = snap.params.clone();
    params.temperature = SUBAGENT_TEMPERATURE;
    params.enable_thinking = false;

    for turn in 0..snap.max_turns {
        if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
            tracing::info!(target: "subagent", id = %id, turn, "aborted by user");
            return Ok("Subagent прерван пользователем.".to_string());
        }

        tracing::debug!(
            target: "subagent",
            id = %id,
            turn,
            history_len = history.len(),
            tools = tool_schemas.len(),
            "request"
        );

        let out = generate_subagent_turn(
            &snap.model,
            &history,
            &tool_schemas,
            &params,
            &snap.abort,
            snap.abort_baseline,
        )
        .map_err(|e| ToolError::BadArgs(format!("LLM error: {e:#}")))?;

        if out.calls.is_empty() {
            let text = strip_thinking(&out.raw_text);
            tracing::debug!(target: "subagent", id = %id, turn, "final text");
            return Ok(text);
        }

        tracing::debug!(
            target: "subagent",
            id = %id,
            turn,
            n_tool_calls = out.calls.len(),
            "tool_calls"
        );

        // Ассистент-сообщение с полным сырым текстом (включая `<tool_call>`) —
        // в локальную историю для следующего turn.
        history.push(Message::assistant(out.raw_text.clone()));

        for (i, raw_call) in out.calls.iter().enumerate() {
            // Двойная защита от рекурсии: catalog уже исключает subagent из
            // дескрипторов, но модель всё равно может сгенерировать call с
            // таким именем. Не зовём executor (там был бы дальнейший
            // depth-check), сразу записываем error tool-result.
            if raw_call.name == KEY_SUBAGENT {
                history.push(Message::tool("ошибка: вложенный subagent запрещён"));
                tracing::warn!(target: "subagent", id = %id, "blocked nested subagent call");
                continue;
            }

            tracing::info!(
                target: "subagent",
                id = %id,
                tool = %raw_call.name,
                args_len = raw_call.arguments_json.len(),
                "exec"
            );

            let chat_call = ChatToolCall {
                id: format!("sub_{id}_{turn}_{i}"),
                kind: "function".to_string(),
                function: ChatToolCallFunction {
                    name: Some(raw_call.name.clone()),
                    arguments: Some(raw_call.arguments_json.clone()),
                },
            };

            // Auto-allow: пропускаем approval-диалог. Пользователь подтвердил
            // сам вызов subagent в основном цикле, дальше всё идёт без UI.
            // Box::pin — `executor::execute` через KEY_SUBAGENT возвращается
            // в этот же цикл; без боксинга компилятор не может вычислить
            // размер async-future. Реальной рекурсии нет (depth-чек в `run`).
            let outcome = tokio::select! {
                o = Box::pin(super::executor::execute(&chat_call)) => o,
                _ = wait_abort(&snap.abort, snap.abort_baseline) => {
                    return Ok("Subagent прерван пользователем.".to_string());
                }
            };
            history.push(Message::tool(outcome.content));
        }
    }

    tracing::info!(
        target: "subagent",
        id = %id,
        turns = snap.max_turns,
        "turn limit reached, forcing final summary"
    );

    // Финальный turn — без tools. Модель обязана ответить текстом.
    force_final_summary_turn(&snap, &mut history, &id).await
}

/// Результат одного нативного turn'а субагента.
struct SubagentTurn {
    /// Полный сырой текст ответа (с `<tool_call>`/`<think>` тегами как есть).
    raw_text: String,
    /// Распознанные tool-вызовы этого turn'а.
    calls: Vec<RawToolCall>,
}

/// Один turn нативной генерации субагента: prompt → generate_streaming →
/// parse tool_calls. Блокирующая (synaptix-генерация синхронна) — вызывается
/// из async-цикла, который её `await`-ит (нечему больше исполняться на
/// current-thread runtime worker'а).
fn generate_subagent_turn(
    model: &LoadedSynModel,
    history: &[Message],
    tool_schemas: &[serde_json::Value],
    params: &SamplingParams,
    abort: &Arc<AtomicU64>,
    abort_snapshot: u64,
) -> anyhow::Result<SubagentTurn> {
    let prompt = model.tokenizer.apply_chat_template_ex_tools(
        history,
        true,
        params.enable_thinking,
        if tool_schemas.is_empty() {
            None
        } else {
            Some(tool_schemas)
        },
    )?;
    let prompt_ids = model.tokenizer.encode(&prompt)?;

    // KV-ring cap — та же математика, что в syn_chat::session::run_agent_loop.
    let model_cap = model.model.config().max_seq_len;
    let mut opts = params.to_options();
    let usable_cap = model_cap.saturating_sub(1);
    let prompt_capped = prompt_ids.len().min(usable_cap);
    let headroom = usable_cap.saturating_sub(prompt_capped);
    if opts.max_new_tokens > headroom {
        opts.max_new_tokens = headroom.max(1);
    }
    opts.max_seq_len = (prompt_capped + opts.max_new_tokens + 128).min(usable_cap);

    let mut runner = LlmGeneration::new(&model.model, opts);
    crate::syn_chat::session::set_qwen3_stops(&mut runner, &model.tokenizer);
    if !tool_schemas.is_empty() {
        runner.add_stop_sequence(crate::syn_chat::session::TOOL_CALL_CLOSE);
    }

    let mut tool_parser = ToolCallParser::new();
    let mut raw_text = String::new();
    let abort_cb = abort.clone();
    runner.generate_streaming(&prompt_ids, &model.tokenizer, |_id, delta| {
        if abort_cb.load(Ordering::Relaxed) != abort_snapshot {
            return false;
        }
        raw_text.push_str(delta);
        let _ = tool_parser.feed(delta);
        // Зафиксирован tool_call и парсер вышел из блока — останавливаемся,
        // не дожидаясь, пока модель уйдёт писать прозу после блока.
        if tool_parser.calls_count() > 0 && tool_parser.is_outside() {
            return false;
        }
        true
    })?;
    drop(runner);
    if let synaptix_core::device::Device::Cuda(ordinal) = model.model.device() {
        let _ = synaptix::facade::llm::cuda_trim_pool(*ordinal as i32);
    }

    let (calls, _tail) = tool_parser.finish();
    Ok(SubagentTurn { raw_text, calls })
}

/// Вырезает `<think>…</think>` из финального текста — родителю уходит только
/// чистый ответ.
fn strip_thinking(raw: &str) -> String {
    let mut tp = ThinkParser::new();
    let split = tp.feed(raw);
    split.body.trim().to_string()
}

/// Поллер «дождаться abort» — оборачивается в `tokio::select!`, чтобы прервать
/// долгие async tool'ы (например web-fetch) при Stop от пользователя.
async fn wait_abort(abort: &Arc<AtomicU64>, snapshot: u64) {
    loop {
        if abort.load(Ordering::Relaxed) != snapshot {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}

/// Делает один финальный turn БЕЗ tools, чтобы модель сжала прогресс.
/// Если ответ всё-таки пуст — отдаём fallback-сообщение.
async fn force_final_summary_turn(
    snap: &SubagentSnapshot,
    history: &mut Vec<Message>,
    id: &str,
) -> Result<String, ToolError> {
    if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
        return Ok("Subagent прерван пользователем.".to_string());
    }

    history.push(Message::user(
        "Ты исчерпал лимит tool-вызовов. Не вызывай больше никаких tools. \
         Сожми накопленный прогресс в краткий итог 1-3 абзаца — что узнал, \
         что осталось неясным, какие ещё шаги нужны. Текст без преамбулы.",
    ));

    let mut params = snap.params.clone();
    params.temperature = SUBAGENT_TEMPERATURE;
    params.enable_thinking = false;

    let out = generate_subagent_turn(
        &snap.model,
        history,
        &[],
        &params,
        &snap.abort,
        snap.abort_baseline,
    )
    .map_err(|e| ToolError::BadArgs(format!("LLM error (final summary): {e:#}")))?;

    let summary = strip_thinking(&out.raw_text);
    if summary.is_empty() {
        tracing::warn!(target: "subagent", id = %id, "final summary empty");
        let limit = snap.max_turns;
        Ok(format!(
            "Subagent достиг лимита в {limit} turn-ов и не сошёлся \
             к финальному ответу. Сформулируй задачу более узко или вызови \
             subagent повторно с уже имеющимся прогрессом."
        ))
    } else {
        tracing::info!(target: "subagent", id = %id, summary_len = summary.len(), "final summary ok");
        Ok(summary)
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
            function: crate::agent::schema::ToolFunctionSchema {
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
