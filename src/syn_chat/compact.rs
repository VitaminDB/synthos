//! Autocompact Syn-чата: автоматическое и ручное сжатие старых сообщений в
//! `system`-summary.
//!
//! Архитектура:
//! - Pure-функции (этот файл): `find_compact_range`, `next_iteration`,
//!   `serialize_for_summary`, `apply_compaction_to_messages`. Покрыты
//!   юнит-тестами.
//! - Async-оркестратор `run_compaction` сериализует старую часть ленты,
//!   зовёт [`session::generate_summary`] (in-process генерация через
//!   synaptix, без tools и стрима), потом на main-потоке атомарно обновляет
//!   `ctx.messages`: помечает свёрнутые `compacted_iter=Some(N)` и вставляет
//!   `ChatMsg::compaction_marker` перед ними.
//! - Автотриггер — [`maybe_autocompact`]: worker `session::start_agent_thread`
//!   зовёт его после завершения agent-loop, когда промпт последнего хода
//!   превысил `autocompact_threshold_percent` от лимита контекста.
//!   Ручная кнопка в шапке чата — [`compact_now`].
//!
//! Маркер превращается в `system`-сообщение для модели в
//! `session::build_history`; свёрнутые сообщения в prompt не идут там же.

use syngui::tr;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use crate::context::AppCtx;
use crate::syn_chat::model_registry::LoadedSynModel;
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, ChatMsgRole, SynChatCtx};
use crate::syn_chat::{session, storage};

/// Что вызвало компактификацию: автоматический триггер по проценту контекста
/// или ручная кнопка пользователя.
#[derive(Debug, Clone, Copy)]
pub enum CompactionTrigger {
    /// Автоматический триггер. `tokens_before` — промпт последнего хода в
    /// токенах (снимок `ctx.last_prompt_tokens`).
    Auto { tokens_before: i64 },
    /// Ручная кнопка «Сжать контекст». Точное значение токенов до сжатия
    /// неизвестно, в маркер пишем 0 — UI рисует «≈N токенов» вместо пары.
    Manual,
}

