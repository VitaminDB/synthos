//! Persistent-конфиг приложения (`~/.config/synthos/config.json`).
//!
//! Один общий файл — тема, раздел «Общие» и пресеты моделей llama.cpp.
//! Любая ошибка чтения/парсинга логируется и падает на `Default` — приложение
//! никогда не паникует из-за поломанного конфига (правило TASK.md).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::syn_chat::params::SamplingParams;

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
    #[serde(default = "default_qwen36_mtp")]
    pub qwen36_mtp: bool,
    /// DFlash — блочная спекуляция Muse Glimmer на драфтере-ассистенте.
    #[serde(default = "default_muse_dflash")]
    pub muse_dflash: bool,
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
    /// Видимость левой/правой панели по страницам — тогглы в общей шапке.
    #[serde(default)]
    pub panels: PanelsConfig,
    /// Разделители в списке плиток нав-рейла: unix-миллисекунды создания
    /// каждого. Плитки и разделители сортируются по времени вместе, так что
    /// разделитель, добавленный через «+», встаёт после последней плитки.
    #[serde(default)]
    pub rail_separators: Vec<u64>,
    /// Системный prompt для Syn-чата. Пусто — в `build_history` не
    /// добавляется. Редактируется в правой панели → Параметры → «Система».
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
    /// Политика квантования модели Syn-чата (Qwen3.6). Конвертируется в
    /// [`synaptix::facade::llm::QuantPolicy`] через [`SynChatQuantConfig::to_policy`].
    /// Редактируется на странице Settings → AI Models.
    #[serde(default)]
    pub syn_chat_quant: SynChatQuantConfig,
    /// Режим CUDA attention для `full_attention` слоёв Qwen3.6:
    /// - `"off"` — reference softmax (baseline, без CUDA flash);
    /// - `"fa2"` — FA-2 + Split-K, БЕЗ WMMA Tensor Cores (скалярный mma);
    /// - `"fa4"` — FA-2 + Split-K + WMMA в auto-режиме (наибыстрейший путь).
    ///
    /// Конвертируется в [`synaptix::facade::llm::FlashAttnMode`] и применяется через
    /// [`synaptix::facade::llm::set_flash_attn_mode`]. Переключается в Settings →
    /// AI Models → Inference (runtime). Default = `"fa4"`.
    #[serde(default = "default_qwen36_attn_mode")]
    pub qwen36_attn_mode: String,
    /// Phase D — CUDA-graph decode для Qwen3.6. Capture'ит один decode step
    /// через `generate_with_graph` / `generate_streaming_with_graph` и
    /// replay'ит его на каждом следующем token'е — ×1.16 на 23K context,
    /// ×1.45 на short. Применяется через
    /// [`synaptix::facade::llm::set_graph_decode_enabled`]. Default = `false` пока что
    /// (новая фича, прогоняем production-soak).
    #[serde(default)]
    pub qwen36_graph_decode: bool,
    /// Phase B-1 — fused `linear_attn` prep kernel (sigmoid + softplus +
    /// 3 × repeat_interleave_cast в один launch). Default = `true`
    /// (bit-exact с старым путём, проверено `forward_raw_linear_attn_smoke`).
    /// Применяется через [`synaptix::facade::llm::set_la_prep_fused_disabled`].
    #[serde(default = "default_true")]
    pub qwen36_la_fused: bool,
    /// Phase B-2 — fused `gated_delta_rule` + `RmsNormGated` kernel. SSM-выход
    /// в shared memory вместо global, RMS-фаза в том же block. Default =
    /// `true`. Применяется через [`synaptix::facade::llm::set_gdr_fused_disabled`].
    #[serde(default = "default_true")]
    pub qwen36_gdr_fused: bool,
    /// Префикс-KV: держать посчитанный контекст диалога между ходами, чтобы
    /// ход дописывал в KV только новый хвост промпта, а не считал историю
    /// заново. Стоит VRAM (кэш живёт между ходами, ёмкость кратна 16384
    /// токенам) и выключается, когда в сообщении есть вложения — медиа-путь
    /// префилла идёт по эмбеддингам, а vision-башне нужна та же память.
    /// Default = `true`.
    #[serde(default = "default_true")]
    pub syn_chat_prefix_kv: bool,
    /// Размер chunk'а для prefill. Default = 256 (лимит пика VRAM активаций на
    /// 24 GB GPU при ~1.5k ток/с). Движок принудительно округляет к кратному
    /// 64 — границы чанков на некратных позициях ломают состояние GDN-скана.
    /// Применяется через [`synaptix::facade::llm::set_prefill_chunk_size`].
    #[serde(default = "default_qwen36_prefill_chunk")]
    pub qwen36_prefill_chunk: usize,
    /// Layer-sync режим (`"auto"` / `"on"` / `"off"`). Управляет
    /// `cudaStreamSynchronize` после каждого decoder-слоя в forward'е Qwen3.6.
    /// На prefill chunk=1024 без sync pool growth от intermediate тензоров
    /// = +2-6 GB peak VRAM. Auto = sync только когда `T > 1` (prefill chunks)
    /// — даёт всю экономию памяти без потери decode speed. On = всегда sync
    /// (-5% decode, +4 GB free). Off = никогда (max speed, риск OOM на long
    /// prompts). Конвертируется в [`synaptix::facade::llm::LayerSyncMode`] через
    /// `FromStr`. Default = `"auto"`.
    #[serde(default = "default_qwen36_layer_sync")]
    pub qwen36_layer_sync: String,
    /// Phase E.2 — native FP4 mma GEMV kernel (`nvfp4_mma_gemv_f16`) на
    #[serde(default)]
    pub qwen36_nvfp4_mma: bool,
    #[serde(default)]
    pub qwen36_nvfp4_gemv: bool,
    /// ACE-Step «общий» bundle с DiT + проектором + lyric_encoder +
    /// timbre_encoder + null_condition_emb + FSQ + Detokenizer (один из
    /// `acestep_v15_xl_base.syn` / `acestep_v15_xl_turbo.syn`). Используется
    /// всеми ACE-Step нодами семейства, кроме Pack. Редактируется на странице
    /// Settings → AI Models → ACE-Step.
    #[serde(default)]
    pub acestep_xl_bundle_path: Option<String>,
    /// ACE-Step VAE bundle (`acestep_vae.syn`). Используется нодами
    /// VaeEncode / VaeDecode.
    #[serde(default)]
    pub acestep_vae_bundle_path: Option<String>,
}

