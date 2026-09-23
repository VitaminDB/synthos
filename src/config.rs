//! Persistent-конфиг приложения (`~/.config/synthos/config.json`).
//!
//! Один общий файл — тема, раздел «Общие» и пресеты моделей llama.cpp.
//! Любая ошибка чтения/парсинга логируется и падает на `Default` — приложение
//! никогда не паникует из-за поломанного конфига (правило TASK.md). Файл,
//! который не разобрался, откладывается в `config.json.bad-<время>`, а не
//! затирается дефолтами; запись — атомарная (`crate::fsutil`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::syn_chat::params::SamplingParams;

/// Сериализует циклы «прочитать → поправить → записать» конфига: без неё
/// две такие правки из разных мест (автосейв главного потока, поток загрузки
/// модели, поиск) молча откатывают друг друга. См. [`AppConfig::update`].
static CONFIG_RW_LOCK: Mutex<()> = Mutex::new(());

// ─────────────────────────────────────────────────────────────────────────────
// Общие настройки (раздел «Общие»)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub display_name: String,
    pub language: String,
    /// Системный промпт, который добавляется первым сообщением (`role=system`)
    /// в каждый запрос к модели. Редактируется в Settings → Общие.
    pub system_prompt: String,
    /// Системный промпт постобработки распознанной речи. Применяется в
    /// FAB-окне распознавания при ручном клике по кнопке «обновить» над
    /// полем «Отредактированный текст». Прогоняется через ту же загруженную
    /// нативную модель Syn-чата. Пустая строка → постобработка no-op
    /// (refined = raw, без вызова модели).
    #[serde(default = "default_voice_refine_prompt")]
    pub voice_refine_prompt: String,
    /// Режим отображения tool-вызовов в ленте чата: `"full"` (по умолчанию),
    /// `"minimal"` (только заголовок карточки), `"hidden"` (карточка не
    /// рендерится). Управляется из popup-меню «⋯» в шапке чата и из
    /// Settings → Общие → Чат.
    #[serde(default = "default_tool_display_mode")]
    pub tool_display_mode: String,
    /// Глобальный режим подтверждения tool-call'ов. Допустимые значения:
    /// `"ask"` (по умолчанию) — каждый раз показывать диалог;
    /// `"always_allow"` — выполнять без подтверждения.
    /// Невалидные значения трактуются как `"ask"` через
    /// [`effective_approval_mode`].
    #[serde(default = "default_tool_approval_default")]
    pub tool_approval_default: String,
    /// Per-tool override глобального режима. Ключ — `Tool::key`
    /// (`"bash"`, `"web_read"`, …), значение — `"ask"` | `"always_allow"` |
    /// `"default"`. Отсутствие ключа эквивалентно `"default"`
    /// (использовать `tool_approval_default`).
    #[serde(default)]
    pub tool_approval_overrides: HashMap<String, String>,
    /// Имя входного аудио-устройства (`Device::name()` в cpal). Пусто или
    /// `"auto"` = автовыбор: default + перебор. Если выбранное устройство
    /// при следующем старте недоступно — система fallback'ает на работающее
    /// и автоматически перезаписывает это поле.
    #[serde(default)]
    pub audio_input_device: String,
    /// Семейство шрифта для окна голосового распознавания (FAB → центр).
    /// Пусто = `"sans-serif"` (system default). Применяется через
    /// MSS-переменную `--voice-font-family` в классе `.voice-overlay-text`.
    #[serde(default)]
    pub voice_font_family: String,
    /// Размер шрифта окна голосового распознавания в логических пикселях.
    /// Применяется через MSS-переменную `--voice-font-size`.
    #[serde(default = "default_voice_font_size")]
    pub voice_font_size: f32,
    /// Максимум tool-turn'ов в одном цикле основного агента
    /// (`syn_chat::session::run_agent_loop`). По его исчерпании генерация
    /// останавливается с подсказкой «Продолжить». Поднимать, если модель
    /// часто упирается.
    #[serde(default = "default_agent_max_turns")]
    pub agent_max_turns: u32,
    /// Максимум tool-turn'ов внутри одного субагента
    /// (`agent::tools::subagent`). По исчерпании делается
    /// финальный summarize-turn без tools — модель сжимает прогресс.
    #[serde(default = "default_subagent_max_turns")]
    pub subagent_max_turns: u32,
    /// Включён ли autocompact: автоматическое сжатие старых сообщений
    /// в `system`-summary, когда промпт последнего хода превышает
    /// `autocompact_threshold_percent` от лимита контекста.
    /// См. `syn_chat::compact::maybe_autocompact`. При выключенном флаге
    /// доступна только ручная кнопка сжатия в шапке чата.
    #[serde(default = "default_autocompact_enabled")]
    pub autocompact_enabled: bool,
    /// Порог срабатывания autocompact в процентах от размера контекстного
    /// окна модели. По умолчанию 85%. Зажимается в диапазон 50..=95 в UI.
    #[serde(default = "default_autocompact_threshold")]
    pub autocompact_threshold_percent: u32,
}

/// Дефолт для `GeneralConfig.agent_max_turns`.
pub fn default_agent_max_turns() -> u32 {
    32
}

/// Дефолт для `GeneralConfig.subagent_max_turns`.
pub fn default_subagent_max_turns() -> u32 {
    24
}

/// Дефолт для `GeneralConfig.autocompact_enabled` — включено по умолчанию.
pub fn default_autocompact_enabled() -> bool {
    true
}

/// Дефолт для `GeneralConfig.autocompact_threshold_percent` — 85% заполнения
/// контекстного окна. При больших значениях триггер срабатывает поздно и
/// модель чаще упирается в обрезку; при меньших — слишком часто и теряем
/// детали. 85 — баланс на основе UX в Claude Code.
pub fn default_autocompact_threshold() -> u32 {
    85
}

/// Дефолт для `GeneralConfig.voice_font_size` — 28px (крупный, чтобы
/// распознанный текст хорошо читался на дистанции в overlay-окне).
pub fn default_voice_font_size() -> f32 {
    28.0
}

/// Дефолт для `GeneralConfig.tool_display_mode`. Используется как
/// `#[serde(default = ...)]` для прозрачной обратной совместимости со
/// старыми конфигами без поля, и в `Default for GeneralConfig`.
pub fn default_tool_display_mode() -> String {
    "full".to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Подсистема разрешений на выполнение tool-call'ов
// ─────────────────────────────────────────────────────────────────────────────

/// Спрашивать каждый раз через диалог.
pub const TOOL_APPROVAL_ASK: &str = "ask";
/// Разрешать без подтверждения.
pub const TOOL_APPROVAL_ALWAYS: &str = "always_allow";
/// Псевдо-значение для override: «использовать глобальный режим». На диск
/// никогда не пишется (`tool_override_row` удаляет ключ из map при выборе
/// этого пункта); хранится только как виртуальный пункт Dropdown'а.
pub const TOOL_APPROVAL_DEFAULT: &str = "default";

/// Дефолт для `GeneralConfig.tool_approval_default`.
pub fn default_tool_approval_default() -> String {
    TOOL_APPROVAL_ASK.to_string()
}

/// Возвращает effective-режим подтверждения для конкретного `tool_key`
/// с учётом per-tool override → global default → fallback `"ask"`.
///
/// Любые невалидные строки трактуются как `"ask"` — таким образом
/// подсистема никогда не вернёт «незнакомое» значение и сборщик `match`'а
/// всегда корректен.
pub fn effective_approval_mode(
    tool_key: &str,
    default_mode: &str,
    overrides: &HashMap<String, String>,
) -> &'static str {
    match overrides.get(tool_key).map(String::as_str) {
        Some(TOOL_APPROVAL_ASK) => TOOL_APPROVAL_ASK,
        Some(TOOL_APPROVAL_ALWAYS) => TOOL_APPROVAL_ALWAYS,
        // `Some("default")` или невалидный override → fallback на global.
        // Отсутствие ключа в map (`None`) — точно так же.
        _ => match default_mode {
            TOOL_APPROVAL_ALWAYS => TOOL_APPROVAL_ALWAYS,
            _ => TOOL_APPROVAL_ASK,
        },
    }
}

/// Текст системного промпта по умолчанию. Используется и в `Default`
/// для `GeneralConfig`, и как запасной вариант, если пользователь очистит поле.
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a friendly AI assistant. \
Answer briefly, to the point, and in the user's language.\n\
\n\
ACTION RULE. If answering requires calling a tool — \
call it IMMEDIATELY in this same response. Never write \"let me check\", \
\"let me read that\", \"now I'll find\", \"checking\" and never end a reply \
with a colon before the action — this is forbidden. Announcing without \
calling the tool is an error. If you're about to take several steps with \
tools, take the first step right away, with no preamble. Explanations and \
the summary come after the tool results, not before.\n\
\n\
If answering requires fresh or specific information from the internet, \
use the `web` tool:\n\
1. First call `web` with `action=\"search\"` and a descriptive query — \
get a list of relevant pages (title + url + snippet).\n\
2. Then call `web` with `action=\"read\"` for the 1-2 best-fitting \
URLs — you'll get a Markdown extract of the article content \
(via Readability + htmd, without nav/footer/ads).\n\
Don't try to read pages at an unknown URL — find them first via action=search.";

/// Текст системного промпта постобработки голосового ввода по умолчанию.
/// Используется в `Default for GeneralConfig` и как `#[serde(default = ...)]`
/// для старых конфигов без этого поля.
pub const DEFAULT_VOICE_REFINE_PROMPT: &str = "You are a transcript editor. \
Take the recognized speech and return the same text with punctuation, \
capitalization, and obvious recognition errors corrected. Don't add \
anything of your own, don't comment, don't answer questions. \
Return only the edited text.";

