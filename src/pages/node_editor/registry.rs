//! Каталог типов нод. Метадата описывает, какие порты и какие поля у ноды,
//! плюс factory `default_fields()` для создания свежей карты значений.

use syngui::core::sync::Mutex;
use syngui::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32};

use syngui::audio::{Biquad, BiquadMode, SchroederReverb};

use crate::icons::{
    MI_ADD, MI_APPS, MI_ARTICLE, MI_ASPECT_RATIO, MI_AUDIOTRACK, MI_AUTORENEW, MI_AUTO_AWESOME, MI_BLUR_ON,
    MI_CAMPAIGN, MI_EDIT_NOTE, MI_FILTER_ALT, MI_FOLDER_OPEN, MI_GRAPHIC_EQ, MI_GROUPS,
    MI_HUB, MI_IMAGE_ICON, MI_INVENTORY_2, MI_LANGUAGE, MI_LIBRARY_MUSIC, MI_MEMORY,
    MI_MERGE_TYPE, MI_MIC, MI_MOVIE, MI_PALETTE,
    MI_PLAY_ARROW, MI_PSYCHOLOGY, MI_RECORD_VOICE_OVER, MI_REMOVE_CIRCLE_OUTLINE, MI_SAVE,
    MI_CROP_SQUARE, MI_LAYERS, MI_TRANSLATE, MI_TUNE,
};

use super::eval::NodeExecutor;
use super::nodes::{
    acestep, asr_gigaam, audio_equalizer, audio_file, audio_filter, audio_gain, audio_mixer,
    audio_player, audio_recorder, audio_reverb, audio_save, ffmpeg_player, flux, flux2, image, llm, ltx,
    markdown_view,
    minimax_h3, omnivoice, scalar, sortformer_diarizer, syn_checkpoint, text_view, vibevoice,
    voxcpm2,
};
use super::types::{
    FieldSchema, FieldType, FieldValue, FilterMode, NodeInstance, NodeKind, NodeRuntime, PortKind,
    PortSchema, PortsLayout, PortsSpec, PostNormMode, PrevSource, SamplerPreset, SaveStatus,
};

pub type NodeBodyBuilder = fn(&NodeInstance) -> Box<dyn Widget>;

pub type PortRowExtra = fn(&NodeInstance, usize) -> Box<dyn Widget>;

/// Hook для глобальной кнопки Run в `run_controls`. Вызывается для каждой
/// ноды графа когда юзер нажимает «play» сверху страницы. None — нода не
/// реагирует на глобальный Run (типичный случай — реактивные ноды вроде
/// Add/Number, которые пересчитываются автоматически через `evaluate_graph`).
/// Some — нода имеет «явный запуск» (ASR-распознавание, file-write, и т.п.).
pub type NodeRunHook = fn(&NodeInstance, &super::state::NodeEditorCtx);

/// Hook для отслеживания «занятости» ноды. Возвращает `RwSignal<bool>`,
/// который true пока нода работает (например, идёт транскрибация).
/// `run_controls` подписывается на эти сигналы у всех нод и сбрасывает
/// верхний Run-state в `Stopped`, когда все они становятся false.
/// None для нод, не имеющих понятия busy (reactive nodes).
pub type NodeBusyHook = fn(&NodeInstance) -> Option<RwSignal<bool>>;

/// Категория ноды для группировки в палитре (cascading-меню «Add Node»).
/// Порядок и набор задаётся `NodeCategory::ORDER`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeCategory {
    Audio,
    Video,
    /// Картинки: загрузка файла → порт `image`.
    Image,
    DspEffects,
    Neuro,
    Math,
    Output,
    Misc,
}

impl NodeCategory {
    pub const ORDER: &'static [NodeCategory] = &[
        NodeCategory::Audio,
        NodeCategory::Video,
        NodeCategory::Image,
        NodeCategory::DspEffects,
        NodeCategory::Neuro,
        NodeCategory::Math,
        NodeCategory::Output,
        NodeCategory::Misc,
    ];

    /// Стабильный ключ для каталога строк и стилей.
    pub fn key(self) -> &'static str {
        match self {
            NodeCategory::Audio => "audio",
            NodeCategory::Video => "video",
            NodeCategory::Image => "image",
            NodeCategory::DspEffects => "dsp_effects",
            NodeCategory::Neuro => "neuro",
            NodeCategory::Math => "math",
            NodeCategory::Output => "output",
            NodeCategory::Misc => "misc",
        }
    }

    /// Английский фолбэк-лейбл (без каталога строк) — используется там, где
    /// нет доступа к `i18n` (например, текстовые отчёты агента в
    /// `agent::tools::pipelines`). Для UI используйте
    /// `crate::i18n::node_category_label(cat)`.
    pub fn label(self) -> &'static str {
        match self {
            NodeCategory::Audio => "Audio",
            NodeCategory::Video => "Video",
            NodeCategory::Image => "Images",
            NodeCategory::DspEffects => "DSP effects",
            NodeCategory::Neuro => "Neuro",
            NodeCategory::Math => "Math",
            NodeCategory::Output => "Output",
            NodeCategory::Misc => "Misc",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            NodeCategory::Audio => MI_LIBRARY_MUSIC,
            NodeCategory::Video => MI_MOVIE,
            NodeCategory::Image => MI_IMAGE_ICON,
            NodeCategory::DspEffects => MI_GRAPHIC_EQ,
            NodeCategory::Neuro => MI_PSYCHOLOGY,
            NodeCategory::Math => MI_AUTO_AWESOME,
            NodeCategory::Output => MI_SAVE,
            NodeCategory::Misc => MI_APPS,
        }
    }
}

/// Меню-метадата: иконка, заголовок, схема портов и схема полей.
#[derive(Clone, Copy)]
pub struct NodeKindMeta {
    pub kind: NodeKind,
    pub icon: &'static str,
    pub title: &'static str,
    /// Категория для группировки в палитре «Add Node».
    pub category: NodeCategory,
    /// Под-категория для группировки внутри меню «Add Node». Если задана,
    /// нода уходит во вложенный submenu с этим заголовком (вместо того,
    /// чтобы быть top-level пунктом своей категории). Используется для
    /// сворачивания группы Equalizer{6,10,20,30} в подменю «Эквалайзеры».
    /// Иконка submenu берётся от первой ноды в группе.
    pub subcategory: Option<&'static str>,
    pub inputs: PortsSpec,
    pub outputs: PortsSpec,
    pub fields: &'static [FieldSchema],
    /// Кастомный builder тела ноды (audio-плеер, рекордер и т.п.).
    /// Если None — используются стандартные field-rows + value-display.
    pub body: Option<NodeBodyBuilder>,
    pub ports_layout: PortsLayout,
    pub port_row_extra: Option<PortRowExtra>,
    /// Логика evaluate'а ноды. Каждая нода предоставляет свой ZST-executor,
    /// зарегистрированный как `&'static dyn NodeExecutor`. Используется
    /// `evaluate_graph` для dispatch'а — это убирает один центральный match
    /// из `eval.rs` и держит логику ноды рядом с её body builder'ом.
    pub executor: &'static dyn NodeExecutor,
    /// Опциональный hook для глобальной кнопки Run в `run_controls`.
    /// Если задан — будет вызван для каждого экземпляра этой ноды при
    /// нажатии «Run» сверху страницы. См. `NodeRunHook`.
    pub on_run: Option<NodeRunHook>,
    /// Опциональный hook возвращающий «idle/busy» сигнал ноды.
    /// `run_controls` использует его для авто-сброса Run-state когда все
    /// активные ноды завершились. См. `NodeBusyHook`.
    pub busy_signal: Option<NodeBusyHook>,
}

static NUMBER_EXEC: scalar::NumberExec = scalar::NumberExec;
static ADD_EXEC: scalar::AddExec = scalar::AddExec;
static OUTPUT_EXEC: scalar::OutputExec = scalar::OutputExec;
static AUDIO_FILE_EXEC: audio_file::AudioFileExec = audio_file::AudioFileExec;
static AUDIO_PLAYER_EXEC: audio_player::AudioPlayerExec = audio_player::AudioPlayerExec;
static AUDIO_RECORDER_EXEC: audio_recorder::AudioRecorderExec = audio_recorder::AudioRecorderExec;
static GAIN_EXEC: audio_gain::GainExec = audio_gain::GainExec;
static FILTER_EXEC: audio_filter::FilterExec = audio_filter::FilterExec;
static REVERB_EXEC: audio_reverb::ReverbExec = audio_reverb::ReverbExec;
static SAVE_TO_FILE_EXEC: audio_save::SaveToFileExec = audio_save::SaveToFileExec;
static EQUALIZER_EXEC: audio_equalizer::EqualizerExec = audio_equalizer::EqualizerExec;
static MIXER_EXEC: audio_mixer::MixerExec = audio_mixer::MixerExec;
static MARKDOWN_VIEW_EXEC: markdown_view::MarkdownViewExec = markdown_view::MarkdownViewExec;
static ASR_GIGAAM_EXEC: asr_gigaam::AsrGigaamExec = asr_gigaam::AsrGigaamExec;
static TEXT_VIEW_EXEC: text_view::TextViewExec = text_view::TextViewExec;
static OMNIVOICE_EXEC: omnivoice::OmniVoiceExec = omnivoice::OmniVoiceExec;
static VOXCPM2_EXEC: voxcpm2::VoxCpm2Exec = voxcpm2::VoxCpm2Exec;
static VIBEVOICE_EXEC: vibevoice::VibeVoiceExec = vibevoice::VibeVoiceExec;
static LLM_EXEC: llm::LlmExec = llm::LlmExec;
static SORTFORMER_DIARIZER_EXEC: sortformer_diarizer::SortformerDiarizerExec =
    sortformer_diarizer::SortformerDiarizerExec;
static ACESTEP_VAE_ENCODE_EXEC: acestep::vae_encode::VaeEncodeExec =
    acestep::vae_encode::VaeEncodeExec;
static ACESTEP_CHECKPOINT_EXEC: acestep::checkpoint::CheckpointExec =
    acestep::checkpoint::CheckpointExec;
static ACESTEP_GENERATE_EXEC: acestep::generate::GenerateExec = acestep::generate::GenerateExec;
static FFMPEG_PLAYER_EXEC: ffmpeg_player::FfmpegPlayerExec = ffmpeg_player::FfmpegPlayerExec;
static LTX_CHECKPOINT_EXEC: ltx::checkpoint::CheckpointExec = ltx::checkpoint::CheckpointExec;
static H3_CHECKPOINT_EXEC: minimax_h3::checkpoint::CheckpointExec = minimax_h3::checkpoint::CheckpointExec;
static H3_TEXT_ENCODER_EXEC: minimax_h3::text_encoder::TextEncoderExec = minimax_h3::text_encoder::TextEncoderExec;
static H3_EMPTY_LATENT_EXEC: minimax_h3::latent::EmptyLatentExec = minimax_h3::latent::EmptyLatentExec;
static H3_KEYFRAME_EXEC: minimax_h3::latent::KeyframeExec = minimax_h3::latent::KeyframeExec;
static H3_REFERENCES_EXEC: minimax_h3::references::ReferencesExec = minimax_h3::references::ReferencesExec;
static H3_SAMPLER_EXEC: minimax_h3::sampler::SamplerExec = minimax_h3::sampler::SamplerExec;
static H3_VAE_DECODE_EXEC: minimax_h3::decode::VaeDecodeExec = minimax_h3::decode::VaeDecodeExec;
static H3_AUDIO_DECODE_EXEC: minimax_h3::decode::AudioDecodeExec = minimax_h3::decode::AudioDecodeExec;
static H3_VIDEO_SAVE_EXEC: minimax_h3::save::VideoSaveExec = minimax_h3::save::VideoSaveExec;
static LTX_TEXT_ENCODER_EXEC: ltx::text_encoder::TextEncoderExec =
    ltx::text_encoder::TextEncoderExec;
