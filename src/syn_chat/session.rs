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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synaptix::facade::llm::{
    LlmGeneration, LlmKvSession, LlmTokenizer, MediaEmbedding, Message,
};

use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
use crate::agent::tools::{self, Tool, ToolDecision};
use crate::context::AppCtx;
use crate::syn_chat::attach::prompt::{self as attach_prompt, MediaCaps};
use crate::syn_chat::channel_parser::{self, ChannelIds, ChannelParser, ATEM_CLOSE};
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{
    ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment, SynChatCtx, ThinkParser,
};
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

/// Шаг ёмкости кэша префикс-KV. Сессия живёт между ходами, а пересоздание
/// стирает префикс — значит расти надо редко и с запасом, а не «в притык» под
/// каждый ход. 16384 токена ≈ 1 ГБ при F16-KV гибрида 27B и ≈0,5 ГБ при MXFP8.
const SESSION_CTX_STEP: usize = 16384;

/// Слот префикс-KV: посчитанный контекст диалога живёт между ходами и между
/// сообщениями, пока это тот же чат на той же модели.
///
/// Ход дописывает в кэш только новый хвост промпта, поэтому agent-loop с
/// инструментами больше не префиллит историю заново на каждом вызове (а это
/// было до 16 полных префиллов на одно сообщение пользователя).
struct KvSlot {
    session: LlmKvSession,
    model: std::path::PathBuf,
    chat: Option<String>,
}

static KV_SLOT: std::sync::Mutex<Option<KvSlot>> = std::sync::Mutex::new(None);

/// Забыть посчитанный контекст (смена/выгрузка модели, переключение чата,
/// освобождение VRAM под vision-башню).
///
/// Зовётся с main thread, поэтому блокироваться нельзя: слот держит worker на
/// всё время хода, а ждать его — это подвесить окно на минуты. Если занято,
/// освобождение уходит в отдельный поток и случится сразу по окончании хода.
pub fn drop_kv_session() {
    match KV_SLOT.try_lock() {
        Ok(mut g) => {
            if g.is_some() {
                log::info!("[syn_chat] префикс-KV: кэш диалога освобождён");
            }
            *g = None;
        }
        Err(std::sync::TryLockError::Poisoned(e)) => {
            *e.into_inner() = None;
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            // Занят генерацией: ждать здесь нельзя, поэтому освобождаем в
            // отдельном потоке — он проснётся, как только ход закончится.
            std::thread::spawn(|| {
                let mut g = KV_SLOT.lock().unwrap_or_else(|e| e.into_inner());
                *g = None;
                log::info!("[syn_chat] префикс-KV: кэш диалога освобождён после хода");
            });
        }
    }
}

/// Взять сессию под этот чат/модель, создав или пересоздав при необходимости.
/// `None` — префикс-KV недоступен (архитектура или нехватка VRAM), вызывающий
/// работает как раньше.
fn ensure_kv_slot<'a>(
    slot: &'a mut Option<KvSlot>,
    model: &LoadedSynModel,
    chat: &Option<String>,
    want_ctx: usize,
    max_new: usize,
) -> Option<&'a mut LlmKvSession> {
    let fits = slot.as_ref().is_some_and(|s| {
        s.model == model.path && &s.chat == chat && s.session.ctx_tokens() >= want_ctx
    });
    if !fits {
        if let Some(old) = slot.take() {
            log::info!(
                "[syn_chat] префикс-KV: пересоздаём кэш (было {} ток, нужно ≥{want_ctx})",
                old.session.ctx_tokens()
            );
            drop(old);
            // Старый ринг должен вернуться в пул до аллокации нового, иначе на
            // границе роста в VRAM живут оба.
            crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
        }
        match model.model.new_kv_session(want_ctx, max_new) {
            Ok(Some(session)) => {
                log::info!(
                    "[syn_chat] префикс-KV: кэш на {} ток ({} MB)",
                    session.ctx_tokens(),
                    (session.ctx_tokens() as u64 * model.model.kv_bytes_per_token() as u64)
                        / (1024 * 1024)
                );
                *slot = Some(KvSlot {
                    session,
                    model: model.path.clone(),
                    chat: chat.clone(),
                });
            }
            Ok(None) => {
                log::info!(
                    "[syn_chat] префикс-KV недоступен для этой модели — префилл как раньше"
                );
                return None;
            }
            Err(e) => {
                log::warn!("[syn_chat] префикс-KV: кэш не создан ({e}) — префилл как раньше");
                return None;
            }
        }
    }
    slot.as_mut().map(|s| &mut s.session)
}

