//! Запуск in-process генерации через llm-qwen36 с поддержкой tool-calling.
//!
//! `send_message` собирает историю + tool-schemas, спавнит worker-thread и в
//! локальном tokio current_thread runtime гоняет [`run_agent_loop`]:
//!
//! ```text
//! loop turn in 0..max_turns:   # настройка «Глубина основного агента»
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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synaptix::facade::llm::{
    Message,
};

use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
use crate::agent::tools::catalog::KEY_VIEW_MEDIA;
use crate::agent::tools::{self, Tool, ToolDecision};
use crate::context::AppCtx;
use crate::syn_chat::attach::prompt::{self as attach_prompt, MediaCaps};
use crate::syn_chat::channel_parser::{self, ChannelIds};
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{
    ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment, QueuedMsg, SynChatCtx,
};
use crate::syn_chat::chat_settings::{ChatSettings, TurnSettings};
use crate::syn_chat::system_prompt::{self, PromptEnv};
use crate::syn_chat::tool_parser::RawToolCall;

/// Cap частоты обновлений streaming-сигналов из worker thread. При 16 мс
/// ≈ 60 fps — UI получает свежий хвост, не задыхаясь от 100+ updates/s.
const FLUSH_INTERVAL_MS: u64 = 16;
/// Бюджет ответа summary-запроса компактификации: сводка в 7–15 абзацев —
/// это сотни токенов, 2048 хватает с запасом и не раздувает ринг.
const SUMMARY_MAX_NEW_TOKENS: usize = 2048;

/// Сколько раз ПОДРЯД агенту прощается один и тот же вызов инструмента.
///
/// Считаем именно подряд идущие повторы: между двумя одинаковыми вызовами
/// подряд не происходит вообще ничего, кроме собственного хода модели, —
/// значит и результат измениться не может. А вот «правка файла → та же
/// команда сборки → правка → та же команда» повтором не является, и блокировать
/// её нельзя.
///
/// Повтор №2 подряд не исполняется: вместо результата модель получает заметку
/// «этот вызов уже был». Повтор №3 останавливает ход. Без guard'а loop
/// вырождается намертво: в сохранённых чатах встречается 106 подряд
/// идентичных `bash`-вызовов на одно сообщение пользователя — greedy-сэмплинг
/// на контексте из одинаковых блоков просто не имеет причины выбрать другое.
const REPEAT_WARN_AT: usize = 2;
/// Порог остановки хода (см. [`REPEAT_WARN_AT`]).
const REPEAT_STOP_AT: usize = 3;
/// С какого по счёту повтора (уже не подряд) к результату дописывается
/// подсказка. Такой вызов исполняется как обычно — он может быть законным
/// (пересборка после правки), но если результат не меняется, модель должна
/// это заметить.
const REPEAT_HINT_AT: usize = 3;
/// Температура, ниже которой ход не опускается после детекта повтора:
/// greedy-декод из петли не выходит в принципе, нужна хоть какая-то
/// стохастика. Пользовательскую настройку не понижаем — только поднимаем.
///
/// Температура — единственное, что анти-loop режим меняет в сэмплинге.
/// До 03.09.2026 он ещё включал frequency-штраф 0.25 по окну в 256 токенов
/// и растягивал до того же окна `repeat_penalty` пользователя. На агентном
/// промпте это ломало сам вызов инструмента: самые частые токены окна —
/// кавычки, переводы строк, `>` из JSON и тегов — получали по −0,25·count
/// логитов, и модель подменяла их редкими вариантами (`>>`, `\r\n`,
/// `\n\n\n`, `<invoke name=bash">` без кавычки). Проза при этом оставалась
/// осмысленной, ломался только синтаксис — в ленте это выглядело как
/// `bash {}` три раза подряд и стоп по guard'у.
///
/// Дефолт сэмплинга теперь тоже 0.6, поэтому порог выше: иначе режим ничего
/// не менял бы, и модель, которая дважды проигнорировала подсказку guard'а
/// (Muse-Glimmer, 03.09), просто останавливалась третьим повтором.
const ANTI_LOOP_TEMPERATURE: f32 = 0.85;
/// Сколько полных периодов чередования «A, B, A, B…» считаем предупреждением
/// (аналог [`REPEAT_WARN_AT`] для петли из двух вызовов) и остановкой.
/// Guard «тот же вызов подряд» такую петлю не видит: 28.08.2026 агент
/// 25 ходов чередовал `bash" {}` и `bash {"arguments":"\n\n\n"}`, пока не
/// кончился бюджет ходов.
const ALTERNATION_WARN_AT: usize = 2;
const ALTERNATION_STOP_AT: usize = 3;
/// Сколько последних ключей вызовов держим для детекта чередования.
const ALTERNATION_WINDOW: usize = 2 * ALTERNATION_STOP_AT;
/// Guard зацикливания внутри одной генерации: если последние
/// [`LOOP_WINDOW_CHARS`] символов хода — один фрагмент не длиннее
/// [`LOOP_MAX_PERIOD_CHARS`] символов, повторённый по кругу, стрим
/// обрывается, ход отбрасывается и переигрывается с анти-loop температурой
/// и заметкой.
///
/// 07.09.2026, qwen3.8-27b при temperature 0: после чтения выписки модель
/// писала python-скрипт с картой категорий и уходила в `'LILIT','LILIT',…`
/// на 18k токенов (8 минут), пока не упиралась в потолок ответа; скрипт
/// с оборванной строкой падал с SyntaxError, и следующий ход повторял то
/// же самое. Guard'ы вызовов бессильны: петля живёт внутри одного вызова.
///
/// Мера — символы декодированного текста, а не id токенов: первый вариант
/// по id (окно 1024, период ≤ 128) на той же петле сработал только на
/// 18672-м токене — на MTP-декоде гибрида один и тот же текст выходит
/// разной нарезкой токенов, и по id хвост периодичным не был. Окно в 3072
/// символа при периоде до 384 — не меньше восьми повторов подряд: честной
/// таблице или коду столько одинаковых кусков не нужно, а зацикленная
/// модель повторяет тысячи.
const LOOP_WINDOW_CHARS: usize = 3072;
const LOOP_MAX_PERIOD_CHARS: usize = 384;
/// Проверка периодичности — раз в столько токенов; непериодичный хвост
/// отбрасывает каждый период на первых же символах, так что проверка
/// дешёвая.
const LOOP_CHECK_EVERY: usize = 64;
/// Сколько вызовов подряд, не прошедших разбор аргументов (битый JSON, нет
/// обязательного поля, неизвестный инструмент), останавливают ход. Модель,
/// трижды не собравшая вызов, не соберёт его и на десятый раз — а бюджет
/// ходов при этом сгорает молча.
const INVALID_ARGS_STOP_AT: usize = 3;
/// Сколько раз один и тот же вызов может вернуть тот же результат, прежде
/// чем guard перестанет его исполнять (и, [`STAGNANT_STOP_AT`], остановит
/// ход).
///
/// Считаем по результату, а не по числу вызовов: «правка файла → та же
/// команда сборки» законна ровно до тех пор, пока сборка отвечает по-разному.
/// Как только один и тот же вызов третий раз подряд отдаёт тот же вывод —
/// между вызовами не изменилось ничего, что модель могла бы заметить, и
/// продолжение петли гарантировано. Изменчивые части вывода (идентификаторы,
/// числа) при сравнении не учитываются: 04.09.2026 агент 21 раз создал
/// страницу «Туду — канбан» с одним и тем же телом, и каждый ответ отличался
/// только новым id и номером в названии-дубликате.
const STAGNANT_WARN_AT: usize = 2;
/// Порог остановки хода (см. [`STAGNANT_WARN_AT`]).
const STAGNANT_STOP_AT: usize = 3;
/// Сколько исполнений одного и того же вызова за ход останавливают его
/// независимо от результатов. Крайний предохранитель на случай, когда вывод
/// формально меняется каждый раз (счётчик, время, новый id), а работы в этом
/// нет: восьми одинаковых вызовов на одно сообщение пользователя не требует
/// ни один законный сценарий.
const TOTAL_STOP_AT: usize = 8;
/// Сколько раз вызов, отличающийся от прежних только числами (координаты,
/// размеры, отступы), может вернуть по сути тот же результат (отпечаток без
/// чисел), прежде чем guard перестанет его исполнять. 14.09.2026, чат MyLife:
/// `blocks op=arrange` одной страницы с y=160, 920, 160, 1650, 250, 160 —
/// точные ключи разные, форма одна подряд (это не чередование), ответ тот же:
/// модель подбирала числа вместо того, чтобы прочитать ответ.
const NUMERIC_STAGNANT_WARN_AT: usize = 2;
/// Порог остановки хода: такой вызов уже не исполнен, а модель пришла снова.
const NUMERIC_STAGNANT_STOP_AT: usize = 3;
/// Сколько периодов чередования по «форме» вызова (см. [`call_shape`])
/// переводят ход в анти-loop сэмплинг.
///
/// Порог выше, чем у чередования точных ключей, и срабатывание мягче — только
/// температура, без блокировки вызова: одинаковая форма бывает и у честной
/// пакетной работы («создать страницу → доску на ней» для каждой из десяти
/// сфер), останавливать такое нельзя.
const SHAPE_ALTERNATION_ANTI_LOOP_AT: usize = 3;
/// Сколько последних сообщений ленты не трогает компактификация внутри хода
/// (последняя tool-пара + плейсхолдер).
const IN_TURN_KEEP_TAIL: usize = 3;

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
    /// `(на карте, всего)` блоков — только при частичном оффлоаде.
    blocks_resident: Option<(u32, u32)>,
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
        ctx.last_blocks_resident.set_always(self.blocks_resident);
        ctx.last_turn_open.set_always(false);
    }
}


/// Отправляет сообщение от пользователя и запускает генерацию ответа.
/// Вызывается с main thread (использует use_context).
///
/// Пока идёт ход — свой или чужого чата (одна карта на всех), — сообщение
/// не уходит модели, а встаёт в очередь отправки (`SynChatCtx::queue`) и
/// уйдёт само, когда чат освободится: см. [`flush_queue`].
pub fn send_message(text: String) {
    let ctx = use_context::<SynChatCtx>();
    let mut attachments = ctx.pending_attachments.get_untracked();
    let share_paths = ctx.attach_share_paths.get_untracked();
    for a in &mut attachments {
        a.share_path = share_paths;
    }
    let text = text.trim().to_string();
    // Сообщение из одних вложений — валидный сценарий («что на картинке?»
    // можно и не писать), поэтому пустой текст блокирует отправку только
    // когда прикреплять тоже нечего.
    if text.is_empty() && attachments.is_empty() {
        return;
    }
    if ctx.attach_busy.get_untracked() > 0 {
        ctx.error.set(Some(tr!("chat.session.error.attachments_busy")));
        return;
    }
    if ctx.pending.get_untracked() || ctx.generating_chat.get_untracked().is_some() {
        enqueue(&ctx, text, attachments);
        return;
    }

    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some(tr!("chat.model.not_loaded")));
        return;
    };

    clear_input(&ctx);
    start_turn(ctx, model, text, attachments);
}

/// Черновик панели ввода отправлен: текст, вложения, счётчик токенов.
fn clear_input(ctx: &SynChatCtx) {
    ctx.clear_draft_attachments();
    ctx.input.set(String::new());
    ctx.input_gen.update(|v| *v += 1);
    ctx.input_tokens.set_always(0);
}

/// Кладёт сообщение в ленту активного чата и запускает ход. Ввод не
/// трогает — из очереди сюда приходят уже отправленные черновики, а в
/// панели к этому моменту может лежать следующий.
fn start_turn(ctx: SynChatCtx, model: Arc<LoadedSynModel>, text: String, attachments: Vec<MsgAttachment>) {
    // Чата может не быть вовсе (все заархивированы, свежий запуск): лента
    // тогда живёт в воздухе — автосейв без `active_chat_id` ничего не
    // сохраняет, и ход уходит впустую. Заводим чат до первого сообщения.
    if ctx.active_chat_id.get_untracked().is_none() {
        crate::syn_chat::registry::create_new();
    }

    // 1. Append user-message + плейсхолдер ассистента.
    ctx.editing_msg.set(None);
    ctx.messages.update(|m| {
        m.push(ChatMsg::user_with_attachments(text.clone(), attachments.clone()));
        m.push(ChatMsg::assistant_empty());
    });
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    ctx.error.set(None);
    ctx.turn_cap_reached.set(false);
    ctx.pending.set(true);

    start_agent_thread(model, ctx);
}

/// Ставит сообщение в очередь отправки активного чата и очищает ввод.
fn enqueue(ctx: &SynChatCtx, body: String, attachments: Vec<MsgAttachment>) {
    if ctx.active_chat_id.get_untracked().is_none() {
        crate::syn_chat::registry::create_new();
    }
    let Some(chat_id) = ctx.active_chat_id.get_untracked() else {
        return;
    };
    let id = ctx.queue_seq.fetch_add(1, Ordering::Relaxed);
    ctx.queue.update(|q| {
        q.push(QueuedMsg {
            id,
            chat_id,
            body,
            attachments,
            time: crate::agent::time::format_hm_now(),
        })
    });
    clear_input(ctx);
    ctx.error.set(None);
}

/// Первое сообщение очереди для чата `chat_id` — очередь общая, порядок
/// внутри чата = порядок постановки.
pub fn next_queued(queue: &[QueuedMsg], chat_id: &str) -> Option<QueuedMsg> {
    queue.iter().find(|m| m.chat_id == chat_id).cloned()
}

/// Убирает сообщение из очереди отправки.
pub fn cancel_queued(id: u64) {
    let ctx = use_context::<SynChatCtx>();
    ctx.queue.update(|q| q.retain(|m| m.id != id));
    if ctx.queue_editing.get_untracked() == Some(id) {
        ctx.queue_editing.set(None);
    }
}

/// Заменяет текст сообщения в очереди; пустой текст без вложений —
/// то же, что убрать его.
pub fn edit_queued(id: u64, body: String) {
    let ctx = use_context::<SynChatCtx>();
    let body = body.trim().to_string();
    let drop_it = body.is_empty()
        && ctx
            .queue
            .get_untracked()
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.attachments.is_empty())
            .unwrap_or(true);
    if drop_it {
        cancel_queued(id);
        return;
    }
    ctx.queue.update(|q| {
        if let Some(m) = q.iter_mut().find(|m| m.id == id) {
            m.body = body;
        }
    });
    ctx.queue_editing.set(None);
}

/// Отправляет первое сообщение очереди активного чата, если чат свободен.
/// Зовётся после каждого завершённого хода (и после «Стоп»: прерванный ход
/// — тоже завершённый, сообщение уходит следующим) и при открытии чата.
/// Следующее сообщение очереди уйдёт после ответа на это.
pub fn flush_queue() {
    let ctx = use_context::<SynChatCtx>();
    let Some(active) = ctx.active_chat_id.get_untracked() else {
        return;
    };
    let Some(next) = next_queued(&ctx.queue.get_untracked(), &active) else {
        return;
    };
    if ctx.pending.get_untracked() || ctx.generating_chat.get_untracked().is_some() {
        return;
    }
    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        // Модель выгрузили, пока сообщение ждало: очередь остаётся, уйдёт
        // при следующем удобном случае.
        return;
    };
    if ctx.attach_busy.get_untracked() > 0 {
        return;
    }
    ctx.queue.update(|q| q.retain(|m| m.id != next.id));
    if ctx.queue_editing.get_untracked() == Some(next.id) {
        ctx.queue_editing.set(None);
    }
    start_turn(ctx, model, next.body, next.attachments);
}

/// Удаляет одно сообщение ленты по индексу. Во время генерации — no-op:
/// лента принадлежит worker'у, и индексы под ним «плывут».
pub fn delete_message(idx: usize) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let mut removed = false;
    ctx.messages.update(|m| {
        if idx < m.len() {
            m.remove(idx);
            removed = true;
        }
    });
    if !removed {
        return;
    }
    ctx.editing_msg.set(None);
    reset_index_keyed_ui(&ctx);
}

/// Удаляет работу инструмента — вызов ВМЕСТЕ с его результатом. История,
/// где call остался без result (или наоборот), ломает парность в промпте:
/// модель видит вызов без ответа и начинает «доделывать» его заново.
/// Клик по любой из двух карточек сносит обе. Во время генерации — no-op.
pub fn delete_tool_work(idx: usize) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let mut removed = false;
    ctx.messages.update(|m| {
        let Some(msg) = m.get(idx) else { return };
        let pair = match &msg.kind {
            // Вызов → его результат ищем вперёд по tool_call_id;
            // исполнитель кладёт его следующим сообщением, но после
            // ручных правок ленты полагаться на соседство нельзя.
            ChatMsgKind::ToolCall { .. } => {
                let ids: Vec<String> = msg
                    .tool_calls
                    .iter()
                    .flatten()
                    .map(|c| c.id.clone())
                    .collect();
                m.iter().enumerate().skip(idx + 1).find_map(|(j, other)| {
                    match &other.kind {
                        ChatMsgKind::ToolResult { tool_call_id, .. }
                            if ids.iter().any(|id| id == tool_call_id) =>
                        {
                            Some(j)
                        }
                        _ => None,
                    }
                })
            }
            // Результат → его вызов ищем назад.
            ChatMsgKind::ToolResult { tool_call_id, .. } => {
                let want = tool_call_id.clone();
                m.iter().enumerate().take(idx).rev().find_map(|(j, other)| {
                    match &other.kind {
                        ChatMsgKind::ToolCall { .. }
                            if other
                                .tool_calls
                                .iter()
                                .flatten()
                                .any(|c| c.id == want) =>
                        {
                            Some(j)
                        }
                        _ => None,
                    }
                })
            }
            _ => None,
        };
        let mut to_remove = vec![idx];
        to_remove.extend(pair);
        to_remove.sort_unstable();
        for j in to_remove.into_iter().rev() {
            m.remove(j);
        }
        removed = true;
    });
    if !removed {
        return;
    }
    ctx.editing_msg.set(None);
    reset_index_keyed_ui(&ctx);
}

