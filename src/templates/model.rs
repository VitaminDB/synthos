//! Сериализуемая модель шаблона графа нод.
//!
//! Шаблон — это снимок «канваса» (или его части): набор нод с позициями и
//! значениями полей плюс связи между ними. Хранится как JSON-файл в
//! `~/.config/synthos/templates/<slug>.json` либо bundled-через-код для
//! builtin-шаблонов (`builtin.rs`).
//!
//! Имена портов и полей в типах графа — `&'static str`, потому что
//! схема нод фиксирована в [`super::super::pages::node_editor::registry`].
//! При загрузке шаблона строковое `port`/`field`-имя резолвится обратно
//! к статическому через [`super::convert::resolve_port_name`] —
//! неизвестные имена пропускаются молча (graceful degradation для
//! кросс-версионных сохранений).

use syngui::core::{Color, Point};
use serde::{Deserialize, Serialize};

use crate::pages::node_editor::types::{FilterMode, NodeKind, PostNormMode};

/// Тип шаблона: целая сцена или «кусок».
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    /// Полный граф — целая канвас-сцена. Применение = «открыть как
    /// новую вкладку» (или заменить текущую).
    Full,
    /// Subgraph — подмножество нод (выделенные). Применение = вставить
    /// в текущую вкладку со смещением курсора.
    Subgraph,
}

impl TemplateKind {
    pub fn label(self) -> &'static str {
        match self {
            TemplateKind::Full => "Full",
            TemplateKind::Subgraph => "Subgraph",
        }
    }
}

/// Сериализуемое значение одного поля ноды. Цвет хранится hex-строкой
/// (`#RRGGBB`) — единственный формат, который мы консистентно
/// roundtrip'аем через JSON и человеко-читаемо редактируется.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum FieldValueData {
    Text(String),
    Float(f32),
    Int(i32),
    Bool(bool),
    /// `#RRGGBB` или `#RRGGBBAA`.
    Color(String),
    Choice(usize),
}

impl FieldValueData {
    /// Цвет → `#RRGGBB` (если alpha=255) либо `#RRGGBBAA`.
    pub fn from_color(c: Color) -> Self {
        let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u8;
        let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u8;
        let hex = if a == 255 {
            format!("#{:02X}{:02X}{:02X}", r, g, b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        };
        FieldValueData::Color(hex)
    }
}

/// Сериализуемая нода. Имя поля — динамическая `String`, при загрузке
/// преобразуется обратно к `&'static str` через лукап в registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeData {
    /// Локальный id внутри шаблона — переназначается при apply.
    pub id: u64,
    pub kind: NodeKind,
    pub pos: PointData,
    #[serde(default)]
    pub fields: std::collections::BTreeMap<String, FieldValueData>,
    /// Визуальный стиль карточки (tint + shadow). Старые шаблоны без
    /// этого поля десериализуются с `NodeStyleData::default()` — нода
    /// рендерится как раньше.
    #[serde(default)]
    pub style: NodeStyleData,
    /// Включена ли нода. Старые шаблоны → `true`.
    #[serde(default = "default_enabled_true")]
    pub enabled: bool,
    /// Per-kind runtime-state ноды (model_path, gain'ы, content и т. д.).
    /// Старые шаблоны без `state` → `None`, нода создаётся с дефолтным
    /// runtime'ом из [`crate::pages::node_editor::registry::default_runtime`].
    /// Несовпадение вариантов `NodeStateData` с `kind` молча игнорируется
    /// (forward/backward compat между версиями схемы).
    #[serde(default)]
    pub state: Option<NodeStateData>,
}

fn default_enabled_true() -> bool { true }

/// Сериализуемый стиль ноды. `tint` хранится hex-строкой (`#RRGGBB`/
/// `#RRGGBBAA`) — как и `FieldValueData::Color`, потому что
/// `syngui::core::Color` не имеет serde-derive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeStyleData {
    #[serde(default)]
    pub tint: Option<String>,
    #[serde(default = "default_enabled_true")]
    pub shadow: bool,
}

impl Default for NodeStyleData {
    fn default() -> Self {
        Self { tint: None, shadow: true }
    }
}

impl NodeStyleData {
    /// Hex-строка из `Color`. Совпадает с форматом `FieldValueData::from_color`.
    pub fn color_to_hex(c: Color) -> String {
        let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u8;
        let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u8;
        if a == 255 {
            format!("#{:02X}{:02X}{:02X}", r, g, b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        }
    }
}

/// Сериализуемая связь между двумя портами разных нод.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnData {
    pub from_node: u64,
    pub from_port: String,
    pub to_node: u64,
    pub to_port: String,
}