static LTX_NAG_PROMPT_EXEC: ltx::nag::NagPromptExec = ltx::nag::NagPromptExec;
static LTX_SAMPLER_STAGE1_EXEC: ltx::sampler_stage1::SamplerStage1Exec =
    ltx::sampler_stage1::SamplerStage1Exec;
static LTX_UPSCALE_EXEC: ltx::upscale::UpscaleExec = ltx::upscale::UpscaleExec;
static LTX_SAMPLER_STAGE2_EXEC: ltx::sampler_stage2::SamplerStage2Exec =
    ltx::sampler_stage2::SamplerStage2Exec;
static LTX_VAE_DECODE_EXEC: ltx::vae_decode::VaeDecodeExec = ltx::vae_decode::VaeDecodeExec;
static LTX_AUDIO_DECODE_EXEC: ltx::audio_decode::AudioDecodeExec =
    ltx::audio_decode::AudioDecodeExec;
static LTX_VIDEO_SAVE_EXEC: ltx::video_save::VideoSaveExec = ltx::video_save::VideoSaveExec;
static LTX_IMAGE_EXEC: ltx::image::ImageExec = ltx::image::ImageExec;
static LTX_VIDEO_INPUT_EXEC: ltx::video_input::VideoInputExec = ltx::video_input::VideoInputExec;
static LTX_RETAKE_EXEC: ltx::retake::RetakeExec = ltx::retake::RetakeExec;
static LTX_IC_LORA_EXEC: ltx::ic_lora::IcLoraExec = ltx::ic_lora::IcLoraExec;
static LTX_AUDIO_INPUT_EXEC: ltx::audio_input::AudioInputExec = ltx::audio_input::AudioInputExec;
static LTX_LIPDUB_EXEC: ltx::lipdub::LipdubExec = ltx::lipdub::LipdubExec;
static LTX_A2V_EXEC: ltx::a2v::A2vExec = ltx::a2v::A2vExec;
static SYN_CHECKPOINT_EXEC: syn_checkpoint::SynCheckpointExec =
    syn_checkpoint::SynCheckpointExec;
static FLUX_CHECKPOINT_EXEC: flux::checkpoint::CheckpointExec = flux::checkpoint::CheckpointExec;
static FLUX_TEXT_ENCODER_EXEC: flux::text_encoder::TextEncoderExec = flux::text_encoder::TextEncoderExec;
static FLUX_EMPTY_LATENT_EXEC: flux::latent::EmptyLatentExec = flux::latent::EmptyLatentExec;
static FLUX_VAE_ENCODE_EXEC: flux::vae::VaeEncodeExec = flux::vae::VaeEncodeExec;
static FLUX_SAMPLER_EXEC: flux::sampler::SamplerExec = flux::sampler::SamplerExec;
static FLUX_VAE_DECODE_EXEC: flux::vae::VaeDecodeExec = flux::vae::VaeDecodeExec;
static FLUX2_CHECKPOINT_EXEC: flux2::checkpoint::CheckpointExec = flux2::checkpoint::CheckpointExec;
static FLUX2_TEXT_ENCODER_EXEC: flux2::text_encoder::TextEncoderExec = flux2::text_encoder::TextEncoderExec;
static FLUX2_REFERENCE_EXEC: flux2::reference::ReferenceExec = flux2::reference::ReferenceExec;
static FLUX2_SAMPLER_EXEC: flux2::sampler::SamplerExec = flux2::sampler::SamplerExec;
static FLUX2_VAE_DECODE_EXEC: flux2::vae::VaeDecodeExec = flux2::vae::VaeDecodeExec;
static IMAGE_LOAD_EXEC: image::ImageLoadExec = image::ImageLoadExec;
static IMAGE_SAVE_EXEC: image::ImageSaveExec = image::ImageSaveExec;

const NUMBER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "out", kind: PortKind::Data },
];
const NUMBER_FIELDS: &[FieldSchema] = &[
    FieldSchema { name: "value", label: "Value", ty: FieldType::Float },
];

const ADD_INPUTS: &[PortSchema] = &[
    PortSchema { name: "a", label: "a", kind: PortKind::Data },
    PortSchema { name: "b", label: "b", kind: PortKind::Data },
];
const ADD_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "a + b", kind: PortKind::Data },
];

const OUTPUT_INPUTS: &[PortSchema] = &[
    PortSchema { name: "in", label: "in", kind: PortKind::Data },
];

const NO_FIELDS: &[FieldSchema] = &[];

const AUDIO_FILE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "audio", kind: PortKind::Audio },
];

const AUDIO_PLAYER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "in", label: "audio", kind: PortKind::Audio },
];

const AUDIO_RECORDER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "audio", kind: PortKind::Audio },
];

// ── Effect / save nodes ──────────────────────────────────────────────────

const AUDIO_IO_INPUTS: &[PortSchema] = &[
    PortSchema { name: "in", label: "audio", kind: PortKind::Audio },
];
const AUDIO_IO_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "audio", kind: PortKind::Audio },
];

// SaveToFile — sink, input only.
const SAVE_TO_FILE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "in", label: "audio", kind: PortKind::Audio },
];

// ── ASR / Транскрибация ──────────────────────────────────────────────────
const ASR_GIGAAM_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "in", label: "audio", kind: PortKind::Audio },
];
const ASR_GIGAAM_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "text", kind: PortKind::Text },
];

// ── Syn Checkpoint: универсальный источник модели для слот-семейств ──────
const SYN_CHECKPOINT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
];

// ── LLM (synaptix Qwen3 / Hybrid): prompt + system? → answer (Text) ──────
const LLM_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "prompt", label: "prompt", kind: PortKind::Text },
    PortSchema { name: "system", label: "system", kind: PortKind::Text },
];
const LLM_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "answer", label: "answer", kind: PortKind::Text },
];

const VOXCPM2_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "text", label: "text", kind: PortKind::Text },
    PortSchema { name: "ref_audio", label: "ref audio", kind: PortKind::Audio },
    PortSchema { name: "prompt_audio", label: "prompt audio", kind: PortKind::Audio },
    PortSchema { name: "prompt_text", label: "prompt text", kind: PortKind::Text },
];
const VOXCPM2_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];

const VIBEVOICE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "script", label: "script", kind: PortKind::Text },
    PortSchema { name: "voice1", label: "voice 1", kind: PortKind::Audio },
    PortSchema { name: "voice2", label: "voice 2", kind: PortKind::Audio },
    PortSchema { name: "voice3", label: "voice 3", kind: PortKind::Audio },
    PortSchema { name: "voice4", label: "voice 4", kind: PortKind::Audio },
];
const VIBEVOICE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];

// ── Диаризация спикеров (Sortformer): Audio → Text(JSON) ─────────────────
const SORTFORMER_DIARIZER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "in", label: "audio", kind: PortKind::Audio },
];
const SORTFORMER_DIARIZER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "json", kind: PortKind::Text },
];

// ── Editable text-view ───────────────────────────────────────────────────
const TEXT_VIEW_INPUTS: &[PortSchema] = &[
    PortSchema { name: "in", label: "text", kind: PortKind::Text },
];
const TEXT_VIEW_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "text", kind: PortKind::Text },
];

// ── TTS (OmniVoice): Text → Audio с опциональным voice-clone ─────────────
//
// `text` — основной таргет; `ref_audio` + `ref_text` (опциональные)
// активируют Clone-mode (voice cloning). Если ref_audio нет — mode
// определяется наличием instruct (Design) или авто-голосом (Auto).
const OMNIVOICE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",     label: "model",     kind: PortKind::Data },
    PortSchema { name: "text",      label: "text",      kind: PortKind::Text },
    PortSchema { name: "ref_audio", label: "ref audio", kind: PortKind::Audio },
    PortSchema { name: "ref_text",  label: "ref text",  kind: PortKind::Text },
];
const OMNIVOICE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];

// ── ACE-Step ─────────────────────────────────────────────────────────────

const ACESTEP_VAE_ENCODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];
const ACESTEP_VAE_ENCODE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
];

const ACESTEP_CHECKPOINT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
];

const ACESTEP_GENERATE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",      label: "model",      kind: PortKind::Data },
    PortSchema { name: "tags",       label: "tags",       kind: PortKind::Text },
    PortSchema { name: "lyrics",     label: "lyrics",     kind: PortKind::Text },
    PortSchema { name: "src_latent", label: "src_latent", kind: PortKind::Data },
];
const ACESTEP_GENERATE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio",  label: "audio",  kind: PortKind::Audio },
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
];

const FFMPEG_PLAYER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "source", label: "source", kind: PortKind::Text },
    PortSchema { name: "frames", label: "frames", kind: PortKind::Video },
    PortSchema { name: "audio",  label: "audio",  kind: PortKind::Audio },
];
const FFMPEG_PLAYER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video", label: "video", kind: PortKind::Video },
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];

const LTX_CHECKPOINT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
];

const H3_MODEL_OUT: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
];
const H3_TEXT_ENCODER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "prompt", label: "prompt", kind: PortKind::Text },
    PortSchema { name: "keyframe", label: "keyframe", kind: PortKind::Data },
    PortSchema { name: "keyframe_last", label: "keyframe 2", kind: PortKind::Data },
    PortSchema { name: "refs", label: "refs", kind: PortKind::Data },
];
const H3_TEXT_ENCODER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "conditioning", label: "conditioning", kind: PortKind::Data },
];
const H3_EMPTY_LATENT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "av_latent", label: "av latent", kind: PortKind::Data },
];
const H3_KEYFRAME_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "keyframe", label: "keyframe", kind: PortKind::Data },
];
const H3_REFERENCES_INPUTS: &[PortSchema] = &[
    PortSchema { name: "av_latent", label: "av latent", kind: PortKind::Data },
];
const H3_REFERENCES_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "refs", label: "refs", kind: PortKind::Data },
];
const H3_SAMPLER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "conditioning", label: "conditioning", kind: PortKind::Data },
    PortSchema { name: "negative", label: "negative", kind: PortKind::Data },
    PortSchema { name: "av_latent", label: "av latent", kind: PortKind::Data },
    PortSchema { name: "keyframe", label: "keyframe", kind: PortKind::Data },
    PortSchema { name: "keyframe_last", label: "keyframe 2", kind: PortKind::Data },
    PortSchema { name: "refs", label: "refs", kind: PortKind::Data },
];
const H3_SAMPLER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_latent", label: "audio latent", kind: PortKind::Data },
];
const H3_VAE_DECODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
];
const H3_VAE_DECODE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "frames", label: "frames", kind: PortKind::Video },
];
const H3_AUDIO_DECODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "audio_latent", label: "audio latent", kind: PortKind::Data },
];
const H3_AUDIO_DECODE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];
const H3_VIDEO_SAVE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "frames", label: "frames", kind: PortKind::Video },
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];
const H3_VIDEO_SAVE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "path", label: "path", kind: PortKind::Text },
];
/// Необязательный вход картинки у LTX Image / H3 Keyframe: провод важнее файла.
const IMAGE_IN: &[PortSchema] = &[
    PortSchema { name: "image", label: "image", kind: PortKind::Image },
];
const IMAGE_OUT: &[PortSchema] = &[
    PortSchema { name: "image", label: "image", kind: PortKind::Image },
];
const IMAGE_SAVE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "path", label: "path", kind: PortKind::Text },
];

