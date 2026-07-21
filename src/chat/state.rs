//! Реактивное состояние чатов + конструктор `ChatCtx`.
//!
//! `ChatCtx` — `Clone` (не `Copy`), потому что внутри живёт
//! `Arc<AtomicU64>` для потокобезопасного abort. Все остальные поля —
//! `RwSignal<T>` (`Copy`), так что клонирование самого `ChatCtx` дешёвое
//! (инкремент `Arc` + копирование id-шников сигналов). В виджетах мы
//! обычно делаем `let chat = use_context::<AppCtx>().chat.clone();` — это
//! атомарный move из cloned-копии `AppCtx`, никакого аллоцирования.
//!
//! Поле `system_prompt` — это тот же сигнал, что лежит в
//! [`crate::context::GeneralCtx::system_prompt`] (одна истина, persist
//! через общий `install_config_autosave`).
//!
//! Abort реализован как `Arc<AtomicU64>`, а не `RwSignal<u64>`: сигналы
//! syngui — thread-local, а async-драйвер стрима крутится на воркере
//! tokio и не может читать `RwSignal` напрямую. Атомарный счётчик
//! безопасен на любом потоке и достаточен, т.к. UI на его значение
//! не подписывается (abort только пишется).
//!
//! Мульти-чат: UI-слой видит набор метаданных [`ChatMeta`] (сигнал `chats`)
//! + id текущего активного ([`active_chat_id`]). Полная лента сообщений
//! активного чата живёт в `messages`; при переключении чата `registry::select`
//! перегружает её из `storage`.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use syngui::prelude::*;
use serde::{Deserialize, Serialize};

use crate::llama::api::ChatToolCall;

use super::time::format_hm_now;

// ─────────────────────────────────────────────────────────────────────────────
// Multimodal attachments
// ─────────────────────────────────────────────────────────────────────────────

/// Прикреплённый к сообщению файл. Сейчас поддерживаются только картинки —
/// рендерятся в bubble и уходят в multipart `content` к llama-server как
/// `image_url` с data: URL.
///
/// Сами байты живут в CAS на диске
/// ([`super::blobs::full_path`]); JSON чата хранит только метаданные.
/// Поля `width`/`height` декодируются один раз при `attach`-операции и
/// нужны для компактного thumbnail-layout без дополнительного декода.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MsgAttachment {
    /// Hex sha256 содержимого файла (64 символа). Имя blob'а на диске.
    pub sha256: String,
    /// MIME-тип, например `"image/png"`. Определяет расширение файла и
    /// префикс data: URL при отправке в API.
    pub mime: String,
    /// Имя файла, выбранного пользователем — для tooltip'а в превью.
    /// Не обязательное, может быть пустым (например, drag&drop без имени).
    #[serde(default)]
    pub original_name: String,
    /// Натуральные размеры в пикселях. `0/0` — не удалось декодировать.
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Размер файла в байтах — для статистики/UX.
    #[serde(default)]
    pub size_bytes: u64,
}