fn default_true() -> bool { true }
// 256: единственный размер, безопасный на 24 ГБ (single-shot/1024 упирается в
// VRAM поверх ~18 ГБ весов 27B) при prefill ~1.5-1.6k ток/с; движок сам
// округляет к кратному 64 (границы GDN-скана).
fn default_qwen36_prefill_chunk() -> usize { 256 }

fn default_qwen36_attn_mode() -> String { "fa4".into() }

fn default_qwen36_layer_sync() -> String { "auto".into() }

// ─────────────────────────────────────────────────────────────────────────────
// SynChatQuantConfig — политика квантования Qwen3.6 для Syn-чата
// ─────────────────────────────────────────────────────────────────────────────

/// Политика квантования модели Syn-чата. Хранит каждый dtype как строку
/// (`"f16"`, `"bf16"`, `"nvfp4"`, `"fp8e4m3"`, ...) — стабильное представление
/// для config.json, не зависящее от Rust-enum. Конвертация в типизированную
/// [`synaptix::facade::llm::QuantPolicy`] — через [`Self::to_policy`].
///
/// `preset` — имя пресета (`"quality"` / `"balance"` / `"vram_saver"` /
/// `"custom"`). При ручном изменении любого dtype-поля UI переключает на
/// `"custom"`; пресеты заполняют все поля из [`synaptix::facade::llm::QuantPolicy`]
/// helper-методов.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynChatQuantConfig {
    pub preset: String,
    /// Формат хранения весов attn/MLP linear-слоёв.
    /// Допустимо: `"f32"`, `"bf16"`, `"f16"`, `"q8_0"`, `"q4_0"`, `"fp8e4m3"`, `"nvfp4"`.
    pub weights_storage: String,
    /// Dtype активаций между слоями. Допустимо: `"f32"`, `"bf16"`, `"f16"`.
    pub compute: String,
    /// KV-cache dtype. Допустимо: `"auto"`, `"f16"`, `"bf16"`, `"f32"`, `"fp8e4m3"`.
    pub kv_dtype: String,
    /// Формат хранения `lm_head` (выходная проекция в vocab).
    /// `"nvfp4"` экономит ~1.9 GB VRAM, но требует chunked-launch kernel
    /// (vocab>32768) — реализация в Фазе 3. FP8 E4M3 — native cuBLASLt FP8
    /// GEMM, экономит ~1.2 GB при vocab=248K и accuracy ≈ F16.
    pub lm_head_storage: String,
    /// Формат хранения `embed_tokens`. Допустимо: `"f32"`/`"bf16"`/`"f16"`/
    /// `"fp8e4m3"`/`"nvfp4"`. FP8 E4M3 — packed bytes + per-tensor scale +
    /// custom gather kernel, экономит ~1.2 GB и accuracy ≈ F16. NVFP4 (Phase F)
    /// экономит ~1.8 GB через FP4 nibbles + tile-major scales, но заметная
    /// просадка качества — только для VRAM-Saver. Load-time setting,
    /// требует перезагрузки модели.
    pub embed_storage: String,
    /// Tied embeddings (`lm_head` шарит веса с `embed_tokens`). Допустимо:
    /// `"auto"` (читает `tie_word_embeddings` из JSON), `"on"` (force tied —
    /// меняет числовой выход на untied checkpoint), `"off"` (force separate).
    pub tied_embeddings: String,
    /// Dtype SSM-recurrence state в Gated DeltaNet (linear-attention слои).
    /// `"f32"` — default (соответствует HF `mamba_ssm_dtype`).
    pub ssm_state_dtype: String,
    /// Dtype conv1d-state в Gated DeltaNet.
    pub conv_state_dtype: String,
}