const FLUX_MODEL_OUT: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
];
const FLUX_TEXT_ENCODER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "prompt", label: "prompt", kind: PortKind::Text },
];
const FLUX_TEXT_ENCODER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "conditioning", label: "conditioning", kind: PortKind::Data },
];
const FLUX_LATENT_OUT: &[PortSchema] = &[
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
];
const FLUX_VAE_ENCODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "image", label: "image", kind: PortKind::Image },
    PortSchema { name: "size", label: "size", kind: PortKind::Data },
];
const FLUX_SAMPLER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "conditioning", label: "conditioning", kind: PortKind::Data },
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
];
const FLUX_VAE_DECODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
];
/// Референс: картинка и (необязательно) предыдущие референсы цепочки.
const FLUX2_REFERENCE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "image", label: "image", kind: PortKind::Image },
    PortSchema { name: "references", label: "references", kind: PortKind::Data },
];
const FLUX2_REFERENCES_OUT: &[PortSchema] = &[
    PortSchema { name: "references", label: "references", kind: PortKind::Data },
];
/// `latent` задаёт размер (FLUX Empty Latent); без него размер берётся с
/// первого референса. `references` — необязательно, для правки.
const FLUX2_SAMPLER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "conditioning", label: "conditioning", kind: PortKind::Data },
    PortSchema { name: "latent", label: "latent", kind: PortKind::Data },
    PortSchema { name: "references", label: "references", kind: PortKind::Data },
];

const LTX_TEXT_ENCODER_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",  label: "model",  kind: PortKind::Data },
    PortSchema { name: "prompt", label: "prompt", kind: PortKind::Text },
];
const LTX_TEXT_ENCODER_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_encoding", label: "video enc", kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc", kind: PortKind::Data },
];

const LTX_NAG_PROMPT_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model", label: "model", kind: PortKind::Data },
    PortSchema { name: "text",  label: "text",  kind: PortKind::Text },
];
const LTX_NAG_PROMPT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "nag", label: "nag", kind: PortKind::Data },
];

const LTX_SAMPLER_STAGE1_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",      kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc",  kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc",  kind: PortKind::Data },
    PortSchema { name: "nag",            label: "nag",        kind: PortKind::Data },
    PortSchema { name: "image_cond",     label: "image",      kind: PortKind::Data },
];

const LTX_IMAGE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "image_cond", label: "image", kind: PortKind::Data },
];

const LTX_VIDEO_INPUT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video", label: "video", kind: PortKind::Data },
];

const LTX_RETAKE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",     kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc", kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc", kind: PortKind::Data },
    PortSchema { name: "video",          label: "video",     kind: PortKind::Data },
];
const LTX_RETAKE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];

const LTX_IC_LORA_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",     kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc", kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc", kind: PortKind::Data },
    PortSchema { name: "ref_video",      label: "ref video", kind: PortKind::Data },
];
const LTX_IC_LORA_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];

const LTX_AUDIO_INPUT_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Data },
];

const LTX_LIPDUB_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",     kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc", kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc", kind: PortKind::Data },
    PortSchema { name: "ref_video",      label: "ref video", kind: PortKind::Data },
    PortSchema { name: "audio",          label: "audio",     kind: PortKind::Data },
];
const LTX_LIPDUB_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];

const LTX_A2V_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",     kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc", kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc", kind: PortKind::Data },
    PortSchema { name: "audio",          label: "audio",     kind: PortKind::Data },
];
const LTX_A2V_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];
const LTX_SAMPLER_STAGE1_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];

const LTX_UPSCALE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",        label: "model",        kind: PortKind::Data },
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
];
const LTX_UPSCALE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent ×2", kind: PortKind::Data },
];

const LTX_SAMPLER_STAGE2_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",          label: "model",        kind: PortKind::Data },
    PortSchema { name: "video_latent",   label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens",   label: "audio tokens", kind: PortKind::Data },
    PortSchema { name: "video_encoding", label: "video enc",    kind: PortKind::Data },
    PortSchema { name: "audio_encoding", label: "audio enc",    kind: PortKind::Data },
    PortSchema { name: "image_cond",     label: "image",        kind: PortKind::Data },
];
const LTX_SAMPLER_STAGE2_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];

const LTX_VAE_DECODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",        label: "model",        kind: PortKind::Data },
    PortSchema { name: "video_latent", label: "video latent", kind: PortKind::Data },
];
const LTX_VAE_DECODE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "frames", label: "frames", kind: PortKind::Video },
];

const LTX_AUDIO_DECODE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "model",        label: "model",        kind: PortKind::Data },
    PortSchema { name: "audio_tokens", label: "audio tokens", kind: PortKind::Data },
];
const LTX_AUDIO_DECODE_OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "audio", label: "audio", kind: PortKind::Audio },
];

const LTX_VIDEO_SAVE_INPUTS: &[PortSchema] = &[
    PortSchema { name: "frames", label: "frames", kind: PortKind::Video },
    PortSchema { name: "audio",  label: "audio",  kind: PortKind::Audio },
];

const NUMBER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Number,
    icon: MI_AUTO_AWESOME,
    title: "Number",
    category: NodeCategory::Math,
    subcategory: None,
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(NUMBER_OUTPUTS),
    fields: NUMBER_FIELDS,
    body: None,
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &NUMBER_EXEC,
    on_run: None,
    busy_signal: None,
};

const ADD: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Add,
    icon: MI_ADD,
    title: "Add",
    category: NodeCategory::Math,
    subcategory: None,
    inputs: PortsSpec::Static(ADD_INPUTS),
    outputs: PortsSpec::Static(ADD_OUTPUTS),
    fields: NO_FIELDS,
    body: None,
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &ADD_EXEC,
    on_run: None,
    busy_signal: None,
};

const OUTPUT_NODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Output,
    icon: MI_LANGUAGE,
    title: "Output",
    category: NodeCategory::Output,
    subcategory: None,
    inputs: PortsSpec::Static(OUTPUT_INPUTS),
    outputs: PortsSpec::Static(&[]),
    fields: NO_FIELDS,
    body: None,
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &OUTPUT_EXEC,
    on_run: None,
    busy_signal: None,
};

const AUDIO_FILE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AudioFile,
    icon: MI_FOLDER_OPEN,
    title: "Audio File",
    category: NodeCategory::Audio,
    subcategory: None,
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(AUDIO_FILE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_file::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &AUDIO_FILE_EXEC,
    on_run: None,
    busy_signal: None,
};

const AUDIO_PLAYER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AudioPlayer,
    icon: MI_PLAY_ARROW,
    title: "Audio Player",
    category: NodeCategory::Audio,
    subcategory: None,
    inputs: PortsSpec::Static(AUDIO_PLAYER_INPUTS),
    outputs: PortsSpec::Static(&[]),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_player::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &AUDIO_PLAYER_EXEC,
    on_run: None,
    busy_signal: None,
};

const AUDIO_RECORDER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AudioRecorder,
    icon: MI_MIC,
    title: "Audio Recorder",
    category: NodeCategory::Audio,
    subcategory: None,
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(AUDIO_RECORDER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_recorder::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &AUDIO_RECORDER_EXEC,
    on_run: None,
    busy_signal: None,
};

const GAIN: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Gain,
    icon: MI_GRAPHIC_EQ,
    title: "Gain",
    category: NodeCategory::DspEffects,
    subcategory: None,
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_gain::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &GAIN_EXEC,
    on_run: None,
    busy_signal: None,
};

const FILTER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Filter,
    icon: MI_FILTER_ALT,
    title: "Filter",
    category: NodeCategory::DspEffects,
    subcategory: None,
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_filter::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &FILTER_EXEC,
    on_run: None,
    busy_signal: None,
};

const REVERB: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Reverb,
    icon: MI_BLUR_ON,
    title: "Reverb",
    category: NodeCategory::DspEffects,
    subcategory: None,
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_reverb::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &REVERB_EXEC,
    on_run: None,
    busy_signal: None,
};

const SAVE_TO_FILE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::SaveToFile,
    icon: MI_SAVE,
    title: "Save to File",
    category: NodeCategory::Output,
    subcategory: None,
    inputs: PortsSpec::Static(SAVE_TO_FILE_INPUTS),
    outputs: PortsSpec::Static(&[]),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_save::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &SAVE_TO_FILE_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── Equalizer (graphic-EQ, каскад peaking-Biquad'ов) ─────────────────────
//
// Высота 180px рассчитана на: gain-label (12px) + slider (120px) + freq-label
// (12px) + gap'ы (8) + padding (~28). Ширина растёт по числу полос:
// 6 → 360, 10 → 500, 20 → 740, 30 → 980. На полосу выходит ~32–46 px.

// Ширина EQ-карточки определяется самой layout-системой через MSS
// `width: fit-content` на `.node-card` — никаких magic-чисел в meta.

const EQUALIZER_6: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Equalizer6,
    icon: MI_TUNE,
    title: "Equalizer 6",
    category: NodeCategory::DspEffects,
    subcategory: Some("equalizers"),
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_equalizer::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &EQUALIZER_EXEC,
    on_run: None,
    busy_signal: None,
};

const EQUALIZER_10: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Equalizer10,
    icon: MI_TUNE,
    title: "Equalizer 10",
    category: NodeCategory::DspEffects,
    subcategory: Some("equalizers"),
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_equalizer::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &EQUALIZER_EXEC,
    on_run: None,
    busy_signal: None,
};

const EQUALIZER_20: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Equalizer20,
    icon: MI_TUNE,
    title: "Equalizer 20",
    category: NodeCategory::DspEffects,
    subcategory: Some("equalizers"),
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_equalizer::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &EQUALIZER_EXEC,
    on_run: None,
    busy_signal: None,
};