/// Простой Point для serde — `syngui::core::Point` не имеет serde-derive.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointData {
    pub x: f32,
    pub y: f32,
}

impl From<Point> for PointData {
    fn from(p: Point) -> Self {
        Self { x: p.x, y: p.y }
    }
}
impl From<PointData> for Point {
    fn from(p: PointData) -> Self {
        Point::new(p.x, p.y)
    }
}

/// Снимок viewport'а (для Full-шаблонов): pan-точка и зум. None — не
/// восстанавливать (использовать дефолт NodeEditorCtx).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewportData {
    pub pan: PointData,
    pub zoom: f32,
}

/// Шаблон графа. Поле `id`/`builtin` не сериализуются: id берётся из
/// имени файла (для disk) либо ставится в [`super::builtin`] вручную;
/// `builtin=true` означает «нельзя удалить/переименовать», читать только.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    #[serde(skip)]
    pub id: String,
    #[serde(skip)]
    pub builtin: bool,

    pub name: String,
    #[serde(default)]
    pub description: String,
    pub kind: TemplateKind,
    #[serde(default)]
    pub nodes: Vec<NodeData>,
    #[serde(default)]
    pub connections: Vec<ConnData>,
    #[serde(default)]
    pub viewport: Option<ViewportData>,
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeStateData — per-kind пользовательский state, живущий в `NodeRuntime`.
//
// `NodeData.fields` сохраняет только `FieldValue`-сигналы (из
// `NodeKindMeta.fields`). Реальные «настройки» большинства нод (выбранный
// `.syn`, dropdown'ы device/storage/compute, content markdown'а, gain'ы,
// текст транскрибата и т. д.) живут в `NodeRuntime`-варианте — отдельно
// от `fields`. `NodeStateData` зеркалит их в JSON.
//
// Runtime-объекты (Transcriber, OmniVoicePipeline, AudioPlayer, RecordingSession,
// thread-handle'ы, channel-receiver'ы, biquad'ы, atomic'и, buffer-cache,
// AudioBuffer'ы) НЕ сериализуются — после load они пересоздаются дефолтно
// через `default_runtime`, тяжёлые модели грузятся лениво на первый Play.
//
// `PathBuf` хранится как `String` через `to_string_lossy()`/`PathBuf::from`
// — тот же паттерн, что и `Color → #RRGGBB` в `FieldValueData::Color`:
// `syngui::core::Color`/`std::path::PathBuf` сами по себе serde-derive не
// имеют, и в JSON мы держим человеко-редактируемое представление.
// ─────────────────────────────────────────────────────────────────────────────

