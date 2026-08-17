//! Запуск in-process генерации через llm-qwen36 с поддержкой tool-calling.
//!
//! `send_message` собирает историю + tool-schemas, спавнит worker-thread и в
//! локальном tokio current_thread runtime гоняет [`run_agent_loop`]:
//!
//! ```text
//! loop turn in 0..MAX_AGENT_TURNS:
//!     prompt = apply_chat_template_ex_tools(history, tools)
//!     result = generate_streaming(prompt) ─┐
//!                                          │  callback парсит <tool_call>
//!                                          │  блоки через ToolCallParser:
//!                                          │  закрытие тега → return false
//!                                          │  (модель «договорила» tool-call)
//!     (calls, tail) = parser.finish()      │
//!     if calls.is_empty(): break  # обычный текстовый ответ
//!     for call in calls:
//!         decision = await_decision(call)  # Portal-диалог подтверждения
//!         outcome = tools::execute(call)   # bash/web/kb_search/...
//!         push tool_call+tool_result в UI
//!         append Message::tool(content) в history
//!     append assistant-msg c <tool_call> в history; placeholder в UI
//! ```
//!
//! Delta batching: callback аккумулирует delta-токены в локальном буфере и
//! сбрасывает их в сигналы раз в ~16 мс (или при detected stop). Без этого
//! при 50 tok/s × ~3 char/token = ~150 update/s main thread задушивается.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synaptix::facade::llm::{LlmGeneration, LlmTokenizer, MediaEmbedding, Message};

use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
use crate::agent::tools::{self, Tool, ToolDecision};
use crate::context::AppCtx;
use crate::syn_chat::attach::prompt::{self as attach_prompt, MediaCaps};
use crate::syn_chat::channel_parser::{self, ChannelIds, ChannelParser, ATEM_CLOSE};
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{ChatMsg, ChatMsgRole, MsgAttachment, SynChatCtx, ThinkParser};
use crate::syn_chat::tool_parser::{RawToolCall, ToolCallParser};

/// Cap частоты обновлений streaming-сигналов из worker thread. При 16 мс
/// ≈ 60 fps — UI получает свежий хвост, не задыхаясь от 100+ updates/s.
const FLUSH_INTERVAL_MS: u64 = 16;
/// Qwen3 chat-template завершает каждый turn `<|im_end|>`, но `eos_token_id`
/// в `config.json` обычно содержит только `<|endoftext|>` (151643). Без явной
/// остановки на `<|im_end|>` модель «расплывается» — генерит свой следующий
/// `user`-turn и продолжает диалог сама с собой.
const IM_END_TOKEN: &str = "<|im_end|>";
/// Закрытие tool-call блока — stop-sequence для генерации, чтобы модель
/// не уходила додумывать после tool-вызова.
pub(crate) const TOOL_CALL_CLOSE: &str = "</tool_call>";
/// Лимит итераций agent-loop: модель в режиме tool-calling может зациклиться,
/// поэтому ограничиваем общее число turn-ов. 16 — баланс между «успеть
/// решить многошаговую задачу» и «не сжечь весь контекст».
const MAX_AGENT_TURNS: usize = 16;

/// Отправляет сообщение от пользователя и запускает генерацию ответа.
/// Вызывается с main thread (использует use_context).
pub fn send_message(text: String) {
    let ctx = use_context::<SynChatCtx>();
    let attachments = ctx.pending_attachments.get_untracked();
    let text = text.trim().to_string();
    // Сообщение из одних вложений — валидный сценарий («что на картинке?»
    // можно и не писать), поэтому пустой текст блокирует отправку только
    // когда прикреплять тоже нечего.
    if text.is_empty() && attachments.is_empty() {
        return;
    }

    let registry = use_context::<SynModelRegistry>();

    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some("Модель не загружена".into()));
        return;
    };
    if ctx.pending.get_untracked() {
        return;
    }
    if ctx.attach_busy.get_untracked() > 0 {
        ctx.error.set(Some("Дождитесь обработки вложений".into()));
        return;
    }

    // 1. Append user-message + плейсхолдер ассистента.
    ctx.messages.update(|m| {
        m.push(ChatMsg::user_with_attachments(text.clone(), attachments.clone()));
        m.push(ChatMsg::assistant_empty());
    });
    ctx.pending_attachments.set(Vec::new());
    ctx.input.set(String::new());
    ctx.input_gen.update(|v| *v += 1);
    ctx.input_tokens.set_always(0);
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.error.set(None);
    ctx.pending.set(true);

    start_agent_thread(model, ctx);
}