const EQUALIZER_30: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Equalizer30,
    icon: MI_TUNE,
    title: "Equalizer 30",
    category: NodeCategory::DspEffects,
    subcategory: Some("equalizers"),
    inputs: PortsSpec::Static(AUDIO_IO_INPUTS),
    outputs: PortsSpec::Static(AUDIO_IO_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_equalizer::body),
    ports_layout: PortsLayout::Compact { centered: true },
    port_row_extra: None,
    executor: &EQUALIZER_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── Mixer (вариативные input'ы через PortsSpec::Dynamic) ─────────────────

const MIXER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Mixer,
    icon: MI_MERGE_TYPE,
    title: "Mixer",
    category: NodeCategory::DspEffects,
    subcategory: None,
    inputs: PortsSpec::Dynamic {
        pool: &audio_mixer::MIXER_PORT_SCHEMAS_FULL,
        runtime: audio_mixer::mixer_inputs,
    },
    outputs: PortsSpec::Static(&audio_mixer::MIXER_OUTPUT_SCHEMAS),
    fields: NO_FIELDS,
    body: Some(super::nodes::audio_mixer::body),
    ports_layout: PortsLayout::Compact { centered: false },
    port_row_extra: Some(super::nodes::audio_mixer::port_row_extra),
    executor: &MIXER_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── Декоративная markdown-нода (без портов и executor-логики) ────────────
//
// Содержимое — `MarkdownView` (Preview) или `MarkdownEditor` (Edit);
// resize-handle'ы делегированы `TransformBox` (см. `nodes::markdown_view`).
// Подкатегория «Аннотации» в меню «Прочее» — задел под будущие
// sticky-notes / comment-ноды.
const MARKDOWN_VIEW: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::MarkdownView,
    icon: MI_ARTICLE,
    title: "Markdown",
    category: NodeCategory::Misc,
    subcategory: Some("annotations"),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(&[]),
    fields: NO_FIELDS,
    body: Some(super::nodes::markdown_view::body),
    ports_layout: PortsLayout::None,
    port_row_extra: None,
    executor: &MARKDOWN_VIEW_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── ASR (GigaAM): Audio → Text ───────────────────────────────────────────
//
// Body: rfd-выбор `.syn`, dropdown'ы device / storage / compute, Play-
// кнопка, editable MultilineTextEdit. Категория «Нейро» → подкатегория
// «Транскрибация» — задел под будущие LLM/TTS/embedding-ноды этой же
// категории.
const ASR_GIGAAM: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AsrGigaam,
    icon: MI_RECORD_VOICE_OVER,
    title: "GigaAM ASR",
    category: NodeCategory::Neuro,
    subcategory: Some("transcription"),
    inputs: PortsSpec::Static(ASR_GIGAAM_INPUTS),
    outputs: PortsSpec::Static(ASR_GIGAAM_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::asr_gigaam::body),
    // Стандартная раскладка как у Demo: port-row сверху + body снизу.
    // Body — field-like rows (label слева, control справа); транскрибат
    // не отображается в карточке (уходит в output → подключайте TextView).
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &ASR_GIGAAM_EXEC,
    // Запускается по глобальному «Run» из run_controls — для каждой
    // AsrGigaam-ноды стартует фоновый воркер транскрибации (то же что и
    // нажатие Per-node Play в строке «Запуск»).
    on_run: Some(asr_gigaam::on_run),
    busy_signal: Some(asr_gigaam::busy_signal),
};

// ── TextView: Text in → editable text → Text out (passthrough) ───────────
//
// Editable multi-line viewer. Подключайте к Text-выходу любой ноды
// (например, GigaAM ASR), чтобы видеть и править результат. Output
// порта несёт текущий текст (с учётом ручных правок).
const TEXT_VIEW: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::TextView,
    icon: MI_EDIT_NOTE,
    title: "Text View",
    category: NodeCategory::Misc,
    subcategory: Some("text"),
    inputs: PortsSpec::Static(TEXT_VIEW_INPUTS),
    outputs: PortsSpec::Static(TEXT_VIEW_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::text_view::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &TEXT_VIEW_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── OmniVoice TTS: Text → Audio с опциональным voice-clone ───────────────
//
// Body: rfd-выбор `.syn` (берётся parent() как model_dir), dropdown'ы
// device/storage/compute, textfield'ы instruct/ref_text/language, sliders
// для cfg/steps/t_shift/speed/seed. Mode выбирается автоматически:
//   ref_audio есть → Clone; иначе instruct непуст → Design; иначе Auto.
// Pipeline загружается лениво при первом Run (~30 c на CPU) и кэшируется.
const OMNIVOICE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::OmniVoice,
    icon: MI_CAMPAIGN,
    title: "OmniVoice TTS",
    category: NodeCategory::Neuro,
    subcategory: Some("tts"),
    inputs: PortsSpec::Static(OMNIVOICE_INPUTS),
    outputs: PortsSpec::Static(OMNIVOICE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::omnivoice::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &OMNIVOICE_EXEC,
    on_run: Some(omnivoice::on_run),
    busy_signal: Some(omnivoice::busy_signal),
};

const VOXCPM2: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::VoxCpm2,
    icon: MI_RECORD_VOICE_OVER,
    title: "VoxCPM2 TTS (synaptix)",
    category: NodeCategory::Neuro,
    subcategory: Some("tts"),
    inputs: PortsSpec::Static(VOXCPM2_INPUTS),
    outputs: PortsSpec::Static(VOXCPM2_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::voxcpm2::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &VOXCPM2_EXEC,
    on_run: Some(voxcpm2::on_run),
    busy_signal: Some(voxcpm2::busy_signal),
};

const VIBEVOICE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::VibeVoice,
    icon: MI_CAMPAIGN,
    title: "VibeVoice (dialogue)",
    category: NodeCategory::Neuro,
    subcategory: Some("tts"),
    inputs: PortsSpec::Static(VIBEVOICE_INPUTS),
    outputs: PortsSpec::Static(VIBEVOICE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::vibevoice::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &VIBEVOICE_EXEC,
    on_run: Some(vibevoice::on_run),
    busy_signal: Some(vibevoice::busy_signal),
};

// ── LLM (synaptix): prompt + system? → answer (Text) ─────────────────────
//
// Нативный synaptix-LLM-стек (Qwen3 dense/MoE либо Qwen3-Next-Hybrid, арх
// детектится из config.json). Body: picker модели (HF-каталог /
// .syn), dropdown'ы device/quant/compute, system-fallback, max_tokens /
// temperature / seed, кнопка отмены, статус. Ответ стримится по токенам в
// порт `answer` — подключайте к TextView. Pipeline грузится лениво при Run
// и кэшируется в runtime.
const LLM: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Llm,
    icon: MI_AUTO_AWESOME,
    title: "LLM (synaptix)",
    category: NodeCategory::Neuro,
    subcategory: Some("llm"),
    inputs: PortsSpec::Static(LLM_INPUTS),
    outputs: PortsSpec::Static(LLM_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::llm::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LLM_EXEC,
    on_run: Some(llm::on_run),
    busy_signal: Some(llm::busy_signal),
};

// ── Syn Checkpoint: путь + предпочтения + резидентность → SynModelHandle ─
//
// Единый стиль с LTX/H3/ACE-Step: чекпойнт-нода не грузит веса, а публикует
// конфиг-хэндл в порт `model`. Потребители — LLM/VoxCPM2/OmniVoice/ASR/
// Sortformer (у них появился вход `model`; без него работают от своих
// полей — legacy-графы не ломаются). Чекбокс «Держать в памяти» —
// слотовое поведение (модель живёт между прогонами).
const SYN_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::SynCheckpoint,
    icon: MI_MEMORY,
    title: "Syn Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: None,
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(SYN_CHECKPOINT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::syn_checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &SYN_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

// ── Sortformer диаризация: Audio → Text(JSON) ────────────────────────────
//
// Аналог AsrGigaam (категория «Нейро»), но в подкатегории «Диаризация».
// Body — file picker + dropdown'ы device/storage/compute + threshold slider
// + Allow-overlap switch + статус. Live-stream пока не поддерживается
// (фаза 2 — streaming через KV-cache).
const SORTFORMER_DIARIZER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::SortformerDiarizer,
    icon: MI_GROUPS,
    title: "Sortformer Diarization",
    category: NodeCategory::Neuro,
    subcategory: Some("diarization"),
    inputs: PortsSpec::Static(SORTFORMER_DIARIZER_INPUTS),
    outputs: PortsSpec::Static(SORTFORMER_DIARIZER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(super::nodes::sortformer_diarizer::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &SORTFORMER_DIARIZER_EXEC,
    on_run: Some(sortformer_diarizer::on_run),
    busy_signal: Some(sortformer_diarizer::busy_signal),
};

// ── ACE-Step v1.5 (8 нод) ────────────────────────────────────────────────

/// Подкатегория в меню «Add Node» (под `NodeCategory::Neuro`). Стабильный
/// ключ каталога строк — отображаемое имя см. `node.subcategory.ace_step`.
const ACESTEP_SUBCATEGORY: &str = "ace_step";

const ACESTEP_VAE_ENCODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AceStepVaeEncode,
    icon: MI_AUDIOTRACK,
    title: "ACE-Step VAE Encode",
    category: NodeCategory::Neuro,
    subcategory: Some(ACESTEP_SUBCATEGORY),
    inputs: PortsSpec::Static(ACESTEP_VAE_ENCODE_INPUTS),
    outputs: PortsSpec::Static(ACESTEP_VAE_ENCODE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(acestep::vae_encode::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &ACESTEP_VAE_ENCODE_EXEC,
    on_run: Some(acestep::vae_encode::on_run),
    busy_signal: Some(acestep::vae_encode::busy_signal),
};

const ACESTEP_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AceStepCheckpoint,
    icon: MI_INVENTORY_2,
    title: "ACE-Step Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: Some(ACESTEP_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(ACESTEP_CHECKPOINT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(acestep::checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &ACESTEP_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

const ACESTEP_GENERATE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::AceStepGenerate,
    icon: MI_LIBRARY_MUSIC,
    title: "ACE-Step Generate",
    category: NodeCategory::Neuro,
    subcategory: Some(ACESTEP_SUBCATEGORY),
    inputs: PortsSpec::Static(ACESTEP_GENERATE_INPUTS),
    outputs: PortsSpec::Static(ACESTEP_GENERATE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(acestep::generate::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &ACESTEP_GENERATE_EXEC,
    on_run: Some(acestep::generate::on_run),
    busy_signal: Some(acestep::generate::busy_signal),
};

const FFMPEG_PLAYER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FfmpegPlayer,
    icon: MI_MOVIE,
    title: "Video player",
    category: NodeCategory::Video,
    subcategory: Some("sources"),
    inputs: PortsSpec::Static(FFMPEG_PLAYER_INPUTS),
    outputs: PortsSpec::Static(FFMPEG_PLAYER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ffmpeg_player::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FFMPEG_PLAYER_EXEC,
    on_run: Some(ffmpeg_player::on_run),
    busy_signal: Some(ffmpeg_player::busy_signal),
};

/// Стабильный ключ каталога строк — отображаемое имя см.
/// `node.subcategory.ltx_video`.
const LTX_SUBCATEGORY: &str = "ltx_video";

/// Стабильный ключ каталога строк — отображаемое имя см.
/// `node.subcategory.minimax_h3`.
const H3_SUBCATEGORY: &str = "minimax_h3";

const H3_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3Checkpoint,
    icon: MI_INVENTORY_2,
    title: "H3 Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(H3_MODEL_OUT),
    fields: NO_FIELDS,
    body: Some(minimax_h3::checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

const H3_TEXT_ENCODER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3TextEncoder,
    icon: MI_TRANSLATE,
    title: "H3 Text Encoder",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_TEXT_ENCODER_INPUTS),
    outputs: PortsSpec::Static(H3_TEXT_ENCODER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::text_encoder::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_TEXT_ENCODER_EXEC,
    on_run: Some(minimax_h3::text_encoder::on_run),
    busy_signal: Some(minimax_h3::text_encoder::busy_signal),
};

const H3_EMPTY_LATENT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3EmptyLatentAv,
    icon: MI_CROP_SQUARE,
    title: "H3 Empty AV Latent",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(H3_EMPTY_LATENT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::latent::empty_latent_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_EMPTY_LATENT_EXEC,
    on_run: None,
    busy_signal: None,
};

const H3_KEYFRAME: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3Keyframe,
    icon: MI_IMAGE_ICON,
    title: "H3 Keyframe",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(IMAGE_IN),
    outputs: PortsSpec::Static(H3_KEYFRAME_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::latent::keyframe_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_KEYFRAME_EXEC,
    on_run: Some(minimax_h3::latent::keyframe_on_run),
    busy_signal: None,
};

const H3_REFERENCES: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3References,
    icon: crate::icons::MI_MOVIE,
    title: "H3 References",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_REFERENCES_INPUTS),
    outputs: PortsSpec::Static(H3_REFERENCES_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::references::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_REFERENCES_EXEC,
    on_run: Some(minimax_h3::references::on_run),
    busy_signal: Some(minimax_h3::references::busy_signal),
};

const H3_SAMPLER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3Sampler,
    icon: MI_AUTO_AWESOME,
    title: "H3 Sampler",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_SAMPLER_INPUTS),
    outputs: PortsSpec::Static(H3_SAMPLER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::sampler::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_SAMPLER_EXEC,
    on_run: Some(minimax_h3::sampler::on_run),
    busy_signal: Some(minimax_h3::sampler::busy_signal),
};

const H3_VAE_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3VaeDecode,
    icon: MI_MOVIE,
    title: "H3 VAE Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_VAE_DECODE_INPUTS),
    outputs: PortsSpec::Static(H3_VAE_DECODE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::decode::vae_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_VAE_DECODE_EXEC,
    on_run: Some(minimax_h3::decode::vae_on_run),
    busy_signal: Some(minimax_h3::decode::vae_busy_signal),
};

const H3_AUDIO_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3AudioDecode,
    icon: MI_GRAPHIC_EQ,
    title: "H3 Audio Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_AUDIO_DECODE_INPUTS),
    outputs: PortsSpec::Static(H3_AUDIO_DECODE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::decode::audio_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_AUDIO_DECODE_EXEC,
    on_run: Some(minimax_h3::decode::audio_on_run),
    busy_signal: Some(minimax_h3::decode::audio_busy_signal),
};

const H3_VIDEO_SAVE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::H3VideoSave,
    icon: MI_SAVE,
    title: "H3 Video Save",
    category: NodeCategory::Output,
    subcategory: Some(H3_SUBCATEGORY),
    inputs: PortsSpec::Static(H3_VIDEO_SAVE_INPUTS),
    outputs: PortsSpec::Static(H3_VIDEO_SAVE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(minimax_h3::save::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &H3_VIDEO_SAVE_EXEC,
    on_run: Some(minimax_h3::save::on_run),
    busy_signal: Some(minimax_h3::save::busy_signal),
};

/// Стабильный ключ каталога строк — отображаемое имя см.
/// `node.subcategory.flux`.
const FLUX_SUBCATEGORY: &str = "flux";

const FLUX_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxCheckpoint,
    icon: MI_INVENTORY_2,
    title: "FLUX Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(FLUX_MODEL_OUT),
    fields: NO_FIELDS,
    body: Some(flux::checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

const FLUX_TEXT_ENCODER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxTextEncoder,
    icon: MI_TRANSLATE,
    title: "FLUX Text Encoder",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_TEXT_ENCODER_INPUTS),
    outputs: PortsSpec::Static(FLUX_TEXT_ENCODER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(flux::text_encoder::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_TEXT_ENCODER_EXEC,
    on_run: Some(flux::text_encoder::on_run),
    busy_signal: Some(flux::text_encoder::busy_signal),
};

const FLUX_EMPTY_LATENT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxEmptyLatent,
    icon: MI_ASPECT_RATIO,
    title: "FLUX Empty Latent",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(FLUX_LATENT_OUT),
    fields: NO_FIELDS,
    body: Some(flux::latent::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_EMPTY_LATENT_EXEC,
    on_run: None,
    busy_signal: None,
};

const FLUX_VAE_ENCODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxVaeEncode,
    icon: MI_IMAGE_ICON,
    title: "FLUX VAE Encode",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_VAE_ENCODE_INPUTS),
    outputs: PortsSpec::Static(FLUX_LATENT_OUT),
    fields: NO_FIELDS,
    body: Some(flux::vae::encode_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_VAE_ENCODE_EXEC,
    on_run: Some(flux::vae::encode_on_run),
    busy_signal: Some(flux::vae::encode_busy_signal),
};

const FLUX_SAMPLER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxSampler,
    icon: MI_AUTO_AWESOME,
    title: "FLUX Sampler",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_SAMPLER_INPUTS),
    outputs: PortsSpec::Static(FLUX_LATENT_OUT),
    fields: NO_FIELDS,
    body: Some(flux::sampler::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_SAMPLER_EXEC,
    on_run: Some(flux::sampler::on_run),
    busy_signal: Some(flux::sampler::busy_signal),
};

const FLUX_VAE_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::FluxVaeDecode,
    icon: MI_PALETTE,
    title: "FLUX VAE Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_VAE_DECODE_INPUTS),
    outputs: PortsSpec::Static(IMAGE_OUT),
    fields: NO_FIELDS,
    body: Some(flux::vae::decode_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX_VAE_DECODE_EXEC,
    on_run: Some(flux::vae::decode_on_run),
    busy_signal: Some(flux::vae::decode_busy_signal),
};

/// Стабильный ключ каталога строк — отображаемое имя см.
/// `node.subcategory.flux2`.
const FLUX2_SUBCATEGORY: &str = "flux2";

const FLUX2_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Flux2Checkpoint,
    icon: MI_INVENTORY_2,
    title: "FLUX.2 Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX2_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(FLUX_MODEL_OUT),
    fields: NO_FIELDS,
    body: Some(flux2::checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX2_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

const FLUX2_TEXT_ENCODER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Flux2TextEncoder,
    icon: MI_TRANSLATE,
    title: "FLUX.2 Text Encoder",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX2_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_TEXT_ENCODER_INPUTS),
    outputs: PortsSpec::Static(FLUX_TEXT_ENCODER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(flux2::text_encoder::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX2_TEXT_ENCODER_EXEC,
    on_run: Some(flux2::text_encoder::on_run),
    busy_signal: Some(flux2::text_encoder::busy_signal),
};

const FLUX2_REFERENCE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Flux2Reference,
    icon: MI_LAYERS,
    title: "FLUX.2 Reference",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX2_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX2_REFERENCE_INPUTS),
    outputs: PortsSpec::Static(FLUX2_REFERENCES_OUT),
    fields: NO_FIELDS,
    body: Some(flux2::reference::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX2_REFERENCE_EXEC,
    on_run: Some(flux2::reference::on_run),
    busy_signal: Some(flux2::reference::busy_signal),
};

const FLUX2_SAMPLER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Flux2Sampler,
    icon: MI_AUTO_AWESOME,
    title: "FLUX.2 Sampler",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX2_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX2_SAMPLER_INPUTS),
    outputs: PortsSpec::Static(FLUX_LATENT_OUT),
    fields: NO_FIELDS,
    body: Some(flux2::sampler::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX2_SAMPLER_EXEC,
    on_run: Some(flux2::sampler::on_run),
    busy_signal: Some(flux2::sampler::busy_signal),
};

const FLUX2_VAE_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::Flux2VaeDecode,
    icon: MI_PALETTE,
    title: "FLUX.2 VAE Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(FLUX2_SUBCATEGORY),
    inputs: PortsSpec::Static(FLUX_VAE_DECODE_INPUTS),
    outputs: PortsSpec::Static(IMAGE_OUT),
    fields: NO_FIELDS,
    body: Some(flux2::vae::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &FLUX2_VAE_DECODE_EXEC,
    on_run: Some(flux2::vae::on_run),
    busy_signal: Some(flux2::vae::busy_signal),
};

const IMAGE_LOAD: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::ImageLoad,
    icon: MI_IMAGE_ICON,
    title: "Image",
    category: NodeCategory::Image,
    subcategory: None,
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(IMAGE_OUT),
    fields: NO_FIELDS,
    body: Some(image::load_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &IMAGE_LOAD_EXEC,
    on_run: None,
    busy_signal: None,
};

const IMAGE_SAVE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::ImageSave,
    icon: MI_SAVE,
    title: "Image Save",
    category: NodeCategory::Output,
    subcategory: None,
    inputs: PortsSpec::Static(IMAGE_IN),
    outputs: PortsSpec::Static(IMAGE_SAVE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(image::save_body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &IMAGE_SAVE_EXEC,
    on_run: Some(image::save_on_run),
    busy_signal: Some(image::save_busy_signal),
};

const LTX_CHECKPOINT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxCheckpoint,
    icon: MI_INVENTORY_2,
    title: "LTX Checkpoint",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(LTX_CHECKPOINT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::checkpoint::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_CHECKPOINT_EXEC,
    on_run: None,
    busy_signal: None,
};

const LTX_TEXT_ENCODER: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxTextEncoder,
    icon: MI_TRANSLATE,
    title: "LTX Text Encoder",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_TEXT_ENCODER_INPUTS),
    outputs: PortsSpec::Static(LTX_TEXT_ENCODER_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::text_encoder::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_TEXT_ENCODER_EXEC,
    on_run: Some(ltx::text_encoder::on_run),
    busy_signal: Some(ltx::text_encoder::busy_signal),
};

const LTX_NAG_PROMPT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxNagPrompt,
    icon: MI_REMOVE_CIRCLE_OUTLINE,
    title: "LTX NAG Prompt",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_NAG_PROMPT_INPUTS),
    outputs: PortsSpec::Static(LTX_NAG_PROMPT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::nag::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_NAG_PROMPT_EXEC,
    on_run: Some(ltx::nag::on_run),
    busy_signal: Some(ltx::nag::busy_signal),
};

const LTX_SAMPLER_STAGE1: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxSamplerStage1,
    icon: MI_BLUR_ON,
    title: "LTX Sampler Stage1",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_SAMPLER_STAGE1_INPUTS),
    outputs: PortsSpec::Static(LTX_SAMPLER_STAGE1_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::sampler_stage1::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_SAMPLER_STAGE1_EXEC,
    on_run: Some(ltx::sampler_stage1::on_run),
    busy_signal: Some(ltx::sampler_stage1::busy_signal),
};