/// Per-kind runtime-state ноды. Один вариант на каждый `NodeKind`, у которого
/// в `NodeRuntime` есть пользовательский state (модель, файл, текст, sliders).
/// Чистые reactive-ноды (`Number`, `Add`, `Output`, `Demo`, `AudioFile`) не
/// имеют state — для них `NodeData.state == None`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum NodeStateData {
    AsrGigaam(AsrGigaamStateData),
    OmniVoice(OmniVoiceStateData),
    VoxCpm2(VoxCpm2StateData),
    Llm(LlmStateData),
    SortformerDiarizer(SortformerDiarizerStateData),
    MarkdownView(MarkdownViewStateData),
    TextView(TextViewStateData),
    Gain(GainStateData),
    Filter(FilterStateData),
    Reverb(ReverbStateData),
    Equalizer(EqualizerStateData),
    Mixer(MixerStateData),
    AudioFile(AudioFileStateData),
    AudioPlayer(AudioPlayerStateData),
    AudioRecorder(AudioRecorderStateData),
    SaveToFile(SaveToFileStateData),
    AceStepTextEncoder(AceStepModelStateData),
    AceStepLyricEncoder(AceStepLyricEncoderStateData),
    AceStepTimbreEncoder(AceStepModelStateData),
    AceStepVaeEncode(AceStepVaeStateData),
    AceStepVaeDecode(AceStepVaeDecodeStateData),
    AceStepArLm(AceStepArLmStateData),
    AceStepSampler(AceStepSamplerStateData),
    AceStepCheckpoint(AceStepCheckpointStateData),
    AceStepGenerate(AceStepGenerateStateData),
    FfmpegPlayer(FfmpegPlayerStateData),
    LtxCheckpoint(LtxCheckpointStateData),
    LtxTextEncoder(LtxTextEncoderStateData),
    LtxNagPrompt(LtxNagPromptStateData),
    LtxSamplerStage1(LtxSamplerStage1StateData),
    LtxSamplerStage2(LtxSamplerStage2StateData),
    LtxVideoSave(LtxVideoSaveStateData),
    LtxImage(LtxImageStateData),
    LtxVideoInput(LtxVideoInputStateData),
    LtxRetake(LtxRetakeStateData),
    LtxIcLora(LtxIcLoraStateData),
    LtxAudioInput(LtxAudioInputStateData),
    LtxLipdub(LtxLipdubStateData),
    LtxA2V(LtxA2VStateData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxA2VStateData {
    #[serde(default = "default_ltx_width")]
    pub width: u32,
    #[serde(default = "default_ltx_height")]
    pub height: u32,
    #[serde(default = "default_ltx_duration")]
    pub duration_seconds: f32,
    #[serde(default)]
    pub fps_idx: usize,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LtxA2VStateData {
    fn default() -> Self {
        Self {
            width: default_ltx_width(),
            height: default_ltx_height(),
            duration_seconds: default_ltx_duration(),
            fps_idx: 0,
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxAudioInputStateData {
    #[serde(default)]
    pub audio_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxLipdubStateData {
    #[serde(default = "default_ltx_width")]
    pub width: u32,
    #[serde(default = "default_ltx_height")]
    pub height: u32,
    #[serde(default = "default_ltx_duration")]
    pub duration_seconds: f32,
    #[serde(default)]
    pub fps_idx: usize,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LtxLipdubStateData {
    fn default() -> Self {
        Self {
            width: default_ltx_width(),
            height: default_ltx_height(),
            duration_seconds: default_ltx_duration(),
            fps_idx: 0,
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxIcLoraStateData {
    #[serde(default = "default_ltx_width")]
    pub width: u32,
    #[serde(default = "default_ltx_height")]
    pub height: u32,
    #[serde(default = "default_ltx_duration")]
    pub duration_seconds: f32,
    #[serde(default)]
    pub fps_idx: usize,
    #[serde(default = "default_ltx_downscale")]
    pub downscale: u32,
    #[serde(default = "default_ltx_image_strength")]
    pub ref_strength: f32,
    #[serde(default)]
    pub control_idx: usize,
    #[serde(default = "default_ltx_canny_low")]
    pub canny_low: f32,
    #[serde(default = "default_ltx_canny_high")]
    pub canny_high: f32,
    #[serde(default)]
    pub depth_model_path: Option<String>,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LtxIcLoraStateData {
    fn default() -> Self {
        Self {
            width: default_ltx_width(),
            height: default_ltx_height(),
            duration_seconds: default_ltx_duration(),
            fps_idx: 0,
            downscale: default_ltx_downscale(),
            ref_strength: default_ltx_image_strength(),
            control_idx: 0,
            canny_low: default_ltx_canny_low(),
            canny_high: default_ltx_canny_high(),
            depth_model_path: None,
            seed: 0,
        }
    }
}

fn default_ltx_canny_low() -> f32 {
    0.1
}
fn default_ltx_canny_high() -> f32 {
    0.3
}

fn default_ltx_downscale() -> u32 {
    2
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxVideoInputStateData {
    #[serde(default)]
    pub video_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxRetakeStateData {
    #[serde(default = "default_ltx_width")]
    pub width: u32,
    #[serde(default = "default_ltx_height")]
    pub height: u32,
    #[serde(default = "default_ltx_duration")]
    pub duration_seconds: f32,
    #[serde(default)]
    pub fps_idx: usize,
    #[serde(default)]
    pub retake_start: f32,
    #[serde(default = "default_ltx_retake_end")]
    pub retake_end: f32,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LtxRetakeStateData {
    fn default() -> Self {
        Self {
            width: default_ltx_width(),
            height: default_ltx_height(),
            duration_seconds: default_ltx_duration(),
            fps_idx: 0,
            retake_start: 0.0,
            retake_end: default_ltx_retake_end(),
            seed: 0,
        }
    }
}

fn default_ltx_retake_end() -> f32 {
    2.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxImageStateData {
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default = "default_ltx_image_strength")]
    pub strength: f32,
    #[serde(default)]
    pub frame_idx: u32,
}

impl Default for LtxImageStateData {
    fn default() -> Self {
        Self { image_path: None, strength: default_ltx_image_strength(), frame_idx: 0 }
    }
}

fn default_ltx_image_strength() -> f32 {
    1.0
}

/// State Checkpoint-ноды LTX-2.3: пути подмоделей + device/quant/compute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxCheckpointStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub gemma_dir: Option<String>,
    #[serde(default)]
    pub upscaler_path: Option<String>,
    #[serde(default)]
    pub lora_path: Option<String>,
    #[serde(default = "default_ltx_lora_strength")]
    pub lora_strength: f32,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub quant_dit_idx: usize,
    #[serde(default)]
    pub quant_enc_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
}

fn default_ltx_lora_strength() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxTextEncoderStateData {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub keep_gemma: bool,
}

/// State NAG-ноды. Дефолты scale/alpha/tau — из
/// `synaptix_video_ltx23::pipeline::NAG_DEFAULT_*`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxNagPromptStateData {
    #[serde(default = "default_ltx_nag_prompt")]
    pub prompt: String,
    #[serde(default = "default_ltx_nag_scale")]
    pub scale: f32,
    #[serde(default = "default_ltx_nag_alpha")]
    pub alpha: f32,
    #[serde(default = "default_ltx_nag_tau")]
    pub tau: f32,
}

impl Default for LtxNagPromptStateData {
    fn default() -> Self {
        Self {
            prompt: default_ltx_nag_prompt(),
            scale: default_ltx_nag_scale(),
            alpha: default_ltx_nag_alpha(),
            tau: default_ltx_nag_tau(),
        }
    }
}

fn default_ltx_nag_prompt() -> String {
    synaptix_video_ltx23::pipeline::DEFAULT_NAG_PROMPT.to_string()
}
fn default_ltx_nag_scale() -> f32 {
    synaptix_video_ltx23::pipeline::NAG_DEFAULT_SCALE
}
fn default_ltx_nag_alpha() -> f32 {
    synaptix_video_ltx23::pipeline::NAG_DEFAULT_ALPHA
}
fn default_ltx_nag_tau() -> f32 {
    synaptix_video_ltx23::pipeline::NAG_DEFAULT_TAU
}

/// State Sampler-Stage1 ноды: целевые размеры/длительность/fps/seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LtxSamplerStage1StateData {
    #[serde(default = "default_ltx_width")]
    pub width: u32,
    #[serde(default = "default_ltx_height")]
    pub height: u32,
    #[serde(default = "default_ltx_duration")]
    pub duration_seconds: f32,
    #[serde(default)]
    pub fps_idx: usize,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LtxSamplerStage1StateData {
    fn default() -> Self {
        Self {
            width: default_ltx_width(),
            height: default_ltx_height(),
            duration_seconds: default_ltx_duration(),
            fps_idx: 0,
            seed: 0,
        }
    }
}

fn default_ltx_width() -> u32 {
    1024
}
fn default_ltx_height() -> u32 {
    576
}
fn default_ltx_duration() -> f32 {
    10.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxSamplerStage2StateData {
    #[serde(default)]
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LtxVideoSaveStateData {
    #[serde(default)]
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct FfmpegPlayerStateData {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default = "default_ffmpeg_volume")]
    pub volume: f32,
    #[serde(default = "default_ffmpeg_size")]
    pub size: (f32, f32),
    #[serde(default)]
    pub hwaccel_idx: usize,
}

fn default_ffmpeg_volume() -> f32 {
    1.0
}

fn default_ffmpeg_size() -> (f32, f32) {
    (320.0, 240.0)
}

/// State ноды диаризации Sortformer. `model_path` хранится как строка
/// (`PathBuf::to_string_lossy`). `output_pretty`/`output_json` сохраняются,
/// чтобы шаблон удерживал ранее полученный результат до следующего Play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SortformerDiarizerStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_sortformer_threshold")]
    pub threshold: f32,
    #[serde(default = "default_sortformer_allow_overlap")]
    pub allow_overlap: bool,
    #[serde(default)]
    pub output_pretty: String,
    #[serde(default)]
    pub output_json: String,
}

fn default_sortformer_threshold() -> f32 {
    0.5
}
fn default_sortformer_allow_overlap() -> bool {
    true
}

/// State ASR-ноды GigaAM. `model_path` — `PathBuf::to_string_lossy()` на save,
/// `PathBuf::from(s)` на load. `output_text` сохраняется, чтобы шаблон-снапшот
/// удерживал ранее полученный транскрибат до повторного Play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AsrGigaamStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default)]
    pub output_text: String,
}

/// State TTS-ноды OmniVoice. Параметры sampling'а — те же, что
/// `synaptix::facade::tts::core::GenerationConfig::default()` в качестве дефолтов.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct OmniVoiceStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default)]
    pub instruct: String,
    /// Fallback ref-text если ref_text-порт не подключён.
    #[serde(default)]
    pub ref_text: String,
    #[serde(default)]
    pub language: String,
    #[serde(default = "default_num_step")]
    pub num_step: u32,
    #[serde(default = "default_guidance_scale")]
    pub guidance_scale: f32,
    #[serde(default = "default_t_shift")]
    pub t_shift: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub seed: u64,
}

/// State LLM-ноды (synaptix Qwen3 / Hybrid). Тяжёлый pipeline не
/// сериализуется — грузится лениво на первый Run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LlmStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub quant_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default = "default_context")]
    pub context: u32,
    #[serde(default)]
    pub think: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub top_k: u32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default)]
    pub min_p: f32,
    #[serde(default = "default_repetition_penalty")]
    pub repetition_penalty: f32,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct VoxCpm2StateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default)]
    pub prompt_text: String,
    #[serde(default = "default_guidance_scale")]
    pub cfg_value: f32,
    #[serde(default = "default_voxcpm_steps")]
    pub n_timesteps: u32,
    #[serde(default = "default_voxcpm_max_len")]
    pub max_len: u32,
    #[serde(default = "default_voxcpm_seed")]
    pub seed: u64,
}

fn default_voxcpm_steps() -> u32 {
    10
}
fn default_voxcpm_max_len() -> u32 {
    2000
}
fn default_voxcpm_seed() -> u64 {
    1988
}

fn default_context() -> u32 {
    4096
}
fn default_max_tokens() -> u32 {
    512
}
fn default_temperature() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    1.0
}
fn default_repetition_penalty() -> f32 {
    1.0
}

/// State `MarkdownView`-ноды: текст + размер карточки + edit-mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct MarkdownViewStateData {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
    #[serde(default)]
    pub edit_mode: bool,
}

/// State `TextView`-ноды: выходной текст + размер карточки.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TextViewStateData {
    #[serde(default)]
    pub output_text: String,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
}

/// State Gain-ноды.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct GainStateData {
    #[serde(default)]
    pub gain_db: f32,
}

