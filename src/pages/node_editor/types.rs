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
    /// Editable text-display нода. Text in → Text out (passthrough).
    /// Body — `MultilineTextEdit`. При смене upstream Text заменяет
    /// содержимое (bump `text_version` → Reactive rebuild). Ручные правки
    /// пользователя сохраняются до следующего изменения upstream'а.
    TextView,

    // ── ACE-Step v1.5 (Нейро → ACE-Step) ──

    // ── YuE2 (Нейро → YuE2) ──

    FfmpegPlayer,

    // ── LTX-2.3 (Нейро → LTX Video) ──

    // ── FLUX.1 (Нейро → FLUX) ──

    // ── FLUX.2 (Нейро → FLUX.2) ──

    // ── Qwen-Image (Нейро → Qwen-Image) ──

    // ── Qwen-Image 2.1 (Нейро → Qwen-Image 2.1) ──

    // ── SDXL (Нейро → SDXL) ──

    // ── Картинки ──
    /// Картинка из файла → порт `image`.
    ImageLoad,
    /// Картинка → PNG/JPEG на диск.
    ImageSave,

    // ── Ноды ядра synaptix-flow (генерация через службу, монтаж ffmpeg) ──
    FlowText,
    FlowFile,
    FlowFormat,
    FlowLines,
    FlowList,
    FlowLlm,
    FlowImage,
    FlowVideo,
    FlowMusic,
    FlowSpeak,
    FlowTranscribe,
    FlowConcat,
    FlowMux,
    FlowFrame,
    FlowTrim,
    FlowStill,
    // Стадии моделей в службе: картинки (FLUX.1/FLUX.2/Qwen-Image/SDXL) и MiniMax-H3.
    FlowImageModel,
    FlowImageEncode,
    FlowImageRefs,
    FlowImageLatent,
    FlowImageToLatent,
    FlowImageSample,
    FlowImageDecode,
    FlowH3Model,
    FlowH3Latent,
    FlowH3Encode,
    FlowH3Sample,
    FlowH3DecodeVideo,
    FlowH3DecodeAudio,
    FlowLtxModel,
    FlowLtxEncode,
    FlowLtxNag,
    FlowLtxImage,
    FlowLtxStage1,
    FlowLtxUpscale,
    FlowLtxStage2,
    FlowLtxRetake,
    FlowLtxIcLora,
    FlowLtxLipdub,
    FlowLtxA2v,
    FlowLtxDecodeVideo,
    FlowLtxDecodeAudio,
    FlowAcestepModel,
    FlowAcestepEncode,
    FlowAcestepGenerate,
    FlowYue2Model,
    FlowYue2Generate,
    FlowYue2Decode,
    FlowYue2Transcribe,
    FlowOmnivoice,
    FlowVoxcpm,
    FlowVibevoice,
    FlowDiarize,
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
        NodeKind::TextView,
        NodeKind::FfmpegPlayer,
        NodeKind::ImageLoad,
        NodeKind::ImageSave,
        NodeKind::FlowText,
        NodeKind::FlowFile,
        NodeKind::FlowFormat,
        NodeKind::FlowLines,
        NodeKind::FlowList,
        NodeKind::FlowLlm,
        NodeKind::FlowImage,
        NodeKind::FlowVideo,
        NodeKind::FlowMusic,
        NodeKind::FlowSpeak,
        NodeKind::FlowTranscribe,
        NodeKind::FlowConcat,
        NodeKind::FlowMux,
        NodeKind::FlowFrame,
        NodeKind::FlowTrim,
        NodeKind::FlowStill,
        NodeKind::FlowImageModel,
        NodeKind::FlowImageEncode,
        NodeKind::FlowImageRefs,
        NodeKind::FlowImageLatent,
        NodeKind::FlowImageToLatent,
        NodeKind::FlowImageSample,
        NodeKind::FlowImageDecode,
        NodeKind::FlowH3Model,
        NodeKind::FlowH3Latent,
        NodeKind::FlowH3Encode,
        NodeKind::FlowH3Sample,
        NodeKind::FlowH3DecodeVideo,
        NodeKind::FlowH3DecodeAudio,
        NodeKind::FlowLtxModel,
        NodeKind::FlowLtxEncode,
        NodeKind::FlowLtxNag,
        NodeKind::FlowLtxImage,
        NodeKind::FlowLtxStage1,
        NodeKind::FlowLtxUpscale,
        NodeKind::FlowLtxStage2,
        NodeKind::FlowLtxRetake,
        NodeKind::FlowLtxIcLora,
        NodeKind::FlowLtxLipdub,
        NodeKind::FlowLtxA2v,
        NodeKind::FlowLtxDecodeVideo,
        NodeKind::FlowLtxDecodeAudio,
        NodeKind::FlowAcestepModel,
        NodeKind::FlowAcestepEncode,
        NodeKind::FlowAcestepGenerate,
        NodeKind::FlowYue2Model,
        NodeKind::FlowYue2Generate,
        NodeKind::FlowYue2Decode,
        NodeKind::FlowYue2Transcribe,
        NodeKind::FlowOmnivoice,
        NodeKind::FlowVoxcpm,
        NodeKind::FlowVibevoice,
        NodeKind::FlowDiarize,
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
    /// Хэндл универсальной «Syn Checkpoint»-ноды (LLM/TTS/ASR-модели).
    SynModel(Arc<SynModelHandle>),
    /// Одна картинка — общий тип для FLUX, LTX Image, H3 Keyframe, Image Save.
    Image(Arc<ImageData>),
    /// Значение ноды ядра synaptix-flow (файлы, списки — пакеты).
    Flow(synaptix_flow::Val),
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

    /// Из тензора `[4, H, W]` (или `[1, 4, H, W]`) RGBA в [0, 1]: `tensor` —
    /// RGB `[3, H, W]` (как у всех), настоящая альфа живёт в `rgba` —
    /// её видят превью, Image Save и референсы Qwen-Image 2.1.
    pub fn from_rgba_tensor(
        tensor: synaptix_core::tensor::Tensor,
        source: Option<PathBuf>,
    ) -> std::result::Result<Self, String> {
        use synaptix_core::{device::Device, dtype::DType};
        let t = tensor
            .to_device(Device::Cpu)
            .and_then(|t| t.to_dtype(DType::F32))
            .and_then(|t| t.contiguous())
            .map_err(|e| e.to_string())?;
        let d = t.dims().to_vec();
        let (c, h, w) = match d.as_slice() {
            [c, h, w] => (*c, *h, *w),
            [1, c, h, w] => (*c, *h, *w),
            _ => return Err(format!("ожидалась картинка [4, H, W], пришло {d:?}")),
        };
        if c == 3 {
            return Self::from_tensor(t, source);
        }
        if c != 4 {
            return Err(format!("ожидалось 4 канала, пришло {c}"));
        }
        let t = t.reshape(vec![4, h, w]).map_err(|e| e.to_string())?;
        let rgb = t.narrow(0, 0, 3).and_then(|x| x.contiguous()).map_err(|e| e.to_string())?;
        let mut img = Self::from_tensor(rgb, source)?;
        let a = t
            .narrow(0, 3, 1)
            .and_then(|x| x.contiguous())
            .and_then(|x| x.reshape(vec![h * w]))
            .and_then(|x| x.to_vec1::<f32>())
            .map_err(|e| e.to_string())?;
        let mut rgba = img.rgba.as_ref().clone();
        for (i, av) in a.iter().enumerate() {
            rgba[i * 4 + 3] = (av.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        img.rgba = Arc::new(rgba);
        Ok(img)
    }

    /// Есть ли в картинке прозрачность (альфа не везде 255).
    pub fn has_alpha(&self) -> bool {
        self.rgba.chunks_exact(4).any(|p| p[3] != 255)
    }

    /// RGBA `[4, H, W]` F32 в [0, 1] (из превью-буфера) — для сохранения с
    /// прозрачностью.
    pub fn rgba_tensor(&self) -> std::result::Result<synaptix_core::tensor::Tensor, String> {
        let (w, h) = (self.width as usize, self.height as usize);
        let plane = w * h;
        let mut v = vec![0f32; plane * 4];
        for (i, p) in self.rgba.chunks_exact(4).enumerate() {
            for c in 0..4 {
                v[c * plane + i] = p[c] as f32 / 255.0;
            }
        }
        synaptix_core::tensor::Tensor::from_vec(v, vec![4, h, w], synaptix_core::device::Device::Cpu)
            .map_err(|e| e.to_string())
    }

    /// Загрузить файл (png/jpeg/webp/bmp): RGB [0, 1] в `tensor`, альфа (если
    /// есть в файле) — в `rgba`.
    pub fn load(path: &std::path::Path) -> std::result::Result<Self, String> {
        let t = synaptix_io::image::png::load_image_rgba(path, synaptix_core::device::Device::Cpu)
            .map_err(|e| e.to_string())?;
        Self::from_rgba_tensor(t, Some(path.to_path_buf()))
    }
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
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






/// Кадры RGBA после видео-VAE. Один экземпляр шарится Arc'ами между
/// превью (FramesView) и Save-нодой. TODO: spool на диск для часовых
/// FullHD-видео.
pub struct MemFrames {
    pub frames: Arc<Vec<Arc<syngui::video::VideoFrame>>>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl std::fmt::Debug for MemFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MemFrames({}x{}, {} кадров, {:.1}fps)", self.width, self.height, self.frames.len(), self.fps)
    }
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
                DataBlob::Flow(v) => write!(f, "Data(Flow({v:?}))"),
                DataBlob::Image(img) => write!(f, "Data({img:?})"),
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










    pub fn as_syn_model(&self) -> Option<Arc<SynModelHandle>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
                DataBlob::SynModel(h) => Some(h.clone()),
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























    /// Извлечь декодированные кадры (выход видео-VAE).
    pub fn as_mem_frames(&self) -> Option<Arc<MemFrames>> {
        match self {
            PortValue::Data(b) => match b.as_ref() {
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
#[derive(Default)]
pub enum NodeRuntime {
    /// Нода без runtime-состояния (Number/Add/Output/etc.).
    #[default]
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

    // ── ACE-Step v1.5 (Нейро → ACE-Step) ──
    //


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
        frames_in: Arc<Mutex<Option<Arc<MemFrames>>>>,
        /// Аудио из памяти (вход `audio`, напр. LTX Audio Decode) для
        /// memory-режима.
        audio_in: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
        /// Активный memory-аудио плеер (играет `audio_in` синхронно с
        /// FramesView).
        mem_audio: Arc<Mutex<Option<AudioPlayer>>>,
        /// Бамп при смене `frames_in` → пересоздание FramesView в body.
        preview_version: RwSignal<u32>,
    },

    ImageLoad {
        path: RwSignal<Option<PathBuf>>,
        error: RwSignal<Option<String>>,
        /// Загруженная картинка — пока путь тот же, отдаётся тот же `Arc`.
        cache: Arc<Mutex<Option<Arc<ImageData>>>>,
    },
    /// Нода ядра synaptix-flow: прогон в потоке, выходы — значения ядра.
    Flow(super::nodes::flow::FlowRuntime),
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
            | R::ImageLoad { error, .. }
            | R::ImageSave { error, .. } => Some(*error),
            R::Flow(f) => Some(f.error),
            _ => None,
        }
    }



    /// Сигнал прогресса воркера (0..1) — для живой карточки прогона в чате.
    /// Стадии службы сообщают шаги не всегда — потребитель должен
    /// переживать «застывший» 0.
    pub fn run_progress_signal(&self) -> Option<RwSignal<f32>> {
        match self {
            NodeRuntime::Flow(f) => Some(f.progress),
            _ => None,
        }
    }

    /// Cooperative-cancel флаг воркера — для отмены прогона извне
    /// (`run_controls::cancel_active_run`). У нод без флага воркер не
    /// отменяется и досчитывает до конца.
    pub fn run_cancel_flag(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        use NodeRuntime as R;
        match self {
            R::SaveToFile { cancel, .. } => Some(cancel.clone()),
            R::Flow(f) => Some(f.cancel.clone()),
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




impl std::fmt::Debug for NodeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRuntime::None => f.write_str("NodeRuntime::None"),
            NodeRuntime::Flow(_) => f.write_str("NodeRuntime::Flow{..}"),
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
            NodeRuntime::TextView { output_text, .. } => {
                let len = output_text.get_untracked().chars().count();
                write!(f, "NodeRuntime::TextView{{chars={len}}}")
            }
            NodeRuntime::FfmpegPlayer { is_playing, current_path, .. } => {
                let path = current_path.get_untracked().map(|p| p.display().to_string()).unwrap_or_else(|| "-".to_string());
                write!(f, "NodeRuntime::FfmpegPlayer{{path={path}, playing={}}}", is_playing.get_untracked())
            }
            NodeRuntime::ImageLoad { path, .. } => {
                let p = path.get_untracked().map(|p| p.display().to_string()).unwrap_or_default();
                write!(f, "NodeRuntime::ImageLoad{{path={p}}}")
            }
            NodeRuntime::ImageSave { path, .. } => {
                let p = path.get_untracked().map(|p| p.display().to_string()).unwrap_or_default();
                write!(f, "NodeRuntime::ImageSave{{path={p}}}")
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
