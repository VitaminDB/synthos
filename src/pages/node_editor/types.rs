//! Типы данных редактора нод: идентификаторы, схема портов, экземпляры
//! нод, соединения и pending-провод (drag-from-port).
//!
//! Хранение значений полей — `RwSignal<...>` внутри `NodeInstance.fields`,
//! чтобы каждое поле могло независимо подписываться на UI и не вызывать
//! полную перерисовку всей карточки при правке одного поля.

use syngui::core::{Color, Point, Rect, Size};
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

// Re-export AudioBuffer и AudioStream из syngui — единые transport-типы
// между нодами и виджетами.
pub use syngui::audio::{AudioBuffer, AudioStream};
use syngui::audio::{AudioPlayer, Biquad, RecordingSession, SchroederReverb};
pub use syngui::video::VideoStream;

/// Уникальный id ноды в графе.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

/// Максимальное количество входов микшера. Определяет размер пула
/// статических `PortSchema` (см. `nodes::audio_mixer::MIXER_PORT_SCHEMAS_FULL`)
/// и длину runtime-векторов в [`NodeRuntime::Mixer`].
pub const MIXER_MAX_INPUTS: usize = 16;

/// Тип данных, текущих по проводу. Пока используем единственное значение —
/// расширим, когда появятся реальные пайплайны.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortKind {
    Data,
    Audio,
    Control,
    Text,
    Video,
    /// Одна картинка (`DataBlob::Image`): выход FLUX VAE Decode / Image,
    /// вход LTX Image / H3 Keyframe / FLUX VAE Encode / Image Save.
    Image,
}

/// Сторона ноды, к которой относится порт.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortSide {
    Input,
    Output,
}

/// Идентификатор порта внутри одной ноды (стабильное имя из registry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PortKey {
    pub side: PortSide,
    /// Имя порта из `NodeKindMeta::inputs/outputs`.
    pub name: &'static str,
}

/// Preset ACE-Step sampler'а. Соответствует variant'у DiT-bundle'а; при смене
/// dropdown'а в UI подменяет infer_steps/cfg_scale/flow_match_shift на
/// заводские значения из reference `_BASE_DEFAULTS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SamplerPreset {
    /// По имени bundle'а через `detect_xl_bundle_kind`.
    Auto,
    /// `acestep_v15_xl_turbo` — 8 шагов, shift=3.0, cfg=1.0 (запечён).
    Turbo,
    /// `acestep_v15_xl_base` — 50 шагов, shift=1.0, cfg=7.0.
    Base,
    /// `acestep_v15_xl_sft` — тот же schedule что и base.
    Sft,
}

impl Default for SamplerPreset {
    fn default() -> Self {
        SamplerPreset::Auto
    }
}

/// Режим DCW (Differential Correction in Wavelet domain).
/// Соответствует `acestep_pipeline::dcw::DcwMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwModeOption {
    Low,
    High,
    Double,
    Pix,
}

impl Default for DcwModeOption {
    fn default() -> Self {
        DcwModeOption::Double
    }
}

impl DcwModeOption {
    pub fn as_str(self) -> &'static str {
        match self {
            DcwModeOption::Low => "low",
            DcwModeOption::High => "high",
            DcwModeOption::Double => "double",
            DcwModeOption::Pix => "pix",
        }
    }
    pub fn to_corrector_mode(self) -> synaptix_music_acestep::dcw::DcwMode {
        use synaptix_music_acestep::dcw::DcwMode;
        match self {
            DcwModeOption::Low => DcwMode::Low,
            DcwModeOption::High => DcwMode::High,
            DcwModeOption::Double => DcwMode::Double,
            DcwModeOption::Pix => DcwMode::Pix,
        }
    }
}

/// Wavelet basis для DCW. Сейчас порт поддерживает только Haar; остальные
/// — UI-only (включить когда добавим pytorch_wavelets-эквивалент).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwWaveletOption {
    Haar,
    Db4,
    Sym8,
}

impl Default for DcwWaveletOption {
    fn default() -> Self {
        DcwWaveletOption::Haar
    }
}

impl DcwWaveletOption {
    pub fn as_str(self) -> &'static str {
        match self {
            DcwWaveletOption::Haar => "haar",
            DcwWaveletOption::Db4 => "db4",
            DcwWaveletOption::Sym8 => "sym8",
        }
    }
}

/// DCW preset — задаёт пару (scaler, high_scaler) из эталонного
/// `acestep/ui/gradio/events/dcw_defaults.py`:
/// * `NoThink`: scaler=0.05, high_scaler=0.02
/// * `Think`:   scaler=0.02, high_scaler=0.06
/// * `Custom`:  значения вручную через слайдеры; смена слайдера
///   переключает preset на Custom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcwPresetOption {
    NoThink,
    Think,
    Custom,
}

impl Default for DcwPresetOption {
    fn default() -> Self {
        DcwPresetOption::Think
    }
}

impl DcwPresetOption {
    pub fn scalers(self) -> Option<(f32, f32)> {
        match self {
            DcwPresetOption::NoThink => Some((0.05, 0.02)),
            DcwPresetOption::Think => Some((0.02, 0.06)),
            DcwPresetOption::Custom => None,
        }
    }
}

/// Схема одного порта.
#[derive(Clone, Copy, Debug)]
pub struct PortSchema {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: PortKind,
}

/// Источник списка портов для конкретной ноды.
///
/// Большинство нод имеет фиксированный (заранее известный) набор портов —
/// для них хранится статический срез [`PortSchema`]. Вариативные ноды
/// (например, `Mixer` с настраиваемым количеством входов) задают функцию,
/// которая вычисляет актуальный список по экземпляру [`NodeInstance`]
/// (через runtime-state ноды), плюс статический «pool» — набор всех
/// возможных портов. Pool используется в schema-based операциях, где
/// конкретного экземпляра ещё нет (например, разрешение имён портов
/// при загрузке шаблона).
///
/// Контракт: все возвращаемые срезы — `&'static`. Имена `PortSchema.name`
/// тоже `&'static str`, что сохраняет совместимость с `Connection.from_port
/// / to_port`. Динамические ноды держат пул всех возможных схем в
/// `static`-массиве (см. `nodes::audio_mixer::MIXER_PORT_SCHEMAS_FULL`).
#[derive(Clone, Copy)]
pub enum PortsSpec {
    Static(&'static [PortSchema]),
    Dynamic {
        /// Полный список всех потенциально возможных портов (max-size).
        /// Используется schema-based операциями без NodeInstance.
        pool: &'static [PortSchema],
        /// Активный под-срез — то, что показывается на текущем экземпляре.
        runtime: fn(&NodeInstance) -> &'static [PortSchema],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortsLayout {
    Rows,
    Compact { centered: bool },
    None,
}

impl PortsSpec {
    /// Активный список портов для конкретного экземпляра ноды.
    pub fn resolve(self, node: &NodeInstance) -> &'static [PortSchema] {
        match self {
            PortsSpec::Static(s) => s,
            PortsSpec::Dynamic { runtime, .. } => runtime(node),
        }
    }

    /// Полный pool возможных портов. Совпадает с `resolve(...)` для
    /// `Static`; для `Dynamic` возвращает `pool`. Используется при
    /// загрузке шаблонов и валидации соединений на этапе схемы.
    pub fn pool(self) -> &'static [PortSchema] {
        match self {
            PortsSpec::Static(s) => s,
            PortsSpec::Dynamic { pool, .. } => pool,
        }
    }
}

impl std::fmt::Debug for PortsSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortsSpec::Static(s) => f.debug_tuple("Static").field(s).finish(),
            PortsSpec::Dynamic { pool, .. } => f
                .debug_struct("Dynamic")
                .field("pool", pool)
                .field("runtime", &"<fn>")
                .finish(),
        }
    }
}

/// Тип значения поля. RwSignal-обёртка лежит в `FieldValue` ниже.
#[derive(Clone, Copy, Debug)]
pub enum FieldType {
    Text,
    Float,
    Int,
    Bool,
    Color,
    Choice(&'static [&'static str]),
}

/// Описание поля ноды (без значения).
#[derive(Clone, Copy, Debug)]
pub struct FieldSchema {
    pub name: &'static str,
    pub label: &'static str,
    pub ty: FieldType,
}

/// Реактивное значение поля. Каждое поле — отдельный RwSignal, чтобы
/// одно поле не дёргало перерисовку всей карточки.
#[derive(Clone)]
pub enum FieldValue {
    Text(RwSignal<String>),
    Float(RwSignal<f32>),
    Int(RwSignal<i32>),
    Bool(RwSignal<bool>),
    Color(RwSignal<Color>),
    Choice(RwSignal<usize>),
}

/// Уникальная категория ноды (то, что можно «добавить» из меню).
///
/// `Serialize`/`Deserialize` нужны для шаблонов нод (см. `crate::templates`):
/// каждое имя сохраняется в JSON в `snake_case`-форме (`demo`, `number`, …),
/// которая совпадает с UI-каноном и устойчива к перекосу регистра.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Number,
    /// Сумматор (a + b → out).
    Add,
    /// Вывод (sink).
    Output,
    /// Источник аудио из файла (open file dialog → pcm).
    AudioFile,
    /// Аудио-плеер: input Audio + transport-controls + waveform с seek.
    AudioPlayer,
    /// Запись с микрофона + live waveform → output Audio.
    AudioRecorder,
    /// Линейный gain (-24…+24 dB) над AudioStream.
    Gain,
    /// Biquad-фильтр (LP/HP/BP) над AudioStream.
    Filter,
    /// Schroeder-реверб (mix + room) над AudioStream.
    Reverb,
    /// Запись AudioStream в WAV-файл по указанному пути.
    SaveToFile,
    /// Graphic-эквалайзер на 6 полос (каскад Biquad-peaking).
    Equalizer6,
    /// Graphic-эквалайзер на 10 полос (ISO octave).
    Equalizer10,
    /// Graphic-эквалайзер на 20 полос (half-octave).
    Equalizer20,
    /// Graphic-эквалайзер на 30 полос (ISO third-octave).
    Equalizer30,
    /// Микшер N → 1: суммирует AudioStream'ы (или offline-буферы) с
    /// per-канальными gain'ами. `n_inputs` живёт в runtime, активный
    /// под-набор определяется через [`PortsSpec::Dynamic`].
    Mixer,
    /// Декоративная нода с markdown-содержимым. Без портов / executor'а;
    /// рендерит `MarkdownView` (по умолчанию) или `MarkdownEditor`
    /// (режим редактирования), размер задаётся пользователем через
    /// resize-handle'ы (`TransformBox`). Видимость handle'ов и режим
    /// редактирования переключаются через ContextMenu по правому клику.
    MarkdownView,
    /// Нода распознавания речи на базе модели **GigaAM** (CTC).
    /// Audio in → Text out. Параметры (путь к `.syn`, device, storage/compute
    /// dtype) выбираются в body ноды; модель загружается лениво при первом
    /// нажатии Play и кэшируется в runtime до удаления ноды или смены
    /// настроек.
    AsrGigaam,
    /// Editable text-display нода. Text in → Text out (passthrough).
    /// Body — `MultilineTextEdit`. При смене upstream Text заменяет
    /// содержимое (bump `text_version` → Reactive rebuild). Ручные правки
    /// пользователя сохраняются до следующего изменения upstream'а.
    TextView,
    /// TTS-нода на базе **OmniVoice** (Qwen3-1B LM + Higgs-Audio codec +
    /// discrete mask-diffusion). Принимает текст и опциональные ref-audio /
    /// ref-text для voice-clone; параметры (cfg, num_step, t_shift, speed,
    /// seed) выбираются в body ноды. Pipeline загружается лениво при первом
    /// Play и кэшируется в runtime до удаления ноды или смены настроек.
    OmniVoice,
    VoxCpm2,
    VibeVoice,
    /// Универсальный чекпойнт .syn/HF для слот-семейств (LLM/TTS/ASR/
    /// диаризация): публикует [`SynModelHandle`] в порт `model`.
    SynCheckpoint,
    /// Нода диаризации спикеров на базе NVIDIA Streaming Sortformer v2.1
    /// (FastConformer encoder + Sortformer head, до 4 спикеров). Audio in
    /// → Text(JSON) out. Параметры (`.syn`, device, storage/compute, threshold,
    /// allow_overlap) выбираются в body. Sortformer загружается лениво при
    /// первом Play и кэшируется в runtime. Live-stream пока не поддерживается
    /// (фаза 2 — streaming с KV-cache).
    SortformerDiarizer,
    /// LLM-нода на нативном **synaptix**-стеке (Qwen3 dense/MoE либо
    /// Qwen3-Next-Hybrid — арх детектится из `config.json`). Входы: `prompt`
    /// (вопрос, Text) и опциональный `system` (Text, переопределяет
    /// system-fallback из body). Выход: `answer` (Text) — подключается к
    /// `TextView`. Параметры (`.syn`/HF-каталог, device, quant, compute,
    /// system-промпт, max_tokens, temperature, seed) задаются в body; pipeline
    /// грузится лениво при первом Run и кэшируется в runtime. Ответ стримится
    /// по токенам (StreamSink) — `answer`/`output_text` обновляются на лету;
    /// `cancel`-флаг прерывает генерацию (sink → false).
    Llm,

    // ── ACE-Step v1.5 (Нейро → ACE-Step) ──
    /// Хэндл чекпойнта (каталог моделей + 4 опц. override + device/quant/
    /// compute) → `model: Data(Model)`. ComfyUI-стиль: один пункт настройки
    /// модели для Generate-ноды.
    AceStepCheckpoint,
    /// Монолитный text→music: `(model, tags, lyrics, опц. src_latent) →
    /// (audio, latent)`. Зовёт `generate_music` (полный пайплайн LM→enc→
    /// DiT→VAE) в одном worker'е. Режим (text2music/retake/repaint/extend/
    /// edit) выбирается дропдауном в body.
    AceStepGenerate,
    /// `audio: Audio → latent: Data(Latent)` (VAE encoder, 48 kHz →
    /// 25 Hz латент). Вспомогательная: внешнее аудио → `src_latent` Generate.
    AceStepVaeEncode,

    FfmpegPlayer,

    // ── LTX-2.3 (Нейро → LTX Video) ──
    /// Хэндл чекпойнта (пути + device/quant/compute) → `model: Data(Model)`.
    LtxCheckpoint,
    /// `(model, prompt) → (video_encoding, audio_encoding)` (Gemma-3-12B +
    /// перцивер-коннекторы).
    LtxTextEncoder,
    /// `(model, text) → nag: Data(Nag)` — NAG negative-prompt той же Gemma.
    LtxNagPrompt,
    /// `(model, v_enc, a_enc, nag?) → (video_latent, audio_tokens)` —
    /// AvDit distilled stage1 на половинной сетке (8 шагов).
    LtxSamplerStage1,
    /// `(model, video_latent) → video_latent ×2` (spatial latent-upscaler).
    LtxUpscale,
    /// `(model, video_latent, audio_tokens, v_enc, a_enc) → (video_latent,
    /// audio_tokens)` — AvDit stage2 re-noise+refine (3 шага).
    LtxSamplerStage2,
    /// `(model, video_latent) → frames: Data(Frames)` (видео-VAE decode,
    /// превью в body).
    LtxVaeDecode,
    /// `(model, audio_tokens) → audio: Audio` (audio-VAE + вокодер, 48 kHz).
    LtxAudioDecode,
    /// `(frames, audio?) → mp4` (ffmpeg mux, sink).
    LtxVideoSave,
    /// Загрузка изображения для i2v: `→ image_cond: Data(ImageCond)`.
    LtxImage,
    /// Источник видео для v2v: `→ video: Data(VideoInput)`.
    LtxVideoInput,
    /// `(model, v_enc, a_enc, video) → (video_latent, audio_tokens)` —
    /// retake региона [start,end] исходного видео.
    LtxRetake,
    /// `(model, v_enc, a_enc, ref_video) → (video_latent, audio_tokens)` —
    /// IC-LoRA video→video (reference control).
    LtxIcLora,
    /// Источник аудио (речь) для lipdub: `→ audio: Data(AudioInput)`.
    LtxAudioInput,
    /// `(model, v_enc, a_enc, ref_video, audio) → (video_latent, audio_tokens)`
    /// — lipdub (синхрон губ под речь).
    LtxLipdub,
    /// `(model, v_enc, a_enc, audio) → (video_latent, audio_tokens)` —
    /// audio→video (видео под фиксированное входное аудио).
    LtxA2V,
    /// MiniMax-H3: конфиг чекпойнта (DiT + VAE + audio VAE + энкодер + LoRA).
    H3Checkpoint,
    /// MiniMax-H3: промпт + ключевые кадры → кондиционирование Qwen3-VL.
    H3TextEncoder,
    /// MiniMax-H3: пустой AV-латент (ширина/высота/кадры со снапом 17k+5).
    H3EmptyLatentAv,
    /// MiniMax-H3: изображение → ключевой кадр (первый/последний).
    H3Keyframe,
    H3References,
    /// MiniMax-H3: совместный денойзинг видео и звука.
    H3Sampler,
    /// MiniMax-H3: видео-латент → RGB-кадры (ViT-декодер VAE).
    H3VaeDecode,
    /// MiniMax-H3: аудио-латент → стерео 32 кГц (DAC + BigVGAN).
    H3AudioDecode,
    /// MiniMax-H3: кадры + звук → mp4 через ffmpeg.
    H3VideoSave,

    // ── FLUX.1 (Нейро → FLUX) ──
    /// FLUX: `.syn`-бандл/каталог + квант, память. Лёгкий хэндл.
    FluxCheckpoint,
    /// FLUX: промпт → CLIP pooled + T5.
    FluxTextEncoder,
    /// FLUX: размер картинки (пустой латент).
    FluxEmptyLatent,
    /// FLUX: картинка → латент для img2img.
    FluxVaeEncode,
    /// FLUX: денойз (txt2img или img2img).
    FluxSampler,
    /// FLUX: латент → картинка.
    FluxVaeDecode,

    // ── Картинки ──
    /// Картинка из файла → порт `image`.
    ImageLoad,
    /// Картинка → PNG/JPEG на диск.
    ImageSave,
}