/// Удаляет цепочку tool-сообщений целиком — `len` подряд идущих сообщений
/// начиная со `start_idx` (свёрнутая группа «bash ×N» в minimal-режиме).
/// Диапазон дополнительно сверяется: сносим только Tool*-сообщения, чтобы
/// разъехавшиеся после чужой правки индексы не задели текст.
/// Во время генерации — no-op.
pub fn delete_tool_chain(start_idx: usize, len: usize) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let mut removed = false;
    ctx.messages.update(|m| {
        let end = (start_idx + len).min(m.len());
        if start_idx >= end {
            return;
        }
        let all_tool = m[start_idx..end].iter().all(|msg| {
            matches!(
                msg.kind,
                ChatMsgKind::ToolCall { .. } | ChatMsgKind::ToolResult { .. }
            )
        });
        if !all_tool {
            return;
        }
        m.drain(start_idx..end);
        removed = true;
    });
    if !removed {
        return;
    }
    ctx.editing_msg.set(None);
    reset_index_keyed_ui(&ctx);
}

/// Заменяет текст сообщения (правка in-place). Вложения и thinking не
/// трогаются. Во время генерации — no-op.
pub fn edit_message(idx: usize, body: String) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let text = body.trim_end().to_string();
    ctx.messages.update(|m| {
        if let Some(msg) = m.get_mut(idx) {
            msg.body = text;
        }
    });
    ctx.editing_msg.set(None);
}

/// Очищает ленту активного чата, оставляя сам чат: название, параметры
/// сэмплинга и плитку в рейле. Во время генерации — no-op (кнопка в шапке
/// на это время disabled).
pub fn clear_chat() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    ctx.messages.set(Vec::new());
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    ctx.error.set(None);
    ctx.turn_cap_reached.set(false);
    ctx.editing_msg.set(None);
    ctx.highlight_msg.set(None);
    let active = ctx.active_chat_id.get_untracked();
    ctx.queue.update(|q| q.retain(|m| Some(&m.chat_id) != active.as_ref()));
    ctx.queue_editing.set(None);
    reset_index_keyed_ui(&ctx);
    log::info!("[syn_chat] лента чата очищена");
}

/// Сбрасывает UI-состояние, ключованное индексами сообщений (раскрытые
/// thinking-блоки, группы и тела tool-карточек, маркеры сжатия): после
/// удаления/очистки индексы смещаются, и старые ключи указывали бы не туда.
fn reset_index_keyed_ui(ctx: &SynChatCtx) {
    ctx.thinking_open.set(HashMap::new());
    ctx.tool_group_open.set(HashMap::new());
    ctx.tool_body_open.set(HashMap::new());
    ctx.compaction_open.set(HashMap::new());
    ctx.wizard_drafts.set(HashMap::new());
}

/// Снять хвостовой пустой пузырь ассистента: он выглядит как «повисло».
fn pop_empty_assistant(m: &mut Vec<ChatMsg>) {
    if m.last()
        .map(|x| x.role == ChatMsgRole::Assistant && x.body.is_empty())
        .unwrap_or(false)
    {
        m.pop();
    }
}

/// Правка ленты чата, которому принадлежит генерация.
///
/// Активный чат правим в UI, фоновый — прямо в файле: с 04.09.2026
/// переключение чата не обрывает ход, и его сообщения не должны попадать в
/// чужую ленту. Файл — та же лента, которую `registry::select` прочитает при
/// возврате, так что накопленное за время отсутствия не теряется. Правку
/// файла делает фоновый писатель (`storage::update_async`) по порядку с
/// остальными записями этого чата; `registry::select` её дождётся.
pub(crate) fn ledger_update<F>(chat_id: &Option<String>, f: F)
where
    F: FnOnce(&mut Vec<ChatMsg>) + Send + 'static,
{
    let owner = chat_id.clone();
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        if ctx.active_chat_id.get_untracked() == owner {
            ctx.messages.update(f);
            return;
        }
        let Some(id) = owner else { return };
        crate::syn_chat::storage::update_async(&id, move |stored| f(&mut stored.messages));
    });
}

/// Перелить накопленный за ход текст в ленту: открытый чат берёт его из
/// живых стрим-сигналов, фоновый — из копии воркера (стрим туда не шёл).
pub(crate) fn commit_turn_text(chat_id: &Option<String>, body: String, thinking: String) {
    let owner = chat_id.clone();
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        if ctx.active_chat_id.get_untracked() == owner {
            // Живой стрим открытого чата — хвост полного текста хода: если
            // пользователь уходил в другой чат и вернулся посреди ответа,
            // в нём только то, что пришло после возврата (выбор чата
            // обнуляет стрим, а в файл начало не попадало). Недостающее
            // начало берём из текста воркера.
            let lost_body = missing_prefix(&body, &ctx.streaming_body.get_untracked());
            let lost_think = missing_prefix(&thinking, &ctx.streaming_thinking.get_untracked());
            if !lost_body.is_empty() || !lost_think.is_empty() {
                ctx.messages.update(|list| {
                    if let Some(last) = list.iter_mut().rev().find(|m| m.role == ChatMsgRole::Assistant) {
                        last.body.push_str(lost_body);
                        last.thinking.push_str(lost_think);
                    }
                });
            }
            ctx.commit_streaming_tail();
            return;
        }
        if body.trim().is_empty() && thinking.trim().is_empty() {
            return;
        }
        let Some(id) = owner else { return };
        crate::syn_chat::storage::update_async(&id, move |stored| {
            if let Some(last) = stored
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == ChatMsgRole::Assistant)
            {
                last.body.push_str(&body);
                last.thinking.push_str(&thinking);
            }
        });
    });
}

/// Вызовы, до которых ход не дошёл (Stop/Cancel посреди пачки), получают в
/// ленте результат «отменено». Иначе в ленте остаётся tool_call без
/// tool_result: такую пару `build_history` отдаёт модели на следующем
/// сообщении, а сжатие рвёт её на границе диапазона.
fn cancel_unanswered(chat_id: &Option<String>, calls: &[ChatToolCall]) {
    for call in calls {
        push_tool_result(chat_id, call, tr!("chat.session.tool.cancelled"), true);
    }
}

/// Часть `full`, которой нет в живом стриме `streamed` (его хвосте). Стрим —
/// не хвост (расхождение) — ничего не досыпаем, чтобы не задвоить текст.
fn missing_prefix<'a>(full: &'a str, streamed: &str) -> &'a str {
    full.strip_suffix(streamed).unwrap_or_default()
}

/// Идущий вызов инструмента для UI: статус «Выполняется инструмент …»
/// (`SynChatCtx::tool_running`) и, если у вызова есть предел времени,
/// обратный отсчёт на его карточке (`tool_limit` + секундомер). При drop
/// всё гасится — и при результате, и при Stop посреди вызова.
struct ToolCountdown;

impl ToolCountdown {
    fn start(chat_id: &Option<String>, tool: String, limit: Option<u64>) -> Self {
        let owner = chat_id.clone();
        run_on_main_thread(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.tool_running.set(Some((owner.clone(), tool)));
            if let Some(secs) = limit {
                ctx.tool_limit.set(Some((owner, secs)));
                ctx.tool_timer.start();
            }
        });
        Self
    }
}

impl Drop for ToolCountdown {
    fn drop(&mut self) {
        run_on_main_thread(|| {
            let ctx = use_context::<SynChatCtx>();
            ctx.tool_running.set(None);
            if ctx.tool_limit.get_untracked().is_some() {
                ctx.tool_timer.finish();
                ctx.tool_limit.set(None);
            }
        });
    }
}

/// Выполнить действие над контекстом, только если чат генерации открыт.
///
/// Живой стрим, плашки ошибок и `pending` принадлежат открытому чату: пока
/// ход доигрывает в фоне, показывать их в другом чате нельзя.
pub(crate) fn if_active<F>(chat_id: &Option<String>, f: F)
where
    F: FnOnce(&SynChatCtx) + Send + 'static,
{
    let owner = chat_id.clone();
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        if ctx.active_chat_id.get_untracked() == owner {
            f(&ctx);
        }
    });
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
/// «Идёт генерация в другом чате (название)» — для действий, которые
/// запустили бы второй параллельный ход.
fn busy_other_chat_error(ctx: &SynChatCtx, chat_id: &str) -> String {
    let chat = ctx
        .chats
        .with_untracked(|v| v.iter().find(|m| m.id == chat_id).map(|m| m.title.clone()))
        .unwrap_or_else(|| chat_id.to_string());
    tr!("chat.session.error.busy_other_chat", chat = chat)
}

pub fn regenerate_last() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        ctx.abort.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // Ход другого чата идёт в фоне: второй параллельный ход перезаписал бы
    // `generating_chat`/`turn_settings`, делил бы с ним `abort` и модель.
    if let Some(busy) = ctx.generating_chat.get_untracked() {
        ctx.error.set(Some(busy_other_chat_error(&ctx, &busy)));
        return;
    }
    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some(tr!("chat.model.not_loaded")));
        return;
    };

    // 1. Подготовить ленту: убрать tail-assistant и tool-result/tool_call если
    // есть, добавить пустой placeholder. Для регенерации режем всё после
    // последнего user-сообщения.
    ctx.editing_msg.set(None);
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
    if !ctx
        .messages
        .with_untracked(|m| m.iter().any(|x| x.role == ChatMsgRole::User))
    {
        return;
    }

    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    ctx.error.set(None);
    ctx.turn_cap_reached.set(false);
    ctx.pending.set(true);

    start_agent_thread(model, ctx);
}

/// Продолжить прерванный ход агента, не трогая уже накопленную историю.
///
/// В отличие от [`regenerate_last`], которая режет ленту до последнего
/// user-сообщения, здесь сохраняются все tool-call'ы и их результаты —
/// цикл просто получает свежий бюджет ходов (настройка «Глубина основного
/// агента») и продолжает с того места, где остановился. Основной сценарий — упёрлись в лимит
/// ходов (`turn_cap_reached`), но кнопка работает и после ручного
/// «Прервать», когда ответ оборвался на полуслове.
pub fn continue_last() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    // См. `regenerate_last`: один ход на приложение.
    if let Some(busy) = ctx.generating_chat.get_untracked() {
        ctx.error.set(Some(busy_other_chat_error(&ctx, &busy)));
        return;
    }
    let registry = use_context::<SynModelRegistry>();
    let Some(model) = registry.current.get_untracked() else {
        ctx.error.set(Some(tr!("chat.model.not_loaded")));
        return;
    };
    if !ctx
        .messages
        .with_untracked(|m| m.iter().any(|x| x.role == ChatMsgRole::User))
    {
        return;
    }

    // Плейсхолдер под новый ход. Если предыдущий ход оставил пустой
    // assistant-пузырь (abort посреди стрима), переиспользуем его, чтобы
    // не плодить пустые бабблы.
    ctx.messages.update(|m| {
        let tail_empty = m
            .last()
            .map(|x| x.role == ChatMsgRole::Assistant && x.body.is_empty())
            .unwrap_or(false);
        if !tail_empty {
            m.push(ChatMsg::assistant_empty());
        }
    });

    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    ctx.error.set(None);
    ctx.turn_cap_reached.set(false);
    ctx.pending.set(true);

    start_agent_thread(model, ctx);
}

