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
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use synaptix::facade::llm::{LlmGeneration, Message};

use crate::agent::schema::{ChatTool, ChatToolCall, ChatToolCallFunction};
use crate::context::AppCtx;
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::session::{
    RingPlan, MAX_OOM_RETRIES, MIN_ANSWER_TOKENS, RING_ANSWER_TOKENS,
};
use crate::syn_chat::state::{SynChatCtx, ThinkParser};
use crate::syn_chat::telemetry::{self, RunKind, RunState};
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
    /// id карточки телеметрии текущего цикла. Вложенный вызов берёт его
    /// как `parent`, и панель «Детали» рисует дерево, а не плоский список.
    static SUBAGENT_RUN: u64;
}

/// Как часто живые метрики цикла уезжают в UI. Каждый токен дергал бы
/// redraw; 200 мс — предел, на котором цифры ещё выглядят «бегущими».
const TELEMETRY_INTERVAL: Duration = Duration::from_millis(200);

/// Задача субагента в заголовке карточки: длинный prompt в панель не
/// влезает, а первой строки хватает, чтобы понять, кто сейчас работает.
const LABEL_CHARS: usize = 120;

fn short_label(task: &str) -> String {
    let one_line = task.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= LABEL_CHARS {
        return one_line;
    }
    let head: String = one_line.chars().take(LABEL_CHARS).collect();
    format!("{head}…")
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
                    "nested subagent is forbidden: remove \"subagent\" from tools".to_string(),
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
    snap.ok_or_else(|| ToolError::Spawn("model not loaded".to_string()))
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
            "nested subagent is forbidden (max depth = 1)".to_string(),
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

    // Карточка в панели «Детали». Родитель — цикл, из которого нас позвали:
    // для вызова из основного чата это `ROOT_RUN`, для вложенного (если
    // лимит глубины когда-нибудь поднимут) — id внешнего субагента.
    let parent = SUBAGENT_RUN.try_with(|v| *v).unwrap_or(telemetry::ROOT_RUN);
    let run_id = telemetry::begin(
        parent,
        depth + 1,
        RunKind::Subagent,
        short_label(&args.task),
        snap.max_turns as u32,
    );
    let abort = snap.abort.clone();
    let abort_baseline = snap.abort_baseline;

    let result = SUBAGENT_DEPTH
        .scope(
            depth + 1,
            SUBAGENT_RUN.scope(run_id, run_subagent_loop(snap, args, id.clone(), run_id)),
        )
        .await;

    // Прерывание пользователем цикл возвращает как Ok(текст), поэтому
    // состояние карточки решаем по abort-счётчику, а не по Result.
    let aborted = abort.load(Ordering::Relaxed) != abort_baseline;
    match &result {
        Ok(text) => {
            telemetry::finish(
                run_id,
                if aborted { RunState::Aborted } else { RunState::Done },
            );
            tracing::info!(
                target: "subagent",
                id = %id,
                output_len = text.len(),
                "end ok"
            );
        }
        Err(e) => {
            telemetry::finish(run_id, RunState::Failed);
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
    run_id: u64,
) -> Result<String, ToolError> {
    let tools_list = select_tools(&snap.active_tools, args.tools.as_deref());
    let tool_schemas: Vec<serde_json::Value> = tools_list
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    // System: префикс с датой + либо пользовательский system_prompt из args,
    // либо общий из Syn-чата.
    let date = crate::agent::tool_flow::today_utc_iso(crate::agent::tool_flow::now_unix_secs());
    let preamble = format!("Current date (UTC): {date}.");
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

    // Карточка панели показывает сумму токенов по всему циклу, а не по
    // одному turn'у — как и у основного чата.
    let mut gen_total: u32 = 0;

    for turn in 0..snap.max_turns {
        if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
            tracing::info!(target: "subagent", id = %id, turn, "aborted by user");
            return Ok("Subagent was interrupted by the user.".to_string());
        }
        let turn_no = turn as u32 + 1;
        telemetry::patch(run_id, move |r| {
            r.turn = turn_no;
            r.tool = None;
        });

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
            &id,
            turn,
            (run_id, gen_total),
        )
        .map_err(|e| ToolError::Runtime(format!("subagent LLM error: {e:#}")))?;
        gen_total += out.gen_tokens;

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
                history.push(Message::tool("error: nested subagent is forbidden"));
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

            // Пока тул работает, генерации нет — без этой пометки карточка
            // замирает на прежних числах и выглядит как повисшая.
            let running_tool = raw_call.name.clone();
            telemetry::patch(run_id, move |r| r.tool = Some(running_tool));

            // Auto-allow: пропускаем approval-диалог. Пользователь подтвердил
            // сам вызов subagent в основном цикле, дальше всё идёт без UI.
            // Box::pin — `executor::execute` через KEY_SUBAGENT возвращается
            // в этот же цикл; без боксинга компилятор не может вычислить
            // размер async-future. Реальной рекурсии нет (depth-чек в `run`).
            let outcome = tokio::select! {
                o = Box::pin(super::executor::execute(&chat_call)) => o,
                _ = wait_abort(&snap.abort, snap.abort_baseline) => {
                    return Ok("Subagent was interrupted by the user.".to_string());
                }
            };
            telemetry::patch(run_id, |r| {
                r.tool = None;
                r.tool_calls += 1;
            });
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
    force_final_summary_turn(&snap, &mut history, &id, (run_id, gen_total)).await
}

/// Результат одного нативного turn'а субагента.
struct SubagentTurn {
    /// Полный сырой текст ответа (с `<tool_call>`/`<think>` тегами как есть).
    raw_text: String,
    /// Распознанные tool-вызовы этого turn'а.
    calls: Vec<RawToolCall>,
    /// Сколько токенов модель выдала за этот turn.
    gen_tokens: u32,
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
    id: &str,
    turn: usize,
    // (id карточки в панели «Детали», токены, накопленные прошлыми turn'ами)
    run: (u64, u32),
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

    // План KV-ринга — тот же, что у основного цикла (`RingPlan`), а не «cap
    // модели минус промпт». Слайдер `max_new_tokens` стоит в конфиге на
    // 131072: без `RING_ANSWER_TOKENS` ринг вырастал до ~132k токенов, что у
    // гибрида 27B (33 КБ на токен) даёт 4+ ГБ VRAM под один ход субагента, а
    // сам ход тянется до этих 131k токенов — часы генерации на один turn.
    // Бюджет по доступной VRAM тут игнорировался полностью.
    let model_cap = model.model.config().max_seq_len;
    let mut answer_budget = (params.max_new_tokens as usize).min(RING_ANSWER_TOKENS);
    let mut oom_attempt = 0usize;

    loop {
        let plan = RingPlan::new(model, prompt_ids.len(), answer_budget, model_cap);
        let mut opts = params.to_options();
        opts.max_seq_len = plan.ring_tokens;
        opts.max_new_tokens = plan.max_new;
        tracing::info!(
            target: "subagent",
            id = %id,
            turn,
            ring_tokens = plan.ring_tokens,
            ring_mb = plan.ring_mb(),
            prompt_tokens = plan.prompt_tokens,
            max_new = plan.max_new,
            by_mem = plan.by_mem,
            "KV-ринг"
        );

        // Размер ринга и потолок контекста в панель — сразу, до генерации:
        // это уже готовые числа, а ждать первого токена можно долго.
        let (run_id, gen_before) = run;
        let plan_stats = (
            plan.prompt_tokens as u32,
            plan.ring_tokens as u32,
            plan.ring_bytes(),
            plan.by_mem.min(plan.cap) as u32,
        );
        telemetry::patch(run_id, move |r| {
            r.stats.prompt_tokens = plan_stats.0;
            r.stats.ring_tokens = plan_stats.1;
            r.stats.ring_bytes = plan_stats.2;
            r.stats.ctx_budget = plan_stats.3;
            // Префикс-KV в субагенте не используется — весь промпт
            // префиллится заново каждый turn.
            r.stats.reused_tokens = 0;
        });

        let mut runner = LlmGeneration::new(&model.model, opts);
        crate::syn_chat::session::set_qwen3_stops(&mut runner, &model.tokenizer);
        if !tool_schemas.is_empty() {
            runner.add_stop_sequence(crate::syn_chat::session::TOOL_CALL_CLOSE);
        }

        let mut tool_parser = ToolCallParser::new();
        let mut raw_text = String::new();
        let mut tokens_this_turn = 0usize;
        let mut ttft_ms: Option<u32> = None;
        let t_turn = Instant::now();
        let mut last_push = Instant::now();
        let abort_cb = abort.clone();
        let stream_res = runner.generate_streaming(&prompt_ids, &model.tokenizer, |_id, delta| {
            if abort_cb.load(Ordering::Relaxed) != abort_snapshot {
                return false;
            }
            tokens_this_turn += 1;
            if ttft_ms.is_none() {
                // Честный префилл — время до первого токена, как в основном
                // цикле (замер до старта показывал бы планирование ринга).
                ttft_ms = Some(t_turn.elapsed().as_millis() as u32);
            }
            raw_text.push_str(delta);
            let _ = tool_parser.feed(delta);

            let now = Instant::now();
            if now.duration_since(last_push) >= TELEMETRY_INTERVAL {
                last_push = now;
                push_live_stats(run_id, gen_before, tokens_this_turn, ttft_ms, t_turn);
            }
            // Зафиксирован tool_call и парсер вышел из блока — останавливаемся,
            // не дожидаясь, пока модель уйдёт писать прозу после блока.
            if tool_parser.calls_count() > 0 && tool_parser.is_outside() {
                return false;
            }
            true
        });
        // Возврат VRAM после хода — так же, как в основном цикле: одного
        // трима пула мало, сегменты держат мёртвые записи кэша
        // TMA-дескрипторов, и за ход субагента так утекал больше гигабайта
        // (на длинной цепочке ходов это заканчивалось OOM на активациях).
        drop(runner);
        crate::syn_chat::model_registry::reclaim_vram(*model.model.device());

        if let Err(e) = stream_res {
            // Тот же ретрай, что в основном цикле: ринг вдвое короче и заново.
            let retryable = crate::syn_chat::session::is_oom_error(&e)
                && tokens_this_turn == 0
                && oom_attempt < MAX_OOM_RETRIES
                && answer_budget > MIN_ANSWER_TOKENS;
            if retryable {
                oom_attempt += 1;
                answer_budget = (answer_budget / 2).max(MIN_ANSWER_TOKENS);
                tracing::warn!(
                    target: "subagent",
                    id = %id,
                    turn,
                    ring_tokens = plan.ring_tokens,
                    attempt = oom_attempt,
                    answer_budget,
                    "OOM на ринге, повтор с меньшим бюджетом: {e}"
                );
                continue;
            }
            return Err(e.into());
        }

        tracing::debug!(
            target: "subagent",
            id = %id,
            turn,
            gen_tokens = tokens_this_turn,
            "turn done"
        );
        push_live_stats(run_id, gen_before, tokens_this_turn, ttft_ms, t_turn);
        let vram_free = crate::syn_chat::model_registry::vram_available_mb() as u32;
        telemetry::patch(run_id, move |r| r.stats.vram_free_mb = vram_free);
        let (calls, _tail) = tool_parser.finish();
        return Ok(SubagentTurn {
            raw_text,
            calls,
            gen_tokens: tokens_this_turn as u32,
        });
    }
}

/// Сбрасывает в карточку панели счётчик токенов и скорость декода. Зовётся
/// throttled'но из стрима (см. [`TELEMETRY_INTERVAL`]) и один раз в конце
/// turn'а.
fn push_live_stats(
    run_id: u64,
    gen_before: u32,
    tokens_this_turn: usize,
    ttft_ms: Option<u32>,
    t_turn: Instant,
) {
    // Скорость считаем по чистому декоду — за вычетом префилла, иначе
    // длинный первый токен занижает tps в разы.
    let elapsed = t_turn.elapsed().as_secs_f64();
    let decode_s = elapsed - ttft_ms.unwrap_or(0) as f64 / 1000.0;
    let tps = if decode_s > 0.0 {
        (tokens_this_turn as f64 / decode_s) as f32
    } else {
        0.0
    };
    let gen = gen_before + tokens_this_turn as u32;
    let prefill = ttft_ms.unwrap_or(0);
    telemetry::patch(run_id, move |r| {
        r.stats.gen_tokens = gen;
        r.stats.decode_tps = tps;
        r.stats.prefill_ms = prefill;
    });
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
    run: (u64, u32),
) -> Result<String, ToolError> {
    if snap.abort.load(Ordering::Relaxed) != snap.abort_baseline {
        return Ok("Subagent was interrupted by the user.".to_string());
    }

    history.push(Message::user(
        "You've exhausted the tool-call limit. Don't call any more tools. \
         Compress your accumulated progress into a brief summary of 1-3 \
         paragraphs — what you learned, what remains unclear, what further \
         steps are needed. Text with no preamble.",
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
        id,
        snap.max_turns,
        run,
    )
    .map_err(|e| ToolError::Runtime(format!("subagent LLM error (final summary): {e:#}")))?;

    let summary = strip_thinking(&out.raw_text);
    if summary.is_empty() {
        tracing::warn!(target: "subagent", id = %id, "final summary empty");
        let limit = snap.max_turns;
        Ok(format!(
            "Subagent reached the limit of {limit} turns and didn't converge \
             on a final answer. Phrase the task more narrowly or call \
             subagent again with the progress already made."
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
