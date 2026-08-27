//! Глобальный контекст приложения.
//!
//! Содержит реактивные сигналы и роутеры, пробрасываемые через
//! `provide_context(AppCtx { .. })` в `run_desktop`. Любой виджет может
//! достать его через `use_context::<AppCtx>()`.

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::appearance::SystemAppearance;
use syngui::prelude::*;
use syngui::window::{BackdropConfig, WindowState};
use syngui::widgets::navigation::router::Router;

use crate::agent::audio::AudioCtx;
use crate::agent::tools::PendingApproval;
use crate::config::{AudioModelConfig, SynChatQuantConfig};
use crate::metrics::MetricsState;

pub const ROUTES: &[&str] = &[
    "syn_chat",
    "music",
    "code",
    "nodes",
    "syn_explorer",
    "huggingface",
    "settings",
];

pub const SETTINGS_ROUTES: &[&str] = &[
    "general",
    "themes",
    "skills",
    "audio_models",
    "ai_models",
    "knowledge_base",
    "archive",
    "about",
];

pub const INITIAL_ROUTE: &str = "syn_chat";
pub const INITIAL_SETTINGS_ROUTE: &str = "general";

/// Сигналы раздела «Общие». Все поля — реактивные, сериализуются в
/// `~/.config/synthos/config.json` через общий effect в `lib.rs`.
#[derive(Clone, Copy)]
pub struct GeneralCtx {
    pub display_name: RwSignal<String>,
    pub language: RwSignal<String>,
    /// Системный промпт для всех запросов к модели (`role=system`).
    /// Редактируется в Settings → Общие; переживает рестарт через
    /// `AppConfig.general.system_prompt`.
    pub system_prompt: RwSignal<String>,
    /// Системный промпт постобработки распознанной речи. Применяется в
    /// FAB-окне распознавания при ручном клике по кнопке «обновить» над
    /// полем «Отредактированный текст». Editing UI — Settings → Общие.
    /// Persist: `AppConfig.general.voice_refine_prompt`.
    pub voice_refine_prompt: RwSignal<String>,
    /// Режим отображения tool-вызовов в ленте чата:
    /// `"full"` | `"minimal"` | `"hidden"`. См. описание в
    /// `config::GeneralConfig.tool_display_mode`.
    pub tool_display_mode: RwSignal<String>,
    /// Глобальный режим подтверждения tool-call'ов:
    /// `"ask"` | `"always_allow"`. См. `config::GeneralConfig.tool_approval_default`.
    /// Проверяется в `chat::session::await_decision_on_tool_call`.
    pub tool_approval_default: RwSignal<String>,
    /// Per-tool override глобального режима. Ключ — `Tool::key`. См.
    /// `config::GeneralConfig.tool_approval_overrides`.
    pub tool_approval_overrides: RwSignal<std::collections::HashMap<String, String>>,
    /// Выбранное входное аудио-устройство для микрофонной записи
    /// (`Device::name()`). Пусто = автовыбор.
    pub audio_input_device: RwSignal<String>,
    /// Семейство шрифта для окна голосового распознавания (FAB → центр).
    /// Пусто = `"sans-serif"`. Применяется через MSS-переменную
    /// `--voice-font-family` в `.voice-overlay-text`. Меняется в реальном
    /// времени через effect, который перегенерирует `theme_mss`.
    pub voice_font_family: RwSignal<String>,
    /// Размер шрифта окна голосового распознавания в логических пикселях.
    /// Применяется через MSS-переменную `--voice-font-size`.
    pub voice_font_size: RwSignal<f32>,
    /// Максимум tool-turn'ов в одном цикле основного агента
    /// (`chat::session::run_agent`). Persist:
    /// `AppConfig.general.agent_max_turns`. Снимается snapshot'ом в
    /// `start_agent_turn` перед запуском async-таски.
    pub agent_max_turns: RwSignal<u32>,
    /// Максимум tool-turn'ов внутри одного субагента
    /// (`chat::tools::subagent::run_subagent_loop`). Persist:
    /// `AppConfig.general.subagent_max_turns`. Снимается snapshot'ом в
    /// `subagent::snapshot_from_main`.
    pub subagent_max_turns: RwSignal<u32>,
    /// Включён ли autocompact (см. `chat::compact::run_compaction`). Если
    /// `false` — авто-триггер не срабатывает, доступна только ручная
    /// кнопка «Compact now». Persist: `AppConfig.general.autocompact_enabled`.
    pub autocompact_enabled: RwSignal<bool>,
    /// Порог автоматического autocompact в процентах от `n_ctx`. При
    /// `prompt_tokens / n_ctx > threshold/100` после очередного turn'а
    /// запускается компактификация. Дефолт 85, валидный диапазон 50..=95.
    /// Persist: `AppConfig.general.autocompact_threshold_percent`.
    pub autocompact_threshold_percent: RwSignal<u32>,
}