/// Запас VRAM, который планировщик ринга НЕ отдаёт под KV, пока кэши ядер не
/// прогреты.
///
/// На первом forward'е модель лениво собирает их сама: у nvfp4 это
/// shuffled-копии весов, ≈1.1 ГБ. Плюс активации префилла — ≈0.15…0.8 ГБ при
/// чанке 256 (от длины промпта они не зависят: пул активаций в synaptix
/// отделён от пула весов и выходит на плато с первого чанка).
const KV_RESERVE_COLD_MB: usize = 3072;
/// То же после первой удачной генерации: кэши ядер уже стоят, и держать под них
/// запас — значит отнимать у контекста десятки тысяч токенов (при MXFP8-KV
/// гигабайт запаса ≈ 30k токенов).
const KV_RESERVE_WARM_MB: usize = 1280;

/// Прогреты ли ленивые кэши ядер текущей модели (см. [`KV_RESERVE_COLD_MB`]).
static KERNEL_CACHES_WARM: AtomicBool = AtomicBool::new(false);

fn kv_reserve_mb() -> usize {
    if KERNEL_CACHES_WARM.load(Ordering::Relaxed) {
        KV_RESERVE_WARM_MB
    } else {
        KV_RESERVE_COLD_MB
    }
}

/// Сбросить признак прогретости — при загрузке/выгрузке модели кэши ядер
/// собираются заново (`release_device_caches` их и чистит).
pub fn reset_kernel_cache_warm() {
    KERNEL_CACHES_WARM.store(false, Ordering::Relaxed);
}
/// На сколько токенов ответа планируется ринг, если слайдер `max_new_tokens`
/// стоит выше. Ринг живёт ровно один ход; планировать его на 130k «про запас»
/// значит впустую занять гигабайты (у гибрида 27B это 64 КБ на токен). 8k
/// токенов ответа — это ~6k слов, для чата с запасом.
pub(crate) const RING_ANSWER_TOKENS: usize = 8192;
/// Гранулярность длины ринга. Промпт растёт от хода к ходу, и ринг «в притык»
/// каждый раз просил бы блоки чуть большего размера — освободившиеся от
/// прошлого ринга пул отдать под них не может. Кратность 4096 делает ринг
/// одинаковым на серии ходов, и блоки переиспользуются.
const RING_GRANULARITY: usize = 4096;
/// Сколько раз пересобирать ход с меньшим рингом после OOM.
pub(crate) const MAX_OOM_RETRIES: usize = 3;
/// Бюджет ответа, ниже которого ретраить уже нечем.
pub(crate) const MIN_ANSWER_TOKENS: usize = 512;

/// Расчёт KV-ринга на один ход: сколько токенов сажаем в кэш и сколько из них
/// остаётся под ответ.
pub(crate) struct RingPlan {
    pub(crate) prompt_tokens: usize,
    pub(crate) ring_tokens: usize,
    /// Ёмкость кэша префикс-KV (крупный шаг — см. [`SESSION_CTX_STEP`]).
    pub(crate) session_ctx: usize,
    pub(crate) max_new: usize,
    /// Сколько токенов контекста вообще влезает в свободную VRAM.
    pub(crate) by_mem: usize,
    pub(crate) cap: usize,
    pub(crate) kv_per_token: usize,
    pub(crate) vram_available_mb: usize,
    /// Промпт не влезает в ринг — движок обрежет контекст.
    pub(crate) truncated_prompt: bool,
}