/// Дефолт для `GeneralConfig.voice_refine_prompt`.
pub fn default_voice_refine_prompt() -> String {
    DEFAULT_VOICE_REFINE_PROMPT.to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            display_name: "Dean Kowalski".into(),
            language: "auto".into(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            voice_refine_prompt: default_voice_refine_prompt(),
            tool_display_mode: default_tool_display_mode(),
            tool_approval_default: default_tool_approval_default(),
            tool_approval_overrides: HashMap::new(),
            audio_input_device: String::new(),
            voice_font_family: String::new(),
            voice_font_size: default_voice_font_size(),
            agent_max_turns: default_agent_max_turns(),
            subagent_max_turns: default_subagent_max_turns(),
            autocompact_enabled: default_autocompact_enabled(),
            autocompact_threshold_percent: default_autocompact_threshold(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy: активные параметры (сохранились в `AudioModelConfig` для обратной
// совместимости старых конфигов; сам llama.cpp-слой удалён).
// ─────────────────────────────────────────────────────────────────────────────

/// Активная (выбранная пользователем) настройка модели (legacy).
/// `key` — короткий id параметра.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveParam {
    pub key: String,
    pub value: ParamValue,
}

// ─────────────────────────────────────────────────────────────────────────────
// ASR-модели (локальное распознавание речи через synaptix::facade::asr)
// ─────────────────────────────────────────────────────────────────────────────

/// Тип ASR-движка. Сериализуется в JSON как обычная строка через
/// `#[serde(rename_all = "snake_case")]`. Привязан 1-к-1 к
/// `synaptix::facade::asr::AsrModelKind` (он же `asr_core::ModelType`),
/// но дублируется здесь, чтобы конфиг не зависел от feature-флага `asr`
/// (без него у synthos не должно быть compile-error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrEngineKind {
    Whisper,
    GigaAm,
}

impl AsrEngineKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Whisper => "Whisper Large v3 Turbo",
            Self::GigaAm => "GigaAM v3 (RU)",
        }
    }

    pub fn all() -> &'static [AsrEngineKind] {
        &[AsrEngineKind::Whisper, AsrEngineKind::GigaAm]
    }
}

impl Default for AsrEngineKind {
    fn default() -> Self {
        AsrEngineKind::Whisper
    }
}

/// Пресет ASR-модели — путь к `.syn` bundle'у + язык + устройство + storage/compute dtype.
///
/// Storage и compute dtype разделены: storage — формат весов в VRAM
/// (потенциально quantized — NVFP4/MXFP8), compute — формат активаций и
/// matmul accumulator'а (всегда float `synaptix_core` DType: f32/bf16/f16).
///
/// Legacy поля (`dtype`, `mmproj_path`, `quantized`, `server_port`,
/// `active_params`) парсятся для совместимости со старыми конфигами, но
/// при `save()` не записываются.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioModelConfig {
    pub name: String,
    /// Тип движка. По умолчанию Whisper.
    #[serde(default)]
    pub kind: AsrEngineKind,
    /// Путь к `.syn` bundle'у модели (см. крейт `syn-format`).
    pub model_path: String,
    /// Hint языка (ISO-639-1). `"auto"` → автодетект (Whisper).
    pub language: String,
    /// Устройство инференса: `"cpu"` | `"gpu_auto"`.
    /// `"gpu_auto"` — CUDA → Metal → fallback CPU.
    #[serde(default = "default_audio_device")]
    pub device: String,
    /// Формат хранения весов в VRAM: `"f32"` | `"bf16"` | `"f16"` | `"q8_0"`
    /// | `"q4_0"` | `"fp8e4m3"` | `"nvfp4"`.
    #[serde(default = "default_audio_storage_dtype")]
    pub storage_dtype: String,
    /// Формат вычислений (активации + matmul accumulator): `"f32"` | `"bf16"` | `"f16"`.
    #[serde(default = "default_audio_compute_dtype")]
    pub compute_dtype: String,

    // ── LEGACY: парсятся, не сериализуются. ──
    /// LEGACY (single-dtype эпоха): один dtype для storage и compute. При
    /// миграции пустые `storage_dtype`/`compute_dtype` заполняются из него.
    #[serde(default, skip_serializing)]
    pub dtype: String,
    #[serde(default, skip_serializing)]
    pub mmproj_path: String,
    #[serde(default, skip_serializing)]
    pub quantized: bool,
    #[serde(default, skip_serializing)]
    pub server_port: u16,
    #[serde(default, skip_serializing)]
    pub active_params: Vec<ActiveParam>,
}

/// Дефолт `AudioModelConfig.device`. CPU работает без feature-флагов
/// `cuda`/`metal` — безопасный fallback для новых пресетов.
pub fn default_audio_device() -> String {
    "cpu".into()
}

/// Дефолт `AudioModelConfig.storage_dtype`. F16 — компромисс по умолчанию;
/// quantized форматы пользователь выбирает явно.
pub fn default_audio_storage_dtype() -> String {
    "f16".into()
}

/// Дефолт `AudioModelConfig.compute_dtype`. F16 — стандарт для GPU inference;
/// на CPU synaptix деградирует до F32 в `Transcriber::load`.
pub fn default_audio_compute_dtype() -> String {
    "f16".into()
}

impl Default for AudioModelConfig {
    fn default() -> Self {
        Self {
            name: "Новая аудио-модель".into(),
            kind: AsrEngineKind::default(),
            model_path: String::new(),
            language: "auto".into(),
            device: default_audio_device(),
            storage_dtype: default_audio_storage_dtype(),
            compute_dtype: default_audio_compute_dtype(),
            dtype: String::new(),
            mmproj_path: String::new(),
            quantized: false,
            server_port: 0,
            active_params: Vec::new(),
        }
    }
}

impl AudioModelConfig {
    /// Миграция со старого однополевого `dtype`: если новые поля пустые,
    /// заполнить из legacy `dtype`. Вызывается из `AppConfig::load` после
    /// десериализации.
    pub fn migrate_legacy_dtype(&mut self) {
        if !self.dtype.is_empty() {
            if self.storage_dtype.is_empty()
                || self.storage_dtype == default_audio_storage_dtype()
            {
                self.storage_dtype = self.dtype.clone();
            }
            if self.compute_dtype.is_empty()
                || self.compute_dtype == default_audio_compute_dtype()
            {
                // legacy "int8" → compute='f16' (storage уже хранит "int8")
                self.compute_dtype = match self.dtype.as_str() {
                    "f32" | "bf16" | "f16" => self.dtype.clone(),
                    _ => "f16".into(),
                };
            }
            self.dtype.clear();
        }
    }
}

/// Значение активной настройки — полиморфное, сериализуется в JSON как
/// `{ "kind": "Int", "v": 42 }` (internally tagged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v")]
pub enum ParamValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    /// Значение из фиксированного набора (`split-mode = "layer"` и т.п.).
    Enum(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Сессии редактора кода (multi-project)
// ─────────────────────────────────────────────────────────────────────────────

/// Persist-описание одной сессии редактора кода. Хранится корень проекта,
/// список открытых файлов и активный файл — при перезапуске synthos
/// сессия восстанавливается «как была». Содержимое unsaved-буферов
/// хранится отдельно (см. `pages::code_editor::drafts` —
/// `~/.config/synthos/editor_drafts/<hash>.draft`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CodeSessionConfig {
    pub root_folder: Option<String>,
    /// Список открытых файлов в порядке вкладок. Восстанавливается
    /// функцией [`pages::code_editor::state::CodeSession::new`] —
    /// каждый файл читается с диска и добавляется в `open_files`.
    /// Файлы, которых уже нет на диске, пропускаются (с warn-логом).
    #[serde(default)]
    pub open_files: Vec<String>,
    /// Активный файл (выбран в редакторе). Должен быть в `open_files`,
    /// иначе игнорируется при восстановлении.
    #[serde(default)]
    pub active_file: Option<String>,
    /// Положение разделителя editor↔terminal (vertical) для этой сессии,
    /// доля верхней панели (0.05..0.95). `None` — миграционный fallback
    /// на [`default_code_editor_split_ratio`]. Записывается при drag
    /// дивайдера через `RwSignal` в [`pages::code_editor::state::CodeSession`].
    #[serde(default)]
    pub split_ratio: Option<f32>,
    /// Положение левого разделителя file_tree↔(center+open_files)
    /// (horizontal) для этой сессии. `None` → fallback
    /// [`default_code_editor_left_split_ratio`].
    #[serde(default)]
    pub left_split_ratio: Option<f32>,
    /// Положение правого разделителя (file_tree+center)↔open_files
    /// (horizontal) для этой сессии. `None` → fallback
    /// [`default_code_editor_right_split_ratio`].
    #[serde(default)]
    pub right_split_ratio: Option<f32>,
    /// Word-wrap toggle для CodeEditor этой сессии. По умолчанию выключен —
    /// пользователь включает кнопкой в header'е editor pane'а.
    #[serde(default)]
    pub soft_wrap: bool,
    /// Показан ли редактор над терминалом (кнопка в шапке центра). `None` —
    /// конфиг до тоггла, считается «показан». Без активного файла при
    /// восстановлении редактор всё равно скрыт.
    #[serde(default)]
    pub editor_visible: Option<bool>,
    /// Persisted-снимки cursor/scroll по каждому открытому файлу. Ключ —
    /// путь файла как строка (`PathBuf::display`). Заполняется автосэйвом
    /// (`install_config_autosave`) на каждом изменении `editor_states`
    /// в `CodeSession`; восстанавливается при `CodeSession::new`.
    /// Файлы, не присутствующие в `open_files`, остаются в карте только до
    /// следующего save'а (нет смысла хранить позицию для закрытого файла).
    #[serde(default)]
    pub editor_states: HashMap<String, EditorStateConfig>,
    /// Unix-миллисекунды создания сессии — порядок плитки в нав-рейле среди
    /// графов и чатов. `None` у конфигов до плиточного рейла: тогда время
    /// назначается по позиции в списке при загрузке (см.
    /// `pages::code_editor::state::CodeEditorCtx::new`).
    #[serde(default)]
    pub created_at: Option<u64>,
}

