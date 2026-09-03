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

use syngui::tr;
use serde::{Deserialize, Serialize};

use crate::agent::schema::ChatToolCall;

use super::time::format_hm_now;

// ─────────────────────────────────────────────────────────────────────────────
// Multimodal attachments
// ─────────────────────────────────────────────────────────────────────────────

/// Модальность вложения. Определяет и рендер в UI, и то, как вложение
/// попадает в промпт: картинки/видео уходят в vision-башню модели,
/// документы разворачиваются в текст, аудио — в транскрипт (если загружена
/// ASR-модель).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    #[default]
    Image,
    Video,
    Audio,
    /// Текстовый документ (md/txt/html/код/json) — инлайнится в промпт.
    Document,
    /// Всё остальное: в промпт уходит только имя и размер файла.
    Other,
}

impl AttachmentKind {
    /// Человекочитаемое имя для UI-подписей.
    pub fn label(self) -> String {
        match self {
            Self::Image => tr!("chat.attachment.kind.image"),
            Self::Video => tr!("chat.attachment.kind.video"),
            Self::Audio => tr!("chat.attachment.kind.audio"),
            Self::Document => tr!("chat.attachment.kind.document"),
            Self::Other => tr!("chat.attachment.kind.other"),
        }
    }

    /// Показывать ли для вложения картинку-превью (иначе — иконка).
    pub fn has_thumbnail(self) -> bool {
        matches!(self, Self::Image | Self::Video)
    }
}

/// Прикреплённый к сообщению файл.
///
/// Сами байты живут в CAS на диске (`~/.config/synthos/blobs/`, см.
/// [`crate::syn_chat::attach::blobs`]); JSON чата хранит только метаданные,
/// поэтому один и тот же файл, прикреплённый в десяти чатах, лежит на диске
/// один раз.
///
/// `width`/`height`/`duration_ms` вычисляются один раз при прикреплении и
/// нужны для layout превью без повторного декода.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MsgAttachment {
    /// Hex sha256 содержимого файла (64 символа). Имя blob'а на диске.
    pub sha256: String,
    /// MIME-тип, например `"image/png"`.
    pub mime: String,
    /// Имя файла, выбранного пользователем — заголовок карточки и tooltip.
    #[serde(default)]
    pub original_name: String,
    /// Натуральные размеры в пикселях. `0/0` — не картинка/не удалось прочесть.
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Размер файла в байтах — для статистики/UX.
    #[serde(default)]
    pub size_bytes: u64,
    /// Модальность. Старые JSON без поля читаются как `Image` — ровно то,
    /// что там и лежало (прежняя версия умела только картинки).
    #[serde(default)]
    pub kind: AttachmentKind,
    /// Расширение blob'а на диске (без точки, lowercase). Пустое — blob
    /// лежит без расширения.
    #[serde(default)]
    pub ext: String,
    /// Длительность в миллисекундах для видео/аудио. `0` — неизвестно.
    #[serde(default)]
    pub duration_ms: u64,
    /// Расширение конвертированной «под модель» копии в `blobs/derived/`.
    /// Пустое — модель читает оригинальный blob (формат уже подходит).
    #[serde(default)]
    pub model_ext: String,
    /// Расширение полноразмерной копии для показа в UI. Нужно там, где
    /// декодер syngui не знает исходный формат (WebP, HEIC, TIFF…): в
    /// `blobs/derived/` лежит PNG, а оригинал остаётся нетронутым.
    /// Пустое — рисуем сам blob.
    #[serde(default)]
    pub ui_ext: String,
    /// Сгенерирован ли thumbnail в `blobs/thumbs/<sha>.png`.
    #[serde(default)]
    pub has_thumb: bool,
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
    /// Дописка к телу, которую видит только модель: заметки agent-loop'а к
    /// результату инструмента («вызов сделан N-й раз», «осталось K ходов»).
    /// В UI не показывается, в промпт уходит вслед за телом — так история,
    /// собранная из ленты на следующем сообщении, совпадает с той, что была у
    /// хода, и префикс-KV диалога не обнуляется. Старые чаты без поля
    /// читаются как пустая строка.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_note: String,
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
            author: tr!("chat.msg.author.you"),
            initials: tr!("chat.msg.author.you_initials"),
            tone_class: "avatar-slate".to_string(),
            time: format_hm_now(),
            body: body.into(),
            thinking: String::new(),
            error: false,
            kind: ChatMsgKind::Text,
            tool_calls: None,
            attachments,
            compacted_iter: None,
            model_note: String::new(),
        }
    }

    /// Пустой placeholder для ответа ассистента — в него стримятся токены.
    pub fn assistant_empty() -> Self {
        Self {
            role: ChatMsgRole::Assistant,
            author: tr!("chat.msg.author.assistant"),
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
            model_note: String::new(),
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
            model_note: String::new(),
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
            author: tr!("chat.msg.author.assistant"),
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
            model_note: String::new(),
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
            model_note: String::new(),
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
            model_note: String::new(),
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
    /// Unix-секунды создания — порядок плиток в нав-рейле.
    pub created_at: u64,
    /// Unix-секунды последнего обновления — для сортировки «новые сверху».
    pub updated_at: u64,
    /// Имя модели, с которой велась последняя переписка (или None).
    pub model_name: Option<String>,
    /// Чат в архиве: в рейле не показывается, доступен из Настройки → Архив.
    pub archived: bool,
}

// (ChatCtx старого llama-чата удалён — нативный Syn-чат использует
//  `crate::syn_chat::state::SynChatCtx`.)