/// Куда вставлять распознанный голосовой текст по кнопке «Вставить» в FAB-панели.
///
/// Сигнал `VoiceFabCtx.focus_target` обновляется в виджетах при получении
/// фокуса (TextField чата → `ChatInput`, terminal-pane → `Terminal`).
/// Используется в `voice_fab::actions::paste_to_target`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusTarget {
    /// Поле ввода чата (`AppCtx.chat.input`).
    ChatInput,
    /// PTY-сессия терминала на странице `code`.
    Terminal,
    /// Никто не сфокусирован — fallback на `ChatInput` при вставке.
    None,
}

/// UI-состояние глобального голосового FAB и панели распознавания.
///
/// Все запись/транскрипция остаётся в [`AudioCtx`]; здесь только видимость
/// панели, накопленный текст всех чанков (Pause→Resume склеивает) и
/// `awaiting_actions=true` после финального Stop.
#[derive(Clone, Copy)]
pub struct VoiceFabCtx {
    /// Открыта ли overlay-панель распознавания.
    pub panel_open: RwSignal<bool>,
    /// Выставляется в `true` после финального Stop, когда транскрипция
    /// завершилась. Меняет actions_row с Pause/Stop на Copy/Paste/Restart/Close.
    pub awaiting_actions: RwSignal<bool>,
    /// Накопленный текст всех чанков сессии (Pause-Resume склеиваются через
    /// пробел). Очищается при Restart и Close.
    pub accumulated: RwSignal<String>,
    /// Текст последнего завершённого чанка — для отдельного отображения
    /// и для save_recording в Sprint 2 (при final_chunk).
    pub last_transcript: RwSignal<String>,
    /// Активный фокус — куда уйдёт Paste.
    pub focus_target: RwSignal<FocusTarget>,
    /// «Запись запросили, но ASR-модель ещё грузится». Effect в `lib.rs` ловит
    /// успешное завершение загрузки и автоматически дёргает `start_recording`.
    /// Сбрасывается также при закрытии панели и при ошибке загрузки.
    pub pending_record_start: RwSignal<bool>,
    /// Текст распознанной речи после прогона через LLM (постобработка).
    /// Заполняется только по ручному клику пользователя по кнопке «обновить»
    /// в окне FAB. При ошибке вызова — копия `accumulated` (fallback).
    pub refined: RwSignal<String>,
    /// Идёт ли LLM-вызов постобработки (между нажатием «обновить» и приходом
    /// ответа от нативной модели). Используется UI для блокировки кнопки и
    /// показа индикатора над полем «Отредактированный текст».
    pub refining: RwSignal<bool>,
    /// Последняя ошибка постобработки. При
    /// `Some(_)` под полем «Отредактированный текст» показывается красная
    /// строка с сообщением.
    pub refine_error: RwSignal<Option<String>>,
    /// Поколение поля «Исходная речь». Подписан Reactive-обёртка поля; bump
    /// (через `update(|n| *n += 1)`) даёт пересоздание `MultilineTextEdit`
    /// с актуальным `accumulated` (нужно при внешнем обновлении после
    /// транскрипции). Пользовательский ввод в поле меняет только
    /// `accumulated`, без бампа — фокус и курсор не теряются.
    pub raw_gen: RwSignal<u64>,
    /// Поколение поля «Отредактированный текст» — аналогично `raw_gen`,
    /// бампается при приходе ответа LLM или при сбросе сессии.
    pub refined_gen: RwSignal<u64>,
}