/// Сериализуемая копия `syngui::widgets::input::code_editor::EditorPersistedState`
/// — `serde` нужен JSON-роундтрип. Поля совпадают по семантике, конвертация
/// 1:1 в `code_editor::state`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EditorStateConfig {
    /// Byte offset primary-курсора в буфере.
    pub cursor_offset: usize,
    /// Visual-row scroll offset (см. `scroll_offset_lines` в CodeEditorElement).
    pub scroll_lines: usize,
    /// Horizontal scroll в логических пикселях.
    pub scroll_x: f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Корневой конфиг
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Ключ темы (имя из `theme_data::builtin_themes`).
    pub theme: String,
    /// Следовать светлой/тёмной схеме рабочего стола. В этом режиме активная
    /// тема выбирается из пары `theme_light`/`theme_dark`, а `theme` хранит
    /// последний ручной выбор.
    #[serde(default)]
    pub follow_system_theme: bool,
    /// Тема для системной светлой схемы. Пусто — дефолтная светлая.
    #[serde(default)]
    pub theme_light: String,
    /// Тема для системной тёмной схемы. Пусто — дефолтная тёмная.
    #[serde(default)]
    pub theme_dark: String,
    /// Подмешивать акцентный цвет рабочего стола вместо акцента темы.
    #[serde(default)]
    pub use_system_accent: bool,
    /// Рисовать кнопки окна так, как их рисует тема декораций рабочего стола.
    #[serde(default)]
    pub system_window_controls: bool,
    /// Просить композитор размывать фон за окном (KWin blur + contrast).
    #[serde(default)]
    pub window_blur: bool,
    /// Непрозрачность фоновых поверхностей: 1.0 — сплошной фон, меньше —
    /// сквозь окно виден рабочий стол (осмысленно вместе с `window_blur`).
    #[serde(default = "default_window_opacity")]
    pub window_opacity: f32,
    pub general: GeneralConfig,
    /// Ключи агентских инструментов, активных по умолчанию. Передаются в
    /// каждый запрос к модели. Дефолт — оба
    /// известных инструмента, чтобы новый пользователь сразу получил
    /// работающий агентский сценарий. `#[serde(default)]` + `default_tools_active`
    /// — старые конфиги без поля не теряют функциональность.
    #[serde(default = "default_tools_active")]
    pub tools_active: Vec<String>,
    /// Пул `autotools`: инструменты, чьи схемы модели не объявляются — она
    /// видит их список в описании `autotools` и подгружает нужный по
    /// запросу. С `tools_active` не пересекается (UI переносит ключ из
    /// одного списка в другой). Пустой — `autotools` модели не уходит.
    #[serde(default)]
    pub tools_auto: Vec<String>,
    /// Инструмент `notes` появился позже стартового набора: конфиг со
    /// своим списком `tools_active` его не содержит, и агент не видел бы
    /// заметок, пока пользователь не найдёт чип. Флаг — «инструмент уже
    /// предложен»: при первой загрузке без него `notes` дописывается в
    /// активные один раз, дальше выбор пользователя не трогается.
    #[serde(default)]
    pub tools_notes_introduced: bool,
    /// Миграция дефолтов сэмплинга (`migrate_sampling_defaults`) уже
    /// выполнена. Без флага она срабатывала на каждой загрузке, и выбранный
    /// пользователем потолок 131072 сбрасывался.
    #[serde(default)]
    pub sampling_defaults_migrated: bool,
    /// То же для `wizard` (08.09.2026): один раз дописывается в активные.
    #[serde(default)]
    pub tools_wizard_introduced: bool,
    /// То же для `view_media` (11.09.2026).
    #[serde(default)]
    pub tools_view_media_introduced: bool,
    /// Slug-id скилов, активных в правой панели (отображаются как
    /// «Активные» чипы рядом с tools_section). Сами скилы хранятся в
    /// `~/.config/synthos/skills/*.md` — здесь только подсветка.
    #[serde(default)]
    pub skills_active: Vec<String>,
    /// Пресеты ASR-моделей (распознавание речи). Каждая модель — локальный
    /// synaptix::facade::asr::Transcriber (Whisper / GigaAM).
    #[serde(default)]
    pub audio_models: Vec<AudioModelConfig>,
    /// Имя выбранной ASR-модели (стабильный id).
    #[serde(default)]
    pub selected_audio_model: Option<String>,
    /// Авто-запуск ASR-сервера при старте приложения, если выбрана модель.
    #[serde(default)]
    pub audio_autostart: bool,
    /// Список сессий редактора кода. Каждая сессия — отдельная вкладка
    /// в боковой панели; одна сессия = один независимый проект (своя папка,
    /// своё дерево, свои терминалы). Открытые файлы и активный файл не
    /// персистятся (per-keystroke persist на N сессий слишком дорог по IO).
    #[serde(default)]
    pub code_sessions: Vec<CodeSessionConfig>,
    /// Индекс активной сессии в `code_sessions` (стабильность по позиции,
    /// потому что runtime-id'ы переинициализируются при загрузке).
    #[serde(default)]
    pub active_code_session: Option<usize>,
    /// Страница, открытая на момент выхода (значение из `context::ROUTES`).
    /// Без неё старт всегда падал на `syn_chat`, даже когда у пользователя
    /// открыты только code-сессии, — приложение встречало пустым
    /// «Select or create a chat».
    #[serde(default)]
    pub last_route: Option<String>,
    /// Активный чат на момент выхода. Восстанавливается только если чат ещё
    /// существует (не удалён и не заархивирован).
    #[serde(default)]
    pub last_chat_id: Option<String>,
    /// LEGACY (эпоха одной активной папки): корень последнего открытого
    /// проекта. При первом запуске новой версии мигрируется в одну запись
    /// `code_sessions` и при следующем save'е затирается в `None`.
    #[serde(default)]
    pub last_code_folder: Option<String>,
    /// Семейство шрифта VTE-терминала. Пустая строка = `"monospace"`
    /// (font-kit резолвит в системный default — DejaVu Sans Mono / Menlo /
    /// Consolas в зависимости от платформы).
    #[serde(default)]
    pub terminal_font_family: String,
    /// Размер шрифта VTE-терминала в логических пикселях.
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: f32,
    /// Семейство шрифта CodeEditor. Пустая строка = `"monospace"`
    /// (системный моноширинный по умолчанию). Применяется через MSS-
    /// переменную `--code-editor-font-family`.
    #[serde(default)]
    pub code_editor_font_family: String,
    /// Размер шрифта CodeEditor в логических пикселях. Применяется через
    /// MSS-переменную `--code-editor-font-size`.
    #[serde(default = "default_code_editor_font_size")]
    pub code_editor_font_size: f32,
    /// Knowledge base / RAG настройки.
    #[serde(default)]
    pub kb: KbConfig,
    /// Закладки папок страницы SynExplorer. Список абсолютных путей; пользователь
    /// добавляет через `+` в левой панели (rfd::pick_folder). Сканирование
    /// каждой закладки даёт список `.syn` пакетов внутри.
    #[serde(default)]
    pub syn_explorer_bookmarks: Vec<String>,
    /// Положение левого разделителя страницы SynExplorer (список закладок ↔
    /// центр). Persist'ится при drag через [`SplitView::ratio_signal`].
    #[serde(default = "default_syn_explorer_left_split_ratio")]
    pub syn_explorer_left_split_ratio: f32,
    /// Положение правого разделителя (центр ↔ TreeView содержимого).
    #[serde(default = "default_syn_explorer_right_split_ratio")]
    pub syn_explorer_right_split_ratio: f32,
    /// Режим эксперта на странице пакетов: показывать префиксы тензоров,
    /// пофайловый состав, точность по группам слоёв и контрольные суммы.
    /// Липкий — включивший его один раз не должен включать снова.
    #[serde(default)]
    pub syn_explorer_expert: bool,
    /// Каталог-кэш для скачанных моделей со страницы HuggingFace.
    /// Пусто = `~/.local/share/synthos/hf` (см. [`resolve_hf_cache_dir`]).
    /// Спрашивается у пользователя при первом скачивании.
    #[serde(default)]
    pub hf_cache_dir: String,
    /// Лимит одновременно скачивающихся файлов на странице HuggingFace.
    /// Остальные ожидают своей очереди в `Pending`-статусе. Дефолт 3 —
    /// разумный баланс между throughput'ом и нагрузкой на сеть/диск.
    #[serde(default = "default_hf_concurrent_downloads")]
    pub hf_concurrent_downloads: u32,
    /// Кол-во HTTP-Range сегментов на один файл при сегментированной
    /// загрузке. >1 — файл качается параллельно несколькими соединениями,
    /// что заметно ускоряет крупные шарды на широких каналах. Минимум 1
    /// (=single-stream), максимум 16. Дефолт 4.
    #[serde(default = "default_hf_segments_per_file")]
    pub hf_segments_per_file: u32,
    /// Глобальный лимит скорости скачивания в МБ/с. `0` — без лимита (дефолт).
    /// Делится между всеми активными загрузками и их HTTP-Range-сегментами.
    #[serde(default = "default_hf_speed_limit_mbps")]
    pub hf_speed_limit_mbps: u32,
    /// «Скачать всё» пропускает тяжёлые/несовместимые форматы (onnx /
    /// openvino / fp32 / `.bin`). Дефолт — включено: эти форматы synthos не
    /// запускает, и тянуть их вместе с safetensors/gguf — лишний трафик.
    /// Кнопка «Скачать» на отдельном файле фильтр игнорирует.
    #[serde(default = "default_hf_skip_unwanted_formats")]
    pub hf_skip_unwanted_formats: bool,
    #[serde(default = "default_hf_gguf_support")]
    pub hf_gguf_support: bool,
    /// Куда пользователь перетащил голосовую FAB-кнопку: отступы от
    /// правого-нижнего угла окна `(справа, снизу)`. `None` — место по умолчанию.
    #[serde(default)]
    pub voice_fab_margin: Option<(f32, f32)>,
    /// Вид панели файлов на странице HuggingFace: `list` или `icons`.
    #[serde(default = "default_hf_files_view")]
    pub hf_files_view: String,
    /// Нижняя панель загрузок развёрнута (очередь и настройки видны).
    #[serde(default)]
    pub hf_dock_expanded: bool,
    /// Токен доступа HuggingFace (`hf_...`). Нужен для gated/private моделей
    /// (FLUX.1-dev, Llama и др.) и снимает rate-limit анонимных запросов.
    /// Создаётся на https://huggingface.co/settings/tokens (роль `read`).
    /// Пусто = анонимный доступ (только публичные модели).
    #[serde(default)]
    pub hf_token: String,
    /// Дефолтные параметры sampling для новых Syn-чатов. Каждый чат может
    /// переопределить их через `StoredChat.syn_params`.
    #[serde(default)]
    pub syn_chat_defaults: SamplingParams,
    /// Путь последнего открытого `.syn`-bundle модели Syn-чата. Загружается
    /// lazy при первом входе на `/syn_chat` (не на старте — 12 GB VRAM
    /// слишком дорого без необходимости).
    #[serde(default)]
    pub last_syn_model: Option<String>,
    /// Ключи элементов, недавно выбранных в глобальном поиске (свежие в
    /// начале, не больше `search::RECENT_LIMIT`). Показываются группой
    /// «Недавнее» при пустом запросе.
    #[serde(default)]
    pub search_recent: Vec<String>,
    /// Каталог моделей для ВСЕХ пайплайнов: .syn-бандлы, LoRA/.safetensors,
    /// HF-каталоги. Агентский инструмент `pipelines` (action=list)
    /// перечисляет его содержимое, чтобы модель могла проставить пути в
    /// чекпойнт-ноды без поиска по диску. По умолчанию —
    /// `~/Storage/syn_models` (см. [`default_models_dir`]).
    #[serde(default = "default_models_dir")]
    pub models_dir: String,
    /// Положение левого разделителя страницы Syn-чата (список чатов ↔ центр).
    #[serde(default = "default_syn_chat_left_split_ratio")]
    pub syn_chat_left_split_ratio: f32,
    /// Положение правого разделителя (центр ↔ панель параметров).
    #[serde(default = "default_syn_chat_right_split_ratio")]
    pub syn_chat_right_split_ratio: f32,
    /// Чат оторван в плавающее окно (`pages::syn_chat::float_window`);
    /// окно свёрнуто в кнопку-аватар; позиция и размер окна. Позиция и
    /// размер `None` — окно ещё ни разу не двигали/растягивали.
    #[serde(default)]
    pub syn_chat_detached: bool,
    #[serde(default)]
    pub syn_chat_window_minimized: bool,
    #[serde(default)]
    pub syn_chat_window_pos: Option<(f32, f32)>,
    #[serde(default)]
    pub syn_chat_window_size: Option<(f32, f32)>,
    /// Какие карточки боковых панелей чата раскрыты.
    #[serde(default)]
    pub syn_chat_cards: SynChatCardsConfig,
    /// Разделители страницы HuggingFace: список моделей ↔ README ↔ файлы.
    #[serde(default = "default_hf_left_split_ratio")]
    pub hf_left_split_ratio: f32,
    #[serde(default = "default_hf_right_split_ratio")]
    pub hf_right_split_ratio: f32,
    /// Разделители страницы настроек: разделы ↔ контент ↔ правая панель.
    #[serde(default = "default_settings_left_split_ratio")]
    pub settings_left_split_ratio: f32,
    #[serde(default = "default_settings_right_split_ratio")]
    pub settings_right_split_ratio: f32,
    /// Разделители страницы «Заметки»: дерево vault ↔ редактор ↔ панель.
    #[serde(default = "default_notes_left_split_ratio")]
    pub notes_left_split_ratio: f32,
    #[serde(default = "default_notes_right_split_ratio")]
    pub notes_right_split_ratio: f32,
    /// Папка vault'а первой волны — источник разовой миграции в проект.
    #[serde(default)]
    pub notes_vault_path: String,
    /// Файл проекта заметок (`.syn`) времён одного проекта. Пусто — дефолт
    /// `~/Documents/SynthOS Notes.syn` (см. `pages::notes::project`). Теперь
    /// только источник миграции в `notes_projects`.
    #[serde(default)]
    pub notes_project_path: String,
    /// Активная страница (id), раскрытые узлы дерева и плитка рейла
    /// (штамп открытия; None — плитка закрыта) — тоже времён одного
    /// проекта, читаются только миграцией.
    #[serde(default)]
    pub notes_active: Option<String>,
    #[serde(default)]
    pub notes_expanded: Vec<String>,
    #[serde(default)]
    pub notes_tile_opened_at: Option<u64>,
    /// Открытые проекты заметок — по плитке рейла на каждый. `None` —
    /// конфиг ещё не видел нескольких проектов: список собирается из полей
    /// выше (см. `pages::notes::projects::restore`).
    #[serde(default)]
    pub notes_projects: Option<Vec<NotesProjectConfig>>,
    /// Проект, который показан на странице заметок (путь файла).
    #[serde(default)]
    pub notes_active_project: String,
    /// Недавние проекты, свежие первыми.
    #[serde(default)]
    pub notes_recent: Vec<String>,
    /// Видимость левой/правой панели по страницам — тогглы в общей шапке.
    #[serde(default)]
    pub panels: PanelsConfig,
    /// Разделители в списке плиток нав-рейла: unix-миллисекунды создания
    /// каждого. Плитки и разделители сортируются по времени вместе, так что
    /// разделитель, добавленный через «+», встаёт после последней плитки.
    #[serde(default)]
    pub rail_separators: Vec<u64>,
    /// Ручной порядок плиток рейла после перетаскивания: ключи
    /// `code:<created_at>` / `graph:<id>` / `chat:<id>` / `sep:<ts>` в порядке
    /// показа. Плитки, которых здесь нет (новые), встают в конец по времени
    /// создания; ключи исчезнувших плиток отбрасываются при следующем
    /// перетаскивании. Пусто — порядок только по `created_at`.
    #[serde(default)]
    pub rail_order: Vec<String>,
    /// Недавние эмодзи панели ввода чата, свежие первыми (до 16).
    #[serde(default)]
    pub chat_recent_emoji: Vec<String>,
    /// Устаревшее: одиночный системный prompt Syn-чата. С библиотекой
    /// пресетов (`syn_chat::prompt_presets`, файл `syn_system_prompts.json`)
    /// больше не пишется; читается один раз при первом запуске без файла
    /// библиотеки — текст становится первым пресетом.
    #[serde(default)]
    pub syn_chat_system_prompt: String,
    /// Потолок vision-токенов на одну картинку-вложение в Syn-чате.
    ///
    /// У Muse Glimmer `processor_config.json` разрешает до 4096 токенов на
    /// картинку — это заметный кусок контекста и долгий prefill на каждое
    /// вложение. 1024 токена ≈ 896×896 px после smart-resize: хватает,
    /// чтобы читать текст на скриншоте, и не съедает чат. `0` — снять
    /// ограничение и отдать решение конфигу модели.
    #[serde(default = "default_syn_chat_max_image_tokens")]
    pub syn_chat_max_image_tokens: usize,
    /// Как настроена каждая модель: ключ — путь к бандлу. Режим `optimal`
    /// значит «как решит движок»: он знает, какие пути у какой архитектуры
    /// выверены замерами. `custom` кладёт поверх ручные правки — заполненные
    /// поля перебивают выверенное, пустые остаются как в `optimal`.
    ///
    /// Редактируется на странице Settings → AI Models.
    #[serde(default)]
    pub model_profiles: std::collections::BTreeMap<String, ModelProfileConfig>,
    /// Префикс-KV: держать посчитанный контекст диалога между ходами, чтобы
    /// ход дописывал в KV только новый хвост промпта, а не считал историю
    /// заново. Стоит VRAM (кэш живёт между ходами, ёмкость кратна 16384
    /// токенам) и выключается, когда в сообщении есть вложения — медиа-путь
    /// префилла идёт по эмбеддингам, а vision-башне нужна та же память.
    /// Default = `true`.
    #[serde(default = "default_true")]
    pub syn_chat_prefix_kv: bool,
}