const LTX_UPSCALE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxUpscale,
    icon: MI_ASPECT_RATIO,
    title: "LTX Upscale ×2",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_UPSCALE_INPUTS),
    outputs: PortsSpec::Static(LTX_UPSCALE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::upscale::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_UPSCALE_EXEC,
    on_run: Some(ltx::upscale::on_run),
    busy_signal: Some(ltx::upscale::busy_signal),
};

const LTX_SAMPLER_STAGE2: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxSamplerStage2,
    icon: MI_TUNE,
    title: "LTX Sampler Stage2",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_SAMPLER_STAGE2_INPUTS),
    outputs: PortsSpec::Static(LTX_SAMPLER_STAGE2_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::sampler_stage2::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_SAMPLER_STAGE2_EXEC,
    on_run: Some(ltx::sampler_stage2::on_run),
    busy_signal: Some(ltx::sampler_stage2::busy_signal),
};

const LTX_VAE_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxVaeDecode,
    icon: MI_MOVIE,
    title: "LTX VAE Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_VAE_DECODE_INPUTS),
    outputs: PortsSpec::Static(LTX_VAE_DECODE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::vae_decode::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_VAE_DECODE_EXEC,
    on_run: Some(ltx::vae_decode::on_run),
    busy_signal: Some(ltx::vae_decode::busy_signal),
};

const LTX_AUDIO_DECODE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxAudioDecode,
    icon: MI_GRAPHIC_EQ,
    title: "LTX Audio Decode",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_AUDIO_DECODE_INPUTS),
    outputs: PortsSpec::Static(LTX_AUDIO_DECODE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::audio_decode::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_AUDIO_DECODE_EXEC,
    on_run: Some(ltx::audio_decode::on_run),
    busy_signal: Some(ltx::audio_decode::busy_signal),
};

const LTX_VIDEO_SAVE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxVideoSave,
    icon: MI_SAVE,
    title: "LTX Video Save",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_VIDEO_SAVE_INPUTS),
    outputs: PortsSpec::Static(&[]),
    fields: NO_FIELDS,
    body: Some(ltx::video_save::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_VIDEO_SAVE_EXEC,
    on_run: Some(ltx::video_save::on_run),
    busy_signal: Some(ltx::video_save::busy_signal),
};