/// State Filter-ноды. `FilterMode` уже Serialize/Deserialize в `types.rs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterStateData {
    #[serde(default = "default_filter_mode")]
    pub mode: FilterMode,
    #[serde(default = "default_filter_cutoff")]
    pub cutoff_hz: f32,
}

impl Default for FilterStateData {
    fn default() -> Self {
        Self {
            mode: default_filter_mode(),
            cutoff_hz: default_filter_cutoff(),
        }
    }
}

/// State Reverb-ноды (Schroeder).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ReverbStateData {
    #[serde(default)]
    pub mix: f32,
    #[serde(default)]
    pub room: f32,
}

/// State Equalizer-ноды. Длина `gains_db` определяется n_bands (6/10/20/30);
/// при mismatch применяем prefix-min (см. `convert.rs::apply_state_to_runtime`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct EqualizerStateData {
    #[serde(default)]
    pub gains_db: Vec<f32>,
}

/// State Mixer-ноды. `n_inputs` ∈ [2..=`MIXER_MAX_INPUTS`]; `gains_db.len()`
/// должно быть == `MIXER_MAX_INPUTS`, но загрузчик truncate/pad'ит если что.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct MixerStateData {
    #[serde(default = "default_mixer_n_inputs")]
    pub n_inputs: usize,
    #[serde(default)]
    pub gains_db: Vec<f32>,
}