fn default_true() -> bool { true }
// 256: единственный размер, безопасный на 24 ГБ (single-shot/1024 упирается в
// VRAM поверх ~18 ГБ весов 27B) при prefill ~1.5-1.6k ток/с; движок сам
// округляет к кратному 64 (границы GDN-скана).

// ─────────────────────────────────────────────────────────────────────────────
// ModelProfileConfig — как настроена одна модель
// ─────────────────────────────────────────────────────────────────────────────

/// Настройки одной модели. `mode` — `"optimal"` или `"custom"`.
///
/// В режиме `optimal` переопределения не читаются вовсе: настройки целиком
/// берутся у движка, который знает, какие пути у какой архитектуры выверены
/// замерами. В `custom` заполненные поля перебивают выверенное, пустые
/// остаются как в `optimal` — поэтому «поправить одно» не тянет за собой
/// остальное.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProfileConfig {
    pub mode: String,
    pub weights_storage: Option<String>,
    pub compute: Option<String>,
    pub kv_dtype: Option<String>,
    pub lm_head_storage: Option<String>,
    pub embed_storage: Option<String>,
    pub graph_decode: Option<bool>,
    pub speculation: Option<bool>,
    pub layer_sync: Option<String>,
    /// Перекодировать уже квантованный бандл (NVFP4/MXFP8/GGUF) в форматы
    /// профиля при загрузке — двойной квант, осознанно.
    #[serde(default)]
    pub transcode: Option<bool>,
}

impl Default for ModelProfileConfig {
    fn default() -> Self {
        Self {
            mode: "optimal".into(),
            weights_storage: None,
            compute: None,
            kv_dtype: None,
            lm_head_storage: None,
            embed_storage: None,
            graph_decode: None,
            speculation: None,
            layer_sync: None,
            transcode: None,
        }
    }
}

/// Настройки модели после разрешения: что движок посчитал выверенным, с
/// наложенными правками пользователя.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub policy: synaptix::facade::llm::QuantPolicy,
    pub graph_decode: bool,
    pub speculation: bool,
    pub layer_sync: synaptix::facade::llm::LayerSyncMode,
}

impl ResolvedProfile {
    /// Применить рантайм-часть профиля. Квант-политика так не применяется —
    /// она нужна при загрузке весов.
    pub fn apply_runtime(&self) {
        use synaptix::facade::llm as f;
        f::set_graph_decode_enabled(self.graph_decode);
        f::set_mtp_enabled(self.speculation);
        f::set_dflash_enabled(self.speculation);
        f::set_layer_sync_mode(self.layer_sync);
    }
}

impl ModelProfileConfig {
    pub fn is_custom(&self) -> bool {
        self.mode == "custom"
    }