impl CompactionTrigger {
    pub fn tokens_before(self) -> i64 {
        match self {
            CompactionTrigger::Auto { tokens_before } => tokens_before,
            CompactionTrigger::Manual => 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure-логика поиска диапазона и итерации
// ─────────────────────────────────────────────────────────────────────────────

/// Найти диапазон сообщений, которые можно свернуть.
///
/// Стратегия — «свежим» считается всё после последнего user-Text сообщения
/// (включительно). Сжимаем всё ДО него. Гарантирует атомарность tool-пар:
/// если правая граница попадает на `ToolCall` без своего `ToolResult` —
/// сдвигаем `end` влево до конца последнего полного блока. Аналогично
/// сдвигаем `start` вправо, если он попадает на `ToolResult` без
/// предшествующего `ToolCall`.
///
/// Возвращает `None`, если кандидатов меньше двух (нет смысла сжимать).
pub fn find_compact_range(msgs: &[ChatMsg]) -> Option<Range<usize>> {
    if msgs.len() < 2 {
        return None;
    }

    // 1. Найти последний user-Text — это правая граница (исключительно).
    let pivot = msgs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| {
            matches!(m.role, ChatMsgRole::User)
                && matches!(m.kind, ChatMsgKind::Text)
                && m.compacted_iter.is_none()
        })
        .map(|(i, _)| i)?;
    if pivot == 0 {
        return None;
    }

    // 2. Левая граница = первый ещё-не-свёрнутый и не-маркер индекс.
    let mut start = 0usize;
    while start < pivot
        && (msgs[start].compacted_iter.is_some()
            || matches!(msgs[start].kind, ChatMsgKind::CompactionMarker { .. }))
    {
        start += 1;
    }

    refine_range(msgs, start, pivot, pivot)
}

/// Диапазон для компактификации ВНУТРИ хода агента: сжимаем tool-цепочку
/// текущего сообщения, оставляя нетронутыми сам запрос пользователя и
/// последние `keep_tail` сообщений ленты.
///
/// Нужен потому, что [`find_compact_range`] по определению бессилен внутри
/// хода: он считает «свежим» всё после последнего user-сообщения, а у агента
/// именно там и живут десятки tool-вызовов, которые раздувают контекст.
pub fn find_compact_range_in_turn(msgs: &[ChatMsg], keep_tail: usize) -> Option<Range<usize>> {
    let pivot = msgs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| {
            matches!(m.role, ChatMsgRole::User)
                && matches!(m.kind, ChatMsgKind::Text)
                && m.compacted_iter.is_none()
        })
        .map(|(i, _)| i)?;

    // Левая граница — сразу после запроса пользователя: сам запрос в сводку
    // уходить не должен, агент обязан видеть задачу дословно.
    let mut start = pivot + 1;
    while start < msgs.len()
        && (msgs[start].compacted_iter.is_some()
            || matches!(msgs[start].kind, ChatMsgKind::CompactionMarker { .. }))
    {
        start += 1;
    }
    let end = msgs.len().checked_sub(keep_tail)?;
    if end <= start {
        return None;
    }
    refine_range(msgs, start, end, msgs.len())
}

/// Общая часть обоих поисков: атомарность tool-пар по обеим границам и
/// проверка, что сжимать вообще есть что.
///
/// `scan_end` — до какого индекса можно искать `ToolResult` для `ToolCall`'а
/// с правой границы (для межходовой компактификации это pivot, для
/// внутриходовой — конец ленты).
fn refine_range(
    msgs: &[ChatMsg],
    mut start: usize,
    mut end: usize,
    scan_end: usize,
) -> Option<Range<usize>> {
    // 3. Атомарность tool-пар по правой границе. Если последний кандидат —
    //    `ToolCall`, его `ToolResult` лежит ПОСЛЕ границы или вообще
    //    отсутствует — сжимать такой ToolCall нельзя (в prompt будет stub).
    //    Сдвигаем `end` назад, пока хвост — это незакрытый ToolCall.
    while end > start {
        let last = &msgs[end - 1];
        let needs_shrink = match &last.kind {
            ChatMsgKind::ToolCall { .. } => {
                if let Some(calls) = &last.tool_calls {
                    calls.iter().any(|c| {
                        !msgs[end..scan_end].iter().any(|mm| matches!(
                            &mm.kind,
                            ChatMsgKind::ToolResult { tool_call_id, .. } if tool_call_id == &c.id
                        ))
                    })
                } else {
                    true
                }
            }
            _ => false,
        };
        if needs_shrink {
            end -= 1;
        } else {
            break;
        }
    }

    if end <= start {
        return None;
    }

    // 4. Атомарность tool-пар по левой границе: `start` на ToolResult без
    //    своего ToolCall в [start..end] — сдвигаем вправо.
    while start < end {
        let first = &msgs[start];
        if let ChatMsgKind::ToolResult { tool_call_id, .. } = &first.kind {
            let has_call = msgs[start..end].iter().any(|mm| match &mm.kind {
                ChatMsgKind::ToolCall { .. } => mm
                    .tool_calls
                    .as_ref()
                    .map(|cs| cs.iter().any(|c| &c.id == tool_call_id))
                    .unwrap_or(false),
                _ => false,
            });
            if !has_call {
                start += 1;
                continue;
            }
        }
        break;
    }

    if end <= start {
        return None;
    }

    // 5. Считаем «полезные» (не-маркер, не уже-свёрнутые) сообщения в диапазоне.
    let useful = msgs[start..end]
        .iter()
        .filter(|m| {
            m.compacted_iter.is_none()
                && !matches!(m.kind, ChatMsgKind::CompactionMarker { .. })
        })
        .count();
    if useful < 2 {
        return None;
    }

    Some(start..end)
}

/// Следующий номер итерации = `max(существующих) + 1`. Учитывает и
/// `CompactionMarker.iteration`, и `compacted_iter` помеченных сообщений.
pub fn next_iteration(msgs: &[ChatMsg]) -> u32 {
    let from_marker = msgs
        .iter()
        .filter_map(|m| match &m.kind {
            ChatMsgKind::CompactionMarker { iteration, .. } => Some(*iteration),
            _ => None,
        })
        .max();
    let from_field = msgs.iter().filter_map(|m| m.compacted_iter).max();
    from_marker.max(from_field).map(|n| n + 1).unwrap_or(1)
}

/// Сериализовать диапазон сообщений для summary-запроса. Урезаем длинные
/// tool-результаты, чтобы запрос сам не упёрся в лимит модели.
pub fn serialize_for_summary(msgs: &[ChatMsg], range: Range<usize>) -> String {
    const MAX_TOOL_BODY: usize = 1500;
    const MAX_TOTAL: usize = 16 * 1024;

    let mut buf = String::new();
    for m in &msgs[range] {
        // Свёрнутые предыдущей итерацией — пропускаем; они уже учтены в
        // более раннем `CompactionMarker`-summary.
        if m.compacted_iter.is_some() {
            continue;
        }
        match &m.kind {
            ChatMsgKind::CompactionMarker { iteration, summary, .. } => {
                buf.push_str(&format!(
                    "[Previously compacted block (iteration {iteration})]: {summary}\n\n"
                ));
            }
            ChatMsgKind::Text => {
                let prefix = match m.role {
                    ChatMsgRole::User => "User:",
                    ChatMsgRole::Assistant => "Assistant:",
                    ChatMsgRole::System => "System:",
                };
                if !m.attachments.is_empty() {
                    buf.push_str(&format!(
                        "{prefix} [attached {} attachment(s)]\n",
                        m.attachments.len()
                    ));
                }
                if !m.body.trim().is_empty() {
                    let body = storage::truncate_chars(m.body.trim(), MAX_TOOL_BODY);
                    buf.push_str(&format!("{prefix} {body}\n"));
                }
                buf.push('\n');
            }
            ChatMsgKind::ToolCall { tool_name } => {
                let args = storage::truncate_chars(m.body.trim(), 400);
                buf.push_str(&format!(
                    "Assistant called tool `{tool_name}`(args={args}).\n\n"
                ));
            }
            ChatMsgKind::ToolResult { tool_name, error, .. } => {
                let body = m.body.trim();
                let truncated = storage::truncate_chars(body, MAX_TOOL_BODY);
                let suffix = if body.chars().count() > MAX_TOOL_BODY {
                    format!(" [result truncated to {MAX_TOOL_BODY} characters]")
                } else {
                    String::new()
                };
                let status = if *error { "error" } else { "ok" };
                buf.push_str(&format!(
                    "Tool `{tool_name}` ({status}): {truncated}{suffix}\n\n"
                ));
            }
        }

        if buf.chars().count() > MAX_TOTAL {
            buf.push_str("\n[…history truncated for the summary request…]\n");
            break;
        }
    }
    buf
}

/// Применить компактификацию к ленте: пометить диапазон `compacted_iter=Some(iter)`
/// и вставить маркер ПЕРЕД диапазоном. Pure для тестов.
pub fn apply_compaction_to_messages(
    msgs: &mut Vec<ChatMsg>,
    range: Range<usize>,
    iteration: u32,
    tokens_before: i64,
    tokens_after: i64,
    summary: String,
) {
    let compacted_count = msgs[range.clone()]
        .iter()
        .filter(|m| m.compacted_iter.is_none())
        .count();
    for m in &mut msgs[range.clone()] {
        if m.compacted_iter.is_none() {
            m.compacted_iter = Some(iteration);
        }
    }
    let marker = ChatMsg::compaction_marker(
        iteration,
        compacted_count,
        tokens_before,
        tokens_after,
        summary,
    );
    msgs.insert(range.start, marker);
}

// ─────────────────────────────────────────────────────────────────────────────
// Async-оркестратор + публичные действия
// ─────────────────────────────────────────────────────────────────────────────

/// Промпт для модели-суммаризатора. Жёстко зашит — это часть контракта,
/// чтобы поведение autocompact'а было предсказуемым независимо от
/// пользовательского `system_prompt`.
const SUMMARY_SYSTEM_PROMPT: &str = "You compress a dialogue fragment into a brief \
technical description: key decisions, file names, tool-call outcomes, \
open questions. Keep the specifics (numbers, identifiers, names). \
Don't retell verbatim. Write neutrally, in 7-15 short paragraphs.";

fn summary_prompt() -> String {
    let lang = syngui::i18n::language();
    let name = syngui::i18n::languages()
        .into_iter()
        .find(|l| l.tag == lang)
        .and_then(|l| l.english)
        .unwrap_or_else(|| "English".to_string());
    format!("{SUMMARY_SYSTEM_PROMPT} Write in {name}.")
}

/// Точка входа из ручной кнопки UI (main thread). Не блокирует UI: спавнит
/// worker-поток с current-thread tokio runtime — как `start_agent_thread`,
/// потому что генерация synaptix блокирует поток. Держит `ctx.pending`,
/// чтобы кнопка и отправка сообщений были disabled на время сжатия.
pub fn compact_now() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let registry = use_context::<crate::syn_chat::SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some(tr!("chat.model.not_loaded")));
        return;
    };
    let abort = ctx.abort.clone();
    let snapshot = abort.load(Ordering::Relaxed);
    ctx.pending.set(true);
    let ctx_done = ctx.clone();
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(run_compaction(
                model,
                abort,
                snapshot,
                CompactionTrigger::Manual,
            )),
            Err(e) => {
                log::error!("[syn_chat] compact: не удалось создать tokio runtime: {e:#}");
            }
        }
        run_on_main_thread(move || ctx_done.pending.set(false));
    });
}

