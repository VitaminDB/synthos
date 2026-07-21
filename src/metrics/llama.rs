//! Интеграция llama-метрик в `MetricsState`.
//!
//! Два источника:
//!   1. Final `ChatStreamChunk.timings/usage` — авторитет; там точные per_second
//!      значения от сервера. Вызывается из `chat::session::drive_stream`.
//!   2. Poll `GET /slots` раз в ~500 мс пока `pending = true` — живое
//!      обновление `n_decoded`/`n_remain`/`n_ctx` (для прогресс-бара
//!      контекста и «live tokens/s»-оценки).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::{run_on_main_thread, spawn};

use crate::llama::api::{LlamaClient, SlotInfo, Timings, Usage};

use super::MetricsState;

/// Применить финальный снимок `timings/usage` из стрим-чанка.
///
/// Вызывается из main thread (внутри `run_on_main_thread`-колбэка в
/// `drive_stream`), поэтому работаем с сигналами напрямую.
pub fn apply_final_chunk(
    state: &MetricsState,
    timings: Option<&Timings>,
    usage: Option<&Usage>,
) {
    if let Some(t) = timings {
        // Сохраняем снимок целиком — UI достанет все поля (cache_n, prompt_ms…)
        state.llama_timings.set_always(Some(t.clone()));

        if let Some(pps) = t.predicted_per_second {
            state.predicted_tps_history.update(|h| h.push(pps));
        }
        if let Some(pps) = t.prompt_per_second {
            state.prompt_tps_history.update(|h| h.push(pps));
        }
    }
    if let Some(u) = usage {
        state.llama_usage.set_always(Some(u.clone()));
    }
}

/// Стартовать background-таск, который опрашивает `/slots` пока
/// `abort_snapshot` не изменился. Полученный первый слот складывается в
/// `state.llama_slot`. Ошибки сети — молча пропускаем (предыдущее значение
/// остаётся, UI не дёргается).
///
/// `abort` — тот же `Arc<AtomicU64>` из `ChatCtx`, что используется для
/// прерывания стрима. Мы полагаемся на его монотонный рост: любая смена
/// → прекращаем опрос.
pub fn spawn_slot_poller(
    base_url: String,
    state: Arc<MetricsState>,
    abort: Arc<std::sync::atomic::AtomicU64>,
    abort_snapshot: u64,
) {
    const POLL_INTERVAL: Duration = Duration::from_millis(500);

    spawn(async move {
        let client = LlamaClient::with_base_url(base_url);
        // Обязательно увидеть `is_processing=true` хотя бы раз, прежде чем
        // принимать idle-слот как сигнал «генерация кончилась». Иначе
        // первый poll до старта обработки ошибочно останавливал бы нас.
        let mut saw_processing = false;
        // Последний известный n_ctx — нужен, чтобы после завершения
        // генерации UI всё ещё знал размер окна для знаменателя индикатора
        // контекста (usage.total_tokens / n_ctx).
        let mut last_n_ctx: Option<i64> = None;

        loop {
            if abort.load(Ordering::Relaxed) != abort_snapshot {
                break;
            }
            match client.slots(false).await {
                Ok(slots) => {
                    let processing: Option<SlotInfo> = slots
                        .iter()
                        .find(|s| s.is_processing.unwrap_or(false))
                        .cloned();

                    match processing {
                        Some(s) => {
                            saw_processing = true;
                            last_n_ctx = s.n_ctx.or(last_n_ctx);
                            let state_cloned = state.clone();
                            let snapshot = abort_snapshot;
                            let abort_cloned = abort.clone();
                            let slot = Some(s);
                            run_on_main_thread(move || {
                                if abort_cloned.load(Ordering::Relaxed) != snapshot {
                                    return;
                                }
                                state_cloned.llama_slot.set_always(slot);
                            });
                        }
                        None => {
                            // Обновим n_ctx из первого слота (любого) для
                            // корректного знаменателя индикатора контекста,
                            // если ещё не знаем.
                            if last_n_ctx.is_none() {
                                last_n_ctx = slots.iter().find_map(|s| s.n_ctx);
                                if let Some(n) = last_n_ctx {
                                    let state_cloned = state.clone();
                                    let snapshot = abort_snapshot;
                                    let abort_cloned = abort.clone();
                                    let stub = SlotInfo {
                                        id: slots.first().map(|s| s.id).unwrap_or(0),
                                        id_task: None,
                                        n_ctx: Some(n),
                                        speculative: None,
                                        is_processing: Some(false),
                                        params: Default::default(),
                                        next_token: Default::default(),
                                        extra: Default::default(),
                                    };
                                    let slot = Some(stub);
                                    run_on_main_thread(move || {
                                        if abort_cloned.load(Ordering::Relaxed) != snapshot {
                                            return;
                                        }
                                        state_cloned.llama_slot.set_always(slot);
                                    });
                                }
                            }
                            // Если уже видели активную обработку — генерация
                            // завершилась. Останавливаемся, оставляя в
                            // `llama_slot` последний processing-снимок.
                            if saw_processing {
                                break;
                            }
                        }
                    }
                }
                Err(_) => {
                    // Сервер может быть недоступен (ещё не запущен) —
                    // молча ждём следующий тик.
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}