/// Общая часть `send_message` / `regenerate_last` / `continue_last`: snapshot всех нужных
/// signal-данных на main thread, спавн worker-thread и запуск agent-loop в
/// локальном tokio current_thread runtime.
fn start_agent_thread(model: Arc<LoadedSynModel>, ctx: SynChatCtx) {
    let app_ctx = use_context::<AppCtx>();
    // Владелец хода: он переживёт переключение чатов, и по нему воркер решает,
    // писать ли в открытую ленту или прямо в файл своего чата.
    ctx.generating_chat.set(ctx.active_chat_id.get_untracked());
    // Настройки хода: если его чат уйдёт в фон, инструменты посреди хода
    // (`autotools`, `subagent`, автокомпакт) возьмут их отсюда, а не из
    // панелей уже другого чата.
    let turn = TurnSettings::capture(&ctx);
    ctx.turn_settings.set(Some(turn.clone()));

    // Карточки субагентов прошлого хода к новому вопросу отношения не
    // имеют — панель «Детали» начинает с чистого листа.
    crate::syn_chat::telemetry::reset();

    // 2. Snapshot params + история + tool-схемы (всё на main thread!).
    // В режиме `default` сэмплинг — из пресета модели.
    let params = turn.params.effective(&model.sampling);
    let max_turns = app_ctx.general.agent_max_turns.get_untracked().max(1) as usize;
    // Системный промпт агента: базовые правила + пользовательская добавка из
    // настроек. Пустое поле в настройках больше не означает «промпта нет».
    let system_prompt = system_prompt::build(&PromptEnv::snapshot(
        declared_tool_labels(&turn.chat.tools_active),
        pool_tool_labels(&turn.chat),
        max_turns,
        turn.chat.system_prompt.clone(),
    ));
    let channel = ChannelIds::detect(&model.tokenizer).is_some();
    let history: Vec<HistoryItem> = build_history(&ctx, &system_prompt, channel);
    let caps = snapshot_media_caps(&app_ctx, &model, params.max_seq_len);
    let tool_schemas: Vec<serde_json::Value> = collect_active_tool_schemas(&app_ctx, &turn.chat);
    let abort_snapshot = ctx.abort.load(Ordering::Relaxed);
    let chat_id = ctx.active_chat_id.get_untracked();
    let repeats = seen_calls_in_current_turn(&ctx);
    let autocompact_enabled = app_ctx.general.autocompact_enabled.get_untracked();
    let autocompact_threshold = app_ctx
        .general
        .autocompact_threshold_percent
        .get_untracked();
    let ctx_for_worker = ctx.clone();
    let chat_for_worker = chat_id.clone();
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
                let owner = chat_for_worker.clone();
                run_on_main_thread(move || {
                    let ctx = use_context::<SynChatCtx>();
                    if ctx.generating_chat.get_untracked() == owner {
                        ctx.generating_chat.set(None);
                    }
                    if ctx.active_chat_id.get_untracked() != owner {
                        return;
                    }
                    ctx.error.set(Some(tr!("chat.session.error.tokio_runtime", error = format!("{e:#}"))));
                    ctx.commit_streaming_tail();
                    ctx.pending.set(false);
                });
                return;
            }
        };
        let abort_for_compact = abort.clone();
        let chat_for_compact = chat_for_worker.clone();
        let result = rt.block_on(async {
            let r = run_agent_loop(
                model,
                history,
                caps,
                tool_schemas,
                params,
                abort,
                abort_snapshot,
                chat_id,
                ctx_for_worker.clone(),
                LoopSettings {
                    max_turns,
                    system_prompt,
                    autocompact_enabled,
                    autocompact_threshold,
                    repeats,
                },
            )
            .await;
            // Автокомпакт — строго после хода: генерация закончилась, guard
            // KV-слота отпущен, VRAM свободна под summary-запрос.
            //
            // Модель берём из реестра ЗАДНИМ ЧИСЛОМ, а не клоном до цикла:
            // upfront-клон жил бы весь ход и не давал `pipelines run` с
            // free_vram реально освободить веса. После free_vram-прогона
            // здесь уже лежит перезагруженная модель.
            if r.is_ok()
                && autocompact_enabled
                && abort_for_compact.load(Ordering::Relaxed) == abort_snapshot
            {
                let (tx, rx) = tokio::sync::oneshot::channel();
                run_on_main_thread(move || {
                    let cur = use_context::<SynModelRegistry>().current.get_untracked();
                    let _ = tx.send(cur);
                });
                if let Ok(Some(model_for_compact)) = rx.await {
                    crate::syn_chat::compact::maybe_autocompact(
                        &model_for_compact,
                        &abort_for_compact,
                        abort_snapshot,
                        autocompact_threshold,
                        chat_for_compact,
                    )
                    .await;
                }
            }
            r
        });
        if let Err(e) = result {
            eprintln!("[syn_chat] agent-loop error: {e:#}");
            if_active(&chat_for_worker, move |c| c.error.set(Some(format!("{e:#}"))));
        }
        // Финализация (всегда, даже при abort/error). Хвост стрима открытого
        // чата добираем здесь; фоновый ход перелил свой текст сам.
        if_active(&chat_for_worker, |c| c.commit_streaming_tail());
        // Прерывание посреди хода оставляло в ленте пустой
        // assistant-пузырь: в UI он выглядит как «повисло», а на
        // следующем сообщении уезжает в промпт пустой assistant-репликой
        // (`build_history` отбрасывал его, только пока он последний).
        ledger_update(&chat_for_worker, |m| {
            if m.last()
                .map(|x| {
                    x.role == ChatMsgRole::Assistant
                        && matches!(x.kind, ChatMsgKind::Text)
                        && x.body.is_empty()
                        && x.thinking.is_empty()
                })
                .unwrap_or(false)
            {
                m.pop();
            }
        });
        let owner = chat_for_worker;
        run_on_main_thread(move || {
            let ctx = use_context::<SynChatCtx>();
            // Ход мог доигрывать в фоне: «идёт генерация» гасим глобально, а
            // `pending` — только если открыт тот самый чат.
            if ctx.generating_chat.get_untracked() == owner {
                ctx.generating_chat.set(None);
            }
            if ctx.active_chat_id.get_untracked() == owner {
                ctx.pending.set(false);
            }
            // Очередь отправки: следующее сообщение уходит отдельным тиком —
            // из закрывающего колбэка хода запускать новый ход не стоит.
            run_on_main_thread(flush_queue);
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

/// Заметка в историю после хода, который не дал ни вызова, ни текста.
/// Уходит от лица пользователя: system лежит в голове контекста, и вставка
/// туда обнулила бы префикс-KV всего диалога ради одного хода.
/// Заметка после хода, отброшенного guard'ом зацикливания (см.
/// [`LOOP_WINDOW_TOKENS`]). Тоже от лица пользователя, на один ход.
const LOOP_NOTE: &str = "[System note: your previous turn was cut off because its \
     output degenerated into an endless repetition of the same fragment, and \
     the whole turn was discarded. Do not enumerate long literal lists or \
     mappings by hand; keep code and text concise, and finish the turn with a \
     valid tool call or a text answer.]";

const EMPTY_TURN_NOTE: &str = "[System note: your previous turn produced neither a tool call nor a text answer — the whole budget went into reasoning. If you meant to call a tool, send the call again as valid JSON: arguments as a real structure, every bracket closed in the right order. Otherwise answer with text.]";

/// Главный цикл агента: prompt → generate → parse tool_calls → execute →
/// append history → next turn. Прерывается по abort, EOS-only ответу (no
/// tool_calls) или по достижении `max_turns` (настройка «Глубина основного
/// агента», `AppConfig.general.agent_max_turns`).
#[allow(clippy::too_many_arguments)]
/// Настройки одного запуска agent-loop'а, снятые на main-потоке.
struct LoopSettings {
    /// «Глубина основного агента» — сколько ходов даётся на одно сообщение.
    max_turns: usize,
    /// Уже собранный системный промпт (правила агента + добавка из настроек).
    /// Нужен и после компактификации — для пересборки истории.
    system_prompt: String,
    autocompact_enabled: bool,
    autocompact_threshold: u32,
    /// Состояние guard'а повторов, восстановленное из ленты.
    repeats: RepeatState,
}

/// Состояние guard'а повторов, восстановленное из ленты: сколько раз каждый
/// вызов уже сделан в текущем ходе, каким был последний вызов и сколько раз
/// он повторён подряд.
///
/// Нужно, чтобы guard пережил «Прервать» и «Продолжить»: иначе кнопка просто
/// перезапускала бы ту же петлю с чистого листа.
///
/// Свёрнутые компактификацией сообщения не считаем: их результатов в промпте
/// больше нет, и повторный вызов там уже осмыслен.
#[derive(Default)]
struct RepeatState {
    totals: HashMap<String, usize>,
    last: Option<String>,
    consecutive: usize,
    /// Последние [`ALTERNATION_WINDOW`] ключей вызовов — для детекта
    /// чередования (см. [`alternation_periods`]).
    recent: std::collections::VecDeque<String>,
    /// То же окно, но по формам вызовов (см. [`call_shape`]): ловит петлю,
    /// в которой значения аргументов каждый раз новые.
    recent_shapes: std::collections::VecDeque<String>,
    /// По каждому вызову: отпечаток его последнего результата и сколько раз
    /// подряд этот результат повторился (см. [`STAGNANT_WARN_AT`]).
    outcomes: HashMap<String, (u64, usize)>,
    /// То же по ключам с числами под маской (см. [`numeric_masked_key`]).
    masked_outcomes: HashMap<String, (u64, usize)>,
}

/// Сколько полных периодов «A, B, A, B…» (A ≠ B) лежит в хвосте `keys`.
/// Период, с которым повторяются последние `window` элементов (`None` — не
/// повторяются или элементов меньше окна). Берётся наименьший период до
/// `max_period` включительно: у `'LILIT','LILIT',…` это длина одного
/// элемента, у зацикленной строки таблицы — вся строка.
fn periodic_tail<T: PartialEq>(items: &[T], window: usize, max_period: usize) -> Option<usize> {
    if items.len() < window || window == 0 {
        return None;
    }
    let tail = &items[items.len() - window..];
    (1..=max_period.min(window / 2)).find(|&p| tail[p..].iter().zip(tail.iter()).all(|(a, b)| a == b))
}

fn alternation_periods(keys: &std::collections::VecDeque<String>) -> usize {
    let n = keys.len();
    if n < 4 {
        return 0;
    }
    let (a, b) = (&keys[n - 2], &keys[n - 1]);
    if a == b {
        return 0;
    }
    let mut periods = 0;
    let mut i = n;
    while i >= 2 && &keys[i - 2] == a && &keys[i - 1] == b {
        periods += 1;
        i -= 2;
    }
    periods
}

/// Запомнить ключ вызова в окне детекта чередования.
fn remember_call(recent: &mut std::collections::VecDeque<String>, key: &str) {
    if recent.len() >= ALTERNATION_WINDOW {
        recent.pop_front();
    }
    recent.push_back(key.to_string());
}

/// Учесть результат исполненного вызова: вернуть, сколько раз подряд этот
/// вызов отдаёт один и тот же вывод (0 — вывод только что изменился).
fn note_outcome(outcomes: &mut HashMap<String, (u64, usize)>, key: &str, content: &str) -> usize {
    let fp = outcome_fingerprint(content);
    match outcomes.get_mut(key) {
        Some(prev) if prev.0 == fp => {
            prev.1 += 1;
            prev.1
        }
        Some(prev) => {
            *prev = (fp, 0);
            0
        }
        None => {
            outcomes.insert(key.to_string(), (fp, 0));
            0
        }
    }
}

fn seen_calls_in_current_turn(ctx: &SynChatCtx) -> RepeatState {
    ctx.messages.with_untracked(|m| repeat_state_from(m))
}

/// Чистая часть [`seen_calls_in_current_turn`] — считает состояние по ленте.
fn repeat_state_from(msgs: &[ChatMsg]) -> RepeatState {
    let from = msgs
        .iter()
        .rposition(|m| m.role == ChatMsgRole::User && matches!(m.kind, ChatMsgKind::Text))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut st = RepeatState::default();
    // Какой вызов ждёт своего результата: guard считает стагнацию по выводу,
    // а в ленте вывод лежит отдельным сообщением со ссылкой на id вызова.
    let mut awaiting: HashMap<String, String> = HashMap::new();
    let mut awaiting_masked: HashMap<String, String> = HashMap::new();
    for m in msgs.iter().skip(from) {
        if m.compacted_iter.is_some() {
            continue;
        }
        match &m.kind {
            ChatMsgKind::ToolCall { .. } => {
                for c in m.tool_calls.iter().flatten() {
                    let key = call_key(c);
                    *st.totals.entry(key.clone()).or_insert(0) += 1;
                    st.consecutive = if st.last.as_deref() == Some(key.as_str()) {
                        st.consecutive + 1
                    } else {
                        1
                    };
                    remember_call(&mut st.recent, &key);
                    remember_call(&mut st.recent_shapes, &call_shape(c));
                    awaiting.insert(c.id.clone(), key.clone());
                    let masked = numeric_masked_key(c);
                    if masked != key {
                        awaiting_masked.insert(c.id.clone(), masked);
                    }
                    st.last = Some(key);
                }
            }
            ChatMsgKind::ToolResult { tool_call_id, .. } => {
                if let Some(key) = awaiting.remove(tool_call_id) {
                    note_outcome(&mut st.outcomes, &key, &m.body);
                }
                if let Some(masked) = awaiting_masked.remove(tool_call_id) {
                    note_outcome(&mut st.masked_outcomes, &masked, &m.body);
                }
            }
            _ => {}
        }
    }
    st
}

/// Пересобрать историю agent-loop'а из ленты — после компактификации внутри
/// хода собственная копия истории устарела.
///
/// Зовётся только на медиа-свободном пути (см. вызов), поэтому
/// `prepare_history` здесь не трогает vision-башню.
async fn rebuild_history(
    ctx: &SynChatCtx,
    system_prompt: &str,
    model: &Arc<LoadedSynModel>,
    caps: &MediaCaps,
    chat_id: &Option<String>,
) -> Option<(Vec<Message>, crate::syn_chat::daemon::MediaAt)> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<HistoryItem>>();
    let ctx_main = ctx.clone();
    let sp = system_prompt.to_string();
    let channel = ChannelIds::detect(&model.tokenizer).is_some();
    let chat_id = chat_id.clone();
    run_on_main_thread(move || {
        // История собирается из открытой ленты — только если это чат хода.
        if ctx_main.active_chat_id.get_untracked() != chat_id {
            return;
        }
        let _ = tx.send(build_history(&ctx_main, &sp, channel));
    });
    let items = rx.await.ok()?;
    Some(prepare_history(&items, caps))
}

async fn run_agent_loop(
    mut model: Arc<LoadedSynModel>,
    mut items: Vec<HistoryItem>,
    caps: MediaCaps,
    tool_schemas: Vec<serde_json::Value>,
    params: SamplingParams,
    abort: Arc<AtomicU64>,
    abort_snapshot: u64,
    chat_id: Option<String>,
    ctx: SynChatCtx,
    settings: LoopSettings,
) -> anyhow::Result<()> {
    let LoopSettings {
        max_turns,
        system_prompt: sys_prompt,
        autocompact_enabled,
        autocompact_threshold,
        repeats:
            RepeatState {
                totals: mut call_seen,
                last: last_call_key,
                consecutive: consecutive_repeats,
                recent: mut recent_keys,
                mut recent_shapes,
                outcomes: mut call_outcomes,
                masked_outcomes: mut numeric_outcomes,
            },
    } = settings;
    let mut total_gen_tokens: u32 = 0;
    // Чистое время декода по всем ходам — без префилла и без пауз на
    // исполнение инструментов. Итоговый tps в панели считается по нему,
    // иначе финальное число не сходится с тем, что бежало вживую.
    let mut total_decode_s: f64 = 0.0;
    let t_overall = Instant::now();

    // Найденное в базах знаний — в копию текущего сообщения пользователя, не
    // в системный промпт: префикс истории не меняется, префикс-KV цел; лента
    // чата тоже (следующий ход переиспользует кэш до этого сообщения).
    if let Some(block) = kb_augment_block(&items).await {
        if let Some(last) = items
            .iter_mut()
            .rev()
            .find(|i| i.role == ChatMsgRole::User && i.tool_name.is_none())
        {
            last.body = format!("{block}\n\n{}", last.body);
        }
    }
    // Картинки и видео истории — файлами для службы: её башня зрения кодирует
    // их и кэширует эмбеддинги сама; `media_at` растёт и посреди хода
    // (`view_media`).
    let (mut history, mut media_at) = prepare_history(&items, &caps);
    if !media_at.is_empty() {
        log::info!("[syn_chat] медиа-вложений: {}", media_at.values().map(Vec::len).sum::<usize>());
    }
    // Префикс-KV в службе — сессия по ключу чата: ход дописывает в кэш
    // только новый хвост промпта.
    let prefix_kv_on = crate::config::AppConfig::load().syn_chat_prefix_kv;
    let mut reused_total: u32 = 0;

    // Отличаем «модель ответила текстом» (break ниже) от «кончился бюджет
    // ходов». Раньше второй случай молча падал в конец функции: UI оставался
    // с пустым assistant-плейсхолдером, в логе — ничего, и снаружи это
    // выглядело как зависшая без ошибки генерация.
    let mut answered = false;
    // `wizard` показал вопрос: ход заканчивается, ответ придёт следующим
    // сообщением пользователя. Ставится после результата вызова, ход
    // обрывается после всех вызовов этого хода (пары call/result целы).
    let mut awaiting_user = false;
    // Ход без вызовов и без текста: весь бюджет ушёл в reasoning, который
    // оборвался на полуслове. Это не ответ — раньше цикл принимал его за
    // ответ и выходил, оставляя в ленте пустой пузырь и брошенную работу.
    let mut empty_answer = false;
    // Предыдущий ход отброшен guard'ом зацикливания (см. [`LOOP_WINDOW_TOKENS`])
    // и переигрывается; второй такой подряд — стоп.
    let mut loop_retried = false;
    // Сколько вызовов инструментов агент реально сделал за всю генерацию.
    // Ноль при активных инструментах — тот самый случай, когда модель
    // объявляет намерение («I'll start by exploring the project») и на этом
    // заканчивает ход: ответ формально есть, работа не сделана.
    let mut tool_calls_made = 0usize;

    // Guard от вырождения в петлю. Счётчики приходят из ленты, поэтому
    // переживают «Прервать» и «Продолжить» — иначе кнопка просто
    // перезапускала бы ту же петлю с чистого листа.
    let mut anti_loop = consecutive_repeats >= REPEAT_WARN_AT
        || alternation_periods(&recent_keys) >= ALTERNATION_WARN_AT
        || alternation_periods(&recent_shapes) >= SHAPE_ALTERNATION_ANTI_LOOP_AT
        || call_outcomes.values().any(|(_, n)| *n >= STAGNANT_WARN_AT);
    let mut last_call: Option<String> = last_call_key;
    let mut consecutive = consecutive_repeats;
    // Сколько вызовов подряд не прошли разбор аргументов (см.
    // [`INVALID_ARGS_STOP_AT`]).
    let mut invalid_streak = 0usize;
    // Заметка модели на один ход (после пустого хода). В `history` не
    // кладём: истории, собранной из ленты на следующем сообщении, такой
    // заметки взять неоткуда, и промпт разошёлся бы с промптом хода навсегда
    // — префикс-KV обнулялся бы на каждом сообщении. Один перепрефилл после
    // хода с заметкой дешевле.
    let mut pending_note: Option<&'static str> = None;
    // Причина досрочной остановки: показывается вместо сообщения о лимите
    // ходов, чтобы пользователь видел именно «агент зациклился».
    let mut stop_reason: Option<String> = None;

    'agent: for turn in 0..max_turns {
        if abort.load(Ordering::Relaxed) != abort_snapshot {
            return Ok(());
        }

        let for_prompt: Vec<Message> = match pending_note.take() {
            Some(note) => {
                let mut h = history.clone();
                h.push(Message::user(note));
                h
            }
            None => history.clone(),
        };
        let mut opts = params.clone();
        if anti_loop {
            // Повтор уже случился. Greedy-декод из петли не выходит никогда,
            // поэтому поднимаем температуру — и только её: штрафы за повторы
            // на агентном промпте ломают синтаксис вызова (см.
            // [`ANTI_LOOP_TEMPERATURE`]). Настройку пользователя только
            // повышаем. Режим снимается первым же неповторённым вызовом.
            opts.temperature = opts.temperature.max(ANTI_LOOP_TEMPERATURE);
            log::info!("[syn_chat] анти-loop сэмплинг: temp={:.2}", opts.temperature);
        }
        // Ключ KV-сессии в службе — чат: ходы и сообщения одного чата
        // дописывают в кэш только новый хвост промпта.
        let session_key = prefix_kv_on.then(|| chat_id.clone().unwrap_or_else(|| "syn_chat".into()));
        let call = crate::syn_chat::daemon::params(&opts, session_key.as_deref());
        // «Детали» с первой секунды хода — про этот ход.
        {
            let first_turn = turn == 0;
            if_active(&chat_id, move |c| {
                c.last_prefill_ms.set_always(0);
                c.last_reused_tokens.set_always(0);
                if first_turn {
                    c.last_gen_tokens.set_always(0);
                    c.last_decode_tps.set_always(0.0);
                    c.last_turns.set_always(1);
                }
                c.last_turn_open.set_always(true);
            });
        }
        let mut buf_body = String::new();
        let mut buf_think = String::new();
        let mut buf_tool = String::new();
        let mut last_flush = Instant::now();
        let flush_interval = Duration::from_millis(FLUSH_INTERVAL_MS);
        let mut tokens_this_turn: u32 = 0;
        // Честный prefill: время до ПЕРВОГО токена.
        let mut ttft_ms: Option<u32> = None;
        // Хвост текста хода (символы) — для guard'а зацикливания;
        // `looped` — найденный период, по нему стрим и оборван.
        let mut turn_tail: Vec<char> = Vec::new();
        let mut looped: Option<usize> = None;
        let t_turn = Instant::now();
        let gen_before = total_gen_tokens;
        let chat_for_cb = chat_id.clone();
        let stop: Arc<dyn Fn() -> bool + Send + Sync> = {
            let abort = abort.clone();
            Arc::new(move || abort.load(Ordering::Relaxed) != abort_snapshot)
        };
        let mut on_event = |ev: synaptixd::Event| -> bool {
            let (body, thinking, tool) = match ev {
                synaptixd::Event::Token { text } => (text, String::new(), String::new()),
                synaptixd::Event::Thinking { text } => (String::new(), text, String::new()),
                synaptixd::Event::ToolText { text } => (String::new(), String::new(), text),
                _ => return true,
            };
            if ttft_ms.is_none() {
                let ms = t_turn.elapsed().as_millis() as u32;
                ttft_ms = Some(ms);
                if_active(&chat_for_cb, move |c| c.last_prefill_ms.set_always(ms));
            }
            tokens_this_turn += 1;
            turn_tail.extend(body.chars().chain(thinking.chars()).chain(tool.chars()));
            if turn_tail.len() > LOOP_WINDOW_CHARS * 2 {
                let extra = turn_tail.len() - LOOP_WINDOW_CHARS;
                turn_tail.drain(..extra);
            }
            if (tokens_this_turn as usize).is_multiple_of(LOOP_CHECK_EVERY) {
                if let Some(p) = periodic_tail(&turn_tail, LOOP_WINDOW_CHARS, LOOP_MAX_PERIOD_CHARS) {
                    looped = Some(p);
                    flush_streaming(&chat_for_cb, &mut buf_body, &mut buf_think, &mut buf_tool, None);
                    return false;
                }
            }
            buf_body.push_str(&body);
            buf_think.push_str(&thinking);
            buf_tool.push_str(&tool);
            let now = Instant::now();
            if now.duration_since(last_flush) >= flush_interval {
                last_flush = now;
                flush_streaming(
                    &chat_for_cb,
                    &mut buf_body,
                    &mut buf_think,
                    &mut buf_tool,
                    Some(live_gen(gen_before, tokens_this_turn, ttft_ms, t_turn)),
                );
            }
            true
        };
        let res = crate::syn_chat::daemon::generate(
            &model.id,
            crate::syn_chat::daemon::to_service(&for_prompt, &media_at),
            tool_schemas.clone(),
            &call,
            stop,
            &mut on_event,
        );
        // Финальный flush — гарантированно сбрасываем хвост буферов.
        flush_streaming(
            &chat_id,
            &mut buf_body,
            &mut buf_think,
            &mut buf_tool,
            Some(live_gen(gen_before, tokens_this_turn, ttft_ms, t_turn)),
        );
        // Остановлено пользователем (в том числе посреди префилла): не сбой.
        if abort.load(Ordering::Relaxed) != abort_snapshot {
            return Ok(());
        }
        let gen = match res {
            Ok(g) => g,
            Err(_) if looped.is_some() => synaptixd::Generated::default(),
            Err(e) => {
                log::error!("[syn_chat] генерация оборвалась: {e:#}");
                return Err(e);
            }
        };
        if let Some(period) = looped {
            // Ход отброшен целиком: в историю и ленту он не попал, так что
            // префикс-KV цел, а повтор идёт с заметкой и анти-loop
            // температурой — greedy из такой петли сам не выходит.
            log::warn!(
                "[syn_chat] генерация зациклилась: фрагмент из {period} симв. повторяется \
                 по кругу, ход отброшен ({tokens_this_turn} ток. за {:?}){}",
                t_turn.elapsed(),
                if loop_retried { " — второй подряд, останавливаемся" } else { "; повторяем" }
            );
            if_active(&chat_id, |c| {
                c.streaming_body.set(String::new());
                c.streaming_thinking.set(String::new());
                c.streaming_tool.set(String::new());
            });
            if loop_retried {
                stop_reason = Some(format!(
                    "Остановлено: модель зацикливается в генерации (фрагмент из {period} \
                     символов повторяется по кругу) второй ход подряд. Переформулируйте \
                     задачу или поднимите температуру."
                ));
                ledger_update(&chat_id, pop_empty_assistant);
                break 'agent;
            }
            loop_retried = true;
            anti_loop = true;
            pending_note = Some(LOOP_NOTE);
            continue 'agent;
        }
        loop_retried = false;
        let turn_decode_s = gen.decode_ms as f64 / 1000.0;
        let tokens_this_turn = gen.tokens as u32;
        let channel_mode = ChannelIds::detect(&model.tokenizer).is_some();
        // Имя приводим к ключу каталога сразу: дальше оно идёт и в историю, и
        // в guard, и в политику подтверждений, и в UI.
        let raw_calls: Vec<RawToolCall> = gen
            .tool_calls
            .iter()
            .map(|c| RawToolCall {
                name: crate::agent::tools::executor::canonical_tool_name(&c.name).to_string(),
                arguments_json: c.arguments.clone(),
            })
            .collect();
        reused_total = reused_total.max(gen.cached_tokens as u32);
        let last_prompt_tokens = gen.prompt_tokens as u32;
        log::info!(
            "[syn_chat] turn={} {} tokens, prefill {} ms, декод {} мс (префикс-KV переиспользовал {} из {} ток промпта), tool_calls={}",
            turn,
            gen.tokens,
            gen.prefill_ms,
            gen.decode_ms,
            gen.cached_tokens,
            gen.prompt_tokens,
            raw_calls.len()
        );
        let stat = TurnStats {
            prompt_tokens: last_prompt_tokens,
            reused_tokens: gen.cached_tokens as u32,
            gen_tokens: total_gen_tokens + tokens_this_turn,
            prefill_ms: gen.prefill_ms as u32,
            turns: turn as u32 + 1,
            ring_tokens: gen.context as u32,
            ring_bytes: 0,
            ctx_budget: gen.context as u32,
            vram_free_mb: 0,
            blocks_resident: None,
        };
        if_active(&chat_id, move |c| stat.apply(c));
        let turn_ctx_budget = gen.context;
        let clean_text = gen.text;
        let think_text = gen.thinking;

        total_gen_tokens += tokens_this_turn;
        total_decode_s += turn_decode_s;

        // Если abort за стримом — выходим.
        if abort.load(Ordering::Relaxed) != abort_snapshot {
            return Ok(());
        }

        if raw_calls.is_empty() {
            if clean_text.trim().is_empty() {
                // Одна повторная попытка: ход в историю не попал, поэтому
                // просто перезапускаем его. Второй пустой подряд — сдаёмся,
                // чтобы не жечь бюджет на молчание.
                log::warn!(
                    "[syn_chat] ход без вызовов и без текста ({tokens_this_turn} ток. \
                     ушли в reasoning); {}",
                    if empty_answer { "второй подряд — останавливаемся" } else { "повторяем" }
                );
                if empty_answer {
                    break;
                }
                empty_answer = true;
                // Без заметки повтор идентичен: та же история даёт тот же
                // префикс-KV и тот же вывод байт в байт — в логах это видно
                // как два хода по одинаковому числу токенов. Заметка меняет
                // контекст и заодно говорит, чего от модели ждали: чаще
                // всего сюда приводит `<tool_call>` с битым JSON, который
                // парсер не смог принять. Живёт один ход (см. `pending_note`).
                pending_note = Some(EMPTY_TURN_NOTE);
                continue;
            }
            // Обычный текстовый ответ — конец хода. Открытый чат доберёт
            // хвост стрима в финализации, фоновому его взять неоткуда:
            // стрим туда не шёл, текст есть только у воркера.
            commit_turn_text(&chat_id, clean_text.clone(), think_text.clone());
            empty_answer = false;
            answered = true;
            break;
        }

        // Tool-calls: коммитим накопленный текст в leading-bubble, дальше
        // создаём отдельные tool_call/tool_result bubble'ы.
        commit_turn_text(&chat_id, clean_text.clone(), think_text.clone());

        // History: реплика ассистента с вызовами в каноническом виде — том
        // же, что `build_history` соберёт из ленты на следующем сообщении.
        // Сырой текст модели сюда не идёт: её собственная расстановка
        // пробелов в JSON (или XML-стиль) в ленте не сохраняется, и промпт
        // следующего сообщения расходился бы с промптом хода — префикс-KV
        // диалога обнулялся на каждом сообщении. В канальном режиме сырой
        // текст содержит заголовки каналов, которые chat-шаблон припишет
        // заново, — там реплика пересобирается из тела и ATEM-блока вызовов.
        history.push(if channel_mode {
            Message::assistant(channel_parser::rebuild_turn_text(&clean_text, &raw_calls))
        } else {
            assistant_turn_message(&clean_text, &think_text, &raw_calls)
        });

        tool_calls_made += raw_calls.len();

        // Конвертируем RawToolCall → ChatToolCall (для UI и executor'а).
        // Id уникален на всю жизнь ленты: `turn` начинается с нуля на каждом
        // сообщении, и id `syn_call_0_0_…` повторялись между сообщениями —
        // сжатие сопоставляло результат с чужим вызовом того же id.
        let call_stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let chat_calls: Vec<ChatToolCall> = raw_calls
            .iter()
            .enumerate()
            .map(|(i, c)| ChatToolCall {
                id: format!("syn_call_{}_{}_{:x}", turn, i, call_stamp),
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
        ledger_update(&chat_id, move |m| {
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

        // Сколько окна остаётся под результаты инструментов этого хода:
        // честный потолок минус то, что уже занято промптом и ответом, минус
        // резерв на следующий шаг. Инструмент, который сам решает, сколько
        // отдать (`notes read` пачкой), режет ответ по этому числу и говорит
        // модели, что осталось; выхлоп остальных укладывает в это же число
        // `fit_for_prompt`. Guard живёт до конца хода и возвращает бюджет
        // родителя, если внутри крутился субагент.
        let _tool_budget = tools::budget::arm_turn(
            turn_ctx_budget,
            last_prompt_tokens as usize + tokens_this_turn as usize,
            tools::budget::model_counter(&model),
        );

        // Выполняем каждый tool: guard повторов → confirm → execute → push.
        for (ci, chat_call) in chat_calls.iter().enumerate() {
            if abort.load(Ordering::Relaxed) != abort_snapshot {
                cancel_unanswered(&chat_id, &chat_calls[ci..]);
                return Ok(());
            }

            // ── Guard: тот же инструмент с теми же аргументами.
            let key = call_key(chat_call);
            let total = {
                let n = call_seen.entry(key.clone()).or_insert(0);
                *n += 1;
                *n
            };
            consecutive = if last_call.as_deref() == Some(key.as_str()) {
                consecutive + 1
            } else {
                1
            };
            remember_call(&mut recent_keys, &key);
            remember_call(&mut recent_shapes, &call_shape(chat_call));
            let periods = alternation_periods(&recent_keys);
            let shape_periods = alternation_periods(&recent_shapes);
            // Сколько раз подряд этот же вызов уже вернул тот же результат.
            let stagnant = call_outcomes.get(&key).map(|(_, n)| *n).unwrap_or(0);
            // То же, но вызов отличается от прежних только числами (без чисел
            // в аргументах это тот же счётчик, что выше — не считаем дважды).
            let masked = numeric_masked_key(chat_call);
            let numeric_stagnant =
                if masked != key { numeric_outcomes.get(&masked).map(|(_, n)| *n).unwrap_or(0) } else { 0 };
            last_call = Some(key.clone());
            // Новый вызов — петля разорвана, возвращаем сэмплинг пользователя.
            if consecutive < REPEAT_WARN_AT
                && periods < ALTERNATION_WARN_AT
                && shape_periods < SHAPE_ALTERNATION_ANTI_LOOP_AT
                && stagnant == 0
                && numeric_stagnant == 0
                && total < REPEAT_HINT_AT
            {
                anti_loop = false;
            }
            if consecutive >= REPEAT_STOP_AT
                || periods >= ALTERNATION_STOP_AT
                || stagnant >= STAGNANT_STOP_AT
                || numeric_stagnant >= NUMERIC_STAGNANT_STOP_AT
                || total >= TOTAL_STOP_AT
            {
                let text = if consecutive >= REPEAT_STOP_AT {
                    format!(
                        "Остановлено: инструмент `{}` вызван с теми же аргументами \
                         {consecutive}-й раз подряд — агент ходит по кругу.",
                        tool_name(chat_call)
                    )
                } else if periods >= ALTERNATION_STOP_AT {
                    format!(
                        "Остановлено: агент {periods} раза подряд чередует одни и те же \
                         два вызова (последний — `{}`) — ходит по кругу.",
                        tool_name(chat_call)
                    )
                } else if stagnant >= STAGNANT_STOP_AT {
                    format!(
                        "Остановлено: `{}` с этими аргументами {stagnant} раза подряд \
                         вернул один и тот же результат — между вызовами не меняется \
                         ничего, агент ходит по кругу.",
                        tool_name(chat_call)
                    )
                } else if numeric_stagnant >= NUMERIC_STAGNANT_STOP_AT {
                    format!(
                        "Остановлено: `{}` вызывается с теми же аргументами, меняются \
                         только числа, а результат по сути один и тот же — агент \
                         подбирает числа по кругу.",
                        tool_name(chat_call)
                    )
                } else {
                    format!(
                        "Остановлено: `{}` с этими аргументами вызван {total}-й раз за \
                         ход — агент ходит по кругу.",
                        tool_name(chat_call)
                    )
                };
                log::warn!("[syn_chat] guard повторов: {text}");
                push_tool_result(&chat_id, chat_call, text.clone(), true);
                history.push(Message::tool_named(tool_name(chat_call), text.clone()));
                stop_reason = Some(text);
                break 'agent;
            }
            if consecutive >= REPEAT_WARN_AT
                || periods >= ALTERNATION_WARN_AT
                || stagnant >= STAGNANT_WARN_AT
                || numeric_stagnant >= NUMERIC_STAGNANT_WARN_AT
            {
                let text = if consecutive >= REPEAT_WARN_AT {
                    format!(
                        "Вызов `{}` с этими аргументами только что выполнялся — между \
                         двумя одинаковыми вызовами подряд ничего не произошло, \
                         результат выше и он не изменится, поэтому повторно он не \
                         исполнен. Смени подход: другая команда, другой путь, другой \
                         инструмент — либо дай текстовый ответ по тому, что уже \
                         известно.",
                        tool_name(chat_call)
                    )
                } else if periods >= ALTERNATION_WARN_AT {
                    format!(
                        "Вызов `{}` с этими аргументами уже чередуется с предыдущим \
                         второй раз подряд — результаты обоих выше и не меняются, \
                         поэтому повторно он не исполнен. Смени подход: другая \
                         команда, другой путь, другой инструмент — либо дай \
                         текстовый ответ по тому, что уже известно.",
                        tool_name(chat_call)
                    )
                } else if stagnant >= STAGNANT_WARN_AT {
                    format!(
                        "Вызов `{}` с этими аргументами уже {} раза вернул один и тот же \
                         результат (он выше) — повторно он не исполнен. Если нужного \
                         эффекта нет, дело не в повторе: перечитай результат, проверь \
                         текущее состояние другим действием и смени подход — либо дай \
                         текстовый ответ по тому, что уже известно.",
                        tool_name(chat_call),
                        stagnant + 1
                    )
                } else {
                    format!(
                        "Вызов `{}` с теми же аргументами, кроме чисел, уже {} раза вернул \
                         по сути один и тот же результат (он выше) — подбор чисел ничего не \
                         меняет, поэтому этот вызов не исполнен. Перечитай последний \
                         результат целиком: что именно в нём не так? Проверь состояние \
                         другим действием и смени подход — либо дай текстовый ответ по \
                         тому, что уже известно.",
                        tool_name(chat_call),
                        numeric_stagnant + 1
                    )
                };
                log::warn!(
                    "[syn_chat] guard повторов: `{}` {}, вызов пропущен",
                    tool_name(chat_call),
                    if consecutive >= REPEAT_WARN_AT {
                        "повторён"
                    } else if periods >= ALTERNATION_WARN_AT {
                        "чередуется"
                    } else if stagnant >= STAGNANT_WARN_AT {
                        "возвращает тот же результат"
                    } else {
                        "меняет только числа при том же результате"
                    }
                );
                // Пропущенный вызов тоже в счёт: придёт с ним снова — стоп.
                if numeric_stagnant >= NUMERIC_STAGNANT_WARN_AT {
                    if let Some(entry) = numeric_outcomes.get_mut(&masked) {
                        entry.1 += 1;
                    }
                }
                anti_loop = true;
                push_tool_result(&chat_id, chat_call, text.clone(), true);
                history.push(Message::tool_named(tool_name(chat_call), text));
                continue;
            }
            // Вызов исполняем, но петля уже просматривается: тот же вызов
            // не в первый раз за ход, либо агент по кругу делает одно и то
            // же над новыми объектами. Greedy-декод из такого не выходит —
            // поднимаем температуру, не трогая сам вызов.
            if total >= REPEAT_HINT_AT || shape_periods >= SHAPE_ALTERNATION_ANTI_LOOP_AT {
                anti_loop = true;
            }

            let decision = crate::agent::tool_flow::await_decision_on_tool_call(
                chat_call,
                chat_id.as_deref().unwrap_or_default(),
                &abort,
                abort_snapshot,
            )
            .await;
            match decision {
                ToolDecision::Cancel => {
                    push_tool_result(
                        &chat_id,
                        chat_call,
                        tr!("chat.session.tool.cancelled"),
                        true,
                    );
                    history.push(Message::tool_named(
                        tool_name(chat_call),
                        "Отменено пользователем",
                    ));
                    // После Cancel прерываем весь loop — пользователь явно
                    // отказал, нет смысла продолжать.
                    cancel_unanswered(&chat_id, &chat_calls[ci + 1..]);
                    return Ok(());
                }
                ToolDecision::AllowAll => {
                    if let Some(chat) = chat_id.clone() {
                        run_on_main_thread(move || {
                            use_context::<AppCtx>().tools.allow_all_chats.update(|s| {
                                s.insert(chat);
                            });
                        });
                    }
                    // Падаем в Allow-ветку.
                }
                ToolDecision::Allow => {}
            }

            // ── pipelines action=run — особый путь: прогон завязан на
            // жизненный цикл LLM (free_vram выгружает её и грузит обратно),
            // поэтому исполняется обёрткой цикла, а не tools::execute.
            if let Some(req) = crate::syn_chat::pipeline_run::parse_run_call(chat_call) {
                let res = run_pipeline_tool(
                    req,
                    model,
                    &abort,
                    abort_snapshot,
                )
                .await;
                let turns_left = max_turns.saturating_sub(turn + 1);
                let note = if turns_left > 0 && turns_left <= system_prompt::BUDGET_NOTE_FROM {
                    system_prompt::budget_note(turns_left)
                } else {
                    String::new()
                };
                // В ленту — та же копия, что уйдёт в историю (см. ниже,
                // `fit_for_prompt`).
                let res = PipelineToolResult {
                    content: fit_for_prompt(&res.content, &tool_name(chat_call)),
                    ..res
                };
                push_tool_result_with(
                    &chat_id,
                    chat_call,
                    res.content.clone(),
                    res.error,
                    res.attachments,
                    note.clone(),
                );
                if res.aborted {
                    cancel_unanswered(&chat_id, &chat_calls[ci + 1..]);
                    return Ok(());
                }
                match res.model {
                    Some(m) => model = m,
                    None => {
                        // Результаты прогона уже в ленте — падаем с внятной
                        // ошибкой хода, чат остаётся без модели не молча.
                        let reason = res
                            .reload_error
                            .unwrap_or_else(|| tr!("chat.session.error.unknown"));
                        anyhow::bail!(tr!("chat.session.error.reload_failed", reason = reason));
                    }
                }
                note_outcome(&mut call_outcomes, &key, &res.content);
                if masked != key {
                    note_outcome(&mut numeric_outcomes, &masked, &res.content);
                }
                tools::budget::spend(&res.content);
                let mut for_history = res.content;
                for_history.push_str(&note);
                history.push(Message::tool_named(tool_name(chat_call), for_history));
                continue;
            }

            // ── view_media: файл кодирует vision-башня этой же модели, и его
            // эмбеддинги входят в промпт следующего хода — поэтому исполняет
            // цикл, а не `tools::execute` (у того нет ни модели, ни медиа).
            let mut viewed: Option<ViewedMedia> = None;
            let outcome = if tools::executor::canonical_tool_name(&tool_name(chat_call))
                == KEY_VIEW_MEDIA
            {
                let v = view_media_call(chat_call, &caps).await;
                let outcome = v.outcome.clone();
                viewed = Some(v);
                outcome
            } else {
                // Предел времени вызова — карточке на обратный отсчёт;
                // гасится, как бы вызов ни кончился (guard).
                let _countdown = ToolCountdown::start(
                    &chat_id,
                    tool_name(chat_call),
                    tools::executor::timeout_for(
                        &tool_name(chat_call),
                        chat_call.function.arguments.as_deref().unwrap_or(""),
                    ),
                );
                // Исполнение с возможностью прерывания на длинных tool'ах
                // (web fetch может висеть 30+ сек).
                let outcome = tokio::select! {
                    o = tools::execute(chat_call) => o,
                    _ = wait_abort(&abort, abort_snapshot) => {
                        cancel_unanswered(&chat_id, &chat_calls[ci..]);
                        return Ok(());
                    }
                };
                outcome
            };

            // Заметки для модели к результату. В ленту они уходят скрытым
            // полем `model_note`, а не в тело: так история, собранная из
            // ленты на следующем сообщении, совпадает с историей хода.
            let mut note = String::new();
            // Результат учтён — со следующего вызова guard знает, изменилось
            // ли что-нибудь (см. [`STAGNANT_WARN_AT`]).
            note_outcome(&mut call_outcomes, &key, &outcome.content);
            if masked != key {
                note_outcome(&mut numeric_outcomes, &masked, &outcome.content);
            }
            if total >= REPEAT_HINT_AT {
                // Не подряд — исполняем (после правки файла та же команда
                // сборки законна), но если результат не меняется, модель
                // должна это заметить сама.
                note.push_str(&format!(
                    "\n\n[Системная заметка: этот вызов с теми же аргументами \
                     сделан {total}-й раз за ход. Если результат не меняется — \
                     меняй подход, а не повторяй.]"
                ));
            }
            // Заметку про остаток ходов дописываем к результату, а не в
            // system: system лежит в голове контекста и держит префикс-KV,
            // правка на каждом ходу обнуляла бы его целиком.
            let turns_left = max_turns.saturating_sub(turn + 1);
            if turns_left > 0 && turns_left <= system_prompt::BUDGET_NOTE_FROM {
                note.push_str(&system_prompt::budget_note(turns_left));
            }
            // Выхлоп укладывается в остаток окна по токенам (`fit_for_prompt`)
            // — и в ленту, и в историю уходит одна и та же копия: история,
            // пересобранная из ленты на следующем сообщении, обязана совпасть
            // с промптом хода байт в байт, иначе префикс-KV обнулится.
            // Исключения — `autoskill`, `autotools` и `notes` (см. `history_limit`).
            let content = fit_for_prompt(&outcome.content, &tool_name(chat_call));
            let mut for_history = format!("{content}{note}");
            match viewed {
                Some(v) => {
                    // Блоки файлов — перед телом, тем же сборщиком, что и
                    // пересборка из ленты (`prepare_tool_view`): промпт
                    // следующего сообщения обязан совпасть с промптом хода.
                    let items: Vec<(&MsgAttachment, &str)> = v
                        .attachments
                        .iter()
                        .zip(&v.parts)
                        .map(|(a, p)| (a, p.as_str()))
                        .collect();
                    for_history = attach_prompt::assemble_tool_view(&for_history, &items);
                    if !v.media.is_empty() {
                        media_at.insert(history.len(), v.media);
                    }
                    push_tool_result_viewed(
                        &chat_id,
                        chat_call,
                        content,
                        outcome.error,
                        v.attachments,
                        note.clone(),
                    );
                }
                None => push_tool_result_with(
                    &chat_id,
                    chat_call,
                    content,
                    outcome.error,
                    Vec::new(),
                    note.clone(),
                ),
            }
            // Результат уже уехал в историю — следующий вызов этого же хода
            // получит окно на его размер меньше.
            tools::budget::spend(&for_history);
            history.push(Message::tool_named(tool_name(chat_call), for_history));
            if !outcome.error && tool_name(chat_call) == tools::catalog::KEY_WIZARD {
                awaiting_user = true;
            }

            // Серия вызовов, не дошедших до исполнения: модель не может
            // собрать вызов — дальше только сгорает бюджет ходов.
            invalid_streak = if outcome.invalid_args { invalid_streak + 1 } else { 0 };
            if invalid_streak >= INVALID_ARGS_STOP_AT {
                let text = format!(
                    "Остановлено: {invalid_streak} вызова подряд не прошли разбор \
                     аргументов (последний — `{}`: {}). Агент не может собрать \
                     вызов инструмента.",
                    tool_name(chat_call),
                    outcome.content.lines().next().unwrap_or("").trim()
                );
                log::warn!("[syn_chat] guard аргументов: {text}");
                stop_reason = Some(text);
                break 'agent;
            }
        }

        // ── Компактификация ВНУТРИ хода. Межходовой автокомпакт тут
        // бессилен: он сжимает только то, что до последнего сообщения
        // пользователя, а tool-цепочка агента растёт после него — в чатах
        // это давало рост промпта с 25k до 70k+ символов за одно сообщение.
        if autocompact_enabled && media_at.is_empty() && turn_ctx_budget > 0 {
            let prompt_tokens = last_prompt_tokens;
            let pct = prompt_tokens as u64 * 100 / turn_ctx_budget as u64;
            if pct >= autocompact_threshold as u64 {
                let compacted = crate::syn_chat::compact::maybe_compact_in_turn(
                    &model,
                    &abort,
                    abort_snapshot,
                    autocompact_threshold,
                    IN_TURN_KEEP_TAIL,
                    prompt_tokens,
                    turn_ctx_budget as u32,
                    chat_id.clone(),
                )
                .await;
                if compacted {
                    match rebuild_history(&ctx, &sys_prompt, &model, &caps, &chat_id).await {
                        Some((h, m)) => {
                            media_at = m;
                            log::info!(
                                "[syn_chat] история хода пересобрана после сжатия: \
                                 {} сообщений",
                                h.len()
                            );
                            history = h;
                        }
                        None => log::warn!(
                            "[syn_chat] не удалось пересобрать историю после сжатия — \
                             продолжаем на прежней"
                        ),
                    }
                }
            }
        }

        // Готовим placeholder для следующего turn (UI bubble — пустой
        // assistant, который заполнится stream'ом).
        if awaiting_user {
            log::info!("[syn_chat] wizard: вопрос показан пользователю — ход завершён, ждём ответ");
            answered = true;
            break 'agent;
        }
        ledger_update(&chat_id, |m| m.push(ChatMsg::assistant_empty()));
        if_active(&chat_id, |c| {
            c.streaming_body.set(String::new());
            c.streaming_thinking.set(String::new());
            c.streaming_tool.set(String::new());
        });
    }

    if let Some(reason) = stop_reason {
        // Плейсхолдер этого хода уже заменён tool_call-пузырём, чистить
        // нечего — но кнопка «Продолжить» нужна: пользователь может дать
        // агенту ещё попытку, уже с заметками guard'а в контексте.
        if_active(&chat_id, move |c| {
            c.turn_cap_reached.set(true);
            c.error.set(Some(format!(
                "{reason}{}",
                tr!("chat.session.error.stopped_suffix")
            )));
        });
    } else if empty_answer {
        // Пустой пузырь этого хода убираем — он выглядит как «повисло».
        ledger_update(&chat_id, pop_empty_assistant);
        // «Продолжить» отдаёт агенту ещё ход с целой историей — ровно то,
        // что здесь нужно.
        if_active(&chat_id, |c| {
            c.turn_cap_reached.set(true);
            c.error.set(Some(tr!("chat.session.error.empty_answer")));
        });
    } else if !answered {
        log::warn!(
            "[syn_chat] agent-loop упёрся в лимит {max_turns} ходов: модель всё \
             это время вызывала инструменты и ни разу не дала текстовый ответ. \
             Генерация остановлена, история цела — можно продолжить."
        );
        // Убираем пустой assistant-плейсхолдер последнего хода — иначе в
        // ленте висит пустой пузырь, который выглядит как «повисло».
        ledger_update(&chat_id, pop_empty_assistant);
        if_active(&chat_id, move |c| {
            c.turn_cap_reached.set(true);
            c.error.set(Some(tr!(
                "chat.session.error.turn_cap_reached",
                max_turns = max_turns
            )));
        });
    } else if !tool_schemas.is_empty() && tool_calls_made == 0 {
        // Ход закончился текстом, но агент не тронул ни один инструмент,
        // хотя они были включены. Чаще всего это заявленное намерение
        // («сейчас посмотрю проект») без действия — история цела, и одного
        // нажатия «Продолжить» хватает, чтобы агент довёл дело до конца.
        // Ошибку не поднимаем: ответ мог быть и полным по существу.
        log::info!(
            "[syn_chat] ход завершён текстом без единого вызова при {} \
             активных инструментах — предлагаем «Продолжить»",
            tool_schemas.len()
        );
        if_active(&chat_id, |c| c.turn_cap_reached.set(true));
    }

    // Финальная статистика.
    let dt_total = t_overall.elapsed();
    let final_tps = if total_decode_s > 0.0 {
        (total_gen_tokens as f64 / total_decode_s) as f32
    } else {
        0.0
    };
    log::info!(
        "[syn_chat] генерация завершена: {total_gen_tokens} ток за {dt_total:?} \
         ({total_decode_s:.1} с чистого декода, {final_tps:.1} tok/s)"
    );
    let final_gen = total_gen_tokens;
    let ctx_final = ctx.clone();
    run_on_main_thread(move || {
        ctx_final.last_gen_tokens.set_always(final_gen);
        ctx_final.last_decode_tps.set_always(final_tps);
    });

    Ok(())
}

/// Одноразовая plain-генерация вне agent-loop: без tools, без thinking, без
/// префикс-KV и без стрима в UI. Используется компактификацией
/// (`syn_chat::compact`) для summary-запроса. Блокирует вызывающий поток на
/// время генерации — звать только из worker-потока.
pub(crate) fn generate_summary(
    model: &Arc<LoadedSynModel>,
    system: &str,
    user: &str,
    abort: &Arc<AtomicU64>,
    abort_snapshot: u64,
) -> anyhow::Result<String> {
    let history = vec![Message::system(system), Message::user(user)];
    // Низкая температура: сводка должна быть детерминированной и сухой.
    let mut params = SamplingParams::default();
    params.temperature = 0.3;
    params.enable_thinking = false;
    params.max_new_tokens = SUMMARY_MAX_NEW_TOKENS as u32;
    let call = crate::syn_chat::daemon::params(&params, None);
    let stop: Arc<dyn Fn() -> bool + Send + Sync> = {
        let abort = abort.clone();
        Arc::new(move || abort.load(Ordering::Relaxed) != abort_snapshot)
    };
    let g = crate::syn_chat::daemon::complete(&model.id, &history, &call, stop)?;
    log::info!("[syn_chat] сводка: промпт {} ток, ответ {} ток", g.prompt_tokens, g.tokens);
    Ok(g.text.trim().to_string())
}

/// Считает живые счётчики хода: сумма токенов по всей генерации и скорость
/// декода за вычетом префилла (иначе долгий первый токен занижает tps в
/// разы).
fn live_gen(gen_before: u32, tokens_this_turn: u32, ttft_ms: Option<u32>, t_turn: Instant) -> LiveGen {
    let decode_s = t_turn.elapsed().as_secs_f64() - ttft_ms.unwrap_or(0) as f64 / 1000.0;
    let tps = if decode_s > 0.0 {
        (tokens_this_turn as f64 / decode_s) as f32
    } else {
        0.0
    };
    LiveGen {
        gen_tokens: gen_before + tokens_this_turn,
        tps,
    }
}

/// Живые счётчики генерации для таба «Детали»: сколько токенов выдано за
/// всю генерацию (не за один ход) и текущая скорость декода. Без них
/// панель до конца хода стоит на числах прошлого — на длинном ответе это
/// выглядит как зависший чат.
#[derive(Clone, Copy)]
struct LiveGen {
    gen_tokens: u32,
    tps: f32,
}

/// Сбрасывает накопленные body/think/tool буферы в реактивные сигналы UI.
/// Передавать `live = None` если статистику обновлять не надо (например,
/// на abort-сбросе).
/// Живой стрим — только в открытый чат: ход, ушедший в фон, догонит ленту
/// целиком в [`commit_turn_text`], а рисовать его в чужом чате нельзя.
fn flush_streaming(
    chat_id: &Option<String>,
    buf_body: &mut String,
    buf_think: &mut String,
    buf_tool: &mut String,
    live: Option<LiveGen>,
) {
    if buf_body.is_empty() && buf_think.is_empty() && buf_tool.is_empty() && live.is_none() {
        return;
    }
    let b = std::mem::take(buf_body);
    let t = std::mem::take(buf_think);
    let tc = std::mem::take(buf_tool);
    if_active(chat_id, move |ctx| {
        if !t.is_empty() {
            ctx.streaming_thinking.update(|s| s.push_str(&t));
        }
        if !b.is_empty() {
            ctx.streaming_body.update(|s| s.push_str(&b));
        }
        if !tc.is_empty() {
            ctx.streaming_tool.update(|s| s.push_str(&tc));
        }
        if let Some(live) = live {
            ctx.last_gen_tokens.set_always(live.gen_tokens);
            ctx.last_decode_tps.set_always(live.tps);
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
/// Ключ вызова для guard'а повторов: имя + канонизированные аргументы.
/// Канонизация нужна, чтобы `{"a":1}` и `{ "a": 1 }` считались одним и тем
/// же вызовом — модель переформатирует JSON от хода к ходу.
fn call_key(call: &ChatToolCall) -> String {
    let args = call.function.arguments.as_deref().unwrap_or("").trim();
    let canonical = serde_json::from_str::<serde_json::Value>(args)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| args.to_string());
    format!("{}\u{1f}{}", tool_name(call), canonical)
}

/// Ключ вызова с числами под маской: `{"y":160}` и `{"y":920}` — один ключ,
/// а разные строки (`find`, заголовки, id страниц) — разные. Для guard'а
/// петли, в которой модель подбирает координаты при том же результате.
///
/// Числа в аргументах-адресах ([`ADDRESS_ARGS`]: номер блока, позиция в
/// документе, id) остаются как есть — это не величина, а другой объект.
/// Под маской перенос блока #24 и блока #14 сливался в один ключ, ответы у
/// них разные, и счётчик сбрасывался на каждой смене блока: пинг-понг трёх
/// блоков по y (14.09.2026, MyLife, «Долги и кредиты») guard не видел.
fn numeric_masked_key(call: &ChatToolCall) -> String {
    fn mask_text(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_number = false;
        for ch in s.chars() {
            if ch.is_ascii_digit() || (in_number && ch == '.') {
                if !in_number {
                    out.push('#');
                    in_number = true;
                }
            } else {
                in_number = false;
                out.push(ch);
            }
        }
        out
    }
    fn mask(v: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Number(_) => Value::String("#".to_string()),
            Value::String(s) => Value::String(mask_text(s)),
            Value::Array(a) => Value::Array(a.iter().map(mask).collect()),
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, x)| (k.clone(), if ADDRESS_ARGS.contains(&k.as_str()) { x.clone() } else { mask(x) }))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    let args = call.function.arguments.as_deref().unwrap_or("").trim();
    match serde_json::from_str::<serde_json::Value>(args) {
        Ok(v) => format!("{}\u{1f}{}", tool_name(call), mask(&v)),
        Err(_) => mask_text(&call_key(call)),
    }
}

/// Аргументы, числа в которых — адрес объекта, а не подбираемая величина.
const ADDRESS_ARGS: &[&str] = &[
    "block", "blocks", "index", "after", "before", "into", "parent", "child", "from", "to", "page", "pages", "board",
    "card", "column", "task", "node", "event", "id",
];

/// «Форма» вызова: инструмент, дискриминанты действия (`action`, `op`, …) и
/// набор имён аргументов — без их значений.
///
/// Guard точных ключей видит петлю, только когда она повторяется байт в байт.
/// Половина реальных петель так не выглядит: агент создаёт страницу, кладёт на
/// неё доску, создаёт следующую страницу, кладёт доску на неё — второй вызов
/// каждый раз адресует свежий id, и `alternation_periods` по точным ключам
/// обрывается на первом же периоде. По форме такая пара совпадает.
fn call_shape(call: &ChatToolCall) -> String {
    let args = call.function.arguments.as_deref().unwrap_or("").trim();
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(args) else {
        // Аргументы не разобрались — формы у вызова нет, остаётся точный ключ.
        return call_key(call);
    };
    let mut shape = tool_name(call);
    for disc in ["action", "op", "mode", "kind", "type"] {
        if let Some(v) = map.get(disc).and_then(|v| v.as_str()) {
            shape.push('\u{1f}');
            shape.push_str(disc);
            shape.push('=');
            shape.push_str(v);
        }
    }
    let mut names: Vec<&str> = map.keys().map(String::as_str).collect();
    names.sort_unstable();
    shape.push('\u{1f}');
    shape.push_str(&names.join(","));
    shape
}

/// Отпечаток результата инструмента: хеш вывода, из которого выброшены
/// изменчивые части — числа и hex-идентификаторы.
///
/// Без этого «created … page: 7836e7842679 · "Туду — канбан 2"» и
/// «created … page: 84b0fe985a42 · "Туду — канбан 3"» — разные строки, хотя
/// произошло в них одно и то же. Слова с буквами вне hex-алфавита не трогаем:
/// `E0308` обязан отличаться от `E0277`, иначе две разные ошибки сборки
/// схлопнутся в одну и guard остановит агента, который на самом деле движется.
fn outcome_fingerprint(out: &str) -> u64 {
    fn is_hex_id(w: &str) -> bool {
        w.len() >= 6
            && w.bytes().any(|b| b.is_ascii_digit())
            && w.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    let mut norm = String::with_capacity(out.len());
    let mut word = String::new();
    let flush = |word: &mut String, norm: &mut String| {
        if !word.is_empty() {
            if word.bytes().all(|b| b.is_ascii_digit()) || is_hex_id(word) {
                norm.push('#');
            } else {
                norm.push_str(word);
            }
            word.clear();
        }
    };
    for c in out.chars() {
        if c.is_ascii_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut norm);
            norm.push(c);
        }
    }
    flush(&mut word, &mut norm);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&norm, &mut h);
    std::hash::Hasher::finish(&h)
}

/// Выхлоп инструмента, уложенный в остаток окна хода
/// ([`tools::budget::fit`]): что влезает — целиком, что нет — голова и хвост
/// по границам строк с пометкой, какие строки вырезаны. Мера — токены по
/// токенизатору модели; предел без живого окна и исключения — в
/// [`executor::history_limit`](crate::agent::tools::executor::history_limit).
///
/// Пометка называет причину («остаток контекста») и номера строк, чтобы
/// модель дочитала именно пропущенное одним вызовом, а не перечитывала всё:
/// прежний совет «уточните команду» на каждом куске файла уводил её по
/// кругу — куски по 250 строк резались до 8000 символов, и модель
/// запрашивала вырезанную середину снова и снова.
pub(crate) fn fit_for_prompt(s: &str, tool: &str) -> String {
    let Some(cap) = crate::agent::tools::executor::history_limit(tool) else {
        return s.to_string();
    };
    tools::budget::fit(s, cap, |c| {
        let window = if c.measured {
            format!(", в окне осталось ~{} токенов", c.left)
        } else {
            String::new()
        };
        format!(
            "…[вывод обрезан по остатку контекста: показаны строки 1–{} и {}–{} из {} \
             (~{} токенов при лимите ~{}{window}); пропущены строки {}–{} этого вывода — \
             если они нужны, запросите отдельным вызовом именно этот диапазон, не \
             перечитывая всё]…\n",
            c.omitted.0 - 1,
            c.omitted.1 + 1,
            c.lines,
            c.lines,
            c.tokens,
            c.allowed,
            c.omitted.0,
            c.omitted.1,
        )
    })
}

fn tool_name(call: &ChatToolCall) -> String {
    call.function.name.clone().unwrap_or_default()
}

/// Итог [`run_pipeline_tool`]. `model: None` — free_vram-прогон прошёл, но
/// LLM не перезагрузилась (подробность в `reload_error`); ход должен
/// завершиться ошибкой ПОСЛЕ пуша результатов прогона в ленту.
struct PipelineToolResult {
    model: Option<Arc<LoadedSynModel>>,
    content: String,
    error: bool,
    attachments: Vec<crate::agent::state::MsgAttachment>,
    reload_error: Option<String>,
    aborted: bool,
}

/// `pipelines action=run` — обёртка уровня agent-loop.
///
/// Принимает `model` ПО ЗНАЧЕНИЮ: при `free_vram` цикл отдаёт своё
/// владение, функция дропает последнюю strong-ссылку (плюс KV-слот — своим
/// guard'ом, т.к. `drop_kv_session()` из `unload()` при занятом мьютексе
/// лишь отложил бы дроп до конца хода) и после прогона возвращает свежую
/// модель из `load_with_notify`.
async fn run_pipeline_tool(
    req: crate::syn_chat::pipeline_run::RunRequest,
    model: Arc<LoadedSynModel>,
    abort: &Arc<AtomicU64>,
    abort_snapshot: u64,
) -> PipelineToolResult {
    use crate::syn_chat::pipeline_run as pr;

    let fail = |model: Option<Arc<LoadedSynModel>>, msg: String| PipelineToolResult {
        model,
        content: msg,
        error: true,
        attachments: Vec::new(),
        reload_error: None,
        aborted: false,
    };

    // 1. Валидация графа и автозаполнение путей save-нод — ДО выгрузки LLM:
    //    если запускать нечего, гонять модель туда-обратно незачем.
    let prepared = match pr::prepare(&req.run_label).await {
        Ok(p) => p,
        Err(e) => return fail(Some(model), e),
    };

    // 2. Видеопамять под прогон освобождает служба: модель чата живёт там
    //    же и уступает карту стадиям графа, а после возвращается сама.
    let model_opt = Some(model);
    let model_path: Option<std::path::PathBuf> = None;
    if req.free_vram {
        log::info!("[syn_chat] pipelines run: free_vram — карту делит служба");
    }

    // 3. Старт прогона.
    let started = pr::start().await;
    let (outcome_rx, started_n) = match started {
        Ok(v) => v,
        Err(e) => {
            let (model, reload_error) = reload_if_needed(model_opt, model_path).await;
            let mut r = fail(model, tr!("chat.session.pipeline.start_failed", error = e));
            r.reload_error = reload_error;
            return r;
        }
    };
    log::info!(
        "[syn_chat] pipelines run «{}»: стартовало корней {}, save-нод {}",
        req.run_label,
        started_n,
        prepared.planned.len()
    );

    // 4. Ожидание итога. Abort хода отменяет прогон (cancel-флаги нод).
    let mut aborted = false;
    let outcome = tokio::select! {
        o = outcome_rx => o.ok(),
        _ = wait_abort(abort, abort_snapshot) => {
            pr::cancel_current().await;
            aborted = true;
            None
        }
    };

    // 5. Артефакты: файлы save-нод → CAS-вложения (worker-поток, диск).
    let (mut attachments, mut artifact_lines) = pr::collect_artifacts(&prepared.planned);
    // 5b. Ноды-просмотрщики (Видео-плеер, Аудио-плеер) держат результат в
    //     памяти: без save-ноды в чат не приходило ничего, хотя результат
    //     посчитан. Материализуем и прикладываем — но не дублируем то, что
    //     уже пришло файлом.
    let (viewer_atts, viewer_lines) =
        pr::collect_viewer_outputs(&req.run_label, &attachments).await;
    attachments.extend(viewer_atts);
    artifact_lines.extend(viewer_lines);

    // 6. Вернуть LLM (и после abort тоже — чат не должен молча остаться
    //    без модели). Перед этим освобождаем VRAM от нодовых моделей
    //    прогона: LTX/H3 держат веса и пулы активаций, и на 24 ГБ LLM
    //    возвращалась в остаток — первый же forward падал с
    //    «alloc_zeros(...) after trim+retries: OOM».
    let was_freed = model_path.is_some();
    if was_freed {
        let (tx, rx) = tokio::sync::oneshot::channel::<(usize, u64, u64)>();
        run_on_main_thread(move || {
            let free_before = crate::models::cuda_free_mb();
            let n = crate::models::unload_all();
            crate::models::trim_all();
            let _ = tx.send((n, free_before, crate::models::cuda_free_mb()));
        });
        if let Ok((n, before, after)) = rx.await {
            log::info!(
                "[syn_chat] после прогона выгружено нодовых моделей: {n}; \
                 VRAM свободно {before} -> {after} MB"
            );
        }
    }
    let (model, reload_error) = reload_if_needed(model_opt, model_path).await;
    let llm_note = if was_freed {
        Some(match (&model, &reload_error) {
            (Some(_), _) => tr!("chat.session.pipeline.llm_reloaded"),
            (None, Some(e)) => tr!("chat.session.pipeline.llm_reload_failed", error = e),
            (None, None) => tr!("chat.session.pipeline.llm_reload_failed_unknown"),
        })
    } else {
        None
    };

    let (content, error) =
        pr::format_envelope(
            outcome.as_ref(),
            &artifact_lines,
            llm_note.as_deref(),
            aborted,
            &prepared.warnings,
        );
    PipelineToolResult {
        model,
        content,
        error,
        attachments,
        reload_error,
        aborted,
    }
}

/// Вернуть модель после free_vram-прогона. `existing` = Some — выгрузки не
/// было (free_vram=false), возвращаем как есть.
async fn reload_if_needed(
    existing: Option<Arc<LoadedSynModel>>,
    path: Option<std::path::PathBuf>,
) -> (Option<Arc<LoadedSynModel>>, Option<String>) {
    if existing.is_some() {
        return (existing, None);
    }
    let Some(path) = path else {
        return (None, Some(tr!("chat.session.error.model_path_unknown")));
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        use_context::<SynModelRegistry>().load_with_notify(path, tx);
    });
    match rx.await {
        Ok(Ok(m)) => (Some(m), None),
        Ok(Err(e)) => (None, Some(e)),
        Err(e) => (None, Some(e.to_string())),
    }
}

/// Пушит tool_result-бабл в ленту.
fn push_tool_result(ctx: &Option<String>, call: &ChatToolCall, content: String, error: bool) {
    push_tool_result_with(ctx, call, content, error, Vec::new(), String::new());
}

/// Как [`push_tool_result`], но с медиа-вложениями (результаты пайплайна:
/// mp4/wav из save-нод). В промпт они не попадают — отдельное сообщение
/// ленты `build_history` пропускает — а в ленте рендерятся плитками с
/// полноэкранным просмотрщиком.
///
/// `model_note` — дописка к телу только для модели (см. `ChatMsg::model_note`).
fn push_tool_result_with(
    chat_id: &Option<String>,
    call: &ChatToolCall,
    content: String,
    error: bool,
    attachments: Vec<crate::agent::state::MsgAttachment>,
    model_note: String,
) {
    let id = call.id.clone();
    let name = call.function.name.clone().unwrap_or_default();
    ledger_update(chat_id, move |m| {
        let mut msg = ChatMsg::tool_result(id, name, content, error);
        msg.model_note = model_note;
        m.push(msg);
        // Медиа — отдельным сообщением ленты, а не внутри карточки
        // инструмента: результат прогона смотрят как результат, а не как
        // приложение к техническому выводу (и карточка не схлопывается
        // вместе с ним). В историю для модели это сообщение не идёт —
        // она живёт отдельным списком.
        if !attachments.is_empty() {
            let mut media = ChatMsg::assistant_empty();
            media.attachments = attachments;
            m.push(media);
        }
    });
}

/// Результат `view_media`: файлы лежат на самой карточке инструмента, а не
/// отдельным сообщением, как у пайплайна, — это часть результата, из которой
/// `build_history` пересобирает промпт на следующем сообщении.
fn push_tool_result_viewed(
    chat_id: &Option<String>,
    call: &ChatToolCall,
    content: String,
    error: bool,
    attachments: Vec<MsgAttachment>,
    model_note: String,
) {
    let id = call.id.clone();
    let name = call.function.name.clone().unwrap_or_default();
    ledger_update(chat_id, move |m| {
        let mut msg = ChatMsg::tool_result(id, name, content, error);
        msg.model_note = model_note;
        msg.attachments = attachments;
        m.push(msg);
    });
}

/// Что `view_media` отдал циклу.
struct ViewedMedia {
    /// Тело результата (список файлов) — в ленту и после блоков в промпт.
    outcome: tools::ToolOutcome,
    /// Файлы, вошедшие в промпт (и те, у которых вместо содержимого стоит
    /// заглушка с причиной), — на карточку результата.
    attachments: Vec<MsgAttachment>,
    /// Их куски промпта в том же порядке (`attach_prompt::attachment_part`).
    parts: Vec<String>,
    /// Файлы картинок и видео среди них — на маркеры результата.
    media: Vec<synaptixd::Media>,
}

/// Исполняет `view_media`: кладёт файлы в CAS, как вложения, и готовит их
/// тем, что умеет модель, — картинки и видео её vision-башней, звук ASR,
/// документы текстом.
///
/// Башня поднимается только под то, чего нет в кэше эмбеддингов, и ложится
/// на ту же VRAM, что держит кэш префикс-KV хода: кэш на это время уезжает в
/// RAM (как в начале цикла). Файлы, которые не влезают в остаток окна,
/// в промпт не идут — модель узнаёт, сколько им было нужно.
async fn view_media_call(
    call: &ChatToolCall,
    caps: &MediaCaps,
) -> ViewedMedia {
    use crate::agent::tools::view_media::{self as vm, Entry, Status};

    let fail = |content: String, invalid_args: bool| ViewedMedia {
        outcome: tools::ToolOutcome {
            tool_call_id: call.id.clone(),
            name: KEY_VIEW_MEDIA.to_string(),
            content,
            error: true,
            invalid_args,
        },
        attachments: Vec::new(),
        parts: Vec::new(),
        media: Vec::new(),
    };
    let args = tools::executor::normalize_args(call.function.arguments.as_deref().unwrap_or(""));
    let paths = match vm::parse_paths(&args) {
        Ok(p) => p,
        Err(e) => return fail(e.to_string(), true),
    };

    // Хеширование, превью и ffmpeg у видео — секунды; не на потоке цикла.
    let ingested = tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|p| {
                let r = vm::check_file(&p).and_then(|()| crate::syn_chat::attach::ingest::ingest(&p));
                (p, r)
            })
            .collect::<Vec<_>>()
    })
    .await;
    let ingested = match ingested {
        Ok(v) => v,
        Err(e) => return fail(format!("view_media: reading files failed: {e}"), false),
    };

    // Половина остатка окна на весь результат — как у остальных
    // инструментов (`budget::grant`); вне цикла бюджета нет.
    let grant = tools::budget::grant(usize::MAX);
    let mut allowed = if grant.measured { grant.tokens } else { usize::MAX };
    let mut entries: Vec<Entry> = Vec::new();
    let mut attachments: Vec<MsgAttachment> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut media: Vec<synaptixd::Media> = Vec::new();
    for (path, r) in ingested {
        let a = match r {
            Ok(a) => a,
            Err(why) => {
                entries.push(Entry { path, status: Status::Failed(why) });
                continue;
            }
        };
        let part = attach_prompt::attachment_part(&a, caps);
        // Картинку кодирует служба: её доля окна — потолок токенов на
        // картинку (видео — с запасом на кадры).
        let tokens = match &part.media {
            Some(m) if m.kind == synaptixd::MediaKind::Video => 8192,
            Some(_) => caps.max_image_tokens.unwrap_or(1024),
            None => tools::budget::count(&part.text),
        };
        if tokens > allowed {
            entries.push(Entry { path, status: Status::Skipped { attachment: a, need: tokens, allowed } });
            continue;
        }
        allowed = allowed.saturating_sub(tokens);
        attachments.push(a.clone());
        parts.push(part.text);
        media.extend(part.media);
        entries.push(Entry {
            path,
            status: Status::Included { index: attachments.len(), attachment: a, tokens, failure: part.failure },
        });
    }
    let shown = entries.iter().filter(|e| e.shown()).count();
    log::info!(
        "[syn_chat] view_media: показано {shown} из {} файлов, медиа {}",
        entries.len(),
        media.len()
    );
    ViewedMedia {
        outcome: tools::ToolOutcome {
            tool_call_id: call.id.clone(),
            name: KEY_VIEW_MEDIA.to_string(),
            content: vm::summary(&entries),
            error: shown == 0,
            invalid_args: false,
        },
        attachments,
        parts,
        media,
    }
}

