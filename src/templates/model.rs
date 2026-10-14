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

use crate::pages::node_editor::types::{FilterMode, NodeKind};

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
    FfmpegPlayer(FfmpegPlayerStateData),
    ImageLoad(ImageLoadStateData),
    ImageSave(ImageSaveStateData),
}

/// State ноды Image: путь к картинке.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageLoadStateData {
    #[serde(default)]
    pub image_path: Option<String>,
}

/// State ноды Image Save: куда писать (формат — по расширению).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageSaveStateData {
    #[serde(default)]
    pub path: Option<String>,
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

fn default_filter_mode() -> FilterMode {
    FilterMode::LowPass
}
fn default_filter_cutoff() -> f32 {
    1_000.0
}
fn default_volume() -> f32 {
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
