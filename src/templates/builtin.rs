//! Встроенные шаблоны графа. Не лежат на диске — генерируются в коде,
//! чтобы пакет synthos был самодостаточен и при первом запуске у юзера
//! сразу было что попробовать.
//!
//! `builtin: true` запрещает удалять/переименовывать; «Duplicate to
//! Custom» через [`super::storage::duplicate_to_custom`] делает копию,
//! которую уже можно править.

use super::model::{
    AceStepCheckpointStateData,
    AceStepGenerateStateData,
    AceStepVaeStateData,
    ConnData,
    Yue2CheckpointStateData,
    Yue2GenerateStateData,
    Yue2TranscribeStateData,
    FieldValueData,
    LtxSamplerStage1StateData,
    NodeData,
    NodeStateData,
    PointData,
    Template,
    TemplateKind,
    TextViewStateData,
    VibeVoiceStateData,
};
use crate::pages::node_editor::types::NodeKind;

/// Все встроенные шаблоны. Порядок ⇒ порядок отображения в панели.
/// Шаблоны со стадиями моделей в службе (FLUX.1, FLUX.2, Qwen-Image,
/// SDXL, MiniMax-H3) — JSON: переведены из прежних родных нод
/// [`super::native`].
const STAGE_TEMPLATES: &[(&str, &str)] = &[
    ("builtin-flux-image-to-image", include_str!("builtin/builtin-flux-image-to-image.json")),
    ("builtin-flux-text-to-image", include_str!("builtin/builtin-flux-text-to-image.json")),
    ("builtin-flux2-edit", include_str!("builtin/builtin-flux2-edit.json")),
    ("builtin-flux2-image-to-image", include_str!("builtin/builtin-flux2-image-to-image.json")),
    ("builtin-flux2-llm-upsampling", include_str!("builtin/builtin-flux2-llm-upsampling.json")),
    ("builtin-flux2-multi-reference", include_str!("builtin/builtin-flux2-multi-reference.json")),
    ("builtin-flux2-text-to-image", include_str!("builtin/builtin-flux2-text-to-image.json")),
    ("builtin-h3-first-frame", include_str!("builtin/builtin-h3-first-frame.json")),
    ("builtin-h3-first-last", include_str!("builtin/builtin-h3-first-last.json")),
    ("builtin-h3-flux-keyframe", include_str!("builtin/builtin-h3-flux-keyframe.json")),
    ("builtin-h3-ref2va", include_str!("builtin/builtin-h3-ref2va.json")),
    ("builtin-h3-text-to-video", include_str!("builtin/builtin-h3-text-to-video.json")),
    ("builtin-h3-turbo", include_str!("builtin/builtin-h3-turbo.json")),
    ("builtin-ltx-flux-keyframe", include_str!("builtin/builtin-ltx-flux-keyframe.json")),
    ("builtin-qwen-image-edit", include_str!("builtin/builtin-qwen-image-edit.json")),
    ("builtin-qwen-image-multi-edit", include_str!("builtin/builtin-qwen-image-multi-edit.json")),
    ("builtin-qwen-image21-edit", include_str!("builtin/builtin-qwen-image21-edit.json")),
    ("builtin-qwen-image21-llm-rewrite", include_str!("builtin/builtin-qwen-image21-llm-rewrite.json")),
    ("builtin-qwen-image21-multi-reference", include_str!("builtin/builtin-qwen-image21-multi-reference.json")),
    ("builtin-qwen-image21-text-to-image", include_str!("builtin/builtin-qwen-image21-text-to-image.json")),
    ("builtin-qwen-image21-transparent", include_str!("builtin/builtin-qwen-image21-transparent.json")),
    ("builtin-sdxl-image-to-image", include_str!("builtin/builtin-sdxl-image-to-image.json")),
    ("builtin-sdxl-text-to-image", include_str!("builtin/builtin-sdxl-text-to-image.json")),
];

fn stage(id: &str) -> Template {
    let raw = STAGE_TEMPLATES.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("нет шаблона {id}"));
    let mut t: Template = serde_json::from_str(raw).unwrap_or_else(|e| panic!("шаблон {id}: {e}"));
    t.id = id.into();
    t.builtin = true;
    t
}

pub fn all() -> Vec<Template> {
    vec![
        empty_template(),
        simple_add_template(),
        audio_playback_template(),
        voice_recording_template(),
        audio_effects_chain_template(),
        save_to_file_template(),
        mix_two_sources_template(),
        mix_mic_and_file_template(),
        voxcpm_voice_clone_template(),
        omnivoice_voice_clone_template(),
        vibevoice_dialogue_template(),
        vibevoice_single_voice_template(),
        vibevoice_llm_podcast_template(),
        acestep_text2music_template(),
        acestep_retake_template(),
        acestep_repaint_template(),
        acestep_extend_template(),
        acestep_edit_template(),
        acestep_cover_template(),
        acestep_extract_template(),
        yue2_song_template(),
        yue2_edit_score_template(),
        yue2_cover_template(),
        stage("builtin-h3-text-to-video"),
        stage("builtin-h3-turbo"),
        stage("builtin-h3-first-frame"),
        stage("builtin-h3-first-last"),
        stage("builtin-h3-ref2va"),
        ltx_text_to_video_template(),
        ltx_image_to_video_template(),
        ltx_retake_template(),
        ltx_ic_lora_template(),
        ltx_lipdub_template(),
        ltx_a2v_template(),
        stage("builtin-flux-text-to-image"),
        stage("builtin-flux-image-to-image"),
        stage("builtin-flux2-text-to-image"),
        stage("builtin-flux2-edit"),
        stage("builtin-flux2-image-to-image"),
        stage("builtin-flux2-llm-upsampling"),
        stage("builtin-flux2-multi-reference"),
        stage("builtin-qwen-image-edit"),
        stage("builtin-qwen-image-multi-edit"),
        stage("builtin-qwen-image21-text-to-image"),
        stage("builtin-qwen-image21-edit"),
        stage("builtin-qwen-image21-multi-reference"),
        stage("builtin-qwen-image21-transparent"),
        stage("builtin-qwen-image21-llm-rewrite"),
        stage("builtin-sdxl-text-to-image"),
        stage("builtin-sdxl-image-to-image"),
        stage("builtin-ltx-flux-keyframe"),
        stage("builtin-h3-flux-keyframe"),
    ]
}

// ── FLUX.2 ────────────────────────────────────────────────────────────────




// ── Qwen-Image ────────────────────────────────────────────────────────────




// ── Qwen-Image 2.1 ────────────────────────────────────────────────────────









// ── SDXL ──────────────────────────────────────────────────────────────────





// ── FLUX.1 ────────────────────────────────────────────────────────────────