impl VoiceFabCtx {
    pub fn new() -> Self {
        Self {
            panel_open: use_signal(false),
            awaiting_actions: use_signal(false),
            accumulated: use_signal(String::new()),
            last_transcript: use_signal(String::new()),
            focus_target: use_signal(FocusTarget::None),
            pending_record_start: use_signal(false),
            refined: use_signal(String::new()),
            refining: use_signal(false),
            refine_error: use_signal(None),
            raw_gen: use_signal(0),
            refined_gen: use_signal(0),
        }
    }
}

impl Default for VoiceFabCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Состояние переключателя табов в правой панели страницы syn_chat.
/// 0 — «Параметры» (модель + sampling), 1 — «Детали» (метрики).
/// Инструменты и скилы живут в левой панели (`pages::syn_chat::left_panel`).
pub const SYN_RIGHT_PANEL_PARAMS: usize = 0;
pub const SYN_RIGHT_PANEL_DETAILS: usize = 1;

/// Видимость боковых панелей трёхпанельного каркаса
/// (`components::workspace_frame`) по страницам. Пара `(левая, правая)`.
/// Тогглы в общей шапке пишут сюда; `install_config_autosave` переливает
/// в `AppConfig.panels`.
#[derive(Clone, Copy)]
pub struct PanelsCtx {
    pub syn_chat: (RwSignal<bool>, RwSignal<bool>),
    pub code: (RwSignal<bool>, RwSignal<bool>),
    pub syn_explorer: (RwSignal<bool>, RwSignal<bool>),
    pub huggingface: (RwSignal<bool>, RwSignal<bool>),
    pub settings: (RwSignal<bool>, RwSignal<bool>),
}

impl PanelsCtx {
    pub fn from_config(cfg: &crate::config::PanelsConfig) -> Self {
        Self {
            syn_chat: (use_signal(cfg.syn_chat_left), use_signal(cfg.syn_chat_right)),
            code: (use_signal(cfg.code_left), use_signal(cfg.code_right)),
            syn_explorer: (
                use_signal(cfg.syn_explorer_left),
                use_signal(cfg.syn_explorer_right),
            ),
            huggingface: (
                use_signal(cfg.huggingface_left),
                use_signal(cfg.huggingface_right),
            ),
            settings: (use_signal(cfg.settings_left), use_signal(cfg.settings_right)),
        }
    }

    /// Снимок для автосейва (`.get()` — подписка эффекта на все флаги).
    pub fn to_config(&self) -> crate::config::PanelsConfig {
        crate::config::PanelsConfig {
            syn_chat_left: self.syn_chat.0.get(),
            syn_chat_right: self.syn_chat.1.get(),
            code_left: self.code.0.get(),
            code_right: self.code.1.get(),
            syn_explorer_left: self.syn_explorer.0.get(),
            syn_explorer_right: self.syn_explorer.1.get(),
            huggingface_left: self.huggingface.0.get(),
            huggingface_right: self.huggingface.1.get(),
            settings_left: self.settings.0.get(),
            settings_right: self.settings.1.get(),
        }
    }
}

/// Реактивное состояние подсистемы инструментов (tools).
///
/// - `active` — активные ключи инструментов, уходят в каждый запрос
///   `ChatRequest.tools`. Меняется кликом по чипу в правой панели;
///   сериализуется в `AppConfig.tools_active`.
/// - `allow_all` — флаг «пропускать диалог подтверждения». Per-chat:
///   сбрасывается при переключении/создании чата, НЕ persist’ится.
/// - `pending_approval` — если `Some`, на экране висит Portal-диалог
///   с кнопками «Отмена / Разрешить / Разрешить все». Оркестратор
///   в `chat::session` блокирован в `await` на одноразовом канале,
///   спрятанном в `PendingApproval`.
#[derive(Clone)]
pub struct ToolsCtx {
    pub active: RwSignal<Vec<String>>,
    pub allow_all: RwSignal<bool>,
    pub pending_approval: RwSignal<Option<Arc<PendingApproval>>>,
}

impl ToolsCtx {
    pub fn new(active: Vec<String>) -> Self {
        Self {
            active: use_signal(active),
            allow_all: use_signal(false),
            pending_approval: use_signal(None),
        }
    }
}