/// Автотриггер: сравнить промпт последнего хода с лимитом контекста и, если
/// порог превышен, запустить компактификацию. Зовётся из worker-потока
/// `start_agent_thread` после завершения agent-loop (модель свободна,
/// `ctx.pending` ещё держится — UI заблокирован от повторной отправки).
pub async fn maybe_autocompact(
    model: &Arc<LoadedSynModel>,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
    threshold_percent: u32,
) {
    let Some((prompt_tokens, budget, max_seq_len)) = read_ctx_usage_on_main().await else {
        return;
    };
    if prompt_tokens == 0 {
        return;
    }
    // Лимит = что раньше кончится: окно модели, `max_seq_len` из настроек
    // или VRAM-бюджет последнего хода (`RingPlan.by_mem`; 0 — ход ещё не
    // считался). Бюджет — с учётом оффлоада: блоки, которые ещё на карте,
    // `fit_blocks_for_context` отправит на хост и освободит VRAM под KV, так
    // что ход на таком контексте пройдёт — медленнее, но пройдёт. Без этой
    // поправки сжатие срабатывало там, где ход прекрасно работал
    // (07.09.2026: 77k токенов при «лимите» 87k свернулись в сводку на 594
    // токена, и модель дальше отвечала на вопросы по документу выдумкой).
    let model_cap = model.model.config().max_seq_len.saturating_sub(1) as u32;
    let offloadable_tokens: u32 = model
        .model
        .block_offload_shape()
        .map(|(block_bytes, total)| {
            let resident = model.model.resident_blocks().unwrap_or(total);
            let kv = model.model.kv_bytes_per_token().max(1) as u64;
            (resident as u64 * block_bytes as u64 / kv).min(u32::MAX as u64) as u32
        })
        .unwrap_or(0);
    let mut effective = model_cap;
    if max_seq_len > 0 {
        effective = effective.min(max_seq_len);
    }
    if budget > 0 {
        effective = effective.min(budget.saturating_add(offloadable_tokens));
    }
    if effective == 0 {
        return;
    }
    let pct = prompt_tokens.saturating_mul(100) / effective;
    if pct < threshold_percent {
        return;
    }
    log::info!(
        "[syn_chat] autocompact: промпт {prompt_tokens} ток = {pct}% от лимита \
         {effective} (порог {threshold_percent}%) — запускаем сжатие"
    );
    run_compaction(
        model.clone(),
        abort.clone(),
        snapshot,
        CompactionTrigger::Auto {
            tokens_before: prompt_tokens as i64,
        },
    )
    .await;
}