fn ltx_a2v_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "a dynamic music video, vibrant motion synced to the beat".into(),
        width: 280.0,
        height: 90.0,
    });
    Template {
        id: "builtin-ltx-a2v".into(),
        builtin: true,
        name: "LTX: Audio to Video".into(),
        description:
            "Аудио (Audio Input) фиксирует звуковую дорожку, промпт задаёт сцену → \
             Text Encoder → A2V (stage1 video-denoise при frozen-аудио → upscale ×2 → \
             stage2 refine) → VAE Decode + Audio Decode → Видеоплеер / Save. В \
             Checkpoint укажите чекпойнт LTX-2.3, Gemma и upscaler; в Audio Input — звук."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 560.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 360.0),
            node_plain(14, NodeKind::LtxAudioInput, 480.0, 60.0),
            node_with_state(
                5,
                NodeKind::LtxA2V,
                920.0,
                160.0,
                NodeStateData::LtxA2V(crate::templates::model::LtxA2VStateData::default()),
            ),
            node_plain(8, NodeKind::LtxVaeDecode, 1380.0, 80.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1380.0, 440.0),
            node_plain(11, NodeKind::LtxVideoSave, 1820.0, 60.0),
            node_plain(12, NodeKind::FfmpegPlayer, 1820.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 14, from_port: "audio".into(), to_node: 5, to_port: "audio".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}

fn ltx_lipdub_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "a person speaking to the camera".into(),
        width: 280.0,
        height: 80.0,
    });
    Template {
        id: "builtin-ltx-lipdub".into(),
        builtin: true,
        name: "LTX: Lipdub".into(),
        description:
            "Reference-видео (лицо) + речь (Audio Input) → синхрон губ под речь: \
             Text Encoder → Lipdub (stage1 A/V append ref+audio → upscale → stage2 \
             refine) → VAE Decode + Audio Decode → Видеоплеер / Save. В Checkpoint \
             укажите чекпойнт LTX-2.3, Gemma, upscaler и lipdub-IC-LoRA (поле LoRA)."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 620.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 420.0),
            node_plain(13, NodeKind::LtxVideoInput, 480.0, 60.0),
            node_plain(14, NodeKind::LtxAudioInput, 480.0, 220.0),
            node_with_state(
                5,
                NodeKind::LtxLipdub,
                920.0,
                160.0,
                NodeStateData::LtxLipdub(crate::templates::model::LtxLipdubStateData::default()),
            ),
            node_plain(8, NodeKind::LtxVaeDecode, 1380.0, 80.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1380.0, 440.0),
            node_plain(11, NodeKind::LtxVideoSave, 1820.0, 60.0),
            node_plain(12, NodeKind::FfmpegPlayer, 1820.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 13, from_port: "video".into(), to_node: 5, to_port: "ref_video".into() },
            ConnData { from_node: 14, from_port: "audio".into(), to_node: 5, to_port: "audio".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}

fn ltx_ic_lora_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "a vibrant scene, cinematic lighting".into(),
        width: 280.0,
        height: 90.0,
    });
    Template {
        id: "builtin-ltx-ic-lora".into(),
        builtin: true,
        name: "LTX: IC-LoRA".into(),
        description:
            "Reference-видео (LTX Video Input) задаёт структуру/движение, промпт — \
             содержание → Text Encoder → IC-LoRA (append ref-control) → VAE Decode + \
             Audio Decode → Видеоплеер / Save. В Checkpoint укажите чекпойнт LTX-2.3, \
             Gemma и IC-LoRA-адаптер (поле LoRA); в Video Input — reference-видео."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 560.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 360.0),
            node_plain(13, NodeKind::LtxVideoInput, 480.0, 60.0),
            node_with_state(
                5,
                NodeKind::LtxIcLora,
                920.0,
                160.0,
                NodeStateData::LtxIcLora(crate::templates::model::LtxIcLoraStateData::default()),
            ),
            node_plain(8, NodeKind::LtxVaeDecode, 1380.0, 80.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1380.0, 440.0),
            node_plain(11, NodeKind::LtxVideoSave, 1820.0, 60.0),
            node_plain(12, NodeKind::FfmpegPlayer, 1820.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 13, from_port: "video".into(), to_node: 5, to_port: "ref_video".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}

fn ltx_retake_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "same scene, smooth natural motion".into(),
        width: 280.0,
        height: 90.0,
    });
    Template {
        id: "builtin-ltx-retake".into(),
        builtin: true,
        name: "LTX: Retake".into(),
        description:
            "Исходное видео (LTX Video Input) + промпт → Text Encoder → Retake \
             (регенерация региона [начало,конец] сек, остальное frozen) → VAE Decode \
             + Audio Decode → Видеоплеер / Save. Одностадийно (без upscale). В \
             Checkpoint укажите чекпойнт LTX-2.3 и Gemma; в Video Input — видео."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 560.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 360.0),
            node_plain(13, NodeKind::LtxVideoInput, 480.0, 60.0),
            node_with_state(
                5,
                NodeKind::LtxRetake,
                920.0,
                160.0,
                NodeStateData::LtxRetake(crate::templates::model::LtxRetakeStateData::default()),
            ),
            node_plain(8, NodeKind::LtxVaeDecode, 1380.0, 80.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1380.0, 440.0),
            node_plain(11, NodeKind::LtxVideoSave, 1820.0, 60.0),
            node_plain(12, NodeKind::FfmpegPlayer, 1820.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 13, from_port: "video".into(), to_node: 5, to_port: "video".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}

fn ltx_image_to_video_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "the scene comes alive, gentle camera motion, natural light".into(),
        width: 280.0,
        height: 100.0,
    });
    Template {
        id: "builtin-ltx-image-to-video".into(),
        builtin: true,
        name: "LTX: Image to Video".into(),
        description:
            "Изображение (кадр 0) + промпт → Text Encoder → Sampler Stage1 (A/V, image-cond, \
             NAG) → Upscale ×2 → Stage2-refine → VAE Decode + Audio Decode → Видеоплеер / Save. \
             В Checkpoint укажите чекпойнт LTX-2.3, Gemma и upscaler; в LTX Image — картинку."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 560.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 420.0),
            node_plain(4, NodeKind::LtxNagPrompt, 480.0, 60.0),
            node_plain(13, NodeKind::LtxImage, 480.0, 700.0),
            node_with_state(
                5,
                NodeKind::LtxSamplerStage1,
                920.0,
                200.0,
                NodeStateData::LtxSamplerStage1(LtxSamplerStage1StateData::default()),
            ),
            node_plain(6, NodeKind::LtxUpscale, 1380.0, 60.0),
            node_plain(7, NodeKind::LtxSamplerStage2, 1380.0, 280.0),
            node_plain(8, NodeKind::LtxVaeDecode, 1840.0, 120.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1840.0, 480.0),
            node_plain(11, NodeKind::LtxVideoSave, 2280.0, 60.0),
            node_plain(12, NodeKind::FfmpegPlayer, 2280.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 4, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 6, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 7, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 4, from_port: "nag".into(), to_node: 5, to_port: "nag".into() },
            ConnData { from_node: 13, from_port: "image_cond".into(), to_node: 5, to_port: "image_cond".into() },
            ConnData { from_node: 13, from_port: "image_cond".into(), to_node: 7, to_port: "image_cond".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 6, to_port: "video_latent".into() },
            ConnData { from_node: 6, from_port: "video_latent".into(), to_node: 7, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 7, to_port: "audio_tokens".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 7, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 7, to_port: "audio_encoding".into() },
            ConnData { from_node: 7, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 7, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}