impl MsgAttachment {
    /// Относительный путь от `blobs_dir()`: `"<sha256>.<ext>"`.
    pub fn rel_path(&self) -> String {
        format!("{}.{}", self.sha256, super::blobs::ext_from_mime(&self.mime))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Типы сообщений UI-слоя
// ─────────────────────────────────────────────────────────────────────────────

/// Роль сообщения в UI. Отличается от [`crate::llama::api::ChatRole`] тем, что
/// здесь нет `Tool`/`Developer` — мы их не показываем, они собираются в
/// `session::build_history` напрямую из `Vec<ChatMsg>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChatMsgRole {
    User,
    Assistant,
    System,
}

/// Подтип сообщения. Разделяет обычные текстовые реплики, tool-call’ы от
/// ассистента и tool-результаты в одной ленте.
///
/// Сделан отдельным полем `kind`, а не новой ролью в [`ChatMsgRole`],
/// чтобы не ломать `match` по роли в существующих виджетах и чтобы
/// `#[serde(default)]` обеспечил полную backward-compat со старыми
/// `~/.config/synthos/chats/*.json` (там поля нет → `Text`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum ChatMsgKind {
    /// Обычное текстовое сообщение (user/assistant/system).
    #[default]
    Text,
    /// Вызов инструмента от ассистента. `body` сообщения уже содержит
    /// pretty-JSON аргументов для UI, но для сервера истина — в `tool_calls`.
    ToolCall {
        /// Имя инструмента (совпадает с `tool_calls[0].function.name`).
        tool_name: String,
    },
    /// Результат выполнения инструмента. `body` сообщения = текстовый
    /// вывод (pretty-JSON или сырой текст).
    ToolResult {
        /// id вызова, на который отвечаем (обязателен для `role=tool`).
        tool_call_id: String,
        /// Имя инструмента — только для отображения в UI.
        tool_name: String,
        /// `true` — результат неуспешен (красим иначе).
        #[serde(default)]
        error: bool,
    },
    /// Маркер autocompact-итерации. Помещается в ленту вместо удалённых
    /// старых сообщений: показывает пользователю, что часть истории сжата,
    /// и при отправке в API превращается в `system`-сообщение с текстом
    /// `summary`. Сами свёрнутые сообщения остаются в `messages` помеченными
    /// `compacted_iter = Some(iteration)` и в API не уходят.
    CompactionMarker {
        /// Монотонный номер итерации сжатия (1, 2, 3…).
        iteration: u32,
        /// Сколько `ChatMsg` свернуто этой итерацией (для UI-плашки).
        compacted_count: usize,
        /// `prompt_tokens` в момент срабатывания триггера (или 0 для Manual
        /// с недоступным usage).
        tokens_before: i64,
        /// Точная оценка токенов в `summary` (через `/tokenize`).
        tokens_after: i64,
        /// Сжатый текст диалога. Уходит в API как `system`-сообщение.
        summary: String,
    },
}

/// Одно сообщение в истории чата.
///
/// Для assistant-сообщений `body` растёт инкрементально по мере прихода
/// токенов из SSE; в остальных случаях — записывается один раз.
///
/// Поле `tone_class` — это MSS-класс аватара (`avatar-slate`, `avatar-blue`,
/// …). Сделано `String`, а не `&'static str`, чтобы сообщение можно было
/// сериализовать в JSON при сохранении чата на диск.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChatMsg {
    pub role: ChatMsgRole,
    /// Отображаемое имя ("Вы" / "Ассистент" / пусто для system).
    pub author: String,
    /// Инициалы для аватара. Пусто у system-сообщений.
    pub initials: String,
    /// CSS-класс аватара (переиспользует палитру `avatar-*` из `mock.rs`).
    pub tone_class: String,
    /// Строка времени «HH:MM». Пусто у system-сообщений.
    pub time: String,
    /// Содержимое сообщения. Для assistant — сырой Markdown (рендерится
    /// через `MarkdownView`); для user — plain-текст.
    pub body: String,
    /// Размышления (chain-of-thought) для reasoning-моделей.
    /// Растёт инкрементально из `delta.reasoning_content` либо из тегов
    /// `<think>...</think>` в `delta.content` (см. `chat::think_parser`).
    /// Пустая строка — у обычных, non-reasoning ответов и у user/system/
    /// tool-сообщений. UI рендерит в отдельном collapsible-блоке внутри
    /// assistant-bubble. `#[serde(default)]` — старые JSON-чаты без поля
    /// читаются как пустая строка.
    #[serde(default)]
    pub thinking: String,
    /// Флаг «это сообщение об ошибке» — красит system-линию в тревожный цвет.
    pub error: bool,
    /// Подтип сообщения: Text (по умолчанию), ToolCall, ToolResult.
    /// `#[serde(default)]` — старые JSON-чаты без поля читаются как `Text`.
    #[serde(default)]
    pub kind: ChatMsgKind,
    /// Накопленные tool_calls для assistant-сообщения с `kind=ToolCall`.
    /// `None` — обычный assistant-ответ без вызова инструментов.
    /// `#[serde(default)]` — backward-compat.
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    /// Прикреплённые к сообщению файлы (только пользовательские реплики
    /// сейчас имеют непустой список). При отправке в API они становятся
    /// `image_url`-частями multipart-сообщения; в UI рендерятся в bubble
    /// перед телом текста. Старые JSON чата без этого поля грузятся как
    /// пустой `Vec` благодаря `#[serde(default)]`.
    #[serde(default)]
    pub attachments: Vec<MsgAttachment>,
    /// `Some(iteration)` — сообщение свёрнуто маркером данной итерации
    /// autocompact и не отправляется в API. В UI рендерится только когда
    /// маркер развёрнут пользователем. `None` (по умолчанию) — обычное
    /// сообщение. Старые JSON-чаты без поля грузятся как `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_iter: Option<u32>,
}