const LTX_IMAGE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxImage,
    icon: MI_IMAGE_ICON,
    title: "LTX Image",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(IMAGE_IN),
    outputs: PortsSpec::Static(LTX_IMAGE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::image::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_IMAGE_EXEC,
    on_run: None,
    busy_signal: None,
};

const LTX_VIDEO_INPUT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxVideoInput,
    icon: MI_MOVIE,
    title: "LTX Video Input",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(LTX_VIDEO_INPUT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::video_input::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_VIDEO_INPUT_EXEC,
    on_run: None,
    busy_signal: None,
};

const LTX_RETAKE: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxRetake,
    icon: MI_AUTORENEW,
    title: "LTX Retake",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_RETAKE_INPUTS),
    outputs: PortsSpec::Static(LTX_RETAKE_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::retake::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_RETAKE_EXEC,
    on_run: Some(ltx::retake::on_run),
    busy_signal: Some(ltx::retake::busy_signal),
};

const LTX_IC_LORA: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxIcLora,
    icon: MI_HUB,
    title: "LTX IC-LoRA",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_IC_LORA_INPUTS),
    outputs: PortsSpec::Static(LTX_IC_LORA_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::ic_lora::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_IC_LORA_EXEC,
    on_run: Some(ltx::ic_lora::on_run),
    busy_signal: Some(ltx::ic_lora::busy_signal),
};

const LTX_AUDIO_INPUT: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxAudioInput,
    icon: MI_AUDIOTRACK,
    title: "LTX Audio Input",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(&[]),
    outputs: PortsSpec::Static(LTX_AUDIO_INPUT_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::audio_input::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_AUDIO_INPUT_EXEC,
    on_run: None,
    busy_signal: None,
};

const LTX_LIPDUB: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxLipdub,
    icon: MI_RECORD_VOICE_OVER,
    title: "LTX Lipdub",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_LIPDUB_INPUTS),
    outputs: PortsSpec::Static(LTX_LIPDUB_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::lipdub::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_LIPDUB_EXEC,
    on_run: Some(ltx::lipdub::on_run),
    busy_signal: Some(ltx::lipdub::busy_signal),
};

const LTX_A2V: NodeKindMeta = NodeKindMeta {
    kind: NodeKind::LtxA2V,
    icon: MI_GRAPHIC_EQ,
    title: "LTX A2V",
    category: NodeCategory::Neuro,
    subcategory: Some(LTX_SUBCATEGORY),
    inputs: PortsSpec::Static(LTX_A2V_INPUTS),
    outputs: PortsSpec::Static(LTX_A2V_OUTPUTS),
    fields: NO_FIELDS,
    body: Some(ltx::a2v::body),
    ports_layout: PortsLayout::Rows,
    port_row_extra: None,
    executor: &LTX_A2V_EXEC,
    on_run: Some(ltx::a2v::on_run),
    busy_signal: Some(ltx::a2v::busy_signal),
};

/// Полный список зарегистрированных типов нод. Доступно из меню «Add Node».
pub const REGISTRY: &[NodeKindMeta] = &[
    NUMBER,
    ADD,
    OUTPUT_NODE,
    AUDIO_FILE,
    AUDIO_PLAYER,
    AUDIO_RECORDER,
    GAIN,
    FILTER,
    REVERB,
    EQUALIZER_6,
    EQUALIZER_10,
    EQUALIZER_20,
    EQUALIZER_30,
    MIXER,
    SAVE_TO_FILE,
    MARKDOWN_VIEW,
    ASR_GIGAAM,
    SORTFORMER_DIARIZER,
    TEXT_VIEW,
    OMNIVOICE,
    VOXCPM2,
    VIBEVOICE,
    LLM,
    SYN_CHECKPOINT,
    ACESTEP_VAE_ENCODE,
    ACESTEP_CHECKPOINT,
    ACESTEP_GENERATE,
    FFMPEG_PLAYER,
    LTX_CHECKPOINT,
    LTX_TEXT_ENCODER,
    LTX_NAG_PROMPT,
    LTX_SAMPLER_STAGE1,
    LTX_UPSCALE,
    LTX_SAMPLER_STAGE2,
    LTX_VAE_DECODE,
    LTX_AUDIO_DECODE,
    LTX_VIDEO_SAVE,
    LTX_IMAGE,
    LTX_VIDEO_INPUT,
    LTX_RETAKE,
    LTX_IC_LORA,
    LTX_AUDIO_INPUT,
    LTX_LIPDUB,
    LTX_A2V,
    H3_CHECKPOINT,
    H3_TEXT_ENCODER,
    H3_EMPTY_LATENT,
    H3_KEYFRAME,
    H3_REFERENCES,
    H3_SAMPLER,
    H3_VAE_DECODE,
    H3_AUDIO_DECODE,
    H3_VIDEO_SAVE,
    FLUX_CHECKPOINT,
    FLUX_TEXT_ENCODER,
    FLUX_EMPTY_LATENT,
    FLUX_VAE_ENCODE,
    FLUX_SAMPLER,
    FLUX_VAE_DECODE,
    FLUX2_CHECKPOINT,
    FLUX2_TEXT_ENCODER,
    FLUX2_REFERENCE,
    FLUX2_SAMPLER,
    FLUX2_VAE_DECODE,
    IMAGE_LOAD,
    IMAGE_SAVE,
];

// ── Equalizer helper'ы ───────────────────────────────────────────────────

/// Кол-во полос для эквалайзер-ноды. None для не-EQ kind'ов.
pub fn equalizer_band_count(kind: NodeKind) -> Option<usize> {
    match kind {
        NodeKind::Equalizer6 => Some(6),
        NodeKind::Equalizer10 => Some(10),
        NodeKind::Equalizer20 => Some(20),
        NodeKind::Equalizer30 => Some(30),
        _ => None,
    }
}

/// ISO octave / third-octave частоты для разных n_bands.
///
/// 10 — каноническая ISO octave (31.25 → 16k); 30 — ISO third-octave
/// (25 → 20k). 6 и 20 — равномерно log-spaced, центрированы в [20..20k].
pub fn equalizer_freqs(n_bands: usize) -> &'static [f32] {
    static EQ6_FREQS: [f32; 6] = [80.0, 250.0, 800.0, 2_500.0, 8_000.0, 16_000.0];
    static EQ10_FREQS: [f32; 10] = [
        31.25, 62.5, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
    ];
    static EQ20_FREQS: [f32; 20] = [
        25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1_000.0, 1_600.0, 2_500.0, 4_000.0,
        6_300.0, 10_000.0, 12_500.0, 14_000.0, 16_000.0, 17_500.0, 19_000.0, 20_000.0,
    ];
    static EQ30_FREQS: [f32; 30] = [
        25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
        630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
        6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
    ];
    match n_bands {
        6 => &EQ6_FREQS,
        10 => &EQ10_FREQS,
        20 => &EQ20_FREQS,
        30 => &EQ30_FREQS,
        _ => &EQ10_FREQS,
    }
}

/// Constant-Q для polyband'а: `Q = sqrt(2^B) / (2^B - 1)`,
/// где `B = log2(20k/20) / n_bands` — bandwidth в октавах.
pub fn equalizer_q(n_bands: usize) -> f32 {
    let bw_oct = (20_000.0_f32 / 20.0).log2() / n_bands.max(1) as f32;
    let r = 2.0_f32.powf(bw_oct);
    r.sqrt() / (r - 1.0).max(0.01)
}

// Helpers `equalizer_column_width` / `equalizer_card_width` удалены —
// размер EQ-карточки теперь shrink-to-fit'ится в layout-системе через
// `.node-card { width: fit-content; height: fit-content; }` (см. syngui
// расширение `Dimension::FitContent`).

pub fn meta(kind: NodeKind) -> &'static NodeKindMeta {
    REGISTRY
        .iter()
        .find(|m| m.kind == kind)
        .unwrap_or(&NUMBER)
}

/// Создать карту значений по умолчанию для свежей ноды данного типа.
pub fn default_fields(kind: NodeKind) -> Arc<Mutex<HashMap<&'static str, FieldValue>>> {
    let m = meta(kind);
    let mut map: HashMap<&'static str, FieldValue> = HashMap::new();
    for f in m.fields {
        let v = match f.ty {
            FieldType::Text => FieldValue::Text(use_signal(String::new())),
            FieldType::Float => FieldValue::Float(use_signal(0.0_f32)),
            FieldType::Int => FieldValue::Int(use_signal(0_i32)),
            FieldType::Bool => FieldValue::Bool(use_signal(false)),
            FieldType::Color => FieldValue::Color(use_signal(Color::from_hex("#3B82F6"))),
            FieldType::Choice(_) => FieldValue::Choice(use_signal(0_usize)),
        };
        map.insert(f.name, v);
    }
    Arc::new(Mutex::new(map))
}