/// State AudioFile-ноды: путь к загруженному файлу. Сам PCM-буфер
/// (`Arc<AudioBuffer>`) тяжёлый и пересоздаётся лениво через `spawn_load`
/// после restore — клиент видит «Загрузка…» доли секунды.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AudioFileStateData {
    #[serde(default)]
    pub loaded_path: Option<String>,
}

/// State AudioPlayer-ноды: только volume (transport-state не сохраняется —
/// плеер всегда стартует со Stopped).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioPlayerStateData {
    #[serde(default = "default_volume")]
    pub volume: f32,
}

impl Default for AudioPlayerStateData {
    fn default() -> Self {
        Self { volume: default_volume() }
    }
}

/// State AudioRecorder-ноды: имя выбранного устройства (None = default).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AudioRecorderStateData {
    #[serde(default)]
    pub device: Option<String>,
}

/// State SaveToFile-ноды: путь к выходному файлу.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SaveToFileStateData {
    #[serde(default)]
    pub path: String,
}

/// Базовый state для ACE-Step моделей без специфичных параметров
/// (TextEncoder, TimbreEncoder). Минимум — путь к `.syn` + dropdown'ы
/// device/storage/compute (raw-индексы как в UI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AceStepModelStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
}

/// State LyricEncoder-ноды — `AceStepModelStateData` + `language` (язык
/// лирики, подсказка токенайзеру, default "en").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AceStepLyricEncoderStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_lyric_language")]
    pub language: String,
}

