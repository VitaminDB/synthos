//! Логика отправки сообщения и потокового чтения ответа llama-server.
//!
//! Все публичные функции безопасно вызываются из обработчиков кликов
//! (главного потока). Сам HTTP-запрос и чтение SSE живут в async-задаче,
//! пробрасываемой через [`syngui::async_runtime::spawn`]; обновления UI
//! возвращаются в главный поток через [`syngui::async_runtime::run_on_main_thread`]
//! — тот же паттерн, что использует [`crate::llama::process`] для логов.
//!
//! Семантика abort: [`ChatCtx::abort`] — монотонный счётчик. На старте запроса
//! делаем snapshot; на каждой итерации и на каждом апдейте UI сверяем его с
//! актуальным значением. Рост = «старый стрим больше не нужен» → драйвер
//! молча завершается, не трогая состояние. Та же ручка закрывает висящий
//! диалог подтверждения tool-вызова.
//!
//! Agent loop: если модель запросила инструменты (`finish_reason=ToolCalls`),
//! оркестратор `run_agent` последовательно проводит каждый вызов через
//! UI-подтверждение, исполняет его и делает следующий turn с обновлённой
//! историей, пока модель не вернёт обычный текстовый ответ.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::context_provider::use_context;

use crate::context::AppCtx;
use crate::llama::api::{
    ChatChunkToolCall, ChatContentPart, ChatImageUrl, ChatMessage as ApiChatMessage,
    ChatRequest, ChatRole, ChatToolCall, ChatToolCallFunction, FinishReason, LlamaClient,
    LlamaError, SamplingParams, StreamOptions,
};
use crate::metrics;

use super::registry;
use super::state::{ChatCtx, ChatMsg, ChatMsgKind, ChatMsgRole};
use super::storage;
use super::tools::{self, PendingApproval, Tool, ToolDecision};

/// Значение temperature по умолчанию. Пер-chat настройку отложили — общий
/// `SamplingParams` со всеми пользовательскими параметрами применяется при
/// запуске `llama-server` через `ModelConfig.active_params`.
const DEFAULT_TEMPERATURE: f32 = 0.7;

// Лимит tool-turn'ов читается из `AppCtx.general.agent_max_turns` в
// `start_agent_turn` и пробрасывается аргументом в `run_agent`. Дефолт
// определяется в `config::default_agent_max_turns()`.

// ─────────────────────────────────────────────────────────────────────────────
// Публичные действия
// ─────────────────────────────────────────────────────────────────────────────

/// Отправить сообщение от пользователя.
///
/// Идемпотентно относительно повторных кликов: во время активного стрима
/// (`pending=true`) повторная отправка игнорируется. Пустой/whitespace-ввод
/// игнорируется тоже — кнопка «отправить» при таком состоянии ничего не делает.
pub fn send_message(text: String) {
    let app = use_context::<AppCtx>();
    let chat = app.chat.clone();

    if chat.pending.get_untracked() {
        return;
    }
    let trimmed = text.trim().to_string();
    let attachments = chat.draft_attachments.get_untracked();
    // Пустое сообщение — это и нет текста, и нет вложений. Реплику без
    // тела с прикреплёнными картинками («посмотри, что тут») разрешаем.
    if trimmed.is_empty() && attachments.is_empty() {
        return;
    }

    // Если активного чата нет — создаём его на лету. Пользователь мог не
    // нажимать «+»: набрать сообщение и отправить — естественный UX.
    if chat.active_chat_id.get_untracked().is_none() {
        registry::create_new();
    }

    // Если чат ещё без title — присвоим по первому user-сообщению.
    // Сравниваем по строгому дефолту: переименовывать вручную пока нельзя,
    // так что любой "Новый чат" — значит не-переименованный автоматом.
    if let Some(meta) = registry::active_meta() {
        if meta.title == "Новый чат" {
            let title_seed = if !trimmed.is_empty() {
                storage::truncate_chars(&trimmed, 40)
            } else {
                // Только картинки — придумаем заголовок по числу вложений.
                format!("Картинки ({})", attachments.len())
            };
            registry::rename_active(title_seed);
        }
    }

    // 1) Синхронно кладём в ленту user-сообщение и пустой assistant-placeholder,
    //    чистим input и draft_attachments, выставляем pending. UI сразу
    //    реактивно покажет оба пузыря и индикатор «печатает».
    let user_msg = ChatMsg::user_with_attachments(trimmed.clone(), attachments);
    let assistant_msg = ChatMsg::assistant_empty();
    chat.messages.update(|v| {
        v.push(user_msg);
        v.push(assistant_msg);
    });
    chat.input.set(String::new());
    chat.draft_attachments.set(Vec::new());
    // Бампаем поколение ввода: Reactive-обёртка вокруг editor пересоздаст
    // MultilineTextEdit с пустым `text` — это визуально очистит поле.
    chat.input_gen.update(|g| *g = g.wrapping_add(1));
    chat.error.set(None);
    chat.streaming_body.set(String::new());
    chat.streaming_thinking.set(String::new());
    chat.pending.set(true);

    // 2-3) Запуск agent-цикла: snapshot настроек, метрики, spawn run_agent.
    //      Ровно тот же flow используется в `regenerate_from`.
    start_agent_turn(&app, &chat);
}

/// Перегенерировать ответ ассистента, начиная с `msg_idx`-го сообщения в ленте.
///
/// Поведение: находим ближайший предыдущий user-Text перед `msg_idx`, обрезаем
/// `messages` до конца этого user-сообщения включительно, добавляем пустой
/// assistant-placeholder и запускаем тот же agent-цикл, что и `send_message`.
/// История для LLM пересобирается стандартным `build_history` — никаких
/// дополнительных кодовых путей.
///
/// No-op если: `pending=true`, `msg_idx` за пределами ленты или перед ним нет
/// ни одного user-Text сообщения. Тихие выходы — UI и так не должен показывать
/// regen-кнопку в этих ситуациях.
pub fn regenerate_from(msg_idx: usize) {
    let app = use_context::<AppCtx>();
    let chat = app.chat.clone();

    if chat.pending.get_untracked() {
        return;
    }
    let msgs = chat.messages.get_untracked();
    let Some(user_idx) = find_last_user_before(&msgs, msg_idx) else {
        return;
    };

    chat.messages.update(|v| {
        v.truncate(user_idx + 1);
        v.push(ChatMsg::assistant_empty());
    });
    chat.error.set(None);
    chat.streaming_body.set(String::new());
    chat.streaming_thinking.set(String::new());
    chat.pending.set(true);

    start_agent_turn(&app, &chat);
}

/// Продолжить незавершённый ответ ассистента в текущем чате.
///
/// Кейс: модель прислала `finish_reason=Stop` посреди фразы (sampler-баг,
/// кривой stop-token, оборванный code-fence) — `text_done`, но содержимое
/// явно неполное. Пользователь жмёт кнопку «Продолжить» под последним
/// bubble: мы не создаём нового assistant_empty, а запускаем agent-цикл
/// поверх существующего partial body. `build_history_from_slice` отдаст
/// его серверу как `assistant`-сообщение, llama-server допишет с того же
/// места; стрим вольётся в тот же bubble через `streaming_body` и
/// `commit_streaming_tail`.
///
/// No-op если: `pending=true`, лента пуста, последнее сообщение — не
/// `Assistant/Text`, или body пустой (продолжать нечего — это типичный
/// плейсхолдер, а не оборванный ответ).
pub fn continue_assistant() {
    let app = use_context::<AppCtx>();
    let chat = app.chat.clone();

    if chat.pending.get_untracked() {
        return;
    }
    let msgs = chat.messages.get_untracked();
    let Some(last) = msgs.last() else {
        return;
    };
    if !matches!(last.role, ChatMsgRole::Assistant)
        || !matches!(last.kind, ChatMsgKind::Text)
        || last.body.is_empty()
    {
        return;
    }

    chat.error.set(None);
    chat.streaming_body.set(String::new());
    chat.streaming_thinking.set(String::new());
    chat.pending.set(true);

    start_agent_turn(&app, &chat);
}

/// Возвращает индекс ближайшего предыдущего User-Text сообщения в `msgs[..=anchor]`.
/// Используется regenerate_from'ом для определения точки обрезки.
///
/// Логика: считаем ближайший user-Text «pivot'ом». Системные сообщения и tool
/// call/result игнорируются. Если `anchor` за пределами — ищем по полной ленте.
pub(crate) fn find_last_user_before(msgs: &[ChatMsg], anchor: usize) -> Option<usize> {
    let end = anchor.min(msgs.len().saturating_sub(1));
    if msgs.is_empty() {
        return None;
    }
    msgs[..=end]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| matches!(m.role, ChatMsgRole::User) && matches!(m.kind, ChatMsgKind::Text))
        .map(|(i, _)| i)
}