/// Прерывает текущую генерацию. Worker увидит несовпадение abort-счётчика
/// в callback'е и вернёт false.
pub fn abort_current() {
    let ctx = use_context::<SynChatCtx>();
    ctx.abort.fetch_add(1, Ordering::Relaxed);
}

/// Перегенерировать последний ответ ассистента. Если хвост — assistant
/// (либо с body, либо placeholder), он удаляется из ленты и стартует свежая
/// генерация с тем же user-сообщением. Если хвост — user, просто добавляем
/// пустой placeholder и стартуем.
pub fn regenerate_last() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        ctx.abort.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some("Модель не загружена".into()));
        return;
    };

    // 1. Подготовить ленту: убрать tail-assistant и tool-result/tool_call если
    // есть, добавить пустой placeholder. Для регенерации режем всё после
    // последнего user-сообщения.
    ctx.messages.update(|m| {
        while m
            .last()
            .map(|x| x.role != ChatMsgRole::User)
            .unwrap_or(false)
        {
            m.pop();
        }
        if !m.iter().any(|x| x.role == ChatMsgRole::User) {
            return;
        }
        m.push(ChatMsg::assistant_empty());
    });
    // Если после удаления ни одного user нет, выходим.
    let msgs = ctx.messages.get_untracked();
    if !msgs.iter().any(|x| x.role == ChatMsgRole::User) {
        return;
    }

    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.error.set(None);
    ctx.pending.set(true);

    start_agent_thread(model, ctx);
}

/// Общая часть `send_message` / `regenerate_last`: snapshot всех нужных
/// signal-данных на main thread, спавн worker-thread и запуск agent-loop в
/// локальном tokio current_thread runtime.
fn start_agent_thread(model: Arc<LoadedSynModel>, ctx: SynChatCtx) {
    let app_ctx = use_context::<AppCtx>();

    // 2. Snapshot params + история + tool-схемы (всё на main thread!).
    let params = ctx.params.get_untracked();
    let system_prompt = ctx.system_prompt.get_untracked();
    let history: Vec<HistoryItem> = build_history(&ctx, &system_prompt);
    let caps = snapshot_media_caps(&app_ctx, &model);
    let tool_schemas: Vec<serde_json::Value> = collect_active_tool_schemas(&app_ctx);
    let abort_snapshot = ctx.abort.load(Ordering::Relaxed);
    let ctx_for_worker = ctx.clone();
    let abort = ctx.abort.clone();

    // 3. Worker — обычный std::thread, не tokio: synaptix CUDA блокирует.
    //    Внутри thread создаём локальный current-thread tokio runtime для
    //    `tools::execute` и `await_decision_on_tool_call` (оба `async`).
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("[syn_chat] не удалось создать tokio runtime: {e:#}");
                let ctx = ctx_for_worker.clone();
                run_on_main_thread(move || {
                    ctx.error.set(Some(format!("tokio runtime: {e:#}")));
                    ctx.commit_streaming_tail();
                    ctx.pending.set(false);
                });
                return;
            }
        };
        let result = rt.block_on(run_agent_loop(
            model,
            history,
            caps,
            tool_schemas,
            params,
            abort,
            abort_snapshot,
            ctx_for_worker.clone(),
        ));
        if let Err(e) = result {
            eprintln!("[syn_chat] agent-loop error: {e:#}");
            let ctx = ctx_for_worker.clone();
            run_on_main_thread(move || {
                ctx.error.set(Some(format!("{e:#}")));
            });
        }
        // Финализация (всегда, даже при abort/error).
        let ctx = ctx_for_worker;
        run_on_main_thread(move || {
            ctx.commit_streaming_tail();
            ctx.pending.set(false);
        });
    });
}