/// State VaeEncode-ноды: chunk/overlap для длинных файлов. Путь к VAE-bundle
/// глобальный (Settings → AI Models → ACE-Step), per-нодного path-поля нет.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AceStepVaeStateData {
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_vae_chunk_seconds")]
    pub chunk_seconds: f32,
    #[serde(default = "default_vae_overlap_seconds")]
    pub overlap_seconds: f32,
}

impl Default for AceStepVaeStateData {
    fn default() -> Self {
        Self {
            device_idx: 0,
            storage_idx: 0,
            compute_idx: 0,
            chunk_seconds: default_vae_chunk_seconds(),
            overlap_seconds: default_vae_overlap_seconds(),
        }
    }
}

/// State VaeDecode-ноды: VAE-state + `post_norm` (нормализация
/// амплитуды после decode, стандарт ACE-Step). Путь к VAE-bundle глобальный.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AceStepVaeDecodeStateData {
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_vae_chunk_seconds")]
    pub chunk_seconds: f32,
    #[serde(default = "default_vae_overlap_seconds")]
    pub overlap_seconds: f32,
    #[serde(default)]
    pub post_norm_mode: PostNormMode,
}

impl Default for AceStepVaeDecodeStateData {
    fn default() -> Self {
        Self {
            device_idx: 0,
            storage_idx: 0,
            compute_idx: 0,
            chunk_seconds: default_vae_chunk_seconds(),
            overlap_seconds: default_vae_overlap_seconds(),
            post_norm_mode: PostNormMode::default(),
        }
    }
}

/// State ArLm-ноды (Qwen AR + FSQ + Detokenizer): sampling-параметры,
/// CFG, negative prompt, CoT-флаг, длина `audio_codes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AceStepArLmStateData {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_ar_temperature")]
    pub temperature: f32,
    #[serde(default = "default_ar_top_p")]
    pub top_p: f32,
    #[serde(default = "default_ar_top_k")]
    pub top_k: u32,
    #[serde(default = "default_ar_min_p")]
    pub min_p: f32,
    #[serde(default = "default_ar_cfg_scale")]
    pub lm_cfg_scale: f32,
    #[serde(default = "default_ar_phase1_cfg_scale")]
    pub lm_phase1_cfg_scale: f32,
    #[serde(default)]
    pub lm_negative_prompt: String,
    #[serde(default = "default_ar_use_cot")]
    pub use_cot: bool,
    #[serde(default = "default_ar_use_phase1_cot")]
    pub use_phase1_cot: bool,
    #[serde(default = "default_ar_use_sdpa")]
    pub use_sdpa: bool,
    #[serde(default = "default_ar_audio_codes_count")]
    pub audio_codes_count: u32,
    /// Seed для AR LM sampling. `0` = wall-clock (random); >0 — детерминированный.
    #[serde(default)]
    pub seed: u64,
}

impl Default for AceStepArLmStateData {
    fn default() -> Self {
        Self {
            model_path: None,
            device_idx: 0,
            storage_idx: 0,
            compute_idx: 0,
            temperature: default_ar_temperature(),
            top_p: default_ar_top_p(),
            top_k: default_ar_top_k(),
            min_p: default_ar_min_p(),
            lm_cfg_scale: default_ar_cfg_scale(),
            lm_phase1_cfg_scale: default_ar_phase1_cfg_scale(),
            lm_negative_prompt: String::new(),
            use_cot: default_ar_use_cot(),
            use_phase1_cot: default_ar_use_phase1_cot(),
            use_sdpa: default_ar_use_sdpa(),
            audio_codes_count: default_ar_audio_codes_count(),
            seed: 0,
        }
    }
}