/// Лейблы тех ключей, что уходят модели схемой (для системного промпта — в
/// том же порядке, в каком уходят схемы): инструменты каталога, кроме
/// неявных. Ключ вне каталога схемы не получает, и в строке «available
/// tools» его быть не должно — 13.09.2026 `web_read`/`web_search` из
/// конфига до слияния в `web` попадали туда именами, и модель при пустых
/// «Активных» перечисляла их как объявленные.
fn declared_tool_labels(keys: &[String]) -> Vec<String> {
    keys.iter()
        .filter_map(|k| Tool::by_key(k))
        .filter(|t| !t.is_implicit())
        .map(|t| t.label.to_string())
        .collect()
}

/// Лейблы инструментов из пула `autotools` — для строки про пул в системном
/// промпте.
fn pool_tool_labels(settings: &ChatSettings) -> Vec<String> {
    tools::autotools::pool(&settings.tools_active, &settings.tools_auto)
        .iter()
        .map(|t| t.label.to_string())
        .collect()
}

/// Собирает JSON-схемы активных инструментов (для передачи в Jinja-шаблон
/// Qwen3 или manual prefix). Особые случаи — `autoskill`, описание которого
/// расширяется актуальным списком скилов через [`build_autoskill_chat_tool`],
/// и пул: он уходит одной схемой `autotools` в конце списка, а схемы самих
/// инструментов из пула модель получает ответом `autotools`.
fn collect_active_tool_schemas(app: &AppCtx, settings: &ChatSettings) -> Vec<serde_json::Value> {
    use crate::agent::tools::catalog::KEY_AUTOSKILL;
    let mut schemas: Vec<serde_json::Value> = settings
        .tools_active
        .iter()
        .filter_map(|k| {
            let tool = Tool::by_key(k).filter(|t| !t.is_implicit())?;
            let t = if k == KEY_AUTOSKILL {
                crate::agent::tool_flow::build_autoskill_chat_tool(app, &settings.skills_active)
            } else {
                tool.to_chat_tool()
            };
            serde_json::to_value(&t).ok()
        })
        .collect();
    if let Some(t) = crate::agent::tool_flow::build_autotools_chat_tool(settings) {
        schemas.extend(serde_json::to_value(&t).ok());
    }
    schemas
}