fn ltx_text_to_video_template() -> Template {
    let prompt_state = NodeStateData::TextView(TextViewStateData {
        output_text: "a woman in a cozy cafe, talking warmly to the camera, soft daylight, \
                      gentle background chatter"
            .into(),
        width: 280.0,
        height: 120.0,
    });
    Template {
        id: "builtin-ltx-text-to-video".into(),
        builtin: true,
        name: "LTX: Text to Video".into(),
        description:
            "Промпт → Text Encoder (Gemma) → Sampler Stage1 (A/V, NAG) → Upscale ×2 → \
             Stage2-refine → VAE Decode + Audio Decode → Video Save (mp4) и Видеоплеер \
             (кадры+звук из памяти, параллельно Save). В Checkpoint-ноде укажите \
             чекпойнт LTX-2.3, Gemma и upscaler — .syn-бандлы из каталога моделей \
             (или .safetensors + HF-каталог Gemma). Чекпойнт нужен ИМЕННО \
             distilled (ltx-2.3-22b-distilled-*): стадии идут по \
             distilled-расписанию 8+3 шага, на dev-модели видео выйдет размытым."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_plain(1, NodeKind::LtxCheckpoint, 60.0, 60.0),
            node_with_state(2, NodeKind::TextView, 60.0, 560.0, prompt_state),
            node_plain(3, NodeKind::LtxTextEncoder, 480.0, 420.0),
            node_plain(4, NodeKind::LtxNagPrompt, 480.0, 60.0),
            node_with_state(
                5,
                NodeKind::LtxSamplerStage1,
                920.0,
                200.0,
                NodeStateData::LtxSamplerStage1(LtxSamplerStage1StateData::default()),
            ),
            node_plain(6, NodeKind::LtxUpscale, 1380.0, 60.0),
            node_plain(7, NodeKind::LtxSamplerStage2, 1380.0, 280.0),
            node_plain(8, NodeKind::LtxVaeDecode, 1840.0, 120.0),
            node_plain(9, NodeKind::LtxAudioDecode, 1840.0, 480.0),
            node_plain(11, NodeKind::LtxVideoSave, 2280.0, 60.0),
            // Видеоплеер из памяти (кадры VAE Decode + аудио) — параллельно Save.
            node_plain(12, NodeKind::FfmpegPlayer, 2280.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 4, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 6, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 7, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 8, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 5, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 5, to_port: "audio_encoding".into() },
            ConnData { from_node: 4, from_port: "nag".into(), to_node: 5, to_port: "nag".into() },
            ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 6, to_port: "video_latent".into() },
            ConnData { from_node: 6, from_port: "video_latent".into(), to_node: 7, to_port: "video_latent".into() },
            ConnData { from_node: 5, from_port: "audio_tokens".into(), to_node: 7, to_port: "audio_tokens".into() },
            ConnData { from_node: 3, from_port: "video_encoding".into(), to_node: 7, to_port: "video_encoding".into() },
            ConnData { from_node: 3, from_port: "audio_encoding".into(), to_node: 7, to_port: "audio_encoding".into() },
            ConnData { from_node: 7, from_port: "video_latent".into(), to_node: 8, to_port: "video_latent".into() },
            ConnData { from_node: 7, from_port: "audio_tokens".into(), to_node: 9, to_port: "audio_tokens".into() },
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 11, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 11, to_port: "audio".into() },
            // Видеоплеер из памяти — параллельно Save (кадры + аудио).
            ConnData { from_node: 8, from_port: "frames".into(), to_node: 12, to_port: "frames".into() },
            ConnData { from_node: 9, from_port: "audio".into(), to_node: 12, to_port: "audio".into() },
        ],
        viewport: None,
    }
}

fn empty_template() -> Template {
    Template {
        id: "builtin-empty".into(),
        builtin: true,
        name: "Empty".into(),
        description: "Пустой канвас — старт с нуля.".into(),
        kind: TemplateKind::Full,
        nodes: Vec::new(),
        connections: Vec::new(),
        viewport: None,
    }
}

fn simple_add_template() -> Template {
    let mut a_fields = std::collections::BTreeMap::new();
    a_fields.insert("value".into(), FieldValueData::Float(2.0));
    let mut b_fields = std::collections::BTreeMap::new();
    b_fields.insert("value".into(), FieldValueData::Float(3.0));

    Template {
        id: "builtin-simple-add".into(),
        builtin: true,
        name: "Simple Add".into(),
        description: "Number + Number → Add → Output. Чистая арифметика.".into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::Number,
                pos: PointData { x: 80.0, y: 80.0 },
                fields: a_fields,
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::Number,
                pos: PointData { x: 80.0, y: 220.0 },
                fields: b_fields,
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 3,
                kind: NodeKind::Add,
                pos: PointData { x: 380.0, y: 140.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 4,
                kind: NodeKind::Output,
                pos: PointData { x: 660.0, y: 160.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "out".into(), to_node: 3, to_port: "a".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "b".into() },
            ConnData { from_node: 3, from_port: "out".into(), to_node: 4, to_port: "in".into() },
        ],
        viewport: None,
    }
}