/// Снимает snapshot настроек, запускает slot-poller метрик и async-цикл
/// `run_agent`. Вынесено из `send_message` чтобы переиспользовать в
/// `regenerate_from` без дублирования кода (и риска рассинхрона).
///
/// Предусловия: вызывается на main-потоке, `chat.pending` уже выставлен в `true`,
/// в `chat.messages` лежит свежий assistant_empty placeholder в конце.
fn start_agent_turn(app: &AppCtx, chat: &ChatCtx) {
    let host = app.general.server_host.get_untracked();
    let port = app.general.server_port.get_untracked();
    let base_url = format!("http://{}:{}", host, port);
    let tools_vec = collect_active_tools(app);
    let sampling = SamplingParams::new().with_temperature(DEFAULT_TEMPERATURE);
    let abort_snapshot = chat.abort.load(Ordering::Relaxed);
    let abort_flag = chat.abort.clone();
    // Snapshot пользовательской настройки. `0` (при ручной правке конфига)
    // означал бы немедленный выход цикла — клампим к минимально полезной 1.
    let max_turns = app
        .general
        .agent_max_turns
        .get_untracked()
        .max(1) as usize;

    // ── Augment pre-step snapshot ──────────────────────────────────────
    // Сбрасываем augment_text прошлого turn'а — атомарно с остальным
    // per-turn state, единая точка вместо трёх (send_message/regenerate/
    // continue_assistant). Если augment не нужен — текст останется пустым
    // и `build_history` молча его пропустит.
    chat.augment_text.set(String::new());
    let auto_augment = app.kb.auto_augment.get_untracked();
    let active_ids = app.kb.active_collection_ids();
    let augment_enabled = auto_augment && !active_ids.is_empty();
    // Query = последний user-Text в ленте. Для send_message это только что
    // добавленное сообщение; для regenerate — user перед обрезкой; для
    // continue_assistant — последний user-сообщение в истории.
    let augment_query: Option<String> = if augment_enabled {
        let msgs = chat.messages.get_untracked();
        let anchor = msgs.len().saturating_sub(1);
        find_last_user_before(&msgs, anchor)
            .map(|i| msgs[i].body.clone())
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    };
    let kb_cfg = crate::config::AppConfig::load().kb;
    let augment_top_k = kb_cfg.default_top_k.max(1);
    let augment_token_budget = kb_cfg.augment_token_budget.max(200);
    if augment_query.is_some() {
        app.kb.augment_in_progress.set(true);
    }
    // ── /augment snapshot ──────────────────────────────────────────────

    metrics::llama::spawn_slot_poller(
        base_url.clone(),
        app.metrics.clone(),
        abort_flag.clone(),
        abort_snapshot,
    );

    let abort_flag_outer = abort_flag.clone();
    spawn(async move {
        // ── Phase 1: augment (если включён и есть query) ──────────────
        if let Some(query) = augment_query {
            // Abort мог прилететь сразу после send_message — не тратим
            // время на compute.
            if abort_flag_outer.load(Ordering::Relaxed) != abort_snapshot {
                run_on_main_thread(|| {
                    use_context::<AppCtx>().kb.augment_in_progress.set(false);
                });
                return;
            }
            match crate::kb::augment::compute(&query, augment_top_k, augment_token_budget).await {
                Ok(text) => {
                    let abort_for_set = abort_flag_outer.clone();
                    run_on_main_thread(move || {
                        let app = use_context::<AppCtx>();
                        app.kb.augment_in_progress.set(false);
                        if abort_for_set.load(Ordering::Relaxed) != abort_snapshot {
                            return;
                        }
                        if !text.trim().is_empty() {
                            app.chat.augment_text.set(text);
                        }
                    });
                }
                Err(e) => {
                    log::warn!("auto-augment: {e}");
                    let msg = e.user_message();
                    run_on_main_thread(move || {
                        let app = use_context::<AppCtx>();
                        app.kb.augment_in_progress.set(false);
                        if let Some(m) = msg {
                            app.notifications.warning(m);
                        }
                    });
                }
            }
        }

        // ── Phase 2: основной agent loop ───────────────────────────────
        if abort_flag_outer.load(Ordering::Relaxed) != abort_snapshot {
            return;
        }
        run_agent(base_url, sampling, tools_vec, max_turns, abort_flag, abort_snapshot).await;
    });
}

/// Прервать текущий стрим. Если стрима нет — безвредно (просто инкрементит
/// счётчик, который никому не нужен).
pub fn abort() {
    let app = use_context::<AppCtx>();
    app.chat.abort.fetch_add(1, Ordering::Relaxed);
    // Висящий confirm-диалог тоже закрываем с Cancel, чтобы оркестратор
    // (если он сейчас в `await` на канале) сразу разблокировался.
    if let Some(pending) = app.tools.pending_approval.get_untracked() {
        pending.send(ToolDecision::Cancel);
    }
    app.tools.pending_approval.set(None);
    // Сразу гасим `pending`, чтобы кнопка «Stop» немедленно вернулась в
    // «Send». Финальный сброс в конце `run_agent` пропускается, когда
    // его snapshot устарел из-за этого `fetch_add` — иначе UI оставался
    // бы в состоянии «генерация идёт» до завершения уже отменённого
    // стрима. Если пользователь сразу нажмёт «Send», `send_message`
    // выставит `pending=true` обратно, а финальный сброс старого
    // `run_agent` его не затрёт (отсекается проверкой snapshot).
    app.chat.pending.set(false);
    // Если abort пришёл во время augment-phase — гасим индикатор
    // «Поиск в KB…», иначе он бы остался зависшим до завершения compute.
    app.kb.augment_in_progress.set(false);
}

// ─────────────────────────────────────────────────────────────────────────────
// Сборка истории
// ─────────────────────────────────────────────────────────────────────────────

/// Превращает ленту UI в `Vec<ApiChatMessage>` для отправки на сервер.
/// Порядок: optional `system` + все user/assistant/tool-сообщения.
/// Последний пустой assistant-placeholder пропускается.
///
/// Висящий `ToolCall` без соответствующего `ToolResult` (бывает, например,
/// после рестарта приложения посреди tool-turn’а) получает синтетический
/// stub-результат — иначе llama-server отвергает запрос с жалобой на
/// отсутствие tool-сообщения для tool_call_id.
pub(crate) fn build_history(chat: &ChatCtx) -> Vec<ApiChatMessage> {
    let msgs = chat.messages.get_untracked();
    let streaming_body = chat.streaming_body.get_untracked();
    let augment_text = chat.augment_text.get_untracked();
    build_history_from_slice_with_streaming(
        chat.system_prompt.get_untracked(),
        &msgs,
        &streaming_body,
        &augment_text,
    )
}

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

/// КРИТИЧЕСКИЙ FIX: версия build_history которая учитывает streaming_body.
/// Если streaming_body не пуст — значит assistant ещё "пишет" ответ, и
/// мы не должны включать partial текст в историю как полноценный ответ.
/// Это решает проблему: при soft-stop модель получает свой же partial
/// ответ как законченный и думает что уже всё сказала.
fn build_history_from_slice_with_streaming(
    system_prompt: String,
    msgs: &[ChatMsg],
    streaming_body: &str,
    augment_text: &str,
) -> Vec<ApiChatMessage> {
    build_history_from_slice_at(
        system_prompt,
        msgs,
        today_utc_iso(now_unix_secs()),
        streaming_body,
        augment_text,
    )
}

/// Жёсткое правило, которое всегда подмешивается в системный preamble
/// поверх пользовательского `system_prompt`. Зашито в код намеренно: это
/// инструкция уровня агент-цикла, без неё модель «анонсирует и закрывает
/// turn» вместо того чтобы вызывать tool, и пользователь вынужден писать
/// «продолжи» вручную (см. логи target=`agent`, `finish_reason=Some(Stop)`
/// после реплик с двоеточием). Не выносим в Settings — иначе при ручной
/// очистке поля правило теряется и баг возвращается.
const ACTION_RULE_PREAMBLE: &str = "ПРАВИЛО ДЕЙСТВИЯ: если для ответа \
нужно вызвать tool — вызывай его НЕМЕДЛЕННО в этом же ответе. Никогда \
не пиши «сейчас посмотрю», «теперь сделаю», «изучу», «проверю», \
«создам» и не заканчивай реплику двоеточием перед действием. Анонс \
без вызова tool — ошибка. Объяснения и итог давай ПОСЛЕ результатов \
tools, не до.";

