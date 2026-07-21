//! Autocompact: автоматическое и ручное сжатие старых сообщений в `system`-summary.
//!
//! Архитектура:
//! - Pure-функции (этот файл): `find_compact_range`, `next_iteration`,
//!   `serialize_for_summary`, `apply_compaction_to_messages`. Покрыты юнит-тестами.
//! - Async-оркестратор `run_compaction` шлёт отдельный non-stream запрос к
//!   llama-server с системным промптом «сожми этот диалог», ждёт ответ,
//!   считает токены через `/tokenize`, потом на main-потоке атомарно
//!   обновляет `chat.messages`: помечает свёрнутые `compacted_iter=Some(N)`
//!   и вставляет `ChatMsg::compaction_marker` перед ними.
//! - Триггер вызывается из `chat::session::run_agent` после успешного turn'а
//!   (см. `maybe_compact`); ручная кнопка — `compact_now`.
//!
//! Маркер autocompact превращается в `system`-сообщение для модели в
//! `chat::session::build_history_from_slice_at`. Свёрнутые сообщения
//! пропускаются там же.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::context_provider::use_context;

use crate::context::AppCtx;
use crate::llama::api::{ChatMessage as ApiChatMessage, ChatRequest, LlamaClient, SamplingParams};

use super::state::{ChatMsg, ChatMsgKind, ChatMsgRole};
use super::storage;

/// Что вызвало компактификацию: автоматический триггер по проценту контекста
/// или ручная кнопка пользователя.
#[derive(Debug, Clone, Copy)]
pub enum CompactionTrigger {
    /// Автоматический триггер. `tokens_before` — снимок `prompt_tokens` из
    /// usage финального чанка, который привёл к срабатыванию.
    Auto { tokens_before: i64 },
    /// Ручная кнопка «Compact now». Точное значение токенов до сжатия
    /// неизвестно, в маркер пишем 0 — UI рисует «—» вместо числа.
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
/// если левая граница попадает на `ToolResult` без своего `ToolCall` —
/// сдвигаем `start` вправо до начала следующего полного блока. Аналогично
/// уменьшаем `end`, если он попадает между `ToolCall` и его `ToolResult`.
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

    let mut end = pivot;