impl Default for SynChatQuantConfig {
    fn default() -> Self {
        // По умолчанию — preset Balance: NVFP4 backbone + FP8 E4M3 lm_head/embed
        // (≈ F16 accuracy, ~2.4 GB экономии = 65K контекста на 24 GB GPU).
        // NVFP4 lm_head/embed доступен через preset `vram_saver` (~1.8 GB
        // дополнительной экономии ценой заметной просадки качества).
        Self {
            preset: "balance".into(),
            weights_storage: "nvfp4".into(),
            compute: "f16".into(),
            kv_dtype: "f16".into(),
            lm_head_storage: "fp8e4m3".into(),
            embed_storage: "fp8e4m3".into(),
            tied_embeddings: "auto".into(),
            ssm_state_dtype: "f32".into(),
            conv_state_dtype: "f16".into(),
        }
    }
}

impl SynChatQuantConfig {
    /// Заполнить из встроенного пресета. Известные имена: `"quality"`,
    /// `"balance"`, `"vram_saver"`. Неизвестные имена — возвращает Custom-копию
    /// (preset_name выставляется как passed).
    pub fn from_preset(name: &str) -> Self {
        use synaptix::facade::llm::QuantPolicy;
        let policy = match name {
            "quality" => QuantPolicy::quality(),
            "balance" => QuantPolicy::balance(),
            "vram_saver" => QuantPolicy::vram_saver(),
            _ => QuantPolicy::balance(),
        };
        let mut cfg = Self::from_policy(&policy);
        if name != cfg.preset {
            cfg.preset = name.into();
        }
        cfg
    }