/// Что именно сжимаем: историю между ходами или tool-цепочку внутри
/// текущего хода агента.
#[derive(Debug, Clone, Copy)]
pub enum CompactScope {
    /// Всё до последнего сообщения пользователя ([`find_compact_range`]).
    BetweenTurns,
    /// Tool-цепочка текущего хода, кроме последних `keep_tail` сообщений
    /// ([`find_compact_range_in_turn`]).
    InTurn { keep_tail: usize },
}

impl CompactScope {
    fn range(self, msgs: &[ChatMsg]) -> Option<Range<usize>> {
        match self {
            CompactScope::BetweenTurns => find_compact_range(msgs),
            CompactScope::InTurn { keep_tail } => find_compact_range_in_turn(msgs, keep_tail),
        }
    }
}

/// Автотриггер ВНУТРИ хода агента: зовётся из `run_agent_loop` между ходами
/// одного сообщения, когда промпт перевалил за порог. Возвращает `true`,
/// если лента была сжата — вызывающий обязан пересобрать свою историю.
///
/// Отдельная точка входа нужна потому, что межходовой автокомпакт внутри хода
/// бессилен: `find_compact_range` не трогает ничего после последнего
/// user-сообщения, а у агента весь рост контекста именно там.
pub async fn maybe_compact_in_turn(
    model: &Arc<LoadedSynModel>,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
    threshold_percent: u32,
    keep_tail: usize,
    prompt_tokens: u32,
    budget_tokens: u32,
) -> bool {
    if prompt_tokens == 0 || budget_tokens == 0 {
        return false;
    }
    let pct = prompt_tokens.saturating_mul(100) / budget_tokens;
    if pct < threshold_percent {
        return false;
    }
    log::info!(
        "[syn_chat] autocompact внутри хода: промпт {prompt_tokens} ток = {pct}% \
         от лимита {budget_tokens} (порог {threshold_percent}%) — сжимаем \
         tool-цепочку текущего хода"
    );
    run_compaction_scoped(
        model.clone(),
        abort.clone(),
        snapshot,
        CompactionTrigger::Auto { tokens_before: prompt_tokens as i64 },
        CompactScope::InTurn { keep_tail },
    )
    .await
}