    /// Выверенные движком настройки под этот бандл плюс ручные правки, если
    /// режим `custom`.
    pub fn resolve(&self, path: &std::path::Path) -> ResolvedProfile {
        use synaptix::facade::llm::{KvDtypePolicy, LayerSyncMode};
        let opt = synaptix::facade::llm::optimal_profile(path);
        let mut out = ResolvedProfile {
            policy: opt.policy,
            graph_decode: opt.graph_decode,
            speculation: opt.speculation,
            layer_sync: opt.layer_sync,
        };
        if !self.is_custom() {
            return out;
        }
        if let Some(v) = &self.weights_storage {
            out.policy.weights_storage = parse_storage_dtype(v, out.policy.weights_storage);
        }
        if let Some(v) = &self.compute {
            out.policy.compute = parse_compute_dtype(v, out.policy.compute);
        }
        if let Some(v) = &self.kv_dtype {
            if let Some(kv) = KvDtypePolicy::from_name(v) {
                out.policy.kv_dtype = kv;
            }
        }
        if let Some(v) = &self.lm_head_storage {
            out.policy.lm_head_storage = parse_storage_dtype(v, out.policy.lm_head_storage);
        }
        if let Some(v) = &self.embed_storage {
            out.policy.embed_storage = parse_storage_dtype(v, out.policy.embed_storage);
        }
        if let Some(v) = self.graph_decode {
            out.graph_decode = v;
        }
        if let Some(v) = self.speculation {
            out.speculation = v;
        }
        if let Some(v) = &self.layer_sync {
            if let Ok(m) = v.parse::<LayerSyncMode>() {
                out.layer_sync = m;
            }
        }
        if let Some(v) = self.transcode {
            out.policy.transcode = v;
        }
        out.policy.preset_name = "custom".into();
        out
    }

    /// Значение поля для UI: своё, если задано, иначе выверенное движком.
    pub fn effective_storage(&self, path: &std::path::Path) -> ResolvedProfile {
        self.resolve(path)
    }
}

/// Профиль модели по её пути: своя запись или `optimal` по умолчанию.
pub fn model_profile_of(
    profiles: &std::collections::BTreeMap<String, ModelProfileConfig>,
    path: &std::path::Path,
) -> ModelProfileConfig {
    profiles
        .get(&path.display().to_string())
        .cloned()
        .unwrap_or_default()
}

/// Разрешённый профиль модели: выверенное движком плюс ручные правки. Сразу
/// применяет рантайм-часть — дальше остаётся отдать `policy` загрузчику.
pub fn resolve_model_profile(
    profiles: &std::collections::BTreeMap<String, ModelProfileConfig>,
    path: &std::path::Path,
) -> ResolvedProfile {
    let resolved = model_profile_of(profiles, path).resolve(path);
    resolved.apply_runtime();
    resolved
}

/// Парсер строки storage-dtype → `synaptix_core::dtype::DType`. Quantized
/// псевдонимы (`"fp8e4m3"`/`"fp8"`/`"mxfp8"` → MXFP8) и legacy GGML-форматы
/// (`"q8_0"`/`"q4_0"`, не поддерживаемые synaptix) → `fallback`. Прочие
/// форматы делегируются `precision::parse_dtype`.
fn parse_storage_dtype(s: &str, fallback: synaptix_core::dtype::DType) -> synaptix_core::dtype::DType {
    match s.to_ascii_lowercase().as_str() {
        "fp8e4m3" | "fp8" | "mxfp8" => synaptix_core::dtype::DType::MXFP8,
        other => synaptix_core::precision::parse_dtype(other).unwrap_or(fallback),
    }
}

/// Парсер `"f16"/"bf16"/"f32"` → `synaptix_core::dtype::DType`. Любая другая
/// строка (включая quantized форматы, для compute невалидные) → `fallback`.
fn parse_compute_dtype(s: &str, fallback: synaptix_core::dtype::DType) -> synaptix_core::dtype::DType {
    match s.to_ascii_lowercase().as_str() {
        "f16" => synaptix_core::dtype::DType::F16,
        "bf16" => synaptix_core::dtype::DType::BF16,
        "f32" => synaptix_core::dtype::DType::F32,
        _ => fallback,
    }
}

/// `synaptix_core::dtype::DType` → стабильная строка для config.json/UI.
pub fn dtype_name(dt: synaptix_core::dtype::DType) -> String {
    use synaptix_core::dtype::DType;
    match dt {
        DType::F32 | DType::F16 | DType::BF16 | DType::NVFP4 | DType::MXFP8 | DType::Sq { .. } => {
            synaptix_core::precision::dtype_name(dt)
        }
        _ => "f16".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Knowledge base config
// ─────────────────────────────────────────────────────────────────────────────

/// Глобальные параметры RAG-подсистемы.
///
/// `serde(default)` на каждом поле гарантирует совместимость со старыми
/// конфигами без секции `kb` — все поля получат свои дефолты.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbConfig {
    /// Ручной путь к эмбеддеру: `.syn`-бандл или каталог HF-снапшота.
    /// Пусто (дефолт) = найти самим в каталоге из «AI модели»
    /// (`bge-m3.syn`), см. `kb::models`. Несуществующий путь поиску не мешает.
    pub embedder_model_path: String,
    /// Устройство инференса: `"cpu"`, `"cuda"`, `"metal"`. Для CUDA/Metal
    /// нужен соответствующий feature-флаг билда (`kb-cuda`, `kb-metal`).
    pub embedder_device: String,
    /// Тип данных вычислений: `"f32"`, `"f16"`, `"bf16"`. F32 — самый
    /// совместимый, F16 быстрее на GPU.
    pub embedder_dtype: String,
    /// Целевая длина чанка в токенах. Должна быть < `embedder.max_tokens`.
    pub chunk_target_tokens: usize,
    /// Перекрытие соседних чанков в токенах (для контекста на границах).
    pub chunk_overlap_tokens: usize,
    /// Сколько фрагментов возвращать в `kb_search` по умолчанию.
    pub default_top_k: usize,
    /// Включать ли auto-augment system prompt'а на старте каждого turn'а
    /// (только когда выбрана хотя бы одна коллекция). По умолчанию выключено
    /// — основной канал интеграции с моделью — tool `kb_search`.
    pub auto_augment_default: bool,
    /// Каталог с .sqlite-файлами коллекций. Дефолт: `~/.config/synthos/kb`.
    /// Пусто = использовать дефолтный путь (`default_kb_dir`).
    pub kb_dir: String,
    /// Бюджет токенов на блок augment'а. ~1500 ≈ 6KB текста — типично
    /// безопасно для контекстного окна 32k+.
    pub augment_token_budget: usize,
    /// Авто-разрешение tool `kb_search` (read-only). Применяется один раз
    /// при первом старте — добавляет override в `tool_approval_overrides`.
    pub auto_approve_kb_search: bool,
    /// Режим векторного индекса: `"auto"` (по умолчанию — sqlite-vec при
    /// большом размере коллекции, иначе full-scan), `"scan"` (всегда scan),
    /// `"sqlite-vec"` (всегда vec0). Влияет на новые `Store::open`.
    pub vector_index_kind: String,
    /// Порог для `auto`-режима: количество чанков, начиная с которого
    /// автоматически включается sqlite-vec. Меньшие коллекции остаются
    /// на full-scan (там это быстрее по latency и не требует extension'а).
    pub vector_index_threshold: usize,
    /// Ручной путь к cross-encoder реранкеру — как `embedder_model_path`:
    /// пусто = найти `bge-reranker-v2-m3.syn` самим. Выключается реранкер
    /// флагом `reranker_enabled`, а не пустым путём.
    pub reranker_model_path: String,
    /// Устройство инференса реранкера: `"cpu"` / `"cuda"` / `"metal"`.
    pub reranker_device: String,
    /// Тип данных для реранкера: `"f32"` / `"f16"` / `"bf16"`.
    pub reranker_dtype: String,
    /// Глобальный флаг — даже если путь задан, можно временно выключить.
    pub reranker_enabled: bool,
    /// Множитель over-fetch'а перед reranking'ом. Hybrid RRF собирает
    /// `top_k * multiplier` кандидатов, реранкер сортирует и оставляет
    /// `top_k`. Чем больше — тем выше recall, но тем дороже cross-encoder.
    pub reranker_top_k_multiplier: usize,
    /// Максимум токенов в одном входе `[CLS] q [SEP] d [SEP]`.
    pub reranker_max_tokens: usize,
}

impl Default for KbConfig {
    fn default() -> Self {
        Self {
            embedder_model_path: String::new(),
            embedder_device: "cpu".to_string(),
            embedder_dtype: "f32".to_string(),
            chunk_target_tokens: 512,
            chunk_overlap_tokens: 64,
            default_top_k: 5,
            auto_augment_default: false,
            kb_dir: String::new(),
            augment_token_budget: 1500,
            auto_approve_kb_search: true,
            vector_index_kind: "auto".to_string(),
            vector_index_threshold: 100_000,
            reranker_model_path: String::new(),
            reranker_device: "cpu".to_string(),
            reranker_dtype: "f32".to_string(),
            reranker_enabled: true,
            reranker_top_k_multiplier: 4,
            reranker_max_tokens: 512,
        }
    }
}

/// `~/.config/synthos/kb` — рядом с `blobs/`, `chats/`. Создаётся лениво
/// при первом scan'е через `CollectionRegistry`.
pub fn default_kb_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/kb")
}

/// Дефолт для `AppConfig.terminal_font_size`. Совпадает с дефолтом
/// `syngui::widgets::Terminal` (13px), вынесен функцией ради `#[serde(default)]`
/// для прозрачной обратной совместимости со старыми конфигами без поля.
pub fn default_terminal_font_size() -> f32 {
    13.0
}

/// Дефолт `CodeSessionConfig.split_ratio` (vertical editor↔terminal). 0.72 —
/// совпадает со значением `initial_ratio(0.72)` в `code_editor::view`, чтобы
/// первый запуск выглядел одинаково с прежним хардкодом.
pub fn default_code_editor_split_ratio() -> f32 {
    0.72
}

/// Дефолт `CodeSessionConfig.left_split_ratio` (horizontal file_tree↔center).
/// Подобран так, чтобы при типичной ширине окна 1600px file_tree был ≈300px
/// (как при прежнем хардкоде MSS `width: 300px;`).
pub fn default_code_editor_left_split_ratio() -> f32 {
    0.18
}