impl ChatMsg {
    /// Сообщение от пользователя без прикреплённых файлов.
    pub fn user(body: impl Into<String>) -> Self {
        Self::user_with_attachments(body, Vec::new())
    }

    /// Сообщение от пользователя с прикреплёнными файлами. Если
    /// `attachments` пуст — эквивалентно [`ChatMsg::user`].
    pub fn user_with_attachments(body: impl Into<String>, attachments: Vec<MsgAttachment>) -> Self {
        Self {
            role: ChatMsgRole::User,
            author: "Вы".to_string(),
            initials: "ВЫ".to_string(),
            tone_class: "avatar-slate".to_string(),
            time: format_hm_now(),
            body: body.into(),
            thinking: String::new(),
            error: false,
            kind: ChatMsgKind::Text,
            tool_calls: None,
            attachments,
            compacted_iter: None,
        }
    }

    /// Пустой placeholder для ответа ассистента — в него стримятся токены.
    pub fn assistant_empty() -> Self {
        Self {
            role: ChatMsgRole::Assistant,
            author: "Ассистент".to_string(),
            initials: "AI".to_string(),
            tone_class: "avatar-blue".to_string(),
            time: format_hm_now(),
            body: String::new(),
            thinking: String::new(),
            error: false,
            kind: ChatMsgKind::Text,
            tool_calls: None,
            attachments: Vec::new(),
            compacted_iter: None,
        }
    }

    /// Системная «плашка» внутри ленты (ошибка соединения, отмена и т.п.).
    pub fn system(body: impl Into<String>, error: bool) -> Self {
        Self {
            role: ChatMsgRole::System,
            author: String::new(),
            initials: String::new(),
            tone_class: "avatar-slate".to_string(),
            time: String::new(),
            body: body.into(),
            thinking: String::new(),
            error,
            kind: ChatMsgKind::Text,
            tool_calls: None,
            attachments: Vec::new(),
            compacted_iter: None,
        }
    }

    /// Сообщение-вызов инструмента от ассистента.
    ///
    /// `args_pretty` — человекочитаемый JSON для показа в UI (без секретов,
    /// это то, что llama вернула в `function.arguments`). `tool_calls` —
    /// структурированные данные для отправки обратно серверу.
    pub fn tool_call(tool_name: impl Into<String>, args_pretty: impl Into<String>, calls: Vec<ChatToolCall>) -> Self {
        Self {
            role: ChatMsgRole::Assistant,
            author: "Ассистент".to_string(),
            initials: "AI".to_string(),
            tone_class: "avatar-blue".to_string(),
            time: format_hm_now(),
            body: args_pretty.into(),
            thinking: String::new(),
            error: false,
            kind: ChatMsgKind::ToolCall {
                tool_name: tool_name.into(),
            },
            tool_calls: Some(calls),
            attachments: Vec::new(),
            compacted_iter: None,
        }
    }