    /// Сконвертировать в типизированную `Qwen36QuantPolicy`. Неизвестные
    /// строковые значения dtype → fallback на default из преcета `balance`.
    pub fn to_policy(&self) -> synaptix::facade::llm::QuantPolicy {
        use synaptix::facade::llm::{KvDtypePolicy, QuantPolicy, TiedEmbeddingsMode};

        let fallback = QuantPolicy::balance();
        QuantPolicy {
            weights_storage: parse_storage_dtype(&self.weights_storage, fallback.weights_storage),
            compute: parse_compute_dtype(&self.compute, fallback.compute),
            kv_dtype: KvDtypePolicy::from_name(&self.kv_dtype).unwrap_or(fallback.kv_dtype),
            lm_head_storage: parse_storage_dtype(&self.lm_head_storage, fallback.lm_head_storage),
            embed_storage: parse_storage_dtype(&self.embed_storage, fallback.embed_storage),
            tied_embeddings: TiedEmbeddingsMode::from_name(&self.tied_embeddings)
                .unwrap_or(fallback.tied_embeddings),
            ssm_state_dtype: KvDtypePolicy::from_name(&self.ssm_state_dtype)
                .unwrap_or(fallback.ssm_state_dtype),
            conv_state_dtype: KvDtypePolicy::from_name(&self.conv_state_dtype)
                .unwrap_or(fallback.conv_state_dtype),
            preset_name: self.preset.clone(),
        }
    }

    /// Обратная конвертация типизированной policy → строковая конфигурация
    /// для UI/persist.
    pub fn from_policy(policy: &synaptix::facade::llm::QuantPolicy) -> Self {
        Self {
            preset: policy.preset_name.clone(),
            weights_storage: dtype_name(policy.weights_storage).into(),
            compute: dtype_name(policy.compute).into(),
            kv_dtype: policy.kv_dtype.name().into(),
            lm_head_storage: dtype_name(policy.lm_head_storage).into(),
            embed_storage: dtype_name(policy.embed_storage).into(),
            tied_embeddings: policy.tied_embeddings.name().into(),
            ssm_state_dtype: policy.ssm_state_dtype.name().into(),
            conv_state_dtype: policy.conv_state_dtype.name().into(),
        }
    }
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
fn dtype_name(dt: synaptix_core::dtype::DType) -> &'static str {
    use synaptix_core::dtype::DType;
    match dt {
        DType::F32 => "f32",
        DType::F16 => "f16",
        DType::BF16 => "bf16",
        DType::NVFP4 => "nvfp4",
        DType::MXFP8 => "mxfp8",
        _ => "f16",
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
    /// Каталог с моделью эмбеддера. Дефолт: `~/models/bge-m3`.
    /// При смене требует перезагрузки эмбеддера через UI.
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
    /// Каталог с моделью cross-encoder реранкера. Пусто = реранкер
    /// отключён, `hybrid_search` работает без post-rerank phase'ы.
    /// Конвенция: `~/models/bge-reranker-v2-m3`.
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
            embedder_model_path: default_kb_embedder_path(),
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
            reranker_model_path: default_kb_reranker_path(),
            reranker_device: "cpu".to_string(),
            reranker_dtype: "f32".to_string(),
            reranker_enabled: true,
            reranker_top_k_multiplier: 4,
            reranker_max_tokens: 512,
        }
    }
}

/// `~/models/bge-reranker-v2-m3.syn` — single-file model bundle (см. крейт
/// `syn-format`); реранкер ~568 MB, multilingual. Пакуется один раз через
/// `syn-pack ~/models/bge-reranker-v2-m3 -o ~/models/bge-reranker-v2-m3.syn`.
pub fn default_kb_reranker_path() -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("models/bge-reranker-v2-m3.syn")
        .display()
        .to_string()
}