/// Дефолт `CodeSessionConfig.right_split_ratio` (horizontal center↔open_files).
/// Подобран так, чтобы при типичной ширине окна 1600px правая панель
/// open_files была ≈260px (как при прежнем хардкоде MSS `width: 260px;`).
pub fn default_code_editor_right_split_ratio() -> f32 {
    0.84
}

/// Дефолт для левого разделителя `SynExplorer` (список закладок/пакетов ↔
/// центр). При ширине окна 1600px даёт ~300px на левую панель.
pub fn default_syn_explorer_left_split_ratio() -> f32 {
    0.19
}

/// Дефолт для правого разделителя `SynExplorer` (центр ↔ TreeView пакета).
/// При ширине окна 1600px даёт ~280px на правую панель.
pub fn default_syn_explorer_right_split_ratio() -> f32 {
    0.78
}

/// Дефолт для `AppConfig.code_editor_font_size` — 13px (совпадает с прежним
/// хардкодом в `.code-editor-mle`, чтобы старые конфиги выглядели как раньше).
pub fn default_code_editor_font_size() -> f32 {
    13.0
}

/// Дефолт для `AppConfig.window_opacity` — сплошной фон, как было до
/// появления настройки «стекло».
pub fn default_window_opacity() -> f32 {
    1.0
}

/// Дефолтный каталог моделей: `~/Storage/syn_models`. Резолвится от `HOME`
/// на текущей машине, а не хардкодом пути — на чужой системе дефолт тоже
/// осмыслен.
pub fn default_models_dir() -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    format!("{home}/Storage/syn_models")
}

/// Каталог моделей с раскрытием `~` в начале пути.
pub fn resolve_models_dir(raw: &str) -> std::path::PathBuf {
    let raw = raw.trim();
    if raw.is_empty() {
        return std::path::PathBuf::from(default_models_dir());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(raw)
}

/// Стартовый набор активных инструментов — используется и как `Default`,
/// и как fallback для конфигов без поля.
pub fn default_tools_active() -> Vec<String> {
    vec![
        "bash".to_string(),
        "web".to_string(),
        "system".to_string(),
        "pipelines".to_string(),
        "notes".to_string(),
        "wizard".to_string(),
        "view_media".to_string(),
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: String::new(), // пусто = default_theme() из theme_data
            follow_system_theme: false,
            theme_light: String::new(),
            theme_dark: String::new(),
            use_system_accent: false,
            system_window_controls: false,
            window_blur: false,
            window_opacity: default_window_opacity(),
            general: GeneralConfig::default(),
            tools_active: default_tools_active(),
            tools_auto: Vec::new(),
            tools_notes_introduced: true,
            sampling_defaults_migrated: true,
            tools_wizard_introduced: true,
            tools_view_media_introduced: true,
            skills_active: Vec::new(),
            audio_models: Vec::new(),
            selected_audio_model: None,
            audio_autostart: false,
            code_sessions: Vec::new(),
            active_code_session: None,
            last_route: None,
            last_chat_id: None,
            last_code_folder: None,
            terminal_font_family: String::new(),
            terminal_font_size: default_terminal_font_size(),
            code_editor_font_family: String::new(),
            code_editor_font_size: default_code_editor_font_size(),
            kb: KbConfig::default(),
            syn_explorer_bookmarks: Vec::new(),
            syn_explorer_left_split_ratio: default_syn_explorer_left_split_ratio(),
            syn_explorer_expert: false,
            syn_explorer_right_split_ratio: default_syn_explorer_right_split_ratio(),
            hf_cache_dir: String::new(),
            hf_concurrent_downloads: default_hf_concurrent_downloads(),
            hf_segments_per_file: default_hf_segments_per_file(),
            hf_speed_limit_mbps: default_hf_speed_limit_mbps(),
            hf_skip_unwanted_formats: default_hf_skip_unwanted_formats(),
            hf_gguf_support: default_hf_gguf_support(),
            voice_fab_margin: None,
            hf_files_view: default_hf_files_view(),
            hf_dock_expanded: false,
            hf_token: String::new(),
            syn_chat_defaults: SamplingParams::default(),
            last_syn_model: None,
            search_recent: Vec::new(),
            syn_chat_left_split_ratio: default_syn_chat_left_split_ratio(),
            syn_chat_right_split_ratio: default_syn_chat_right_split_ratio(),
            syn_chat_detached: false,
            syn_chat_window_minimized: false,
            syn_chat_window_pos: None,
            syn_chat_window_size: None,
            syn_chat_cards: SynChatCardsConfig::default(),
            hf_left_split_ratio: default_hf_left_split_ratio(),
            hf_right_split_ratio: default_hf_right_split_ratio(),
            settings_left_split_ratio: default_settings_left_split_ratio(),
            settings_right_split_ratio: default_settings_right_split_ratio(),
            panels: PanelsConfig::default(),
            notes_left_split_ratio: default_notes_left_split_ratio(),
            notes_right_split_ratio: default_notes_right_split_ratio(),
            notes_vault_path: String::new(),
            notes_project_path: String::new(),
            notes_active: None,
            notes_expanded: Vec::new(),
            notes_tile_opened_at: None,
            notes_projects: None,
            notes_active_project: String::new(),
            notes_recent: Vec::new(),
            rail_separators: Vec::new(),
            rail_order: Vec::new(),
            chat_recent_emoji: Vec::new(),
            syn_chat_system_prompt: String::new(),
            syn_chat_max_image_tokens: default_syn_chat_max_image_tokens(),
            model_profiles: std::collections::BTreeMap::new(),
            syn_chat_prefix_kv: default_true(),
            models_dir: default_models_dir(),
        }
    }
}

fn default_hf_concurrent_downloads() -> u32 { 3 }
fn default_hf_segments_per_file() -> u32 { 4 }
fn default_hf_speed_limit_mbps() -> u32 { 0 }
fn default_hf_skip_unwanted_formats() -> bool { true }
fn default_hf_gguf_support() -> bool { false }
fn default_hf_files_view() -> String { "list".to_string() }

/// Дефолт потолка vision-токенов на картинку — см. `syn_chat_max_image_tokens`.
pub fn default_syn_chat_max_image_tokens() -> usize {
    1024
}

/// Дефолт левого разделителя страницы Syn-чата. При окне 1600px даёт ~300px.
pub fn default_syn_chat_left_split_ratio() -> f32 {
    0.19
}

/// Дефолт правого разделителя. При окне 1600px даёт ~320px на панель параметров.
pub fn default_syn_chat_right_split_ratio() -> f32 {
    0.80
}

/// HuggingFace: список моделей слева ~26% ширины.
pub fn default_hf_left_split_ratio() -> f32 {
    0.26
}

pub fn default_notes_left_split_ratio() -> f32 {
    0.22
}

pub fn default_notes_right_split_ratio() -> f32 {
    0.78
}

/// HuggingFace: панель файлов справа ~30% ширины.
pub fn default_hf_right_split_ratio() -> f32 {
    0.70
}

/// Настройки: колонка разделов ~19% (≈300px при 1600px).
pub fn default_settings_left_split_ratio() -> f32 {
    0.19
}

/// Настройки: правая панель ~24% (≈360px при 1600px).
pub fn default_settings_right_split_ratio() -> f32 {
    0.76
}

/// Открытый проект заметок: файл, плитка рейла и где в нём остановились.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NotesProjectConfig {
    pub path: String,
    /// Штамп появления плитки в рейле (unix-миллисекунды).
    pub opened_at: u64,
    /// Активная страница (id) и раскрытые узлы дерева.
    pub active_page: Option<String>,
    pub expanded: Vec<String>,
}

/// Видимость боковых панелей по страницам. Каждая страница трёхпанельного
/// каркаса (`components::workspace_frame`) держит пару флагов; тогглы в
/// общей шапке пишут в сигналы `context::PanelsCtx`, автосейв — сюда.
/// По умолчанию всё открыто — как выглядело приложение до тогглов.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelsConfig {
    #[serde(default = "default_true")]
    pub syn_chat_left: bool,
    #[serde(default = "default_true")]
    pub syn_chat_right: bool,
    #[serde(default = "default_true")]
    pub code_left: bool,
    #[serde(default = "default_true")]
    pub code_right: bool,
    #[serde(default = "default_true")]
    pub notes_left: bool,
    #[serde(default = "default_true")]
    pub notes_right: bool,
    #[serde(default = "default_true")]
    pub syn_explorer_left: bool,
    #[serde(default = "default_true")]
    pub syn_explorer_right: bool,
    #[serde(default = "default_true")]
    pub huggingface_left: bool,
    #[serde(default = "default_true")]
    pub huggingface_right: bool,
    #[serde(default = "default_true")]
    pub settings_left: bool,
    #[serde(default = "default_true")]
    pub settings_right: bool,
}

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            syn_chat_left: true,
            syn_chat_right: true,
            code_left: true,
            code_right: true,
            notes_left: true,
            notes_right: true,
            syn_explorer_left: true,
            syn_explorer_right: true,
            huggingface_left: true,
            huggingface_right: true,
            settings_left: true,
            settings_right: true,
        }
    }
}

/// Раскрытие сворачиваемых карточек боковых панелей Syn-чата
/// (`components::collapsible_card`): слева «Инструменты / Autotools /
/// Скилы», справа «Модель / Thinking / Sampling / Контекст / Система».
/// Сигналы — `syn_chat::state::CardsOpen`, автосейв пишет их сюда.
/// Раскрыто всё, кроме Sampling: девять его слайдеров выдавливали
/// системный prompt за нижний край панели, а трогают их редко.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SynChatCardsConfig {
    #[serde(default = "default_true")]
    pub tools: bool,
    #[serde(default = "default_true")]
    pub autotools: bool,
    #[serde(default = "default_true")]
    pub skills: bool,
    #[serde(default = "default_true")]
    pub model: bool,
    #[serde(default = "default_true")]
    pub thinking: bool,
    #[serde(default)]
    pub sampling: bool,
    #[serde(default = "default_true")]
    pub context: bool,
    #[serde(default = "default_true")]
    pub system: bool,
}