/// Debounced подсчёт токенов в `ctx.input`. Вызывается из `on_change` editor'а
/// после bump'а `ctx.input_tok_gen`. Worker спит 300 мс и проверяет, что
/// gen не изменился — иначе ответ устарел.
pub fn schedule_tokenize() {
    let ctx = use_context::<SynChatCtx>();
    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        return;
    };
    let gen = ctx.input_tok_gen.fetch_add(1, Ordering::Relaxed) + 1;
    let text = ctx.input.get_untracked();
    if text.is_empty() {
        ctx.input_tokens.set_always(0);
        return;
    }

    let ctx_clone = ctx.clone();
    let tok = model.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        if ctx_clone.input_tok_gen.load(Ordering::Relaxed) != gen {
            return; // устарел
        }
        match tok.tokenizer.encode(&text) {
            Ok(ids) => {
                let count = ids.len();
                let ctx2 = ctx_clone.clone();
                run_on_main_thread(move || {
                    if ctx2.input_tok_gen.load(Ordering::Relaxed) == gen {
                        ctx2.input_tokens.set_always(count);
                    }
                });
            }
            Err(e) => {
                eprintln!("[syn_chat] tokenize error: {e:#}");
            }
        }
    });
}

/// Разбор потока генерации: у моделей разный «протокол хода».
///
/// - [`Self::ChatML`] — Qwen3 и прочие: reasoning в `<think>…</think>`,
///   вызовы в `<tool_call>…</tool_call>`, всё внутри одного текста.
/// - [`Self::Channel`] — Muse Glimmer: ход состоит из нескольких сообщений
///   со своими адресатами (`to=self` / `to=user` / `to=<функция>`), а
///   разделители — спецтокены, невидимые в декодированном тексте
///   (см. [`crate::syn_chat::channel_parser`]).
enum StreamParser {
    ChatML { think: ThinkParser, tools: ToolCallParser },
    Channel(ChannelParser),
}

impl StreamParser {
    /// Канальный разбор — если словарь модели знает `<|start|>`/`<|message|>`.
    fn for_model(tokenizer: &LlmTokenizer, enable_thinking: bool) -> Self {
        match ChannelIds::detect(tokenizer) {
            Some(ids) => Self::Channel(ChannelParser::new(ids)),
            None => Self::ChatML {
                // Qwen3-VL / Qwen3-Thinking chat-template подаёт открывающий
                // `<think>` прямо в prompt — модель не пишет open-тег сама,
                // только закрывающий.
                think: if enable_thinking {
                    ThinkParser::new_implicit_open()
                } else {
                    ThinkParser::new()
                },
                tools: ToolCallParser::new(),
            },
        }
    }

    fn is_channel(&self) -> bool {
        matches!(self, Self::Channel(_))
    }

    /// Очередной токен → (текст ответа, текст размышлений).
    fn feed(&mut self, id: u32, delta: &str) -> (String, String) {
        match self {
            Self::ChatML { think, tools } => {
                let feed = tools.feed(delta);
                if feed.clean_delta.is_empty() {
                    return (String::new(), String::new());
                }
                let split = think.feed(&feed.clean_delta);
                (split.body, split.thinking)
            }
            Self::Channel(p) => {
                let split = p.feed(id, delta);
                (split.body, split.thinking)
            }
        }
    }

    /// Модель дописала tool-вызов — стрим можно рвать, не дожидаясь, пока
    /// она уйдёт писать прозу после блока.
    fn tool_call_ready(&self) -> bool {
        match self {
            Self::ChatML { tools, .. } => tools.calls_count() > 0 && tools.is_outside(),
            Self::Channel(p) => p.has_closed_call(),
        }
    }

    fn finish(self) -> Vec<RawToolCall> {
        match self {
            Self::ChatML { tools, .. } => tools.finish().0,
            Self::Channel(p) => p.finish(),
        }
    }
}