/// Тип CRUD-диалога для скилов (см. `pages::settings::skills_dialog`).
/// Один Portal в `pages::settings::view()` слушает `AppCtx.skills_dialog`
/// и подменяет тело по варианту.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillDialogKind {
    /// Создать новый скил (TextField для имени и описания).
    Create,
    /// Редактировать имя и описание существующего скила.
    Edit {
        id: String,
        current_name: String,
        current_description: String,
    },
    /// Подтвердить удаление — без TextField.
    Delete { id: String, name: String },
}

/// Всё, что связано с оформлением окна: системная тема, акцент рабочего стола,
/// вид кнопок титлбара и «стекло» за окном.
///
/// Сигнал `system` наполняет сам фреймворк
/// (`AppBuilder::with_system_appearance`), остальные поля — пользовательские
/// настройки из раздела «Темы».
#[derive(Clone, Copy)]
pub struct AppearanceCtx {
    /// Текущее системное оформление: светлая/тёмная схема, акцент DE.
    pub system: RwSignal<SystemAppearance>,
    /// Следовать системной светлой/тёмной схеме.
    pub follow_system: RwSignal<bool>,
    /// Ключ темы для системной светлой схемы.
    pub theme_light: RwSignal<String>,
    /// Ключ темы для системной тёмной схемы.
    pub theme_dark: RwSignal<String>,
    /// Подмешивать системный акцент поверх палитры темы.
    pub use_system_accent: RwSignal<bool>,
    /// Рисовать кнопки окна темой декораций рабочего стола.
    pub system_window_controls: RwSignal<bool>,
    /// Просить композитор размывать фон за окном.
    pub window_blur: RwSignal<bool>,
    /// Непрозрачность фоновых поверхностей (1.0 — сплошной фон).
    pub window_opacity: RwSignal<f32>,
    /// То, что уходит в `AppBuilder::with_backdrop` — пересобирается из
    /// `window_blur` и состояния окна.
    pub backdrop: RwSignal<BackdropConfig>,
    /// Развёрнутость/фокус окна; наполняет фреймворк. От неё зависят форма
    /// области размытия и вид кнопок титлбара.
    pub window_state: RwSignal<WindowState>,
}