/// Реплика ленты в форме, пригодной для сборки промпта на worker-потоке:
/// сигналы уже прочитаны, вложения — по значению.
/// Блок фрагментов из активных баз знаний под последнее сообщение
/// пользователя, если включено «Подмешивать найденное» (`kb.auto_augment`) и
/// в чате выбрана хоть одна коллекция. Ошибка поиска хода не валит — ход
/// идёт без блока, модель может сама вызвать `kb_search`.
async fn kb_augment_block(items: &[HistoryItem]) -> Option<String> {
    let query = items
        .iter()
        .rev()
        .find(|i| i.role == ChatMsgRole::User && i.tool_name.is_none())
        .map(|i| i.body.trim().to_string())
        .filter(|q| !q.is_empty())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let kb = use_context::<AppCtx>().kb;
        let on = kb.auto_augment.get_untracked() && !kb.active_collection_ids().is_empty();
        let _ = tx.send(on);
    });
    if !rx.await.unwrap_or(false) {
        return None;
    }
    let cfg = crate::config::AppConfig::load().kb;
    match crate::kb::augment::compute(&query, cfg.default_top_k.max(1), cfg.augment_token_budget.max(64)).await {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(e) => {
            log::warn!("kb augment: {e}");
            None
        }
    }
}

struct HistoryItem {
    role: ChatMsgRole,
    body: String,
    attachments: Vec<MsgAttachment>,
    /// `Some(имя)` — результат инструмента: в prompt уходит как `role=tool`
    /// (`Message::tool_named`), поле `role` при этом не используется.
    tool_name: Option<String>,
    /// Размышления и вызовы реплики ассистента с инструментами
    /// (см. [`assistant_turn_message`]); у остальных реплик пусты.
    reasoning: String,
    calls: Vec<RawToolCall>,
}