fn audio_playback_template() -> Template {
    Template {
        id: "builtin-audio-playback".into(),
        builtin: true,
        name: "Audio Playback".into(),
        description: "Audio File → Audio Player. Открой WAV, нажми Play."
            .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioFile,
                pos: PointData { x: 80.0, y: 100.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::AudioPlayer,
                pos: PointData { x: 380.0, y: 100.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![ConnData {
            from_node: 1,
            from_port: "out".into(),
            to_node: 2,
            to_port: "in".into(),
        }],
        viewport: None,
    }
}

fn voice_recording_template() -> Template {
    Template {
        id: "builtin-voice-recording".into(),
        builtin: true,
        name: "Voice Recording".into(),
        description: "Audio Recorder → Audio Player. Запиши голос и сразу проиграй."
            .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioRecorder,
                pos: PointData { x: 80.0, y: 100.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::AudioPlayer,
                pos: PointData { x: 400.0, y: 100.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![ConnData {
            from_node: 1,
            from_port: "out".into(),
            to_node: 2,
            to_port: "in".into(),
        }],
        viewport: None,
    }
}

/// Recorder → Gain → Filter → Reverb → Player. Полная streaming-цепочка
/// эффектов поверх AudioStream — каждый эффект-нод обрабатывает чанки
/// в собственном worker-потоке без full-buffer.
fn audio_effects_chain_template() -> Template {
    Template {
        id: "builtin-audio-effects-chain".into(),
        builtin: true,
        name: "Audio Effects Chain".into(),
        description:
            "Recorder → Gain → Filter → Reverb → Player. Запиши голос и услышь его \
             с эффектами в реальном времени."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioRecorder,
                pos: PointData { x: 60.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::Gain,
                pos: PointData { x: 580.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 3,
                kind: NodeKind::Filter,
                pos: PointData { x: 980.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 4,
                kind: NodeKind::Reverb,
                pos: PointData { x: 1440.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 5,
                kind: NodeKind::AudioPlayer,
                pos: PointData { x: 1900.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![
            ConnData { from_node: 1, from_port: "out".into(), to_node: 2, to_port: "in".into() },
            ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "in".into() },
            ConnData { from_node: 3, from_port: "out".into(), to_node: 4, to_port: "in".into() },
            ConnData { from_node: 4, from_port: "out".into(), to_node: 5, to_port: "in".into() },
        ],
        viewport: None,
    }
}

/// Recorder → Save to File. Запись с микрофона прямо в WAV без проигрывания.
fn save_to_file_template() -> Template {
    Template {
        id: "builtin-save-to-file".into(),
        builtin: true,
        name: "Save Recording".into(),
        description: "Audio Recorder → Save to File. Запись микрофона в WAV-файл (PCM 16-bit)."
            .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioRecorder,
                pos: PointData { x: 80.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::SaveToFile,
                pos: PointData { x: 600.0, y: 120.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![ConnData {
            from_node: 1,
            from_port: "out".into(),
            to_node: 2,
            to_port: "in".into(),
        }],
        viewport: None,
    }
}

/// Два audio-source через Mixer (n=2) → Player. Демонстрирует работу
/// микшера в простейшей конфигурации. Mixer стартует с дефолтным
/// `n_inputs=2` (см. `default_runtime` в registry) — этого достаточно для
/// связей `in_1`/`in_2`. Если нужно больше входов — можно открыть spinbox
/// в карточке и нарастить N до 16.
fn mix_two_sources_template() -> Template {
    Template {
        id: "builtin-mix-two-sources".into(),
        builtin: true,
        name: "Mix 2 Sources".into(),
        description:
            "Audio File + Audio File → Mixer (2 источника) → Audio Player. \
             Базовый шаблон микширования двух треков."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioFile,
                pos: PointData { x: 80.0, y: 80.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::AudioFile,
                pos: PointData { x: 80.0, y: 280.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 3,
                kind: NodeKind::Mixer,
                pos: PointData { x: 580.0, y: 160.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 4,
                kind: NodeKind::AudioPlayer,
                pos: PointData { x: 1080.0, y: 160.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![
            ConnData {
                from_node: 1,
                from_port: "out".into(),
                to_node: 3,
                to_port: "in_1".into(),
            },
            ConnData {
                from_node: 2,
                from_port: "out".into(),
                to_node: 3,
                to_port: "in_2".into(),
            },
            ConnData {
                from_node: 3,
                from_port: "out".into(),
                to_node: 4,
                to_port: "in".into(),
            },
        ],
        viewport: None,
    }
}

// ── ACE-Step builtin templates ────────────────────────────────────────────
//
// Все 4 ACE-Step шаблона опираются на 8 нод из категории «Нейро → ACE-Step»
// и пробрасывают `device_idx=1` (GPU auto) в state, чтобы при первом
// запуске пользователь не получал CPU-инференс по умолчанию. `model_path`
// остаётся `None` — выбирается через file-picker в карточке ноды.

/// Конструктор ноды без полей (`fields` пустой, `enabled=true`, дефолтный
/// `style`). Используется для большинства ACE-Step builtin'ов — у нод
/// нет registry-fields, всё состояние живёт в `state`.
/// Voice-clone TTS на VoxCPM2: референс-голос (Audio File) + текст (Text) →
/// VoxCPM2 (`ref_audio`) → плеер и сохранение в файл.
fn voxcpm_voice_clone_template() -> Template {
    let text_state = NodeStateData::TextView(TextViewStateData {
        output_text: "Привет! Это синтез моего голоса из короткого образца."
            .into(),
        width: 280.0,
        height: 120.0,
    });
    Template {
        id: "builtin-voice-clone-voxcpm".into(),
        builtin: true,
        name: "VoxCPM: Voice Clone".into(),
        description:
            "Референс-голос (Audio File) + текст (Text) → VoxCPM2 TTS (клон по ref_audio) → \
             Audio Player и Save to File. Откройте короткий WAV с образцом голоса, впишите \
             текст, укажите .syn-модель VoxCPM в ноде Syn Checkpoint и нажмите Run."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::TextView, 60.0, 60.0, text_state),
            node_plain(2, NodeKind::AudioFile, 60.0, 320.0),
            node_plain(6, NodeKind::SynCheckpoint, 60.0, 560.0),
            node_plain(3, NodeKind::VoxCpm2, 480.0, 140.0),
            node_plain(4, NodeKind::AudioPlayer, 900.0, 60.0),
            node_plain(5, NodeKind::SaveToFile, 900.0, 300.0),
        ],
        connections: vec![
            ConnData { from_node: 6, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "out".into(),   to_node: 3, to_port: "text".into() },
            ConnData { from_node: 2, from_port: "out".into(),   to_node: 3, to_port: "ref_audio".into() },
            ConnData { from_node: 3, from_port: "audio".into(), to_node: 4, to_port: "in".into() },
            ConnData { from_node: 3, from_port: "audio".into(), to_node: 5, to_port: "in".into() },
        ],
        viewport: None,
    }
}

/// Voice-clone TTS на OmniVoice с авто-транскрипцией референса через GigaAM:
/// Audio File → GigaAM ASR → Text View (транскрипт, можно поправить) →
/// `ref_text` OmniVoice; тот же Audio File → `ref_audio` (активирует
/// Clone-mode). RunQueue сам дождётся GigaAM перед стартом OmniVoice.
fn omnivoice_voice_clone_template() -> Template {
    let text_state = NodeStateData::TextView(TextViewStateData {
        output_text: "Привет! Это синтез моего голоса из короткого образца."
            .into(),
        width: 280.0,
        height: 120.0,
    });
    Template {
        id: "builtin-voice-clone-omnivoice".into(),
        builtin: true,
        name: "OmniVoice: Voice Clone".into(),
        description:
            "Референс-голос (Audio File) → GigaAM ASR (авто-транскрипт → ref text) + текст (Text) → \
             OmniVoice TTS (Clone-mode по ref_audio) → Audio Player и Save to File. Откройте WAV с \
             образцом голоса, впишите текст, укажите .syn-модели в нодах Syn Checkpoint: GigaAM \
             (gigaam-v3.syn) для ASR и OmniVoice (omnivoice.syn) для TTS, нажмите Run. Транскрипт \
             в Text View можно поправить перед синтезом."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::TextView, 60.0, 60.0, text_state),
            node_plain(2, NodeKind::AudioFile, 60.0, 320.0),
            node_plain(8, NodeKind::SynCheckpoint, 60.0, 560.0),
            node_plain(6, NodeKind::AsrGigaam, 420.0, 320.0),
            node_plain(7, NodeKind::TextView, 420.0, 580.0),
            node_plain(9, NodeKind::SynCheckpoint, 420.0, 60.0),
            node_plain(3, NodeKind::OmniVoice, 840.0, 140.0),
            node_plain(4, NodeKind::AudioPlayer, 1280.0, 60.0),
            node_plain(5, NodeKind::SaveToFile, 1280.0, 300.0),
        ],
        connections: vec![
            ConnData { from_node: 8, from_port: "model".into(), to_node: 6, to_port: "model".into() },
            ConnData { from_node: 9, from_port: "model".into(), to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "out".into(),   to_node: 3, to_port: "text".into() },
            ConnData { from_node: 2, from_port: "out".into(),   to_node: 3, to_port: "ref_audio".into() },
            ConnData { from_node: 2, from_port: "out".into(),   to_node: 6, to_port: "in".into() },
            ConnData { from_node: 6, from_port: "out".into(),   to_node: 7, to_port: "in".into() },
            ConnData { from_node: 7, from_port: "out".into(),   to_node: 3, to_port: "ref_text".into() },
            ConnData { from_node: 3, from_port: "audio".into(), to_node: 4, to_port: "in".into() },
            ConnData { from_node: 3, from_port: "audio".into(), to_node: 5, to_port: "in".into() },
        ],
        viewport: None,
    }
}








fn vibevoice_dialogue_template() -> Template {
    let script_state = NodeStateData::TextView(TextViewStateData {
        output_text: "Speaker 1: Привет! Сегодня мы записываем первый выпуск.\n\
                      Speaker 2: Отлично, начнём с главного вопроса."
            .into(),
        width: 320.0,
        height: 140.0,
    });
    Template {
        id: "builtin-vibevoice-dialogue".into(),
        builtin: true,
        name: "VibeVoice: диалог двух голосов".into(),
        description:
            "Сценарий вида «Speaker 1: …» / «Speaker 2: …» плюс два коротких WAV с образцами \
             голосов → VibeVoice синтезирует сплошную дорожку диалога 24 кГц. Укажите \
             vibevoice-1.5b.syn в ноде Syn Checkpoint и нажмите Run."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::TextView, 60.0, 60.0, script_state),
            node_plain(2, NodeKind::AudioFile, 60.0, 340.0),
            node_plain(3, NodeKind::AudioFile, 60.0, 520.0),
            node_plain(4, NodeKind::SynCheckpoint, 60.0, 700.0),
            node_with_state(5, NodeKind::VibeVoice, 520.0, 140.0, vibevoice_state()),
            node_plain(6, NodeKind::AudioPlayer, 980.0, 60.0),
            node_plain(7, NodeKind::SaveToFile, 980.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 4, from_port: "model".into(), to_node: 5, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "out".into(),   to_node: 5, to_port: "script".into() },
            ConnData { from_node: 2, from_port: "out".into(),   to_node: 5, to_port: "voice1".into() },
            ConnData { from_node: 3, from_port: "out".into(),   to_node: 5, to_port: "voice2".into() },
            ConnData { from_node: 5, from_port: "audio".into(), to_node: 6, to_port: "in".into() },
            ConnData { from_node: 5, from_port: "audio".into(), to_node: 7, to_port: "in".into() },
        ],
        viewport: None,
    }
}

fn vibevoice_single_voice_template() -> Template {
    let script_state = NodeStateData::TextView(TextViewStateData {
        output_text: "Speaker 1: Это длинный текст, озвученный моим голосом из короткого образца."
            .into(),
        width: 320.0,
        height: 120.0,
    });
    Template {
        id: "builtin-vibevoice-single".into(),
        builtin: true,
        name: "VibeVoice: клон одного голоса".into(),
        description:
            "Один WAV с образцом голоса + текст → VibeVoice озвучивает его целиком (модель \
             держит контекст на десятки минут). Модель — vibevoice-1.5b.syn или \
             vibevoice-7b.syn в ноде Syn Checkpoint."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::TextView, 60.0, 60.0, script_state),
            node_plain(2, NodeKind::AudioFile, 60.0, 320.0),
            node_plain(3, NodeKind::SynCheckpoint, 60.0, 520.0),
            node_with_state(4, NodeKind::VibeVoice, 520.0, 120.0, vibevoice_state()),
            node_plain(5, NodeKind::AudioPlayer, 980.0, 60.0),
            node_plain(6, NodeKind::SaveToFile, 980.0, 320.0),
        ],
        connections: vec![
            ConnData { from_node: 3, from_port: "model".into(), to_node: 4, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "out".into(),   to_node: 4, to_port: "script".into() },
            ConnData { from_node: 2, from_port: "out".into(),   to_node: 4, to_port: "voice1".into() },
            ConnData { from_node: 4, from_port: "audio".into(), to_node: 5, to_port: "in".into() },
            ConnData { from_node: 4, from_port: "audio".into(), to_node: 6, to_port: "in".into() },
        ],
        viewport: None,
    }
}

fn vibevoice_llm_podcast_template() -> Template {
    let topic_state = NodeStateData::TextView(TextViewStateData {
        output_text: "Напиши сценарий подкаста на 6 реплик о том, как работают нейросети.                       Формат строк строго «Speaker 1: …» и «Speaker 2: …», без заголовков."
            .into(),
        width: 320.0,
        height: 140.0,
    });
    Template {
        id: "builtin-vibevoice-llm-podcast".into(),
        builtin: true,
        name: "VibeVoice: подкаст из темы (LLM → голос)".into(),
        description:
            "Тема → LLM пишет сценарий строками «Speaker N: …» → Text View (правится вручную) \
             → VibeVoice озвучивает двумя голосами → плеер и сохранение. Нужны два \
             Syn Checkpoint: LLM-бандл и vibevoice-*.syn."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::TextView, 60.0, 60.0, topic_state),
            node_plain(2, NodeKind::SynCheckpoint, 60.0, 340.0),
            node_plain(3, NodeKind::Llm, 460.0, 60.0),
            node_plain(4, NodeKind::TextView, 900.0, 60.0),
            node_plain(5, NodeKind::AudioFile, 60.0, 560.0),
            node_plain(6, NodeKind::AudioFile, 60.0, 740.0),
            node_plain(7, NodeKind::SynCheckpoint, 460.0, 560.0),
            node_with_state(8, NodeKind::VibeVoice, 900.0, 400.0, vibevoice_state()),
            node_plain(9, NodeKind::AudioPlayer, 1360.0, 340.0),
            node_plain(10, NodeKind::SaveToFile, 1360.0, 600.0),
        ],
        connections: vec![
            ConnData { from_node: 2, from_port: "model".into(),  to_node: 3, to_port: "model".into() },
            ConnData { from_node: 1, from_port: "out".into(),    to_node: 3, to_port: "prompt".into() },
            ConnData { from_node: 3, from_port: "answer".into(), to_node: 4, to_port: "in".into() },
            ConnData { from_node: 4, from_port: "out".into(),    to_node: 8, to_port: "script".into() },
            ConnData { from_node: 7, from_port: "model".into(),  to_node: 8, to_port: "model".into() },
            ConnData { from_node: 5, from_port: "out".into(),    to_node: 8, to_port: "voice1".into() },
            ConnData { from_node: 6, from_port: "out".into(),    to_node: 8, to_port: "voice2".into() },
            ConnData { from_node: 8, from_port: "audio".into(),  to_node: 9, to_port: "in".into() },
            ConnData { from_node: 8, from_port: "audio".into(),  to_node: 10, to_port: "in".into() },
        ],
        viewport: None,
    }
}

fn vibevoice_state() -> NodeStateData {
    NodeStateData::VibeVoice(VibeVoiceStateData {
        model_path: None,
        device_idx: 0,
        compute_idx: 0,
        script: String::new(),
        cfg_value: 1.3,
        ddpm_steps: 20,
        max_length_times: 2.0,
        seed: 0,
    })
}

fn node_with_state(id: u64, kind: NodeKind, x: f32, y: f32, state: NodeStateData) -> NodeData {
    NodeData {
        id,
        kind,
        pos: PointData { x, y },
        fields: Default::default(),
        style: Default::default(),
        enabled: true,
        state: Some(state),
    }
}

/// Нода без `state` (Pack, AudioFile, AudioPlayer). `default_runtime` сам
/// инициализирует значения по умолчанию.
fn node_plain(id: u64, kind: NodeKind, x: f32, y: f32) -> NodeData {
    NodeData {
        id,
        kind,
        pos: PointData { x, y },
        fields: Default::default(),
        style: Default::default(),
        enabled: true,
        state: None,
    }
}


// ── ACE-Step builtin-шаблоны (по одному на режим Generate-ноды) ──

fn conn(fr: u64, fp: &str, to: u64, tp: &str) -> ConnData {
    ConnData { from_node: fr, from_port: fp.into(), to_node: to, to_port: tp.into() }
}

fn acestep_text_state(text: &str, h: f32) -> NodeStateData {
    NodeStateData::TextView(TextViewStateData { output_text: text.into(), width: 240.0, height: h })
}

fn acestep_checkpoint_state() -> NodeStateData {
    NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
        models_dir: None,
        device_idx: 1, // GPU
        ..Default::default()
    })
}

fn acestep_vae_encode_state() -> NodeStateData {
    NodeStateData::AceStepVaeEncode(AceStepVaeStateData {
        chunk_seconds: 30.0,
        overlap_seconds: 0.5,
        ..Default::default()
    })
}

fn acestep_gen(mode_idx: usize, tweak: impl FnOnce(&mut AceStepGenerateStateData)) -> NodeStateData {
    let mut s = AceStepGenerateStateData { mode_idx, ..Default::default() };
    tweak(&mut s);
    NodeStateData::AceStepGenerate(s)
}

/// Шаблон без исходного аудио (text2music / retake): Checkpoint + tags + lyrics
/// → Generate → AudioPlayer.
fn acestep_t2m_like(id: &str, name: &str, desc: &str, tags: &str, gen: NodeStateData) -> Template {
    Template {
        id: id.into(),
        builtin: true,
        name: name.into(),
        description: desc.into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::AceStepCheckpoint, 60.0, 60.0, acestep_checkpoint_state()),
            node_with_state(2, NodeKind::TextView, 60.0, 360.0, acestep_text_state(tags, 80.0)),
            node_with_state(3, NodeKind::TextView, 60.0, 500.0, acestep_text_state("", 200.0)),
            node_with_state(4, NodeKind::AceStepGenerate, 480.0, 180.0, gen),
            node_plain(5, NodeKind::AudioPlayer, 980.0, 200.0),
        ],
        connections: vec![
            conn(1, "model", 4, "model"),
            conn(2, "out", 4, "tags"),
            conn(3, "out", 4, "lyrics"),
            conn(4, "audio", 5, "in"),
        ],
        viewport: None,
    }
}