/// State Sampler-ноды (DiT + scheduler): только Text2Music-параметры
/// (длительность / steps / cfg / seed / DCW-toggle). Multi-task-режимы
/// (Cover / Repaint / Extract / Lego / Complete) и связанные параметры
/// (task_type, repaint_*, lego_*, track_name, sliding-attention, BPM/
/// timesignature/keyscale hints) удалены вместе со старой страницей
/// `music_gen` — пользователь собирает такие сценарии из нод вручную
/// (Audio → VaeEncode → Sampler.src_latent). Путь к xl-bundle глобальный
/// (Settings → AI Models → ACE-Step), per-нодного path-поля нет.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AceStepSamplerStateData {
    #[serde(default)]
    pub device_idx: usize,
    #[serde(default)]
    pub storage_idx: usize,
    #[serde(default)]
    pub compute_idx: usize,
    #[serde(default = "default_sampler_duration_seconds")]
    pub duration_seconds: f32,
    #[serde(default = "default_sampler_infer_steps")]
    pub infer_steps: u32,
    #[serde(default = "default_sampler_cfg_scale")]
    pub cfg_scale: f32,
    #[serde(default = "default_sampler_flow_match_shift")]
    pub flow_match_shift: f32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_sampler_enable_dcw")]
    pub enable_dcw: bool,
    #[serde(default = "default_sampler_dcw_mode")]
    pub dcw_mode: DcwModeData,
    #[serde(default = "default_sampler_dcw_scaler")]
    pub dcw_scaler: f32,
    #[serde(default = "default_sampler_dcw_high_scaler")]
    pub dcw_high_scaler: f32,
    #[serde(default = "default_sampler_dcw_wavelet")]
    pub dcw_wavelet: DcwWaveletData,
    #[serde(default = "default_sampler_dcw_preset")]
    pub dcw_preset: DcwPresetData,
    #[serde(default = "default_sampler_preset")]
    pub preset: SamplerPresetData,
}

/// Зеркало `node_editor::types::DcwModeOption` для persist в JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwModeData {
    Low,
    High,
    Double,
    Pix,
}

/// Зеркало `node_editor::types::DcwWaveletOption` для persist в JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwWaveletData {
    Haar,
    Db4,
    Sym8,
}

/// Зеркало `node_editor::types::DcwPresetOption` для persist в JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwPresetData {
    NoThink,
    Think,
    Custom,
}

/// Зеркало `node_editor::types::SamplerPreset` для persist в JSON шаблонов.
/// Имеем дублирование чтобы не тянуть `types.rs` в `templates/model.rs`
/// (разные слои абстракции, model — чистый serde, types — runtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SamplerPresetData {
    Auto,
    Turbo,
    Base,
    Sft,
}

impl Default for AceStepSamplerStateData {
    fn default() -> Self {
        Self {
            device_idx: 0,
            storage_idx: 0,
            compute_idx: 0,
            duration_seconds: default_sampler_duration_seconds(),
            infer_steps: default_sampler_infer_steps(),
            cfg_scale: default_sampler_cfg_scale(),
            flow_match_shift: default_sampler_flow_match_shift(),
            seed: 0,
            enable_dcw: default_sampler_enable_dcw(),
            dcw_mode: default_sampler_dcw_mode(),
            dcw_scaler: default_sampler_dcw_scaler(),
            dcw_high_scaler: default_sampler_dcw_high_scaler(),
            dcw_wavelet: default_sampler_dcw_wavelet(),
            dcw_preset: default_sampler_dcw_preset(),
            preset: default_sampler_preset(),
        }
    }
}

/// State ACE-Step Checkpoint-ноды: каталог моделей + 4 опц. override +
/// device/quant/compute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AceStepCheckpointStateData {
    pub models_dir: Option<String>,
    pub lm_path: Option<String>,
    pub text_encoder_path: Option<String>,
    pub dit_path: Option<String>,
    pub vae_path: Option<String>,
    pub device_idx: usize,
    pub quant_dit_idx: usize,
    pub quant_enc_idx: usize,
    pub compute_idx: usize,
}

/// State ACE-Step Generate-ноды (монолит): режим + sampler/AR/DCW-параметры
/// + режим-зависимые (retake/repaint/edit). Контент (tags/lyrics/src_latent)
/// и модель приходят портами — здесь не хранятся.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AceStepGenerateStateData {
    pub mode_idx: usize,
    pub preset: SamplerPresetData,
    pub duration_seconds: f32,
    pub infer_steps: u32,
    pub cfg_scale: f32,
    pub flow_match_shift: f32,
    pub seed: u64,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub lm_cfg_scale: f32,
    pub use_cot: bool,
    pub use_ar: bool,
    pub bpm: u32,
    pub keyscale_idx: usize,
    pub timesig_idx: usize,
    pub norm_mode: PostNormMode,
    pub enable_dcw: bool,
    pub dcw_mode: DcwModeData,
    pub dcw_scaler: f32,
    pub dcw_high_scaler: f32,
    pub dcw_wavelet: DcwWaveletData,
    pub dcw_preset: DcwPresetData,
    pub retake_variance: f32,
    pub retake_seed: u64,
    pub repaint_start_sec: f32,
    pub repaint_end_sec: f32,
    pub repaint_strength: f32,
    pub edit_n_min: f32,
    pub edit_n_max: f32,
}