impl RingPlan {
    pub(crate) fn new(
        model: &LoadedSynModel,
        prompt_tokens: usize,
        answer_tokens: usize,
        model_cap: usize,
    ) -> Self {
        Self::with_session(model, prompt_tokens, answer_tokens, model_cap, 0)
    }

    /// Как [`Self::new`], но с поправкой на VRAM, которую держит кэш
    /// префикс-KV: он освобождается ДО аллокации нового, поэтому в бюджет
    /// входит.
    pub(crate) fn with_session(
        model: &LoadedSynModel,
        prompt_tokens: usize,
        answer_tokens: usize,
        model_cap: usize,
        session_held_mb: usize,
    ) -> Self {
        let cap = model_cap.saturating_sub(1);
        let kv_per_token = model.model.kv_bytes_per_token();
        // Кэш префикс-KV держит VRAM прямо сейчас, но при пересоздании
        // освобождается ДО новой аллокации — иначе бюджет занижался бы ровно на
        // размер уже живущего кэша и контекст перестал бы расти.
        let vram_available_mb =
            crate::syn_chat::model_registry::vram_available_mb() + session_held_mb;
        let by_mem = if kv_per_token > 0 {
            let budget = vram_available_mb.saturating_sub(kv_reserve_mb()) * 1024 * 1024;
            budget / kv_per_token
        } else {
            cap
        };
        // Даже когда бюджет ушёл в ноль (оценка пессимистична сразу после
        // загрузки, пока пулы не прогрелись), пробуем посадить хотя бы промпт
        // с коротким ответом: не выйдет — ретрай по OOM отработает честно,
        // а гарантированно провальный ринг на 1024 токена не помогает никому.
        let floor = prompt_tokens.min(cap) + MIN_ANSWER_TOKENS;
        let hard_cap = cap.min(by_mem.max(floor));
        let want = prompt_tokens.min(cap) + answer_tokens + 128;
        let ring_tokens = want.div_ceil(RING_GRANULARITY) * RING_GRANULARITY;
        let ring_tokens = ring_tokens.min(hard_cap).max(1);
        let session_ctx = want
            .div_ceil(SESSION_CTX_STEP)
            .max(1)
            .saturating_mul(SESSION_CTX_STEP)
            .min(hard_cap)
            .max(ring_tokens);
        let prompt_capped = prompt_tokens.min(ring_tokens.saturating_sub(1));
        let max_new = ring_tokens.saturating_sub(prompt_capped + 128).max(1);
        Self {
            prompt_tokens,
            ring_tokens,
            session_ctx,
            max_new,
            by_mem,
            cap,
            kv_per_token,
            vram_available_mb,
            truncated_prompt: prompt_tokens + 1 > ring_tokens,
        }
    }

    pub(crate) fn ring_bytes(&self) -> u64 {
        (self.ring_tokens as u64) * (self.kv_per_token as u64)
    }

    pub(crate) fn ring_mb(&self) -> u64 {
        self.ring_bytes() / (1024 * 1024)
    }
}

/// Метрики хода для таба «Детали». Собираются в worker-потоке и одним
/// пакетом переливаются в сигналы на main thread.
struct TurnStats {
    prompt_tokens: u32,
    /// Сколько токенов промпта не пришлось считать заново (префикс-KV).
    reused_tokens: u32,
    gen_tokens: u32,
    prefill_ms: u32,
    turns: u32,
    ring_tokens: u32,
    ring_bytes: u64,
    ctx_budget: u32,
    vram_free_mb: u32,
}

impl TurnStats {
    fn apply(&self, ctx: &SynChatCtx) {
        ctx.last_prompt_tokens.set_always(self.prompt_tokens);
        ctx.last_reused_tokens.set_always(self.reused_tokens);
        ctx.last_gen_tokens.set_always(self.gen_tokens);
        ctx.last_prefill_ms.set_always(self.prefill_ms);
        ctx.last_turns.set_always(self.turns);
        ctx.last_ring_tokens.set_always(self.ring_tokens);
        ctx.kv_cache_bytes.set_always(self.ring_bytes);
        ctx.ctx_budget_tokens.set_always(self.ctx_budget);
        ctx.last_vram_free_mb.set_always(self.vram_free_mb);
    }
}

