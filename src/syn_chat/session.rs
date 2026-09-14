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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synaptix::facade::llm::{
    LlmGeneration, LlmKvSession, LlmTokenizer, MediaEmbedding, Message,
};

use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
use crate::agent::tools::catalog::{KEY_SUBAGENT, KEY_VIEW_MEDIA};
use crate::agent::tools::{self, Tool, ToolDecision};
use crate::context::AppCtx;
use crate::syn_chat::attach::prompt::{self as attach_prompt, MediaCaps};
use crate::syn_chat::channel_parser::{self, ChannelIds, ChannelParser, ATEM_CLOSE};
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{
    ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment, QueuedMsg, SynChatCtx, ThinkParser,
};
use crate::syn_chat::system_prompt::{self, PromptEnv};
use crate::syn_chat::tool_parser::{RawToolCall, ToolCallParser};
use synaptix_tokenizer::{Gemma4Ids, Gemma4StreamParser};

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
    /// Промпт последнего хода через эту сессию — только для диагностики:
    /// когда следующий ход не переиспользовал ничего, журнал показывает,
    /// на какой позиции промпты разошлись и что там стояло.
    last_prompt: Vec<u32>,
}

/// Пишет в журнал первое расхождение между промптом прошлого хода и нового.
/// Зовётся только когда префикс-KV не переиспользовал ни токена при живой
/// сессии: из «0 из N» причина не видна, а расхождение на 3 токена от конца
/// (заголовок реплики) и расхождение в системном промпте — разные проблемы.
fn log_prefix_divergence(tokenizer: &LlmTokenizer, prev: &[u32], cur: &[u32]) {
    let common = prev.iter().zip(cur.iter()).take_while(|(a, b)| a == b).count();
    let decode = |ids: &[u32]| tokenizer.decode(ids).unwrap_or_default().replace('\n', "⏎");
    if common == prev.len().min(cur.len()) {
        log::info!(
            "[syn_chat] префикс-KV: расхождения нет, промпт {} (было {} ток): {}",
            if cur.len() < prev.len() { "короче кэша" } else { "продолжает прошлый" },
            prev.len(),
            "кэш не подошёл по другой причине (пересоздание, точка возврата не снята)"
        );
        return;
    }
    let lo = common.saturating_sub(24);
    let hi_prev = (common + 12).min(prev.len());
    let hi_cur = (common + 12).min(cur.len());
    log::info!(
        "[syn_chat] префикс-KV: промпт разошёлся с прошлым на позиции {common} из {} \
         (прошлый {} ток): было «{}», стало «{}»",
        cur.len(),
        prev.len(),
        decode(&prev[lo..hi_prev]),
        decode(&cur[lo..hi_cur])
    );
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

/// Подходит ли уже живущая сессия под этот ход.
///
/// Сравнивать надо с тем, что ходу НУЖНО (`need_ctx` — промпт, ответ и
/// хвост), а не с тем, сколько мы хотели бы выделить: `want_ctx` растёт
/// вслед за свободной VRAM, и пока условие смотрело на него, кэш выбрасывался
/// от того, что памяти стало БОЛЬШЕ. В журнале это выглядело так: сессия на
/// 27056 токенов, промпт 9112, ринг 20480 — всё влезало, но бюджет подрос,
/// `want_ctx` подскочил до 32768, кэш пересоздали и ход заплатил 11.9 с
/// полного префилла на ровном месте.
fn kv_slot_fits(slot: &KvSlot, model_path: &std::path::Path, chat: &Option<String>, need_ctx: usize) -> bool {
    slot.model == model_path && &slot.chat == chat && slot.session.ctx_tokens() >= need_ctx
}

/// Взять сессию под этот чат/модель, создав или пересоздав при необходимости.
/// `None` — префикс-KV недоступен (архитектура или нехватка VRAM), вызывающий
/// работает как раньше.
///
/// `need_ctx` — минимум под этот ход, `want_ctx` — ёмкость, которую берём при
/// пересоздании (с запасом, см. [`SESSION_CTX_STEP`]).
fn ensure_kv_slot<'a>(
    slot: &'a mut Option<KvSlot>,
    model: &LoadedSynModel,
    chat: &Option<String>,
    need_ctx: usize,
    want_ctx: usize,
    max_new: usize,
) -> Option<&'a mut LlmKvSession> {
    let fits = slot
        .as_ref()
        .is_some_and(|s| kv_slot_fits(s, &model.path, chat, need_ctx));
    if !fits {
        if let Some(old) = slot.take() {
            log::info!(
                "[syn_chat] префикс-KV: пересоздаём кэш (было {} ток, ходу нужно \
                 ≥{need_ctx}, берём {want_ctx})",
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
                    last_prompt: Vec::new(),
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

/// Отправляет посчитанный контекст диалога в host-RAM на время вложенной
/// генерации и возвращает VRAM драйверу. `true` — кэш припаркован, и его
/// надо забрать обратно [`unpark_kv_session`].
///
/// Альтернатива — сбросить кэш совсем, но тогда следующий ход платит полным
/// префиллом всей истории (на 10k токенов это ~10 с против ~40 мс перевоза
/// 250 МБ через PCIe).
fn park_kv_session(
    kv_slot: &mut std::sync::MutexGuard<'_, Option<KvSlot>>,
    model: &LoadedSynModel,
) -> bool {
    let Some(slot) = kv_slot.as_mut() else {
        return false;
    };
    if slot.session.device_bytes() == 0 {
        return false;
    }
    let t = Instant::now();
    match slot.session.park_to_host() {
        Ok(bytes) => {
            let (freed, descs) =
                crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
            log::info!(
                "[syn_chat] префикс-KV: {} MB освобождены (кэш уехал в RAM) за {:?}; \
                 трим вернул {freed} MB ({descs} TMA-деск.), VRAM доступно {} MB",
                bytes / (1024 * 1024),
                t.elapsed(),
                crate::syn_chat::model_registry::vram_available_mb()
            );
            true
        }
        Err(e) => {
            // Не вышло — работаем как раньше: сбрасываем кэш, вложенный
            // прогон получит место, а следующий ход префиллит заново.
            log::warn!("[syn_chat] префикс-KV: выгрузка в RAM не удалась ({e}) — сбрасываем кэш");
            **kv_slot = None;
            crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
            false
        }
    }
}

/// Возвращает припаркованный кэш в VRAM. Не вышло (места не нашлось) —
/// выбрасываем сессию: полный префилл всегда возможен, половина кэша на
/// устройстве — нет.
fn unpark_kv_session(
    kv_slot: &mut std::sync::MutexGuard<'_, Option<KvSlot>>,
    model: &LoadedSynModel,
) {
    let Some(slot) = kv_slot.as_mut() else {
        return;
    };
    if !slot.session.is_parked() {
        return;
    }
    let t = Instant::now();
    match slot.session.unpark_to(*model.model.device()) {
        Ok(bytes) => log::info!(
            "[syn_chat] префикс-KV: {} MB вернулись в VRAM за {:?}, доступно {} MB",
            bytes / (1024 * 1024),
            t.elapsed(),
            crate::syn_chat::model_registry::vram_available_mb()
        ),
        Err(e) => {
            log::warn!(
                "[syn_chat] префикс-KV: кэш не вернулся в VRAM ({e}) — сбрасываем, \
                 следующий ход префиллит историю заново"
            );
            **kv_slot = None;
            crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
        }
    }
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

/// Сколько VRAM нужно ходу под KV: кольцо на `tokens` плюс постоянные
/// ring-окна sliding-слоёв плюс запас под активации и кэши ядер.
fn context_need_mb(model: &LoadedSynModel, tokens: usize) -> usize {
    let per_token = model.model.kv_bytes_per_token();
    let fixed = model.model.kv_fixed_bytes(tokens);
    (tokens * per_token + fixed) / (1024 * 1024) + kv_reserve_mb()
}

/// Подогнать резидентность блоков под нужный контекст: если ход в память не
/// помещается, часть блоков уезжает на хост и стримится во время forward'а,
/// освобождая VRAM под KV. Когда контекст снова короткий и памяти с запасом —
/// блоки возвращаются на карту.
///
/// Плата за оффлоад — скорость (каждый нерезидентный блок едет по PCIe на
/// каждом forward'е, CUDA-графы выключаются), поэтому включается он только
/// когда иначе ход не проходит, и снимается при первой возможности.
///
/// `session_held_mb` — VRAM живого кэша префикс-KV. Ход считает в этом кэше и
/// новой памяти под него не просит (а при росте кэш освобождается ДО новой
/// аллокации), поэтому из потребности он вычитается — ровно как в
/// [`RingPlan::with_session`]. Без этого ход с кэшем на 30k токенов решал, что
/// ему не хватает пары мегабайт, и выселял блоки под память, которая у него
/// уже была: 07.09.2026 чат на промпте 10k так потерял 32 → 20 ток/с и назад
/// уже не вернулся (гистерезис возврата требует запаса в два блока).
fn fit_blocks_for_context(model: &LoadedSynModel, tokens: usize, session_held_mb: usize) {
    let Some((block_bytes, total)) = model.model.block_offload_shape() else {
        return; // архитектура со своим оффлоадом (MoE) — не наше дело
    };
    if block_bytes == 0 || total == 0 {
        return;
    }
    let Some(resident) = model.model.resident_blocks() else {
        return;
    };
    let block_mb = block_bytes / (1024 * 1024);
    if block_mb == 0 {
        return;
    }
    let need_mb = context_need_mb(model, tokens).saturating_sub(session_held_mb);
    let free_mb = crate::syn_chat::model_registry::vram_available_mb();
    let device = *model.model.device();
    // Трим — ДО решения, а не после выселения: кэши ядер и слабина пулов
    // весов/staging в `vram_available_mb` не входят, а отдают они стабильно
    // 300–600 МБ (журнал 07.09.2026). Дефицит же бывал в 2 МБ — выселять
    // блок ради него значило платить треть скорости за память, которую
    // драйвер вернул бы и так.
    let mut trimmed_mb = 0u64;
    let after_trim = || {
        let (freed, _) = crate::syn_chat::model_registry::reclaim_vram(device);
        trimmed_mb = freed;
        crate::syn_chat::model_registry::vram_available_mb()
    };
    let target = block_residency_target(need_mb, free_mb, after_trim, block_mb, resident, total);
    let Some(want) = target else {
        if trimmed_mb > 0 {
            log::info!(
                "[syn_chat] блоки остаются на карте ({resident} из {total}): ходу не хватало \
                 {} MB под контекст {tokens} ток, трим вернул {trimmed_mb} MB",
                need_mb.saturating_sub(free_mb)
            );
        }
        return;
    };
    let got = model.model.set_block_residency(want).unwrap_or(resident);
    if want < resident {
        // Память уехавших блоков остаётся в резерве default-пула, а планировщик
        // ринга считает по «свободно по драйверу»: без трима он не увидит
        // освобождённого, упрёт ринг в пол `промпт + 512` и выдаст сессию,
        // которую следующий ход пересоздаст с полным префиллом (07.09.2026:
        // 103 с на 77k токенов ровно из-за этого).
        let (after, _) = crate::syn_chat::model_registry::reclaim_vram(device);
        log::info!(
            "[syn_chat] оффлоад блоков: {resident} → {got} из {total} на карте \
             (не хватало {} MB под контекст {tokens} ток, блок {block_mb} MB, \
             кэш префикс-KV держит {session_held_mb} MB, трим до этого вернул \
             {trimmed_mb} MB, после выселения — {after} MB); остальные стримятся с хоста",
            need_mb.saturating_sub(free_mb + trimmed_mb as usize)
        );
    } else if got > resident {
        log::info!(
            "[syn_chat] блоки вернулись на карту: {resident} → {got} из {total} \
             (свободно {free_mb} MB, ходу нужно ещё {need_mb} MB)"
        );
    }
}

/// Арифметика [`fit_blocks_for_context`] без модели — чтобы её проверял тест.
/// `need_mb` — то, что ходу осталось выделить (потребность за вычетом живого
/// кэша префикс-KV), `free_mb` — доступная VRAM, `after_trim` — она же после
/// трима кэшей ядер и пулов (зовётся только когда без него не хватает).
/// `None` — резидентность не трогаем.
fn block_residency_target(
    need_mb: usize,
    free_mb: usize,
    after_trim: impl FnOnce() -> usize,
    block_mb: usize,
    resident: usize,
    total: usize,
) -> Option<usize> {
    if block_mb == 0 {
        return None;
    }
    let free_mb = if need_mb > free_mb { free_mb.max(after_trim()) } else { free_mb };
    if need_mb > free_mb {
        // Не хватает: выселяем столько блоков, сколько нужно, плюс один — на
        // сам стриминг (на карте живут текущий и префетченный), но только
        // когда дефицит не меньше блока: при дефиците в мегабайты один
        // выселенный блок и так оставляет почти весь свой размер запасом.
        let missing = need_mb - free_mb;
        let evict = missing.div_ceil(block_mb) + usize::from(missing >= block_mb);
        let want = resident.saturating_sub(evict);
        return (want < resident).then_some(want);
    }
    if resident >= total {
        return None;
    }
    // Памяти хватает — возвращаем блоки. Гистерезис в два блока, чтобы не
    // гонять их туда-сюда на каждом ходу; но если запаса хватает, доводим до
    // полной резидентности: последний стримящийся блок стоит и CUDA-графов,
    // и заметной части скорости (04.09.2026: 51 из 52 на карте — 14 ток/с
    // против 22 при полной резидентности).
    let spare = (free_mb - need_mb) / block_mb;
    if spare < 2 {
        return None;
    }
    // Если до полной резидентности не хватает меньше блока — всё равно
    // дотягиваем: последний стримящийся блок отнимает больше (нет CUDA-графов,
    // треть скорости), чем стоит запас под него, а если ход всё же не влезет,
    // ретрай по OOM отработает честно.
    let want = if resident + spare + 1 >= total { total } else { resident + spare };
    (want > resident).then_some(want)
}

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
/// значит впустую занять гигабайты. 16k — потолок ответа по умолчанию
/// (`SamplingParams::default`): ниже него ринг молча резал бы длинный ответ,
/// хотя настройка обещает больше; при MXFP8-KV (33 КБ/ток у гибрида 27B)
/// это ~530 МБ — столько же, сколько прежние 8k стоили при F16.
pub(crate) const RING_ANSWER_TOKENS: usize = 16384;
/// Гранулярность длины ринга. Промпт растёт от хода к ходу, и ринг «в притык»
/// каждый раз просил бы блоки чуть большего размера — освободившиеся от
/// прошлого ринга пул отдать под них не может. Кратность 4096 делает ринг
/// одинаковым на серии ходов, и блоки переиспользуются.
const RING_GRANULARITY: usize = 4096;
/// Сколько раз пересобирать ход с меньшим рингом после OOM.
pub(crate) const MAX_OOM_RETRIES: usize = 3;
/// Бюджет ответа, ниже которого ретраить уже нечем.
pub(crate) const MIN_ANSWER_TOKENS: usize = 512;
/// Сколько просим у отдаваемых кэшей на первом OOM-ретрае. Меньше
/// полугигабайта просить бессмысленно: и пул, и арена экспертов возвращают
/// драйверу целыми блоками.
const RECLAIM_ON_OOM_MB: usize = 1024;

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
        // Кэш префикс-KV держит VRAM прямо сейчас, но при пересоздании
        // освобождается ДО новой аллокации — иначе бюджет занижался бы ровно на
        // размер уже живущего кэша и контекст перестал бы расти.
        let vram_available_mb =
            crate::syn_chat::model_registry::vram_available_mb() + session_held_mb;
        // Часть блоков стримится с хоста — VRAM уже в дефиците, и запас под
        // сессию только отнял бы её у рабочего набора стриминга.
        let offloading = model
            .model
            .block_offload_shape()
            .is_some_and(|(_, total)| model.model.resident_blocks().unwrap_or(total) < total);
        Self::compute(
            prompt_tokens,
            answer_tokens,
            model_cap,
            model.model.kv_bytes_per_token(),
            // Sliding-слои на ring-KV держат окно постоянного размера — оно не
            // входит в ставку «на токен», но VRAM занимает.
            model.model.kv_fixed_bytes(cap),
            vram_available_mb,
            offloading,
        )
    }

    /// Арифметика плана — без модели, чтобы её можно было проверить тестом.
    pub(crate) fn compute(
        prompt_tokens: usize,
        answer_tokens: usize,
        model_cap: usize,
        kv_per_token: usize,
        kv_fixed_bytes: usize,
        vram_available_mb: usize,
        offloading: bool,
    ) -> Self {
        let cap = model_cap.saturating_sub(1);
        let by_mem = if kv_per_token > 0 {
            // Постоянные ring-окна вычитаем до деления, иначе бюджет завышен
            // ровно на их сумму.
            let budget = (vram_available_mb.saturating_sub(kv_reserve_mb()) * 1024 * 1024)
                .saturating_sub(kv_fixed_bytes);
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
        // Ёмкость сессии берём с запасом: пересоздание кэша стирает префикс,
        // и на моделях, где бюджет позволяет лишь пару шагов (Muse-30B: 2 ГБ
        // свободных), каждый рост промпта стоил полного префилла. Запас —
        // вдвое от нужного ходу, но не больше того, что честно влезает. При
        // оффлоаде блоков удвоения нет: каждый лишний гигабайт кэша — это ещё
        // четыре блока на хосте и треть скорости.
        let spare = if offloading { want } else { want.saturating_mul(2) };
        let session_ctx = want
            .div_ceil(SESSION_CTX_STEP)
            .max(1)
            .saturating_mul(SESSION_CTX_STEP)
            .max(spare)
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
    }
}

/// `(на карте, всего)` блоков модели, если часть их стримится с хоста.
fn blocks_resident_of(model: &LoadedSynModel) -> Option<(u32, u32)> {
    let (_, total) = model.model.block_offload_shape()?;
    let on_card = model.model.resident_blocks().unwrap_or(total);
    (on_card < total).then_some((on_card as u32, total as u32))
}

/// Ошибка — это исчерпание VRAM? Драйвер отдаёт `CUDA_ERROR_OUT_OF_MEMORY`,
/// наши аллокаторы добавляют свои формулировки («after trim+retries: OOM»).
pub(crate) fn is_oom_error<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("OUT_OF_MEMORY") || s.contains("out of memory") || s.contains(": OOM")
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
    // Префикс-KV — это токены истории по порядку; после правки середины
    // всё дальше точки правки невалидно. Освобождаем явно, чтобы VRAM под
    // устаревший кэш не висела до следующего хода.
    drop_kv_session();
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
    drop_kv_session();
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
    drop_kv_session();
}

/// Заменяет текст сообщения (правка in-place). Вложения и thinking не
/// трогаются. Во время генерации — no-op.
pub fn edit_message(idx: usize, body: String) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        return;
    }
    let text = body.trim_end().to_string();
    let mut changed = false;
    ctx.messages.update(|m| {
        if let Some(msg) = m.get_mut(idx) {
            if msg.body != text {
                msg.body = text;
                changed = true;
            }
        }
    });
    ctx.editing_msg.set(None);
    if changed {
        drop_kv_session();
    }
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
    drop_kv_session();
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
pub fn regenerate_last() {
    let ctx = use_context::<SynChatCtx>();
    if ctx.pending.get_untracked() {
        ctx.abort.fetch_add(1, Ordering::Relaxed);
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

    // Карточки субагентов прошлого хода к новому вопросу отношения не
    // имеют — панель «Детали» начинает с чистого листа.
    crate::syn_chat::telemetry::reset();

    // 2. Snapshot params + история + tool-схемы (всё на main thread!).
    // В режиме `default` сэмплинг — из пресета модели.
    let params = ctx.params.get_untracked().effective(&model.sampling);
    let max_turns = app_ctx.general.agent_max_turns.get_untracked().max(1) as usize;
    // Системный промпт агента: базовые правила + пользовательская добавка из
    // настроек. Пустое поле в настройках больше не означает «промпта нет».
    let system_prompt = system_prompt::build(&PromptEnv::snapshot(
        active_tool_labels(&app_ctx),
        pool_tool_labels(&app_ctx),
        max_turns,
        ctx.system_prompt.get_untracked(),
    ));
    let channel = ChannelIds::detect(&model.tokenizer).is_some();
    let history: Vec<HistoryItem> = build_history(&ctx, &system_prompt, channel);
    let caps = snapshot_media_caps(&app_ctx, &model, params.max_seq_len);
    let tool_schemas: Vec<serde_json::Value> = collect_active_tool_schemas(&app_ctx);
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

/// Разбор потока генерации: у моделей разный «протокол хода».
///
/// - [`Self::ChatML`] — Qwen3 и прочие: reasoning в `<think>…</think>`,
///   вызовы в `<tool_call>…</tool_call>`, всё внутри одного текста.
/// - [`Self::Channel`] — Muse Glimmer: ход состоит из нескольких сообщений
///   со своими адресатами (`to=self` / `to=user` / `to=<функция>`), а
///   разделители — спецтокены, невидимые в декодированном тексте
///   (см. [`crate::syn_chat::channel_parser`]).
/// - [`Self::Gemma`] — Gemma-4: размышления в `<|channel>thought…<channel|>`,
///   вызовы в `<|tool_call>call:имя{…}<tool_call|>` со строками в кавычках
///   `<|"|>`; всё это спецтокены, в тексте их нет — разбор по id живёт в
///   движке (`synaptix_tokenizer::parsers::gemma4`). Без него в ленту
///   утекало `call:notes{action:read,page:all}` текстом (MyLife, 09.09.2026).
pub(crate) enum StreamParser {
    ChatML { think: ThinkParser, tools: ToolCallParser },
    Channel(ChannelParser),
    Gemma(Gemma4StreamParser),
}

impl StreamParser {
    /// Канальный разбор — если словарь модели знает `<|start|>`/`<|message|>`.
    ///
    /// Gemma-4 — если словарь знает `<|tool_call>` и `<|"|>`. `prompt` —
    /// отрендеренный промпт хода: у Gemma после результата инструмента при
    /// включённых размышлениях он кончается `<|channel>thought⏎`, и модель
    /// начинает прямо с размышлений, без заголовка канала в потоке.
    pub(crate) fn for_model(tokenizer: &LlmTokenizer, enable_thinking: bool, prompt: &str) -> Self {
        if let Some(ids) = ChannelIds::detect(tokenizer) {
            return Self::Channel(ChannelParser::new(ids));
        }
        match gemma4_ids(tokenizer) {
            Some(ids) => Self::Gemma(Gemma4StreamParser::new(
                ids,
                prompt.ends_with("<|channel>thought\n"),
            )),
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

    pub(crate) fn is_channel(&self) -> bool {
        matches!(self, Self::Channel(_))
    }

    /// Протокол хода завершается собственными спецтокенами модели (они уже
    /// в `eos_ids` бандла), стоп на `<|im_end|>` ему не нужен.
    pub(crate) fn has_native_stops(tokenizer: &LlmTokenizer) -> bool {
        ChannelIds::detect(tokenizer).is_some() || gemma4_ids(tokenizer).is_some()
    }

    /// Очередной токен → (текст ответа, размышления, live-текст tool-вызова).
    pub(crate) fn feed(&mut self, id: u32, delta: &str) -> (String, String, String) {
        match self {
            Self::ChatML { think, tools } => {
                let feed = tools.feed(delta);
                let (body, thinking) = if feed.clean_delta.is_empty() {
                    (String::new(), String::new())
                } else {
                    let split = think.feed(&feed.clean_delta);
                    (split.body, split.thinking)
                };
                (body, thinking, feed.tool_delta)
            }
            Self::Channel(p) => {
                let split = p.feed(id, delta);
                (split.body, split.thinking, split.tool)
            }
            Self::Gemma(p) => {
                let split = p.feed(id, delta);
                (split.body, split.thinking, split.tool)
            }
        }
    }

    /// Модель дописала tool-вызов — стрим можно рвать, не дожидаясь, пока
    /// она уйдёт писать прозу после блока.
    pub(crate) fn tool_call_ready(&self) -> bool {
        match self {
            Self::ChatML { tools, .. } => tools.calls_count() > 0 && tools.is_outside(),
            Self::Channel(p) => p.has_closed_call(),
            Self::Gemma(p) => p.has_closed_call(),
        }
    }

    pub(crate) fn finish(self) -> Vec<RawToolCall> {
        match self {
            Self::ChatML { tools, .. } => tools.finish().0,
            Self::Channel(p) => p.finish(),
            Self::Gemma(p) => p
                .finish()
                .into_iter()
                .map(|c| RawToolCall { arguments_json: c.arguments_json(), name: c.name })
                .collect(),
        }
    }
}

/// Id маркеров протокола Gemma-4 в словаре модели. Признак — каждый маркер
/// кодируется ровно одним токеном; у ChatML-моделей таких строк в словаре
/// нет, и токенайзер разбирает их на куски.
fn gemma4_ids(tokenizer: &LlmTokenizer) -> Option<Gemma4Ids> {
    Gemma4Ids::detect_with(|s| match tokenizer.encode(s) {
        Ok(ids) if ids.len() == 1 => Some(ids[0]),
        _ => None,
    })
}

/// Промпт хода с поправкой под протокол модели: у Gemma-4 при включённых
/// размышлениях — [`gemma4_restore_empty_thinking`]; остальным отдаётся как
/// есть.
pub(crate) fn prompt_for_model(tokenizer: &LlmTokenizer, enable_thinking: bool, prompt: String) -> String {
    if enable_thinking && gemma4_ids(tokenizer).is_some() {
        gemma4_restore_empty_thinking(&prompt)
    } else {
        prompt
    }
}

/// Gemma-4 при включённых размышлениях продолжает ход после результата
/// инструмента с открытого шаблоном `<|channel>thought⏎`; если модель думать
/// не стала, она сразу пишет `<channel|>`, и в токенах хода остаётся пустой
/// канал. Шаблон же пустые размышления не рендерит вовсе, и промпт
/// следующего хода расходится с предыдущим ровно на этом месте — а
/// префикс-KV движка живёт только концом прошлого промпта, так что весь
/// контекст префиллился заново (22 с на 11k ток., живой прогон 09.09.2026).
/// Возвращаем пустой канал после каждого `<tool_response|>`, за которым идёт
/// не размышление и не конец промпта.
fn gemma4_restore_empty_thinking(prompt: &str) -> String {
    const CLOSE: &str = "<tool_response|>";
    const EMPTY_THOUGHT: &str = "<|channel>thought\n<channel|>";
    let mut out = String::with_capacity(prompt.len() + 64);
    let mut rest = prompt;
    while let Some(pos) = rest.find(CLOSE) {
        let after = pos + CLOSE.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        if !tail.is_empty() && !tail.starts_with("<|channel>") && !tail.starts_with("<|tool_response>") {
            out.push_str(EMPTY_THOUGHT);
        }
        rest = tail;
    }
    out.push_str(rest);
    out
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

const EMPTY_TURN_NOTE: &str = "[System note: your previous turn produced      neither a tool call nor a text answer — the whole budget went into      reasoning. If you meant to call a tool, send the call again as valid      JSON: arguments as a real structure, every bracket closed in the right      order. Otherwise answer with text.]";

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
) -> Option<Vec<Message>> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<HistoryItem>>();
    let ctx_main = ctx.clone();
    let sp = system_prompt.to_string();
    let channel = ChannelIds::detect(&model.tokenizer).is_some();
    run_on_main_thread(move || {
        let _ = tx.send(build_history(&ctx_main, &sp, channel));
    });
    let items = rx.await.ok()?;
    let (history, _media) = prepare_history(&items, model, caps, ctx);
    Some(history)
}

async fn run_agent_loop(
    mut model: Arc<LoadedSynModel>,
    items: Vec<HistoryItem>,
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
    let model_cap = model.model.config().max_seq_len;
    let mut total_gen_tokens: u32 = 0;
    // Чистое время декода по всем ходам — без префилла и без пауз на
    // исполнение инструментов. Итоговый tps в панели считается по нему,
    // иначе финальное число не сходится с тем, что бежало вживую.
    let mut total_decode_s: f64 = 0.0;
    let t_overall = Instant::now();

    // Вложения кодируются один раз на весь agent-loop: тексты сообщений с
    // блоками-заполнителями и эмбеддинги дальше переиспользуются на каждом
    // turn'е. Vision-башня нужна только здесь — сразу после кодирования её
    // выгружаем, чтобы KV-ring получил свободную VRAM.
    // Слот держим на весь agent-loop: ходы внутри одного сообщения — главные
    // потребители префикса (каждый tool-вызов раньше требовал полного
    // префилла выросшей истории).
    let mut kv_slot = KV_SLOT.lock().unwrap_or_else(|e| e.into_inner());
    // Vision-башне под НОВЫЕ вложения нужна та же VRAM, что держит кэш
    // диалога: на время кодирования кэш уезжает в RAM и возвращается
    // следом. Раньше он сбрасывался при любой картинке в истории, даже давно
    // закодированной, — и каждое сообщение платило полным префиллом
    // (журнал 11.09.2026: 55k токенов, 40 с на каждый ход).
    let needs_vision = caps.vision
        && items
            .iter()
            .any(|i| attach_prompt::needs_tower(&i.attachments, &caps));
    let parked = needs_vision && park_kv_session(&mut kv_slot, &model);
    // `media` растёт и посреди хода: `view_media` дописывает эмбеддинги
    // файлов, которые модель попросила посмотреть (порядок — порядок
    // заполнителей в промпте, новые всегда в хвосте).
    let (mut history, mut media) = prepare_history(&items, &model, &caps, &ctx);
    if parked {
        unpark_kv_session(&mut kv_slot, &model);
    }
    if !media.is_empty() {
        let tokens: usize = media.iter().map(|m| m.tokens).sum();
        log::info!(
            "[syn_chat] медиа-вложений: {} ({} vision-токенов)",
            media.len(),
            tokens
        );
    }

    // Медиа-промпт продолжает сессию там, где движок держит строки вложений
    // в префиксе: гибрид Qwen3.6/3.8, Muse, Gemma-4, Qwen4Exp (подмену
    // картинки при той же разметке ловит отпечаток строк). Архитектуры без
    // сессии идут полным префиллом.
    let mut prefix_kv_on = (media.is_empty() || model.model.kv_session_media_ok())
        && crate::config::AppConfig::load().syn_chat_prefix_kv;
    if !prefix_kv_on {
        *kv_slot = None;
    }
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

        let prompt = {
            let with_note;
            let for_prompt: &[Message] = match pending_note.take() {
                Some(note) => {
                    let mut h = history.clone();
                    h.push(Message::user(note));
                    with_note = h;
                    &with_note
                }
                None => &history,
            };
            let rendered = model.tokenizer.apply_chat_template_reasoning(
                for_prompt,
                true,
                params.enable_thinking,
                params.effort(),
                if tool_schemas.is_empty() {
                    None
                } else {
                    Some(&tool_schemas)
                },
            )?;
            prompt_for_model(&model.tokenizer, params.enable_thinking, rendered)
        };
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
        // Префикс-KV на этом ходу. Гаснет после OOM: кэш сессии — самый
        // крупный кусок, который ход может отдать, и пересоздавать его тут же
        // бессмысленно (ровно это и делал ретрай до 04.09.2026: сбрасывал
        // 416 МБ, `ensure_kv_slot` немедленно брал их обратно, и все три
        // попытки падали на той же аллокации).
        let mut prefix_kv_turn = prefix_kv_on;
        // Пишется удавшейся попыткой хода; ретраи по OOM до присваивания не
        // доходят, поэтому ни инициализатор, ни `mut` не нужны.
        let turn_decode_s: f64;
        let turn_result = loop {
            // Припаркованный в RAM кэш VRAM не держит — его размер в бюджет
            // не идёт ни как «освободится под новый», ни как «уже выделено».
            let session_held_mb = kv_slot
                .as_ref()
                .filter(|s| !s.session.is_parked())
                .map(|s| {
                    (s.session.ctx_tokens() * model.model.kv_bytes_per_token()) / (1024 * 1024)
                })
                .unwrap_or(0);
            // Контекст важнее скорости: если ход в память не помещается,
            // часть блоков уезжает на хост и стримится, освобождая VRAM под
            // KV. Считаем по тому, что ходу реально нужно — промпт плюс
            // бюджет ответа, минус уже занятый кэш префикс-KV.
            fit_blocks_for_context(
                &model,
                prompt_ids.len() + answer_budget + 128,
                session_held_mb,
            );
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
            if anti_loop {
                // Повтор уже случился. Greedy-декод из петли не выходит
                // никогда, поэтому поднимаем температуру — и только её:
                // штрафы за повторы на агентном промпте ломают синтаксис
                // вызова (см. [`ANTI_LOOP_TEMPERATURE`]). Настройку
                // пользователя только повышаем, но не понижаем. Режим
                // снимается первым же неповторённым вызовом.
                opts.temperature = opts.temperature.max(ANTI_LOOP_TEMPERATURE);
                log::info!(
                    "[syn_chat] анти-loop сэмплинг: temp={:.2}",
                    opts.temperature
                );
            }
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
                // Движок историю НЕ подрезает: без sliding-window он вернёт
                // «KV overflow», а со скользящим окном молча выбросит голову
                // контекста — то есть системный промпт и саму задачу.
                log::warn!(
                    "[syn_chat] промпт {} ток не влезает в ринг {}: движок либо \
                     упадёт с KV overflow, либо потеряет голову контекста \
                     (системный промпт и задачу). Сожмите чат или уменьшите \
                     max_new_tokens",
                    prompt_ids.len(),
                    plan.ring_tokens
                );
            }
            let mut runner = LlmGeneration::new(&model.model, opts);
            let channel_mode = ChannelIds::detect(&model.tokenizer).is_some();
            if StreamParser::has_native_stops(&model.tokenizer) {
                // Канальный протокол завершает ход `<|eot|>`, Gemma-4 —
                // `<turn|>`/`<|tool_response>`; всё это уже в eos_ids бандла,
                // своих стопов добавлять не нужно (и `<|im_end|>` в этих
                // словарях всё равно нет).
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
            let mut parser =
                StreamParser::for_model(&model.tokenizer, params.enable_thinking, &prompt);
            // Текст ответа за этот turn вне tool_call-блоков. Реплика для
            // истории собирается из него и разобранных вызовов
            // (`assistant_turn_message`) — сырой поток модели в историю не идёт.
            let mut clean_text: String = String::new();
            // Размышления хода — уходят в реплику истории блоком `<think>`:
            // шаблон Qwen3 рендерит их для реплик после последнего вопроса
            // пользователя, и без них следующий промпт расходится с
            // предыдущим на границе `<think>⏎` (см. `assistant_turn_message`).
            let mut think_text: String = String::new();
            let mut buf_body = String::new();
            let mut buf_think = String::new();
            let mut buf_tool = String::new();
            let mut last_flush = Instant::now();
            let flush_interval = Duration::from_millis(FLUSH_INTERVAL_MS);
            let mut tokens_this_turn: u32 = 0;
            // Честный prefill: время до ПЕРВОГО токена. Прежний замер стоял до
            // старта генерации и показывал в панели 0 — время планирования ринга.
            let mut ttft_ms: Option<u32> = None;
            // Хвост текста хода (символы) — для guard'а зацикливания;
            // `looped` — найденный период, по нему стрим и оборван.
            let mut turn_tail: Vec<char> = Vec::new();
            let mut looped: Option<usize> = None;

            let t_turn = Instant::now();
            // Копия, а не захват `total_gen_tokens`: он дописывается уже
            // после хода, а замыкание живёт только внутри стрима.
            let gen_before = total_gen_tokens;
            let abort_for_cb = abort.clone();
            let chat_for_cb = chat_id.clone();
            let on_token = |id: u32, delta: &str| {
                // Abort: сбросить накопленные буферы и выйти.
                if abort_for_cb.load(Ordering::Relaxed) != abort_snapshot {
                    flush_streaming(&chat_for_cb, &mut buf_body, &mut buf_think, &mut buf_tool, None);
                    return false;
                }
                if ttft_ms.is_none() {
                    ttft_ms = Some(t_turn.elapsed().as_millis() as u32);
                }
                tokens_this_turn += 1;
                turn_tail.extend(delta.chars());
                if turn_tail.len() > LOOP_WINDOW_CHARS * 2 {
                    let extra = turn_tail.len() - LOOP_WINDOW_CHARS;
                    turn_tail.drain(..extra);
                }
                if tokens_this_turn as usize % LOOP_CHECK_EVERY == 0 {
                    if let Some(p) = periodic_tail(&turn_tail, LOOP_WINDOW_CHARS, LOOP_MAX_PERIOD_CHARS) {
                        looped = Some(p);
                        flush_streaming(&chat_for_cb, &mut buf_body, &mut buf_think, &mut buf_tool, None);
                        return false;
                    }
                }

                let (body, thinking, tool) = parser.feed(id, delta);
                clean_text.push_str(&body);
                think_text.push_str(&thinking);
                buf_body.push_str(&body);
                buf_think.push_str(&thinking);
                buf_tool.push_str(&tool);

                // Throttled flush в UI.
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
            let session = if prefix_kv_turn {
                ensure_kv_slot(
                    &mut kv_slot,
                    &model,
                    &chat_id,
                    plan.ring_tokens,
                    plan.session_ctx,
                    plan.max_new,
                )
            } else {
                None
            };
            let media_refs: Vec<&MediaEmbedding> = media.iter().collect();
            let stream_res = match (session, media_refs.is_empty()) {
                (Some(session), false) => runner
                    .generate_streaming_cached_media(
                        session,
                        &prompt_ids,
                        &model.tokenizer,
                        &media_refs,
                        on_token,
                    )
                    .map(|n| reused = n),
                (None, false) => runner.generate_streaming_media(
                    &prompt_ids,
                    &model.tokenizer,
                    &media_refs,
                    on_token,
                ),
                (Some(session), true) => runner
                    .generate_streaming_cached(session, &prompt_ids, &model.tokenizer, on_token)
                    .map(|n| reused = n),
                (None, true) => {
                    runner.generate_streaming(&prompt_ids, &model.tokenizer, on_token)
                }
            };

            // Финальный flush — гарантированно сбрасываем хвост буферов.
            flush_streaming(
                &chat_id,
                &mut buf_body,
                &mut buf_think,
                &mut buf_tool,
                Some(live_gen(gen_before, tokens_this_turn, ttft_ms, t_turn)),
            );

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
                // Сессия как резерв считается только если ход ею пользовался:
                // после первого OOM префикс-KV на этом ходу уже выключен.
                let has_session = prefix_kv_turn && kv_slot.is_some();
                let retryable = oom
                    && tokens_this_turn == 0
                    && oom_attempt < MAX_OOM_RETRIES
                    && (has_session || answer_budget > MIN_ANSWER_TOKENS);
                if retryable {
                    oom_attempt += 1;
                    // Первым делом двигаем кэш экспертов: он перечитывается из
                    // бандла за миллисекунды, а посчитанный префикс-KV стоит
                    // секунд десять полного префилла. Пока порядок был
                    // обратный, ход платил самым дорогим, что у него было.
                    // Только на первой попытке: кэш экспертов готов отдавать
                    // сколько угодно раз подряд, и без этого условия все
                    // ретраи ушли бы в него, так и не дойдя до более крупных
                    // резервов.
                    let gave_mb = if oom_attempt == 1 {
                        synaptix::facade::llm::cuda_reclaim_mb(0, RECLAIM_ON_OOM_MB)
                    } else {
                        0
                    };
                    if gave_mb > 0 {
                        log::warn!(
                            "[syn_chat] OOM на ринге {} ток ({} MB): {e}. Повтор {}/{}: \
                             отдаваемые кэши вернули {gave_mb} MB, кэш префикс-KV цел",
                            plan.ring_tokens,
                            plan.ring_mb(),
                            oom_attempt,
                            MAX_OOM_RETRIES,
                        );
                        if_active(&chat_id, |c| {
                            c.streaming_body.set(String::new());
                            c.streaming_thinking.set(String::new());
                            c.streaming_tool.set(String::new());
                        });
                        continue;
                    }
                    if has_session {
                        // Главный резерв: кэш префикс-KV держит гигабайты в
                        // пуле активаций, и пока он жив, forward'у может не
                        // хватать места под рабочие буферы (наблюдали OOM на
                        // alloc в 8 MB при живой сессии на 3.9 GB). Сбрасываем
                        // и переигрываем ход БЕЗ него: полный префилл дороже
                        // по времени, но дешевле по памяти, а пересоздание
                        // кэша тут же вернуло бы ход в ту же аллокацию.
                        let held_mb = kv_slot
                            .as_ref()
                            .map(|s| {
                                (s.session.ctx_tokens() * model.model.kv_bytes_per_token())
                                    / (1024 * 1024)
                            })
                            .unwrap_or(0);
                        *kv_slot = None;
                        prefix_kv_turn = false;
                        let (freed, _) =
                            crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
                        log::warn!(
                            "[syn_chat] OOM на ринге {} ток ({} MB): {e}. Повтор {}/{}: \
                             сброшен кэш префикс-KV ({held_mb} MB, трим вернул {freed} MB), \
                             ход доигрывается без префикс-KV",
                            plan.ring_tokens,
                            plan.ring_mb(),
                            oom_attempt,
                            MAX_OOM_RETRIES,
                        );
                    } else {
                        answer_budget = (answer_budget / 2).max(MIN_ANSWER_TOKENS);
                        log::warn!(
                            "[syn_chat] OOM на ринге {} ток ({} MB): {e}. Повтор {}/{} \
                             с бюджетом ответа {} ток",
                            plan.ring_tokens,
                            plan.ring_mb(),
                            oom_attempt,
                            MAX_OOM_RETRIES,
                            answer_budget
                        );
                    }
                    if_active(&chat_id, |c| {
                        c.streaming_body.set(String::new());
                        c.streaming_thinking.set(String::new());
                        c.streaming_tool.set(String::new());
                    });
                    continue;
                }
                log::error!("[syn_chat] генерация оборвалась: {e:#}");
                return Err(e.into());
            }

            KERNEL_CACHES_WARM.store(true, Ordering::Relaxed);
            if let Some(period) = looped {
                // Ход отброшен целиком: в историю и ленту он не попал, так
                // что префикс-KV цел, а повтор идёт с заметкой и анти-loop
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
                    // Пустой пузырь этого хода снимаем — он выглядит как
                    // «повисло»; после tool-результата снимать нечего.
                    ledger_update(&chat_id, pop_empty_assistant);
                    break 'agent;
                }
                loop_retried = true;
                anti_loop = true;
                pending_note = Some(LOOP_NOTE);
                continue 'agent;
            }
            loop_retried = false;
            let dt = t_turn.elapsed();
            turn_decode_s = (dt.as_secs_f64() - ttft_ms.unwrap_or(0) as f64 / 1000.0).max(0.0);
            let tok_per_s = if dt.as_secs_f64() > 0.0 {
                tokens_this_turn as f64 / dt.as_secs_f64()
            } else {
                0.0
            };
            // Имя приводим к ключу каталога сразу на выходе парсера: дальше
            // оно идёт и в историю (`<atem:invoke name=…>` следующего
            // промпта), и в guard, и в политику подтверждений, и в UI.
            // Канальный шаблон Muse разрешает модели `notes.action` — если
            // это имя доживёт до истории, модель увидит его в своём же
            // прошлом ходу и повторит.
            let raw_calls: Vec<RawToolCall> = parser
                .finish()
                .into_iter()
                .map(|mut c| {
                    c.name = crate::agent::tools::executor::canonical_tool_name(&c.name).to_string();
                    c
                })
                .collect();
            reused_total = reused_total.max(reused as u32);
            if let Some(slot) = kv_slot.as_mut() {
                if reused == 0 && !slot.last_prompt.is_empty() {
                    log_prefix_divergence(&model.tokenizer, &slot.last_prompt, &prompt_ids);
                }
                slot.last_prompt = prompt_ids.clone();
            }
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
                blocks_resident: blocks_resident_of(&model),
            };
            if_active(&chat_id, move |c| stat.apply(c));

            break (
                clean_text,
                think_text,
                raw_calls,
                tokens_this_turn,
                channel_mode,
                // Честный потолок контекста этого хода — нужен
                // компактификации внутри хода.
                plan.by_mem.min(plan.cap),
            );
        };
        let (clean_text, think_text, raw_calls, tokens_this_turn, channel_mode, turn_ctx_budget) =
            turn_result;

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
            prompt_ids.len() + tokens_this_turn as usize,
            tools::budget::model_counter(&model),
        );

        // Выполняем каждый tool: guard повторов → confirm → execute → push.
        for chat_call in chat_calls.iter() {
            if abort.load(Ordering::Relaxed) != abort_snapshot {
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

            // ── pipelines action=run — особый путь: прогон завязан на
            // жизненный цикл LLM (free_vram выгружает её и грузит обратно),
            // поэтому исполняется обёрткой цикла, а не tools::execute.
            if let Some(req) = crate::syn_chat::pipeline_run::parse_run_call(chat_call) {
                let res = run_pipeline_tool(
                    req,
                    model,
                    &mut kv_slot,
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

            // ── Субагент крутит свой agent-loop на той же модели и просит
            // под него собственный KV-ринг. Пока кэш префикс-KV этого хода
            // жив, его гигабайты остаются в VRAM, и субагенту не хватает
            // места уже на активациях MoE — в ленту вместо ответа приходит
            // «alloc_zeros(...): OOM». Отправляем кэш в RAM на время вызова
            // и забираем обратно: перевоз через PCIe — десятки мс, полный
            // префилл истории — секунды.
            // ── view_media: файл кодирует vision-башня этой же модели, и его
            // эмбеддинги входят в промпт следующего хода — поэтому исполняет
            // цикл, а не `tools::execute` (у того нет ни модели, ни медиа).
            let mut viewed: Option<ViewedMedia> = None;
            let outcome = if tools::executor::canonical_tool_name(&tool_name(chat_call))
                == KEY_VIEW_MEDIA
            {
                let v = view_media_call(chat_call, &model, &caps, &mut kv_slot).await;
                let outcome = v.outcome.clone();
                viewed = Some(v);
                outcome
            } else {
                let parked = tool_name(chat_call) == KEY_SUBAGENT
                    && park_kv_session(&mut kv_slot, &model);

                // Исполнение с возможностью прерывания на длинных tool'ах
                // (web fetch может висеть 30+ сек).
                let outcome = tokio::select! {
                    o = tools::execute(chat_call) => o,
                    _ = wait_abort(&abort, abort_snapshot) => {
                        if parked {
                            unpark_kv_session(&mut kv_slot, &model);
                        }
                        return Ok(());
                    }
                };
                if parked {
                    unpark_kv_session(&mut kv_slot, &model);
                }
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
                        media.extend(v.media);
                        if prefix_kv_on && !model.model.kv_session_media_ok() {
                            log::info!(
                                "[syn_chat] view_media: у модели нет сессии с медиа — \
                                 префикс-KV до конца хода выключен"
                            );
                            prefix_kv_on = false;
                            *kv_slot = None;
                        }
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
        if autocompact_enabled && media.is_empty() && turn_ctx_budget > 0 {
            let prompt_tokens = prompt_ids.len() as u32;
            let pct = prompt_tokens as u64 * 100 / turn_ctx_budget as u64;
            if pct >= autocompact_threshold as u64 {
                // Сводку считает та же модель: освобождаем кэш префикс-KV
                // ДО summary-запроса — после сжатия он всё равно не совпадёт
                // с новой историей, а его VRAM нужна ринг-буферу сводки.
                if kv_slot.is_some() {
                    *kv_slot = None;
                    crate::syn_chat::model_registry::reclaim_vram(*model.model.device());
                }
                let compacted = crate::syn_chat::compact::maybe_compact_in_turn(
                    &model,
                    &abort,
                    abort_snapshot,
                    autocompact_threshold,
                    IN_TURN_KEEP_TAIL,
                    prompt_tokens,
                    turn_ctx_budget as u32,
                )
                .await;
                if compacted {
                    match rebuild_history(&ctx, &sys_prompt, &model, &caps).await {
                        Some(h) => {
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
    let prompt = model
        .tokenizer
        .apply_chat_template_ex_tools(&history, true, false, None)?;
    let prompt_ids = model.tokenizer.encode(&prompt)?;
    let model_cap = model.model.config().max_seq_len;
    let plan = RingPlan::new(model, prompt_ids.len(), SUMMARY_MAX_NEW_TOKENS, model_cap);
    log::info!(
        "[syn_chat] summary-запрос: промпт {} ток, ринг {} ток ({} MB)",
        prompt_ids.len(),
        plan.ring_tokens,
        plan.ring_mb()
    );

    // Низкая температура: сводка должна быть детерминированной и сухой.
    let mut params = SamplingParams::default();
    params.temperature = 0.3;
    let mut opts = params.to_options();
    opts.max_seq_len = plan.ring_tokens;
    opts.max_new_tokens = plan.max_new;

    let mut runner = LlmGeneration::new(&model.model, opts);
    if StreamParser::has_native_stops(&model.tokenizer) {
        runner.set_stop_tokens(model.tokenizer.eos_ids().to_vec());
    } else {
        set_qwen3_stops(&mut runner, &model.tokenizer);
    }

    let mut parser = StreamParser::for_model(&model.tokenizer, false, &prompt);
    let mut clean = String::new();
    let abort_cb = abort.clone();
    let res = runner.generate_streaming(&prompt_ids, &model.tokenizer, |id, delta| {
        if abort_cb.load(Ordering::Relaxed) != abort_snapshot {
            return false;
        }
        let (body, _thinking, _tool) = parser.feed(id, delta);
        clean.push_str(&body);
        true
    });

    // Ринг summary-запроса больше не нужен — возвращаем VRAM (см. комментарий
    // про cudaMallocAsync-пул в run_agent_loop).
    drop(runner);
    let _ = crate::syn_chat::model_registry::reclaim_vram(*model.model.device());

    res?;
    Ok(clean.trim().to_string())
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
    kv_slot: &mut std::sync::MutexGuard<'_, Option<KvSlot>>,
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

    // 2. free_vram: освободить всё, что держит ход.
    let mut model_opt = Some(model);
    let mut model_path: Option<std::path::PathBuf> = None;
    if req.free_vram {
        let m = model_opt.take().expect("model взята выше");
        model_path = Some(m.path.clone());
        let device = *m.model.device();
        // Проверка после выгрузки: если кто-то в ходе ещё держит клон модели,
        // веса остаются на карте, а прогон упадёт в OOM (так было с бюджетом
        // инструментов, державшим `Arc<LoadedSynModel>`).
        let weak = Arc::downgrade(&m);
        **kv_slot = None;
        drop(m);
        let (tx, rx) = tokio::sync::oneshot::channel();
        run_on_main_thread(move || {
            use_context::<SynModelRegistry>().unload();
            let _ = tx.send(());
        });
        let _ = rx.await;
        let alive = weak.strong_count();
        if alive > 0 {
            log::warn!(
                "[syn_chat] pipelines run: LLM НЕ освобождена — живых ссылок {alive}, \
                 веса остаются в VRAM"
            );
        }
        let (freed, descs) = crate::syn_chat::model_registry::reclaim_vram(device);
        log::info!(
            "[syn_chat] pipelines run: LLM выгружена (+{} MB, {} дескрипторов), \
             VRAM доступно {} MB",
            freed,
            descs,
            crate::syn_chat::model_registry::vram_available_mb()
        );
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
        let app_ctx = use_context::<AppCtx>();
        let policy = crate::config::resolve_model_profile(
            &app_ctx.model_profiles.get_untracked(),
            &path,
        )
        .policy;
        use_context::<SynModelRegistry>().load_with_notify(path, policy, tx);
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
    /// Эмбеддинги картинок и видео среди них — в хвост медиа хода.
    media: Vec<MediaEmbedding>,
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
    model: &Arc<LoadedSynModel>,
    caps: &MediaCaps,
    kv_slot: &mut std::sync::MutexGuard<'_, Option<KvSlot>>,
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

    let ready: Vec<MsgAttachment> = ingested
        .iter()
        .filter_map(|(_, r)| r.as_ref().ok().cloned())
        .collect();
    let needs_vision = caps.vision && attach_prompt::needs_tower(&ready, caps);
    let parked = needs_vision && park_kv_session(kv_slot, model);
    let tower = attach_prompt::ensure_tower(&model.model, needs_vision);
    let mut caps = MediaCaps { vision: tower.is_ok(), ..caps.clone() };
    if let Err(reason) = &tower {
        // Пустая причина — башня и не требовалась (всё в кэше).
        if !reason.is_empty() {
            caps.vision_error = Some(reason.clone());
        }
    }

    // Половина остатка окна на весь результат — как у остальных
    // инструментов (`budget::grant`); вне цикла бюджета нет.
    let grant = tools::budget::grant(usize::MAX);
    let mut allowed = if grant.measured { grant.tokens } else { usize::MAX };
    let mut entries: Vec<Entry> = Vec::new();
    let mut attachments: Vec<MsgAttachment> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut media: Vec<MediaEmbedding> = Vec::new();
    for (path, r) in ingested {
        let a = match r {
            Ok(a) => a,
            Err(why) => {
                entries.push(Entry { path, status: Status::Failed(why) });
                continue;
            }
        };
        let part = attach_prompt::attachment_part(&a, &model.model, &caps);
        let tokens = part
            .media
            .as_ref()
            .map(|m| m.tokens)
            .unwrap_or_else(|| tools::budget::count(&part.text));
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
    if tower.is_ok() {
        attach_prompt::release_tower(&model.model);
    }
    if parked {
        unpark_kv_session(kv_slot, model);
    }

    let shown = entries.iter().filter(|e| e.shown()).count();
    log::info!(
        "[syn_chat] view_media: показано {shown} из {} файлов, медиа {} ({} vision-токенов)",
        entries.len(),
        media.len(),
        media.iter().map(|m| m.tokens).sum::<usize>()
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

/// Лейблы активных инструментов для системного промпта — в том же порядке,
/// в каком они уходят в tool-схемы.
fn active_tool_labels(app: &AppCtx) -> Vec<String> {
    declared_tool_labels(&app.tools.active.get_untracked())
}

/// Лейблы тех ключей, что уходят модели схемой: инструменты каталога, кроме
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
fn pool_tool_labels(app: &AppCtx) -> Vec<String> {
    tools::autotools::pool(&app.tools.active.get_untracked(), &app.tools.auto.get_untracked())
        .iter()
        .map(|t| t.label.to_string())
        .collect()
}

/// Собирает JSON-схемы активных инструментов (для передачи в Jinja-шаблон
/// Qwen3 или manual prefix). Особые случаи — `autoskill`, описание которого
/// расширяется актуальным списком скилов через [`build_autoskill_chat_tool`],
/// и пул: он уходит одной схемой `autotools` в конце списка, а схемы самих
/// инструментов из пула модель получает ответом `autotools`.
fn collect_active_tool_schemas(app: &AppCtx) -> Vec<serde_json::Value> {
    use crate::agent::tools::catalog::KEY_AUTOSKILL;
    let keys = app.tools.active.get_untracked();
    let mut schemas: Vec<serde_json::Value> = keys
        .iter()
        .filter_map(|k| {
            let tool = Tool::by_key(k).filter(|t| !t.is_implicit())?;
            let t = if k == KEY_AUTOSKILL {
                crate::agent::tool_flow::build_autoskill_chat_tool(app)
            } else {
                tool.to_chat_tool()
            };
            serde_json::to_value(&t).ok()
        })
        .collect();
    if let Some(t) = crate::agent::tool_flow::build_autotools_chat_tool(app) {
        schemas.extend(serde_json::to_value(&t).ok());
    }
    schemas
}

/// Полный набор stop-токенов для Qwen3 ChatML: EOS из конфига + `<|im_end|>`.
pub(crate) fn set_qwen3_stops(runner: &mut LlmGeneration<'_>, tokenizer: &LlmTokenizer) {
    let mut stops: Vec<u32> = tokenizer.eos_ids().to_vec();
    if StreamParser::has_native_stops(tokenizer) {
        // Muse Glimmer / Gemma-4: `<|im_end|>` в словаре нет, ход закрывают
        // их собственные токены из eos_ids.
        runner.set_stop_tokens(stops);
        return;
    }
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
    /// Размышления и вызовы реплики ассистента с инструментами
    /// (см. [`assistant_turn_message`]); у остальных реплик пусты.
    reasoning: String,
    calls: Vec<RawToolCall>,
}

impl HistoryItem {
    fn has_media(&self) -> bool {
        !self.attachments.is_empty()
    }
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
    let window = model.model.config().max_seq_len.min(max_seq_len as usize);
    MediaCaps {
        // Кэш, а не Llm::supports_media(): функция зовётся с main thread, а
        // мьютекс пайплайна занят на всё время идущей генерации.
        vision: model.supports_media,
        vision_error: None,
        max_image_tokens: (max > 0).then_some(max),
        asr: Some(app.audio.asr.clone()),
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
            let msg = tr!("chat.session.error.attachment_rejected", reason = reason);
            let ctx = ctx.clone();
            run_on_main_thread(move || ctx.error.set(Some(msg)));
            caps.vision_error = Some(reason);
        }
    }

    let mut out: Vec<Message> = Vec::with_capacity(items.len());
    let mut media: Vec<MediaEmbedding> = Vec::new();
    for item in items {
        if let Some(name) = &item.tool_name {
            if item.attachments.is_empty() {
                out.push(Message::tool_named(name.as_str(), item.body.as_str()));
            } else {
                // `view_media`: блоки файлов перед телом, как в ходе.
                let prepared = attach_prompt::prepare_tool_view(
                    &item.body,
                    &item.attachments,
                    &model.model,
                    &caps,
                );
                media.extend(prepared.media);
                out.push(Message::tool_named(name.as_str(), prepared.text));
            }
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
            ChatMsgRole::Assistant if !item.calls.is_empty() || !item.reasoning.is_empty() => {
                out.push(assistant_turn_message(&item.body, &item.reasoning, &item.calls))
            }
            ChatMsgRole::Assistant => out.push(Message::assistant(&item.body)),
            ChatMsgRole::System => out.push(Message::system(&item.body)),
        }
    }
    if vision_ready {
        attach_prompt::release_tower(&model.model);
    }
    (out, media)
}


#[cfg(test)]
mod tests {
    use super::*;

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

        let bundle = std::path::Path::new("/home/master/Storage/syn_models/gemma-4-26b-a4b-it.syn");
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

    /// Пустой канал размышлений возвращается только там, где после
    /// результата инструмента модель продолжила ход без размышлений; конец
    /// промпта и непустые размышления не трогаются.
    #[test]
    fn gemma4_empty_thinking_restored_after_tool_response() {
        let p = "<|tool_call>call:bash{}<tool_call|><|tool_response>response:bash{value:<|\"|>ok<|\"|>}<tool_response|>Ответ.<turn|>\n<|turn>user\nещё<turn|>\n<|turn>model\n";
        let fixed = gemma4_restore_empty_thinking(p);
        assert_eq!(
            fixed,
            "<|tool_call>call:bash{}<tool_call|><|tool_response>response:bash{value:<|\"|>ok<|\"|>}<tool_response|><|channel>thought\n<channel|>Ответ.<turn|>\n<|turn>user\nещё<turn|>\n<|turn>model\n"
        );
        // Конец промпта (генерация начнётся с `<|channel>thought⏎` от шаблона).
        let p = "…<tool_response|><|channel>thought\n";
        assert_eq!(gemma4_restore_empty_thinking(p), p);
        let p = "…<tool_response|>";
        assert_eq!(gemma4_restore_empty_thinking(p), p);
        // Непустые размышления шаблон отрендерил сам.
        let p = "…<tool_response|><|channel>thought\nмысль\n<channel|>Ответ.";
        assert_eq!(gemma4_restore_empty_thinking(p), p);
        // Несколько результатов подряд (несколько вызовов в одном ходе).
        let p = "…<tool_response|><|tool_response>response:x{value:1}<tool_response|><|tool_call>call:y{}<tool_call|>";
        assert_eq!(
            gemma4_restore_empty_thinking(p),
            "…<tool_response|><|tool_response>response:x{value:1}<tool_response|><|channel>thought\n<channel|><|tool_call>call:y{}<tool_call|>"
        );
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

    /// План хода для qwen3.8-flash-next: 13440 Б/ток KV, окно 262144 и
    /// ≈1.7 ГБ постоянных ring-окон sliding-слоёв. С этими числами `compute`
    /// повторяет журнал разобранной сессии токен в токен.
    fn plan(prompt: usize, answer: usize, vram_mb: usize) -> RingPlan {
        RingPlan::compute(prompt, answer, 262_144, 13_440, 1_761_830_912, vram_mb, false)
    }

    /// При стриминге блоков сессия не удваивается: ровно под ход (с шагом
    /// `SESSION_CTX_STEP`), даже если VRAM формально позволяет больше.
    #[test]
    fn session_is_not_doubled_while_blocks_are_streamed() {
        let resident = plan(20_000, 8_192, 20_000);
        let streamed = RingPlan::compute(20_000, 8_192, 262_144, 13_440, 1_761_830_912, 20_000, true);
        assert_eq!(resident.ring_tokens, streamed.ring_tokens);
        assert!(streamed.session_ctx < resident.session_ctx, "{} vs {}", streamed.session_ctx, resident.session_ctx);
        assert!(streamed.session_ctx >= streamed.ring_tokens);
    }

    #[test]
    fn ring_does_not_grow_with_a_bigger_budget() {
        // Ход требует столько же независимо от того, сколько VRAM свободно:
        // промпт и бюджет ответа те же.
        let tight = plan(9_045, 11_307, 5_099);
        let roomy = plan(9_045, 11_307, 20_000);
        assert_eq!(tight.ring_tokens, roomy.ring_tokens);
    }

    #[test]
    fn session_from_a_tight_budget_survives_a_bigger_one() {
        // Разобранный случай из журнала: сессию выдали при скудном бюджете
        // (by_mem 27056), на следующем ходу памяти стало больше и `session_ctx`
        // подскочил до 32768. Пересоздавать кэш из-за этого нельзя — ход
        // заплатил бы полным префиллом, хотя старой ёмкости хватало.
        let tight = plan(9_045, 11_307, 5_099);
        assert_eq!(tight.session_ctx, 27_056, "ожидали ёмкость из журнала");
        let roomy = plan(9_112, 11_240, 24_000);
        assert!(
            tight.session_ctx >= roomy.ring_tokens,
            "сессия на {} ток не покрывает ход на {} ток",
            tight.session_ctx,
            roomy.ring_tokens
        );
        assert!(
            roomy.session_ctx > tight.session_ctx,
            "тест бессмыслен: при большем бюджете ёмкость обязана расти ({} → {})",
            tight.session_ctx,
            roomy.session_ctx
        );
    }

    #[test]
    fn session_is_recreated_when_the_turn_outgrows_it() {
        // А вот когда ходу действительно не хватает — пересоздание законно.
        let small = plan(8_481, 384, 1_067);
        let bigger = plan(8_889, 384, 4_459);
        assert!(small.session_ctx < bigger.ring_tokens);
    }

    /// Журнал 07.09.2026, qwen3.8-27b: 64 блока по 229 МБ, живой кэш
    /// префикс-KV на 30748 ток (1921 МБ) — ход считает прямо в нём и новой
    /// памяти под KV не просит. Пока кэш не вычитался из потребности, ход на
    /// промпте 10k видел «не хватает 2 МБ», выселял два блока и терял
    /// 32 → 20 ток/с.
    #[test]
    fn a_live_prefix_kv_cache_does_not_evict_blocks() {
        assert_eq!(block_residency_target(2431 - 1921, 2429, || 2429, 229, 64, 64), None);
        assert!(
            block_residency_target(2431, 2429, || 2429, 229, 64, 64).is_some(),
            "регресс: без учёта кэша ход выселял блоки"
        );
    }

    /// И обратно: после такого выселения блоки не возвращались никогда —
    /// свободных 2835 МБ при потребности 2431 давали запас ровно в один блок,
    /// а гистерезис возврата требует двух.
    #[test]
    fn blocks_come_back_once_the_cache_is_counted() {
        assert_eq!(block_residency_target(2431, 2835, || 2835, 229, 62, 64), None);
        assert_eq!(block_residency_target(2431 - 1921, 2835, || 2835, 229, 62, 64), Some(64));
    }

    #[test]
    fn a_turn_that_really_does_not_fit_still_offloads() {
        // Кэш на 1921 МБ жив, но контекст вырос: ходу нужно ещё 2500 МБ при
        // 1000 свободных и бесполезном триме — выселяем 1500/229 + 1 = 8.
        assert_eq!(block_residency_target(2500, 1000, || 1000, 229, 64, 64), Some(56));
    }

    /// Журнал 07.09.2026: «не хватало 2 MB … трим вернул 384 MB» — трим
    /// шёл ПОСЛЕ выселения двух блоков. Теперь он идёт до решения и такой
    /// дефицит закрывает сам; а если не закрыл — за дефицит меньше блока
    /// уезжает один блок, а не «один плюс один на стриминг».
    #[test]
    fn trim_runs_before_eviction_and_a_tiny_deficit_costs_one_block_at_most() {
        assert_eq!(block_residency_target(2431, 2429, || 2429 + 384, 229, 64, 64), None);
        assert_eq!(block_residency_target(2431, 2429, || 2429, 229, 64, 64), Some(63));
        // Дефицит в блок и больше — по-прежнему с запасом на стриминг.
        assert_eq!(block_residency_target(2429 + 229, 2429, || 2429, 229, 64, 64), Some(62));
        // Трим не зовётся, когда памяти и так хватает.
        assert_eq!(block_residency_target(100, 2429, || unreachable!(), 229, 64, 64), None);
    }

    /// Шаг A2 плана 07.09.2026 — «ёмкость кэша префикс-KV (удвоение `want`)
    /// не должна выселять блоки». После того как живой кэш вычитается из
    /// потребности хода, размер кэша из решения выпадает алгебраически:
    /// `need − free = want·kv + reserve − available` при любом `session_ctx`.
    /// Тест держит это как инвариант — если кто-то перестанет вычитать кэш,
    /// удвоение снова начнёт стоить блоков.
    #[test]
    fn session_capacity_does_not_change_the_eviction_decision() {
        // Журнал 07.09: промпт 10108, ответ 8192, 65536 Б/ток, доступно с
        // учётом кэша 4350 МБ (2429 свободно + 1921 в кэше), блок 229 МБ.
        let kv = 65_536usize;
        let want = 10_108 + 8_192 + 128;
        let available = 2_429 + 1_921;
        let need_total = want * kv / (1024 * 1024) + kv_reserve_mb();
        let decision = |session_tokens: usize| {
            let held = session_tokens * kv / (1024 * 1024);
            let free = available - held;
            block_residency_target(need_total.saturating_sub(held), free, || free, 229, 64, 64)
        };
        assert_eq!(decision(want), decision(want * 2));
        assert_eq!(decision(want), decision(0));
        assert_eq!(decision(want), None);
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
        same.extend(std::iter::repeat(7).take(1100));
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