/// Создать пустой long-lived `NodeRuntime` для ноды данного типа. Содержит
/// готовые RwSignal'ы (sender'ы для UI) и пустые `Option<AudioPlayer>` /
/// `Option<AudioRecorder>` — реальные инстансы создаются по нажатию кнопки.
pub fn default_runtime(kind: NodeKind) -> Arc<Mutex<NodeRuntime>> {
    let r = match kind {
        NodeKind::AudioFile => NodeRuntime::AudioFile {
            buffer: use_signal(None),
            loaded_path: use_signal(None),
            load_error: use_signal(None),
        },
        NodeKind::AudioPlayer => NodeRuntime::AudioPlayer {
            player: None,
            is_playing: use_signal(false),
            is_paused: use_signal(false),
            progress: use_signal(0.0_f32),
            volume: use_signal(1.0_f32),
            last_input_kind: PrevSource::None,
            pcm_view: use_signal(None),
            pending_stream: Mutex::new(None),
            stream_sample_rate: 0,
            is_streaming: use_signal(false),
        },
        NodeKind::AudioRecorder => NodeRuntime::AudioRecorder {
            session: syngui::audio::RecordingSession::new(syngui::audio::RecordingOptions {
                // Нода публикует декодированный буфер в output-порт сразу
                // после остановки — decode_on_stop кладёт AudioBuffer в
                // `session.last_result()`.
                decode_on_stop: true,
                // Live PCM нужен для downstream-эффект-нод (Gain/Filter/Reverb)
                // и стриминга в Player — открываем стрим сразу при старте.
                open_stream_on_start: true,
                ..Default::default()
            }),
            device: use_signal(None),
        },
        NodeKind::Gain => NodeRuntime::Gain {
            gain_db: use_signal(0.0_f32),
            // 0 dB = linear 1.0; UI пишет f32 → bits.
            live_gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            last_input_kind: PrevSource::None,
            out_stream: None,
            worker: None,
            buffer_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::Filter => NodeRuntime::Filter {
            mode: use_signal(FilterMode::LowPass),
            cutoff_hz: use_signal(1_000.0_f32),
            // FilterMode индекс (0=LP) и cutoff (Hz, f32-bits).
            live_mode: Arc::new(AtomicU8::new(0)),
            live_cutoff: Arc::new(AtomicU32::new(1_000.0_f32.to_bits())),
            // Стартуем грязным, чтобы worker обновил коэффициенты сразу при старте.
            coeffs_dirty: Arc::new(AtomicBool::new(true)),
            biquad: Arc::new(Mutex::new({
                let mut b = Biquad::new();
                b.update_coeffs(BiquadMode::LowPass, 48_000, 1_000.0, 0.707, 0.0);
                b
            })),
            last_input_kind: PrevSource::None,
            out_stream: None,
            worker: None,
            buffer_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::Reverb => NodeRuntime::Reverb {
            mix: use_signal(0.3_f32),
            room: use_signal(0.5_f32),
            live_mix: Arc::new(AtomicU32::new(0.3_f32.to_bits())),
            live_room: Arc::new(AtomicU32::new(0.5_f32.to_bits())),
            params_dirty: Arc::new(AtomicBool::new(true)),
            reverb: Arc::new(Mutex::new(SchroederReverb::new(48_000))),
            last_input_kind: PrevSource::None,
            out_stream: None,
            worker: None,
            buffer_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::Equalizer6
        | NodeKind::Equalizer10
        | NodeKind::Equalizer20
        | NodeKind::Equalizer30 => {
            let n = equalizer_band_count(kind).unwrap_or(10);
            NodeRuntime::Equalizer {
                n_bands: n,
                gains_db: (0..n).map(|_| use_signal(0.0_f32)).collect(),
                live_gains: (0..n)
                    .map(|_| Arc::new(AtomicU32::new(0.0_f32.to_bits())))
                    .collect(),
                coeffs_dirty: Arc::new(AtomicBool::new(true)),
                biquads: Arc::new(Mutex::new(Vec::new())),
                last_input_kind: PrevSource::None,
                out_stream: None,
                worker: None,
                buffer_cache: Arc::new(Mutex::new(None)),
            }
        }
        NodeKind::SaveToFile => NodeRuntime::SaveToFile {
            path: use_signal(default_save_path()),
            is_writing: use_signal(false),
            written_seconds: use_signal(0.0_f64),
            written_bytes: use_signal(0_u64),
            status: use_signal(SaveStatus::Idle),
            pending_rx: Arc::new(Mutex::new(None)),
            has_upstream: use_signal(false),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            last_input_kind: PrevSource::None,
        },
        NodeKind::Mixer => {
            use crate::pages::node_editor::types::MIXER_MAX_INPUTS;
            NodeRuntime::Mixer {
                // 2 — минимально-полезный микшер; UI-spinbox растит до MIXER_MAX_INPUTS.
                n_inputs: use_signal(2_usize),
                gains_db: (0..MIXER_MAX_INPUTS).map(|_| use_signal(0.0_f32)).collect(),
                live_gains: (0..MIXER_MAX_INPUTS)
                    .map(|_| Arc::new(AtomicU32::new(1.0_f32.to_bits())))
                    .collect(),
                last_input_kinds: vec![PrevSource::None; MIXER_MAX_INPUTS],
                out_stream: None,
                worker: None,
                buffer_cache: Arc::new(Mutex::new(None)),
            }
        }
        NodeKind::MarkdownView => {
            use syngui::core::Size;
            NodeRuntime::MarkdownView {
                content: use_signal(default_markdown_content()),
                edit_mode: use_signal(false),
                resize_mode: use_signal(false),
                size: use_signal(Size::new(320.0, 200.0)),
            }
        }
        NodeKind::AsrGigaam => {
            use crate::pages::node_editor::types::PrevSource;
            NodeRuntime::AsrGigaam {
                model_path: use_signal(None),
                // 0 = CPU, 1 = GPU auto. См. nodes::asr_gigaam::DEVICE_OPTIONS.
                device_idx: use_signal(0_usize),
                // f16 — оптимум для GigaAM на CUDA. На CPU все равно
                // fallback'нется в F32 внутри Transcriber::load.
                storage_idx: use_signal(asr_gigaam::default_storage_idx()),
                compute_idx: use_signal(asr_gigaam::default_compute_idx()),
                transcriber: Arc::new(Mutex::new(None)),
                loaded_cfg: Arc::new(Mutex::new(None)),
                running: use_signal(false),
                error: use_signal(None),
                loaded_name: use_signal(None),
                output_text: use_signal(String::new()),
                text_version: use_signal(0_u32),
                last_input_kind: PrevSource::None,
            }
        }
        NodeKind::TextView => {
            use syngui::core::Size;
            NodeRuntime::TextView {
                output_text: use_signal(String::new()),
                text_version: use_signal(0_u32),
                last_input: Arc::new(Mutex::new(None)),
                resize_mode: use_signal(false),
                // 280×360 — вертикалка под одну колонку транскрипта. Пользователь
                // может изменить размер через ContextMenu → «Изменить размер».
                size: use_signal(Size::new(280.0, 360.0)),
            }
        }
        NodeKind::OmniVoice => NodeRuntime::OmniVoice {
            model_path: use_signal(None),
            // 0 = CPU, 1 = GPU auto. См. nodes::omnivoice::DEVICE_OPTIONS.
            device_idx: use_signal(0_usize),
            storage_idx: use_signal(omnivoice::default_storage_idx()),
            compute_idx: use_signal(omnivoice::default_compute_idx()),
            instruct: use_signal(String::new()),
            ref_text_field: use_signal(String::new()),
            // GigaAM — RU-моноязычная; OmniVoice мультиязычная, но дефолт
            // совпадает с типичным workflow synthos.
            language: use_signal("ru".to_string()),
            // Дефолты из synaptix::facade::tts::core::GenerationConfig::default()
            // (см. synaptix omnivoice config).
            num_step: use_signal(32_u32),
            guidance_scale: use_signal(2.0_f32),
            t_shift: use_signal(0.1_f32),
            speed: use_signal(1.0_f32),
            seed: use_signal(0_u64),
            pipeline: Arc::new(Mutex::new(None)),
            loaded_cfg: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            output_buf: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::VoxCpm2 => NodeRuntime::VoxCpm2 {
            model_path: use_signal(None),
            device_idx: use_signal(voxcpm2::default_device_idx()),
            compute_idx: use_signal(voxcpm2::default_compute_idx()),
            prompt_text_field: use_signal(String::new()),
            cfg_value: use_signal(2.0_f32),
            n_timesteps: use_signal(10_u32),
            max_len: use_signal(2000_u32),
            seed: use_signal(1988_u64),
            pipeline: Arc::new(Mutex::new(None)),
            loaded_cfg: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            output_buf: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::VibeVoice => NodeRuntime::VibeVoice {
            model_path: use_signal(None),
            device_idx: use_signal(vibevoice::default_device_idx()),
            compute_idx: use_signal(vibevoice::default_compute_idx()),
            script_field: use_signal(String::new()),
            cfg_value: use_signal(1.3_f32),
            ddpm_steps: use_signal(20_u32),
            max_length_times: use_signal(2.0_f32),
            seed: use_signal(0_u64),
            pipeline: Arc::new(Mutex::new(None)),
            loaded_cfg: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            progress: use_signal(0_u32),
            output_buf: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::SynCheckpoint => NodeRuntime::SynCheckpoint {
            model_path: use_signal(None),
            device_idx: use_signal(0_usize),
            storage_idx: use_signal(0_usize),
            compute_idx: use_signal(0_usize),
            // Дефолт — слотовое поведение: так слот-ноды жили всегда.
            resident: use_signal(true),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::Llm => NodeRuntime::Llm {
            model_path: use_signal(None),
            device_idx: use_signal(llm::default_device_idx()),
            quant_idx: use_signal(llm::default_quant_idx()),
            compute_idx: use_signal(llm::default_compute_idx()),
            system_prompt: use_signal(String::new()),
            context: use_signal(4096_u32),
            think: use_signal(false),
            max_tokens: use_signal(512_u32),
            temperature: use_signal(0.7_f32),
            top_k: use_signal(0_u32),
            top_p: use_signal(1.0_f32),
            min_p: use_signal(0.0_f32),
            repetition_penalty: use_signal(1.0_f32),
            seed: use_signal(0_u64),
            pipeline: Arc::new(Mutex::new(None)),
            loaded_cfg: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            output_text: use_signal(String::new()),
            text_version: use_signal(0_u32),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        NodeKind::SortformerDiarizer => {
            use crate::pages::node_editor::types::PrevSource;
            NodeRuntime::SortformerDiarizer {
                model_path: use_signal(None),
                device_idx: use_signal(0_usize),
                storage_idx: use_signal(sortformer_diarizer::default_storage_idx()),
                compute_idx: use_signal(sortformer_diarizer::default_compute_idx()),
                threshold: use_signal(0.5_f32),
                allow_overlap: use_signal(true),
                diarizer: Arc::new(Mutex::new(None)),
                loaded_cfg: Arc::new(Mutex::new(None)),
                running: use_signal(false),
                error: use_signal(None),
                loaded_name: use_signal(None),
                output_pretty: use_signal(String::new()),
                output_json: use_signal(String::new()),
                text_version: use_signal(0_u32),
                last_input_kind: PrevSource::None,
            }
        }
        NodeKind::AceStepVaeEncode => NodeRuntime::AceStepVaeEncode {
            // 1 = GPU, как у ACE-Step Checkpoint. Было 0 = CPU: агент видел
            // его в «state (example with defaults)», «возвращал дефолт» — и
            // энкод трека в 3,5 минуты уходил на процессор.
            device_idx: use_signal(1_usize),
            storage_idx: use_signal(acestep::default_storage_idx()),
            compute_idx: use_signal(acestep::default_compute_idx()),
            chunk_seconds: use_signal(30.0_f32),
            overlap_seconds: use_signal(0.8_f32),
            loaded_cfg: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            output_buf: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        NodeKind::FfmpegPlayer => NodeRuntime::FfmpegPlayer {
            player: Arc::new(Mutex::new(None)),
            current_path: use_signal(None),
            load_error: use_signal(None),
            is_playing: use_signal(false),
            is_paused: use_signal(false),
            progress: use_signal(0.0_f32),
            duration: use_signal(0.0_f32),
            position: use_signal(0.0_f32),
            volume: use_signal(1.0_f32),
            hwaccel_idx: use_signal(0_usize),
            size: use_signal(syngui::core::Size::new(320.0, 240.0)),
            video_out: Arc::new(Mutex::new(None)),
            audio_out: Arc::new(Mutex::new(None)),
            out_version: use_signal(0_u32),
            frames_in: Arc::new(Mutex::new(None)),
            audio_in: Arc::new(Mutex::new(None)),
            mem_audio: Arc::new(Mutex::new(None)),
            preview_version: use_signal(0_u32),
        },
        NodeKind::H3Checkpoint => NodeRuntime::H3Checkpoint {
            model_path: use_signal(None),
            encoder_path: use_signal(None),
            lora_path: use_signal(None),
            lora_strength: use_signal(1.0_f32),
            variant_idx: use_signal(0_usize),
            device_idx: use_signal(0_usize),
            quant_dit_idx: use_signal(0_usize),
            quant_enc_idx: use_signal(0_usize),
            compute_idx: use_signal(0_usize),
            memory_mode_idx: use_signal(0_usize),
            // Резидентность по умолчанию выключена — прежнее weak-поведение.
            resident: use_signal(false),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::H3TextEncoder => NodeRuntime::H3TextEncoder {
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3EmptyLatentAv => NodeRuntime::H3EmptyLatentAv {
            width: use_signal(1344_u32),
            height: use_signal(768_u32),
            duration_seconds: use_signal(5.0_f32),
            aspect_idx: use_signal(0_usize),
        },
        NodeKind::H3Keyframe => NodeRuntime::H3Keyframe {
            path: use_signal(None),
            frame_slot_idx: use_signal(0_usize),
            resize_idx: use_signal(0_usize),
            image: Arc::new(Mutex::new(None)),
            error: use_signal(None),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3References => NodeRuntime::H3References {
            items: use_signal(Vec::new()),
            image_size_idx: use_signal(0_usize),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3Sampler => NodeRuntime::H3Sampler {
            steps: use_signal(20_u32),
            cfg_scale: use_signal(5.0_f32),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3VaeDecode => NodeRuntime::H3VaeDecode {
            running: use_signal(false),
            error: use_signal(None),
            frames: Arc::new(Mutex::new(None)),
            preview_version: use_signal(0_u32),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3AudioDecode => NodeRuntime::H3AudioDecode {
            running: use_signal(false),
            error: use_signal(None),
            buffer: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::H3VideoSave => NodeRuntime::H3VideoSave {
            path: use_signal(None),
            running: use_signal(false),
            error: use_signal(None),
            saved: use_signal(None),
            preview: Arc::new(Mutex::new(None)),
            preview_version: use_signal(0_u32),
        },
        NodeKind::FluxCheckpoint => NodeRuntime::FluxCheckpoint {
            model_path: use_signal(None),
            device_idx: use_signal(0_usize),
            quant_idx: use_signal(flux::DEFAULT_QUANT_IDX),
            memory_mode_idx: use_signal(0_usize),
            resident: use_signal(false),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::FluxTextEncoder => NodeRuntime::FluxTextEncoder {
            seq_len_idx: use_signal(0_usize),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::FluxEmptyLatent => NodeRuntime::FluxEmptyLatent {
            width: use_signal(flux::latent::DEFAULT_SIDE),
            height: use_signal(flux::latent::DEFAULT_SIDE),
            aspect_idx: use_signal(0_usize),
        },
        NodeKind::FluxVaeEncode => NodeRuntime::FluxVaeEncode {
            resize_idx: use_signal(0_usize),
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::FluxSampler => NodeRuntime::FluxSampler {
            steps: use_signal(flux::sampler::DEFAULT_STEPS),
            guidance: use_signal(flux::sampler::DEFAULT_GUIDANCE),
            seed: use_signal(0_u64),
            denoise: use_signal(1.0_f32),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::FluxVaeDecode => NodeRuntime::FluxVaeDecode {
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::Flux2Checkpoint => NodeRuntime::Flux2Checkpoint {
            model_path: use_signal(None),
            device_idx: use_signal(0_usize),
            quant_idx: use_signal(flux2::DEFAULT_QUANT_IDX),
            memory_mode_idx: use_signal(0_usize),
            resident: use_signal(false),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::Flux2TextEncoder => NodeRuntime::Flux2TextEncoder {
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::Flux2Reference => NodeRuntime::Flux2Reference {
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::Flux2Sampler => NodeRuntime::Flux2Sampler {
            steps: use_signal(flux2::sampler::DEFAULT_STEPS),
            guidance: use_signal(flux2::sampler::DEFAULT_GUIDANCE),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::Flux2VaeDecode => NodeRuntime::Flux2VaeDecode {
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::ImageLoad => NodeRuntime::ImageLoad {
            path: use_signal(None),
            error: use_signal(None),
            cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::ImageSave => NodeRuntime::ImageSave {
            path: use_signal(None),
            running: use_signal(false),
            error: use_signal(None),
            saved: use_signal(None),
            preview: Arc::new(Mutex::new(None)),
            preview_version: use_signal(0_u32),
        },
        NodeKind::LtxCheckpoint => NodeRuntime::LtxCheckpoint {
            model_path: use_signal(None),
            gemma_dir: use_signal(None),
            upscaler_path: use_signal(None),
            lora_path: use_signal(None),
            lora_strength: use_signal(1.0_f32),
            device_idx: use_signal(0_usize),
            quant_dit_idx: use_signal(0_usize),
            quant_enc_idx: use_signal(0_usize),
            compute_idx: use_signal(0_usize),
            // Резидентность по умолчанию выключена — прежнее weak-поведение.
            resident: use_signal(false),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::LtxTextEncoder => NodeRuntime::LtxTextEncoder {
            prompt_field: use_signal(String::new()),
            keep_gemma: use_signal(false),
            gemma_keep: Arc::new(Mutex::new(None)),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxNagPrompt => NodeRuntime::LtxNagPrompt {
            prompt_field: use_signal(
                synaptix_video_ltx23::pipeline::DEFAULT_NAG_PROMPT.to_string(),
            ),
            scale: use_signal(synaptix_video_ltx23::pipeline::NAG_DEFAULT_SCALE),
            alpha: use_signal(synaptix_video_ltx23::pipeline::NAG_DEFAULT_ALPHA),
            tau: use_signal(synaptix_video_ltx23::pipeline::NAG_DEFAULT_TAU),
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxSamplerStage1 => NodeRuntime::LtxSamplerStage1 {
            width: use_signal(1024_u32),
            height: use_signal(576_u32),
            duration_seconds: use_signal(10.0_f32),
            fps_idx: use_signal(0_usize),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxUpscale => NodeRuntime::LtxUpscale {
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxSamplerStage2 => NodeRuntime::LtxSamplerStage2 {
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxVaeDecode => NodeRuntime::LtxVaeDecode {
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            frames_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
            preview_version: use_signal(0_u32),
        },
        NodeKind::LtxAudioDecode => NodeRuntime::LtxAudioDecode {
            running: use_signal(false),
            error: use_signal(None),
            out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxVideoSave => NodeRuntime::LtxVideoSave {
            path: use_signal(String::new()),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            status: use_signal(SaveStatus::Idle),
        },
        NodeKind::LtxImage => NodeRuntime::LtxImage {
            image_path: use_signal(None),
            strength: use_signal(1.0_f32),
            frame_idx: use_signal(0_u32),
            error: use_signal(None),
            cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::LtxVideoInput => NodeRuntime::LtxVideoInput {
            video_path: use_signal(None),
        },
        NodeKind::LtxRetake => NodeRuntime::LtxRetake {
            width: use_signal(1024_u32),
            height: use_signal(576_u32),
            duration_seconds: use_signal(10.0_f32),
            fps_idx: use_signal(0_usize),
            retake_start: use_signal(0.0_f32),
            retake_end: use_signal(2.0_f32),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxIcLora => NodeRuntime::LtxIcLora {
            width: use_signal(1024_u32),
            height: use_signal(576_u32),
            duration_seconds: use_signal(10.0_f32),
            fps_idx: use_signal(0_usize),
            downscale: use_signal(2_u32),
            ref_strength: use_signal(1.0_f32),
            control_idx: use_signal(0_usize),
            canny_low: use_signal(0.1_f32),
            canny_high: use_signal(0.3_f32),
            depth_model_path: use_signal(None),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxAudioInput => NodeRuntime::LtxAudioInput {
            audio_path: use_signal(None),
        },
        NodeKind::LtxLipdub => NodeRuntime::LtxLipdub {
            width: use_signal(1024_u32),
            height: use_signal(576_u32),
            duration_seconds: use_signal(10.0_f32),
            fps_idx: use_signal(0_usize),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::LtxA2V => NodeRuntime::LtxA2V {
            width: use_signal(1024_u32),
            height: use_signal(576_u32),
            duration_seconds: use_signal(10.0_f32),
            fps_idx: use_signal(0_usize),
            seed: use_signal(0_u64),
            running: use_signal(false),
            error: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            cancel: Arc::new(AtomicBool::new(false)),
            v_out: Arc::new(Mutex::new(None)),
            a_out: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        NodeKind::AceStepCheckpoint => NodeRuntime::AceStepCheckpoint {
            // Каталог моделей приложения — чтобы нода из шаблона/меню сразу
            // резолвила 4 бандла по дефолтным именам без ручного выбора.
            models_dir: use_signal(Some(acestep::app_models_dir())),
            lm_path: use_signal(None),
            text_encoder_path: use_signal(None),
            dit_path: use_signal(None),
            vae_path: use_signal(None),
            // GPU по умолчанию: инференс ACE-Step на CPU нереалистичен
            // (минуты). DEVICE_OPTIONS[1] = "GPU (auto)".
            device_idx: use_signal(1_usize),
            quant_dit_idx: use_signal(acestep::default_storage_idx()),
            quant_enc_idx: use_signal(acestep::default_storage_idx()),
            compute_idx: use_signal(acestep::default_compute_idx()),
            // Резидентность по умолчанию выключена — прежнее weak-поведение.
            resident: use_signal(false),
            handle_cache: Arc::new(Mutex::new(None)),
        },
        NodeKind::AceStepGenerate => NodeRuntime::AceStepGenerate {
            mode_idx: use_signal(0_usize),
            preset: use_signal(SamplerPreset::Auto),
            duration_seconds: use_signal(-1.0_f32),
            infer_steps: use_signal(32_u32),
            cfg_scale: use_signal(7.0_f32),
            flow_match_shift: use_signal(1.0_f32),
            seed: use_signal(0_u64),
            temperature: use_signal(0.85_f32),
            top_p: use_signal(0.9_f32),
            top_k: use_signal(0_u32),
            min_p: use_signal(0.0_f32),
            lm_cfg_scale: use_signal(2.0_f32),
            use_cot: use_signal(true),
            use_ar: use_signal(true),
            bpm: use_signal(0_u32),
            keyscale_idx: use_signal(0_usize),
            timesig_idx: use_signal(0_usize),
            norm_mode: use_signal(PostNormMode::Peak),
            enable_dcw: use_signal(false),
            dcw_mode: use_signal(crate::pages::node_editor::types::DcwModeOption::Double),
            dcw_scaler: use_signal(0.02_f32),
            dcw_high_scaler: use_signal(0.06_f32),
            dcw_wavelet: use_signal(crate::pages::node_editor::types::DcwWaveletOption::Haar),
            dcw_preset: use_signal(crate::pages::node_editor::types::DcwPresetOption::Think),
            retake_variance: use_signal(0.5_f32),
            retake_seed: use_signal(0_u64),
            repaint_start_sec: use_signal(0.0_f32),
            repaint_end_sec: use_signal(-1.0_f32),
            repaint_strength: use_signal(0.7_f32),
            edit_n_min: use_signal(0.0_f32),
            edit_n_max: use_signal(1.0_f32),
            track_idx: use_signal(0_usize),
            running: use_signal(false),
            error: use_signal(None),
            loaded_name: use_signal(None),
            progress_pct: use_signal(0.0_f32),
            output_buf_audio: Arc::new(Mutex::new(None)),
            output_buf_latent: Arc::new(Mutex::new(None)),
            output_version: use_signal(0_u32),
        },
        _ => NodeRuntime::None,
    };
    Arc::new(Mutex::new(r))
}

/// Дефолтный markdown-текст для свежей `MarkdownView`-ноды.
/// Краткая подсказка по управлению режимами и обзор поддерживаемого синтаксиса.
fn default_markdown_content() -> String {
    tr!("nodes.markdown.default_content")
}

/// Дефолтный путь для SaveToFile-ноды: `~/Downloads/synthos-<unix>.wav`.
/// Если `$HOME` недоступен — `synthos-<unix>.wav` в cwd.
fn default_save_path() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("synthos-{secs}.wav");
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join("Downloads")
            .join(name)
            .to_string_lossy()
            .to_string();
    }
    name
}

/// Иконка-логотип для иконки-кнопки иконки. (workaround чтобы импорт MI_AUDIOTRACK
/// не помечался как unused: используется в иконе вкладки превью).
#[allow(dead_code)]
pub const AUDIO_BADGE_ICON: &str = MI_AUDIOTRACK;