/// Шаблон с исходным аудио (repaint / extend / edit / cover / extract):
/// AudioFile → VaeEncode → Generate.src_latent + Checkpoint + tags + lyrics →
/// Generate → AudioPlayer.
fn acestep_audio_cond(id: &str, name: &str, desc: &str, tags: &str, gen: NodeStateData) -> Template {
    Template {
        id: id.into(),
        builtin: true,
        name: name.into(),
        description: desc.into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::AceStepCheckpoint, 60.0, 60.0, acestep_checkpoint_state()),
            node_plain(2, NodeKind::AudioFile, 60.0, 360.0),
            node_with_state(3, NodeKind::AceStepVaeEncode, 380.0, 360.0, acestep_vae_encode_state()),
            node_with_state(4, NodeKind::TextView, 60.0, 600.0, acestep_text_state(tags, 80.0)),
            node_with_state(5, NodeKind::TextView, 60.0, 740.0, acestep_text_state("", 200.0)),
            node_with_state(6, NodeKind::AceStepGenerate, 820.0, 220.0, gen),
            node_plain(7, NodeKind::AudioPlayer, 1320.0, 240.0),
        ],
        connections: vec![
            conn(1, "model", 6, "model"),
            conn(1, "model", 3, "model"),
            conn(2, "out", 3, "audio"),
            conn(3, "latent", 6, "src_latent"),
            conn(4, "out", 6, "tags"),
            conn(5, "out", 6, "lyrics"),
            conn(6, "audio", 7, "in"),
        ],
        viewport: None,
    }
}