    // 3. Атомарность tool-пар по правой границе. Если последний кандидат —
    //    `ToolCall`, его `ToolResult` лежит ПОСЛЕ pivot'а или вообще
    //    отсутствует — сжимать такой ToolCall нельзя (в API будет stub).
    //    Сдвигаем `end` назад, пока хвост — это незакрытый ToolCall.
    while end > start {
        let last = &msgs[end - 1];
        let needs_shrink = match &last.kind {
            ChatMsgKind::ToolCall { .. } => {
                // Есть ли ToolResult с тем же id внутри [last.., end)?
                if let Some(calls) = &last.tool_calls {
                    let any_unmatched = calls.iter().any(|c| {
                        !msgs[end..pivot].iter().any(|mm| matches!(
                            &mm.kind,
                            ChatMsgKind::ToolResult { tool_call_id, .. } if tool_call_id == &c.id
                        )) && !msgs[(end - 1) + 1..end].iter().any(|mm| matches!(
                            &mm.kind,
                            ChatMsgKind::ToolResult { tool_call_id, .. } if tool_call_id == &c.id
                        ))
                    });
                    // На end-границе у нас только last — Result не может
                    // лежать «внутри» одного элемента; проверяем именно то,
                    // что между last и pivot: если ВСЕ результаты для last
                    // отсутствуют до pivot — нельзя сжимать.
                    any_unmatched
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

    // 4. Атомарность tool-пар по левой границе: если первый кандидат —
    //    `ToolResult`, его `ToolCall` ушёл бы в свёрнутые предыдущей
    //    итерации? Это невозможно для не-свёрнутого ToolResult'а в
    //    нормальной ленте, но защитимся: если `start` = ToolResult без
    //    предшествующего ToolCall в [start..end] — сдвигаем `start` вправо.
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
                    "[Ранее сжатый блок (итерация {iteration})]: {summary}\n\n"
                ));
            }
            ChatMsgKind::Text => {
                let prefix = match m.role {
                    ChatMsgRole::User => "Пользователь:",
                    ChatMsgRole::Assistant => "Ассистент:",
                    ChatMsgRole::System => "Система:",
                };
                if !m.attachments.is_empty() {
                    buf.push_str(&format!(
                        "{prefix} [приложил {} изображени(й)]\n",
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
                    "Ассистент вызвал tool `{tool_name}`(args={args}).\n\n"
                ));
            }
            ChatMsgKind::ToolResult { tool_name, error, .. } => {
                let body = m.body.trim();
                let truncated = storage::truncate_chars(body, MAX_TOOL_BODY);
                let suffix = if body.chars().count() > MAX_TOOL_BODY {
                    format!(" [результат обрезан до {MAX_TOOL_BODY} символов]")
                } else {
                    String::new()
                };
                let status = if *error { "ошибка" } else { "ok" };
                buf.push_str(&format!(
                    "Tool `{tool_name}` ({status}): {truncated}{suffix}\n\n"
                ));
            }
        }

        if buf.chars().count() > MAX_TOTAL {
            // Дальше уже не влезаем; добавим маркер обрыва и выходим.
            buf.push_str("\n[…история обрезана для summary-запроса…]\n");
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
const SUMMARY_SYSTEM_PROMPT: &str = "Ты сжимаешь фрагмент диалога в краткое \
техническое описание: ключевые решения, имена файлов, итоги tool-вызовов, \
открытые вопросы. Сохраняй конкретику (числа, идентификаторы, имена). \
Не пересказывай дословно. Пиши на русском, нейтрально, в 7–15 коротких абзацах.";

/// Точка входа из ручной кнопки UI. Не блокирует UI: spawn'ит async-таску
/// и сразу возвращается. Игнорирует повторный клик во время компактификации
/// через тот же `chat.pending`-флаг (UI её disable'ит).
pub fn compact_now() {
    let app = use_context::<AppCtx>();
    let chat = app.chat.clone();
    let host = app.general.server_host.get_untracked();
    let port = app.general.server_port.get_untracked();
    let base_url = format!("http://{}:{}", host, port);
    let abort = chat.abort.clone();
    let snapshot = abort.load(Ordering::Relaxed);
    spawn(async move {
        run_compaction(base_url, abort, snapshot, CompactionTrigger::Manual).await;
    });
}

/// Главный async-оркестратор autocompact. Идемпотентен: при отсутствии
/// кандидатов — просто возвращается. Все ошибки логируются + snackbar,
/// состояние ленты НЕ модифицируется.
pub async fn run_compaction(
    base_url: String,
    abort: Arc<AtomicU64>,
    snapshot: u64,
    trigger: CompactionTrigger,
) {
    if abort.load(Ordering::Relaxed) != snapshot {
        return;
    }

    // 1. Снимаем snapshot ленты на main и считаем диапазон.
    let Some((msgs_snapshot, range, iteration)) = read_compact_plan_on_main().await else {
        tracing::debug!(target: "autocompact", "no candidates, skipping");
        return;
    };
    if abort.load(Ordering::Relaxed) != snapshot {
        return;
    }

    let block = serialize_for_summary(&msgs_snapshot, range.clone());
    if block.trim().is_empty() {
        return;
    }

    push_snackbar(format!("Сжатие контекста (итерация {iteration})…"), false);

    // 2. Запрашиваем у модели summary через non-stream chat-completion.
    let client = LlamaClient::with_base_url(base_url.clone());
    let req = ChatRequest::new(vec![
        ApiChatMessage::system(SUMMARY_SYSTEM_PROMPT.to_string()),
        ApiChatMessage::user(block),
    ])
    .with_sampling(SamplingParams::new().with_temperature(0.3));

    let resp = match client.chat_completions(&req).await {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("Не удалось сжать контекст: {e}");
            tracing::warn!(target: "autocompact", "summary request failed: {e}");
            push_snackbar(msg, true);
            return;
        }
    };
    if abort.load(Ordering::Relaxed) != snapshot {
        return;
    }

    let summary = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default()
        .trim()
        .to_string();
    if summary.is_empty() {
        let msg = "Не удалось сжать контекст: модель вернула пустой ответ".to_string();
        tracing::warn!(target: "autocompact", "empty summary");
        push_snackbar(msg, true);
        return;
    }

    // 3. Считаем токены summary через /tokenize. Любая ошибка — fallback на
    //    грубую оценку (chars/4).
    let tokens_after = match client.tokenize_text(summary.clone()).await {
        Ok(ids) => ids.len() as i64,
        Err(_) => (summary.chars().count() as i64 / 4).max(1),
    };

    // 4. На main: применяем компактификацию атомарно.
    let tokens_before = trigger.tokens_before();
    let summary_for_apply = summary.clone();
    let abort_c = abort.clone();
    let mut applied = false;
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    run_on_main_thread(move || {
        if abort_c.load(Ordering::Relaxed) != snapshot {
            let _ = tx.send(false);
            return;
        }
        let chat = use_context::<AppCtx>().chat.clone();
        let mut applied_ok = false;
        chat.messages.update(|v| {
            // Перепроверяем диапазон: пользователь мог за это время отправить
            // новое сообщение, что сдвинуло индексы. Пересчитываем по
            // актуальной ленте.
            if let Some(fresh_range) = find_compact_range(v) {
                let iter = next_iteration(v);
                apply_compaction_to_messages(
                    v,
                    fresh_range,
                    iter,
                    tokens_before,
                    tokens_after,
                    summary_for_apply.clone(),
                );
                applied_ok = true;
            }
        });
        let _ = tx.send(applied_ok);
    });
    if let Ok(ok) = rx.await {
        applied = ok;
    }

    if applied {
        let saved_tokens = (tokens_before - tokens_after).max(0);
        push_snackbar(
            format!(
                "Контекст сжат: {tokens_before} → {tokens_after} токенов (-{saved_tokens})"
            ),
            false,
        );
    }
}

/// Читает на main-потоке текущую ленту, диапазон для сжатия и следующий
/// номер итерации. `None` если кандидатов нет.
async fn read_compact_plan_on_main() -> Option<(Vec<ChatMsg>, Range<usize>, u32)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let chat = use_context::<AppCtx>().chat.clone();
        let msgs = chat.messages.get_untracked();
        let plan = find_compact_range(&msgs).map(|range| {
            let iteration = next_iteration(&msgs);
            (msgs.clone(), range, iteration)
        });
        let _ = tx.send(plan);
    });
    rx.await.ok().flatten()
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
    use crate::chat::ChatMsg;
    use crate::llama::api::{ChatToolCall, ChatToolCallFunction};

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
        assert!(s.contains("Пользователь:"));
        assert!(s.contains("результат обрезан"));
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