/// Главный цикл агента: prompt → generate → parse tool_calls → execute →
/// append history → next turn. Прерывается по abort, EOS-only ответу (no
/// tool_calls) или по достижении `MAX_AGENT_TURNS`.
async fn run_agent_loop(
    model: Arc<LoadedSynModel>,
    items: Vec<HistoryItem>,
    caps: MediaCaps,
    tool_schemas: Vec<serde_json::Value>,
    params: SamplingParams,
    abort: Arc<AtomicU64>,
    abort_snapshot: u64,
    ctx: SynChatCtx,
) -> anyhow::Result<()> {
    let model_cap = model.model.config().max_seq_len;
    let mut total_gen_tokens: u32 = 0;
    let t_overall = Instant::now();

    // Вложения кодируются один раз на весь agent-loop: тексты сообщений с
    // блоками-заполнителями и эмбеддинги дальше переиспользуются на каждом
    // turn'е. Vision-башня нужна только здесь — сразу после кодирования её
    // выгружаем, чтобы KV-ring получил свободную VRAM.
    let (mut history, media) = prepare_history(&items, &model, &caps);
    let media_refs: Vec<&MediaEmbedding> = media.iter().collect();
    if !media.is_empty() {
        let tokens: usize = media.iter().map(|m| m.tokens).sum();
        eprintln!(
            "[syn_chat] медиа-вложений: {} ({} vision-токенов)",
            media.len(),
            tokens
        );
    }

    for turn in 0..MAX_AGENT_TURNS {
        if abort.load(Ordering::Relaxed) != abort_snapshot {
            return Ok(());
        }

        let prompt = model.tokenizer.apply_chat_template_ex_tools(
            &history,
            true,
            params.enable_thinking,
            if tool_schemas.is_empty() {
                None
            } else {
                Some(&tool_schemas)
            },
        )?;
        let prompt_ids = model.tokenizer.encode(&prompt)?;
        eprintln!(
            "[syn_chat] turn={} prompt={} chars / {} tokens, tools={}",
            turn,
            prompt.len(),
            prompt_ids.len(),
            tool_schemas.len()
        );

        const KV_RESERVE_MB: usize = 2048;
        let prefill_start = Instant::now();
        let mut opts = params.to_options();
        let usable_cap = model_cap.saturating_sub(1);
        let prompt_capped = prompt_ids.len().min(usable_cap);
        let headroom = usable_cap.saturating_sub(prompt_capped);
        if opts.max_new_tokens > headroom {
            opts.max_new_tokens = headroom.max(1);
        }
        let realistic = prompt_capped + opts.max_new_tokens + 128;
        let vram_pre_kv = crate::syn_chat::model_registry::vram_free_mb();
        let kv_per_token = model.model.kv_bytes_per_token();
        let ring_by_mem = if kv_per_token > 0 {
            let budget = vram_pre_kv.saturating_sub(KV_RESERVE_MB) * 1024 * 1024;
            (budget / kv_per_token).max(prompt_capped + 256)
        } else {
            usable_cap
        };
        opts.max_seq_len = realistic.min(usable_cap).min(ring_by_mem);
        if opts.max_seq_len < prompt_capped + opts.max_new_tokens + 128 {
            opts.max_new_tokens = opts
                .max_seq_len
                .saturating_sub(prompt_capped + 128)
                .max(1);
        }
        let ring_len = opts.max_seq_len;
        let ring_max_new = opts.max_new_tokens;
        let mut runner = LlmGeneration::new(&model.model, opts);
        let vram_post_kv = crate::syn_chat::model_registry::vram_free_mb();
        eprintln!(
            "[syn_chat] KV-ring: max_seq_len={ring_len} (prompt={prompt_capped} + \
             max_new={ring_max_new} + 128, cap={usable_cap}, по памяти={ring_by_mem}, \
             {kv_per_token} B/ток); VRAM: KV {} MB, свободно {vram_post_kv} MB",
            vram_pre_kv.saturating_sub(vram_post_kv)
        );
        let channel_mode = ChannelIds::detect(&model.tokenizer).is_some();
        if channel_mode {
            // Канальный протокол завершает ход `<|eot|>`, а он уже в eos_ids
            // бандла — своих стопов добавлять не нужно (и `<|im_end|>` в
            // этом словаре всё равно нет).
            runner.set_stop_tokens(model.tokenizer.eos_ids().to_vec());
        } else {
            set_qwen3_stops(&mut runner, &model.tokenizer);
        }
        // На случай если парсер callback'а не успеет отработать перед
        // следующей итерацией — добавим явный text-level stop на закрытии
        // tool-call (encoder в LlmGeneration сравнивает накопленный
        // decoded-text).
        if !tool_schemas.is_empty() {
            runner.add_stop_sequence(if channel_mode { ATEM_CLOSE } else { TOOL_CALL_CLOSE });
        }

        // Только для первого turn'а отчитываем prefill_ms; в следующих
        // итерациях время уйдёт почти полностью в prefill заново
        // построенного prompt'а с tool-result'ами.
        let prefill_ms = prefill_start.elapsed().as_millis() as u32;
        if turn == 0 {
            let prompt_tokens = prompt_ids.len() as u32;
            let ctx_stat = ctx.clone();
            run_on_main_thread(move || {
                ctx_stat.last_prompt_tokens.set_always(prompt_tokens);
                ctx_stat.last_prefill_ms.set_always(prefill_ms);
            });
        }

        // Разбор потока — по протоколу модели (ChatML или канальный).
        let mut parser = StreamParser::for_model(&model.tokenizer, params.enable_thinking);
        // Полный текст ответа модели за этот turn — нужен для канонического
        // assistant-msg в history (с `<tool_call>` тегами как-есть). В
        // канальном режиме сырой текст непригоден: в нём заголовки каналов
        // (` to=user`), поэтому там историю собираем из разобранного тела.
        let mut raw_text: String = String::new();
        let mut clean_text: String = String::new();
        let mut buf_body = String::new();
        let mut buf_think = String::new();
        let mut last_flush = Instant::now();
        let flush_interval = Duration::from_millis(FLUSH_INTERVAL_MS);
        let mut tokens_this_turn: u32 = 0;

        let t_turn = Instant::now();
        let abort_for_cb = abort.clone();
        let ctx_for_cb = ctx.clone();
        let on_token = |id: u32, delta: &str| {
            // Abort: сбросить накопленные буферы и выйти.
            if abort_for_cb.load(Ordering::Relaxed) != abort_snapshot {
                flush_streaming(&ctx_for_cb, &mut buf_body, &mut buf_think, None);
                return false;
            }
            raw_text.push_str(delta);
            tokens_this_turn += 1;

            let (body, thinking) = parser.feed(id, delta);
            clean_text.push_str(&body);
            buf_body.push_str(&body);
            buf_think.push_str(&thinking);

            // Throttled flush в UI.
            let now = Instant::now();
            if now.duration_since(last_flush) >= flush_interval {
                last_flush = now;
                flush_streaming(
                    &ctx_for_cb,
                    &mut buf_body,
                    &mut buf_think,
                    Some(tokens_this_turn),
                );
            }

            // Если парсер уже зафиксировал tool_call — останавливаемся,
            // не дожидаясь, пока модель уйдёт писать прозу после блока.
            // (Stop-sequence на закрытии блока дублирует эту защиту, но
            // text-level stop срабатывает только когда decoded-tail совпадает,
            // что зависит от tokenizer-decode таймингов.)
            if parser.tool_call_ready() {
                return false;
            }
            true
        };

        // Медиа-промпт идёт своим путём: prefill по готовым эмбеддингам
        // вместо embed'а id-токенов. Спекулятивные декодеры (DFlash /
        // lookup / CUDA-graph) на нём не применяются.
        if media_refs.is_empty() {
            runner.generate_streaming(&prompt_ids, &model.tokenizer, on_token)?;
        } else {
            runner.generate_streaming_media(
                &prompt_ids,
                &model.tokenizer,
                &media_refs,
                on_token,
            )?;
        }

        // Финальный flush — гарантированно сбрасываем хвост буферов.
        flush_streaming(&ctx, &mut buf_body, &mut buf_think, Some(tokens_this_turn));

        let channel_mode = parser.is_channel();
        let raw_calls = parser.finish();

        // Явно освобождаем runner + trim mempool. cudaMallocAsync держит
        // освобождённые chunks в pool с release threshold ≥ 2 GB по
        // умолчанию — после Drop KV-ring (4 GB на 32K) память остаётся в
        // pool и не возвращается ОС, что вызывает OOM на следующем turn'е
        // при alloc нового ring. Trim форсит возврат всей свободной
        // памяти ОС → следующая итерация стартует с полным free VRAM.
        drop(runner);
        if let synaptix_core::device::Device::Cuda(ordinal) = model.model.device() {
            let freed = synaptix::facade::llm::cuda_trim_pool(*ordinal as i32);
            eprintln!("[syn_chat] trim mempool: +{freed} MB free");
        }
        let dt = t_turn.elapsed();
        let tok_per_s = if dt.as_secs_f64() > 0.0 {
            tokens_this_turn as f64 / dt.as_secs_f64()
        } else {
            0.0
        };
        eprintln!(
            "[syn_chat] turn={} {} tokens in {:?} ({:.1} tok/s), tool_calls={}",
            turn,
            tokens_this_turn,
            dt,
            tok_per_s,
            raw_calls.len()
        );
        total_gen_tokens += tokens_this_turn;

        // Если abort за стримом — выходим.
        if abort.load(Ordering::Relaxed) != abort_snapshot {
            return Ok(());
        }

        if raw_calls.is_empty() {
            // Обычный текстовый ответ. commit_streaming_tail сделает
            // финализацию в send_message wrapper'е.
            break;
        }

        // Tool-calls: коммитим накопленный текст в leading-bubble, дальше
        // создаём отдельные tool_call/tool_result bubble'ы.
        let ctx_commit = ctx.clone();
        run_on_main_thread(move || ctx_commit.commit_streaming_tail());

        // History: assistant с полным сырым текстом (включая `<tool_call>`).
        // В канальном режиме сырой текст содержит заголовки каналов, которые
        // chat-шаблон припишет заново, — поэтому пересобираем реплику из
        // разобранного тела и ATEM-блока вызовов.
        history.push(Message::assistant(if channel_mode {
            channel_parser::rebuild_turn_text(&clean_text, &raw_calls)
        } else {
            raw_text.clone()
        }));

        // Конвертируем RawToolCall → ChatToolCall (для UI и executor'а).
        let chat_calls: Vec<ChatToolCall> = raw_calls
            .iter()
            .enumerate()
            .map(|(i, c)| ChatToolCall {
                id: format!("syn_call_{}_{}_{}", turn, i, abort_snapshot),
                kind: "function".to_string(),
                function: ChatToolCallFunction {
                    name: Some(c.name.clone()),
                    arguments: Some(c.arguments_json.clone()),
                },
            })
            .collect();

        // Один общий tool_call-bubble на UI (как у llama-чата).
        let calls_for_ui = chat_calls.clone();
        let name_for_ui = chat_calls
            .first()
            .and_then(|c| c.function.name.clone())
            .unwrap_or_default();
        let args_pretty = tools::pretty_args(
            chat_calls
                .first()
                .and_then(|c| c.function.arguments.as_deref()),
        );
        let ctx_call = ctx.clone();
        run_on_main_thread(move || {
            ctx_call.messages.update(|m| {
                // Удаляем хвост пустого assistant-placeholder'а (создан в
                // send_message); вместо него ставим tool_call bubble.
                if m.last()
                    .map(|x| x.role == ChatMsgRole::Assistant && x.body.is_empty())
                    .unwrap_or(false)
                {
                    m.pop();
                }
                m.push(ChatMsg::tool_call(name_for_ui, args_pretty, calls_for_ui));
            });
        });

        // Выполняем каждый tool: confirm → execute → push result.
        for chat_call in chat_calls.iter() {
            if abort.load(Ordering::Relaxed) != abort_snapshot {
                return Ok(());
            }
            let decision = crate::agent::tool_flow::await_decision_on_tool_call(
                chat_call,
                &abort,
                abort_snapshot,
            )
            .await;
            match decision {
                ToolDecision::Cancel => {
                    push_tool_result(
                        &ctx,
                        chat_call,
                        "Отменено пользователем".to_string(),
                        true,
                    );
                    history.push(Message::tool_named(
                        tool_name(chat_call),
                        "Отменено пользователем",
                    ));
                    // После Cancel прерываем весь loop — пользователь явно
                    // отказал, нет смысла продолжать.
                    return Ok(());
                }
                ToolDecision::AllowAll => {
                    run_on_main_thread(|| {
                        use_context::<AppCtx>().tools.allow_all.set_always(true);
                    });
                    // Падаем в Allow-ветку.
                }
                ToolDecision::Allow => {}
            }

            // Исполнение с возможностью прерывания на длинных tool'ах
            // (web fetch может висеть 30+ сек).
            let outcome = tokio::select! {
                o = tools::execute(chat_call) => o,
                _ = wait_abort(&abort, abort_snapshot) => return Ok(()),
            };

            push_tool_result(&ctx, chat_call, outcome.content.clone(), outcome.error);
            history.push(Message::tool_named(tool_name(chat_call), outcome.content));
        }

        // Готовим placeholder для следующего turn (UI bubble — пустой
        // assistant, который заполнится stream'ом).
        let ctx_ph = ctx.clone();
        run_on_main_thread(move || {
            ctx_ph.messages.update(|m| {
                m.push(ChatMsg::assistant_empty());
            });
            ctx_ph.streaming_body.set(String::new());
            ctx_ph.streaming_thinking.set(String::new());
        });
    }

    // Финальная статистика.
    let dt_total = t_overall.elapsed();
    let final_tps = if dt_total.as_secs_f64() > 0.0 {
        (total_gen_tokens as f64 / dt_total.as_secs_f64()) as f32
    } else {
        0.0
    };
    let final_gen = total_gen_tokens;
    let ctx_final = ctx.clone();
    run_on_main_thread(move || {
        ctx_final.last_gen_tokens.set_always(final_gen);
        ctx_final.last_decode_tps.set_always(final_tps);
    });

    Ok(())
}