impl NodeKind {
    pub const ALL: &'static [NodeKind] = &[
        NodeKind::Number,
        NodeKind::Add,
        NodeKind::Output,
        NodeKind::AudioFile,
        NodeKind::AudioPlayer,
        NodeKind::AudioRecorder,
        NodeKind::Gain,
        NodeKind::Filter,
        NodeKind::Reverb,
        NodeKind::SaveToFile,
        NodeKind::Equalizer6,
        NodeKind::Equalizer10,
        NodeKind::Equalizer20,
        NodeKind::Equalizer30,
        NodeKind::Mixer,
        NodeKind::MarkdownView,
        NodeKind::AsrGigaam,
        NodeKind::TextView,
        NodeKind::OmniVoice,
        NodeKind::VoxCpm2,
        NodeKind::VibeVoice,
        NodeKind::SynCheckpoint,
        NodeKind::SortformerDiarizer,
        NodeKind::Llm,
        NodeKind::AceStepCheckpoint,
        NodeKind::AceStepGenerate,
        NodeKind::AceStepVaeEncode,
        NodeKind::FfmpegPlayer,
        NodeKind::LtxCheckpoint,
        NodeKind::LtxTextEncoder,
        NodeKind::LtxNagPrompt,
        NodeKind::LtxSamplerStage1,
        NodeKind::LtxUpscale,
        NodeKind::LtxSamplerStage2,
        NodeKind::LtxVaeDecode,
        NodeKind::LtxAudioDecode,
        NodeKind::LtxVideoSave,
        NodeKind::LtxImage,
        NodeKind::LtxVideoInput,
        NodeKind::LtxRetake,
        NodeKind::LtxIcLora,
        NodeKind::LtxAudioInput,
        NodeKind::LtxLipdub,
        NodeKind::LtxA2V,
        NodeKind::H3Checkpoint,
        NodeKind::H3TextEncoder,
        NodeKind::H3EmptyLatentAv,
        NodeKind::H3Keyframe,
        NodeKind::H3References,
        NodeKind::H3Sampler,
        NodeKind::H3VaeDecode,
        NodeKind::H3AudioDecode,
        NodeKind::H3VideoSave,
        NodeKind::FluxCheckpoint,
        NodeKind::FluxTextEncoder,
        NodeKind::FluxEmptyLatent,
        NodeKind::FluxVaeEncode,
        NodeKind::FluxSampler,
        NodeKind::FluxVaeDecode,
        NodeKind::ImageLoad,
        NodeKind::ImageSave,
    ];
}

/// Режим биквадного фильтра (зеркалит [`syngui::audio::BiquadMode`]).
/// Хранится отдельным enum'ом, чтобы держать `Serialize/Deserialize` для
/// сохранения в шаблонах и не зависеть от MGUI-крейта в JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    LowPass,
    HighPass,
    BandPass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PostNormMode {
    None,
    Rms,
    #[default]
    Peak,
}

impl PostNormMode {
    pub const ALL: &'static [PostNormMode] =
        &[PostNormMode::None, PostNormMode::Rms, PostNormMode::Peak];
    pub fn label(self) -> &'static str {
        match self {
            PostNormMode::None => "None",
            PostNormMode::Rms => "RMS",
            PostNormMode::Peak => "Peak",
        }
    }
    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.label() == s)
    }
}

impl FilterMode {
    pub const ALL: &'static [FilterMode] = &[
        FilterMode::LowPass,
        FilterMode::HighPass,
        FilterMode::BandPass,
    ];
    pub fn as_index(self) -> usize {
        match self {
            FilterMode::LowPass => 0,
            FilterMode::HighPass => 1,
            FilterMode::BandPass => 2,
        }
    }
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => FilterMode::HighPass,
            2 => FilterMode::BandPass,
            _ => FilterMode::LowPass,
        }
    }
    /// Конвертация в DSP-enum syngui.
    pub fn to_biquad(self) -> syngui::audio::BiquadMode {
        match self {
            FilterMode::LowPass => syngui::audio::BiquadMode::LowPass,
            FilterMode::HighPass => syngui::audio::BiquadMode::HighPass,
            FilterMode::BandPass => syngui::audio::BiquadMode::BandPass,
        }
    }
}

/// Кэш offline-обработки буфера эффект-нодой. Чтобы при evaluate с тем же
/// `input_ptr` и теми же `params` мы возвращали тот же `Arc<AudioBuffer>` —
/// иначе Player видит «новый ptr» через `PrevSource::Buffer(...)` и
/// перезапускается на каждое evaluate (любая смена slider'а в графе).
///
/// `params` — короткий `Vec<f32>` со снимком всех влияющих параметров
/// эффекта (gain_db / mode+cutoff / mix+room). Сравнение `==` по f32-bits
/// — точное (NaN не используются).
#[derive(Clone, Debug)]
pub struct EffectBufferCache {
    pub input_ptr: usize,
    pub params: Vec<f32>,
    pub output: Arc<AudioBuffer>,
}

/// Статус Save-to-file ноды.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    /// Готов к записи; файл ещё не открыт.
    Idle,
    /// Worker-поток пишет в файл.
    Writing,
    /// Запись успешно finalize'нута; путь — итоговый файл.
    Saved(PathBuf),
    /// Ошибка (создание файла, write, finalize). Сообщение для UI.
    Error(String),
}

/// Значение, текущее по проводу. Расширяет старую `f32`-only модель —
/// разные типы портов (Data/Audio) переносят разные payload'ы. Float
/// сохраняется для обратной совместимости с существующими нодами.
///
/// `PartialEq` нужен `RwSignal::set`-у для skip-если-не-изменилось:
/// Float сравниваем по значению, Audio/AudioStream — по `Arc::ptr_eq`
/// (deep PCM compare был бы дорогим, для AudioStream он невозможен —
/// receiver не клонируется).
#[derive(Clone, Default)]
pub enum PortValue {
    #[default]
    Empty,
    Float(f32),
    Audio(Arc<AudioBuffer>),
    AudioStream(Arc<AudioStream>),
    Text(String),
    Data(Arc<DataBlob>),
    VideoStream(Arc<VideoStream>),
}

/// Контейнер для типизированных payload'ов, передаваемых через
/// `PortValue::Data`. Сейчас содержит только ACE-Step тензоры; новые
/// семейства добавляются как варианты `DataBlob` (общая категория) или
/// внутри `AceStepBlob` (специфичные для ACE-Step стадии).
#[derive(Debug)]
pub enum DataBlob {
    /// Тензоры из пайплайна ACE-Step v1.5.
    AceStep(AceStepBlob),
    /// Хэндлы и тензоры пайплайна LTX-2.3 (synaptix).
    Ltx(LtxBlob),
    /// Хэндлы и тензоры пайплайна MiniMax-H3 (synaptix).
    H3(H3Blob),
    /// Хэндл универсальной «Syn Checkpoint»-ноды (LLM/TTS/ASR-модели).
    SynModel(Arc<SynModelHandle>),
    /// Одна картинка — общий тип для FLUX, LTX Image, H3 Keyframe, Image Save.
    Image(Arc<ImageData>),
    /// Хэндлы и тензоры пайплайна FLUX.1 (synaptix).
    Flux(FluxBlob),
}

/// Картинка на проводе: RGB `[3, H, W]` F32 в [0, 1] на CPU плюс готовое
/// RGBA8 для превью, чтобы каждая нода не конвертировала тензор заново.
pub struct ImageData {
    pub tensor: synaptix_core::tensor::Tensor,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    /// Ключ текстуры превью — уникален на каждую картинку, иначе syngui
    /// показал бы закэшированную прежнюю.
    pub key: String,
    /// Файл, из которого картинку загрузили (у сгенерированной — `None`).
    pub source: Option<PathBuf>,
}

impl ImageData {
    /// Из тензора `[3, H, W]` (или `[1, 3, H, W]`) в [0, 1].
    pub fn from_tensor(
        tensor: synaptix_core::tensor::Tensor,
        source: Option<PathBuf>,
    ) -> std::result::Result<Self, String> {
        use synaptix_core::{device::Device, dtype::DType};
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let t = tensor
            .to_device(Device::Cpu)
            .and_then(|t| t.to_dtype(DType::F32))
            .and_then(|t| t.contiguous())
            .map_err(|e| e.to_string())?;
        let d = t.dims().to_vec();
        let (c, h, w) = match d.as_slice() {
            [c, h, w] => (*c, *h, *w),
            [1, c, h, w] => (*c, *h, *w),
            _ => return Err(format!("ожидалась картинка [3, H, W], пришло {d:?}")),
        };
        if c != 3 {
            return Err(format!("ожидалось 3 канала, пришло {c}"));
        }
        let t = t.reshape(vec![3, h, w]).map_err(|e| e.to_string())?;
        let v = t
            .reshape(vec![3 * h * w])
            .and_then(|f| f.to_vec1::<f32>())
            .map_err(|e| e.to_string())?;
        let plane = h * w;
        let mut rgba = Vec::with_capacity(plane * 4);
        let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        for i in 0..plane {
            rgba.push(q(v[i]));
            rgba.push(q(v[plane + i]));
            rgba.push(q(v[2 * plane + i]));
            rgba.push(255);
        }
        Ok(Self {
            tensor: t,
            width: w as u32,
            height: h as u32,
            rgba: Arc::new(rgba),
            key: format!("node-image-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            source,
        })
    }

    /// Загрузить файл (png/jpeg/webp/bmp) в RGB [0, 1].
    pub fn load(path: &std::path::Path) -> std::result::Result<Self, String> {
        let t = synaptix_io::image::load_image(path, synaptix_core::device::Device::Cpu)
            .map_err(|e| e.to_string())?;
        Self::from_tensor(t, Some(path.to_path_buf()))
    }
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

/// Типизированный payload одной из стадий FLUX.1.
#[derive(Debug)]
pub enum FluxBlob {
    Model(Arc<FluxModelHandle>),
    /// CLIP pooled `[1, 768]` + T5 `[1, L, 4096]`.
    Conditioning(Arc<synaptix_image_flux::FluxConditioning>),
    /// Латент: пустой (только размер) или картинки для img2img/декода.
    Latent(Arc<FluxLatent>),
}

/// Конфиг FLUX-чекпойнта. Дешёвый POD — веса грузят потребители через
/// `nodes::flux::shared`.
#[derive(Clone, Debug, PartialEq)]
pub struct FluxModelHandle {
    /// `.syn`-бандл или каталог diffusers.
    pub model_path: PathBuf,
    pub device_idx: usize,
    pub quant_idx: usize,
    pub memory_mode_idx: usize,
    /// Держать трансформер в VRAM между прогонами.
    pub resident: bool,
}

/// Латент FLUX. `tensor == None` — пустой латент от Empty Latent: сэмплер
/// начинает с шума. С тензором — `[1, 16, H/8, W/8]` нормированный латент
/// картинки (VAE Encode) или результат денойза (Sampler).
pub struct FluxLatent {
    pub width: usize,
    pub height: usize,
    pub tensor: Option<synaptix_core::tensor::Tensor>,
}

impl std::fmt::Debug for FluxLatent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.tensor {
            Some(t) => write!(f, "FluxLatent({}x{}, {:?})", self.width, self.height, t.dims()),
            None => write!(f, "FluxLatent({}x{}, пустой)", self.width, self.height),
        }
    }
}