/// Главный async-оркестратор. Идемпотентен: при отсутствии кандидатов —
/// просто возвращается. Все ошибки логируются + notification, состояние
/// ленты НЕ модифицируется.
pub async fn run_compaction(
    model: Arc<LoadedSynModel>,
    abort: Arc<AtomicU64>,
    snapshot: u64,
    trigger: CompactionTrigger,
) {
    run_compaction_scoped(model, abort, snapshot, trigger, CompactScope::BetweenTurns).await;
}

/// Тело оркестратора, параметризованное областью сжатия. `true` — сжатие
/// применено к ленте.
async fn run_compaction_scoped(
    model: Arc<LoadedSynModel>,
    abort: Arc<AtomicU64>,
    snapshot: u64,
    trigger: CompactionTrigger,
    scope: CompactScope,
) -> bool {
    if abort.load(Ordering::Relaxed) != snapshot {
        return false;
    }

    // 1. Снимаем snapshot ленты на main и считаем диапазон.
    let Some((msgs_snapshot, range, iteration)) = read_compact_plan_on_main(scope).await else {
        log::debug!("[syn_chat] autocompact: кандидатов нет, пропуск");
        return false;
    };
    if abort.load(Ordering::Relaxed) != snapshot {
        return false;
    }

    let block = serialize_for_summary(&msgs_snapshot, range);
    if block.trim().is_empty() {
        return false;
    }

    push_snackbar(tr!("chat.compact.running", iteration = iteration), false);

    // 2. Summary через in-process генерацию. Кэш префикс-KV сбрасываем до
    //    неё: после компактификации история всё равно перестанет совпадать с
    //    посчитанным префиксом, а его VRAM пригодится summary-запросу.
    //    Внутри хода слот держит сам agent-loop — он его и освобождает, а
    //    `drop_kv_session` тут только повесил бы отложенную очистку.
    if matches!(scope, CompactScope::BetweenTurns) {
        session::drop_kv_session();
    }
    let prompt = summary_prompt();
    let summary = match session::generate_summary(
        &model,
        &prompt,
        &block,
        &abort,
        snapshot,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[syn_chat] autocompact: summary-запрос упал: {e:#}");
            push_snackbar(tr!("chat.compact.failed", error = e), true);
            return false;
        }
    };
    if abort.load(Ordering::Relaxed) != snapshot {
        return false;
    }
    if summary.is_empty() {
        log::warn!("[syn_chat] autocompact: модель вернула пустой summary");
        push_snackbar(
            tr!("chat.compact.empty"),
            true,
        );
        return false;
    }

    // 3. Токены summary — честно через токенизатор, fallback на chars/4.
    let tokens_after = model
        .tokenizer
        .encode(&summary)
        .map(|ids| ids.len() as i64)
        .unwrap_or_else(|_| (summary.chars().count() as i64 / 4).max(1));

    // 4. На main: применяем компактификацию атомарно. Диапазон пересчитываем
    //    по актуальной ленте — пользователь мог успеть дописать сообщение.
    let tokens_before = trigger.tokens_before();
    let abort_c = abort.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    run_on_main_thread(move || {
        if abort_c.load(Ordering::Relaxed) != snapshot {
            let _ = tx.send(false);
            return;
        }
        let ctx = use_context::<SynChatCtx>();
        let mut applied_ok = false;
        ctx.messages.update(|v| {
            if let Some(fresh_range) = scope.range(v) {
                let iter = next_iteration(v);
                apply_compaction_to_messages(
                    v,
                    fresh_range,
                    iter,
                    tokens_before,
                    tokens_after,
                    summary.clone(),
                );
                applied_ok = true;
            }
        });
        let _ = tx.send(applied_ok);
    });
    let applied = rx.await.unwrap_or(false);

    if applied {
        let msg = if tokens_before > 0 {
            let saved = (tokens_before - tokens_after).max(0);
            tr!("chat.compact.done", before = tokens_before, after = tokens_after, saved = saved)
        } else {
            tr!("chat.compact.done_short", after = tokens_after)
        };
        push_snackbar(msg, false);
    }
    applied
}