/// Сбрасывает накопленный body/think буфер в реактивные сигналы UI.
/// Передавать `tokens_emitted = None` если статистику обновлять не надо
/// (например, на abort-сбросе).
fn flush_streaming(
    ctx: &SynChatCtx,
    buf_body: &mut String,
    buf_think: &mut String,
    tokens_emitted: Option<u32>,
) {
    if buf_body.is_empty() && buf_think.is_empty() && tokens_emitted.is_none() {
        return;
    }
    let b = std::mem::take(buf_body);
    let t = std::mem::take(buf_think);
    let ctx = ctx.clone();
    run_on_main_thread(move || {
        if !t.is_empty() {
            ctx.streaming_thinking.update(|s| s.push_str(&t));
        }
        if !b.is_empty() {
            ctx.streaming_body.update(|s| s.push_str(&b));
        }
        if let Some(n) = tokens_emitted {
            ctx.last_gen_tokens.set_always(n);
        }
    });
}

/// Поллер «дождаться abort» — оборачивается в `tokio::select!` чтобы прервать
/// долгие async-операции (например `tools::execute` для web-fetch).
async fn wait_abort(abort: &Arc<AtomicU64>, snapshot: u64) {
    loop {
        if abort.load(Ordering::Relaxed) != snapshot {
            return;
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

/// Имя вызванной функции — им подписывается блок результата в prompt'е.
fn tool_name(call: &ChatToolCall) -> String {
    call.function.name.clone().unwrap_or_default()
}

/// Пушит tool_result-бабл в ленту.
fn push_tool_result(ctx: &SynChatCtx, call: &ChatToolCall, content: String, error: bool) {
    let id = call.id.clone();
    let name = call.function.name.clone().unwrap_or_default();
    let ctx = ctx.clone();
    run_on_main_thread(move || {
        ctx.messages.update(|m| {
            m.push(ChatMsg::tool_result(id, name, content, error));
        });
    });
}

/// Собирает JSON-схемы активных инструментов (для передачи в Jinja-шаблон
/// Qwen3 или manual prefix). Особый случай — `autoskill`, описание которого
/// расширяется актуальным списком скилов через [`build_autoskill_chat_tool`].
fn collect_active_tool_schemas(app: &AppCtx) -> Vec<serde_json::Value> {
    use crate::agent::tools::catalog::KEY_AUTOSKILL;
    let keys = app.tools.active.get_untracked();
    keys.iter()
        .filter_map(|k| {
            let tool_json = if k == KEY_AUTOSKILL {
                let t = crate::agent::tool_flow::build_autoskill_chat_tool(app);
                serde_json::to_value(&t).ok()
            } else {
                let t = Tool::by_key(k)?.to_chat_tool();
                serde_json::to_value(&t).ok()
            }?;
            Some(tool_json)
        })
        .collect()
}

/// Полный набор stop-токенов для Qwen3 ChatML: EOS из конфига + `<|im_end|>`.
pub(crate) fn set_qwen3_stops(runner: &mut LlmGeneration<'_>, tokenizer: &LlmTokenizer) {
    let mut stops: Vec<u32> = tokenizer.eos_ids().to_vec();
    match tokenizer.encode(IM_END_TOKEN) {
        Ok(ids) if ids.len() == 1 => {
            if !stops.contains(&ids[0]) {
                stops.push(ids[0]);
            }
        }
        Ok(ids) => eprintln!("[syn_chat] неожиданный encode(<|im_end|>) → {ids:?}"),
        Err(e) => eprintln!("[syn_chat] не удалось encode(<|im_end|>): {e:#}"),
    }
    runner.set_stop_tokens(stops);
}

/// Реплика ленты в форме, пригодной для сборки промпта на worker-потоке:
/// сигналы уже прочитаны, вложения — по значению.
struct HistoryItem {
    role: ChatMsgRole,
    body: String,
    attachments: Vec<MsgAttachment>,
}

/// Снимок возможностей модели и окружения по части вложений. Читает
/// сигналы, поэтому вызывается только с main thread.
fn snapshot_media_caps(app: &AppCtx, model: &Arc<LoadedSynModel>) -> MediaCaps {
    let max = app.syn_chat_max_image_tokens.get_untracked();
    MediaCaps {
        vision: model.model.supports_media(),
        max_image_tokens: (max > 0).then_some(max),
        asr: Some(app.audio.asr.clone()),
    }
}

/// Строит prompt-историю для chat-template. Пустой плейсхолдер ассистента
/// в конце ленты исключается (он добавлен в `send_message` только для UI).
/// При непустом `system_prompt` префиксует историю system-сообщением.
/// Tool-call и tool-result сообщения, накопленные в `ctx.messages` в
/// предыдущих сессиях, ПОКА не переэкспортируются в prompt — это требует
/// рекувырки структуры. Сейчас в `regenerate_last` мы режем всё после
/// последнего user, так что несовпадения не возникает.
fn build_history(ctx: &SynChatCtx, system_prompt: &str) -> Vec<HistoryItem> {
    let msgs = ctx.messages.get_untracked();
    let mut out: Vec<HistoryItem> = Vec::with_capacity(msgs.len() + 1);
    if !system_prompt.trim().is_empty() {
        out.push(HistoryItem {
            role: ChatMsgRole::System,
            body: system_prompt.to_string(),
            attachments: Vec::new(),
        });
    }
    for (i, m) in msgs.iter().enumerate() {
        // Skip последний assistant-плейсхолдер с пустым body.
        let is_last = i + 1 == msgs.len();
        if is_last && m.role == ChatMsgRole::Assistant && m.body.is_empty() {
            continue;
        }
        // Свернутые autocompact-сообщения не идут в prompt.
        if m.compacted_iter.is_some() {
            continue;
        }
        out.push(HistoryItem {
            role: m.role,
            body: m.body.clone(),
            attachments: m.attachments.clone(),
        });
    }
    out
}

/// Раскрывает вложения в промпт: картинки и видео — в vision-эмбеддинги
/// плюс блок токенов-заполнителей, документы и аудио — в текст.
///
/// Vision-башня грузится один раз на весь вызов и сразу выгружается: её
/// VRAM нужна KV-рингу, а эмбеддинги вложений живут отдельными тензорами
/// и переживают выгрузку.
fn prepare_history(
    items: &[HistoryItem],
    model: &Arc<LoadedSynModel>,
    caps: &MediaCaps,
) -> (Vec<Message>, Vec<MediaEmbedding>) {
    let needs_vision = caps.vision
        && items.iter().any(|i| {
            i.attachments
                .iter()
                .any(|a| a.kind.has_thumbnail())
        });
    let vision_ready = attach_prompt::ensure_tower(&model.model, needs_vision);
    let caps = MediaCaps { vision: vision_ready, ..caps.clone() };

    let mut out: Vec<Message> = Vec::with_capacity(items.len());
    let mut media: Vec<MediaEmbedding> = Vec::new();
    for item in items {
        match item.role {
            ChatMsgRole::User if !item.attachments.is_empty() => {
                let prepared = attach_prompt::prepare_user_message(
                    &item.body,
                    &item.attachments,
                    &model.model,
                    &caps,
                );
                media.extend(prepared.media);
                out.push(Message::user(prepared.text));
            }
            ChatMsgRole::User => out.push(Message::user(&item.body)),
            ChatMsgRole::Assistant => out.push(Message::assistant(&item.body)),
            ChatMsgRole::System => out.push(Message::system(&item.body)),
        }
    }
    if vision_ready {
        model.model.release_media_tower();
        if let synaptix_core::device::Device::Cuda(ordinal) = model.model.device() {
            let freed = synaptix::facade::llm::cuda_trim_pool(*ordinal as i32);
            eprintln!("[syn_chat] vision-башня выгружена, trim: +{freed} MB");
        }
    }
    (out, media)
}