#[derive(Clone)]
pub struct AppCtx {
    /// Ключ темы — стабильный id для persistence (имя темы из `theme_data`).
    pub theme_key: RwSignal<String>,
    /// Подпись темы — MSS-блок `:root { --var: ... }`.
    /// Обновляется синхронно с `theme_key`; нужен для `.with_dynamic_theme`.
    pub theme_mss: RwSignal<String>,
    /// Системное оформление и настройки окна (раздел «Темы»).
    pub appearance: AppearanceCtx,
    /// Роутер верхнего уровня (между основными страницами).
    pub router: Arc<Mutex<Router>>,
    /// Текущий ключ маршрута верхнего уровня — дублирует router.current(),
    /// но в виде сигнала, к которому можно реактивно подписываться.
    pub current_route: RwSignal<String>,
    /// Вложенный роутер внутри страницы настроек.
    pub settings_router: Arc<Mutex<Router>>,
    /// Текущий подмаршрут настроек.
    pub selected_settings_tab: RwSignal<String>,
    /// Полный список скилов из `~/.config/synthos/skills/*.md`.
    /// Грузится один раз при старте через `skills::load_all()` и
    /// перезаписывается после CRUD-операций.
    pub skills: RwSignal<Vec<crate::skills::Skill>>,
    /// Slug-id выделенного скила в редакторе настроек.
    pub skills_selected_id: RwSignal<Option<String>>,
    /// Slug-id скилов, активных в правой панели (чипы рядом с tools).
    /// Persist'ится в `AppConfig.skills_active`.
    pub skills_active: RwSignal<Vec<String>>,
    /// Открытый CRUD-диалог (создание/переименование/удаление). `None` —
    /// диалог закрыт.
    pub skills_dialog: RwSignal<Option<SkillDialogKind>>,
    /// Открытый URL prompt в Settings → Knowledge Base. `Some(collection_id)`
    /// — диалог открыт и сохранит URL в указанную коллекцию. `None` — закрыт.
    pub kb_url_dialog: RwSignal<Option<String>>,
    /// Настройки раздела «Общие».
    pub general: GeneralCtx,
    /// Пресеты ASR-моделей. Каждый — конфиг для локального
    /// [`synaptix::facade::asr::Transcriber`] (Whisper / GigaAM / …).
    pub audio_models: RwSignal<Vec<AudioModelConfig>>,
    /// Имя выбранной ASR-модели (стабильный id).
    pub selected_audio_model: RwSignal<Option<String>>,
    /// Подсистема записи с микрофона + транскрипции (toggle на input panel).
    pub audio: AudioCtx,
    /// Глобальный голосовой FAB + панель распознавания (доступны на всех страницах).
    pub voice: VoiceFabCtx,
    /// Системные метрики (CPU/RAM/GPU/VRAM). Фоновый поток опрашивает
    /// sysinfo/NVML раз в секунду для вкладки «Детали».
    pub metrics: Arc<MetricsState>,
    /// Подсистема агентских инструментов (bash/web_read + диалог
    /// подтверждения + активный список).
    pub tools: ToolsCtx,
    /// Семейство шрифта VTE-терминала (страница «code» → нижняя панель).
    /// Пусто = `"monospace"` (резолвится font-kit'ом). Биндится к
    /// панели настроек в Settings → Терминал и к gear-popover самого
    /// терминала; persist'ится в `AppConfig.terminal_font_family`.
    pub terminal_font_family: RwSignal<String>,
    /// Размер шрифта VTE-терминала в логических пикселях.
    /// Persist'ится в `AppConfig.terminal_font_size`.
    pub terminal_font_size: RwSignal<f32>,
    /// Семейство шрифта CodeEditor (страница «code» → центральная панель).
    /// Пусто = `"monospace"` (системный default). Биндится к панели Settings →
    /// Общие; persist'ится в `AppConfig.code_editor_font_family`. Применяется
    /// в реальном времени через MSS-переменную `--code-editor-font-family`.
    pub code_editor_font_family: RwSignal<String>,
    /// Размер шрифта CodeEditor в логических пикселях. Persist'ится в
    /// `AppConfig.code_editor_font_size`. Применяется через MSS-переменную
    /// `--code-editor-font-size`.
    pub code_editor_font_size: RwSignal<f32>,
    /// Knowledge base / RAG. Реестр коллекций, активные коллекции для
    /// текущего чата, lazy-загруженный embedder, прогресс ingestion'а.
    /// См. `crate::kb::KbCtx`.
    pub kb: crate::kb::KbCtx,
    /// Глобальный канал для notification-ов в правом верхнем углу.
    /// Default duration = 15s (per TASK.md). Используется как:
    /// `ctx.notifications.info("...")`, `.success(...)`, `.warning(...)`, `.error(...)`.
    pub notifications: syngui::widgets::feedback::NotificationCtx,
    /// Политика квантования Qwen3.6 для Syn-чата (preset + per-component
    /// dtypes). Сохраняется в `AppConfig.syn_chat_quant`. Применяется при
    /// `SynModelRegistry::load(path)`: `cfg.to_policy()` → `load_qwen36`.
    /// Изменения требуют ручного reload модели через Settings → AI Models.
    pub syn_chat_quant: RwSignal<SynChatQuantConfig>,
    /// Потолок vision-токенов на одну картинку-вложение в чате.
    /// `0` — снять ограничение и довериться конфигу модели (у Muse Glimmer
    /// это 4096 токенов на картинку). Сохраняется в
    /// `AppConfig.syn_chat_max_image_tokens`, применяется к следующей
    /// отправке — перезагрузка модели не нужна.
    pub syn_chat_max_image_tokens: RwSignal<usize>,
    /// Режим CUDA attention для `full_attention` слоёв Qwen3.6:
    /// - `"off"` — reference softmax (без CUDA flash);
    /// - `"fa2"` — FA-2 + Split-K без WMMA Tensor Cores;
    /// - `"fa4"` — FA-2 + Split-K + WMMA (auto, default).
    ///
    /// Сохраняется в `AppConfig.qwen36_attn_mode`. Изменения применяются
    /// без reload модели: эффект-handler в `lib.rs` маппит строку в
    /// [`synaptix::facade::llm::FlashAttnMode`] и вызывает
    /// [`synaptix::facade::llm::set_flash_attn_mode`].
    pub qwen36_attn_mode: RwSignal<String>,
    /// Phase D CUDA-graph decode для Qwen3.6. Когда ON — `generate(...)` и
    /// `generate_streaming(...)` идут через `generate_with_graph` /
    /// `generate_streaming_with_graph`: capture одного decode step + replay.
    /// На short context ×1.45, на 23K ×1.16. Требует device, созданный через
    /// `CudaDevice::new_with_stream` (см. `syn_chat::model_registry`).
    ///
    /// Сохраняется в `AppConfig.qwen36_graph_decode`. Изменения применяются
    /// без reload модели через [`synaptix::facade::llm::set_graph_decode_enabled`]
    /// в effect-handler'е `lib.rs`.
    pub qwen36_graph_decode: RwSignal<bool>,
    pub qwen36_mtp: RwSignal<bool>,
    pub muse_dflash: RwSignal<bool>,
    /// Phase B-1 fused `linear_attn` prep kernel (5 → 1 launch).
    /// Bit-exact; default = `true`. Сохраняется в `AppConfig.qwen36_la_fused`,
    /// применяется через [`synaptix::facade::llm::set_la_prep_fused_disabled`].
    pub qwen36_la_fused: RwSignal<bool>,
    /// Phase B-2 fused `gated_delta_rule` + `RmsNormGated` kernel.
    /// Bit-exact; default = `true`. Сохраняется в `AppConfig.qwen36_gdr_fused`,
    /// применяется через [`synaptix::facade::llm::set_gdr_fused_disabled`].
    pub qwen36_gdr_fused: RwSignal<bool>,
    /// Размер chunk'а в `prefill_chunked`. На 24 GB GPU + ≥13K промпте может
    /// потребоваться уменьшить с default 1024 до 64-256. Сохраняется в
    /// `AppConfig.qwen36_prefill_chunk`, применяется через
    /// [`synaptix::facade::llm::set_prefill_chunk_size`].
    pub qwen36_prefill_chunk: RwSignal<usize>,
    /// Layer-sync режим (`"auto"` / `"on"` / `"off"`). Управляет
    /// `cudaStreamSynchronize` после каждого decoder-слоя в forward'е
    /// Qwen3.6. Auto = sync только когда T > 1 (prefill) — нулевая цена на
    /// decode, +4 GB free на длинном prefill. Сохраняется в
    /// `AppConfig.qwen36_layer_sync`, применяется через
    /// [`synaptix::facade::llm::set_layer_sync_mode`].
    pub qwen36_layer_sync: RwSignal<String>,
    pub qwen36_nvfp4_mma: RwSignal<bool>,
    pub qwen36_nvfp4_gemv: RwSignal<bool>,
    /// Путь к ACE-Step «общему» bundle'у (xl-base или xl-turbo). Используется
    /// нодами TextEncoder/LyricEncoder/TimbreEncoder/Sampler/ArLm для загрузки
    /// TextProjector / LyricEncoder / TimbreEncoder / DiT / NullCondEmb /
    /// Каталог моделей всех пайплайнов (.syn/LoRA/HF-каталоги). Сохраняется
    /// в `AppConfig.models_dir`; читают агентский `pipelines list` и
    /// настройки. Резолв `~` — `config::resolve_models_dir`.
    pub models_dir: RwSignal<String>,
    /// FSQ / Detokenizer. Сохраняется в `AppConfig.acestep_xl_bundle_path`.
    pub acestep_xl_bundle_path: RwSignal<Option<String>>,
    /// Путь к ACE-Step VAE bundle'у (`acestep_vae.syn`). Используется нодами
    /// VaeEncode/VaeDecode. Сохраняется в `AppConfig.acestep_vae_bundle_path`.
    pub acestep_vae_bundle_path: RwSignal<Option<String>>,
    /// Видимость левой/правой панели по страницам (тогглы в общей шапке).
    /// Persist: `AppConfig.panels`.
    pub panels: PanelsCtx,
    /// Разделители нав-рейла — штампы `created_at` (unix-мс). Persist:
    /// `AppConfig.rail_separators`. См. `crate::rail`.
    pub rail_separators: RwSignal<Vec<u64>>,
    /// Разделители страниц HuggingFace и настроек (persist в `AppConfig`).
    pub hf_left_split_ratio: RwSignal<f32>,
    pub hf_right_split_ratio: RwSignal<f32>,
    pub settings_left_split_ratio: RwSignal<f32>,
    pub settings_right_split_ratio: RwSignal<f32>,
}