/// Читает на main-потоке текущую ленту, диапазон для сжатия и следующий
/// номер итерации. `None` если кандидатов нет.
async fn read_compact_plan_on_main(
    scope: CompactScope,
) -> Option<(Vec<ChatMsg>, Range<usize>, u32)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        let msgs = ctx.messages.get_untracked();
        let plan = scope.range(&msgs).map(|range| {
            let iteration = next_iteration(&msgs);
            (msgs.clone(), range, iteration)
        });
        let _ = tx.send(plan);
    });
    rx.await.ok().flatten()
}

/// Читает на main-потоке промпт последнего хода и VRAM-бюджет контекста
/// (сигналы таба «Детали», обновляются `TurnStats::apply` на каждом ходу).
/// `(промпт последнего хода, бюджет VRAM в токенах, max_seq_len из настроек)`.
async fn read_ctx_usage_on_main() -> Option<(u32, u32, u32)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        let _ = tx.send((
            ctx.last_prompt_tokens.get_untracked(),
            ctx.ctx_budget_tokens.get_untracked(),
            ctx.params.get_untracked().max_seq_len,
        ));
    });
    rx.await.ok()
}

/// Положить notification на main-потоке. is_error → error severity, иначе info.
fn push_snackbar(msg: String, is_error: bool) {
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        if is_error {
            app.notifications.error(msg);
        } else {
            app.notifications.info(msg);
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};

    fn user(body: &str) -> ChatMsg {
        ChatMsg::user(body)
    }
    fn assistant(body: &str) -> ChatMsg {
        let mut m = ChatMsg::assistant_empty();
        m.body = body.to_string();
        m
    }
    fn tool_call(id: &str, name: &str) -> ChatMsg {
        let calls = vec![ChatToolCall {
            id: id.to_string(),
            kind: "function".to_string(),
            function: ChatToolCallFunction {
                name: Some(name.to_string()),
                arguments: Some("{}".to_string()),
            },
        }];
        ChatMsg::tool_call(name, "{}", calls)
    }
    fn tool_result(id: &str, name: &str, body: &str) -> ChatMsg {
        ChatMsg::tool_result(id, name, body, false)
    }

    #[test]
    fn find_compact_range_no_user_returns_none() {
        let msgs = vec![assistant("Привет!")];
        assert_eq!(find_compact_range(&msgs), None);
    }

    #[test]
    fn find_compact_range_only_first_user_returns_none() {
        let msgs = vec![user("первое сообщение")];
        assert_eq!(find_compact_range(&msgs), None);
    }

    #[test]
    fn find_compact_range_basic_pivot_at_last_user() {
        // [user, asst, user, asst, user] — pivot=4, range=0..4.
        let msgs = vec![
            user("u1"), assistant("a1"),
            user("u2"), assistant("a2"),
            user("u3"),
        ];
        assert_eq!(find_compact_range(&msgs), Some(0..4));
    }

    #[test]
    fn find_compact_range_atomic_for_tool_pair() {
        // Хвост [u1, a1, ToolCall, ToolResult, u2]: pivot=4, end=4 покрывает
        // обе стороны пары — это валидно.
        let msgs = vec![
            user("u1"),
            assistant("a1"),
            tool_call("c1", "bash"),
            tool_result("c1", "bash", "ok"),
            user("u2"),
        ];
        let r = find_compact_range(&msgs).unwrap();
        assert_eq!(r, 0..4);
    }

    #[test]
    fn find_compact_range_shrinks_when_toolcall_unmatched() {
        // [u1, asst, ToolCall(c1), u2] — ToolCall без ToolResult → исключаем
        // из range. Остаётся только [u1, asst] = 0..2.
        let msgs = vec![
            user("u1"),
            assistant("a1"),
            tool_call("c1", "bash"),
            user("u2"),
        ];
        let r = find_compact_range(&msgs).unwrap();
        assert_eq!(r, 0..2);
    }

    #[test]
    fn find_compact_range_skips_already_compacted() {
        // Первые два уже сжаты — start пропускает их.
        let mut msgs = vec![
            user("u1"), assistant("a1"),
            user("u2"), assistant("a2"),
            user("u3"),
        ];
        msgs[0].compacted_iter = Some(1);
        msgs[1].compacted_iter = Some(1);
        // Также вставим маркер на месте свёрнутых — он тоже игнорируется.
        msgs.insert(0, ChatMsg::compaction_marker(1, 2, 100, 30, "сводка".to_string()));
        // Теперь msgs: [marker, u1*, a1*, u2, a2, u3], pivot=5, range=3..5.
        let r = find_compact_range(&msgs).unwrap();
        assert_eq!(r, 3..5);
    }

    #[test]
    fn find_compact_range_too_few_useful_returns_none() {
        // [user, user] — pivot=1, start=0; всего один полезный msg в range,
        // меньше двух → None.
        let msgs = vec![user("u1"), user("u2")];
        assert_eq!(find_compact_range(&msgs), None);
    }

    #[test]
    fn in_turn_range_keeps_user_message_and_tail() {
        // [u1, call, res, call, res, placeholder] — сжимаем только первую
        // tool-пару: запрос пользователя и хвост из keep_tail=3 остаются.
        let msgs = vec![
            user("u1"),
            tool_call("c1", "bash"),
            tool_result("c1", "bash", "ok"),
            tool_call("c2", "bash"),
            tool_result("c2", "bash", "ok"),
            assistant(""),
        ];
        assert_eq!(find_compact_range_in_turn(&msgs, 3).unwrap(), 1..3);
    }

    #[test]
    fn in_turn_range_never_swallows_the_task() {
        // Запрос пользователя обязан остаться дословно: он не должен попасть
        // ни в один диапазон, иначе агент теряет задачу.
        let msgs = vec![
            user("u1"),
            tool_call("c1", "bash"),
            tool_result("c1", "bash", "ok"),
            tool_call("c2", "bash"),
            tool_result("c2", "bash", "ok"),
        ];
        let r = find_compact_range_in_turn(&msgs, 2).unwrap();
        assert!(r.start > 0, "range={r:?} затрагивает user-сообщение");
    }

    #[test]
    fn in_turn_range_shrinks_unmatched_tool_call() {
        // Хвост keep_tail=1 отрезает ToolResult у второй пары — значит и её
        // ToolCall в диапазон брать нельзя.
        let msgs = vec![
            user("u1"),
            tool_call("c1", "bash"),
            tool_result("c1", "bash", "ok"),
            tool_call("c2", "bash"),
            tool_result("c2", "bash", "ok"),
        ];
        assert_eq!(find_compact_range_in_turn(&msgs, 2).unwrap(), 1..3);
    }

    #[test]
    fn in_turn_range_none_when_nothing_to_compact() {
        let msgs = vec![user("u1"), tool_call("c1", "bash"), tool_result("c1", "bash", "ok")];
        // keep_tail съедает всю цепочку.
        assert_eq!(find_compact_range_in_turn(&msgs, 4), None);
        // Нет ни одного user-сообщения.
        assert_eq!(find_compact_range_in_turn(&[assistant("a")], 0), None);
    }

    #[test]
    fn next_iteration_starts_at_one() {
        let msgs = vec![user("u1"), assistant("a1")];
        assert_eq!(next_iteration(&msgs), 1);
    }

    #[test]
    fn next_iteration_advances_with_marker() {
        let msgs = vec![
            ChatMsg::compaction_marker(1, 3, 100, 30, "s1".to_string()),
            ChatMsg::compaction_marker(2, 5, 200, 50, "s2".to_string()),
            user("u"),
        ];
        assert_eq!(next_iteration(&msgs), 3);
    }

    #[test]
    fn next_iteration_handles_orphan_compacted_iter() {
        // Маркер удалили, но `compacted_iter` остался — не должно
        // зацикливать итерацию на 1.
        let mut msgs = vec![user("u1"), assistant("a1"), user("u2")];
        msgs[0].compacted_iter = Some(7);
        assert_eq!(next_iteration(&msgs), 8);
    }

    #[test]
    fn serialize_for_summary_truncates_long_tool_results() {
        let big = "x".repeat(2000);
        let msgs = vec![
            user("u"),
            tool_result("c1", "bash", &big),
        ];
        let s = serialize_for_summary(&msgs, 0..2);
        assert!(s.contains("User:"));
        assert!(s.contains("result truncated"));
        // Не должно содержать всю длинную строку.
        assert!(!s.contains(&"x".repeat(1800)));
    }

    #[test]
    fn serialize_for_summary_skips_already_compacted() {
        let mut msgs = vec![user("u1"), assistant("a1"), user("u2")];
        msgs[0].compacted_iter = Some(1);
        let s = serialize_for_summary(&msgs, 0..3);
        // u1 свёрнут раньше — не должно появиться в выходе.
        assert!(!s.contains("u1"));
        assert!(s.contains("a1"));
        assert!(s.contains("u2"));
    }

    #[test]
    fn apply_compaction_marks_and_inserts_marker() {
        let mut msgs = vec![
            user("u1"), assistant("a1"),
            user("u2"),
        ];
        apply_compaction_to_messages(&mut msgs, 0..2, 1, 100, 30, "сводка".to_string());
        // [marker, u1*, a1*, u2]
        assert_eq!(msgs.len(), 4);
        assert!(matches!(
            msgs[0].kind,
            ChatMsgKind::CompactionMarker { iteration: 1, .. }
        ));
        assert_eq!(msgs[1].compacted_iter, Some(1));
        assert_eq!(msgs[2].compacted_iter, Some(1));
        assert_eq!(msgs[3].compacted_iter, None);
    }
}