/// `~/models/bge-m3.syn` — конвенция single-file model bundle'а (см. крейт
/// `syn-format`). Старый layout `~/models/bge-m3/` с россыпью файлов больше
/// не поддерживается embedding-bge — его нужно один раз перепаковать через
/// `syn-pack ~/models/bge-m3 -o ~/models/bge-m3.syn`.
pub fn default_kb_embedder_path() -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("models/bge-m3.syn")
        .display()
        .to_string()
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
        "web_read".to_string(),
        "system".to_string(),
        "pipelines".to_string(),
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
            skills_active: Vec::new(),
            audio_models: Vec::new(),
            selected_audio_model: None,
            audio_autostart: false,
            code_sessions: Vec::new(),
            active_code_session: None,
            last_code_folder: None,
            terminal_font_family: String::new(),
            terminal_font_size: default_terminal_font_size(),
            code_editor_font_family: String::new(),
            code_editor_font_size: default_code_editor_font_size(),
            kb: KbConfig::default(),
            syn_explorer_bookmarks: Vec::new(),
            syn_explorer_left_split_ratio: default_syn_explorer_left_split_ratio(),
            syn_explorer_right_split_ratio: default_syn_explorer_right_split_ratio(),
            hf_cache_dir: String::new(),
            hf_concurrent_downloads: default_hf_concurrent_downloads(),
            hf_segments_per_file: default_hf_segments_per_file(),
            hf_speed_limit_mbps: default_hf_speed_limit_mbps(),
            hf_skip_unwanted_formats: default_hf_skip_unwanted_formats(),
            hf_gguf_support: default_hf_gguf_support(),
            qwen36_mtp: default_qwen36_mtp(),
            muse_dflash: default_muse_dflash(),
            hf_token: String::new(),
            syn_chat_defaults: SamplingParams::default(),
            last_syn_model: None,
            search_recent: Vec::new(),
            syn_chat_left_split_ratio: default_syn_chat_left_split_ratio(),
            syn_chat_right_split_ratio: default_syn_chat_right_split_ratio(),
            hf_left_split_ratio: default_hf_left_split_ratio(),
            hf_right_split_ratio: default_hf_right_split_ratio(),
            settings_left_split_ratio: default_settings_left_split_ratio(),
            settings_right_split_ratio: default_settings_right_split_ratio(),
            panels: PanelsConfig::default(),
            rail_separators: Vec::new(),
            syn_chat_system_prompt: String::new(),
            syn_chat_max_image_tokens: default_syn_chat_max_image_tokens(),
            syn_chat_quant: SynChatQuantConfig::default(),
            qwen36_attn_mode: default_qwen36_attn_mode(),
            qwen36_graph_decode: false,
            qwen36_la_fused: true,
            qwen36_gdr_fused: true,
            syn_chat_prefix_kv: true,
            qwen36_prefill_chunk: default_qwen36_prefill_chunk(),
            qwen36_layer_sync: default_qwen36_layer_sync(),
            qwen36_nvfp4_mma: false,
            qwen36_nvfp4_gemv: false,
            acestep_xl_bundle_path: None,
            acestep_vae_bundle_path: None,
            models_dir: default_models_dir(),
        }
    }
}

fn default_hf_concurrent_downloads() -> u32 { 3 }
fn default_hf_segments_per_file() -> u32 { 4 }
fn default_hf_speed_limit_mbps() -> u32 { 0 }
fn default_hf_skip_unwanted_formats() -> bool { true }
fn default_hf_gguf_support() -> bool { false }
fn default_qwen36_mtp() -> bool { true }
fn default_muse_dflash() -> bool { true }

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
            syn_explorer_left: true,
            syn_explorer_right: true,
            huggingface_left: true,
            huggingface_right: true,
            settings_left: true,
            settings_right: true,
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
        return PathBuf::from(stored);
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
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut cfg) => {
                    // Миграция со старого однополевого dtype в storage/compute.
                    for m in &mut cfg.audio_models {
                        m.migrate_legacy_dtype();
                    }
                    cfg
                }
                Err(e) => {
                    eprintln!(
                        "[synthos] Не удалось распарсить {:?}: {e}. \
                         Использую настройки по умолчанию (файл не перезаписан).",
                        path
                    );
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
                if let Err(e) = std::fs::write(&path, content) {
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