/// Типизированный payload одной из стадий MiniMax-H3.
#[derive(Debug)]
pub enum H3Blob {
    /// Конфиг чекпойнта от H3Checkpoint-ноды.
    Model(Arc<H3ModelHandle>),
    /// Кондиционирование: hidden `[1, L, 5120]` + adaLN-теги токенов.
    Conditioning(Arc<H3Conditioning>),
    /// Пустой AV-латент: только геометрия, шум генерит семплер по seed.
    AvLatent(H3Geometry),
    /// Видео-латент `[1, 24, T', H', W']` + геометрия.
    VideoLatent(Arc<H3VideoLatent>),
    /// Аудио-латент `[1, 32, 2, T]` (40 Гц, стерео).
    AudioLatent(synaptix_core::tensor::Tensor),
    /// Ключевой кадр: RGB `[3,H,W]` в [0,1] + индекс пиксель-кадра.
    Keyframe(Arc<H3Keyframe>),
    /// Декодированные кадры для превью и сохранения.
    Frames(Arc<LtxFrames>),
    /// Референсы Ref2VA: декодированные и нормализованные медиа по порядку.
    Refs(Arc<H3Refs>),
}

/// Конфиг H3-чекпойнта: модель (`.syn`-бандл либо HF-каталог) + энкодер +
/// LoRA + device/quant. Дешёвый POD — веса грузят потребители через
/// `nodes::minimax_h3::shared`.
#[derive(Clone, Debug, PartialEq)]
pub struct H3ModelHandle {
    /// `.syn`-бандл MiniMax-H3 или каталог варианта (FL2VA/Ref2VA).
    pub model_path: PathBuf,
    /// Отдельный `.syn`/каталог энкодера. `None` — энкодер берётся из модели.
    pub encoder_path: Option<PathBuf>,
    pub lora_path: Option<PathBuf>,
    pub lora_strength: f32,
    pub variant_idx: usize,
    pub device_idx: usize,
    pub quant_dit_idx: usize,
    pub quant_enc_idx: usize,
    pub compute_idx: usize,
    pub memory_mode_idx: usize,
    /// Держать DiT в VRAM после прогона (чекбокс «Держать в памяти»).
    pub resident: bool,
}

/// Геометрия генерации: снапнутое число кадров + латентная сетка.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct H3Geometry {
    pub width: usize,
    pub height: usize,
    pub frame_count: usize,
    pub latent_t: usize,
    pub latent_h: usize,
    pub latent_w: usize,
    pub audio_t: usize,
}

/// Выход энкодера Qwen3-VL: hidden-состояния слоя 50 + теги модальности.
pub struct H3Conditioning {
    pub hidden: synaptix_core::tensor::Tensor,
    pub tags: Vec<u8>,
}

impl std::fmt::Debug for H3Conditioning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H3Conditioning({:?})", self.hidden.dims())
    }
}

#[derive(Debug)]
pub struct H3VideoLatent {
    pub tensor: synaptix_core::tensor::Tensor,
    pub geometry: H3Geometry,
}

#[derive(Debug)]
pub struct H3Keyframe {
    pub image: synaptix_core::tensor::Tensor,
    pub frame_index: usize,
    /// Как класть кадр на холст генерации: `false` — растянуть, `true` —
    /// покрыть и обрезать по центру. Применяет сэмплер: размер знает он.
    pub center_crop: bool,
}

/// Строка списка ноды H3 References. `has_audio` — есть ли в файле
/// аудиопоток (узнаётся пробой при добавлении): от него зависит нумерация
/// `<Audio j>`, а она нужна уже при написании промпта, до прогона.
#[derive(Debug, Clone, PartialEq)]
pub struct H3RefEntry {
    pub path: PathBuf,
    pub kind: synaptix_video_minimax_h3::refs::RefKind,
    pub use_audio: bool,
    pub has_audio: bool,
}

/// Декодированные референсы под конкретную геометрию: энкодеру нужны кадры
/// на 2 fps, сэмплеру — латенты тех же самых медиа, поэтому декодируем один
/// раз и отдаём обоим. `frame_count` — под какую длину обрезаны видео и звук.
pub struct H3Refs {
    pub media: Vec<synaptix_video_minimax_h3::refs::RefMedia>,
    pub labels: Vec<String>,
    pub frame_count: usize,
}

impl std::fmt::Debug for H3Refs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H3Refs{{{} шт., {} кадров}}", self.media.len(), self.frame_count)
    }
}

/// Типизированный payload одной из стадий LTX-2.3. Тензоры —
/// `synaptix_core`: живут на устройстве из [`LtxModelHandle`].
#[derive(Debug)]
pub enum LtxBlob {
    /// Конфиг чекпойнта от Checkpoint-ноды. Подмодели грузят потребители
    /// через shared-кэш `nodes::ltx::shared` по ключу из хэндла.
    Model(Arc<LtxModelHandle>),
    /// Видео-контекст `[1, T_txt, 4096]` (Gemma → видео-коннектор).
    VideoEncoding(synaptix_core::tensor::Tensor),
    /// Аудио-контекст `[1, T_txt, 2048]` (Gemma → аудио-коннектор).
    AudioEncoding(synaptix_core::tensor::Tensor),
    /// NAG negative-encoding + параметры (scale/alpha/tau).
    Nag {
        encoding: synaptix_core::tensor::Tensor,
        scale: f32,
        alpha: f32,
        tau: f32,
    },
    /// Видео-латент `[1, 128, F', H', W']` + метаданные сетки.
    VideoLatent(LtxVideoLatent),
    /// Аудио-токены `[1, Fa, 128]`.
    AudioTokens(synaptix_core::tensor::Tensor),
    /// Декодированные RGBA-кадры (выход видео-VAE) для Save/превью.
    Frames(Arc<LtxFrames>),
    /// Image-conditioning для i2v/keyframe: исходное изображение `[3,H,W]` в
    /// [0,1] (resize+VAE-encode на нужную сетку делает Sampler) + сила (1.0 =
    /// полная замена) + пиксель-кадр (`0` = i2v replace кадра 0, `>0` =
    /// keyframe append на пиксель-кадре).
    ImageCond {
        image: synaptix_core::tensor::Tensor,
        strength: f32,
        frame_idx: usize,
    },
    /// Путь к исходному видео для v2v-режимов (retake и т.п.). Декодирование
    /// (ffmpeg→кадры→VAE encode) на нужную сетку делает нода-потребитель.
    VideoInput(PathBuf),
    /// Путь к аудио (речь) для lipdub. Загрузка (16k→mel→audio-VAE encode)
    /// делает нода-потребитель.
    AudioInput(PathBuf),
}

/// Конфиг универсальной «Syn Checkpoint»-ноды: один .syn-бандл (или
/// HF-каталог) + предпочтения device/storage/compute + резидентность.
/// Дешёвый POD в стиле [`LtxModelHandle`] — грузит модель потребитель
/// (LLM/TTS/ASR-нода), маппя индексы «Auto/…» на свои family-опции
/// (Auto = дефолт семейства). Индексы — по спискам опций в
/// `nodes::syn_checkpoint` (DEVICE/STORAGE/COMPUTE_PREF_OPTIONS).
#[derive(Clone, Debug, PartialEq)]
pub struct SynModelHandle {
    pub model_path: PathBuf,
    /// 0=Auto, 1=CUDA, 2=CPU.
    pub device_idx: usize,
    /// 0=Auto, 1=F16, 2=BF16, 3=FP8, 4=NVFP4.
    pub storage_idx: usize,
    /// 0=Auto, 1=F16, 2=BF16, 3=F32.
    pub compute_idx: usize,
    /// «Слотовое» поведение: держать модель в памяти после прогона
    /// (как это всегда делали LLM/TTS/ASR-ноды). false — потребитель
    /// очищает свой слот по завершении прогона, освобождая VRAM.
    pub resident: bool,
}

/// Конфиг LTX-чекпойнта: пути подмоделей + device/quant/compute. Дешёвый
/// POD — открытие mmap и загрузка весов происходят у потребителей.
#[derive(Clone, Debug, PartialEq)]
pub struct LtxModelHandle {
    pub model_path: PathBuf,
    pub gemma_dir: PathBuf,
    pub upscaler_path: Option<PathBuf>,
    pub lora_path: Option<PathBuf>,
    pub lora_strength: f32,
    pub device_idx: usize,
    pub quant_dit_idx: usize,
    pub quant_enc_idx: usize,
    pub compute_idx: usize,
    /// Держать тяжёлые компоненты в VRAM после прогона (см. чекбокс
    /// «Держать в памяти» на Checkpoint-ноде). В ключи weak-кэшей НЕ
    /// входит — резидентность не меняет сами веса.
    pub resident: bool,
}

/// Конфиг ACE-Step чекпойнта: каталог моделей (как CLI `--models`) +
/// опциональные per-bundle override'ы (lm / text-encoder / dit / vae) +
/// device/quant/compute. Дешёвый POD — резолв путей и загрузка весов
/// (через `generate_music` с sequential-drop под 24GB) у Generate-ноды.
/// Резолвинг 4 путей по дефолтным именам бандлов делает потребитель —
/// `nodes::acestep::generate::resolve_paths` (зеркалит CLI `pick`).
#[derive(Clone, Debug, PartialEq)]
pub struct AceStepModelHandle {
    pub models_dir: Option<PathBuf>,
    pub lm_path: Option<PathBuf>,
    pub text_encoder_path: Option<PathBuf>,
    pub dit_path: Option<PathBuf>,
    pub vae_path: Option<PathBuf>,
    pub device_idx: usize,
    pub quant_dit_idx: usize,
    pub quant_enc_idx: usize,
    pub compute_idx: usize,
    /// Держать модели в VRAM после прогона (чекбокс «Держать в памяти»).
    pub resident: bool,
}

/// Видео-латент с метаданными латентной сетки (нужны downstream-нодам для
/// pixel_coords/audio_token_count без пере-вычисления из UI-полей).
#[derive(Clone, Debug)]
pub struct LtxVideoLatent {
    pub tensor: synaptix_core::tensor::Tensor,
    pub fp: usize,
    pub hp: usize,
    pub wp: usize,
    pub fps: f64,
}

/// Кадры RGBA после видео-VAE. Один экземпляр шарится Arc'ами между
/// превью (FramesView) и Save-нодой. TODO: spool на диск для часовых
/// FullHD-видео.
pub struct LtxFrames {
    pub frames: Arc<Vec<Arc<syngui::video::VideoFrame>>>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl std::fmt::Debug for LtxFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LtxFrames({}x{}, {} кадров, {:.1}fps)", self.width, self.height, self.frames.len(), self.fps)
    }
}

/// Типизированный тензор одной из стадий ACE-Step. Распознавание типа
/// внутри `evaluate()` нод даёт читаемые ошибки при mismatch'е, не падая
/// с panic'ом.
#[derive(Debug)]
pub enum AceStepBlob {
    /// Конфиг чекпойнта от ACE-Step Checkpoint-ноды. Подмодели грузит
    /// Generate-нода через `generate_music` по путям из хэндла.
    Model(Arc<AceStepModelHandle>),
    /// Hidden-state encoder'ов: `text_emb` / `lyric_emb` / `timbre_emb`
    /// формы `[B, T, 2048]` (timbre — `[B, 1, 2048]`).
    Hidden(synaptix_core::tensor::Tensor),
    /// Packed conditioning после `Pack`-ноды: `[B, T_cond, 2048]`.
    Conditioning(synaptix_core::tensor::Tensor),
    /// Packed conditioning + длины сегментов `[lyric, timbre, text]` (для
    /// `task_chunk_masks` в DiT при Repaint/Cover).
    ConditioningFull {
        emb: synaptix_core::tensor::Tensor,
        lyric_len: usize,
        timbre_len: usize,
        text_len: usize,
    },
    /// Латент VAE / sampler'а: `[B, 64, T]`. Используется для
    /// `VaeEncode → Sampler`, `Sampler → VaeDecode`, `ArLm → Sampler`.
    Latent(synaptix_core::tensor::Tensor),
    /// Phase 1 CoT-метаданные (bpm/keyscale/timesig/duration/language/caption).
    /// Передаются от `ArLm` к `TextEncoder`/`LyricEncoder`.
    Phase1Metas(Arc<synaptix_music_acestep::tokenizer::Metadata>),
}

impl PartialEq for PortValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PortValue::Empty, PortValue::Empty) => true,
            (PortValue::Float(a), PortValue::Float(b)) => a == b,
            (PortValue::Audio(a), PortValue::Audio(b)) => Arc::ptr_eq(a, b),
            (PortValue::AudioStream(a), PortValue::AudioStream(b)) => Arc::ptr_eq(a, b),
            (PortValue::Text(a), PortValue::Text(b)) => a == b,
            (PortValue::Data(a), PortValue::Data(b)) => Arc::ptr_eq(a, b),
            (PortValue::VideoStream(a), PortValue::VideoStream(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl std::fmt::Debug for PortValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortValue::Empty => f.write_str("Empty"),
            PortValue::Float(v) => write!(f, "Float({v})"),
            PortValue::Audio(buf) => {
                write!(f, "Audio({}sr, {}ch, {}f)", buf.sample_rate, buf.channels, buf.frames())
            }
            PortValue::AudioStream(s) => {
                write!(f, "AudioStream({}sr, {}ch)", s.sample_rate, s.channels)
            }
            PortValue::Text(s) => {
                // Усечение для логов: длинные транскрибаты не захламляют debug.
                let preview: String = s.chars().take(40).collect();
                let suffix = if s.chars().count() > 40 { "…" } else { "" };
                write!(f, "Text({:?}{})", preview, suffix)
            }
            PortValue::VideoStream(s) => {
                write!(f, "VideoStream({}x{}, {:.1}fps)", s.width, s.height, s.fps_estimate)
            }
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(blob) => write!(f, "Data(H3::{blob:?})"),
                DataBlob::AceStep(AceStepBlob::Model(h)) => {
                    write!(f, "Data(AceStep::Model(dir={:?}))", h.models_dir.as_ref().and_then(|p| p.file_name()).unwrap_or_default())
                }
                DataBlob::AceStep(AceStepBlob::Hidden(t)) => {
                    write!(f, "Data(AceStep::Hidden{:?})", t.dims())
                }
                DataBlob::AceStep(AceStepBlob::Conditioning(t)) => {
                    write!(f, "Data(AceStep::Conditioning{:?})", t.dims())
                }
                DataBlob::AceStep(AceStepBlob::ConditioningFull {
                    emb,
                    lyric_len,
                    timbre_len,
                    text_len,
                }) => {
                    write!(
                        f,
                        "Data(AceStep::ConditioningFull{:?} lyric={} timbre={} text={})",
                        emb.dims(),
                        lyric_len,
                        timbre_len,
                        text_len
                    )
                }
                DataBlob::AceStep(AceStepBlob::Latent(t)) => {
                    write!(f, "Data(AceStep::Latent{:?})", t.dims())
                }
                DataBlob::AceStep(AceStepBlob::Phase1Metas(m)) => {
                    write!(
                        f,
                        "Data(AceStep::Phase1Metas{{bpm={:?}, key={:?}, ts={:?}, dur={:?}, lang={:?}}})",
                        m.bpm, m.keyscale, m.timesignature, m.duration, m.language
                    )
                }
                DataBlob::Ltx(LtxBlob::Model(h)) => {
                    write!(f, "Data(Ltx::Model({:?}))", h.model_path.file_name().unwrap_or_default())
                }
                DataBlob::Ltx(LtxBlob::VideoEncoding(t)) => {
                    write!(f, "Data(Ltx::VideoEncoding{:?})", t.dims())
                }
                DataBlob::Ltx(LtxBlob::AudioEncoding(t)) => {
                    write!(f, "Data(Ltx::AudioEncoding{:?})", t.dims())
                }
                DataBlob::Ltx(LtxBlob::Nag { encoding, scale, alpha, tau }) => {
                    write!(f, "Data(Ltx::Nag{:?} s={scale} a={alpha} t={tau})", encoding.dims())
                }
                DataBlob::Ltx(LtxBlob::VideoLatent(l)) => {
                    write!(f, "Data(Ltx::VideoLatent{:?} {}x{}x{} {:.0}fps)", l.tensor.dims(), l.fp, l.hp, l.wp, l.fps)
                }
                DataBlob::Ltx(LtxBlob::AudioTokens(t)) => {
                    write!(f, "Data(Ltx::AudioTokens{:?})", t.dims())
                }
                DataBlob::Ltx(LtxBlob::Frames(fr)) => write!(f, "Data(Ltx::{:?})", fr),
                DataBlob::Ltx(LtxBlob::ImageCond { image, strength, frame_idx }) => {
                    write!(f, "Data(Ltx::ImageCond{:?} s={strength} f={frame_idx})", image.dims())
                }
                DataBlob::Ltx(LtxBlob::VideoInput(p)) => {
                    write!(f, "Data(Ltx::VideoInput({:?}))", p.file_name().unwrap_or_default())
                }
                DataBlob::Ltx(LtxBlob::AudioInput(p)) => {
                    write!(f, "Data(Ltx::AudioInput({:?}))", p.file_name().unwrap_or_default())
                }
                DataBlob::Image(img) => write!(f, "Data({img:?})"),
                DataBlob::Flux(FluxBlob::Model(h)) => {
                    write!(f, "Data(Flux::Model({:?}))", h.model_path.file_name().unwrap_or_default())
                }
                DataBlob::Flux(FluxBlob::Conditioning(c)) => write!(f, "Data(Flux::{c:?})"),
                DataBlob::Flux(FluxBlob::Latent(l)) => write!(f, "Data(Flux::{l:?})"),
                DataBlob::SynModel(h) => {
                    write!(
                        f,
                        "Data(SynModel({:?}, resident={}))",
                        h.model_path.file_name().unwrap_or_default(),
                        h.resident
                    )
                }
            },
        }
    }
}