fn build_history_from_slice_at(
    system_prompt: String,
    msgs: &[ChatMsg],
    today_iso: String,
    streaming_body: &str,
    augment_text: &str,
) -> Vec<ApiChatMessage> {
    let mut out: Vec<ApiChatMessage> = Vec::new();

    // Порядок: preamble(date+ACTION_RULE) → augment(RAG) → user_system_prompt.
    // ACTION_RULE идёт перед augment'ом, чтобы инструкция уровня агент-цикла
    // не «утонула» в 1500 токенах RAG-контекста. Пользовательский prompt —
    // в самом конце: его инструкции имеют последнее слово.
    let preamble = format!(
        "Текущая дата (UTC): {today_iso}.\n\n{ACTION_RULE_PREAMBLE}"
    );
    let augment_trimmed = augment_text.trim();
    let user_trimmed = system_prompt.trim();
    let mut merged = preamble;
    if !augment_trimmed.is_empty() {
        merged.push_str("\n\n");
        merged.push_str(augment_trimmed);
    }
    if !user_trimmed.is_empty() {
        merged.push_str("\n\n");
        merged.push_str(user_trimmed);
    }
    out.push(ApiChatMessage::system(merged));

    let mut i = 0;
    while i < msgs.len() {
        let m = &msgs[i];
        // Свёрнутые autocompact-маркером сообщения в API не уходят: модель
        // получит сжатый `summary` через сам `CompactionMarker`-message.
        // Атомарность (compact range не разрывает tool-пары) гарантирована
        // в `chat::compact::find_compact_range`.
        if m.compacted_iter.is_some() {
            i += 1;
            continue;
        }
        match (&m.kind, m.role) {
            (ChatMsgKind::CompactionMarker { summary, iteration, .. }, _) => {
                // Маркер autocompact превращается в `system`-сообщение с
                // явным указанием итерации — модели проще ориентироваться,
                // когда она «разговаривает» с текстом из своих же ответов.
                out.push(ApiChatMessage::system(format!(
                    "[Краткое содержание раннего диалога — autocompact-итерация {iteration}]:\n{summary}"
                )));
                i += 1;
            }
            (ChatMsgKind::ToolCall { .. }, _) => {
                let calls = m.tool_calls.clone().unwrap_or_default();
                // Assistant-message с tool_calls и без контентного текста.
                out.push(ApiChatMessage {
                    role: ChatRole::Assistant,
                    content: None,
                    name: None,
                    tool_call_id: None,
                    tool_calls: Some(calls.clone()),
                    reasoning_content: None,
                });
                // Для каждого tool_call ищем следующий ToolResult с тем же id.
                // Если не нашли — подставляем synthetic stub. Свёрнутые
                // ToolResult'ы в поиске игнорируются (см. предусловие выше).
                for call in &calls {
                    let match_idx = msgs
                        .iter()
                        .enumerate()
                        .skip(i + 1)
                        .find_map(|(idx, mm)| match &mm.kind {
                            ChatMsgKind::ToolResult { tool_call_id, .. }
                                if tool_call_id == &call.id
                                    && mm.compacted_iter.is_none() =>
                            {
                                Some(idx)
                            }
                            _ => None,
                        });
                    match match_idx {
                        Some(idx) => {
                            out.push(ApiChatMessage::tool(
                                msgs[idx].body.clone(),
                                call.id.clone(),
                            ));
                        }
                        None => {
                            out.push(ApiChatMessage::tool(
                                "[result unavailable after restart]".to_string(),
                                call.id.clone(),
                            ));
                        }
                    }
                }
                i += 1;
            }
            (ChatMsgKind::ToolResult { .. }, _) => {
                // ToolResult уже прошит к соответствующему ToolCall выше —
                // здесь пропускаем, чтобы не дублировать.
                i += 1;
            }
            (ChatMsgKind::Text, ChatMsgRole::User) => {
                if m.attachments.is_empty() {
                    out.push(ApiChatMessage::user(m.body.clone()));
                } else {
                    let mut parts: Vec<ChatContentPart> =
                        Vec::with_capacity(1 + m.attachments.len());
                    if !m.body.is_empty() {
                        parts.push(ChatContentPart::Text {
                            text: m.body.clone(),
                        });
                    }
                    for att in &m.attachments {
                        match super::blobs::data_url(att) {
                            Ok(url) => parts.push(ChatContentPart::ImageUrl {
                                image_url: ChatImageUrl { url, detail: None },
                            }),
                            Err(e) => log::warn!(
                                "attachment {} unavailable, отправляю без неё: {e}",
                                att.sha256
                            ),
                        }
                    }
                    if parts.is_empty() {
                        // Все вложения недоступны и текста нет — нечего слать,
                        // но молча пропустить пользовательскую реплику нельзя:
                        // оставляем минимум — пустую текстовую часть, иначе
                        // server отдаст 400.
                        parts.push(ChatContentPart::Text {
                            text: String::new(),
                        });
                    }
                    out.push(ApiChatMessage::multipart(ChatRole::User, parts));
                }
                i += 1;
            }
            (ChatMsgKind::Text, ChatMsgRole::Assistant) => {
                // КРИТИЧЕСКИЙ FIX: если streaming_body не пуст — значит assistant
                // ещё "пишет" ответ (soft-stop случай). Не включаем partial
                // текст в историю как полноценный ответ — иначе модель получит
                // свой же незавершённый текст и подумает что уже всё сказала.
                let is_last = i == msgs.len() - 1;
                let body_to_send = if is_last && !streaming_body.is_empty() {
                    // Partial ответ: не отправляем в API
                    String::new()
                } else {
                    m.body.clone()
                };
                if !body_to_send.is_empty() {
                    out.push(ApiChatMessage::assistant(body_to_send));
                }
                i += 1;
            }
            (ChatMsgKind::Text, ChatMsgRole::System) => {
                // System-плашки UI (ошибки, отмены) не отправляем серверу.
                i += 1;
            }
        }
    }

    out
}

/// Собирает `Vec<ChatTool>` из активных ключей `AppCtx.tools.active`.
///
/// Особый случай — `autoskill`: статическое описание из catalog'а
/// дополняется списком актуальных скилов (id + краткое описание) и enum'ом
/// допустимых id в JSON-schema. Это даёт модели «жирный» tool-descriptor с
/// контекстом «вот что у пользователя есть» без правки prompt'а.
fn collect_active_tools(app: &AppCtx) -> Vec<crate::llama::api::ChatTool> {
    use super::tools::catalog::KEY_AUTOSKILL;
    let keys = app.tools.active.get_untracked();
    keys.iter()
        .filter_map(|k| {
            if k == KEY_AUTOSKILL {
                Some(build_autoskill_chat_tool(app))
            } else {
                Tool::by_key(k).map(|t| t.to_chat_tool())
            }
        })
        .collect()
}