/// Ошибка — это исчерпание VRAM? Драйвер отдаёт `CUDA_ERROR_OUT_OF_MEMORY`,
/// наши аллокаторы добавляют свои формулировки («after trim+retries: OOM»).
pub(crate) fn is_oom_error<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("OUT_OF_MEMORY") || s.contains("out of memory") || s.contains(": OOM")
}

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
    let chat_id = ctx.active_chat_id.get_untracked();
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
            chat_id,
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
#[allow(clippy::too_many_arguments)]
async fn run_agent_loop(
    model: Arc<LoadedSynModel>,
    items: Vec<HistoryItem>,
    caps: MediaCaps,
    tool_schemas: Vec<serde_json::Value>,
    params: SamplingParams,
    abort: Arc<AtomicU64>,
    abort_snapshot: u64,
    chat_id: Option<String>,
    ctx: SynChatCtx,
) -> anyhow::Result<()> {
    let model_cap = model.model.config().max_seq_len;
    let mut total_gen_tokens: u32 = 0;
    let t_overall = Instant::now();

    // Вложения кодируются один раз на весь agent-loop: тексты сообщений с
    // блоками-заполнителями и эмбеддинги дальше переиспользуются на каждом
    // turn'е. Vision-башня нужна только здесь — сразу после кодирования её
    // выгружаем, чтобы KV-ring получил свободную VRAM.
    // Медиа-путь идёт по готовым эмбеддингам и префикс-KV не поддерживает, а
    // vision-башне нужна та же VRAM, что держит кэш диалога, — освобождаем ДО
    // кодирования вложений.
    if items.iter().any(HistoryItem::has_media) {
        drop_kv_session();
    }
    let (mut history, media) = prepare_history(&items, &model, &caps, &ctx);
    let media_refs: Vec<&MediaEmbedding> = media.iter().collect();
    if !media.is_empty() {
        let tokens: usize = media.iter().map(|m| m.tokens).sum();
        log::info!(
            "[syn_chat] медиа-вложений: {} ({} vision-токенов)",
            media.len(),
            tokens
        );
    }

    // Слот держим на весь agent-loop: ходы внутри одного сообщения — главные
    // потребители префикса (каждый tool-вызов раньше требовал полного
    // префилла выросшей истории).
    let mut kv_slot = KV_SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let prefix_kv_on = media.is_empty() && crate::config::AppConfig::load().syn_chat_prefix_kv;
    if !prefix_kv_on {
        *kv_slot = None;
    }
    let mut reused_total: u32 = 0;

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
        log::info!(
            "[syn_chat] turn={} prompt={} chars / {} tokens, tools={}",
            turn,
            prompt.len(),
            prompt_ids.len(),
            tool_schemas.len()
        );

        // ── План KV-ринга и запуск с ретраем по OOM.
        let mut answer_budget = (params.max_new_tokens as usize).min(RING_ANSWER_TOKENS);
        let mut oom_attempt = 0usize;
        let turn_result = loop {
            let session_held_mb = kv_slot
                .as_ref()
                .map(|s| {
                    (s.session.ctx_tokens() * model.model.kv_bytes_per_token()) / (1024 * 1024)
                })
                .unwrap_or(0);
            let plan = RingPlan::with_session(
                &model,
                prompt_ids.len(),
                answer_budget,
                model_cap,
                session_held_mb,
            );
            let mut opts = params.to_options();
            opts.max_seq_len = plan.ring_tokens;
            opts.max_new_tokens = plan.max_new;
            log::info!(
                "[syn_chat] KV-ринг: {} ток ({} MB) = промпт {} + ответ {} + 128; \
                 по VRAM влезает {} ток, cap модели {}, {} B/ток; VRAM доступно {} MB \
                 (свободно по драйверу {} MB)",
                plan.ring_tokens,
                plan.ring_mb(),
                plan.prompt_tokens,
                plan.max_new,
                plan.by_mem,
                plan.cap,
                plan.kv_per_token,
                plan.vram_available_mb,
                crate::syn_chat::model_registry::vram_free_mb()
            );
            if plan.truncated_prompt {
                log::warn!(
                    "[syn_chat] промпт {} ток не влезает в ринг {} — история будет \
                     обрезана движком; сожмите чат или уменьшите max_new_tokens",
                    prompt_ids.len(),
                    plan.ring_tokens
                );
            }
            let mut runner = LlmGeneration::new(&model.model, opts);
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
            // Честный prefill: время до ПЕРВОГО токена. Прежний замер стоял до
            // старта генерации и показывал в панели 0 — время планирования ринга.
            let mut ttft_ms: Option<u32> = None;

            let t_turn = Instant::now();
            let abort_for_cb = abort.clone();
            let ctx_for_cb = ctx.clone();
            let on_token = |id: u32, delta: &str| {
                // Abort: сбросить накопленные буферы и выйти.
                if abort_for_cb.load(Ordering::Relaxed) != abort_snapshot {
                    flush_streaming(&ctx_for_cb, &mut buf_body, &mut buf_think, None);
                    return false;
                }
                if ttft_ms.is_none() {
                    ttft_ms = Some(t_turn.elapsed().as_millis() as u32);
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
            let mut reused = 0usize;
            let stream_res = if !media_refs.is_empty() {
                runner.generate_streaming_media(
                    &prompt_ids,
                    &model.tokenizer,
                    &media_refs,
                    on_token,
                )
            } else {
                let session = if prefix_kv_on {
                    ensure_kv_slot(
                        &mut kv_slot,
                        &model,
                        &chat_id,
                        plan.session_ctx,
                        plan.max_new,
                    )
                } else {
                    None
                };
                match session {
                    Some(session) => runner
                        .generate_streaming_cached(
                            session,
                            &prompt_ids,
                            &model.tokenizer,
                            on_token,
                        )
                        .map(|n| reused = n),
                    None => runner.generate_streaming(&prompt_ids, &model.tokenizer, on_token),
                }
            };

            // Финальный flush — гарантированно сбрасываем хвост буферов.
            flush_streaming(&ctx, &mut buf_body, &mut buf_think, Some(tokens_this_turn));

            let channel_mode = parser.is_channel();

            // Явно освобождаем runner + возвращаем VRAM. cudaMallocAsync держит
            // освобождённые chunks в pool с release threshold ≥ 2 GB по
            // умолчанию — после Drop KV-ring (4 GB на 32K) память остаётся в
            // pool и не возвращается ОС, что вызывает OOM на следующем turn'е
            // при alloc нового ring. Одного трима мало: сегменты пула держат
            // мёртвые записи кэша TMA-дескрипторов (ключ — адрес тензора), и
            // за ход так утекает больше гигабайта — см. `reclaim_vram`.
            drop(runner);
            let (freed, descs) =
                crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
            let vram_after = crate::syn_chat::model_registry::vram_available_mb();
            log::info!(
                "[syn_chat] после хода: trim +{freed} MB ({descs} TMA-деск.), VRAM \
                 доступно {vram_after} MB (свободно по драйверу {} MB)",
                crate::syn_chat::model_registry::vram_free_mb()
            );

            if let Err(e) = stream_res {
                let oom = is_oom_error(&e);
                let retryable = oom
                    && tokens_this_turn == 0
                    && oom_attempt < MAX_OOM_RETRIES
                    && answer_budget > MIN_ANSWER_TOKENS;
                if retryable {
                    oom_attempt += 1;
                    answer_budget = (answer_budget / 2).max(MIN_ANSWER_TOKENS);
                    log::warn!(
                        "[syn_chat] OOM на ринге {} ток ({} MB): {e}. Повтор {}/{} \
                         с бюджетом ответа {} ток",
                        plan.ring_tokens,
                        plan.ring_mb(),
                        MAX_OOM_RETRIES,
                        oom_attempt,
                        answer_budget
                    );
                    let ctx_clear = ctx.clone();
                    run_on_main_thread(move || {
                        ctx_clear.streaming_body.set(String::new());
                        ctx_clear.streaming_thinking.set(String::new());
                    });
                    continue;
                }
                log::error!("[syn_chat] генерация оборвалась: {e:#}");
                return Err(e.into());
            }

            KERNEL_CACHES_WARM.store(true, Ordering::Relaxed);
            let dt = t_turn.elapsed();
            let tok_per_s = if dt.as_secs_f64() > 0.0 {
                tokens_this_turn as f64 / dt.as_secs_f64()
            } else {
                0.0
            };
            let raw_calls = parser.finish();
            reused_total = reused_total.max(reused as u32);
            log::info!(
                "[syn_chat] turn={} {} tokens in {:?} ({:.1} tok/s), prefill {} ms \
                 (префикс-KV переиспользовал {} из {} ток промпта), tool_calls={}",
                turn,
                tokens_this_turn,
                dt,
                tok_per_s,
                ttft_ms.unwrap_or(0),
                reused,
                prompt_ids.len(),
                raw_calls.len()
            );

            // Статистика панели — по КАЖДОМУ ходу, а не только по первому:
            // после tool-вызовов промпт вырастает в разы, и старое значение
            // (промпт первого хода) выглядело как «токенов мало, а OOM».
            let ctx_stat = ctx.clone();
            let stat = TurnStats {
                prompt_tokens: prompt_ids.len() as u32,
                reused_tokens: reused as u32,
                gen_tokens: total_gen_tokens + tokens_this_turn,
                prefill_ms: ttft_ms.unwrap_or(0),
                turns: turn as u32 + 1,
                ring_tokens: plan.ring_tokens as u32,
                ring_bytes: plan.ring_bytes(),
                ctx_budget: plan.by_mem as u32,
                vram_free_mb: vram_after as u32,
            };
            run_on_main_thread(move || stat.apply(&ctx_stat));

            break (raw_text, clean_text, raw_calls, tokens_this_turn, channel_mode);
        };
        let (raw_text, clean_text, raw_calls, tokens_this_turn, channel_mode) = turn_result;

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
    /// `Some(имя)` — результат инструмента: в prompt уходит как `role=tool`
    /// (`Message::tool_named`), поле `role` при этом не используется.
    tool_name: Option<String>,
}

impl HistoryItem {
    fn has_media(&self) -> bool {
        !self.attachments.is_empty()
    }
}

/// Снимок возможностей модели и окружения по части вложений. Читает
/// сигналы, поэтому вызывается только с main thread.
fn snapshot_media_caps(app: &AppCtx, model: &Arc<LoadedSynModel>) -> MediaCaps {
    let max = app.syn_chat_max_image_tokens.get_untracked();
    MediaCaps {
        // Кэш, а не Llm::supports_media(): функция зовётся с main thread, а
        // мьютекс пайплайна занят на всё время идущей генерации.
        vision: model.supports_media,
        vision_error: None,
        max_image_tokens: (max > 0).then_some(max),
        asr: Some(app.audio.asr.clone()),
    }
}

/// Строит prompt-историю для chat-template. Пустой плейсхолдер ассистента
/// в конце ленты исключается (он добавлен в `send_message` только для UI).
/// При непустом `system_prompt` префиксует историю system-сообщением.
///
/// Роль в ленте — не роль в prompt'е. Шаблоны Qwen принимают `system`
/// только первым сообщением и raise'ят на нём в середине истории, а лента
/// хранит с `role=System` и tool-результаты, и служебные плашки. Поэтому:
///   - tool_result уходит как `role=tool` — так же, как agent-loop кладёт
///     его в свою историю внутри одного хода;
///   - tool_call восстанавливается в канонический ChatML-блок `<tool_call>`
///     из структурированных `tool_calls` (в `body` лежит pretty-JSON для UI);
///   - плашки ленты (ошибка соединения, отмена и т. п.) — UI-only, в prompt
///     не идут;
///   - summary autocompact-маркеров дописывается в начальное
///     system-сообщение, сами маркеры в историю не попадают.
fn build_history(ctx: &SynChatCtx, system_prompt: &str) -> Vec<HistoryItem> {
    let msgs = ctx.messages.get_untracked();

    let mut sys = system_prompt.trim().to_string();
    for m in msgs.iter().filter(|m| m.compacted_iter.is_none()) {
        if let ChatMsgKind::CompactionMarker { summary, .. } = &m.kind {
            if summary.trim().is_empty() {
                continue;
            }
            if !sys.is_empty() {
                sys.push_str("\n\n");
            }
            sys.push_str("Сжатая история более ранних сообщений:\n");
            sys.push_str(summary.trim());
        }
    }

    let mut out: Vec<HistoryItem> = Vec::with_capacity(msgs.len() + 1);
    if !sys.is_empty() {
        out.push(HistoryItem {
            role: ChatMsgRole::System,
            body: sys,
            attachments: Vec::new(),
            tool_name: None,
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
        match &m.kind {
            ChatMsgKind::Text => {
                if m.role == ChatMsgRole::System {
                    continue;
                }
                out.push(HistoryItem {
                    role: m.role,
                    body: m.body.clone(),
                    attachments: m.attachments.clone(),
                    tool_name: None,
                });
            }
            ChatMsgKind::ToolCall { .. } => {
                let mut body = String::new();
                for c in m.tool_calls.iter().flatten() {
                    let name = c.function.name.clone().unwrap_or_default();
                    let args = c
                        .function
                        .arguments
                        .as_deref()
                        .and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok())
                        .unwrap_or(serde_json::Value::Null);
                    let call = serde_json::json!({ "name": name, "arguments": args });
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(&format!("<tool_call>\n{call}\n</tool_call>"));
                }
                if body.is_empty() {
                    continue;
                }
                out.push(HistoryItem {
                    role: ChatMsgRole::Assistant,
                    body,
                    attachments: Vec::new(),
                    tool_name: None,
                });
            }
            ChatMsgKind::ToolResult { tool_name, .. } => {
                out.push(HistoryItem {
                    role: m.role,
                    body: m.body.clone(),
                    attachments: Vec::new(),
                    tool_name: Some(tool_name.clone()),
                });
            }
            // Уже учтён в system-префиксе выше.
            ChatMsgKind::CompactionMarker { .. } => continue,
        }
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
    ctx: &SynChatCtx,
) -> (Vec<Message>, Vec<MediaEmbedding>) {
    // Башня нужна только под новые вложения: всё, что уже посчитано, берётся
    // из кэша эмбеддингов и переживает и регенерацию, и выгрузку башни.
    let needs_vision = caps.vision
        && items
            .iter()
            .any(|i| attach_prompt::needs_tower(&i.attachments, caps));
    let tower = attach_prompt::ensure_tower(&model.model, needs_vision);
    let vision_ready = tower.is_ok();
    let mut caps = MediaCaps { vision: vision_ready, ..caps.clone() };
    if let Err(reason) = tower {
        // Пустая причина — vision и не требовался (новых вложений нет).
        if !reason.is_empty() {
            let msg = format!("Вложение не передано модели: {reason}");
            let ctx = ctx.clone();
            run_on_main_thread(move || ctx.error.set(Some(msg)));
            caps.vision_error = Some(reason);
        }
    }

    let mut out: Vec<Message> = Vec::with_capacity(items.len());
    let mut media: Vec<MediaEmbedding> = Vec::new();
    for item in items {
        if let Some(name) = &item.tool_name {
            out.push(Message::tool_named(name.as_str(), item.body.as_str()));
            continue;
        }
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
        attach_prompt::release_tower(&model.model);
    }
    (out, media)
}