impl PortValue {
    /// Извлечь Float-компонент. Audio/AudioStream/Empty → 0.0 (compat
    /// для существующих арифметических нод).
    pub fn as_float(&self) -> f32 {
        match self {
            PortValue::Float(v) => *v,
            _ => 0.0,
        }
    }

    /// Извлечь Audio-буфер если порт несёт ready-to-play PCM.
    pub fn as_audio(&self) -> Option<Arc<AudioBuffer>> {
        match self {
            PortValue::Audio(buf) => Some(buf.clone()),
            _ => None,
        }
    }

    /// Извлечь AudioStream если порт несёт live-PCM-стрим.
    pub fn as_audio_stream(&self) -> Option<Arc<AudioStream>> {
        match self {
            PortValue::AudioStream(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Извлечь обёртку DataBlob (общий тензорный payload).
    pub fn as_data(&self) -> Option<Arc<DataBlob>> {
        match self {
            PortValue::Data(b) => Some(b.clone()),
            _ => None,
        }
    }

    /// Извлечь ACE-Step Hidden-тензор (`text_emb` / `lyric_emb` / `timbre_emb`).
    /// Используется encoder-нодами при чтении upstream'а в `Pack`.
    pub fn as_acestep_hidden(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::Hidden(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь ACE-Step Conditioning (packed). Используется Sampler-нодой.
    /// Распознаёт оба варианта: `Conditioning(Tensor)` и `ConditioningFull{...}`.
    pub fn as_acestep_conditioning(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::Conditioning(t)) => Some(t.clone()),
                DataBlob::AceStep(AceStepBlob::ConditioningFull { emb, .. }) => Some(emb.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь ACE-Step Conditioning + длины сегментов. Для `Conditioning(Tensor)`
    /// (legacy без lens) — fallback `(t, 0, dim(1), 0)` чтобы не ломать
    /// существующие workflow'ы. Используется Sampler'ом для логирования и
    /// (опционально) построения task_chunk_masks для Repaint/Cover.
    pub fn as_acestep_conditioning_with_lens(
        &self,
    ) -> Option<(synaptix_core::tensor::Tensor, usize, usize, usize)> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::ConditioningFull {
                    emb,
                    lyric_len,
                    timbre_len,
                    text_len,
                }) => Some((emb.clone(), *lyric_len, *timbre_len, *text_len)),
                DataBlob::AceStep(AceStepBlob::Conditioning(t)) => {
                    let t_cond = t.dims().get(1).copied().unwrap_or(0);
                    Some((t.clone(), 0, t_cond, 0))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь Phase 1 CoT метаданные (`bpm/keyscale/timesignature/...`).
    /// Используется `TextEncoder` и `LyricEncoder` для подмешивания в prompt.
    pub fn as_acestep_phase1_metas(&self) -> Option<Arc<synaptix_music_acestep::tokenizer::Metadata>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::Phase1Metas(m)) => Some(m.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь ACE-Step Latent (`[B, 64, T]`). Используется
    /// `VaeEncode → Sampler`, `Sampler → VaeDecode`, `ArLm → Sampler`.
    pub fn as_acestep_latent(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::Latent(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь хэндл ACE-Step чекпойнта (выход Checkpoint-ноды).
    pub fn as_acestep_model(&self) -> Option<Arc<AceStepModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::AceStep(AceStepBlob::Model(h)) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь хэндл LTX-чекпойнта (выход Checkpoint-ноды).
    pub fn as_ltx_model(&self) -> Option<Arc<LtxModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::Model(h)) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_syn_model(&self) -> Option<Arc<SynModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::SynModel(h) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь хэндл H3-чекпойнта.
    pub fn as_h3_model(&self) -> Option<Arc<H3ModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::Model(h)) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь H3-кондиционирование.
    pub fn as_h3_conditioning(&self) -> Option<Arc<H3Conditioning>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::Conditioning(c)) => Some(c.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь геометрию пустого AV-латента.
    pub fn as_h3_av_latent(&self) -> Option<H3Geometry> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::AvLatent(g)) => Some(*g),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь H3 видео-латент.
    pub fn as_h3_video_latent(&self) -> Option<Arc<H3VideoLatent>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::VideoLatent(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь H3 аудио-латент.
    pub fn as_h3_audio_latent(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::AudioLatent(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь ключевой кадр H3.
    pub fn as_h3_keyframe(&self) -> Option<Arc<H3Keyframe>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::Keyframe(k)) => Some(k.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь референсы H3.
    pub fn as_h3_refs(&self) -> Option<Arc<H3Refs>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::Refs(r)) => Some(r.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь кадры H3 (после VAE-декода).
    pub fn as_h3_frames(&self) -> Option<Arc<LtxFrames>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::H3(H3Blob::Frames(f)) => Some(f.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь картинку (выход FLUX VAE Decode / Image).
    pub fn as_image(&self) -> Option<Arc<ImageData>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Image(i) => Some(i.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_flux_model(&self) -> Option<Arc<FluxModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Flux(FluxBlob::Model(h)) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_flux_conditioning(&self) -> Option<Arc<synaptix_image_flux::FluxConditioning>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Flux(FluxBlob::Conditioning(c)) => Some(c.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_flux_latent(&self) -> Option<Arc<FluxLatent>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Flux(FluxBlob::Latent(l)) => Some(l.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь видео-контекст `[1, T_txt, 4096]`.
    pub fn as_ltx_video_encoding(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::VideoEncoding(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь аудио-контекст `[1, T_txt, 2048]`.
    pub fn as_ltx_audio_encoding(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::AudioEncoding(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь NAG-конфиг (encoding + scale/alpha/tau).
    pub fn as_ltx_nag(&self) -> Option<(synaptix_core::tensor::Tensor, f32, f32, f32)> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::Nag { encoding, scale, alpha, tau }) => {
                    Some((encoding.clone(), *scale, *alpha, *tau))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь видео-латент с метаданными сетки.
    pub fn as_ltx_video_latent(&self) -> Option<LtxVideoLatent> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::VideoLatent(l)) => Some(l.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь аудио-токены `[1, Fa, 128]`.
    pub fn as_ltx_audio_tokens(&self) -> Option<synaptix_core::tensor::Tensor> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::AudioTokens(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь декодированные кадры (выход видео-VAE).
    pub fn as_ltx_frames(&self) -> Option<Arc<LtxFrames>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::Frames(fr)) => Some(fr.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь путь исходного видео (v2v: retake и т.п.).
    pub fn as_ltx_video_input(&self) -> Option<PathBuf> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::VideoInput(p)) => Some(p.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь путь аудио (lipdub-речь).
    pub fn as_ltx_audio_input(&self) -> Option<PathBuf> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::AudioInput(p)) => Some(p.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Извлечь image-conditioning (изображение + сила + пиксель-кадр).
    pub fn as_ltx_image_cond(&self) -> Option<(synaptix_core::tensor::Tensor, f32, usize)> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::Ltx(LtxBlob::ImageCond { image, strength, frame_idx }) => {
                    Some((image.clone(), *strength, *frame_idx))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// True если значение пустое (нет связи или explicit Empty).
    pub fn is_empty(&self) -> bool {
        matches!(self, PortValue::Empty)
    }
}

/// Идентификатор источника, подключённого к плееру в прошлый evaluate.
/// Используется для detection-смены источника без ложных срабатываний:
/// два разных Arc'а из разных арен теоретически могут совпасть как usize,
/// поэтому payload-варианты `Buffer`/`Stream` различаются на уровне типа.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PrevSource {
    /// Нет источника (либо Empty, либо Float — игнорируется).
    None,
    /// Готовый PCM-буфер. `usize` = `Arc::as_ptr(buf) as usize`.
    Buffer(usize),
    /// Live-стрим. `usize` = `Arc::as_ptr(stream) as usize`.
    Stream(usize),
}

/// Long-lived per-instance state для нод, у которых есть «живой» runtime
/// (плеер играет, рекордер пишет, файл загружен). Этот state НЕ
/// пересоздаётся при каждом `evaluate_graph` — иначе плеер «переключался»
/// бы каждый тик create_effect.
///
/// Хранится в `NodeInstance.runtime: Arc<Mutex<NodeRuntime>>`, чтобы:
/// 1. NodeInstance остался Clone (Arc дёшев),
/// 2. UI и eval могли читать состояние без race с background-thread'ами
///    cpal-стримов (lock'и короткие, RwSignal-обвязка реактивна).
pub enum NodeRuntime {
    /// Нода без runtime-состояния (Number/Add/Output/etc.).
    None,
    /// Источник аудио из файла. PCM грузится async (std::thread + signal).
    AudioFile {
        buffer: RwSignal<Option<Arc<AudioBuffer>>>,
        loaded_path: RwSignal<Option<PathBuf>>,
        load_error: RwSignal<Option<String>>,
    },
    /// Аудио-плеер. `player` стартует на нажатие Play — до этого None.
    AudioPlayer {
        /// Активный плеер, либо None (Stop / ещё не запущен).
        player: Option<AudioPlayer>,
        /// True пока плеер играет (активен и не paused).
        is_playing: RwSignal<bool>,
        /// True когда плеер на паузе.
        is_paused: RwSignal<bool>,
        /// Прогресс [0..1] — обновляется animate-callback'ом.
        progress: RwSignal<f32>,
        /// Громкость [0..2], 1.0 = native level.
        volume: RwSignal<f32>,
        /// Идентификатор последнего accepted источника. Если в evaluate
        /// приходит другой kind/ptr — input сменился: drop player, обновить
        /// pcm_view/pending_stream, остаться в Stop (явный Play требуется).
        last_input_kind: PrevSource,
        /// Снимок текущего источника-буфера для отрисовки waveform в карточке.
        /// Для streaming-режима — `None` (длительность неизвестна, рендерим
        /// «LIVE» бейдж вместо bars).
        pcm_view: RwSignal<Option<Arc<AudioBuffer>>>,
        /// Захваченный receiver streaming-источника. Заполняется executor'ом
        /// в момент detection `PrevSource::Stream(...)` (через
        /// `AudioStream::take_receiver`). На Play-клик плеер забирает его
        /// и стартует через `AudioPlayer::start_streaming`.
        pending_stream: Mutex<Option<std::sync::mpsc::Receiver<Vec<f32>>>>,
        /// Sample rate streaming-источника (snapshotится в момент detection).
        /// Используется при `start_streaming(rx, sr)`.
        stream_sample_rate: u32,
        /// True пока подключённый source — это `PortValue::AudioStream(...)`.
        /// Body использует флаг для UI-веток (LIVE-бейдж, скрыть seek/duration).
        is_streaming: RwSignal<bool>,
    },
    /// Линейный gain. Effect-нода: проксирует AudioStream → AudioStream
    /// с применённым множителем (или Audio(buf) → Audio(buf) offline).
    /// Worker-поток создаётся executor'ом при detection нового streaming
    /// источника; для buffer-режима — никаких потоков.
    Gain {
        /// Текущий gain в децибелах (UI-side state, читается worker'ом
        /// через `Arc<AtomicU32>`-bits в `live`).
        gain_db: RwSignal<f32>,
        /// Снимок live-параметра для realtime worker thread'а. Записывается
        /// UI при изменении slider'а, читается worker'ом sample-by-sample.
        live_gain: Arc<std::sync::atomic::AtomicU32>,
        /// Detection последнего источника, чтобы не пересоздавать worker
        /// каждое evaluate.
        last_input_kind: PrevSource,
        /// Текущий output stream (производитель worker'а). Sender дропается
        /// вместе с worker'ом, downstream видит RecvError → останавливается.
        out_stream: Option<Arc<AudioStream>>,
        /// Активный handle worker thread'а. Используется только для drop
        /// (join не нужен — поток сам завершится при RecvError).
        worker: Option<std::thread::JoinHandle<()>>,
        /// Кэш offline-обработки буфера: `(input_ptr, params) → output`.
        /// Используется только в buffer-режиме (Audio(buf)).
        buffer_cache: Arc<Mutex<Option<EffectBufferCache>>>,
    },
    /// Biquad-фильтр LP/HP/BP над AudioStream / AudioBuffer.
    Filter {
        mode: RwSignal<FilterMode>,
        cutoff_hz: RwSignal<f32>,
        /// Снимки live-параметров для worker'а (без локов в hot-loop).
        live_mode: Arc<std::sync::atomic::AtomicU8>,
        live_cutoff: Arc<std::sync::atomic::AtomicU32>,
        /// Нужно ли пересчитать коэффициенты (UI ставит при изменении).
        coeffs_dirty: Arc<std::sync::atomic::AtomicBool>,
        biquad: Arc<Mutex<Biquad>>,
        last_input_kind: PrevSource,
        out_stream: Option<Arc<AudioStream>>,
        worker: Option<std::thread::JoinHandle<()>>,
        buffer_cache: Arc<Mutex<Option<EffectBufferCache>>>,
    },
    /// Schroeder-реверб над AudioStream / AudioBuffer.
    Reverb {
        mix: RwSignal<f32>,
        room: RwSignal<f32>,
        live_mix: Arc<std::sync::atomic::AtomicU32>,
        live_room: Arc<std::sync::atomic::AtomicU32>,
        params_dirty: Arc<std::sync::atomic::AtomicBool>,
        reverb: Arc<Mutex<SchroederReverb>>,
        last_input_kind: PrevSource,
        out_stream: Option<Arc<AudioStream>>,
        worker: Option<std::thread::JoinHandle<()>>,
        buffer_cache: Arc<Mutex<Option<EffectBufferCache>>>,
    },
    /// Запись AudioStream в WAV-файл.
    SaveToFile {
        /// Путь к файлу (UI редактирует TextField).
        path: RwSignal<String>,
        /// True пока worker пишет.
        is_writing: RwSignal<bool>,
        /// Прогресс записи (секунды). Обновляется worker'ом, Reactive в UI.
        written_seconds: RwSignal<f64>,
        /// Размер файла в байтах (живой счётчик).
        written_bytes: RwSignal<u64>,
        /// Текущий статус.
        status: RwSignal<SaveStatus>,
        /// Перехваченный executor'ом receiver upstream'а + метаданные.
        /// Single-sub: после `take()` body получает receiver и стартует
        /// worker, в pending_rx остаётся None до следующего detection.
        pending_rx: Arc<Mutex<Option<(std::sync::mpsc::Receiver<Vec<f32>>, u32, u16)>>>,
        /// True если в pending_rx есть данные. RwSignal — для реактивного
        /// disabled-state Record-кнопки (нет источника → серая кнопка).
        has_upstream: RwSignal<bool>,
        /// Cooperative cancel — UI выставляет в true, worker видит и
        /// finalize'ит файл. Дроп worker JoinHandle тоже сработает (Sender
        /// внутри будет жить, но recv увидит cancel и выйдет).
        cancel: Arc<std::sync::atomic::AtomicBool>,
        /// Активный worker.
        worker: Option<std::thread::JoinHandle<()>>,
        last_input_kind: PrevSource,
    },
    /// Graphic-эквалайзер: каскад N peaking-Biquad'ов с независимыми gain'ами.
    /// Один runtime-вариант для всех 4 нод (Equalizer6/10/20/30); количество
    /// полос задаётся `n_bands` при создании.
    Equalizer {
        /// Количество полос (6/10/20/30).
        n_bands: usize,
        /// UI-сигналы gain'ов в дБ (длина = n_bands). Range [-18..+18].
        gains_db: Vec<RwSignal<f32>>,
        /// Snapshot live-параметров для worker'а (без локов в hot-loop). f32-bits.
        live_gains: Vec<Arc<std::sync::atomic::AtomicU32>>,
        /// Нужен ли пересчёт коэффициентов (UI ставит при изменении).
        coeffs_dirty: Arc<std::sync::atomic::AtomicBool>,
        /// `[band][channel]` biquad'ы. Заполняется при detection нового стрима
        /// под фактический channel-count входа.
        biquads: Arc<Mutex<Vec<Vec<Biquad>>>>,
        last_input_kind: PrevSource,
        out_stream: Option<Arc<AudioStream>>,
        worker: Option<std::thread::JoinHandle<()>>,
        buffer_cache: Arc<Mutex<Option<EffectBufferCache>>>,
    },
    /// Микшер N → 1: суммирует AudioStream'ы (или offline-буферы) с
    /// per-канальными gain'ами. Все векторы длины [`MIXER_MAX_INPUTS`];
    /// `n_inputs` определяет «активный» префикс — UI скрывает остальные
    /// porty, executor игнорирует входы за пределами.
    Mixer {
        /// Количество активных входов (2..=`MIXER_MAX_INPUTS`).
        n_inputs: RwSignal<usize>,
        /// Per-канальные gain'ы в дБ (длина = `MIXER_MAX_INPUTS`). Range [-24..+12].
        gains_db: Vec<RwSignal<f32>>,
        /// Snapshot live-параметров для worker'а (без локов в hot-loop).
        live_gains: Vec<Arc<std::sync::atomic::AtomicU32>>,
        /// Последняя зафиксированная сигнатура входов (по индексам). Если
        /// сигнатура изменилась — пересоздаём worker. Длина = `MIXER_MAX_INPUTS`.
        last_input_kinds: Vec<PrevSource>,
        /// Текущий output-stream (производитель worker'а).
        out_stream: Option<Arc<AudioStream>>,
        /// Активный handle worker thread'а (drop = сигнал downstream'у).
        worker: Option<std::thread::JoinHandle<()>>,
        /// Кэш offline-сложения для Buffer-режима.
        buffer_cache: Arc<Mutex<Option<EffectBufferCache>>>,
    },
    /// Рекордер с микрофона. Lifecycle полностью в [`RecordingSession`]:
    /// она держит cpal-recorder, vis_handle, audio_stream, elapsed-таймер
    /// и (благодаря `decode_on_stop=true`) автоматически декодирует WAV
    /// в `AudioBuffer` после `stop()`. Снаружи остаётся только выбор
    /// устройства, потому что это персистится отдельно через UI dropdown.
    AudioRecorder {
        session: RecordingSession,
        /// Выбранное устройство (None = default).
        device: RwSignal<Option<String>>,
    },
    /// Декоративная markdown-нода: рендерит `MarkdownView` (Preview) либо
    /// `MarkdownEditor` (Edit) — переключение через ContextMenu. Без портов
    /// и без executor-логики; включена в `evaluate_graph` как no-op.
    MarkdownView {
        /// Markdown-source. Используется и в Preview, и в Edit (общий буфер).
        content: RwSignal<String>,
        /// True — body показывает MarkdownEditor; false — MarkdownView.
        edit_mode: RwSignal<bool>,
        /// True — `TransformBox` рисует selection-рамку с resize-handle'ами
        /// и принимает drag (8 handle'ов на гранях/углах).
        resize_mode: RwSignal<bool>,
        /// Явный размер body (без header'а карточки). Двусторонне
        /// синхронизирован с `TransformBox.size_signal`.
        size: RwSignal<Size>,
    },
    /// Runtime ASR-ноды на базе GigaAM.
    ///
    /// Жизненный цикл:
    /// - UI выбирает `.syn` + dtype/device; пока модель не загружена,
    ///   `transcriber` хранит `None`.
    /// - На нажатии Play body-builder читает текущий `PortValue::Audio` на
    ///   входе из `NodeEditorCtx.values`, спавнит worker-thread. Если
    ///   `transcriber=None` или `loaded_cfg` не совпадает с текущими
    ///   значениями — `Transcriber::load(...)` (блокирующая, ~секунды).
    /// - На успехе worker записывает текст в `output_text` через
    ///   `run_on_main_thread`, бампает `text_version` (триггер Reactive-
    ///   rebuild MultilineTextEdit). Текст становится также output-портом —
    ///   `AsrGigaamExec::evaluate` копирует `output_text.get()` в
    ///   `PortValue::Text(...)`.
    /// - Пользователь может редактировать текст в editor'е; правки уходят
    ///   downstream через тот же output_text (single source of truth).
    /// Runtime editable text-view ноды (Text in → Text out passthrough).
    ///
    /// `last_input` отслеживает последнюю строку пришедшую с input-порта.
    /// Когда executor видит новое значение (отличается от `last_input`) —
    /// перезаписывает `output_text` и бампает `text_version` (Reactive в
    /// body пересоздаёт `MultilineTextEdit` с новым initial-text).
    /// Ручные правки пользователя не трогают `last_input`, поэтому повторный
    /// evaluate с тем же upstream-значением их не затрёт.
    ///
    /// `size` + `resize_mode` управляют размером карточки через `TransformBox`
    /// (паттерн `MarkdownView`). По дефолту 280×360 — вертикалка под одну
    /// колонку текста; toggle handle'ов — через ContextMenu.
    TextView {
        output_text: RwSignal<String>,
        text_version: RwSignal<u32>,
        last_input: Arc<Mutex<Option<String>>>,
        /// True — `TransformBox` рисует selection-рамку с resize-handle'ами.
        resize_mode: RwSignal<bool>,
        /// Явный размер body. Двусторонне синхронизирован с
        /// `TransformBox.size_signal`.
        size: RwSignal<Size>,
    },
    AsrGigaam {
        /// Путь к выбранному `.syn` bundle'у. None = не выбран.
        model_path: RwSignal<Option<PathBuf>>,
        /// Индекс в `nodes::asr_gigaam::DEVICE_OPTIONS` (0=CPU, 1=GPU auto).
        device_idx: RwSignal<usize>,
        /// Индекс в `nodes::asr_gigaam::STORAGE_OPTIONS`.
        storage_idx: RwSignal<usize>,
        /// Индекс в `nodes::asr_gigaam::COMPUTE_OPTIONS`.
        compute_idx: RwSignal<usize>,
        /// Закэшированный экземпляр модели. Lock короткий: только swap
        /// при load/unload; transcribe держит lock на всё время инференса.
        ///
        /// Lock — `syngui::core::sync::Mutex` (на native — `std::sync::Mutex`).
        transcriber: Arc<Mutex<Option<synaptix::facade::asr::Transcriber>>>,
        /// Конфиг, с которым модель сейчас загружена. Если перед Play любое
        /// из (model_path/device/storage/compute) изменилось — модель
        /// перезагружается.
        loaded_cfg: Arc<Mutex<Option<AsrLoadedCfg>>>,
        /// Идёт ли в данный момент загрузка или транскрибация.
        running: RwSignal<bool>,
        /// Сообщение об ошибке последней операции. None = OK.
        error: RwSignal<Option<String>>,
        /// Имя загруженной модели — для UI-баджа после успешной загрузки.
        loaded_name: RwSignal<Option<String>>,
        /// Текст транскрибата. Редактируется пользователем; output порта
        /// тоже отражает текущее значение этого сигнала.
        output_text: RwSignal<String>,
        /// Bump'ится после транскрибации → Reactive пересоздаёт
        /// `MultilineTextEdit` с новым initial text. На user-edit НЕ
        /// меняется → курсор не прыгает на каждое нажатие клавиши.
        text_version: RwSignal<u32>,
        /// Detect нового input audio (как у Gain/Reverb). Сейчас не
        /// используется напрямую, но оставлен для будущей авто-инвалидации
        /// текста при смене upstream-источника.
        last_input_kind: PrevSource,
    },
    /// Runtime TTS-ноды на базе OmniVoice (Qwen3-1B LM + Higgs-Audio codec +
    /// discrete mask-diffusion).
    ///
    /// Жизненный цикл (аналог AsrGigaam):
    /// - UI выбирает `.syn` bundle + dropdown'ы device/storage/compute +
    ///   параметры генерации; пока pipeline не загружен — `pipeline` хранит `None`.
    /// - На Play body-builder читает текущий `PortValue::Text` с порта `text`,
    ///   опционально `PortValue::Audio` с `ref_audio`, `PortValue::Text` с
    ///   `ref_text` (fallback на `ref_text_field`). Mode выбирается автоматически:
    ///     - ref_audio есть → `Clone(VoiceClonePrompt)`,
    ///     - иначе instruct непуст → `Design{instruct}`,
    ///     - иначе → `Auto`.
    /// - Worker-thread: при несовпадении `loaded_cfg` грузит pipeline
    ///   (`OmniVoicePipeline::from_syn` ~30 c на CPU), затем `pipeline.synthesize`,
    ///   результат в `output_buf` + bump `output_version` через
    ///   `run_on_main_thread` — Reactive в downstream-нодах перечитывает порт.
    /// - Output порт `audio`: `OmniVoiceExec::evaluate` копирует
    ///   `output_buf` в `PortValue::Audio(Arc<AudioBuffer>)`.
    OmniVoice {
        /// Путь к выбранному `.syn` bundle (single-file model). `start`
        /// передаёт его как `bundle_path` для `OmniVoicePipeline::from_syn`.
        model_path: RwSignal<Option<PathBuf>>,
        /// Индекс в `nodes::omnivoice::DEVICE_OPTIONS` (0=CPU, 1=GPU auto).
        device_idx: RwSignal<usize>,
        /// Индекс в `nodes::omnivoice::STORAGE_OPTIONS`. Пробрасывается в
        /// `OmniVoicePipeline::from_syn` через `synaptix_core::dtype::DType` —
        /// влияет на квантизацию LM (NVFP4/MXFP8) и инвалидирует
        /// кэш через `OmniLoadedCfg`.
        storage_idx: RwSignal<usize>,
        /// Индекс в `nodes::omnivoice::COMPUTE_OPTIONS`. Compute-dtype для
        /// LM/codec forward + квантизованных GEMM dequant'ов.
        compute_idx: RwSignal<usize>,
        /// Voice-Design instruct (textfield). Используется когда нет
        /// ref_audio: mode становится `Design{instruct}`.
        instruct: RwSignal<String>,
        /// Fallback-textfield для ref_text, когда соответствующий порт
        /// не подключён.
        ref_text_field: RwSignal<String>,
        /// Язык ref-аудио (подсказка для phonemization). Default — "ru".
        language: RwSignal<String>,
        /// Число шагов diffusion-сэмплера. Default = 32 (range [8, 64]).
        num_step: RwSignal<u32>,
        /// CFG / guidance_scale. Default = 2.0 (range [0.0, 5.0]).
        guidance_scale: RwSignal<f32>,
        /// Timestep-shift. Default = 0.1 (range [0.0, 1.0]).
        t_shift: RwSignal<f32>,
        /// Speech speed multiplier. Default = 1.0 (range [0.5, 2.0]).
        speed: RwSignal<f32>,
        /// Random seed для SplitMix64 в sampler'е. Default = 0.
        seed: RwSignal<u64>,
        /// Закэшированный pipeline. Lock короткий: только swap при load;
        /// `synthesize` держит lock на всё время инференса (sync API).
        pipeline: Arc<Mutex<Option<synaptix::facade::tts::TtsPipeline>>>,
        /// Конфиг, с которым pipeline сейчас загружен. Если любое из полей
        /// snapshot'а изменилось перед Play — pipeline сбрасывается и
        /// перезагружается.
        loaded_cfg: Arc<Mutex<Option<OmniLoadedCfg>>>,
        /// Идёт ли в данный момент загрузка или синтез.
        running: RwSignal<bool>,
        /// Сообщение об ошибке последней операции. None = OK.
        error: RwSignal<Option<String>>,
        /// Имя каталога загруженной модели — для UI после load.
        loaded_name: RwSignal<Option<String>>,
        /// Синтезированный AudioBuffer (24 kHz mono PCM). `evaluate`
        /// читает через lock и пробрасывает в `PortValue::Audio`.
        output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        /// Bump'ится worker'ом после успешного синтеза → подписанный
        /// graph-executor пересчитывает `audio`-порт. На user-edit
        /// настроек НЕ меняется.
        output_version: RwSignal<u32>,
    },
    VoxCpm2 {
        model_path: RwSignal<Option<PathBuf>>,
        device_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        prompt_text_field: RwSignal<String>,
        cfg_value: RwSignal<f32>,
        n_timesteps: RwSignal<u32>,
        max_len: RwSignal<u32>,
        seed: RwSignal<u64>,
        pipeline: Arc<Mutex<Option<synaptix_tts_voxcpm::VoxCpmPipeline>>>,
        loaded_cfg: Arc<Mutex<Option<VoxCpm2LoadedCfg>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        output_version: RwSignal<u32>,
    },
    VibeVoice {
        model_path: RwSignal<Option<PathBuf>>,
        device_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        script_field: RwSignal<String>,
        cfg_value: RwSignal<f32>,
        ddpm_steps: RwSignal<u32>,
        max_length_times: RwSignal<f32>,
        seed: RwSignal<u64>,
        pipeline: Arc<Mutex<Option<synaptix_tts_vibevoice::VibeVoicePipeline>>>,
        loaded_cfg: Arc<Mutex<Option<VibeVoiceLoadedCfg>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        progress: RwSignal<u32>,
        output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        output_version: RwSignal<u32>,
    },
    /// Runtime ноды диаризации (NVIDIA Streaming Sortformer v2.1).
    ///
    /// Жизненный цикл — близкая копия `AsrGigaam`:
    /// - UI выбирает `.syn` bundle + dropdown'ы device/storage/compute +
    ///   threshold + allow_overlap;
    /// - На Play worker лениво грузит `Diarizer` (если loaded_cfg изменился),
    ///   читает аудио на входе, вызывает `diarize_pcm` → DiarizationResult,
    ///   пишет JSON в `output_json` (идёт в порт) и pretty-text в `output_pretty`;
    /// - `text_version` бампается через `run_on_main_thread`, Reactive в
    ///   body пересоздаёт статус-строку.
    /// Универсальная «Syn Checkpoint»-нода: не грузит веса, публикует
    /// [`SynModelHandle`] (путь + предпочтения + резидентность) в порт
    /// `model`. Потребители — слот-семейства (LLM/TTS/ASR/диаризация).
    SynCheckpoint {
        /// .syn-бандл или HF-каталог (LLM-нода умеет оба).
        model_path: RwSignal<Option<PathBuf>>,
        /// Индексы по спискам `nodes::syn_checkpoint::*_PREF_OPTIONS`.
        device_idx: RwSignal<usize>,
        storage_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        /// Держать модель в памяти после прогона (слотовое поведение).
        resident: RwSignal<bool>,
        /// Стабильный Arc, пока параметры не менялись (ptr_eq downstream).
        handle_cache: Arc<Mutex<Option<Arc<SynModelHandle>>>>,
    },
    SortformerDiarizer {
        model_path: RwSignal<Option<PathBuf>>,
        device_idx: RwSignal<usize>,
        storage_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        /// Порог бинаризации probability → активность (0.1..0.9).
        threshold: RwSignal<f32>,
        /// Разрешать overlap'ы спикеров (Sortformer multi-label).
        allow_overlap: RwSignal<bool>,
        /// Закэшированный экземпляр модели.
        diarizer: Arc<Mutex<Option<synaptix::facade::diarization::Diarizer>>>,
        loaded_cfg: Arc<Mutex<Option<SortformerLoadedCfg>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        /// Human-readable текст диаризации для UI карточки.
        output_pretty: RwSignal<String>,
        /// Machine-readable JSON, отдаётся в порт `out`.
        output_json: RwSignal<String>,
        text_version: RwSignal<u32>,
        last_input_kind: PrevSource,
    },
    /// Runtime LLM-ноды на нативном synaptix-стеке (Qwen3 / Qwen3-Next-Hybrid).
    ///
    /// Жизненный цикл (аналог `OmniVoice`, движок — synaptix):
    /// - UI выбирает каталог модели (HF-дир или `.syn`) + dropdown'ы
    ///   device/quant/compute + system-fallback + max_tokens/temperature/seed;
    ///   пока pipeline не загружен — `pipeline` хранит `None`.
    /// - На Run worker лениво грузит `LlmPipeline` (детект арх по `config.json`),
    ///   собирает чат-промпт ([system?, user(prompt)]) через `ChatTemplate`,
    ///   стримит ответ `generate_streaming` — sink декодит накопленные токены и
    ///   пишет их в `output_text` на лету (TextView обновляется в реальном
    ///   времени). `cancel` прерывает генерацию (sink → false).
    /// - `output_text` отдаётся в порт `answer`; `text_version` бампается для
    ///   Reactive-rebuild статус-строки body.
    Llm {
        /// Путь к модели: HF-каталог (config.json + safetensors) или `.syn`.
        model_path: RwSignal<Option<PathBuf>>,
        /// Индекс в `nodes::llm::DEVICE_OPTIONS` (0=CUDA, 1=CPU).
        device_idx: RwSignal<usize>,
        /// Индекс в `nodes::llm::QUANT_OPTIONS` (none/nvfp4/mxfp8).
        quant_idx: RwSignal<usize>,
        /// Индекс в `nodes::llm::COMPUTE_OPTIONS` (bf16/f16/f32).
        compute_idx: RwSignal<usize>,
        /// System-промпт fallback. Порт `system` переопределяет его, если
        /// подключён и непуст.
        system_prompt: RwSignal<String>,
        /// Размер контекста (max_seq): preallocated KV-буфер + RoPE capacity.
        context: RwSignal<u32>,
        /// Thinking-режим (Qwen3 `<think>`): true → enable_thinking в чат-шаблоне.
        think: RwSignal<bool>,
        /// Лимит новых токенов генерации.
        max_tokens: RwSignal<u32>,
        /// Температура сэмплинга (0 = greedy).
        temperature: RwSignal<f32>,
        /// top-k (0 = выкл).
        top_k: RwSignal<u32>,
        /// top-p nucleus (1.0 = выкл).
        top_p: RwSignal<f32>,
        /// min-p (0.0 = выкл).
        min_p: RwSignal<f32>,
        /// Штраф повторов (1.0 = выкл).
        repetition_penalty: RwSignal<f32>,
        /// Seed RNG сэмплинга.
        seed: RwSignal<u64>,
        /// Закэшированный pipeline (enum Qwen3/Hybrid). Lock короткий на
        /// swap при load; generate держит lock на всё время инференса.
        pipeline: Arc<Mutex<Option<crate::pages::node_editor::nodes::llm::LlmPipeline>>>,
        /// Конфиг, с которым pipeline сейчас загружен — для invalidation.
        loaded_cfg: Arc<Mutex<Option<LlmLoadedCfg>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        /// Сгенерированный ответ. Стримится по токенам; output порта `answer`
        /// отражает текущее значение.
        output_text: RwSignal<String>,
        /// Bump после каждого инкремента стрима → Reactive пересоздаёт
        /// статус-строку.
        text_version: RwSignal<u32>,
        /// Флаг отмены — sink возвращает false → генерация прерывается.
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },

    // ── ACE-Step v1.5 (Нейро → ACE-Step) ──
    //
    AceStepVaeEncode {
        device_idx: RwSignal<usize>,
        storage_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        /// Размер chunk'а encode в секундах (для длинных файлов).
        chunk_seconds: RwSignal<f32>,
        /// Overlap в секундах между chunk'ами (для cross-fade).
        overlap_seconds: RwSignal<f32>,
        loaded_cfg: Arc<Mutex<Option<AceStepLoadedCfg>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        /// `latent [B, 64, T_latent]`.
        output_buf: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
        /// Cooperative-cancel: воркер проверяет его между окнами энкода.
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    /// ACE-Step Checkpoint: каталог моделей + 4 опц. override'а + device/
    /// quant/compute. Веса не грузит — публикует хэндл в порт `model`.
    AceStepCheckpoint {
        models_dir: RwSignal<Option<PathBuf>>,
        lm_path: RwSignal<Option<PathBuf>>,
        text_encoder_path: RwSignal<Option<PathBuf>>,
        dit_path: RwSignal<Option<PathBuf>>,
        vae_path: RwSignal<Option<PathBuf>>,
        device_idx: RwSignal<usize>,
        quant_dit_idx: RwSignal<usize>,
        quant_enc_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        /// Держать модели в VRAM после прогона.
        resident: RwSignal<bool>,
        /// Кэш Arc-хэндла: новый Arc только при смене параметров (иначе
        /// ptr_eq в PortValue видел бы «новое» значение каждый evaluate).
        handle_cache: Arc<Mutex<Option<Arc<AceStepModelHandle>>>>,
    },
    /// ACE-Step Generate (монолит): весь text→music пайплайн в одном worker'е
    /// через `generate_music`. Входы (model/tags/lyrics/src_latent) — порты;
    /// в body только режим-дропдаун и слайдеры. Выходы — `audio` + `latent`.
    AceStepGenerate {
        /// 0=text2music, 1=retake, 2=repaint, 3=extend, 4=edit, 5=cover, 6=extract.
        mode_idx: RwSignal<usize>,
        /// Preset Auto/Turbo/Base/SFT (Auto → detect по имени DiT-бандла).
        preset: RwSignal<SamplerPreset>,
        /// Длительность сек; `-1` = Авто (Phase-1 CoT сам предсказывает).
        duration_seconds: RwSignal<f32>,
        infer_steps: RwSignal<u32>,
        cfg_scale: RwSignal<f32>,
        flow_match_shift: RwSignal<f32>,
        seed: RwSignal<u64>,
        // ── AR LM (генерация audio-кодов) ──
        temperature: RwSignal<f32>,
        top_p: RwSignal<f32>,
        top_k: RwSignal<u32>,
        min_p: RwSignal<f32>,
        /// AR-CFG scale на logits LM (`final = uncond + cfg·(cond − uncond)`).
        lm_cfg_scale: RwSignal<f32>,
        /// Phase-1 CoT (bpm/keyscale/timesig/duration/language) перед Phase-2.
        use_cot: RwSignal<bool>,
        /// AR (5Hz LM) вкл/выкл. `false` (no-codes) штатно ТОЛЬКО для turbo;
        /// base/sft вне распределения и аварийно падают → гард в `start()`.
        use_ar: RwSignal<bool>,
        /// BPM-оверрайд (`0` = N/A → модель сама предсказывает в Phase-1 CoT).
        bpm: RwSignal<u32>,
        /// Тональность: индекс в `KEYSCALE_OPTIONS` (`0` = N/A).
        keyscale_idx: RwSignal<usize>,
        /// Размер такта: индекс в `TIMESIG_OPTIONS` (`0` = N/A).
        timesig_idx: RwSignal<usize>,
        /// Нормализация выходного PCM (Peak/RMS/None) — общий `PostNormMode`.
        norm_mode: RwSignal<PostNormMode>,
        // ── DCW (пост-коррекция латента) ──
        enable_dcw: RwSignal<bool>,
        dcw_mode: RwSignal<DcwModeOption>,
        dcw_scaler: RwSignal<f32>,
        dcw_high_scaler: RwSignal<f32>,
        dcw_wavelet: RwSignal<DcwWaveletOption>,
        dcw_preset: RwSignal<DcwPresetOption>,
        // ── Режим-зависимые (retake/repaint/extend) ──
        /// retake: дисперсия вариации src-латента [0..1] (0 = no-op).
        retake_variance: RwSignal<f32>,
        retake_seed: RwSignal<u64>,
        /// repaint/extend: регион [start,end] сек (-1 в end = до конца).
        repaint_start_sec: RwSignal<f32>,
        repaint_end_sec: RwSignal<f32>,
        repaint_strength: RwSignal<f32>,
        // ── Режим edit (flow-edit) ──
        /// усечение расписания: денойз только шаги [n_min, n_max] (доли [0..1]).
        edit_n_min: RwSignal<f32>,
        edit_n_max: RwSignal<f32>,
        // ── Режим extract ──
        /// Дорожка: индекс в `TRACK_OPTIONS` (`0` = vocals).
        track_idx: RwSignal<usize>,
        // ── Статус + выходы ──
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        /// Готовый mono/stereo PCM для порта `audio`.
        output_buf_audio: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        /// Финальный латент `[1, 64, T]` для порта `latent` (цепочка retake).
        output_buf_latent: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },

    /// Универсальный видеоплеер: файл (ffmpeg `VideoPlayer`) ИЛИ кадры из
    /// памяти (`FramesView` + `AudioPlayer`). Режим выбирается по наличию
    /// входа `frames` (память приоритетнее файла) — позволяет ставить плеер
    /// на выход LTX-пайплайна вместо/параллельно Video Save.
    FfmpegPlayer {
        player: Arc<Mutex<Option<Arc<Mutex<syngui::video::VideoPlayer>>>>>,
        current_path: RwSignal<Option<PathBuf>>,
        load_error: RwSignal<Option<String>>,
        is_playing: RwSignal<bool>,
        is_paused: RwSignal<bool>,
        progress: RwSignal<f32>,
        duration: RwSignal<f32>,
        position: RwSignal<f32>,
        volume: RwSignal<f32>,
        hwaccel_idx: RwSignal<usize>,
        size: RwSignal<Size>,
        video_out: Arc<Mutex<Option<Arc<VideoStream>>>>,
        audio_out: Arc<Mutex<Option<Arc<AudioStream>>>>,
        out_version: RwSignal<u32>,
        /// Кадры из памяти (вход `frames`, напр. LTX VAE Decode). Если есть —
        /// memory-режим: `FramesView` ведёт видео, файл игнорируется.
        frames_in: Arc<Mutex<Option<Arc<LtxFrames>>>>,
        /// Аудио из памяти (вход `audio`, напр. LTX Audio Decode) для
        /// memory-режима.
        audio_in: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        /// Активный memory-аудио плеер (играет `audio_in` синхронно с
        /// FramesView).
        mem_audio: Arc<Mutex<Option<AudioPlayer>>>,
        /// Бамп при смене `frames_in` → пересоздание FramesView в body.
        preview_version: RwSignal<u32>,
    },

    LtxCheckpoint {
        model_path: RwSignal<Option<PathBuf>>,
        gemma_dir: RwSignal<Option<PathBuf>>,
        upscaler_path: RwSignal<Option<PathBuf>>,
        lora_path: RwSignal<Option<PathBuf>>,
        lora_strength: RwSignal<f32>,
        device_idx: RwSignal<usize>,
        quant_dit_idx: RwSignal<usize>,
        quant_enc_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        /// Держать DiT в VRAM после прогона (внимание: на 24 ГБ рядом с
        /// VAE-decode может не хватить памяти — осознанный опт-ин).
        resident: RwSignal<bool>,
        /// Кэш Arc-хэндла: новый Arc только при смене параметров (иначе
        /// ptr_eq в PortValue видел бы «новое» значение каждый evaluate).
        handle_cache: Arc<Mutex<Option<Arc<LtxModelHandle>>>>,
    },
    LtxTextEncoder {
        prompt_field: RwSignal<String>,
        /// Держать Gemma загруженной после encode (strong-ref в gemma_keep).
        /// ВЫКЛ по умолчанию: 12-24GB Gemma + DiT-активации не влезают в
        /// 24GB VRAM.
        keep_gemma: RwSignal<bool>,
        /// Strong-ref на Gemma при keep_gemma=true (продлевает жизнь
        /// Weak-кэша `nodes::ltx::shared::GEMMA_CACHE`).
        gemma_keep: Arc<Mutex<Option<Arc<synaptix_llm_gemma3::pipeline::GemmaPipeline>>>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        v_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxNagPrompt {
        prompt_field: RwSignal<String>,
        scale: RwSignal<f32>,
        alpha: RwSignal<f32>,
        tau: RwSignal<f32>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxSamplerStage1 {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        fps_idx: RwSignal<usize>,
        /// 0 = детерминированный дефолт synaptix (парити с CLI).
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxUpscale {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<LtxVideoLatent>>>,
        output_version: RwSignal<u32>,
    },
    LtxSamplerStage2 {
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxVaeDecode {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        frames_out: Arc<Mutex<Option<Arc<LtxFrames>>>>,
        output_version: RwSignal<u32>,
        /// Бамп = перезапуск превью в body (replay декодированных кадров).
        preview_version: RwSignal<u32>,
    },
    LtxAudioDecode {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        output_version: RwSignal<u32>,
    },
    LtxVideoSave {
        path: RwSignal<String>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        status: RwSignal<SaveStatus>,
    },
    LtxImage {
        image_path: RwSignal<Option<PathBuf>>,
        strength: RwSignal<f32>,
        /// Пиксель-кадр conditioning: 0 = i2v (replace кадра 0), >0 = keyframe
        /// (append на этом пиксель-кадре).
        frame_idx: RwSignal<u32>,
        error: RwSignal<Option<String>>,
        /// Кэш: (путь, Arc<image-тензор [3,H,W]>) — грузим один раз, новый
        /// Arc только при смене пути (ptr_eq стабилен в PortValue).
        cache: Arc<Mutex<Option<(PathBuf, synaptix_core::tensor::Tensor)>>>,
    },
    LtxVideoInput {
        video_path: RwSignal<Option<PathBuf>>,
    },
    LtxRetake {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        fps_idx: RwSignal<usize>,
        retake_start: RwSignal<f32>,
        retake_end: RwSignal<f32>,
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxIcLora {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        fps_idx: RwSignal<usize>,
        downscale: RwSignal<u32>,
        ref_strength: RwSignal<f32>,
        /// Control-препроцессинг ref-видео: 0=none, 1=canny, 2=depth.
        control_idx: RwSignal<usize>,
        canny_low: RwSignal<f32>,
        canny_high: RwSignal<f32>,
        depth_model_path: RwSignal<Option<PathBuf>>,
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    LtxAudioInput {
        audio_path: RwSignal<Option<PathBuf>>,
    },
    LtxLipdub {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        fps_idx: RwSignal<usize>,
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    H3Checkpoint {
        model_path: RwSignal<Option<PathBuf>>,
        encoder_path: RwSignal<Option<PathBuf>>,
        lora_path: RwSignal<Option<PathBuf>>,
        lora_strength: RwSignal<f32>,
        variant_idx: RwSignal<usize>,
        device_idx: RwSignal<usize>,
        quant_dit_idx: RwSignal<usize>,
        quant_enc_idx: RwSignal<usize>,
        compute_idx: RwSignal<usize>,
        memory_mode_idx: RwSignal<usize>,
        /// Держать DiT в VRAM после прогона.
        resident: RwSignal<bool>,
        handle_cache: Arc<Mutex<Option<Arc<H3ModelHandle>>>>,
    },
    H3TextEncoder {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<H3Conditioning>>>>,
        output_version: RwSignal<u32>,
    },
    H3EmptyLatentAv {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        /// Индекс в `minimax_h3::latent::ASPECT_OPTIONS`: 0 = «Свободно»,
        /// дальше фиксированные пропорции (16:9, 9:16, …) — ширина и
        /// высота связаны, портретные варианты дают вертикальное видео.
        aspect_idx: RwSignal<usize>,
    },
    H3Keyframe {
        path: RwSignal<Option<PathBuf>>,
        frame_slot_idx: RwSignal<usize>,
        resize_idx: RwSignal<usize>,
        image: Arc<Mutex<Option<Arc<H3Keyframe>>>>,
        error: RwSignal<Option<String>>,
        output_version: RwSignal<u32>,
    },
    H3References {
        /// Порядок значим: он задаёт номера меток и RoPE-часы раскладки.
        items: RwSignal<Vec<H3RefEntry>>,
        /// Индекс в `minimax_h3::references::IMAGE_SIZE_OPTIONS`.
        image_size_idx: RwSignal<usize>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<H3Refs>>>>,
        output_version: RwSignal<u32>,
    },
    H3Sampler {
        steps: RwSignal<u32>,
        cfg_scale: RwSignal<f32>,
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<Arc<H3VideoLatent>>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    H3VaeDecode {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        frames: Arc<Mutex<Option<Arc<LtxFrames>>>>,
        preview_version: RwSignal<u32>,
        output_version: RwSignal<u32>,
    },
    H3AudioDecode {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        buffer: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        output_version: RwSignal<u32>,
    },
    H3VideoSave {
        path: RwSignal<Option<PathBuf>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        saved: RwSignal<Option<String>>,
        preview: Arc<Mutex<Option<(Arc<LtxFrames>, Option<Arc<AudioBuffer>>)>>>,
        preview_version: RwSignal<u32>,
    },
    LtxA2V {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        duration_seconds: RwSignal<f32>,
        fps_idx: RwSignal<usize>,
        seed: RwSignal<u64>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        v_out: Arc<Mutex<Option<LtxVideoLatent>>>,
        a_out: Arc<Mutex<Option<synaptix_core::tensor::Tensor>>>,
        output_version: RwSignal<u32>,
    },
    FluxCheckpoint {
        model_path: RwSignal<Option<PathBuf>>,
        device_idx: RwSignal<usize>,
        /// Индекс в `nodes::flux::QUANT_OPTIONS`.
        quant_idx: RwSignal<usize>,
        /// Индекс в `nodes::flux::MEMORY_MODE_OPTIONS`.
        memory_mode_idx: RwSignal<usize>,
        resident: RwSignal<bool>,
        handle_cache: Arc<Mutex<Option<Arc<FluxModelHandle>>>>,
    },
    FluxTextEncoder {
        /// Индекс в `nodes::flux::SEQ_LEN_OPTIONS` (0 — по модели).
        seq_len_idx: RwSignal<usize>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        loaded_name: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<synaptix_image_flux::FluxConditioning>>>>,
        output_version: RwSignal<u32>,
    },
    FluxEmptyLatent {
        width: RwSignal<u32>,
        height: RwSignal<u32>,
        /// Индекс в `nodes::flux::latent::ASPECT_OPTIONS`, 0 — свободно.
        aspect_idx: RwSignal<usize>,
    },
    FluxVaeEncode {
        /// Как вписать картинку в размер латента со входа `size`:
        /// 0 — растянуть, 1 — покрыть и обрезать по центру.
        resize_idx: RwSignal<usize>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<FluxLatent>>>>,
        output_version: RwSignal<u32>,
    },
    FluxSampler {
        steps: RwSignal<u32>,
        guidance: RwSignal<f32>,
        seed: RwSignal<u64>,
        /// Доля шума для img2img (латент с картинкой на входе).
        denoise: RwSignal<f32>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        progress_pct: RwSignal<f32>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        out: Arc<Mutex<Option<Arc<FluxLatent>>>>,
        output_version: RwSignal<u32>,
    },
    FluxVaeDecode {
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        out: Arc<Mutex<Option<Arc<ImageData>>>>,
        output_version: RwSignal<u32>,
    },
    ImageLoad {
        path: RwSignal<Option<PathBuf>>,
        error: RwSignal<Option<String>>,
        /// Загруженная картинка — пока путь тот же, отдаётся тот же `Arc`.
        cache: Arc<Mutex<Option<Arc<ImageData>>>>,
    },
    ImageSave {
        path: RwSignal<Option<PathBuf>>,
        running: RwSignal<bool>,
        error: RwSignal<Option<String>>,
        saved: RwSignal<Option<String>>,
        preview: Arc<Mutex<Option<Arc<ImageData>>>>,
        preview_version: RwSignal<u32>,
    },
}

impl NodeRuntime {
    /// Сигнал ошибки последнего запуска ноды — для сбора итога прогона
    /// (`run_controls::RunOutcome`). None у нод без понятия «ошибка запуска»
    /// (реактивные ноды, чекпойнты); `load_error` AudioFile/FfmpegPlayer —
    /// про загрузку файла, не про прогон, поэтому сюда не входит.
    pub fn run_error_signal(&self) -> Option<RwSignal<Option<String>>> {
        use NodeRuntime as R;
        match self {
            R::AsrGigaam { error, .. }
            | R::OmniVoice { error, .. }
            | R::VoxCpm2 { error, .. }
            | R::VibeVoice { error, .. }
            | R::SortformerDiarizer { error, .. }
            | R::Llm { error, .. }
            | R::AceStepVaeEncode { error, .. }
            | R::AceStepGenerate { error, .. }
            | R::LtxTextEncoder { error, .. }
            | R::LtxNagPrompt { error, .. }
            | R::LtxSamplerStage1 { error, .. }
            | R::LtxUpscale { error, .. }
            | R::LtxSamplerStage2 { error, .. }
            | R::LtxVaeDecode { error, .. }
            | R::LtxAudioDecode { error, .. }
            | R::LtxVideoSave { error, .. }
            | R::LtxImage { error, .. }
            | R::LtxRetake { error, .. }
            | R::LtxIcLora { error, .. }
            | R::LtxLipdub { error, .. }
            | R::LtxA2V { error, .. }
            | R::H3TextEncoder { error, .. }
            | R::H3Keyframe { error, .. }
            | R::H3References { error, .. }
            | R::H3Sampler { error, .. }
            | R::H3VaeDecode { error, .. }
            | R::H3AudioDecode { error, .. }
            | R::H3VideoSave { error, .. }
            | R::FluxTextEncoder { error, .. }
            | R::FluxVaeEncode { error, .. }
            | R::FluxSampler { error, .. }
            | R::FluxVaeDecode { error, .. }
            | R::ImageLoad { error, .. }
            | R::ImageSave { error, .. } => Some(*error),
            _ => None,
        }
    }

    /// Пустые ОБЯЗАТЕЛЬНЫЕ пути моделей ноды — pre-check агентского прогона
    /// (`pipeline_run::prepare`): запускать граф с незаполненным чекпойнтом
    /// бессмысленно, нода упадёт. ACE-Step здесь не проверяется — его
    /// чекпойнт умеет fallback на глобальные настройки бандлов; LoRA и
    /// LTX-upscaler опциональны; encoder H3 подхватывается из бандла.
    /// Пуст ли `upscaler_path` LTX-чекпойнта. Отдельно от
    /// [`missing_model_paths`](Self::missing_model_paths): upscaler нужен
    /// только графам со стадией `LtxUpscale`, и требовать его от всех
    /// (retake, a2v, lipdub) было бы ложной тревогой.
    pub fn ltx_upscaler_missing(&self) -> bool {
        matches!(
            self,
            NodeRuntime::LtxCheckpoint { upscaler_path, .. }
                if upscaler_path.get_untracked().is_none()
        )
    }

    pub fn missing_model_paths(&self) -> Vec<&'static str> {
        use NodeRuntime as R;
        let empty = |p: &RwSignal<Option<PathBuf>>| p.get_untracked().is_none();
        match self {
            R::LtxCheckpoint {
                model_path,
                gemma_dir,
                ..
            } => {
                let mut v = Vec::new();
                if empty(model_path) {
                    v.push("model_path");
                }
                if empty(gemma_dir) {
                    v.push("gemma_dir");
                }
                v
            }
            R::H3Checkpoint { model_path, .. }
            | R::FluxCheckpoint { model_path, .. }
            | R::SynCheckpoint { model_path, .. }
            | R::Llm { model_path, .. }
            | R::AsrGigaam { model_path, .. }
            | R::OmniVoice { model_path, .. }
            | R::VoxCpm2 { model_path, .. }
            | R::VibeVoice { model_path, .. }
            | R::SortformerDiarizer { model_path, .. } => {
                if empty(model_path) {
                    vec!["model_path"]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Сигнал прогресса воркера (0..1) — для живой карточки прогона в чате.
    /// У части нод он есть, но не пишется (ACE-Step Generate; Lipdub пишет
    /// вехи 0.1/0.5/1.0) — потребитель должен переживать «застывший» 0.
    pub fn run_progress_signal(&self) -> Option<RwSignal<f32>> {
        use NodeRuntime as R;
        match self {
            R::AceStepGenerate { progress_pct, .. }
            | R::LtxTextEncoder { progress_pct, .. }
            | R::LtxSamplerStage1 { progress_pct, .. }
            | R::LtxSamplerStage2 { progress_pct, .. }
            | R::LtxVaeDecode { progress_pct, .. }
            | R::LtxVideoSave { progress_pct, .. }
            | R::LtxRetake { progress_pct, .. }
            | R::LtxIcLora { progress_pct, .. }
            | R::LtxLipdub { progress_pct, .. }
            | R::LtxA2V { progress_pct, .. }
            | R::H3Sampler { progress_pct, .. }
            | R::FluxSampler { progress_pct, .. } => Some(*progress_pct),
            _ => None,
        }
    }

    /// Cooperative-cancel флаг воркера — для отмены прогона извне
    /// (`run_controls::cancel_active_run`). У нод без флага (в т.ч.
    /// ACE-Step Generate) воркер не отменяется и досчитывает до конца.
    pub fn run_cancel_flag(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        use NodeRuntime as R;
        match self {
            R::SaveToFile { cancel, .. }
            | R::Llm { cancel, .. }
            | R::LtxSamplerStage1 { cancel, .. }
            | R::LtxSamplerStage2 { cancel, .. }
            | R::LtxRetake { cancel, .. }
            | R::LtxIcLora { cancel, .. }
            | R::LtxLipdub { cancel, .. }
            | R::LtxA2V { cancel, .. }
            | R::AceStepVaeEncode { cancel, .. }
            | R::H3Sampler { cancel, .. }
            | R::FluxSampler { cancel, .. } => Some(cancel.clone()),
            _ => None,
        }
    }
}

/// Снимок конфига, с которым в данный момент загружен `Transcriber` ASR-ноды.
/// Сравнивается перед каждым Play: если изменилось — модель сбрасывается и
/// перезагружается.
#[derive(Clone, Debug, PartialEq)]
pub struct AsrLoadedCfg {
    pub model_path: PathBuf,
    pub device_idx: usize,
    pub storage_idx: usize,
    pub compute_idx: usize,
}

/// Снимок конфига, с которым в данный момент загружен `OmniVoicePipeline`
/// для OmniVoice-ноды. Сравнивается перед каждым Play: если изменилось —
/// pipeline сбрасывается и перезагружается. `storage_idx`/`compute_idx`
/// напрямую пробрасываются в `OmniVoicePipeline::from_syn` через
/// `synaptix_core::dtype::DType`.
#[derive(Clone, Debug, PartialEq)]
pub struct OmniLoadedCfg {
    pub bundle_path: PathBuf,
    pub device_idx: usize,
    pub storage_idx: usize,
    pub compute_idx: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoxCpm2LoadedCfg {
    pub bundle_path: PathBuf,
    pub device_idx: usize,
    pub compute_idx: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VibeVoiceLoadedCfg {
    pub bundle_path: PathBuf,
    pub device_idx: usize,
    pub compute_idx: usize,
}

/// Снимок конфига, с которым загружен synaptix `LlmPipeline` LLM-ноды.
/// Сравнивается перед каждым Run: если путь/device/quant/compute изменились —
/// pipeline сбрасывается и перезагружается.
#[derive(Clone, Debug, PartialEq)]
pub struct LlmLoadedCfg {
    pub model_path: PathBuf,
    pub device_idx: usize,
    pub quant_idx: usize,
    pub compute_idx: usize,
}

/// Снимок конфига, с которым в данный момент загружен `Diarizer` ноды
/// `SortformerDiarizer`. Если изменилось — модель сбрасывается и
/// перезагружается при следующем Play.
#[derive(Clone, Debug, PartialEq)]
pub struct SortformerLoadedCfg {
    pub model_path: PathBuf,
    pub device_idx: usize,
    pub storage_idx: usize,
    pub compute_idx: usize,
}

/// Снимок конфига, с которым в данный момент загружены модели ACE-Step
/// нод. Один общий тип для всех 7 «модельных» нод (TextEncoder /
/// LyricEncoder / TimbreEncoder / VaeEncode / VaeDecode / ArLm / Sampler) —
/// поля идентичны (path + device + storage_dtype + compute_dtype),
/// а конкретный тип модели в shared registry разруливается отдельным
/// кэшем per-нода-тип.
#[derive(Clone, Debug, PartialEq)]
pub struct AceStepLoadedCfg {
    pub model_path: PathBuf,
    pub device_idx: usize,
    pub storage_idx: usize,
    pub compute_idx: usize,
}


impl Default for NodeRuntime {
    fn default() -> Self {
        NodeRuntime::None
    }
}

impl std::fmt::Debug for NodeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRuntime::None => f.write_str("NodeRuntime::None"),
            NodeRuntime::AudioFile { .. } => f.write_str("NodeRuntime::AudioFile{..}"),
            NodeRuntime::AudioPlayer { .. } => f.write_str("NodeRuntime::AudioPlayer{..}"),
            NodeRuntime::AudioRecorder { .. } => f.write_str("NodeRuntime::AudioRecorder{..}"),
            NodeRuntime::Gain { .. } => f.write_str("NodeRuntime::Gain{..}"),
            NodeRuntime::Filter { .. } => f.write_str("NodeRuntime::Filter{..}"),
            NodeRuntime::Reverb { .. } => f.write_str("NodeRuntime::Reverb{..}"),
            NodeRuntime::SaveToFile { .. } => f.write_str("NodeRuntime::SaveToFile{..}"),
            NodeRuntime::Equalizer { n_bands, .. } => write!(f, "NodeRuntime::Equalizer{{n_bands={n_bands}}}"),
            NodeRuntime::Mixer { n_inputs, .. } => {
                let n = n_inputs.get_untracked();
                write!(f, "NodeRuntime::Mixer{{n_inputs={n}}}")
            }
            NodeRuntime::MarkdownView { .. } => f.write_str("NodeRuntime::MarkdownView{..}"),
            NodeRuntime::AsrGigaam { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "-".to_string());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::AsrGigaam{{name={name}, running={run}}}")
            }
            NodeRuntime::TextView { output_text, .. } => {
                let len = output_text.get_untracked().chars().count();
                write!(f, "NodeRuntime::TextView{{chars={len}}}")
            }
            NodeRuntime::OmniVoice { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "-".to_string());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::OmniVoice{{name={name}, running={run}}}")
            }
            NodeRuntime::VibeVoice { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "—".into());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::VibeVoice{{name={name}, running={run}}}")
            }
            NodeRuntime::VoxCpm2 { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "-".to_string());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::VoxCpm2{{name={name}, running={run}}}")
            }
            NodeRuntime::SortformerDiarizer { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "-".to_string());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::SortformerDiarizer{{name={name}, running={run}}}")
            }
            NodeRuntime::SynCheckpoint { model_path, resident, .. } => {
                let p = model_path
                    .get_untracked()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "-".to_string());
                let r = resident.get_untracked();
                write!(f, "NodeRuntime::SynCheckpoint{{path={p}, resident={r}}}")
            }
            NodeRuntime::Llm { loaded_name, running, .. } => {
                let name = loaded_name.get_untracked().unwrap_or_else(|| "-".to_string());
                let run = running.get_untracked();
                write!(f, "NodeRuntime::Llm{{name={name}, running={run}}}")
            }
            NodeRuntime::AceStepVaeEncode { running, .. } => {
                write!(f, "NodeRuntime::AceStepVaeEncode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::FfmpegPlayer { is_playing, current_path, .. } => {
                let path = current_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::FfmpegPlayer{{path={path}, playing={}}}", is_playing.get_untracked())
            }
            NodeRuntime::LtxCheckpoint { model_path, .. } => {
                let path = model_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::LtxCheckpoint{{path={path}}}")
            }
            NodeRuntime::LtxTextEncoder { running, .. } => {
                write!(f, "NodeRuntime::LtxTextEncoder{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxNagPrompt { running, .. } => {
                write!(f, "NodeRuntime::LtxNagPrompt{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxSamplerStage1 { running, .. } => {
                write!(f, "NodeRuntime::LtxSamplerStage1{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxUpscale { running, .. } => {
                write!(f, "NodeRuntime::LtxUpscale{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxSamplerStage2 { running, .. } => {
                write!(f, "NodeRuntime::LtxSamplerStage2{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxVaeDecode { running, .. } => {
                write!(f, "NodeRuntime::LtxVaeDecode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxAudioDecode { running, .. } => {
                write!(f, "NodeRuntime::LtxAudioDecode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxVideoSave { running, .. } => {
                write!(f, "NodeRuntime::LtxVideoSave{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxImage { image_path, .. } => {
                let p = image_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::LtxImage{{path={p}}}")
            }
            NodeRuntime::LtxVideoInput { video_path } => {
                let p = video_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::LtxVideoInput{{path={p}}}")
            }
            NodeRuntime::LtxRetake { running, .. } => {
                write!(f, "NodeRuntime::LtxRetake{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxIcLora { running, .. } => {
                write!(f, "NodeRuntime::LtxIcLora{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxAudioInput { audio_path } => {
                let p = audio_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::LtxAudioInput{{path={p}}}")
            }
            NodeRuntime::LtxLipdub { running, .. } => {
                write!(f, "NodeRuntime::LtxLipdub{{running={}}}", running.get_untracked())
            }
            NodeRuntime::LtxA2V { running, .. } => {
                write!(f, "NodeRuntime::LtxA2V{{running={}}}", running.get_untracked())
            }
            NodeRuntime::H3Checkpoint { model_path, .. } => {
                let p = model_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::H3Checkpoint{{model={p}}}")
            }
            NodeRuntime::H3TextEncoder { running, .. } => {
                write!(f, "NodeRuntime::H3TextEncoder{{running={}}}", running.get_untracked())
            }
            NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, .. } => {
                write!(
                    f,
                    "NodeRuntime::H3EmptyLatentAv{{{}x{}, {:.1}s}}",
                    width.get_untracked(),
                    height.get_untracked(),
                    duration_seconds.get_untracked()
                )
            }
            NodeRuntime::H3Keyframe { path, .. } => {
                let p = path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::H3Keyframe{{path={p}}}")
            }
            NodeRuntime::H3References { items, .. } => {
                write!(f, "NodeRuntime::H3References{{{} шт.}}", items.get_untracked().len())
            }
            NodeRuntime::FluxCheckpoint { model_path, .. } => {
                let p = model_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_default();
                write!(f, "NodeRuntime::FluxCheckpoint{{path={p}}}")
            }
            NodeRuntime::FluxTextEncoder { running, .. } => {
                write!(f, "NodeRuntime::FluxTextEncoder{{running={}}}", running.get_untracked())
            }
            NodeRuntime::FluxEmptyLatent { width, height, .. } => {
                write!(f, "NodeRuntime::FluxEmptyLatent{{{}x{}}}", width.get_untracked(), height.get_untracked())
            }
            NodeRuntime::FluxVaeEncode { running, .. } => {
                write!(f, "NodeRuntime::FluxVaeEncode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::FluxSampler { running, steps, .. } => {
                write!(
                    f,
                    "NodeRuntime::FluxSampler{{running={}, steps={}}}",
                    running.get_untracked(),
                    steps.get_untracked()
                )
            }
            NodeRuntime::FluxVaeDecode { running, .. } => {
                write!(f, "NodeRuntime::FluxVaeDecode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::ImageLoad { path, .. } => {
                let p = path.get_untracked().map(|p| p.display().to_string()).unwrap_or_default();
                write!(f, "NodeRuntime::ImageLoad{{path={p}}}")
            }
            NodeRuntime::ImageSave { path, .. } => {
                let p = path.get_untracked().map(|p| p.display().to_string()).unwrap_or_default();
                write!(f, "NodeRuntime::ImageSave{{path={p}}}")
            }
            NodeRuntime::H3Sampler { running, steps, .. } => {
                write!(
                    f,
                    "NodeRuntime::H3Sampler{{running={}, steps={}}}",
                    running.get_untracked(),
                    steps.get_untracked()
                )
            }
            NodeRuntime::H3VaeDecode { running, .. } => {
                write!(f, "NodeRuntime::H3VaeDecode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::H3AudioDecode { running, .. } => {
                write!(f, "NodeRuntime::H3AudioDecode{{running={}}}", running.get_untracked())
            }
            NodeRuntime::H3VideoSave { saved, .. } => {
                write!(f, "NodeRuntime::H3VideoSave{{saved={:?}}}", saved.get_untracked())
            }
            NodeRuntime::AceStepCheckpoint { models_dir, .. } => {
                let p = models_dir.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::AceStepCheckpoint{{dir={p}}}")
            }
            NodeRuntime::AceStepGenerate { running, mode_idx, .. } => {
                write!(f, "NodeRuntime::AceStepGenerate{{running={}, mode={}}}", running.get_untracked(), mode_idx.get_untracked())
            }
        }
    }
}

/// Визуальные настройки ноды, общие для всех `NodeKind`. Применяются в
/// `node_view::node_card` поверх дефолтного стиля карточки: `tint`
/// блендится с базовым фоном (`#2A2D38`, alpha ~0.25), `shadow=false`
/// добавляет класс `.no-shadow` (MSS `box-shadow: none`).
///
/// Сериализация в шаблоны — через `templates::model::NodeStyleData`
/// (Color не имеет serde-derive, hex-строка в JSON — общий паттерн
/// с `FieldValueData::Color`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeStyle {
    /// Оттенок ноды. `None` — без окраски, базовый тёмный фон.
    pub tint: Option<Color>,
    /// Drop-shadow карточки. По умолчанию `true`.
    pub shadow: bool,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self { tint: None, shadow: true }
    }
}

/// Конкретный экземпляр ноды в графе. `pos` — RwSignal, чтобы drag двигал
/// единственную ноду без полного рейбилда Stack'а нод.
///
/// PartialEq — по `id`. Для сигнала `RwSignal<Vec<NodeInstance>>` нужен
/// `PartialEq<Vec>`, а тот делегирует поэлементно.
#[derive(Clone)]
pub struct NodeInstance {
    pub id: NodeId,
    pub kind: NodeKind,
    pub pos: RwSignal<Point>,
    /// Реальный layout-bounds карточки в position-pass-системе координат
    /// (= global-without-render-transform). Заполняется `SizeReport`-обёрткой
    /// в `node_view::view`. Используется `port_world_pos` для вычисления
    /// координат портов в той же системе, что и `Event::MouseMove`-pos
    /// после `accumulated_event_transform` — иначе drag-wire рисуется
    /// со смещением, равным parent_pos PanZoomViewport (sidebar+nav offsets).
    ///
    /// До первого measure — `Rect::zero()`. CSS-аналог: `ResizeObserver`
    /// + `getBoundingClientRect()` в DOM-API.
    pub bounds: RwSignal<Rect>,
    /// Поля ноды — индекс по имени из `NodeKindMeta.fields`. Хранится как
    /// `Arc<Mutex<...>>` чтобы Clone у Vec<NodeInstance> был дешёвый и общая
    /// карта значений переживала перестройку Vec в RwSignal.
    pub fields: Arc<Mutex<std::collections::HashMap<&'static str, FieldValue>>>,
    /// Long-lived runtime-state (player/recorder/loaded buffer). Для нод
    /// без runtime — `NodeRuntime::None`. Каждая нода имеет собственный
    /// runtime — лишний lock contention отсутствует, потому что обращения
    /// идут только из UI thread.
    pub runtime: Arc<Mutex<NodeRuntime>>,
    /// Визуальный стиль карточки (tint + shadow). Per-instance, читается
    /// `node_view::node_card` через `Reactive`.
    pub style: RwSignal<NodeStyle>,
    /// Включена ли нода в граф. `false` → executor пропускается в
    /// `evaluate_graph`, карточка рендерится с классом `.disabled`.
    pub enabled: RwSignal<bool>,
    /// Секундомер ноды. Стартует/останавливается эффектом в
    /// `state::NodeEditorCtx::new` по флипу `busy_signal` этой ноды —
    /// значит меряет и глобальный Run, и per-node Play. Показывается
    /// бейджем в шапке карточки (`timing::node_timer_badge`). Не
    /// персистится: измерение принадлежит сессии, а не графу.
    pub timing: super::timing::Stopwatch,
}

impl PartialEq for NodeInstance {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Соединение между двумя портами разных нод.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Connection {
    pub from_node: NodeId,
    pub from_port: &'static str,
    pub to_node: NodeId,
    pub to_port: &'static str,
}

/// Состояние «провод тащится от порта». Конец — текущая позиция курсора
/// в world-coords.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingWire {
    pub from_node: NodeId,
    pub from_port: &'static str,
    pub from_kind: PortKind,
    /// world-coord конца провода (под курсором).
    pub current: Point,
}