/// Динамический ChatTool для `autoskill`: статическое описание + актуальный
/// список доступных скилов (читается из `AppCtx.skills`). Если скилов нет —
/// description явно сообщает об этом, enum в schema пустой → модель не
/// сможет вызвать tool, что и ожидается.
pub(crate) fn build_autoskill_chat_tool(app: &AppCtx) -> crate::llama::api::ChatTool {
    use super::tools::catalog::KEY_AUTOSKILL;
    let descriptor = Tool::by_key(KEY_AUTOSKILL).expect("autoskill descriptor должен существовать");
    let skills = app.skills.get_untracked();

    let mut description = descriptor.description.to_string();
    description.push_str("\n\nДоступные скилы (id — описание):\n");
    if skills.is_empty() {
        description.push_str("- (пусто, у пользователя нет скилов)\n");
    } else {
        for s in &skills {
            let desc = if s.description.is_empty() {
                "(без описания)"
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

    crate::llama::api::ChatTool {
        kind: "function".to_string(),
        function: crate::llama::api::ToolFunctionSchema {
            name: descriptor.key.to_string(),
            description: Some(description),
            parameters: Some(schema),
            strict: None,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent loop
// ─────────────────────────────────────────────────────────────────────────────

/// Что вернул один turn к llama-server.
enum TurnOutcome {
    /// Модель выдала обычный текстовый ответ (уже лежит в `messages`).
    Text,
    /// Модель ушла в reasoning, но не выдала ни одного байта body после
    /// `</think>` — «закончила думать и замолчала». agent-loop делает
    /// auto-continue с лимитом `MAX_SOFT_STOP_RETRIES`, чтобы дать модели
    /// «договорить».
    SoftStop,
    /// Модель попросила выполнить набор инструментов.
    ToolCalls(Vec<ChatToolCall>),
    /// Ошибка, уже записанная в UI.
    Error,
}

/// Сколько раз подряд агент готов продолжить turn после `SoftStop`,
/// прежде чем сдаться и завершиться `text_done`. Защита от модели,
/// упёртой в «думаю-и-молчу» или бесконечный preamble — без cap'а
/// дотянет до `agent_max_turns` и съест бюджет.
/// 
/// Увеличено до 3 для Qwen моделей на llama.cpp, которые часто делают
/// пустые "думательные" ходы перед реальным ответом.
const MAX_SOFT_STOP_RETRIES: usize = 3;

/// Аккумулятор одной tool-call дельты во время стрима. Сервер присылает
/// пустой `index` (обычно 0), нарастающие кусочки `arguments` и один раз —
/// `id` + `function.name`. Мы upsert’им по `index` и склеиваем строки.
#[derive(Default)]
struct AccumulatedToolCall {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl AccumulatedToolCall {
    fn finalize(self, fallback_index: i32) -> ChatToolCall {
        ChatToolCall {
            id: self
                .id
                .unwrap_or_else(|| format!("call_{fallback_index}")),
            kind: self.kind.unwrap_or_else(|| "function".to_string()),
            function: ChatToolCallFunction {
                name: self.name,
                arguments: Some(self.arguments),
            },
        }
    }
}

/// Чистая функция сборки tool-call’а из потоковых дельт — выделена,
/// чтобы тестироваться без async.
fn fold_tool_call_deltas(
    acc: &mut Vec<(i32, AccumulatedToolCall)>,
    deltas: &[ChatChunkToolCall],
) {
    for d in deltas {
        let pos = acc.iter().position(|(i, _)| *i == d.index);
        let idx = match pos {
            Some(p) => p,
            None => {
                acc.push((d.index, AccumulatedToolCall::default()));
                acc.len() - 1
            }
        };
        let slot = &mut acc[idx].1;
        if let Some(id) = &d.id {
            slot.id = Some(id.clone());
        }
        if let Some(k) = &d.kind {
            slot.kind = Some(k.clone());
        }
        if let Some(f) = &d.function {
            if let Some(name) = &f.name {
                slot.name = Some(name.clone());
            }
            if let Some(args) = &f.arguments {
                slot.arguments.push_str(args);
            }
        }
    }
}

/// Главный оркестратор. Крутит turn’ы «LLM → (tool-calls) → tools → LLM»,
/// пока модель не отдаст обычный текстовый ответ или не случится abort/error.
/// В конце ровно один раз сбрасывает `pending=false`.
///
/// Каждый turn пишет `tracing::info!` под target `agent`: старт (history-len,
/// tools), результат (text/tool_calls/error), причина финального выхода
/// (text_done, error, aborted, max_turns_reached). Это единственная отладка
/// agent-цикла — пишется в `~/.local/state/synthos/logs/synthos.log.*`,
/// фильтр `EnvFilter` пускает `info` по умолчанию.
async fn run_agent(
    base_url: String,
    sampling: SamplingParams,
    tools_list: Vec<crate::llama::api::ChatTool>,
    max_turns: usize,
    abort: Arc<AtomicU64>,
    snapshot: u64,
) {
    let client = LlamaClient::with_base_url(base_url.clone());

    tracing::info!(
        target: "agent",
        tools = tools_list.len(),
        max_turns,
        "start"
    );

    let mut exit_reason = "max_turns_reached";
    let mut completed_turns: usize = 0;
    // Сколько `SoftStop` подряд уже прожили. Сбрасывается каждый раз,
    // когда турн вернул `Text` или `ToolCalls`.
    let mut soft_stop_streak: usize = 0;

    for turn in 0..max_turns {
        completed_turns = turn + 1;
        if abort.load(Ordering::Relaxed) != snapshot {
            exit_reason = "aborted";
            break;
        }

        // Autocompact-триггер. Проверяется на КАЖДОМ turn'е (включая нулевой),
        // чтобы успеть сжать историю ДО следующего запроса в llama-server,
        // а не после уже-провалившейся генерации. На turn=0 usage берётся
        // от прошлого запроса той же сессии — небольшое отставание в меньшую
        // сторону безопасно: чуть позже сработает, чем нужно. Если usage
        // отсутствует (свежий старт), maybe_compact_before_turn no-op'ит.
        maybe_compact_before_turn(&base_url, &abort, snapshot).await;
        if abort.load(Ordering::Relaxed) != snapshot {
            exit_reason = "aborted";
            break;
        }

        // Историю пересобираем на каждом turn’е из актуальной ленты —
        // она включает свежеполученные ToolCall + ToolResult сообщения.
        let history = match read_history_on_main(&abort, snapshot).await {
            Some(h) => h,
            None => {
                tracing::info!(target: "agent", turn, "exit: aborted while reading history");
                return;
            }
        };

        tracing::info!(
            target: "agent",
            turn,
            history_len = history.len(),
            "turn request"
        );

        let req = ChatRequest::new(history)
            .with_sampling(sampling.clone())
            .with_stream_options(StreamOptions { include_usage: true });
        let req = if tools_list.is_empty() {
            req
        } else {
            req.with_tools(tools_list.clone())
                // Параллельные tool-calls пока не поддерживаем — диалог
                // подтверждения и executor идут последовательно.
                // `parallel_tool_calls = Some(false)`.
                .with_parallel_tool_calls_disabled()
        };

        let outcome = match client.chat_completions_stream(&req).await {
            Ok(stream) => drive_stream(Box::pin(stream), &abort, snapshot).await,
            Err(e) => {
                // HTTP 500 с broken JSON в tool call args — типичная ошибка
                // local-семплеров (Qwen Q4 etc.): модель сгенерила невалидный
                // JSON и backend (llama.cpp/openai-compat) отвалился до того
                // как stream открылся. Не показываем как fatal — просто
                // retry'аем как SoftStop, дав модели шанс ответить чисто.
                let msg = format_error(&e, &base_url);
                if is_tool_call_parse_error(&msg) {
                    tracing::warn!(
                        target: "agent",
                        error = %msg,
                        turn,
                        "tool-call JSON parse error → retry as soft-stop"
                    );
                    TurnOutcome::SoftStop
                } else {
                    finish_with_error(msg);
                    TurnOutcome::Error
                }
            }
        };

        match outcome {
            TurnOutcome::Text => {
                tracing::info!(target: "agent", turn, "turn outcome: text");
                exit_reason = "text_done";
                break;
            }
            TurnOutcome::SoftStop => {
                soft_stop_streak += 1;
                if soft_stop_streak > MAX_SOFT_STOP_RETRIES {
                    tracing::warn!(
                        target: "agent",
                        turn,
                        soft_stop_streak,
                        max_soft_stop_retries = MAX_SOFT_STOP_RETRIES,
                        "turn outcome: soft-stop limit reached → stop"
                    );
                    exit_reason = "soft_stop_limit";
                    break;
                }
                tracing::info!(
                    target: "agent",
                    turn,
                    soft_stop_streak,
                    max_soft_stop_retries = MAX_SOFT_STOP_RETRIES,
                    remaining_retries = MAX_SOFT_STOP_RETRIES.saturating_sub(soft_stop_streak),
                    "turn outcome: soft-stop → auto-continue"
                );
                // Не создаём новый placeholder: текущий либо пуст
                // (`build_history` его пропустит, модель сгенерит свежий
                // ответ), либо содержит preamble (уйдёт в API как
                // assistant-message — модель допишет продолжение, UI
                // вольёт его в тот же bubble через `push_body_to_last`).
                continue;
            }
            TurnOutcome::Error => {
                tracing::warn!(target: "agent", turn, "turn outcome: error");
                exit_reason = "error";
                break;
            }
            TurnOutcome::ToolCalls(calls) => {
                let names: Vec<&str> = calls
                    .iter()
                    .filter_map(|c| c.function.name.as_deref())
                    .collect();
                tracing::info!(
                    target: "agent",
                    turn,
                    n_calls = calls.len(),
                    tools = ?names,
                    "turn outcome: tool_calls"
                );
                if abort.load(Ordering::Relaxed) != snapshot {
                    exit_reason = "aborted";
                    break;
                }
                if !process_tool_calls(calls, &abort, snapshot).await {
                    tracing::info!(target: "agent", turn, "exit: tool flow stopped (cancel/abort)");
                    exit_reason = "tool_flow_stopped";
                    break;
                }
                // Готовим новый пустой assistant-placeholder для следующего turn’а.
                spawn_on_main(|| {
                    let ctx = use_context::<AppCtx>().chat;
                    ctx.messages.update(|v| v.push(ChatMsg::assistant_empty()));
                });
            }
        }
    }

    tracing::info!(
        target: "agent",
        reason = exit_reason,
        turns = completed_turns,
        "exit"
    );

    // Финальный сброс pending — независимо от пути выхода.
    let abort_cloned = abort.clone();
    run_on_main_thread(move || {
        if abort_cloned.load(Ordering::Relaxed) != snapshot {
            return;
        }
        use_context::<AppCtx>().chat.pending.set(false);
    });
}

/// Проверяет, не пора ли запустить autocompact. Если `enabled=true` и
/// `prompt_tokens / n_ctx > threshold/100` — синхронно (await) запускает
/// компактификацию. Тихо пропускает при отсутствии usage (свежий старт)
/// или нулевом `n_ctx`.
async fn maybe_compact_before_turn(
    base_url: &str,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
) {
    let Some(tokens_before) = read_compact_threshold_check_on_main().await else {
        return;
    };
    if abort.load(Ordering::Relaxed) != snapshot {
        return;
    }
    super::compact::run_compaction(
        base_url.to_string(),
        abort.clone(),
        snapshot,
        super::compact::CompactionTrigger::Auto { tokens_before },
    )
    .await;
}

/// Читает на main-потоке: `enabled`, `threshold_percent`, `prompt_tokens`
/// (из `metrics.llama_usage`), `n_ctx` (из `metrics.llama_slot.n_ctx`,
/// fallback на `ModelConfig.ctx_size` для активной модели).
/// Возвращает `Some(prompt_tokens)`, если триггер сработал, иначе `None`.
async fn read_compact_threshold_check_on_main() -> Option<i64> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<i64>>();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        if !app.general.autocompact_enabled.get_untracked() {
            let _ = tx.send(None);
            return;
        }
        let threshold = app.general.autocompact_threshold_percent.get_untracked();
        // `Usage.prompt_tokens` — i64; если None, мы не знаем заполненность.
        // Не пытаемся гадать — просто пропускаем триггер на этом turn'е.
        let Some(usage) = app.metrics.llama_usage.get_untracked() else {
            let _ = tx.send(None);
            return;
        };
        let Some(prompt_tokens) = usage.prompt_tokens else {
            let _ = tx.send(None);
            return;
        };
        // n_ctx live из slot; fallback — ctx_size активной модели.
        let n_ctx_live = app
            .metrics
            .llama_slot
            .get_untracked()
            .and_then(|s| s.n_ctx);
        let n_ctx = n_ctx_live.or_else(|| {
            let selected = app.selected_model.get_untracked();
            let models = app.models.get_untracked();
            selected.and_then(|name| {
                models
                    .iter()
                    .find(|m| m.name == name)
                    .map(|m| m.ctx_size as i64)
            })
        });
        let Some(n_ctx) = n_ctx else {
            let _ = tx.send(None);
            return;
        };
        if n_ctx <= 0 {
            let _ = tx.send(None);
            return;
        }
        let ratio = prompt_tokens as f64 / n_ctx as f64;
        let trigger = ratio > (threshold as f64 / 100.0);
        let _ = tx.send(if trigger { Some(prompt_tokens) } else { None });
    });
    rx.await.ok().flatten()
}

/// Читает историю текущего чата на main-потоке и возвращает её в async-таск.
/// `None` — если случился abort.
async fn read_history_on_main(abort: &Arc<AtomicU64>, snapshot: u64) -> Option<Vec<ApiChatMessage>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let abort_c = abort.clone();
    run_on_main_thread(move || {
        if abort_c.load(Ordering::Relaxed) != snapshot {
            let _ = tx.send(None);
            return;
        }
        let chat = use_context::<AppCtx>().chat;
        let _ = tx.send(Some(build_history(&chat)));
    });
    match rx.await {
        Ok(v) => v,
        Err(_) => None,
    }
}

/// Обрабатывает пачку tool_calls: по каждому — подтверждение, исполнение,
/// запись bubble’ов в ленту. Возвращает `false`, если дальше крутить цикл
/// не надо (abort или user-cancel).
async fn process_tool_calls(
    calls: Vec<ChatToolCall>,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
) -> bool {
    for call in calls {
        if abort.load(Ordering::Relaxed) != snapshot {
            return false;
        }

        let name = call.function.name.clone().unwrap_or_default();

        // Инструмент неактивен — отвечаем сами, без диалога и executor’а.
        if !is_tool_active_on_main(&name).await {
            spawn_on_main({
                let call = call.clone();
                move || {
                    let ctx = use_context::<AppCtx>().chat;
                    ctx.messages.update(|v| {
                        v.push(ChatMsg::tool_result(
                            call.id.clone(),
                            call.function.name.clone().unwrap_or_default(),
                            format!(
                                "Инструмент «{}» отключён в настройках. \
                                 Активируйте его в правой панели, чтобы использовать.",
                                call.function.name.clone().unwrap_or_default()
                            ),
                            true,
                        ));
                    });
                }
            });
            continue;
        }

        let decision = await_decision_on_tool_call(&call, abort, snapshot).await;
        if abort.load(Ordering::Relaxed) != snapshot {
            return false;
        }
        match decision {
            ToolDecision::Cancel => {
                spawn_on_main({
                    let call = call.clone();
                    move || {
                        let ctx = use_context::<AppCtx>().chat;
                        ctx.messages.update(|v| {
                            v.push(ChatMsg::tool_result(
                                call.id.clone(),
                                call.function.name.clone().unwrap_or_default(),
                                "Отменено пользователем.".to_string(),
                                true,
                            ));
                        });
                    }
                });
                return false;
            }
            ToolDecision::AllowAll => {
                spawn_on_main(|| {
                    use_context::<AppCtx>().tools.allow_all.set(true);
                });
                // fallthrough — выполняем этот вызов.
            }
            ToolDecision::Allow => {}
        }

        let outcome = tools::execute(&call).await;
        if abort.load(Ordering::Relaxed) != snapshot {
            return false;
        }
        spawn_on_main(move || {
            let ctx = use_context::<AppCtx>().chat;
            ctx.messages.update(|v| {
                v.push(ChatMsg::tool_result(
                    outcome.tool_call_id.clone(),
                    outcome.name.clone(),
                    outcome.content.clone(),
                    outcome.error,
                ));
            });
        });
    }
    true
}

/// Проверяет активность инструмента по ключу — читает сигнал на main-потоке.
async fn is_tool_active_on_main(name: &str) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let name = name.to_string();
    run_on_main_thread(move || {
        let active = use_context::<AppCtx>().tools.active.get_untracked();
        let _ = tx.send(active.iter().any(|k| k == &name));
    });
    rx.await.unwrap_or(false)
}

/// Запрашивает решение по tool-call’у: если `allow_all` — сразу `Allow`,
/// иначе открывает диалог через `ctx.tools.pending_approval` и ждёт ответ.
///
/// Model-agnostic: используется и из llama-чата, и из `syn_chat::session`
/// (нативный Qwen3.6 agent-loop). Источники политики — общие в `AppCtx.general`.
pub(crate) async fn await_decision_on_tool_call(
    call: &ChatToolCall,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
) -> ToolDecision {
    let tool_key = call.function.name.clone().unwrap_or_default();

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
// Async-драйвер SSE-стрима
// ─────────────────────────────────────────────────────────────────────────────

/// Пуш-токена body в `streaming_body`-сигнал. На стрим-токенах НЕ трогаем
/// `messages`-сигнал — иначе подписка `scroll_list` (`message_area.rs`)
/// пересобирает весь Column со всеми MarkdownView на каждом chunk'е, и
/// при длинном диалоге UI лагает (квадратичная стоимость). Bubble
/// последнего ассистент-сообщения подписан только на `streaming_body` и
/// перерисовывает только себя; commit в `messages[last]` происходит
/// в [`commit_streaming_tail`] на финале turn'а.
fn push_streaming_body(piece: &str) {
    let chat = use_context::<AppCtx>().chat;
    chat.streaming_body.update(|s| s.push_str(piece));
}

/// Пуш-токена reasoning в `streaming_thinking`-сигнал. Симметрично
/// [`push_streaming_body`]: только последний bubble (через подписку
/// в `message_bubble.rs`) перерисовывает thinking-блок.
fn push_streaming_thinking(piece: &str) {
    let chat = use_context::<AppCtx>().chat;
    chat.streaming_thinking.update(|s| s.push_str(piece));
}

/// Сливает накопленные `streaming_body` и `streaming_thinking` в
/// `messages[last]` (при условии `Assistant/Text`), затем сбрасывает оба
/// сигнала. Вызывается на каждом завершении turn'а и перед операциями,
/// которые меняют последний bubble (commit tool-call, replace на system).
/// Идемпотентна: повторный вызов с пустыми сигналами — no-op.
fn commit_streaming_tail() {
    let chat = use_context::<AppCtx>().chat;
    let body = chat.streaming_body.get_untracked();
    let think = chat.streaming_thinking.get_untracked();
    if body.is_empty() && think.is_empty() {
        return;
    }
    chat.messages.update(|v| {
        if let Some(last) = v.last_mut() {
            if matches!(last.role, ChatMsgRole::Assistant)
                && matches!(last.kind, ChatMsgKind::Text)
            {
                last.body.push_str(&body);
                last.thinking.push_str(&think);
            }
        }
    });
    chat.streaming_body.set(String::new());
    chat.streaming_thinking.set(String::new());
}

/// Крутит SSE-стрим до `None` / `[DONE]`, пушит текст в последний
/// assistant-плейсхолдер, аккумулирует tool_calls-дельты. Возвращает
/// [`TurnOutcome`] — обычный текст либо собранный набор tool_calls.
async fn drive_stream<S>(
    mut stream: S,
    abort: &Arc<AtomicU64>,
    snapshot: u64,
) -> TurnOutcome
where
    S: futures_util::Stream<Item = Result<crate::llama::api::ChatStreamChunk, LlamaError>> + Unpin,
{
    let mut got_any_text = false;
    let mut stream_error: Option<String> = None;
    let mut tool_acc: Vec<(i32, AccumulatedToolCall)> = Vec::new();
    let mut had_tool_calls = false;
    let mut finish_reason: Option<FinishReason> = None;
    // Был ли вообще reasoning/thinking — отличает «модель просто ничего не
    // сказала» от «закончила думать и замолчала, не выдав ответ». Auto-continue
    // делаем только во втором случае.
    let mut had_reasoning = false;
    // Локальный буфер body текущего turn'а. ВАЖНО: drive_stream крутится
    // в tokio worker'е, а сигналы `chat.streaming_body` thread-local —
    // читать их отсюда = panic. Дублируем в локальный String, который
    // async-safe.
    let mut body_buf = String::new();
    // Парсер `<think>...</think>` тегов для случаев, когда llama-server
    // запущен с `reasoning_format=none` и эмиттит reasoning прямо в
    // `delta.content`. Состояние держится между чанками одного turn'а.
    let mut think_parser = super::think_parser::ThinkParser::new();

    while let Some(ev) = stream.next().await {
        if abort.load(Ordering::Relaxed) != snapshot {
            break;
        }
        match ev {
            Ok(chunk) => {
                // Reasoning через отдельное поле `delta.reasoning_content` —
                // современный формат llama-server при `reasoning_format=auto|deepseek`.
                if let Some(r) = chunk.first_delta_reasoning() {
                    if !r.is_empty() {
                        had_reasoning = true;
                        let piece = r.to_string();
                        let abort_c = abort.clone();
                        run_on_main_thread(move || {
                            if abort_c.load(Ordering::Relaxed) != snapshot {
                                return;
                            }
                            push_streaming_thinking(&piece);
                        });
                    }
                }

                // Текстовая дельта: прогоняем через `ThinkParser`, чтобы
                // отделить тело ответа от reasoning, если он встроен в
                // content тегами `<think>...</think>`. На обычных моделях
                // (без тегов) парсер прозрачно отдаёт всё в `body`.
                if let Some(delta) = chunk.first_delta_content() {
                    if !delta.is_empty() {
                        let split = think_parser.feed(delta);
                        if !split.body.is_empty() {
                            got_any_text = true;
                            body_buf.push_str(&split.body);
                            let body_piece = split.body;
                            let abort_c = abort.clone();
                            run_on_main_thread(move || {
                                if abort_c.load(Ordering::Relaxed) != snapshot {
                                    return;
                                }
                                push_streaming_body(&body_piece);
                            });
                        }
                        if !split.thinking.is_empty() {
                            had_reasoning = true;
                            let think_piece = split.thinking;
                            let abort_c = abort.clone();
                            run_on_main_thread(move || {
                                if abort_c.load(Ordering::Relaxed) != snapshot {
                                    return;
                                }
                                push_streaming_thinking(&think_piece);
                            });
                        }
                    }
                }

                // Tool-calls дельты — аккумулируем и сразу апдейтим UI-bubble.
                if let Some(choice) = chunk.choices.first() {
                    if let Some(deltas) = choice.delta.tool_calls.as_ref() {
                        if !deltas.is_empty() {
                            had_tool_calls = true;
                            fold_tool_call_deltas(&mut tool_acc, deltas);
                            let snapshot_calls: Vec<ChatToolCall> = tool_acc
                                .iter()
                                .map(|(idx, a)| {
                                    let cloned = AccumulatedToolCall {
                                        id: a.id.clone(),
                                        kind: a.kind.clone(),
                                        name: a.name.clone(),
                                        arguments: a.arguments.clone(),
                                    };
                                    cloned.finalize(*idx)
                                })
                                .collect();
                            let abort_c = abort.clone();
                            run_on_main_thread(move || {
                                if abort_c.load(Ordering::Relaxed) != snapshot {
                                    return;
                                }
                                update_streaming_tool_call_bubble(&snapshot_calls);
                            });
                        }
                    }
                    if let Some(fr) = choice.finish_reason {
                        finish_reason = Some(fr);
                    }
                }

                // Финальный чанк — пишем usage/timings (как раньше).
                let has_metrics = chunk.timings.is_some() || chunk.usage.is_some();
                if has_metrics {
                    let timings = chunk.timings.clone();
                    let usage = chunk.usage.clone();
                    let abort_c = abort.clone();
                    run_on_main_thread(move || {
                        if abort_c.load(Ordering::Relaxed) != snapshot {
                            return;
                        }
                        let ctx = use_context::<AppCtx>();
                        metrics::llama::apply_final_chunk(
                            &ctx.metrics,
                            timings.as_ref(),
                            usage.as_ref(),
                        );
                    });
                }
            }
            Err(e) => {
                stream_error = Some(format_error(&e, ""));
                break;
            }
        }
    }

    let was_aborted = abort.load(Ordering::Relaxed) != snapshot;

    if let Some(err) = stream_error {
        run_on_main_thread(move || {
            commit_streaming_tail();
            let ctx = use_context::<AppCtx>().chat;
            ctx.error.set(Some(err.clone()));
            ctx.messages.update(|v| {
                if !got_any_text && !had_tool_calls {
                    replace_last_assistant_with_system(v, err, true);
                } else if let Some(last) = v.last_mut() {
                    last.body.push_str("\n\n_Ответ прерван: ");
                    last.body.push_str(&err);
                    last.body.push('_');
                }
            });
        });
        return TurnOutcome::Error;
    }

    if was_aborted {
        run_on_main_thread(move || {
            commit_streaming_tail();
            let ctx = use_context::<AppCtx>().chat;
            ctx.messages.update(|v| {
                if !got_any_text && !had_tool_calls {
                    replace_last_assistant_with_system(
                        v,
                        "Запрос отменён до первого токена.".to_string(),
                        false,
                    );
                } else if let Some(last) = v.last_mut() {
                    last.body.push_str("\n\n_(прервано)_");
                }
            });
        });
        return TurnOutcome::Error;
    }

    // Финализация — tool_calls vs обычный текст.
    if had_tool_calls
        || matches!(finish_reason, Some(FinishReason::ToolCalls))
        || (!got_any_text && !tool_acc.is_empty())
    {
        let calls: Vec<ChatToolCall> = tool_acc
            .drain(..)
            .map(|(idx, a)| a.finalize(idx))
            .collect();
        if calls.is_empty() {
            tracing::debug!(
                target: "agent",
                ?finish_reason,
                got_any_text,
                "stream end: empty tool_acc → fallback to Text"
            );
            run_on_main_thread(commit_streaming_tail);
            return TurnOutcome::Text;
        }
        tracing::debug!(
            target: "agent",
            ?finish_reason,
            n_calls = calls.len(),
            "stream end: tool_calls"
        );
        // Финализируем UI-bubble: превращаем плейсхолдер в ToolCall с
        // окончательными аргументами. Перед replace'ом сливаем накопленный
        // streaming-хвост (если был partial текст до tool_call) в bubble,
        // иначе он потеряется при замене на ToolCall-карточку.
        let calls_ui = calls.clone();
        run_on_main_thread(move || {
            commit_streaming_tail();
            commit_tool_call_bubble(&calls_ui);
        });
        return TurnOutcome::ToolCalls(calls);
    }

    // Soft-stop: модель завершила стрим со `Stop`/`Length`/`None`, ушла в
    // reasoning, но не выдала ни одного байта body после `</think>` —
    // классическое «закончила думать и замолчала». `run_agent` сделает
    // auto-continue (с лимитом `MAX_SOFT_STOP_RETRIES`).
    let is_pure_stop = matches!(
        finish_reason,
        Some(FinishReason::Stop) | Some(FinishReason::Length) | None
    );
    if is_pure_stop && !got_any_text && had_reasoning {
        tracing::info!(
            target: "agent",
            ?finish_reason,
            had_reasoning,
            "stream end: soft-stop after reasoning (auto-continue)"
        );
        // body пустой по определению, thinking — реальный; коммитим оба,
        // чтобы UI после retry не «забыл» reasoning, а next turn начал с
        // чистого `streaming_thinking`.
        run_on_main_thread(commit_streaming_tail);
        return TurnOutcome::SoftStop;
    }

    // КРИТИЧЕСКИЙ FIX: Пустой ответ от модели (особенно Qwen на llama.cpp).
    // Когда finish_reason=Stop/None, но нет ни текста, ни reasoning, ни tool_calls —
    // это баг/особенность семплера. Делаем auto-continue как SoftStop вместо
    // завершения цикла с пустым ответом. Без этого пользователь видит "молчание"
    // и вынужден жать "Continue" вручную.
    if is_pure_stop && !got_any_text && !had_reasoning && !had_tool_calls {
        tracing::info!(
            target: "agent",
            ?finish_reason,
            "stream end: soft-stop on empty response (auto-continue)"
        );
        run_on_main_thread(commit_streaming_tail);
        return TurnOutcome::SoftStop;
    }

    // Если дошли сюда — ни один из soft-stop триггеров не сработал.
    // Логируем причину почему мы считаем этот ответ "завершённым".
    if got_any_text {
        tracing::info!(
            target: "agent",
            ?finish_reason,
            body_len = body_buf.len(),
            last_char = %body_buf.trim_end().chars().last().unwrap_or('?'),
            body_preview = %body_buf.chars().take(80).collect::<String>(),
            "stream end: text (body present)"
        );
    } else {
        tracing::info!(
            target: "agent",
            ?finish_reason,
            had_reasoning,
            had_tool_calls,
            "stream end: text (empty body)"
        );
    }
    run_on_main_thread(commit_streaming_tail);
    TurnOutcome::Text
}

/// Поддерживает «стриминговый» вид tool-call пузыря: последний
/// assistant-placeholder (`kind=Text, body="", thinking=""`) или уже
/// существующий `kind=ToolCall` апдейтится свежим срезом аргументов. Если
/// до tool_call'а в стриме был preamble-текст или reasoning — сначала
/// коммитим его через `commit_streaming_tail` (вольёт в `last.body`/
/// `last.thinking`), и тогда replace-ветка не съедает preamble: last уже
/// не пустой, идём в push-ветку и создаём НОВЫЙ ToolCall-bubble.
fn update_streaming_tool_call_bubble(calls: &[ChatToolCall]) {
    if calls.is_empty() {
        return;
    }
    let first = &calls[0];
    let tool_name = first.function.name.clone().unwrap_or_default();
    let args_pretty = tools::pretty_args(first.function.arguments.as_deref());
    let chat = use_context::<AppCtx>().chat;

    // Snapshot streaming-сигналов ДО любых update'ов: иначе reactive
    // подписки в UI могут увидеть промежуточное состояние.
    let body_pending = chat.streaming_body.get_untracked();
    let think_pending = chat.streaming_thinking.get_untracked();
    if !body_pending.is_empty() || !think_pending.is_empty() {
        // Сливаем хвост в текущий placeholder (ещё `Assistant/Text`),
        // чтобы preamble стал отдельным закоммиченным bubble выше.
        commit_streaming_tail();
    }

    chat.messages.update(|v| {
        if let Some(last) = v.last_mut() {
            match &last.kind {
                // Полностью пустой placeholder — replace на ToolCall.
                // Проверяем И body, И thinking: reasoning-only placeholder
                // тоже нельзя терять.
                ChatMsgKind::Text
                    if last.role == ChatMsgRole::Assistant
                        && last.body.is_empty()
                        && last.thinking.is_empty() =>
                {
                    *last = ChatMsg::tool_call(tool_name, args_pretty, calls.to_vec());
                }
                // Уже ToolCall (последующие tool_calls дельты) — апдейт.
                ChatMsgKind::ToolCall { .. } => {
                    last.body = args_pretty;
                    last.kind = ChatMsgKind::ToolCall { tool_name };
                    last.tool_calls = Some(calls.to_vec());
                }
                // Был preamble (текст/thinking) — last это закоммиченный
                // assistant-text. Создаём НОВЫЙ ToolCall-bubble.
                _ => {
                    v.push(ChatMsg::tool_call(tool_name, args_pretty, calls.to_vec()));
                }
            }
        } else {
            v.push(ChatMsg::tool_call(tool_name, args_pretty, calls.to_vec()));
        }
    });
}

/// Финализация ToolCall-bubble в конце turn’а: гарантируем, что последний
/// элемент ленты — это ToolCall с правильными tool_calls.
fn commit_tool_call_bubble(calls: &[ChatToolCall]) {
    if calls.is_empty() {
        return;
    }
    let first = &calls[0];
    let tool_name = first.function.name.clone().unwrap_or_default();
    let args_pretty = tools::pretty_args(first.function.arguments.as_deref());
    let ctx = use_context::<AppCtx>().chat;
    ctx.messages.update(|v| {
        if let Some(last) = v.last_mut() {
            match &last.kind {
                // Полностью пустой placeholder — replace. И body, и thinking
                // должны быть пустыми, иначе теряем preamble/reasoning.
                ChatMsgKind::Text
                    if last.role == ChatMsgRole::Assistant
                        && last.body.is_empty()
                        && last.thinking.is_empty() =>
                {
                    *last = ChatMsg::tool_call(tool_name, args_pretty, calls.to_vec());
                    return;
                }
                ChatMsgKind::ToolCall { .. } => {
                    last.body = args_pretty;
                    last.kind = ChatMsgKind::ToolCall { tool_name };
                    last.tool_calls = Some(calls.to_vec());
                    return;
                }
                _ => {}
            }
        }
        v.push(ChatMsg::tool_call(tool_name, args_pretty, calls.to_vec()));
    });
}

/// Обработчик «не удалось даже начать стрим» (ошибка connect/DNS/TLS/HTTP).
fn finish_with_error(message: String) {
    run_on_main_thread(move || {
        let ctx = use_context::<AppCtx>().chat;
        ctx.error.set(Some(message.clone()));
        ctx.messages.update(|v| {
            replace_last_assistant_with_system(v, message, true);
        });
    });
}

/// Если в хвосте ленты пустой assistant-placeholder, заменить его на system-
/// сообщение (с флагом ошибки). Иначе — просто добавить system-сообщение
/// в конец (сохраняем уже собранный частичный ответ).
fn replace_last_assistant_with_system(v: &mut Vec<ChatMsg>, body: String, error: bool) {
    let last_is_empty_assistant = v
        .last()
        .map(|m| {
            m.role == ChatMsgRole::Assistant
                && matches!(m.kind, ChatMsgKind::Text)
                && m.body.is_empty()
        })
        .unwrap_or(false);

    if last_is_empty_assistant {
        if let Some(last) = v.last_mut() {
            *last = ChatMsg::system(body, error);
        }
    } else {
        v.push(ChatMsg::system(body, error));
    }
}

/// Удобный sugar: закинуть замыкание на main-поток без дополнительных
/// проверок. Отдельная функция, чтобы не повторять импорт.
fn spawn_on_main<F: FnOnce() + Send + 'static>(f: F) {
    run_on_main_thread(f);
}

// ─────────────────────────────────────────────────────────────────────────────
// Форматирование ошибок
// ─────────────────────────────────────────────────────────────────────────────

/// Преобразует [`LlamaError`] в понятную пользователю строку. `base_url`
/// подставляется в сообщение про недоступный сервер; можно передать пустую
/// строку, если контекст (например, обрыв посреди стрима) не требует URL.
/// `true` если ошибка — backend не смог распарсить JSON в `tool_call.arguments`
/// (типично для local-семплеров, которые иногда выдают broken-JSON).
/// Используется для тихого retry без показа fatal error пользователю.
pub fn is_tool_call_parse_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("tool call") && (m.contains("parse") || m.contains("json"))
}

pub fn format_error(err: &LlamaError, base_url: &str) -> String {
    match err {
        LlamaError::Transport(e) if e.is_connect() => {
            if base_url.is_empty() {
                "Сервер llama недоступен. Запустите его в правом сайдбаре.".to_string()
            } else {
                format!(
                    "Сервер llama недоступен на {}. Запустите его в правом сайдбаре.",
                    base_url
                )
            }
        }
        LlamaError::Transport(e) if e.is_timeout() => "Таймаут запроса к llama.".to_string(),
        LlamaError::Transport(e) => format!("Сетевая ошибка: {}", e),
        LlamaError::Http { status, payload } => {
            use crate::llama::api::ApiErrorPayload;
            let detail = match payload {
                ApiErrorPayload::Parsed(api) => api.message.clone(),
                ApiErrorPayload::Raw(body) => truncate_for_ui(body, 256),
            };
            format!("HTTP {}: {}", status, detail)
        }
        LlamaError::Decode { body_hint, .. } => {
            format!(
                "Некорректный ответ сервера (JSON): {}",
                truncate_for_ui(body_hint, 256)
            )
        }
        LlamaError::Sse(s) => format!("Ошибка SSE-потока: {}", s),
    }
}

/// Обрезка строки до `limit` байт с сохранением char-boundary.
fn truncate_for_ui(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::api::{ChatChunkToolCall, ChatToolCallFunction};

    #[test]
    fn truncate_keeps_char_boundary() {
        assert_eq!(truncate_for_ui("abcdef", 3), "abc…");
        let s = "Привет, мир!";
        let got = truncate_for_ui(s, 5);
        assert!(got.ends_with('…'));
        assert!(got.is_char_boundary(got.len() - "…".len()));
    }


    #[test]
    fn fold_deltas_glues_arguments_and_captures_id_name() {
        let mut acc: Vec<(i32, AccumulatedToolCall)> = Vec::new();
        fold_tool_call_deltas(
            &mut acc,
            &[ChatChunkToolCall {
                index: 0,
                id: Some("c1".into()),
                kind: Some("function".into()),
                function: Some(ChatToolCallFunction {
                    name: Some("bash".into()),
                    arguments: Some(r#"{"comm"#.into()),
                }),
            }],
        );
        fold_tool_call_deltas(
            &mut acc,
            &[ChatChunkToolCall {
                index: 0,
                id: None,
                kind: None,
                function: Some(ChatToolCallFunction {
                    name: None,
                    arguments: Some(r#"and":"echo hi"}"#.into()),
                }),
            }],
        );
        assert_eq!(acc.len(), 1);
        let finalized = acc.pop().unwrap().1.finalize(0);
        assert_eq!(finalized.id, "c1");
        assert_eq!(finalized.function.name.as_deref(), Some("bash"));
        assert_eq!(
            finalized.function.arguments.as_deref(),
            Some(r#"{"command":"echo hi"}"#)
        );
    }

    #[test]
    fn build_history_maps_tool_call_and_result() {
        let calls = vec![ChatToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: ChatToolCallFunction {
                name: Some("bash".into()),
                arguments: Some(r#"{"command":"echo hi"}"#.into()),
            },
        }];
        let msgs = vec![
            ChatMsg::user("запусти echo"),
            ChatMsg::tool_call("bash", r#"{"command":"echo hi"}"#, calls),
            ChatMsg::tool_result("c1", "bash", "exit=0 stdout=hi", false),
            ChatMsg::assistant_empty(),
        ];
        let api = build_history_from_slice_at(String::new(), &msgs, "2026-05-01".into(), "", "");
        // system(дата) + user + assistant(tool_calls) + tool(c1)
        // — пустой placeholder не отправляется.
        assert_eq!(api.len(), 4);
        assert_eq!(api[0].role, ChatRole::System);
        assert_eq!(api[1].role, ChatRole::User);
        assert_eq!(api[2].role, ChatRole::Assistant);
        assert!(api[2].tool_calls.is_some());
        assert_eq!(api[3].role, ChatRole::Tool);
        assert_eq!(api[3].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn build_history_skips_compacted_messages() {
        // Свёрнутые `compacted_iter=Some(_)` сообщения в API не уходят.
        // Маркер итерации превращается в `system`-сообщение с summary.
        let mut user1 = ChatMsg::user("первый вопрос");
        user1.compacted_iter = Some(1);
        let mut asst1 = ChatMsg::assistant_empty();
        asst1.body = "первый ответ".into();
        asst1.compacted_iter = Some(1);
        let marker = ChatMsg::compaction_marker(1, 2, 1500, 200, "пользователь спросил X, ассистент ответил Y");

        let msgs = vec![
            marker,
            user1,
            asst1,
            ChatMsg::user("второй вопрос"),
        ];
        let api = build_history_from_slice_at(String::new(), &msgs, "2026-05-01".into(), "", "");
        // system(дата) + system(marker summary) + user(второй вопрос).
        assert_eq!(api.len(), 3);
        assert_eq!(api[0].role, ChatRole::System);
        assert_eq!(api[1].role, ChatRole::System);
        assert!(api[1]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap_or("")
            .contains("autocompact-итерация 1"));
        assert_eq!(api[2].role, ChatRole::User);
    }

    #[test]
    fn build_history_synthesizes_stub_for_dangling_tool_call() {
        // ToolCall без соответствующего ToolResult — «висячий» после краша/рестарта.
        let calls = vec![ChatToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: ChatToolCallFunction {
                name: Some("bash".into()),
                arguments: Some(r#"{}"#.into()),
            },
        }];
        let msgs = vec![
            ChatMsg::user("go"),
            ChatMsg::tool_call("bash", "{}", calls),
        ];
        let api = build_history_from_slice_at(String::new(), &msgs, "2026-05-01".into(), "", "");
        // system(дата) + user + assistant(tool_calls) + tool(stub)
        assert_eq!(api.len(), 4);
        assert_eq!(api[3].role, ChatRole::Tool);
        assert!(api[3]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap_or("")
            .contains("restart"));
    }

    #[test]
    fn build_history_user_with_attachments_emits_multipart() {
        // Реальный data: URL не нужен — `build_history` дёргает blob CAS.
        // Тест-заглушка: создаём `MsgAttachment` с известным sha и mime,
        // но без файла на диске → `data_url` вернёт Err и attachment будет
        // пропущен с `log::warn!`. Зато текстовая часть всё равно уйдёт
        // как Parts (multipart-режим включается по факту наличия attachments).
        use crate::chat::state::MsgAttachment;
        let att = MsgAttachment {
            sha256: "deadbeef".repeat(8),
            mime: "image/png".into(),
            original_name: String::new(),
            width: 0,
            height: 0,
            size_bytes: 0,
        };
        let msgs = vec![ChatMsg::user_with_attachments("опиши", vec![att])];
        let api = build_history_from_slice_at(String::new(), &msgs, "2026-05-01".into(), "", "");
        // system(дата) + user
        assert_eq!(api.len(), 2);
        assert_eq!(api[0].role, ChatRole::System);
        assert_eq!(api[1].role, ChatRole::User);
        // content должен быть Parts (multipart), а не Text:
        let content = api[1].content.as_ref().expect("content present");
        assert!(content.as_text().is_none(), "ожидаем multipart, не plain text");
    }

    // ── regenerate_from / find_last_user_before ─────────────────────────

    fn user_msg(body: &str) -> ChatMsg {
        ChatMsg::user(body.to_string())
    }
    fn assistant_text(body: &str) -> ChatMsg {
        let mut m = ChatMsg::assistant_empty();
        m.body = body.to_string();
        m
    }
    fn assistant_tool_call() -> ChatMsg {
        ChatMsg::tool_call("bash".to_string(), "{}".to_string(), Vec::new())
    }
    fn tool_result_msg() -> ChatMsg {
        ChatMsg::tool_result("c1".to_string(), "bash".to_string(), "ok".to_string(), false)
    }

    #[test]
    fn find_last_user_anchor_in_text_returns_preceding_user() {
        // [User, Assistant] — клик по assistant (idx 1) → ожидаем user idx 0.
        let msgs = vec![user_msg("hi"), assistant_text("hello")];
        assert_eq!(find_last_user_before(&msgs, 1), Some(0));
    }

    #[test]
    fn find_last_user_anchor_in_tool_chain_skips_to_user() {
        // [User, ToolCall, ToolResult, Assistant] — клик по последнему
        // assistant (idx 3) обязан вернуть user idx 0.
        let msgs = vec![
            user_msg("сделай"),
            assistant_tool_call(),
            tool_result_msg(),
            assistant_text("done"),
        ];
        assert_eq!(find_last_user_before(&msgs, 3), Some(0));
    }

    #[test]
    fn find_last_user_picks_most_recent_user() {
        // Несколько user'ов — берём последний *перед* anchor.
        let msgs = vec![
            user_msg("first"),
            assistant_text("a1"),
            user_msg("second"),
            assistant_text("a2"),
        ];
        assert_eq!(find_last_user_before(&msgs, 3), Some(2));
        // Если anchor — между user'ами, возвращается первый.
        assert_eq!(find_last_user_before(&msgs, 1), Some(0));
    }

    #[test]
    fn find_last_user_no_user_returns_none() {
        // Только system + assistant (без user) — некуда отступать.
        let msgs = vec![ChatMsg::system("error".to_string(), true), assistant_text("oops")];
        assert_eq!(find_last_user_before(&msgs, 1), None);
    }

    #[test]
    fn find_last_user_empty_returns_none() {
        let msgs: Vec<ChatMsg> = Vec::new();
        assert_eq!(find_last_user_before(&msgs, 0), None);
        assert_eq!(find_last_user_before(&msgs, 100), None);
    }

    #[test]
    fn find_last_user_anchor_out_of_bounds_clamps() {
        let msgs = vec![user_msg("hi"), assistant_text("hello")];
        // anchor за пределами — clamp до len-1, всё равно находим user.
        assert_eq!(find_last_user_before(&msgs, 999), Some(0));
    }

    #[test]
    fn find_last_user_skips_assistant_text_user_role_combo() {
        // ToolCall с role=Assistant не должен спутать с user.
        let msgs = vec![
            user_msg("u1"),
            assistant_tool_call(),
            tool_result_msg(),
            user_msg("u2"),
            assistant_text("a2"),
        ];
        assert_eq!(find_last_user_before(&msgs, 4), Some(3));
        assert_eq!(find_last_user_before(&msgs, 2), Some(0));
    }

    // ── today_utc_iso / системный префикс с датой ───────────────────────

    #[test]
    fn today_utc_iso_known_timestamps() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(today_utc_iso(0), "1970-01-01");
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(today_utc_iso(946_684_800), "2000-01-01");
        // 2020-02-29 12:00:00 UTC = 1582977600 (leap year)
        assert_eq!(today_utc_iso(1_582_977_600), "2020-02-29");
        // 2026-05-01 00:00:00 UTC = 1777593600
        assert_eq!(today_utc_iso(1_777_593_600), "2026-05-01");
    }

    #[test]
    fn build_history_always_prefixes_system_with_date() {
        // Пустой пользовательский system_prompt — всё равно уйдёт system с датой.
        let api = build_history_from_slice_at(String::new(), &[], "2026-05-01".into(), "", "");
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].role, ChatRole::System);
        let body = api[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(body.contains("2026-05-01"), "ожидаем дату в system: {body}");
    }

    #[test]
    fn build_history_always_includes_action_rule() {
        // ПРАВИЛО ДЕЙСТВИЯ должно подмешиваться в каждый запрос — это
        // защита от случая, когда пользователь почистил system_prompt в
        // настройках до первой строки и DEFAULT_SYSTEM_PROMPT не применяется.
        let api_empty =
            build_history_from_slice_at(String::new(), &[], "2026-05-01".into(), "", "");
        let body = api_empty[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(body.contains("ПРАВИЛО ДЕЙСТВИЯ"), "ожидаем правило в system: {body}");

        // С пользовательским prompt'ом — правило по-прежнему присутствует.
        let api_custom = build_history_from_slice_at(
            "Я кратко отвечаю.".into(),
            &[],
            "2026-05-01".into(),
            "",
            "",
        );
        let body2 = api_custom[0]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap_or("");
        assert!(body2.contains("ПРАВИЛО ДЕЙСТВИЯ"), "правило теряется: {body2}");
        assert!(body2.contains("Я кратко отвечаю."));
    }

    #[test]
    fn build_history_merges_date_and_user_system_prompt() {
        // Передаём непустой user-prompt и проверяем, что в финальном
        // system-сообщении присутствуют ОБА элемента: дата и user-текст.
        let api = build_history_from_slice_at(
            "Ты — ассистент.".into(),
            &[],
            "2026-05-01".into(),
            "",
            "",
        );
        assert_eq!(api.len(), 1);
        let body = api[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(body.contains("2026-05-01"));
        assert!(body.contains("Ты — ассистент."));
    }

    // ── augment-блок RAG в system prompt ────────────────────────────────

    #[test]
    fn build_history_includes_augment_block_when_nonempty() {
        let augment = "=== Контекст из базы знаний ===\n[1] [KB] doc.md:\nкусок текста\n=== Конец контекста ===";
        let api = build_history_from_slice_at(
            "Пользовательский prompt.".into(),
            &[],
            "2026-05-01".into(),
            "",
            augment,
        );
        assert_eq!(api.len(), 1);
        let body = api[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(body.contains("кусок текста"), "augment-снippet потерян: {body}");
        assert!(body.contains("Контекст из базы знаний"));
    }

    #[test]
    fn build_history_skips_augment_when_empty() {
        // Пустой augment_text (или whitespace) — блок не вставляется,
        // нет лишних `\n\n` между preamble и user_system_prompt.
        let api_empty =
            build_history_from_slice_at("Pro.".into(), &[], "2026-05-01".into(), "", "");
        let body_empty = api_empty[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(!body_empty.contains("Контекст из базы знаний"));

        let api_ws =
            build_history_from_slice_at("Pro.".into(), &[], "2026-05-01".into(), "", "   \n\n ");
        let body_ws = api_ws[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        assert!(!body_ws.contains("Контекст из базы знаний"));
    }

    #[test]
    fn build_history_augment_order_preamble_then_augment_then_user_prompt() {
        // Порядок строго: preamble (date + ACTION_RULE) → augment → user_system_prompt.
        // Проверяем, что в финальном тексте они идут именно в этом порядке.
        let api = build_history_from_slice_at(
            "USER_PROMPT_MARKER".into(),
            &[],
            "2026-05-01".into(),
            "",
            "AUGMENT_MARKER",
        );
        let body = api[0].content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
        let rule_idx = body.find("ПРАВИЛО ДЕЙСТВИЯ").expect("ACTION_RULE присутствует");
        let aug_idx = body.find("AUGMENT_MARKER").expect("augment вставлен");
        let user_idx = body.find("USER_PROMPT_MARKER").expect("user prompt вставлен");
        assert!(rule_idx < aug_idx, "augment должен идти после ACTION_RULE");
        assert!(aug_idx < user_idx, "user prompt должен идти после augment'а");
    }

    // ── build_autoskill_chat_tool ───────────────────────────────────────

    fn make_skill(id: &str, name: &str, description: &str) -> crate::skills::Skill {
        crate::skills::Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            content: String::new(),
        }
    }

    /// Маленькая чистая функция-копия логики `build_autoskill_chat_tool`,
    /// без зависимости от `AppCtx` (он сидит в thread-local и не доступен
    /// в обычных unit-тестах). Принимает напрямую список скилов.
    fn build_autoskill_for_tests(skills: &[crate::skills::Skill]) -> crate::llama::api::ChatTool {
        use super::tools::catalog::KEY_AUTOSKILL;
        let descriptor = Tool::by_key(KEY_AUTOSKILL).expect("autoskill descriptor present");
        let mut description = descriptor.description.to_string();
        description.push_str("\n\nДоступные скилы (id — описание):\n");
        if skills.is_empty() {
            description.push_str("- (пусто, у пользователя нет скилов)\n");
        } else {
            for s in skills {
                let desc = if s.description.is_empty() {
                    "(без описания)"
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
        crate::llama::api::ChatTool {
            kind: "function".to_string(),
            function: crate::llama::api::ToolFunctionSchema {
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
            make_skill("greet", "Приветствие", "Тон первого сообщения"),
            make_skill("apo", "Извинения", ""),
        ];
        let tool = build_autoskill_for_tests(&skills);
        let desc = tool.function.description.unwrap();
        assert!(desc.contains("- greet — Тон первого сообщения"), "{desc}");
        assert!(desc.contains("- apo — (без описания)"), "{desc}");
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
        assert!(desc.contains("(пусто, у пользователя нет скилов)"));
        let params = tool.function.parameters.unwrap();
        // enum НЕ добавляется при пустом списке — иначе модель не сможет
        // подставить ни одного валидного значения.
        assert!(params
            .get("properties")
            .and_then(|p| p.get("id"))
            .and_then(|f| f.get("enum"))
            .is_none());
    }

    #[test]
    fn backward_compat_old_chat_msg_json_has_text_kind() {
        // Старый формат: без поля `kind`/`tool_calls`.
        let raw = serde_json::json!({
            "role": "User",
            "author": "Вы",
            "initials": "ВЫ",
            "tone_class": "avatar-slate",
            "time": "10:00",
            "body": "hi",
            "error": false
        });
        let m: ChatMsg = serde_json::from_value(raw).unwrap();
        assert!(matches!(m.kind, ChatMsgKind::Text));
        assert!(m.tool_calls.is_none());
    }
}
