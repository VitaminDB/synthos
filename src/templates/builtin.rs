//! Встроенные шаблоны графа. Не лежат на диске — генерируются в коде,
//! чтобы пакет synthos был самодостаточен и при первом запуске у юзера
//! сразу было что попробовать.
//!
//! `builtin: true` запрещает удалять/переименовывать; «Duplicate to
//! Custom» через [`super::storage::duplicate_to_custom`] делает копию,
//! которую уже можно править.

use super::model::{
    ConnData,
    FieldValueData,
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
/// SDXL, MiniMax-H3, LTX-2.3, ACE-Step, YuE2) — JSON: переведены из прежних родных нод
/// [`super::native`].
const STAGE_TEMPLATES: &[(&str, &str)] = &[
    ("builtin-acestep-text2music", include_str!("builtin/builtin-acestep-text2music.json")),
    ("builtin-acestep-retake", include_str!("builtin/builtin-acestep-retake.json")),
    ("builtin-acestep-repaint", include_str!("builtin/builtin-acestep-repaint.json")),
    ("builtin-acestep-extend", include_str!("builtin/builtin-acestep-extend.json")),
    ("builtin-acestep-edit", include_str!("builtin/builtin-acestep-edit.json")),
    ("builtin-acestep-cover", include_str!("builtin/builtin-acestep-cover.json")),
    ("builtin-acestep-extract", include_str!("builtin/builtin-acestep-extract.json")),
    ("builtin-yue2-song", include_str!("builtin/builtin-yue2-song.json")),
    ("builtin-yue2-edit-score", include_str!("builtin/builtin-yue2-edit-score.json")),
    ("builtin-yue2-cover", include_str!("builtin/builtin-yue2-cover.json")),
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
    ("builtin-ltx-a2v", include_str!("builtin/builtin-ltx-a2v.json")),
    ("builtin-ltx-flux-keyframe", include_str!("builtin/builtin-ltx-flux-keyframe.json")),
    ("builtin-ltx-ic-lora", include_str!("builtin/builtin-ltx-ic-lora.json")),
    ("builtin-ltx-image-to-video", include_str!("builtin/builtin-ltx-image-to-video.json")),
    ("builtin-ltx-lipdub", include_str!("builtin/builtin-ltx-lipdub.json")),
    ("builtin-ltx-retake", include_str!("builtin/builtin-ltx-retake.json")),
    ("builtin-ltx-text-to-video", include_str!("builtin/builtin-ltx-text-to-video.json")),
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
        stage("builtin-acestep-text2music"),
        stage("builtin-acestep-retake"),
        stage("builtin-acestep-repaint"),
        stage("builtin-acestep-extend"),
        stage("builtin-acestep-edit"),
        stage("builtin-acestep-cover"),
        stage("builtin-acestep-extract"),
        stage("builtin-yue2-song"),
        stage("builtin-yue2-edit-score"),
        stage("builtin-yue2-cover"),
        stage("builtin-h3-text-to-video"),
        stage("builtin-h3-turbo"),
        stage("builtin-h3-first-frame"),
        stage("builtin-h3-first-last"),
        stage("builtin-h3-ref2va"),
        stage("builtin-ltx-text-to-video"),
        stage("builtin-ltx-image-to-video"),
        stage("builtin-ltx-retake"),
        stage("builtin-ltx-ic-lora"),
        stage("builtin-ltx-lipdub"),
        stage("builtin-ltx-a2v"),
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