impl Default for SynChatCardsConfig {
    fn default() -> Self {
        Self {
            tools: true,
            autotools: true,
            skills: true,
            model: true,
            thinking: true,
            sampling: false,
            context: true,
            system: true,
        }
    }
}

/// Unix-миллисекунды «сейчас» — общий штамп `created_at` для плиток рейла.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Резолвит пользовательский путь кэша HF-моделей. Пустая строка — дефолт
/// `~/.local/share/synthos/hf` (XDG_DATA_HOME-конвенция). Сама папка
/// создаётся лениво в [`crate::pages::huggingface::download::start_download`].
pub fn resolve_hf_cache_dir(stored: &str) -> PathBuf {
    if !stored.trim().is_empty() {
        return crate::paths::expand_home(stored);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/synthos/hf")
}

impl AppConfig {
    /// `$HOME/.config/synthos/config.json`.
    pub fn path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".config/synthos/config.json")
    }

    /// Загружает конфиг с диска. На любую ошибку — `eprintln!` + `Default`
    /// с попыткой тут же сохранить «чистый» конфиг, чтобы следующий запуск
    /// стартовал с валидного файла.
    /// Один раз включить инструмент `notes` в конфиге, сохранённом до его
    /// появления (см. `tools_notes_introduced`).
    pub fn introduce_notes_tool(&mut self) {
        if self.tools_notes_introduced {
            return;
        }
        self.tools_notes_introduced = true;
        if !self.tools_active.iter().any(|k| k == "notes") {
            self.tools_active.push("notes".to_string());
        }
    }

    /// Один раз включить инструмент `wizard` (см. `tools_wizard_introduced`).
    pub fn introduce_wizard_tool(&mut self) {
        if self.tools_wizard_introduced {
            return;
        }
        self.tools_wizard_introduced = true;
        if !self.tools_active.iter().any(|k| k == "wizard") {
            self.tools_active.push("wizard".to_string());
        }
    }

    /// Один раз включить инструмент `view_media` (см.
    /// `tools_view_media_introduced`).
    pub fn introduce_view_media_tool(&mut self) {
        if self.tools_view_media_introduced {
            return;
        }
        self.tools_view_media_introduced = true;
        if !self.tools_active.iter().any(|k| k == "view_media") {
            self.tools_active.push("view_media".to_string());
        }
    }

    /// `web_read` / `web_search` — инструменты до слияния в `web`. Каталог их
    /// не знает: чипа нет, снять нельзя, а в системный промпт они уходили
    /// именами. На их месте встаёт `web`, если его ещё нет ни в активных,
    /// ни в пуле.
    pub fn migrate_legacy_web_tool_keys(&mut self) {
        let is_legacy = |k: &String| k == "web_read" || k == "web_search";
        let Some(pos) = self.tools_active.iter().position(is_legacy) else {
            return;
        };
        self.tools_active.retain(|k| !is_legacy(k));
        let has_web = |list: &[String]| list.iter().any(|k| k == "web");
        if !has_web(&self.tools_active) && !has_web(&self.tools_auto) {
            self.tools_active.insert(pos, "web".to_string());
        }
    }

    /// Одноразовая миграция дефолтов сэмплинга Syn-чата (03.09.2026): старый
    /// набор 0.7 / 0.9 / 40 / repeat 1.05 → рекомендованные Qwen 0.6 / 0.95 /
    /// 20 без штрафа за повторы. Переписываем только конфиг, в котором лежит
    /// ровно старый дефолт, — значения, изменённые пользователем, остаются.
    /// Параметры уже существующих чатов (per-chat override) не трогаем.
    pub fn migrate_sampling_defaults(&mut self) {
        if self.sampling_defaults_migrated {
            return;
        }
        self.sampling_defaults_migrated = true;
        // Потолок ответа до 07.09.2026 — 131072 (= `max_seq_len`): он уходил
        // в план KV каждого хода и занимал память под ринг впустую.
        // Нетронутый дефолт переводим на новый, изменённое пользователем
        // значение не трогаем.
        if self.syn_chat_defaults.max_new_tokens == SamplingParams::LEGACY_MAX_NEW_TOKENS {
            self.syn_chat_defaults.max_new_tokens = SamplingParams::default().max_new_tokens;
        }
        if self.syn_chat_defaults == SamplingParams::legacy_v1() {
            self.syn_chat_defaults = SamplingParams::default();
        }
    }

    /// Держать на всё время чтения-правки-записи, если [`Self::update`] не
    /// подходит (автосейв собирает конфиг из сигналов и нескольких `load()`).
    /// `load`/`save` сами блокировку не берут — повторного захвата нет.
    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        CONFIG_RW_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Поправить конфиг на диске: свежий `load()`, правка, `save()` — под
    /// общей блокировкой, чтобы параллельная запись не потеряла ни одну из
    /// правок. Возвращает сохранённый конфиг.
    pub fn update(edit: impl FnOnce(&mut Self)) -> Self {
        let _guard = Self::lock();
        let mut cfg = Self::load();
        edit(&mut cfg);
        cfg.save();
        cfg
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut cfg) => {
                    // Миграция со старого однополевого dtype в storage/compute.
                    for m in &mut cfg.audio_models {
                        m.migrate_legacy_dtype();
                    }
                    cfg.introduce_notes_tool();
                    cfg.introduce_wizard_tool();
                    cfg.introduce_view_media_tool();
                    cfg.migrate_legacy_web_tool_keys();
                    cfg.migrate_sampling_defaults();
                    cfg
                }
                Err(e) => {
                    // Автосейв при старте запишет поверх то, что вернёт
                    // `load()`. Чтобы он не затёр настройки пользователя
                    // дефолтами, битый файл откладываем в сторону: его можно
                    // починить руками и вернуть. Следующие `load()` видят
                    // отсутствующий файл и начинают с чистого.
                    match crate::fsutil::quarantine(&path) {
                        Ok(backup) => {
                            eprintln!(
                                "[synthos] Не удалось распарсить {:?}: {e}. \
                                 Файл сохранён как {:?}, беру настройки по умолчанию.",
                                path, backup
                            );
                        }
                        Err(re) => eprintln!(
                            "[synthos] Не удалось распарсить {:?}: {e}; \
                             отложить файл тоже не вышло: {re}.",
                            path
                        ),
                    }
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Self::default();
                cfg.save();
                cfg
            }
            Err(e) => {
                eprintln!(
                    "[synthos] Не удалось прочитать {:?}: {e}. \
                     Использую настройки по умолчанию.",
                    path
                );
                Self::default()
            }
        }
    }

    /// Сохраняет конфиг. Ошибки не панические: логируются и игнорируются,
    /// чтобы проблема с файловой системой не крошила UI.
    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("[synthos] Не удалось создать {:?}: {e}", parent);
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                // Атомарно: процесс, убитый посреди записи, не должен
                // оставить обрезанный JSON — следующий запуск его не разберёт.
                if let Err(e) = crate::fsutil::write_atomic(&path, content) {
                    eprintln!("[synthos] Не удалось записать {:?}: {e}", path);
                }
            }
            Err(e) => eprintln!("[synthos] Ошибка сериализации конфига: {e}"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Миграция сэмплинга — один раз: потолок 131072, выставленный
    /// пользователем после неё, не сбрасывается.
    #[test]
    fn sampling_migration_runs_once() {
        let mut cfg = AppConfig { sampling_defaults_migrated: false, ..AppConfig::default() };
        cfg.syn_chat_defaults.max_new_tokens = SamplingParams::LEGACY_MAX_NEW_TOKENS;
        cfg.migrate_sampling_defaults();
        assert_eq!(cfg.syn_chat_defaults.max_new_tokens, SamplingParams::default().max_new_tokens);
        cfg.syn_chat_defaults.max_new_tokens = SamplingParams::LEGACY_MAX_NEW_TOKENS;
        cfg.migrate_sampling_defaults();
        assert_eq!(cfg.syn_chat_defaults.max_new_tokens, SamplingParams::LEGACY_MAX_NEW_TOKENS);
    }

    /// Старый конфиг со своим списком инструментов получает `notes` один
    /// раз; выключенный пользователем после этого — не возвращается.
    #[test]
    fn notes_tool_is_introduced_once() {
        let mut cfg: AppConfig =
            serde_json::from_str(r#"{"tools_active":["bash","web"]}"#).expect("конфиг без флага");
        assert!(!cfg.tools_notes_introduced);
        cfg.introduce_notes_tool();
        assert_eq!(cfg.tools_active, ["bash", "web", "notes"]);
        assert!(cfg.tools_notes_introduced);
        cfg.tools_active.retain(|k| k != "notes");
        cfg.introduce_notes_tool();
        assert_eq!(cfg.tools_active, ["bash", "web"], "выбор пользователя не трогается");
        // Свежий конфиг: notes уже в стартовом наборе, флаг взведён.
        assert!(AppConfig::default().tools_active.iter().any(|k| k == "notes"));
        assert!(AppConfig::default().tools_notes_introduced);
    }

    #[test]
    fn wizard_tool_is_introduced_once() {
        let mut cfg: AppConfig =
            serde_json::from_str(r#"{"tools_active":["bash"],"tools_notes_introduced":true}"#).unwrap();
        assert!(!cfg.tools_wizard_introduced);
        cfg.introduce_wizard_tool();
        assert_eq!(cfg.tools_active, ["bash", "wizard"]);
        cfg.tools_active.retain(|k| k != "wizard");
        cfg.introduce_wizard_tool();
        assert_eq!(cfg.tools_active, ["bash"], "выбор пользователя не трогается");
        assert!(AppConfig::default().tools_active.iter().any(|k| k == "wizard"));
    }

    #[test]
    fn view_media_tool_is_introduced_once() {
        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"tools_active":["bash"],"tools_notes_introduced":true,"tools_wizard_introduced":true}"#,
        )
        .unwrap();
        assert!(!cfg.tools_view_media_introduced);
        cfg.introduce_view_media_tool();
        assert_eq!(cfg.tools_active, ["bash", "view_media"]);
        cfg.tools_active.retain(|k| k != "view_media");
        cfg.introduce_view_media_tool();
        assert_eq!(cfg.tools_active, ["bash"], "выбор пользователя не трогается");
        assert!(AppConfig::default().tools_active.iter().any(|k| k == "view_media"));
    }

    /// `web_read`/`web_search` до слияния в `web`: в активных становятся
    /// `web`, а если `web` уже в пуле — просто уходят (конфиг 13.09.2026).
    #[test]
    fn legacy_web_tool_keys_migrate_to_web() {
        let mut cfg: AppConfig =
            serde_json::from_str(r#"{"tools_active":["bash","web_read","web_search","notes"]}"#).unwrap();
        cfg.migrate_legacy_web_tool_keys();
        assert_eq!(cfg.tools_active, ["bash", "web", "notes"]);

        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"tools_active":["web_read","web_search"],"tools_auto":["bash","web"]}"#,
        )
        .unwrap();
        cfg.migrate_legacy_web_tool_keys();
        assert!(cfg.tools_active.is_empty(), "{:?}", cfg.tools_active);
        assert_eq!(cfg.tools_auto, ["bash", "web"]);

        assert!(!AppConfig::default().tools_active.iter().any(|k| k.starts_with("web_")));
    }

    /// Пул `autotools` появился 13.09.2026: старый конфиг его не содержит и
    /// ведёт себя как раньше — никакой инструмент сам в пул не переезжает.
    #[test]
    fn autotools_pool_defaults_to_empty() {
        let cfg: AppConfig = serde_json::from_str(r#"{"tools_active":["bash","notes"]}"#).unwrap();
        assert!(cfg.tools_auto.is_empty());
        assert_eq!(cfg.tools_active, ["bash", "notes"]);
        assert!(AppConfig::default().tools_auto.is_empty());
        let round: AppConfig =
            serde_json::from_str(r#"{"tools_active":["bash"],"tools_auto":["notes"]}"#).unwrap();
        assert_eq!(round.tools_auto, ["notes"]);
    }

    #[test]
    fn approval_unknown_key_falls_back_to_default() {
        let overrides = HashMap::new();
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ASK, &overrides), TOOL_APPROVAL_ASK);
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ALWAYS, &overrides), TOOL_APPROVAL_ALWAYS);
    }

    #[test]
    fn approval_override_wins_over_default() {
        let mut overrides = HashMap::new();
        overrides.insert("bash".into(), TOOL_APPROVAL_ASK.into());
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ALWAYS, &overrides), TOOL_APPROVAL_ASK);

        overrides.insert("web_read".into(), TOOL_APPROVAL_ALWAYS.into());
        assert_eq!(effective_approval_mode("web_read", TOOL_APPROVAL_ASK, &overrides), TOOL_APPROVAL_ALWAYS);
    }

    #[test]
    fn approval_default_marker_falls_back_to_global() {
        let mut overrides = HashMap::new();
        overrides.insert("bash".into(), TOOL_APPROVAL_DEFAULT.into());
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ALWAYS, &overrides), TOOL_APPROVAL_ALWAYS);
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ASK, &overrides), TOOL_APPROVAL_ASK);
    }

    #[test]
    fn approval_invalid_values_fall_back_to_ask() {
        let mut overrides = HashMap::new();
        overrides.insert("bash".into(), "garbage".into());
        // невалидный override → fallback на global → invalid global → "ask"
        assert_eq!(effective_approval_mode("bash", "nonsense", &overrides), TOOL_APPROVAL_ASK);
        // валидный global переживает невалидный override
        assert_eq!(effective_approval_mode("bash", TOOL_APPROVAL_ALWAYS, &overrides), TOOL_APPROVAL_ALWAYS);
    }

    #[test]
    fn old_app_config_without_terminal_fields_deserializes() {
        // Старый AppConfig без `terminal_font_family` / `terminal_font_size`
        // (эпоха до 2026-04-27, до feature `terminal` в synthos). Должен
        // десериализоваться через #[serde(default)] на структуре +
        // именованные default-функции для отдельных полей. Без этого
        // существующие установки synthos ломаются при первом запуске
        // обновлённой версии — ровно та регрессия, от которой защищает тест.
        let old_json = r#"{
            "theme": "",
            "general": {
                "display_name": "User",
                "language": "ru",
                "notifications": true,
                "sounds": true,
                "quiet_hours": false,
                "auto_translate": false,
                "server_path": "llama-server",
                "server_host": "127.0.0.1",
                "server_port": 8080,
                "system_prompt": "test",
                "tool_display_mode": "full"
            },
            "models": []
        }"#;
        let cfg: AppConfig =
            serde_json::from_str(old_json).expect("deserialize old AppConfig");
        // Пусто = дефолт `monospace` downstream в TerminalConfig.
        assert_eq!(cfg.terminal_font_family, "");
        // Совпадает с дефолтом `syngui::widgets::Terminal::font_size`.
        assert_eq!(cfg.terminal_font_size, 13.0);
    }

    #[test]
    fn old_config_without_new_fields_deserializes() {
        // Старый формат, в котором ещё не было tool_approval_*. Должно
        // сериализоваться через #[serde(default)] на структуре.
        let old_json = r#"{
            "display_name": "User",
            "language": "ru",
            "notifications": true,
            "sounds": true,
            "quiet_hours": false,
            "auto_translate": false,
            "server_path": "llama-server",
            "server_host": "127.0.0.1",
            "server_port": 8080,
            "system_prompt": "test",
            "tool_display_mode": "full"
        }"#;
        let cfg: GeneralConfig = serde_json::from_str(old_json).expect("deserialize old config");
        assert_eq!(cfg.tool_approval_default, TOOL_APPROVAL_ASK);
        assert!(cfg.tool_approval_overrides.is_empty());
    }

    #[test]
    fn old_general_config_without_voice_font_fields_deserializes() {
        // Старый GeneralConfig без `voice_font_family` / `voice_font_size`
        // (эпоха до Voice FAB). Проверяем, что #[serde(default)] на структуре
        // + именованная default-функция для font_size — корректно дают
        // дефолты, а не ломают парсинг.
        let old_json = r#"{
            "display_name": "User",
            "language": "ru",
            "notifications": true,
            "sounds": true,
            "quiet_hours": false,
            "auto_translate": false,
            "server_path": "llama-server",
            "server_host": "127.0.0.1",
            "server_port": 8080,
            "system_prompt": "test",
            "tool_display_mode": "full",
            "audio_input_device": ""
        }"#;
        let cfg: GeneralConfig = serde_json::from_str(old_json).expect("deserialize");
        assert_eq!(cfg.voice_font_family, "");
        assert_eq!(cfg.voice_font_size, 28.0);
    }

    #[test]
    fn old_audio_model_config_without_device_dtype_deserializes() {
        // Старый AudioModelConfig из mmproj/quantized-эпохи без полей
        // device/dtype. Должен парситься: device → "cpu", storage/compute → "f16".
        let old_json = r#"{
            "name": "Whisper",
            "kind": "whisper",
            "model_path": "/m/whisper.syn",
            "mmproj_path": "",
            "language": "ru",
            "quantized": true,
            "server_port": 0
        }"#;
        let cfg: AudioModelConfig =
            serde_json::from_str(old_json).expect("deserialize old AudioModelConfig");
        assert_eq!(cfg.device, "cpu");
        assert_eq!(cfg.storage_dtype, "f16");
        assert_eq!(cfg.compute_dtype, "f16");
        // legacy парсится в структуру, но при save() уйдёт через skip_serializing.
        assert!(cfg.quantized);
    }

    #[test]
    fn audio_model_config_drops_legacy_on_serialize() {
        let cfg = AudioModelConfig {
            quantized: true,
            mmproj_path: "x".into(),
            dtype: "f16".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(!json.contains("mmproj_path"));
        assert!(!json.contains("quantized"));
        assert!(!json.contains("\"dtype\""));
        assert!(json.contains("\"device\""));
        assert!(json.contains("\"storage_dtype\""));
        assert!(json.contains("\"compute_dtype\""));
    }

    #[test]
    fn legacy_sampling_defaults_migrate_to_current() {
        // Конфиг до миграции: флага в файле нет → `false`.
        let mut cfg = AppConfig { sampling_defaults_migrated: false, ..AppConfig::default() };
        cfg.syn_chat_defaults = SamplingParams::legacy_v1();
        cfg.migrate_sampling_defaults();
        assert_eq!(cfg.syn_chat_defaults, SamplingParams::default());
    }

    #[test]
    fn customized_sampling_defaults_survive_migration() {
        let mut cfg = AppConfig { sampling_defaults_migrated: false, ..AppConfig::default() };
        cfg.syn_chat_defaults = SamplingParams { temperature: 0.2, ..SamplingParams::legacy_v1() };
        cfg.migrate_sampling_defaults();
        assert_eq!(cfg.syn_chat_defaults.temperature, 0.2);
        assert_eq!(cfg.syn_chat_defaults.top_k, 40);
    }

    #[test]
    fn old_audio_model_config_with_single_dtype_migrates() {
        // Эпоха одного `dtype` (предыдущий PR). Миграция: storage = compute = dtype.
        let old_json = r#"{
            "name": "GigaAM",
            "kind": "giga_am",
            "model_path": "/m/gigaam.syn",
            "language": "ru",
            "device": "gpu_auto",
            "dtype": "bf16"
        }"#;
        let mut cfg: AudioModelConfig =
            serde_json::from_str(old_json).expect("deserialize single-dtype config");
        cfg.migrate_legacy_dtype();
        assert_eq!(cfg.storage_dtype, "bf16");
        assert_eq!(cfg.compute_dtype, "bf16");
        assert!(cfg.dtype.is_empty());
    }

    #[test]
    fn old_audio_model_config_with_legacy_int8_dtype_migrates() {
        // Legacy "int8" → storage="int8" (quantized), compute="f16" (safe default).
        let old_json = r#"{
            "name": "GigaAM",
            "kind": "giga_am",
            "model_path": "/m/gigaam.syn",
            "dtype": "int8"
        }"#;
        let mut cfg: AudioModelConfig =
            serde_json::from_str(old_json).expect("deserialize");
        cfg.migrate_legacy_dtype();
        assert_eq!(cfg.storage_dtype, "int8");
        assert_eq!(cfg.compute_dtype, "f16");
        assert!(cfg.dtype.is_empty());
    }
}