impl Default for AceStepGenerateStateData {
    fn default() -> Self {
        Self {
            mode_idx: 0,
            preset: SamplerPresetData::Auto,
            duration_seconds: -1.0,
            infer_steps: 32,
            cfg_scale: 7.0,
            flow_match_shift: 1.0,
            seed: 0,
            temperature: 0.85,
            top_p: 0.9,
            top_k: 50,
            min_p: 0.0,
            lm_cfg_scale: 1.5,
            use_cot: false,
            use_ar: true,
            bpm: 0,
            keyscale_idx: 0,
            timesig_idx: 0,
            norm_mode: PostNormMode::Peak,
            enable_dcw: false,
            dcw_mode: DcwModeData::Double,
            dcw_scaler: 0.02,
            dcw_high_scaler: 0.06,
            dcw_wavelet: DcwWaveletData::Haar,
            dcw_preset: DcwPresetData::Think,
            retake_variance: 0.5,
            retake_seed: 0,
            repaint_start_sec: 0.0,
            repaint_end_sec: -1.0,
            repaint_strength: 0.7,
            edit_n_min: 0.0,
            edit_n_max: 1.0,
        }
    }
}

fn default_lyric_language() -> String {
    "en".into()
}
fn default_vae_chunk_seconds() -> f32 {
    30.0
}
fn default_vae_overlap_seconds() -> f32 {
    0.5
}
fn default_ar_temperature() -> f32 {
    1.0
}
fn default_ar_top_p() -> f32 {
    0.9
}
fn default_ar_top_k() -> u32 {
    50
}
fn default_ar_min_p() -> f32 {
    0.0
}
fn default_ar_cfg_scale() -> f32 {
    1.5
}
fn default_ar_phase1_cfg_scale() -> f32 {
    1.0
}
fn default_ar_use_cot() -> bool {
    true
}
fn default_ar_use_sdpa() -> bool {
    true
}
fn default_ar_use_phase1_cot() -> bool {
    true
}
fn default_ar_audio_codes_count() -> u32 {
    600
}
fn default_sampler_duration_seconds() -> f32 {
    30.0
}
fn default_sampler_infer_steps() -> u32 {
    // Base preset из reference `_BASE_DEFAULTS`. Auto при загрузке bundle'а
    // переопределит на правильное значение для конкретного варианта.
    50
}
fn default_sampler_cfg_scale() -> f32 {
    7.0
}
fn default_sampler_flow_match_shift() -> f32 {
    1.0
}
fn default_sampler_enable_dcw() -> bool {
    // DCW в нашем порте сломан. Дефолт OFF, чтобы старые шаблоны без
    // явного `enable_dcw` поля не включали кашу.
    false
}
fn default_sampler_dcw_mode() -> DcwModeData {
    DcwModeData::Double
}
fn default_sampler_dcw_scaler() -> f32 {
    // Think preset (consistent с Phase 1 CoT). См. `dcw_defaults.py:5-8`.
    0.02
}
fn default_sampler_dcw_high_scaler() -> f32 {
    0.06
}
fn default_sampler_dcw_wavelet() -> DcwWaveletData {
    DcwWaveletData::Haar
}
fn default_sampler_dcw_preset() -> DcwPresetData {
    DcwPresetData::Think
}
fn default_sampler_preset() -> SamplerPresetData {
    SamplerPresetData::Auto
}

fn default_filter_mode() -> FilterMode {
    FilterMode::LowPass
}
fn default_filter_cutoff() -> f32 {
    1_000.0
}
fn default_volume() -> f32 {
    1.0
}
fn default_num_step() -> u32 {
    32
}
fn default_guidance_scale() -> f32 {
    2.0
}
fn default_t_shift() -> f32 {
    0.1
}
fn default_speed() -> f32 {
    1.0
}
fn default_mixer_n_inputs() -> usize {
    2
}

impl Template {
    /// Пустой шаблон с этим именем (для UI «Save current as template»
    /// до того как пользователь начнёт редактировать).
    pub fn empty(name: impl Into<String>, kind: TemplateKind) -> Self {
        Self {
            id: String::new(),
            builtin: false,
            name: name.into(),
            description: String::new(),
            kind,
            nodes: Vec::new(),
            connections: Vec::new(),
            viewport: None,
        }
    }
}