fn acestep_text2music_template() -> Template {
    acestep_t2m_like(
        "builtin-acestep-text2music",
        "ACE-Step: Text→Music",
        "Checkpoint + tags/lyrics → Generate (text2music) → плеер. Базовая генерация музыки из текста.",
        "energetic electronic dance, driving synths, 128 bpm",
        acestep_gen(0, |_| {}),
    )
}

/// Партитура и лирика показываются текстовыми нодами: их можно прочитать,
/// поправить и подать назад во вход `abc` — на этом и держится «белый ящик»
/// YuE2.
fn yue2_checkpoint_state() -> NodeStateData {
    NodeStateData::Yue2Checkpoint(Yue2CheckpointStateData {
        models_dir: None,
        device_idx: 1, // GPU
        ..Default::default()
    })
}

fn yue2_gen(cot_idx: usize, seconds: f32) -> NodeStateData {
    NodeStateData::Yue2Generate(Yue2GenerateStateData {
        cot_idx,
        seconds,
        ..Default::default()
    })
}

/// Checkpoint + стиль/лирика → Generate → плеер, а партитура выводится в
/// текстовую ноду.
fn yue2_song_like(
    id: &str,
    name: &str,
    desc: &str,
    style: &str,
    lyrics: &str,
    gen: NodeStateData,
    with_abc_input: bool,
) -> Template {
    let mut nodes = vec![
        node_with_state(1, NodeKind::Yue2Checkpoint, 60.0, 60.0, yue2_checkpoint_state()),
        node_with_state(2, NodeKind::TextView, 60.0, 380.0, acestep_text_state(style, 80.0)),
        node_with_state(3, NodeKind::TextView, 60.0, 520.0, acestep_text_state(lyrics, 220.0)),
        node_with_state(4, NodeKind::Yue2Generate, 480.0, 180.0, gen),
        node_plain(5, NodeKind::AudioPlayer, 1000.0, 200.0),
        // Партитура с выхода `score`: её видно целиком и можно скопировать.
        node_with_state(6, NodeKind::TextView, 1000.0, 420.0, acestep_text_state("", 260.0)),
    ];
    let mut connections = vec![
        conn(1, "model", 4, "model"),
        conn(2, "out", 4, "style"),
        conn(3, "out", 4, "lyrics"),
        conn(4, "audio", 5, "in"),
        conn(4, "score", 6, "in"),
    ];
    if with_abc_input {
        // Партитура из отдельной ноды сильнее сгенерированной: сюда вставляется
        // правленый ABC.
        nodes.push(node_with_state(7, NodeKind::TextView, 60.0, 820.0, acestep_text_state("", 260.0)));
        connections.push(conn(7, "out", 4, "abc"));
    }
    Template {
        id: id.into(),
        builtin: true,
        name: name.into(),
        description: desc.into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

fn yue2_song_template() -> Template {
    yue2_song_like(
        "builtin-yue2-song",
        "YuE2: песня",
        "Стиль и лирика → партитура с аккордами → песня; партитуру видно в текстовой ноде.",
        "english, dream pop, female vocal, warm synths, soft drums",
        "[verse]\nTonight I'm awake, watching city lights\n[chorus]\nHold on, hold on, the morning comes slow",
        yue2_gen(0, 60.0),
        false,
    )
}

fn yue2_edit_score_template() -> Template {
    yue2_song_like(
        "builtin-yue2-edit-score",
        "YuE2: рендер по партитуре",
        "Партитура берётся из ноды abc: вставьте правленый ABC и отрендерьте заново.",
        "english, jazz-funk, warm lead vocal, Rhodes, bass and drums",
        "[verse]\nTonight I'm awake, watching city lights",
        yue2_gen(0, 60.0),
        true,
    )
}

/// Кавер записи: SheetSage2 снимает мелодию (вокал и инструментальную тему,
/// без аккордов), она видна и правится в текстовой ноде, а Generate с
/// `cot = melody` поёт её в новом стиле — аккомпанемент придумывается заново
/// (так кавер и задуман в релизе). Длительность 0 — песня идёт, пока не
/// кончится партитура.
fn yue2_cover_template() -> Template {
    Template {
        id: "builtin-yue2-cover".into(),
        builtin: true,
        name: "YuE2: кавер записи".into(),
        description: "Запись → мелодия без аккордов (SheetSage2, правится в ноде) → песня в новом стиле с новой лирикой."
            .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(1, NodeKind::Yue2Checkpoint, 60.0, 60.0, yue2_checkpoint_state()),
            node_plain(2, NodeKind::AudioFile, 60.0, 420.0),
            node_with_state(
                3,
                NodeKind::Yue2Transcribe,
                440.0,
                420.0,
                NodeStateData::Yue2Transcribe(Yue2TranscribeStateData::default()),
            ),
            // Мелодия записи: её видно целиком, можно поправить до генерации.
            node_with_state(4, NodeKind::TextView, 820.0, 420.0, acestep_text_state("", 260.0)),
            node_with_state(
                5,
                NodeKind::TextView,
                440.0,
                60.0,
                acestep_text_state("english, acoustic folk, male vocal, fingerpicked guitar", 80.0),
            ),
            node_with_state(
                6,
                NodeKind::TextView,
                440.0,
                180.0,
                acestep_text_state("[verse]\nWe were younger then, the road was long", 200.0),
            ),
            node_with_state(7, NodeKind::Yue2Generate, 1200.0, 60.0, yue2_gen(1, 0.0)),
            node_plain(8, NodeKind::AudioPlayer, 1720.0, 80.0),
        ],
        connections: vec![
            conn(1, "model", 3, "model"),
            conn(2, "out", 3, "audio"),
            conn(3, "score", 4, "in"),
            conn(1, "model", 7, "model"),
            conn(5, "out", 7, "style"),
            conn(6, "out", 7, "lyrics"),
            conn(4, "out", 7, "abc"),
            conn(7, "audio", 8, "in"),
        ],
        viewport: None,
    }
}

fn acestep_retake_template() -> Template {
    acestep_t2m_like(
        "builtin-acestep-retake",
        "ACE-Step: Retake",
        "Вариация трека при тех же тегах (микс двух шумовых seed'ов). Retake variance задаёт силу вариации; src-аудио не нужно.",
        "energetic electronic dance, driving synths, 128 bpm",
        acestep_gen(1, |s| s.retake_variance = 0.5),
    )
}

fn acestep_repaint_template() -> Template {
    acestep_audio_cond(
        "builtin-acestep-repaint",
        "ACE-Step: Repaint",
        "Перегенерация региона [start,end] исходного аудио (вне региона — оригинал). ⚠ Установи Duration = длине исходного аудио.",
        "energetic electronic dance, driving synths",
        acestep_gen(2, |s| {
            s.repaint_start_sec = 2.0;
            s.repaint_end_sec = 4.0;
            s.repaint_strength = 0.6;
        }),
    )
}

fn acestep_extend_template() -> Template {
    acestep_audio_cond(
        "builtin-acestep-extend",
        "ACE-Step: Extend",
        "Продление аудио: оригинал сохраняется, новый хвост генерируется. ⚠ Установи Duration > длины исходного, регион = новая часть (start=длина src, end=-1).",
        "energetic electronic dance, driving synths",
        acestep_gen(3, |s| {
            s.repaint_start_sec = 5.0;
            s.repaint_end_sec = -1.0;
            s.repaint_strength = 0.9;
        }),
    )
}

fn acestep_edit_template() -> Template {
    acestep_audio_cond(
        "builtin-acestep-edit",
        "ACE-Step: Edit",
        "Морфинг исходного аудио к новому промпту (SDEdit: ре-шум до n_max → денойз). n_max=0 ⇒ без изменений, ближе к 1 ⇒ сильнее. ⚠ Duration = длине src.",
        "calm acoustic guitar, soft and warm",
        acestep_gen(4, |s| s.edit_n_max = 0.6),
    )
}

fn acestep_cover_template() -> Template {
    acestep_audio_cond(
        "builtin-acestep-cover",
        "ACE-Step: Cover",
        "Кавер: исходное аудио → контекст DiT, теги задают новый стиль/инструменты. ⚠ Duration = длине src.",
        "jazz cover, smooth saxophone, brushed drums",
        acestep_gen(5, |_| {}),
    )
}

fn acestep_extract_template() -> Template {
    acestep_audio_cond(
        "builtin-acestep-extract",
        "ACE-Step: Extract",
        "Выделение стема из микса: исходное аудио → контекст DiT, поле «Дорожка» у Generate выбирает стем (vocals, drums, bass, guitar…), теги — его имя. Для вокала впиши текст песни в lyrics: без него голос выходит без слов. LM не участвует, длина = длине исходника.",
        "vocals",
        acestep_gen(6, |_| {}),
    )
}

/// Микрофон + audio-файл через Mixer → Player. Удобно для караоке-сценариев
/// (живой голос поверх backing track).
fn mix_mic_and_file_template() -> Template {
    Template {
        id: "builtin-mix-mic-and-file".into(),
        builtin: true,
        name: "Mic + File".into(),
        description:
            "Audio Recorder + Audio File → Mixer → Audio Player. \
             Караоке-сценарий: голос поверх трека."
                .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            NodeData {
                id: 1,
                kind: NodeKind::AudioRecorder,
                pos: PointData { x: 80.0, y: 80.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 2,
                kind: NodeKind::AudioFile,
                pos: PointData { x: 80.0, y: 300.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 3,
                kind: NodeKind::Mixer,
                pos: PointData { x: 580.0, y: 180.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
            NodeData {
                id: 4,
                kind: NodeKind::AudioPlayer,
                pos: PointData { x: 1080.0, y: 180.0 },
                fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: None,
            },
        ],
        connections: vec![
            ConnData {
                from_node: 1,
                from_port: "out".into(),
                to_node: 3,
                to_port: "in_1".into(),
            },
            ConnData {
                from_node: 2,
                from_port: "out".into(),
                to_node: 3,
                to_port: "in_2".into(),
            },
            ConnData {
                from_node: 3,
                from_port: "out".into(),
                to_node: 4,
                to_port: "in".into(),
            },
        ],
        viewport: None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::node_editor::state::NodeEditorCtx;
    use crate::pages::node_editor::types::PortSide;
    use crate::templates::convert::{load_into_ctx, resolve_port_name};

    /// Каждый builtin шаблон должен:
    /// 1) сериализоваться/десериализоваться через JSON без потерь;
    /// 2) все `ConnData` ссылки на порты должны резолвиться в registry;
    /// 3) после `load_into_ctx` число нод и connections должно совпадать
    ///    с исходным (никаких silent-skip из-за неизвестных портов/нод).
    #[test]
    fn all_builtin_templates_load_without_loss() {
        for t in all() {
            let label = t.id.clone();
            let json = serde_json::to_string(&t)
                .unwrap_or_else(|e| panic!("{label}: serialize failed: {e}"));
            let restored: Template = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("{label}: deserialize failed: {e}"));
            assert_eq!(restored.nodes.len(), t.nodes.len(), "{label}: node count");
            assert_eq!(
                restored.connections.len(),
                t.connections.len(),
                "{label}: connection count"
            );

            for c in &restored.connections {
                let from_kind = restored.kind_of(c.from_node);
                let to_kind = restored.kind_of(c.to_node);
                assert!(
                    resolve_port_name(from_kind, PortSide::Output, &c.from_port).is_some(),
                    "{label}: unknown output port {:?} on node {}",
                    c.from_port,
                    c.from_node
                );
                assert!(
                    resolve_port_name(to_kind, PortSide::Input, &c.to_port).is_some(),
                    "{label}: unknown input port {:?} on node {}",
                    c.to_port,
                    c.to_node
                );
            }

            let ctx = NodeEditorCtx::new();
            ctx.nodes.set(Vec::new());
            ctx.connections.set(Vec::new());
            load_into_ctx(&ctx, &restored);
            assert_eq!(
                ctx.nodes.get_untracked().len(),
                t.nodes.len(),
                "{label}: ctx node count after load_into_ctx"
            );
            assert_eq!(
                ctx.connections.get_untracked().len(),
                t.connections.len(),
                "{label}: ctx connection count after load_into_ctx"
            );
        }
    }


    /// Оба voice-clone TTS-шаблона зарегистрированы, попадают в раздел
    /// «Речь» и все их связи ссылаются на валидные порты registry.
    /// Signal-free (без `load_into_ctx`), поэтому проходит в обычном
    /// многопоточном тест-раннере.
    #[test]
    fn voice_clone_templates_wired_correctly() {
        use crate::templates::TemplateCategory;
        let all = all();
        let vc: Vec<&Template> = all
            .iter()
            .filter(|t| t.id.starts_with("builtin-voice-clone-"))
            .collect();
        assert_eq!(vc.len(), 2, "оба voice-clone шаблона зарегистрированы");
        for t in &vc {
            assert_eq!(
                TemplateCategory::for_template(t),
                TemplateCategory::Speech,
                "{}: должен попадать в раздел «Речь»",
                t.id
            );
            assert!(!t.connections.is_empty(), "{}: есть связи", t.id);
            for c in &t.connections {
                assert!(
                    resolve_port_name(t.kind_of(c.from_node), PortSide::Output, &c.from_port)
                        .is_some(),
                    "{}: неизвестный output-порт {:?} на ноде {}",
                    t.id,
                    c.from_port,
                    c.from_node
                );
                assert!(
                    resolve_port_name(t.kind_of(c.to_node), PortSide::Input, &c.to_port).is_some(),
                    "{}: неизвестный input-порт {:?} на ноде {}",
                    t.id,
                    c.to_port,
                    c.to_node
                );
            }
        }
    }

    /// Каждое семейство моделей — в своём разделе окна шаблонов. «Базовые» —
    /// только пустой граф и пример: новый шаблон без своего префикса в
    /// [`TemplateCategory::for_template`] туда больше не провалится молча.
    #[test]
    fn builtin_templates_in_own_categories() {
        use crate::templates::TemplateCategory as C;
        let expected = |id: &str| -> Option<C> {
            let rules: &[(&str, C)] = &[
                ("builtin-ltx-", C::Video),
                ("builtin-h3-", C::Video),
                ("builtin-flux-", C::Image),
                ("builtin-flux2-", C::Image),
                ("builtin-qwen-image", C::Image),
                ("builtin-sdxl-", C::Image),
                ("builtin-acestep-", C::Music),
                ("builtin-yue2-", C::Music),
                ("builtin-voice-clone-", C::Speech),
                ("builtin-vibevoice-", C::Speech),
                ("builtin-audio-", C::Audio),
                ("builtin-voice-recording", C::Audio),
                ("builtin-save-", C::Audio),
                ("builtin-mix-", C::Audio),
                ("builtin-empty", C::Basic),
                ("builtin-simple-add", C::Basic),
            ];
            rules.iter().find(|(p, _)| id.starts_with(p)).map(|(_, c)| *c)
        };
        let all = all();
        for t in &all {
            let want = expected(&t.id).unwrap_or_else(|| panic!("{}: семейство без раздела", t.id));
            assert_eq!(C::for_template(t), want, "{}", t.id);
        }
        for cat in C::ORDER {
            assert!(all.iter().any(|t| C::for_template(t) == cat), "раздел {cat:?} пуст");
        }
    }
}