    /// Сообщение-результат работы инструмента. Рендерится отдельной
    /// «нейтральной» плашкой, не принадлежит ни user, ни assistant —
    /// `role=System` тут только для визуальной группировки.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
        error: bool,
    ) -> Self {
        Self {
            role: ChatMsgRole::System,
            author: String::new(),
            initials: String::new(),
            tone_class: "avatar-slate".to_string(),
            time: format_hm_now(),
            body: content.into(),
            thinking: String::new(),
            error,
            kind: ChatMsgKind::ToolResult {
                tool_call_id: tool_call_id.into(),
                tool_name: tool_name.into(),
                error,
            },
            tool_calls: None,
            attachments: Vec::new(),
            compacted_iter: None,
        }
    }

    /// Маркер autocompact-итерации в ленте. Кладётся вместо удалённого
    /// диапазона старых сообщений (которые остаются в `messages`,
    /// помеченные `compacted_iter = Some(iteration)`). В UI рисуется
    /// раскрываемой плашкой, в API уходит как `system` со списком фактов
    /// из `summary` (см. `build_history`).
    pub fn compaction_marker(
        iteration: u32,
        compacted_count: usize,
        tokens_before: i64,
        tokens_after: i64,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            role: ChatMsgRole::System,
            author: String::new(),
            initials: String::new(),
            tone_class: "avatar-slate".to_string(),
            time: format_hm_now(),
            body: String::new(),
            thinking: String::new(),
            error: false,
            kind: ChatMsgKind::CompactionMarker {
                iteration,
                compacted_count,
                tokens_before,
                tokens_after,
                summary: summary.into(),
            },
            tool_calls: None,
            attachments: Vec::new(),
            compacted_iter: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Мета-запись для левой колонки
// ─────────────────────────────────────────────────────────────────────────────

/// Лёгкая «шапка» чата для реактивного списка слева. Хранит ровно то, что
/// нужно отрисовать в карточке + сортировать/находить — без всей ленты.
///
/// Полный [`super::storage::StoredChat`] читается только при выборе чата.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMeta {
    pub id: String,
    pub title: String,
    /// Превью последнего сообщения (≤ 80 символов, char-boundary-safe).
    pub preview: String,
    /// Unix-секунды последнего обновления — для сортировки «новые сверху».
    pub updated_at: u64,
    /// Имя модели, с которой велась последняя переписка (или None).
    pub model_name: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Контекст чата
// ─────────────────────────────────────────────────────────────────────────────

/// Реактивное состояние мультиконтекстного чата. Живёт внутри
/// [`crate::context::AppCtx`], поэтому переживает переключение маршрутов:
/// начатый стрим продолжит писать в `messages` и тогда, когда пользователь
/// ушёл на другую страницу.
#[derive(Clone)]
pub struct ChatCtx {
    /// Список метаданных всех чатов. Реактивно обновляется при create/delete.
    /// Хранится отсортированным по `updated_at` убывающе — свежие вверху.
    pub chats: RwSignal<Vec<ChatMeta>>,
    /// id активного чата или `None`, если ни одного чата ещё не создано /
    /// все удалены.
    pub active_chat_id: RwSignal<Option<String>>,
    /// Имя модели, привязанной к активному чату. Инициализируется из
    /// `StoredChat.model_name` при select, пишется в `storage` при
    /// изменении `AppCtx.selected_model`.
    pub active_model: RwSignal<Option<String>>,
    /// Флаг «идёт массовое обновление сигналов (подгрузка с диска)».
    /// Подавляет автосейв в `install_chat_autosave` на время load, иначе
    /// свежепрочитанный файл записался бы обратно почти без изменений.
    pub loading: RwSignal<bool>,
    /// Fingerprint последнего *сохранённого* состояния активного чата
    /// (messages + model_name + title). Автосейв сверяет с ним свежий
    /// снимок и пропускает запись, если ничего не изменилось — тогда
    /// простое переключение чата не будет зря бампать `updated_at` и
    /// пересортировывать список. Обновляется:
    /// - в `registry::select_internal` сразу после чтения с диска;
    /// - в `install_chat_autosave` после успешной записи.
    pub last_saved_fp: RwSignal<u64>,

    /// Лента сообщений активного чата.
    pub messages: RwSignal<Vec<ChatMsg>>,
    /// Inkrементальный «хвост» body последнего assistant-плейсхолдера во
    /// время активного стрима. drive_stream пушит сюда токены на каждом
    /// chunk'е; на финале turn'а `commit_streaming_tail` вливает значение
    /// в `messages[last].body` и сбрасывает сигнал. UI рисует body
    /// последнего бабла как `msg.body + streaming_body.get()`, и только
    /// этот один bubble подписан на сигнал — остальная лента не
    /// пересобирается на каждом токене.
    pub streaming_body: RwSignal<String>,
    /// То же, что и [`Self::streaming_body`], но для reasoning/thinking
    /// блока. Бабл показывает `msg.thinking + streaming_thinking.get()`.
    pub streaming_thinking: RwSignal<String>,
    /// Черновик в поле ввода — двусторонняя связка с `MultilineTextEdit`.
    pub input: RwSignal<String>,
    /// Поколение поля ввода: монотонный счётчик, который бампается каждый
    /// раз, когда содержимое сброшено извне (после отправки сообщения).
    /// `MultilineTextEdit` сам не двунаправленно связан с `input` — он
    /// забирает initial text при создании; поэтому пересоздаём editor
    /// при смене поколения через `Reactive`-обёртку.
    pub input_gen: RwSignal<u64>,
    /// Оценка количества токенов в текущем `input`. Обновляется из двух
    /// источников: мгновенно (локальная эвристика при on_change) и точнее
    /// — debounced-запросом в `/tokenize` llama-server через 300 мс после
    /// последнего нажатия. Когда сервер недоступен, остаётся эвристика.
    pub input_tokens: RwSignal<usize>,
    /// Монотонный счётчик «запросов на токенизацию». Каждый on_change
    /// бампает его; debounced async-задача сверяется со снимком — если
    /// значение изменилось, задача молча завершается (устаревший ввод).
    /// Нужен Arc<AtomicU64>, потому что async-воркер tokio читает/пишет
    /// его вне UI-потока, а `RwSignal` — thread-local.
    pub input_tok_gen: Arc<AtomicU64>,
    /// `true`, пока идёт генерация ответа (SSE-стрим активен).
    pub pending: RwSignal<bool>,
    /// Последняя человекочитаемая ошибка — для возможного snackbar’а.
    pub error: RwSignal<Option<String>>,
    /// Монотонный счётчик «запросов на прерывание».
    pub abort: Arc<AtomicU64>,
    /// Системный промпт — общий с `AppCtx::general::system_prompt`.
    pub system_prompt: RwSignal<String>,
    /// Состояние раскрытия thinking-блоков в ленте: ключ — индекс
    /// сообщения в текущем чате (валиден до перехода на другой чат
    /// или массового rebuild ленты), значение — `true` если блок
    /// развёрнут, `false`/отсутствует — свёрнут. Эфемерное UI-состояние,
    /// не persist'ится. Сбрасывается на каждом `select`/новом чате.
    pub thinking_open: RwSignal<std::collections::HashMap<usize, bool>>,
    /// Состояние раскрытия групп подряд идущих tool-вызовов в minimal-
    /// режиме отображения. Ключ — индекс первого `ChatMsg` группы в
    /// текущей ленте, значение — `true` если группа развёрнута. Дефолт
    /// (отсутствие записи) — закрыто. Эфемерное UI-состояние, не
    /// persist'ится; сбрасывается при переключении чата (та же семантика,
    /// что у `thinking_open`).
    pub tool_group_open: RwSignal<std::collections::HashMap<usize, bool>>,
    /// Состояние раскрытия маркеров autocompact-итераций. Ключ — `iteration`
    /// (стабильный u32, не индекс — иначе сдвиги при добавлении сообщений
    /// сломают связь). Значение — `true` если маркер развёрнут (показывает
    /// summary + свёрнутые сообщения). Эфемерное UI-состояние, не
    /// persist'ится; сбрасывается при переключении чата.
    pub compaction_open: RwSignal<std::collections::HashMap<u32, bool>>,
    /// Прикреплённые пользователем файлы для следующего сообщения.
    /// Реактивно отображаются в полосе превью над input-bar и уходят в
    /// `ChatMsg::attachments` при `send_message`. Сбрасывается:
    /// (1) после успешного `send_message`,
    /// (2) при переключении чата в `registry::select`.
    pub draft_attachments: RwSignal<Vec<MsgAttachment>>,
    /// RAG-блок для текущего хода, сформированный `kb::augment::compute`
    /// в `chat::session::start_agent_turn` до спавна `run_agent`. Читается
    /// untracked'ом из `build_history` и подмешивается в system prompt
    /// между ACTION_RULE и user_system_prompt. Per-chat: сбрасывается в
    /// `registry::select_internal` и в начале каждого нового turn'а.
    pub augment_text: RwSignal<String>,
}

impl ChatCtx {
    /// Создать пустой контекст, подключив уже созданный сигнал системного
    /// промпта из `GeneralCtx`. Это гарантирует, что правка поля в Settings
    /// сразу влияет на следующие запросы без дополнительной синхронизации.
    pub fn new(system_prompt: RwSignal<String>) -> Self {
        Self {
            chats: use_signal(Vec::new()),
            active_chat_id: use_signal(None),
            active_model: use_signal(None),
            loading: use_signal(false),
            last_saved_fp: use_signal(0),
            messages: use_signal(Vec::new()),
            streaming_body: use_signal(String::new()),
            streaming_thinking: use_signal(String::new()),
            input: use_signal(String::new()),
            input_tokens: use_signal(0),
            input_tok_gen: Arc::new(AtomicU64::new(0)),
            input_gen: use_signal(0),
            pending: use_signal(false),
            error: use_signal(None),
            abort: Arc::new(AtomicU64::new(0)),
            system_prompt,
            thinking_open: use_signal(std::collections::HashMap::new()),
            tool_group_open: use_signal(std::collections::HashMap::new()),
            compaction_open: use_signal(std::collections::HashMap::new()),
            draft_attachments: use_signal(Vec::new()),
            augment_text: use_signal(String::new()),
        }
    }
}