/// Снимок возможностей модели и окружения по части вложений. Читает
/// сигналы, поэтому вызывается только с main thread.
///
/// `max_seq_len` — окно чата из параметров: документ-вложение получает
/// половину меньшего из него и окна модели, мерой служит токенизатор
/// модели. Половина — чтобы после документа оставалось место на переписку
/// и ответ; величина стабильна от хода к ходу (см. `DocBudget`).
fn snapshot_media_caps(app: &AppCtx, model: &Arc<LoadedSynModel>, max_seq_len: u32) -> MediaCaps {
    let max = app.syn_chat_max_image_tokens.get_untracked();
    let window = model.window.min(if max_seq_len > 0 { max_seq_len as usize } else { usize::MAX });
    MediaCaps {
        // Кэш, а не Llm::supports_media(): функция зовётся с main thread, а
        // мьютекс пайплайна занят на всё время идущей генерации.
        vision: model.supports_media,
        vision_error: None,
        max_image_tokens: (max > 0).then_some(max),
        doc_budget: Some(attach_prompt::DocBudget {
            tokens: (window / 2).max(1024),
            count: tools::budget::model_counter(model),
        }),
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
/// Реплика ассистента с вызовами для ChatML-шаблонов: текст, размышления и
/// структурные вызовы — шаблон сам рендерит `<think>`-блок и вызовы в своём
/// родном формате (у Qwen3.8 — `<function=…><parameter=…>`). Одна функция на
/// оба пути (ход агента и пересборка из ленты), иначе промпт следующего
/// сообщения расходится с промптом хода.
///
/// Размышления обязательны: промпт хода заканчивается `…assistant⏎<think>⏎`,
/// и без `reasoning_content` шаблон рендерит `<think>⏎⏎</think>` — токен `⏎⏎`
/// не равен `⏎`, префикс-KV Qwen4Exp (точка возврата на конце промпта) терял
/// всё, гибрид — ходы, где граница попадала в хвост (04.09.2026).
pub(crate) fn assistant_turn_message(prose: &str, thinking: &str, calls: &[RawToolCall]) -> Message {
    let reasoning = thinking.trim_matches('\n');
    Message::assistant_turn(
        prose.trim(),
        (!reasoning.trim().is_empty()).then(|| reasoning.to_string()),
        calls
            .iter()
            .map(|c| (c.name.clone(), c.arguments_json.clone()))
            .collect(),
    )
}

/// `channel` — канальный шаблон (Muse Glimmer): реплики с вызовами
/// собираются в ATEM-блок (`channel_parser::rebuild_turn_text`), как и в
/// ходе. Пока сюда шёл `<tool_call>`-JSON, модель со второго сообщения
/// копировала его как обычный текст, и инструменты больше не вызывались
/// (живой прогон 03.09.2026).
fn build_history(ctx: &SynChatCtx, system_prompt: &str, channel: bool) -> Vec<HistoryItem> {
    ctx.messages
        .with_untracked(|msgs| history_from_messages(msgs, system_prompt, channel))
}

/// Чистая часть [`build_history`]: промпт по ленте, без клона всей истории.
fn history_from_messages(msgs: &[ChatMsg], system_prompt: &str, channel: bool) -> Vec<HistoryItem> {
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
            reasoning: String::new(),
            calls: Vec::new(),
        });
    }
    // Текст и размышления ассистента перед вызовом инструмента лежат в
    // ленте отдельным пузырём прямо перед ним, а в промпте это одна реплика
    // (как в ходе): индекс пузыря в `out` (если текст был) и его thinking.
    let mut prose_before_call: Option<(Option<usize>, String)> = None;
    for m in msgs.iter() {
        // Пустой assistant — это плейсхолдер под стрим (в том числе
        // оставшийся от прерванного хода). В промпт он не идёт никогда: не
        // только последний, иначе после «Прервать» в истории навсегда
        // остаётся пустая реплика ассистента. Но размышления из него — те же,
        // что были у хода перед вызовом, — переносим в реплику вызова.
        if m.role == ChatMsgRole::Assistant
            && matches!(m.kind, ChatMsgKind::Text)
            && m.body.trim().is_empty()
        {
            if !m.thinking.trim().is_empty() {
                prose_before_call = Some((None, m.thinking.clone()));
            }
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
                // Размышления ассистента шаблон Qwen3.x рендерит из
                // `reasoning_content`; без них финальный ответ хода
                // превращается в `<think>⏎⏎</think>`, и следующее сообщение
                // пользователя начинает промпт, который расходится с
                // предыдущим (префикс-KV обнуляется).
                out.push(HistoryItem {
                    role: m.role,
                    body: m.body.clone(),
                    attachments: m.attachments.clone(),
                    tool_name: None,
                    reasoning: if m.role == ChatMsgRole::Assistant {
                        m.thinking.clone()
                    } else {
                        String::new()
                    },
                    calls: Vec::new(),
                });
                prose_before_call = (m.role == ChatMsgRole::Assistant)
                    .then(|| (Some(out.len() - 1), m.thinking.clone()));
            }
            ChatMsgKind::ToolCall { .. } => {
                let (prose, thinking) = match prose_before_call.take() {
                    Some((Some(idx), th)) if idx + 1 == out.len() => {
                        (out.pop().map(|i| i.body).unwrap_or_default(), th)
                    }
                    Some((None, th)) => (String::new(), th),
                    _ => (String::new(), String::new()),
                };
                let calls: Vec<RawToolCall> = m
                    .tool_calls
                    .iter()
                    .flatten()
                    .map(|c| RawToolCall {
                        name: c.function.name.clone().unwrap_or_default(),
                        arguments_json: c.function.arguments.clone().unwrap_or_else(|| "{}".into()),
                    })
                    .collect();
                if channel {
                    // Канальный шаблон: ATEM-блок текстом, как в ходе.
                    let body = channel_parser::rebuild_turn_text(&prose, &calls);
                    if body.is_empty() {
                        continue;
                    }
                    out.push(HistoryItem {
                        role: ChatMsgRole::Assistant,
                        body,
                        attachments: Vec::new(),
                        tool_name: None,
                        reasoning: String::new(),
                        calls: Vec::new(),
                    });
                } else {
                    if calls.is_empty() && prose.trim().is_empty() {
                        continue;
                    }
                    out.push(HistoryItem {
                        role: ChatMsgRole::Assistant,
                        body: prose,
                        attachments: Vec::new(),
                        tool_name: None,
                        reasoning: thinking,
                        calls,
                    });
                }
            }
            ChatMsgKind::ToolResult { tool_name, .. } => {
                // Тело ленты — уже уложенная в окно копия (`fit_for_prompt`
                // хода), плюс заметки для модели: иначе промпт следующего
                // сообщения не совпадает с промптом хода и префикс-KV
                // обнуляется. Чаты, записанные до укладки, приезжают целиком.
                let mut body = m.body.clone();
                body.push_str(&m.model_note);
                // Файлы на карточке — часть результата только у `view_media`
                // (модель сама их смотрела). Медиа пайплайна в старых чатах
                // тоже висело на карточке, но модели не показывалось.
                let attachments =
                    if tools::executor::canonical_tool_name(tool_name) == KEY_VIEW_MEDIA {
                        m.attachments.clone()
                    } else {
                        Vec::new()
                    };
                out.push(HistoryItem {
                    role: m.role,
                    body,
                    attachments,
                    tool_name: Some(tool_name.clone()),
                    reasoning: String::new(),
                    calls: Vec::new(),
                });
                prose_before_call = None;
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
fn prepare_history(items: &[HistoryItem], caps: &MediaCaps) -> (Vec<Message>, crate::syn_chat::daemon::MediaAt) {
    let mut out: Vec<Message> = Vec::with_capacity(items.len());
    let mut media = crate::syn_chat::daemon::MediaAt::new();
    for item in items {
        if let Some(name) = &item.tool_name {
            if item.attachments.is_empty() {
                out.push(Message::tool_named(name.as_str(), item.body.as_str()));
            } else {
                // `view_media`: блоки файлов перед телом, как в ходе.
                let prepared = attach_prompt::prepare_tool_view(&item.body, &item.attachments, caps);
                if !prepared.media.is_empty() {
                    media.insert(out.len(), prepared.media);
                }
                out.push(Message::tool_named(name.as_str(), prepared.text));
            }
            continue;
        }
        match item.role {
            ChatMsgRole::User if !item.attachments.is_empty() => {
                let prepared = attach_prompt::prepare_user_message(&item.body, &item.attachments, caps);
                if !prepared.media.is_empty() {
                    media.insert(out.len(), prepared.media);
                }
                out.push(Message::user(prepared.text));
            }
            ChatMsgRole::User => out.push(Message::user(&item.body)),
            ChatMsgRole::Assistant if !item.calls.is_empty() || !item.reasoning.is_empty() => {
                out.push(assistant_turn_message(&item.body, &item.reasoning, &item.calls))
            }
            ChatMsgRole::Assistant => out.push(Message::assistant(&item.body)),
            ChatMsgRole::System => out.push(Message::system(&item.body)),
        }
    }
    (out, media)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Вернулись в чат посреди ответа: в стриме только хвост, начало —
    /// из текста воркера. Без переключения недостающего нет.
    #[test]
    fn missing_prefix_restores_text_before_return() {
        assert_eq!(missing_prefix("Привет, мир", "мир"), "Привет, ");
        assert_eq!(missing_prefix("Привет, мир", "Привет, мир"), "");
        assert_eq!(missing_prefix("Привет", ""), "Привет");
        assert_eq!(missing_prefix("Привет", "чужое"), "");
    }

    /// Строка «available tools» называет только объявленные схемой
    /// инструменты: ни ключей вне каталога, ни неявного `autotools`.
    #[test]
    fn declared_tool_labels_skip_keys_outside_catalog() {
        let keys: Vec<String> = ["web_read", "bash", "web_search", "autotools", "notes"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(declared_tool_labels(&keys), ["bash", "notes"]);
    }

    /// Сквозной рендер шаблона Gemma-4 с настоящими схемами инструментов и
    /// историей «вызов → результат»: объявления в системном ходе, вызов в
    /// нотации `call:notes{…}` со строками в `<|"|>`, ответ инструмента в
    /// `<|tool_response>`. Нужен бандл — запускать вручную:
    /// `cargo test --release --lib gemma4_template_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn gemma4_template_round_trip() {
        use synaptix_tokenizer::templates::chat_template::RenderOptions;
        use synaptix_tokenizer::{ChatTemplate, Message as TokMessage, MessageRole};
        use synaptix_tokenizer::{ToolCall as TokToolCall, ToolCallFunction as TokToolCallFunction};

        let bundle = std::path::Path::new("/home/master/Storage/Нейросети/Модели/gemma-4-26b-a4b-it.syn");
        let Some(src) = synaptix::facade::arch::read_model_file(bundle, "chat_template.jinja") else {
            eprintln!("нет бандла {} — пропуск", bundle.display());
            return;
        };
        let tmpl = ChatTemplate::from_source(String::from_utf8(src).unwrap());
        let tools: Vec<serde_json::Value> = ["notes", "bash"]
            .iter()
            .map(|k| serde_json::to_value(Tool::by_key(k).unwrap().to_chat_tool()).unwrap())
            .collect();
        let args = r#"{"action":"read","page":"Моя Жизнь","blocks":true,"limit":3}"#;
        let msgs = vec![
            TokMessage::system("Ты ассистент."),
            TokMessage::user("прочитай все заметки"),
            TokMessage::assistant_with_reasoning(
                "",
                None,
                vec![TokToolCall {
                    id: None,
                    call_type: "function".into(),
                    function: TokToolCallFunction { name: "notes".into(), arguments: args.into() },
                }],
            ),
            TokMessage {
                role: MessageRole::Tool,
                content: "# Моя Жизнь\n- пункт".into(),
                name: Some("notes".into()),
                tool_call_id: None,
                tool_calls: Vec::new(),
                reasoning_content: None,
            },
        ];
        let opts = RenderOptions::new()
            .with_generation_prompt(true)
            .with_var("enable_thinking", serde_json::Value::Bool(false))
            .with_var("tools", serde_json::Value::Array(tools));
        let out = tmpl.render(&msgs, &opts).expect("render");
        eprintln!("{out}");
        assert!(out.contains("<|tool>declaration:notes{"), "объявление notes");
        assert!(out.contains("<|tool>declaration:bash{"), "объявление bash");
        assert!(
            out.contains("<|tool_call>call:notes{action:<|\"|>read<|\"|>,blocks:true,limit:3,page:<|\"|>Моя Жизнь<|\"|>}<tool_call|>"),
            "вызов в нотации Gemma: {out}"
        );
        assert!(out.contains("<|tool_response>response:notes{value:<|\"|># Моя Жизнь\n- пункт<|\"|>}<tool_response|>"), "ответ инструмента");
        assert!(out.ends_with("<tool_response|>"), "после результата модель продолжает ход без заголовка: {out:?}");
    }

    fn queued(id: u64, chat: &str) -> QueuedMsg {
        QueuedMsg {
            id,
            chat_id: chat.to_string(),
            body: format!("m{id}"),
            attachments: Vec::new(),
            time: String::new(),
        }
    }

    /// Очередь общая на все чаты: первым уходит самое раннее сообщение
    /// именно того чата, что открыт, а чужие ждут своего чата.
    #[test]
    fn next_queued_is_first_of_that_chat() {
        let q = vec![queued(1, "b"), queued(2, "a"), queued(3, "a")];
        assert_eq!(next_queued(&q, "a").map(|m| m.id), Some(2));
        assert_eq!(next_queued(&q, "b").map(|m| m.id), Some(1));
        assert_eq!(next_queued(&q, "c"), None);
    }

    fn call(name: &str, args: &str) -> ChatToolCall {
        ChatToolCall {
            id: "id".to_string(),
            kind: "function".to_string(),
            function: ChatToolCallFunction {
                name: Some(name.to_string()),
                arguments: Some(args.to_string()),
            },
        }
    }

    #[test]
    fn call_key_ignores_json_formatting() {
        // Модель переформатирует JSON от хода к ходу — guard обязан видеть
        // в этом один и тот же вызов.
        let a = call("bash", r#"{"command":"ls -la"}"#);
        let b = call("bash", "{\n  \"command\": \"ls -la\"\n}");
        assert_eq!(call_key(&a), call_key(&b));
    }

    #[test]
    fn call_key_separates_different_args_and_tools() {
        assert_ne!(
            call_key(&call("bash", r#"{"command":"ls"}"#)),
            call_key(&call("bash", r#"{"command":"pwd"}"#))
        );
        assert_ne!(
            call_key(&call("bash", r#"{"q":"x"}"#)),
            call_key(&call("web", r#"{"q":"x"}"#))
        );
    }

    #[test]
    fn call_key_survives_broken_json() {
        // Невалидный JSON не должен схлопывать разные вызовы в один ключ.
        assert_ne!(call_key(&call("bash", "{oops")), call_key(&call("bash", "{other")));
    }

    fn tool_call_msg(name: &str, args: &str) -> ChatMsg {
        ChatMsg::tool_call(name, args, vec![call(name, args)])
    }

    #[test]
    fn repeat_state_counts_only_current_turn() {
        // Вызовы до последнего user-сообщения к текущему ходу не относятся.
        let msgs = vec![
            ChatMsg::user("старый запрос"),
            tool_call_msg("bash", r#"{"command":"ls"}"#),
            ChatMsg::user("новый запрос"),
            tool_call_msg("bash", r#"{"command":"ls"}"#),
        ];
        let st = repeat_state_from(&msgs);
        assert_eq!(st.totals.values().sum::<usize>(), 1);
        assert_eq!(st.consecutive, 1);
    }

    #[test]
    fn repeat_state_tracks_consecutive_run() {
        let msgs = vec![
            ChatMsg::user("u"),
            tool_call_msg("bash", r#"{"command":"a"}"#),
            tool_call_msg("bash", r#"{"command":"b"}"#),
            tool_call_msg("bash", r#"{"command":"b"}"#),
        ];
        let st = repeat_state_from(&msgs);
        // «Продолжить» после двух одинаковых подряд обязано видеть счётчик 2,
        // иначе кнопка перезапускает ту же петлю с нуля.
        assert_eq!(st.consecutive, 2);
        assert_eq!(st.totals[&call_key(&call("bash", r#"{"command":"b"}"#))], 2);
    }

    #[test]
    fn repeat_state_resets_run_on_different_call() {
        let msgs = vec![
            ChatMsg::user("u"),
            tool_call_msg("bash", r#"{"command":"build"}"#),
            tool_call_msg("bash", r#"{"command":"edit"}"#),
            tool_call_msg("bash", r#"{"command":"build"}"#),
        ];
        let st = repeat_state_from(&msgs);
        // Правка между двумя сборками — это не петля: подряд идущих нет.
        assert_eq!(st.consecutive, 1);
        assert_eq!(st.totals[&call_key(&call("bash", r#"{"command":"build"}"#))], 2);
    }

    fn call_with_id(id: &str, name: &str, args: &str) -> ChatToolCall {
        let mut c = call(name, args);
        c.id = id.to_string();
        c
    }

    /// Аргументы одного вызова из петли 04.09.2026 (чат «LifeBalance»).
    const LOOP_CREATE: &str = r#"{"action":"create","title":"Туду — канбан","parent":"root","icon":"K","layout":"free","content":"board"}"#;

    fn loop_kanban(page: &str) -> String {
        format!(r#"{{"action":"kanban","op":"create","page":"{page}","columns":["a","b"]}}"#)
    }

    #[test]
    fn call_shape_ignores_argument_values() {
        // Один и тот же вызов над разными объектами — одна форма.
        assert_eq!(
            call_shape(&call("notes", &loop_kanban("a2ffdb012dfe"))),
            call_shape(&call("notes", &loop_kanban("7836e7842679")))
        );
    }

    #[test]
    fn call_shape_separates_actions_and_argument_sets() {
        // Разное действие того же инструмента — разные формы.
        assert_ne!(
            call_shape(&call("notes", r#"{"action":"create","title":"x"}"#)),
            call_shape(&call("notes", r#"{"action":"update","title":"x"}"#))
        );
        // Разный набор аргументов — тоже: `read` страницы и `read` доски
        // делают разное.
        assert_ne!(
            call_shape(&call("notes", r#"{"action":"read","page":"a"}"#)),
            call_shape(&call("notes", r#"{"action":"read","board":"a"}"#))
        );
    }

    #[test]
    fn shape_alternation_sees_a_loop_over_fresh_objects() {
        // Ровно петля из чата «LifeBalance»: «создать страницу → положить на
        // неё доску», и так до конца бюджета ходов. Второй вызов каждый раз
        // адресует свежий id, поэтому по точным ключам петли не видно.
        let pages = ["a2ffdb012dfe", "7836e7842679", "84b0fe985a42"];
        let mut keys = std::collections::VecDeque::new();
        let mut shapes = std::collections::VecDeque::new();
        for page in pages {
            for c in [call("notes", LOOP_CREATE), call("notes", &loop_kanban(page))] {
                remember_call(&mut keys, &call_key(&c));
                remember_call(&mut shapes, &call_shape(&c));
            }
        }
        assert!(
            alternation_periods(&keys) < ALTERNATION_WARN_AT,
            "guard точных ключей эту петлю и не видел — тест ловит регресс наоборот"
        );
        assert!(alternation_periods(&shapes) >= SHAPE_ALTERNATION_ANTI_LOOP_AT);
    }

    /// Петля 14.09.2026 (чат MyLife): `arrange` одной страницы, меняется
    /// только y, ответ по сути тот же. Точные ключи разные и не чередуются,
    /// форма одна подряд — ловит ключ с числами под маской.
    #[test]
    fn numeric_masked_key_sees_a_loop_over_coordinates() {
        let arrange = |y: u32| {
            format!(r#"{{"action":"blocks","gap":24,"only":"all","op":"arrange","page":"73db23908f82","w":860,"x":1040,"y":{y}}}"#)
        };
        let answer = |y: u32| {
            format!(
                "arranged 13 blocks in a column at x=1040 (gap 24); the column ends at y={}\n#0 heading1 \"Календарь\" · x=1040 y={y} w=860 h=~97.6\n#12 embed:calendar:3c07c2fd3135 · x=1040 y={} w=860 h=700\npage: 73db23908f82 · \"Календарь\"\n",
                y + 1774,
                y + 1074
            )
        };
        let mut outcomes = HashMap::new();
        let mut last = 0;
        for y in [160, 920, 160] {
            let c = call("notes", &arrange(y));
            let masked = numeric_masked_key(&c);
            assert_ne!(masked, call_key(&c));
            assert_eq!(masked, numeric_masked_key(&call("notes", &arrange(1650))));
            last = note_outcome(&mut outcomes, &masked, &answer(y));
        }
        assert_eq!(last, NUMERIC_STAGNANT_WARN_AT, "четвёртый такой вызов guard уже не исполнит");
        // Разные правки одной страницы — разные строки: это не петля.
        assert_ne!(
            numeric_masked_key(&call("notes", r#"{"action":"update","page":"p1","find":"a","replace":"b"}"#)),
            numeric_masked_key(&call("notes", r#"{"action":"update","page":"p1","find":"c","replace":"d"}"#))
        );
        // Перенос разных блоков — разные ключи: номер блока — адрес.
        let mut moves = HashMap::new();
        let mv = |b: u32| call("notes", &format!(r#"{{"action":"blocks","op":"move","page":"p1","block":{b},"x":40,"y":40}}"#));
        assert_ne!(numeric_masked_key(&mv(1)), numeric_masked_key(&mv(2)));
        for (b, label) in [(1, "heading1 \"Семья\""), (2, "table \"Дата · Человек\""), (3, "todo \"Подарки\"")] {
            let n = note_outcome(&mut moves, &numeric_masked_key(&mv(b)), &format!("moved\n#{b} {label} · x=40 y=40\n"));
            assert_eq!(n, 0, "{label}");
        }
    }

    /// Пинг-понг 14.09.2026 (MyLife, «Долги и кредиты»): `set_attrs` гонял
    /// #14, #24 и #25 по y вперемешку, ответ у каждого блока по сути один.
    /// С номером блока под маской ключ был общий, и смена блока сбрасывала
    /// счётчик; теперь у каждого блока свой.
    #[test]
    fn numeric_masked_key_sees_a_ping_pong_of_several_blocks() {
        let set = |b: u32, x: u32, y: u32| {
            call(
                "notes",
                &format!(r#"{{"action":"blocks","attrs":{{"x":{x},"y":{y}}},"block":{b},"op":"set_attrs","page":"Долги и кредиты"}}"#),
            )
        };
        let answer = |b: u32, x: u32, y: u32| match b {
            14 => format!("set x={x}, y={y}\n#14 toggle \"Полная таблица графика (52 платежа)\" · x={x} y={y} w=720 h=2100\n!! overlaps #24 (x=40 y=1825 w=1520 h=44.4)\n"),
            _ => format!("set x={x}, y={y}\n#24 heading2 \"Доска долгов\" · x={x} y={y} w=1520 h=~44.4\n!! overlaps #14 (x=840 y=1025 w=720 h=2100)\n"),
        };
        let mut outcomes = HashMap::new();
        let mut last_24 = 0;
        for (b, x, y) in [(24, 840, 1365), (14, 840, 1025), (24, 40, 1825), (14, 840, 1025), (24, 840, 1825)] {
            let n = note_outcome(&mut outcomes, &numeric_masked_key(&set(b, x, y)), &answer(b, x, y));
            if b == 24 {
                last_24 = n;
            }
        }
        assert_eq!(last_24, NUMERIC_STAGNANT_WARN_AT, "четвёртый перенос #24 guard уже не исполнит");
    }

    #[test]
    fn outcome_fingerprint_ignores_ids_and_counters() {
        // Два ответа `create` из петли: разный id страницы и разный номер в
        // названии-дубликате — произошло при этом одно и то же.
        let a = "created\ncontent: 35 words\npage: 7836e7842679 · \"Туду — канбан 2\"\n";
        let b = "created\ncontent: 35 words\npage: 84b0fe985a42 · \"Туду — канбан 3\"\n";
        assert_eq!(outcome_fingerprint(a), outcome_fingerprint(b));
    }

    #[test]
    fn outcome_fingerprint_keeps_meaningful_differences() {
        // Коды ошибок сборки схлопывать нельзя: агент, у которого E0308
        // сменилась на E0277, движется — останавливать его guard не должен.
        assert_ne!(
            outcome_fingerprint("error[E0308]: mismatched types"),
            outcome_fingerprint("error[E0277]: trait not satisfied")
        );
        assert_ne!(
            outcome_fingerprint("compiled with 0 errors"),
            outcome_fingerprint("compiled with warnings")
        );
    }

    #[test]
    fn note_outcome_counts_repeats_and_resets_on_change() {
        let mut outcomes = HashMap::new();
        assert_eq!(note_outcome(&mut outcomes, "k", "created page: aaaa11"), 0);
        assert_eq!(note_outcome(&mut outcomes, "k", "created page: bbbb22"), 1);
        assert_eq!(note_outcome(&mut outcomes, "k", "created page: cccc33"), 2);
        // Вывод изменился по существу — счётчик стагнации обнуляется.
        assert_eq!(note_outcome(&mut outcomes, "k", "error: no such page"), 0);
    }

    #[test]
    fn repeat_state_tracks_stagnation_from_ledger() {
        // «Продолжить» после петли обязано видеть, что вызов уже дважды отдал
        // тот же результат, — иначе кнопка запускает её заново.
        let msgs = vec![
            ChatMsg::user("u"),
            ChatMsg::tool_call("notes", LOOP_CREATE, vec![call_with_id("c1", "notes", LOOP_CREATE)]),
            ChatMsg::tool_result("c1", "notes", "created\npage: 7836e7842679 · \"Туду 2\"", false),
            ChatMsg::tool_call("notes", LOOP_CREATE, vec![call_with_id("c2", "notes", LOOP_CREATE)]),
            ChatMsg::tool_result("c2", "notes", "created\npage: 84b0fe985a42 · \"Туду 3\"", false),
        ];
        let st = repeat_state_from(&msgs);
        let key = call_key(&call("notes", LOOP_CREATE));
        assert_eq!(st.outcomes[&key].1, 1, "второй вызов отдал тот же результат");
    }

    #[test]
    fn repeat_state_keeps_stagnation_at_zero_when_output_changes() {
        // Правка → сборка → правка → сборка: вывод сборки меняется, и это не
        // петля, сколько бы раз команда ни повторялась.
        let build = r#"{"command":"cargo check"}"#;
        let msgs = vec![
            ChatMsg::user("u"),
            ChatMsg::tool_call("bash", build, vec![call_with_id("b1", "bash", build)]),
            ChatMsg::tool_result("b1", "bash", "error[E0308]: mismatched types", false),
            ChatMsg::tool_call("bash", build, vec![call_with_id("b2", "bash", build)]),
            ChatMsg::tool_result("b2", "bash", "error[E0277]: trait bound", false),
        ];
        let st = repeat_state_from(&msgs);
        assert_eq!(st.outcomes[&call_key(&call("bash", build))].1, 0);
    }

    #[test]
    fn repeat_state_ignores_compacted() {
        let mut msgs = vec![
            ChatMsg::user("u"),
            tool_call_msg("bash", r#"{"command":"a"}"#),
            tool_call_msg("bash", r#"{"command":"a"}"#),
        ];
        msgs[1].compacted_iter = Some(1);
        let st = repeat_state_from(&msgs);
        assert_eq!(st.consecutive, 1);
    }

    /// По символам — как в guard'е: `'LILIT',` по кругу после честного
    /// скрипта ловится с периодом 8, тот же текст с одним лишним символом в
    /// хвосте — нет.
    #[test]
    fn periodic_tail_on_chars_catches_the_lilit_loop() {
        let script: String = (1..=60).map(|i| format!("row_{i} = parse(line_{i})\n")).collect();
        let mut looped: Vec<char> = script.chars().collect();
        looped.extend("'LILIT',".repeat(500).chars());
        assert_eq!(periodic_tail(&looped, LOOP_WINDOW_CHARS, LOOP_MAX_PERIOD_CHARS), Some(8));
        let honest: Vec<char> = script.repeat(10).chars().collect();
        assert!(honest.len() > LOOP_WINDOW_CHARS);
        assert_eq!(periodic_tail(&honest, LOOP_WINDOW_CHARS, LOOP_MAX_PERIOD_CHARS), None, "строки разные");
        looped.push('!');
        assert_eq!(periodic_tail(&looped, LOOP_WINDOW_CHARS, LOOP_MAX_PERIOD_CHARS), None);
    }

    /// Зацикленный хвост находится с наименьшим периодом; честная
    /// последовательность и короткий хвост — нет.
    #[test]
    fn periodic_tail_finds_the_shortest_period() {
        let mut ids: Vec<u32> = (0..500).map(|i| (i * 7919 % 1000) as u32).collect();
        assert_eq!(periodic_tail(&ids, 1024, 128), None, "короче окна");
        ids.extend((0..2000).map(|i| (i * 7919 % 1000) as u32));
        assert_eq!(periodic_tail(&ids, 1024, 128), None, "без повторов");
        // `'LILIT',` как 4 токена по кругу поверх честного начала.
        let mut looped = ids.clone();
        for _ in 0..300 {
            looped.extend([10, 11, 12, 13]);
        }
        assert_eq!(periodic_tail(&looped, 1024, 128), Some(4));
        // Один и тот же токен — период 1.
        let mut same = ids.clone();
        same.extend(std::iter::repeat_n(7, 1100));
        assert_eq!(periodic_tail(&same, 1024, 128), Some(1));
        // Период длиннее допустимого — не петля для guard'а.
        let mut long = ids.clone();
        for _ in 0..10 {
            long.extend(0..200u32);
        }
        assert_eq!(periodic_tail(&long, 1024, 128), None);
        assert_eq!(periodic_tail(&long, 1024, 256), Some(200));
    }

    use crate::agent::tools::budget;
    use crate::agent::tools::catalog::KEY_BASH;

    fn chars3(s: &str) -> usize {
        s.chars().count() / 3
    }

    /// Выхлоп `sed -n '1,250p'` по 16 КБ: при свободном окне уходит целиком
    /// — именно на этом ломался чат «MyLife» (07.09.2026), где статический
    /// клип резал каждый кусок и модель перечитывала файл по кругу.
    /// Файлы, которые модель посмотрела через `view_media`, лежат на карточке
    /// результата и уходят в пересборку промпта (иначе следующее сообщение
    /// теряло бы картинку и префикс-KV). Медиа пайплайна на карточке старых
    /// чатов модели не показывалось — и не показывается.
    #[test]
    fn view_media_attachments_reach_history_pipeline_media_does_not() {
        let img = MsgAttachment {
            sha256: "a".repeat(64),
            mime: "image/png".into(),
            original_name: "me.png".into(),
            width: 640,
            height: 480,
            size_bytes: 1000,
            kind: crate::agent::state::AttachmentKind::Image,
            ext: "png".into(),
            duration_ms: 0,
            model_ext: String::new(),
            ui_ext: String::new(),
            has_thumb: false,
            share_path: false,
        };
        let mut viewed = ChatMsg::tool_result("c1", "view_media", "view_media: 1 of 1", false);
        viewed.attachments = vec![img.clone()];
        viewed.model_note = "\n[note]".into();
        // Канальный шаблон зовёт инструмент с пространством имён.
        let mut viewed_ns = ChatMsg::tool_result("c2", "view_media.show", "ok", false);
        viewed_ns.attachments = vec![img.clone()];
        let mut pipe = ChatMsg::tool_result("c3", "pipelines", "done", false);
        pipe.attachments = vec![img.clone()];

        let ctx = SynChatCtx::new();
        ctx.messages.set(vec![ChatMsg::user("найди меня"), viewed, viewed_ns, pipe]);
        let items = build_history(&ctx, "", false);
        let tools: Vec<&HistoryItem> = items.iter().filter(|i| i.tool_name.is_some()).collect();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].attachments, [img.clone()]);
        assert_eq!(tools[0].body, "view_media: 1 of 1\n[note]", "заметка — в тело, как в ходе");
        assert_eq!(tools[1].attachments, [img]);
        assert!(tools[2].attachments.is_empty(), "медиа пайплайна модели не показывается");
    }

    #[test]
    fn fit_for_prompt_keeps_a_chunk_that_fits_the_window() {
        let _serial = budget::test_serial();
        let _guard = budget::arm(100_000, std::sync::Arc::new(chars3));
        let body = format!(
            "$ sed -n '1,250p' doc.md\nexit: 0\n--- stdout ---\n{}",
            (1..=250)
                .map(|i| format!("| {i:02}.05 | Покупка | MERKURII SUPERMARKET | −9 007.00 |\n"))
                .collect::<String>()
        );
        assert!(body.len() > 8_000, "кусок заведомо больше прежнего клипа");
        assert_eq!(fit_for_prompt(&body, KEY_BASH), body);
    }

    /// Тесное окно: голова с командой и exit-кодом, хвост, пометка с
    /// номерами вырезанных строк.
    #[test]
    fn fit_for_prompt_cuts_the_middle_in_a_tight_window() {
        let _serial = budget::test_serial();
        let _guard = budget::arm(3_000, std::sync::Arc::new(chars3));
        let body = format!(
            "$ cat big.log\nexit: 0\n--- stdout ---\n{}--- stderr ---\nwarning: хвост\n",
            (1..=2000).map(|i| format!("line {i}: {}\n", "x".repeat(40))).collect::<String>()
        );
        let out = fit_for_prompt(&body, KEY_BASH);
        assert!(out.starts_with("$ cat big.log\nexit: 0\n"));
        assert!(out.ends_with("--- stderr ---\nwarning: хвост\n"), "хвост со stderr обязан остаться");
        assert!(out.contains("вывод обрезан по остатку контекста"));
        assert!(out.contains("пропущены строки"));
        assert!(out.contains("в окне осталось ~3000 токенов"));
        assert!(budget::count(&out) <= 1_500, "{} токенов при гранте 1500", budget::count(&out));
    }

    #[test]
    fn fit_for_prompt_is_char_safe() {
        let _serial = budget::test_serial();
        let _guard = budget::arm(600, std::sync::Arc::new(chars3));
        // Резка идёт по символам, а не байтам: кириллица не должна биться.
        let body = "я".repeat(20_000);
        let out = fit_for_prompt(&body, KEY_BASH);
        assert!(out.contains("вырезан") || out.contains("обрезан"));
        assert!(out.chars().all(|c| c != '\u{fffd}'));
    }

    #[test]
    fn skill_reaches_the_model_whole() {
        let _serial = budget::test_serial();
        let _guard = budget::arm(600, std::sync::Arc::new(chars3));
        // Скил — инструкция, а не выхлоп: вырезанная середина уносит ровно
        // то знание, ради которого его подключали.
        use crate::agent::tools::catalog::KEY_AUTOSKILL;
        let body = "я\n".repeat(20_000);
        assert_eq!(fit_for_prompt(&body, KEY_AUTOSKILL), body);
        assert!(fit_for_prompt(&body, KEY_BASH).contains("вывод обрезан"));
    }
}
