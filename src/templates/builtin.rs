//! Встроенные шаблоны графа. Не лежат на диске — генерируются в коде,
//! чтобы пакет synthos был самодостаточен и при первом запуске у юзера
//! сразу было что попробовать.
//!
//! `builtin: true` запрещает удалять/переименовывать; «Duplicate to
//! Custom» через [`super::storage::duplicate_to_custom`] делает копию,
//! которую уже можно править.

use super::model::{
    AceStepCheckpointStateData, AceStepGenerateStateData, AceStepVaeStateData, ConnData,
    Yue2CheckpointStateData, Yue2GenerateStateData,
    FieldValueData, Flux2CheckpointStateData, Flux2SamplerStateData, FluxEmptyLatentStateData, FluxSamplerStateData,
    QwenImageCheckpointStateData, SdxlCheckpointStateData, SdxlSamplerStateData,
    H3KeyframeStateData, H3SamplerStateData, LlmStateData, LtxSamplerStage1StateData, NodeData,
    NodeStateData,
    PointData, SynCheckpointStateData, Template, TemplateKind, TextViewStateData, VibeVoiceStateData,
};
use crate::pages::node_editor::types::NodeKind;

/// Все встроенные шаблоны. Порядок ⇒ порядок отображения в панели.
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
        h3_text_to_video_template(),
        h3_turbo_template(),
        h3_first_frame_template(),
        h3_first_last_template(),
        h3_ref2va_template(),
        ltx_text_to_video_template(),
        ltx_image_to_video_template(),
        ltx_retake_template(),
        ltx_ic_lora_template(),
        ltx_lipdub_template(),
        ltx_a2v_template(),
        flux_text_to_image_template(),
        flux_image_to_image_template(),
        flux2_text_to_image_template(),
        flux2_edit_template(),
        flux2_image_to_image_template(),
        flux2_llm_upsampling_template(),
        flux2_multi_reference_template(),
        qwen_image_edit_template(),
        qwen_image_multi_edit_template(),
        sdxl_text_to_image_template(),
        sdxl_image_to_image_template(),
        ltx_flux_keyframe_template(),
        h3_flux_keyframe_template(),
    ]
}

// ── FLUX.2 ────────────────────────────────────────────────────────────────

/// Цепочка FLUX.2 txt2img: Checkpoint(1) → Text Encoder(3) ← промпт(2),
/// Empty Latent(4) → Sampler(5) → VAE Decode(6) → Image Save(7). Empty
/// Latent — общий с FLUX.1 (это только размер).
fn flux2_base_nodes(prompt: &str, width: u32, height: u32, aspect_idx: usize) -> Vec<NodeData> {
    vec![
        node_with_state(
            1,
            NodeKind::Flux2Checkpoint,
            60.0,
            60.0,
            NodeStateData::Flux2Checkpoint(Flux2CheckpointStateData::default()),
        ),
        node_with_state(2, NodeKind::TextView, 60.0, 520.0, flux_prompt_state(prompt)),
        node_plain(3, NodeKind::Flux2TextEncoder, 520.0, 420.0),
        node_with_state(
            4,
            NodeKind::FluxEmptyLatent,
            520.0,
            640.0,
            NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData { width, height, aspect_idx }),
        ),
        node_plain(5, NodeKind::Flux2Sampler, 960.0, 420.0),
        node_plain(6, NodeKind::Flux2VaeDecode, 1400.0, 420.0),
        node_plain(7, NodeKind::ImageSave, 1840.0, 420.0),
    ]
}

fn flux2_text_to_image_template() -> Template {
    Template {
        id: "builtin-flux2-text-to-image".into(),
        builtin: true,
        name: "FLUX.2: Text to Image".into(),
        description: "Промпт → LLM-энкодер (Mistral-24B у dev, Qwen3 у klein) → Sampler (шаги по \
             модели: dev 50, klein 4) → VAE Decode → PNG. В Checkpoint — flux.2-dev.syn, \
             flux.2-klein-4b.syn или flux.2-klein-9b.syn; не влезшее в VRAM стримится."
            .into(),
        kind: TemplateKind::Full,
        nodes: flux2_base_nodes(
            "A cozy reading nook by a rainy window at dusk: a ginger cat asleep on a chunky knitted \
             blanket, a brass floor lamp casting warm light, raindrops on the glass, photograph",
            1024,
            1024,
            0,
        ),
        connections: vec![
            conn(1, "model", 3, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(2, "out", 3, "prompt"),
            conn(3, "conditioning", 5, "conditioning"),
            conn(4, "latent", 5, "latent"),
            conn(5, "latent", 6, "latent"),
            conn(6, "image", 7, "image"),
        ],
        viewport: None,
    }
}

/// Правка: Image(8) → Reference(9) → Sampler.references; размер — с
/// референса, Empty Latent не нужен.
fn flux2_edit_template() -> Template {
    let mut nodes: Vec<NodeData> =
        flux2_base_nodes("Make it a snowy winter evening; keep everything else unchanged", 1024, 1024, 0)
            .into_iter()
            .filter(|n| n.kind != NodeKind::FluxEmptyLatent)
            .collect();
    nodes.push(node_plain(8, NodeKind::ImageLoad, 60.0, 780.0));
    nodes.push(node_plain(9, NodeKind::Flux2Reference, 520.0, 720.0));
    Template {
        id: "builtin-flux2-edit".into(),
        builtin: true,
        name: "FLUX.2: Edit Image".into(),
        description: "Картинка → Reference → Sampler вместе с инструкцией правки → PNG в размере \
             исходника (до 1 Мп). Пишите, что изменить и что оставить как есть."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections: vec![
            conn(1, "model", 3, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(1, "model", 9, "model"),
            conn(2, "out", 3, "prompt"),
            conn(3, "conditioning", 5, "conditioning"),
            conn(8, "image", 9, "image"),
            conn(9, "references", 5, "references"),
            conn(5, "latent", 6, "latent"),
            conn(6, "image", 7, "image"),
        ],
        viewport: None,
    }
}

/// Img2img: Image(8) → VAE Encode(9) → Sampler.latent с denoise 0.7; размер —
/// картинки (до 2 Мп), Empty Latent не нужен.
fn flux2_image_to_image_template() -> Template {
    let mut nodes: Vec<NodeData> = flux2_base_nodes(
        "The same scene as a watercolor painting, soft washes of color, visible paper texture",
        1024,
        1024,
        0,
    )
    .into_iter()
    .filter(|n| n.kind != NodeKind::FluxEmptyLatent)
    .collect();
    nodes.push(node_plain(8, NodeKind::ImageLoad, 60.0, 780.0));
    nodes.push(node_plain(9, NodeKind::Flux2VaeEncode, 520.0, 720.0));
    for n in nodes.iter_mut().filter(|n| n.kind == NodeKind::Flux2Sampler) {
        n.state = Some(NodeStateData::Flux2Sampler(Flux2SamplerStateData { denoise: 0.7, ..Default::default() }));
    }
    Template {
        id: "builtin-flux2-image-to-image".into(),
        builtin: true,
        name: "FLUX.2: Image to Image".into(),
        description: "Картинка → VAE Encode → Sampler с denoise 0.7 → VAE Decode → PNG. Чем меньше \
             denoise, тем ближе к исходнику по композиции и цветам; для правки по инструкции — \
             шаблон Edit Image."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections: vec![
            conn(1, "model", 3, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(1, "model", 9, "model"),
            conn(2, "out", 3, "prompt"),
            conn(3, "conditioning", 5, "conditioning"),
            conn(8, "image", 9, "image"),
            conn(9, "latent", 5, "latent"),
            conn(5, "latent", 6, "latent"),
            conn(6, "image", 7, "image"),
        ],
        viewport: None,
    }
}

/// Системная инструкция «апсемплинга промпта» из официального пайплайна
/// FLUX.2 (Black Forest Labs, `flux2/src/flux2/system_messages.py`, её же
/// берёт diffusers `Flux2Pipeline.upsample_prompt`): там промпт переписывает
/// сама Mistral из энкодера dev, у нас энкодер урезан и текст не генерирует,
/// поэтому переписывает любая чат-LLM. Добавлено одно правило: результат —
/// по-английски (так лучше всего понимают энкодеры FLUX.2), надписи — на
/// языке оригинала.
const FLUX2_UPSAMPLING_SYSTEM: &str = "You are an expert prompt engineer for FLUX.2 by Black Forest Labs. \
Rewrite user prompts to be more descriptive while strictly preserving their core subject and intent.

Guidelines:
1. Structure: Keep structured inputs structured (enhance within fields). Convert natural language to detailed paragraphs.
2. Details: Add concrete visual specifics - form, scale, textures, materials, lighting (quality, direction, color), \
shadows, spatial relationships, and environmental context.
3. Text in Images: Put ALL text in quotation marks, matching the prompt's language. Always provide explicit quoted \
text for objects that would contain text in reality (signs, labels, screens, etc.) - without it, the model \
generates gibberish.
4. Language: Write the revised prompt in English, whatever the language of the request; only the quoted text that \
must appear in the image keeps its original language.

Output only the revised prompt and nothing else.";

/// Апсемплинг промпта LLM: короткий промпт(2) → LLM(9) (модель — Syn
/// Checkpoint(8), без «Держать в памяти»: VRAM нужна FLUX.2) → Text View(10)
/// с подробным промптом (правится руками) → Text Encoder(3) → Sampler(5) →
/// VAE Decode(6) → Image Save(7).
fn flux2_llm_upsampling_template() -> Template {
    let llm = NodeStateData::Llm(LlmStateData {
        model_path: None,
        device_idx: crate::pages::node_editor::nodes::llm::default_device_idx(),
        quant_idx: crate::pages::node_editor::nodes::llm::default_quant_idx(),
        compute_idx: crate::pages::node_editor::nodes::llm::default_compute_idx(),
        system_prompt: FLUX2_UPSAMPLING_SYSTEM.into(),
        context: 4096,
        think: false,
        max_tokens: 512,
        // Как у пайплайна BFL: 0.15 — переписать, а не сочинить заново.
        temperature: 0.15,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        seed: 0,
    });
    // NVFP4: «Auto» у LLM-ноды — плотный BF16, 27B-модель на 24 ГБ не
    // влезала и стримилась с хоста. Замер шаблона (qwen3.8-27b + klein-4B):
    // NVFP4 — 15 с на загрузку и переписывание, FP8 — 95 с (не влезает
    // целиком); у gemma-4-26b в NVFP4 текст промпта чистый (9 с).
    let checkpoint = NodeStateData::SynCheckpoint(SynCheckpointStateData {
        model_path: None,
        device_idx: 0,
        storage_idx: 4,
        compute_idx: 0,
        resident: false,
    });
    let upsampled = NodeStateData::TextView(TextViewStateData { output_text: String::new(), width: 360.0, height: 260.0 });
    Template {
        id: "builtin-flux2-llm-upsampling".into(),
        builtin: true,
        name: "FLUX.2: LLM Upsampling".into(),
        description: "Короткий промпт → LLM переписывает его подробно по инструкции Black Forest Labs \
             (детали, свет, материалы, надписи в кавычках) → Text View (можно поправить) → FLUX.2 → PNG. \
             В Syn Checkpoint — любая чат-LLM; «Держать в памяти» выключено, чтобы VRAM досталась FLUX.2."
            .into(),
        kind: TemplateKind::Full,
        nodes: vec![
            node_with_state(
                1,
                NodeKind::Flux2Checkpoint,
                60.0,
                60.0,
                NodeStateData::Flux2Checkpoint(Flux2CheckpointStateData::default()),
            ),
            node_with_state(2, NodeKind::TextView, 60.0, 420.0, flux_prompt_state("кот читает газету в уличном кафе в Париже")),
            node_with_state(8, NodeKind::SynCheckpoint, 60.0, 680.0, checkpoint),
            node_with_state(9, NodeKind::Llm, 480.0, 420.0, llm),
            node_with_state(10, NodeKind::TextView, 940.0, 420.0, upsampled),
            node_plain(3, NodeKind::Flux2TextEncoder, 1380.0, 300.0),
            node_with_state(
                4,
                NodeKind::FluxEmptyLatent,
                1380.0,
                560.0,
                NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData { width: 1024, height: 1024, aspect_idx: 0 }),
            ),
            node_plain(5, NodeKind::Flux2Sampler, 1820.0, 300.0),
            node_plain(6, NodeKind::Flux2VaeDecode, 2260.0, 300.0),
            node_plain(7, NodeKind::ImageSave, 2700.0, 300.0),
        ],
        connections: vec![
            conn(2, "out", 9, "prompt"),
            conn(8, "model", 9, "model"),
            conn(9, "answer", 10, "in"),
            conn(10, "out", 3, "prompt"),
            conn(1, "model", 3, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(3, "conditioning", 5, "conditioning"),
            conn(4, "latent", 5, "latent"),
            conn(5, "latent", 6, "latent"),
            conn(6, "image", 7, "image"),
        ],
        viewport: None,
    }
}

/// Два референса цепочкой: Image(8) → Reference(9) → Reference(11) ← Image(10)
/// → Sampler.references, размер — Empty Latent.
fn flux2_multi_reference_template() -> Template {
    let mut nodes = flux2_base_nodes(
        "The person from image 1 sitting at the café table from image 2, natural daylight, photograph",
        1024,
        1024,
        0,
    );
    nodes.push(node_plain(8, NodeKind::ImageLoad, 60.0, 900.0));
    nodes.push(node_plain(9, NodeKind::Flux2Reference, 520.0, 860.0));
    nodes.push(node_plain(10, NodeKind::ImageLoad, 60.0, 1160.0));
    nodes.push(node_plain(11, NodeKind::Flux2Reference, 520.0, 1100.0));
    Template {
        id: "builtin-flux2-multi-reference".into(),
        builtin: true,
        name: "FLUX.2: Multi-Reference".into(),
        description: "Две картинки → Reference → Reference (цепочкой) → Sampler: персонаж, предмет \
             или стиль с одной картинки в сцене другой. В промпте ссылайтесь на «image 1», \
             «image 2» по порядку цепочки."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections: vec![
            conn(1, "model", 3, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(1, "model", 9, "model"),
            conn(1, "model", 11, "model"),
            conn(2, "out", 3, "prompt"),
            conn(3, "conditioning", 5, "conditioning"),
            conn(4, "latent", 5, "latent"),
            conn(8, "image", 9, "image"),
            conn(9, "references", 11, "references"),
            conn(10, "image", 11, "image"),
            conn(11, "references", 5, "references"),
            conn(5, "latent", 6, "latent"),
            conn(6, "image", 7, "image"),
        ],
        viewport: None,
    }
}

// ── Qwen-Image ────────────────────────────────────────────────────────────

/// Правка Qwen-Image: Checkpoint(1), промпт(2) → Text Encoder(3) → Sampler(5)
/// → VAE Decode(6) → Image Save(7); Image(8) → Reference(9) — и в энкодер,
/// и в сэмплер. Размер — с картинки (~1 Мп).
fn qwen_image_edit_nodes(prompt: &str) -> Vec<NodeData> {
    vec![
        node_with_state(
            1,
            NodeKind::QwenImageCheckpoint,
            60.0,
            60.0,
            NodeStateData::QwenImageCheckpoint(QwenImageCheckpointStateData::default()),
        ),
        node_with_state(2, NodeKind::TextView, 60.0, 520.0, flux_prompt_state(prompt)),
        node_plain(3, NodeKind::QwenImageTextEncoder, 960.0, 420.0),
        node_plain(5, NodeKind::QwenImageSampler, 1400.0, 420.0),
        node_plain(6, NodeKind::QwenImageVaeDecode, 1840.0, 420.0),
        node_plain(7, NodeKind::ImageSave, 2280.0, 420.0),
        node_plain(8, NodeKind::ImageLoad, 60.0, 780.0),
        node_plain(9, NodeKind::QwenImageReference, 520.0, 720.0),
    ]
}

fn qwen_image_edit_connections(last_ref: u64) -> Vec<ConnData> {
    vec![
        conn(1, "model", 3, "model"),
        conn(1, "model", 5, "model"),
        conn(1, "model", 6, "model"),
        conn(1, "model", 9, "model"),
        conn(2, "out", 3, "prompt"),
        conn(8, "image", 9, "image"),
        conn(last_ref, "references", 3, "references"),
        conn(last_ref, "references", 5, "references"),
        conn(3, "conditioning", 5, "conditioning"),
        conn(5, "latent", 6, "latent"),
        conn(6, "image", 7, "image"),
    ]
}

fn qwen_image_edit_template() -> Template {
    Template {
        id: "builtin-qwen-image-edit".into(),
        builtin: true,
        name: "Qwen-Image: Edit Image".into(),
        description: "Картинка → Reference (в энкодер Qwen2.5-VL и в сэмплер) + инструкция правки → \
             Sampler (true CFG 4, шаги по модели) → PNG ~1 Мп в пропорциях исходника. В \
             Checkpoint — qwen-image-edit-2511.syn или qwen-image-edit.syn."
            .into(),
        kind: TemplateKind::Full,
        nodes: qwen_image_edit_nodes(
            "Replace the sky with a dramatic orange sunset; keep the buildings, people and lighting on them unchanged",
        ),
        connections: qwen_image_edit_connections(9),
        viewport: None,
    }
}

/// Две картинки цепочкой (2509/2511): Image(8) → Reference(9) → Reference(11)
/// ← Image(10); в промпте — «Picture 1», «Picture 2».
fn qwen_image_multi_edit_template() -> Template {
    let mut nodes = qwen_image_edit_nodes(
        "The woman from Picture 1 is sitting at the café table from Picture 2, holding a cup of coffee, natural daylight",
    );
    nodes.push(node_plain(10, NodeKind::ImageLoad, 60.0, 1040.0));
    nodes.push(node_plain(11, NodeKind::QwenImageReference, 520.0, 980.0));
    let mut connections = qwen_image_edit_connections(11);
    connections.extend([
        conn(1, "model", 11, "model"),
        conn(9, "references", 11, "references"),
        conn(10, "image", 11, "image"),
    ]);
    Template {
        id: "builtin-qwen-image-multi-edit".into(),
        builtin: true,
        name: "Qwen-Image: Multi-Image Edit".into(),
        description: "Две картинки → Reference → Reference (цепочкой) → энкодер и Sampler: человек, \
             предмет или стиль с одной картинки в сцене другой. Только Qwen-Image-Edit-2509/2511; \
             в промпте — «Picture 1», «Picture 2» по порядку цепочки."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

// ── SDXL ──────────────────────────────────────────────────────────────────

/// SDXL txt2img: Checkpoint(1), промпт(2) и негатив(8) → Text Encoder(3),
/// FLUX Empty Latent(4) → Sampler(5) → VAE Decode(6) → Image Save(7).
fn sdxl_base_nodes(prompt: &str) -> Vec<NodeData> {
    vec![
        node_with_state(
            1,
            NodeKind::SdxlCheckpoint,
            60.0,
            60.0,
            NodeStateData::SdxlCheckpoint(SdxlCheckpointStateData::default()),
        ),
        node_with_state(2, NodeKind::TextView, 60.0, 520.0, flux_prompt_state(prompt)),
        node_with_state(
            8,
            NodeKind::TextView,
            60.0,
            780.0,
            flux_prompt_state("blurry, low quality, deformed, watermark, text"),
        ),
        node_plain(3, NodeKind::SdxlTextEncoder, 520.0, 420.0),
        node_with_state(
            4,
            NodeKind::FluxEmptyLatent,
            520.0,
            640.0,
            NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData { width: 1024, height: 1024, aspect_idx: 0 }),
        ),
        node_plain(5, NodeKind::SdxlSampler, 960.0, 420.0),
        node_plain(6, NodeKind::SdxlVaeDecode, 1400.0, 420.0),
        node_plain(7, NodeKind::ImageSave, 1840.0, 420.0),
    ]
}

fn sdxl_base_connections() -> Vec<ConnData> {
    vec![
        conn(1, "model", 3, "model"),
        conn(1, "model", 5, "model"),
        conn(1, "model", 6, "model"),
        conn(2, "out", 3, "prompt"),
        conn(8, "out", 3, "negative"),
        conn(3, "conditioning", 5, "conditioning"),
        conn(4, "latent", 5, "latent"),
        conn(5, "latent", 6, "latent"),
        conn(6, "image", 7, "image"),
    ]
}

fn sdxl_text_to_image_template() -> Template {
    Template {
        id: "builtin-sdxl-text-to-image".into(),
        builtin: true,
        name: "SDXL: Text to Image".into(),
        description: "Промпт и негатив → CLIP-L + bigG → Sampler (30 шагов, CFG 5) → VAE Decode → \
             PNG 1024². В Checkpoint — sdxl-base-1.0.syn."
            .into(),
        kind: TemplateKind::Full,
        nodes: sdxl_base_nodes(
            "a lighthouse on a rocky cliff at golden hour, crashing waves, dramatic clouds, highly detailed photograph",
        ),
        connections: sdxl_base_connections(),
        viewport: None,
    }
}

fn sdxl_image_to_image_template() -> Template {
    // Размер латента задаёт картинка (VAE Encode), Empty Latent не нужен.
    let mut nodes: Vec<NodeData> = sdxl_base_nodes("the same scene as an oil painting, thick brush strokes, vivid colors")
        .into_iter()
        .filter(|n| n.kind != NodeKind::FluxEmptyLatent)
        .collect();
    nodes.push(node_plain(9, NodeKind::ImageLoad, 60.0, 1040.0));
    nodes.push(node_plain(10, NodeKind::SdxlVaeEncode, 520.0, 720.0));
    for n in nodes.iter_mut().filter(|n| n.kind == NodeKind::SdxlSampler) {
        n.state = Some(NodeStateData::SdxlSampler(SdxlSamplerStateData { denoise: 0.6, ..Default::default() }));
    }
    let mut connections: Vec<ConnData> = sdxl_base_connections().into_iter().filter(|c| c.from_node != 4).collect();
    connections.extend([conn(1, "model", 10, "model"), conn(9, "image", 10, "image"), conn(10, "latent", 5, "latent")]);
    Template {
        id: "builtin-sdxl-image-to-image".into(),
        builtin: true,
        name: "SDXL: Image to Image".into(),
        description: "Картинка → SDXL VAE Encode → Sampler с denoise 0.6 → VAE Decode → PNG (до 1 Мп, \
             стороны кратны 64). Чем меньше denoise, тем ближе к исходнику."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

// ── FLUX.1 ────────────────────────────────────────────────────────────────

fn flux_prompt_state(text: &str) -> NodeStateData {
    NodeStateData::TextView(TextViewStateData { output_text: text.into(), width: 300.0, height: 140.0 })
}

/// Цепочка FLUX txt2img: Checkpoint(1) → Text Encoder(3) ← промпт(2),
/// Empty Latent(4) → Sampler(5) → VAE Decode(6) → Image Save(7).
fn flux_base_nodes(prompt: &str, width: u32, height: u32, aspect_idx: usize) -> Vec<NodeData> {
    vec![
        node_plain(1, NodeKind::FluxCheckpoint, 60.0, 60.0),
        node_with_state(2, NodeKind::TextView, 60.0, 520.0, flux_prompt_state(prompt)),
        node_plain(3, NodeKind::FluxTextEncoder, 520.0, 420.0),
        node_with_state(
            4,
            NodeKind::FluxEmptyLatent,
            520.0,
            640.0,
            NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData { width, height, aspect_idx }),
        ),
        node_plain(5, NodeKind::FluxSampler, 960.0, 420.0),
        node_plain(6, NodeKind::FluxVaeDecode, 1400.0, 420.0),
        node_plain(7, NodeKind::ImageSave, 1840.0, 420.0),
    ]
}

fn flux_base_connections() -> Vec<ConnData> {
    let c = |from_node: u64, from_port: &str, to_node: u64, to_port: &str| ConnData {
        from_node,
        from_port: from_port.into(),
        to_node,
        to_port: to_port.into(),
    };
    vec![
        c(1, "model", 3, "model"),
        c(1, "model", 5, "model"),
        c(1, "model", 6, "model"),
        c(2, "out", 3, "prompt"),
        c(3, "conditioning", 5, "conditioning"),
        c(4, "latent", 5, "latent"),
        c(5, "latent", 6, "latent"),
        c(6, "image", 7, "image"),
    ]
}

fn flux_text_to_image_template() -> Template {
    Template {
        id: "builtin-flux-text-to-image".into(),
        builtin: true,
        name: "FLUX: Text to Image".into(),
        description: "Промпт → CLIP-L + T5-XXL → Sampler (28 шагов, guidance 3.5) → VAE Decode → \
             PNG. В Checkpoint укажите бандл flux.1-dev.syn (или каталог diffusers); размер \
             картинки — в Empty Latent."
            .into(),
        kind: TemplateKind::Full,
        nodes: flux_base_nodes(
            "a red fox sitting in a snowy birch forest at golden hour, soft backlight, \
             shallow depth of field, photograph",
            1024,
            1024,
            0,
        ),
        connections: flux_base_connections(),
        viewport: None,
    }
}

fn flux_image_to_image_template() -> Template {
    // Размер латента задаёт картинка (VAE Encode), Empty Latent не нужен.
    let mut nodes: Vec<NodeData> = flux_base_nodes(
        "the same scene as a watercolor painting, soft washes of color, paper texture",
        1024,
        1024,
        0,
    )
    .into_iter()
    .filter(|n| n.kind != NodeKind::FluxEmptyLatent)
    .collect();
    nodes.push(node_plain(8, NodeKind::ImageLoad, 60.0, 780.0));
    nodes.push(node_plain(9, NodeKind::FluxVaeEncode, 520.0, 720.0));
    for n in nodes.iter_mut().filter(|n| n.kind == NodeKind::FluxSampler) {
        n.state = Some(NodeStateData::FluxSampler(FluxSamplerStateData { denoise: 0.6, ..Default::default() }));
    }
    let mut connections: Vec<ConnData> =
        flux_base_connections().into_iter().filter(|c| c.from_node != 4).collect();
    connections.extend([
        ConnData { from_node: 1, from_port: "model".into(), to_node: 9, to_port: "model".into() },
        ConnData { from_node: 8, from_port: "image".into(), to_node: 9, to_port: "image".into() },
        ConnData { from_node: 9, from_port: "latent".into(), to_node: 5, to_port: "latent".into() },
    ]);
    Template {
        id: "builtin-flux-image-to-image".into(),
        builtin: true,
        name: "FLUX: Image to Image".into(),
        description: "Картинка → VAE Encode → Sampler с denoise 0.6 → VAE Decode → PNG. Чем \
             меньше denoise, тем ближе к исходнику; 1.0 — рисовать заново в размере картинки."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

/// Узлы и связи шаблона со сдвигом id и позиции — чтобы вставить готовый
/// граф рядом с цепочкой FLUX, не переписывая его.
fn shifted(t: Template, id_add: u64, dy: f32) -> (Vec<NodeData>, Vec<ConnData>) {
    let nodes = t
        .nodes
        .into_iter()
        .map(|mut n| {
            n.id += id_add;
            n.pos.y += dy;
            n
        })
        .collect();
    let conns = t
        .connections
        .into_iter()
        .map(|mut c| {
            c.from_node += id_add;
            c.to_node += id_add;
            c
        })
        .collect();
    (nodes, conns)
}

fn set_prompt(nodes: &mut [NodeData], id: u64, text: &str) {
    if let Some(n) = nodes.iter_mut().find(|n| n.id == id) {
        if let Some(NodeStateData::TextView(tv)) = n.state.as_mut() {
            tv.output_text = text.into();
        }
    }
}

fn ltx_flux_keyframe_template() -> Template {
    let mut nodes = flux_base_nodes(
        "a lone lighthouse on a rocky coast at dusk, waves crashing, dramatic clouds, \
         cinematic wide shot, photograph",
        1344,
        768,
        1,
    );
    let mut connections = flux_base_connections();
    let (ltx_nodes, ltx_conns) = shifted(ltx_image_to_video_template(), 100, 1100.0);
    nodes.extend(ltx_nodes);
    connections.extend(ltx_conns);
    set_prompt(
        &mut nodes,
        102,
        "waves roll in and crash against the rocks, the lighthouse beam sweeps across the \
         sky, clouds drift, slow push-in, sound of surf and wind",
    );
    // Кадр FLUX идёт в LTX Image проводом (файл в ноде не нужен).
    connections.push(ConnData {
        from_node: 6,
        from_port: "image".into(),
        to_node: 113,
        to_port: "image".into(),
    });
    Template {
        id: "builtin-ltx-flux-keyframe".into(),
        builtin: true,
        name: "LTX: FLUX Frame to Video".into(),
        description: "FLUX рисует первый кадр по промпту (16:9, 1344×768) → LTX Image → \
             LTX-2.3 image-to-video со звуком. Два промпта: верхний — что на кадре, нижний — \
             что происходит в видео. В FLUX Checkpoint укажите flux.1-dev.syn, в LTX \
             Checkpoint — чекпойнт, Gemma и upscaler."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

fn h3_flux_keyframe_template() -> Template {
    let mut nodes = flux_base_nodes(
        "portrait of a street musician playing violin under a streetlamp at night, light \
         rain, warm bokeh, cinematic photograph",
        1344,
        768,
        1,
    );
    let mut connections = flux_base_connections();
    let (h3_nodes, h3_conns) = shifted(h3_first_frame_template(), 100, 1100.0);
    nodes.extend(h3_nodes);
    connections.extend(h3_conns);
    set_prompt(
        &mut nodes,
        102,
        "the musician keeps playing, bow moving across the strings, raindrops glinting in \
         the lamplight, slow dolly-in; violin melody, soft rain on the pavement",
    );
    // Первый кадр H3 — картинка FLUX (Keyframe берёт её с провода).
    connections.push(ConnData {
        from_node: 6,
        from_port: "image".into(),
        to_node: 110,
        to_port: "image".into(),
    });
    Template {
        id: "builtin-h3-flux-keyframe".into(),
        builtin: true,
        name: "H3: FLUX Frame to Video".into(),
        description: "FLUX рисует первый кадр (16:9, 1344×768) → H3 Keyframe → MiniMax-H3 \
             видео со звуком от этого кадра. Верхний промпт — кадр, нижний — действие и звук \
             в формате H3. В FLUX Checkpoint укажите flux.1-dev.syn, в H3 Checkpoint — бандл \
             MiniMax-H3."
            .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

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

fn h3_ref2va_template() -> Template {
    let mut nodes = h3_base_nodes(
        "subject_definitions:\n<Subject 1> is the person in <Picture 1>.\n\nsummary:\n\
         <Subject 1> walks through a sunlit park toward the camera.\n\nretention_analysis:\n\
         <Subject 1> (appears in [Shot 1]): fully_preserved - identity, hair and clothing are \
         kept.\n\ndetailed_description:\n[Shot 1] <Subject 1> walks along a park path toward \
         the camera, smiling, leaves moving in a light breeze.\n\noverall_soundscape:\n\
         Footsteps on gravel, birdsong, distant voices.\n\nnon_diegetic_music:\nNone.",
    );
    nodes.push(node_plain(10, NodeKind::H3References, 60.0, 900.0));
    let mut connections = h3_base_connections();
    connections.push(ConnData {
        from_node: 4,
        from_port: "av_latent".into(),
        to_node: 10,
        to_port: "av_latent".into(),
    });
    for to in [3u64, 5u64] {
        connections.push(ConnData {
            from_node: 10,
            from_port: "refs".into(),
            to_node: to,
            to_port: "refs".into(),
        });
    }
    Template {
        id: "builtin-h3-ref2va".into(),
        builtin: true,
        name: "H3: References to Video".into(),
        description:
            "Видео по референсам (чекпойнт Ref2VA): до 9 картинок, 3 видео и 3 аудио, всего до \
             12, в ноде H3 References. Порядок списка задаёт метки <Picture i> / <Video k> / \
             <Audio j> — нода показывает их у каждой строки, на них и ссылается промпт. \
             Промпт — шесть секций (subject_definitions, summary, retention_analysis, \
             detailed_description, overall_soundscape, non_diegetic_music). В Checkpoint нужен \
             бандл Ref2VA: FL2VA референсов не понимает."
                .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
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


fn h3_prompt_state(text: &str) -> NodeStateData {
    NodeStateData::TextView(TextViewStateData {
        output_text: text.into(),
        width: 300.0,
        height: 140.0,
    })
}

fn h3_base_nodes(prompt: &str) -> Vec<NodeData> {
    vec![
        node_plain(1, NodeKind::H3Checkpoint, 60.0, 60.0),
        node_with_state(2, NodeKind::TextView, 60.0, 620.0, h3_prompt_state(prompt)),
        node_plain(3, NodeKind::H3TextEncoder, 520.0, 560.0),
        node_plain(4, NodeKind::H3EmptyLatentAv, 520.0, 780.0),
        node_plain(5, NodeKind::H3Sampler, 940.0, 560.0),
        node_plain(6, NodeKind::H3VaeDecode, 1360.0, 420.0),
        node_plain(7, NodeKind::H3AudioDecode, 1360.0, 800.0),
        node_plain(8, NodeKind::H3VideoSave, 1800.0, 480.0),
        node_plain(9, NodeKind::FfmpegPlayer, 1800.0, 760.0),
    ]
}

fn h3_base_connections() -> Vec<ConnData> {
    vec![
        ConnData { from_node: 1, from_port: "model".into(), to_node: 3, to_port: "model".into() },
        ConnData { from_node: 1, from_port: "model".into(), to_node: 5, to_port: "model".into() },
        ConnData { from_node: 1, from_port: "model".into(), to_node: 6, to_port: "model".into() },
        ConnData { from_node: 1, from_port: "model".into(), to_node: 7, to_port: "model".into() },
        ConnData { from_node: 2, from_port: "out".into(), to_node: 3, to_port: "prompt".into() },
        ConnData { from_node: 3, from_port: "conditioning".into(), to_node: 5, to_port: "conditioning".into() },
        ConnData { from_node: 4, from_port: "av_latent".into(), to_node: 5, to_port: "av_latent".into() },
        ConnData { from_node: 5, from_port: "video_latent".into(), to_node: 6, to_port: "video_latent".into() },
        ConnData { from_node: 5, from_port: "audio_latent".into(), to_node: 7, to_port: "audio_latent".into() },
        ConnData { from_node: 6, from_port: "frames".into(), to_node: 8, to_port: "frames".into() },
        ConnData { from_node: 7, from_port: "audio".into(), to_node: 8, to_port: "audio".into() },
        ConnData { from_node: 6, from_port: "frames".into(), to_node: 9, to_port: "frames".into() },
        ConnData { from_node: 7, from_port: "audio".into(), to_node: 9, to_port: "audio".into() },
    ]
}

fn h3_text_to_video_template() -> Template {
    Template {
        id: "builtin-h3-text-to-video".into(),
        builtin: true,
        name: "H3: Text to Video + Audio".into(),
        description:
            "Промпт → Qwen3-VL Text Encoder → Sampler (совместный денойзинг видео и звука) → \
             VAE Decode + Audio Decode → mp4 и плеер. В Checkpoint укажите .syn-бандл MiniMax-H3; \
             энкодер подхватится из бандла. Оригинальный пайплайн: 20 шагов, CFG 5 — \
             дефолты Sampler."
                .into(),
        kind: TemplateKind::Full,
        nodes: h3_base_nodes(
            "a woman in a cozy cafe, talking warmly to the camera, soft daylight, \
             gentle background chatter and the clink of cups",
        ),
        connections: h3_base_connections(),
        viewport: None,
    }
}

fn h3_turbo_template() -> Template {
    let mut nodes = h3_base_nodes(
        "close-up of rain hitting a neon-lit window at night, droplets sliding down, \
         distant thunder and soft synth pads",
    );
    // Sampler турбо-шаблона несёт свои параметры явно: 6 шагов без CFG.
    nodes[4] = node_with_state(
        5,
        NodeKind::H3Sampler,
        940.0,
        560.0,
        NodeStateData::H3Sampler(H3SamplerStateData { steps: 6, cfg_scale: 1.0, seed: 0 }),
    );
    Template {
        id: "builtin-h3-turbo".into(),
        builtin: true,
        name: "H3: Turbo (6 шагов)".into(),
        description:
            "Тот же граф, что и Text to Video, но под Turbo LoRA: 6 шагов без CFG (задано в \
             Sampler этого шаблона). Укажите minimax_h3_turbo_v4_step600_ema.safetensors \
             в поле LoRA у Checkpoint — без неё на 6 шагах будет рябь."
                .into(),
        kind: TemplateKind::Full,
        nodes,
        connections: h3_base_connections(),
        viewport: None,
    }
}

fn h3_first_frame_template() -> Template {
    let mut nodes = h3_base_nodes(
        "the person in the photo turns to the camera and smiles, hair moving in a light breeze, \
         ambient street sound",
    );
    nodes.push(node_plain(10, NodeKind::H3Keyframe, 60.0, 900.0));
    let mut connections = h3_base_connections();
    connections.push(ConnData {
        from_node: 10,
        from_port: "keyframe".into(),
        to_node: 3,
        to_port: "keyframe".into(),
    });
    connections.push(ConnData {
        from_node: 10,
        from_port: "keyframe".into(),
        to_node: 5,
        to_port: "keyframe".into(),
    });
    Template {
        id: "builtin-h3-first-frame".into(),
        builtin: true,
        name: "H3: First Frame to Video".into(),
        description:
            "Оживление изображения: Keyframe (слот «первый кадр») уходит и в Text Encoder \
             (как <Picture 1> в презентации промпта), и в Sampler (VAE-энкод в cond-строки). \
             Разрешение AV-латента задайте под пропорции исходника."
                .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

fn h3_first_last_template() -> Template {
    let mut nodes = h3_base_nodes(
        "a smooth cinematic transition between the two frames, steady camera, \
         evolving ambient soundscape",
    );
    nodes.push(node_plain(10, NodeKind::H3Keyframe, 60.0, 900.0));
    nodes.push(node_with_state(
        11,
        NodeKind::H3Keyframe,
        60.0,
        1120.0,
        NodeStateData::H3Keyframe(H3KeyframeStateData {
            image_path: None,
            frame_slot_idx: 1,
            resize_idx: 1,
        }),
    ));
    let mut connections = h3_base_connections();
    for (from, port) in [(10u64, "keyframe"), (11u64, "keyframe_last")] {
        connections.push(ConnData {
            from_node: from,
            from_port: "keyframe".into(),
            to_node: 3,
            to_port: port.into(),
        });
        connections.push(ConnData {
            from_node: from,
            from_port: "keyframe".into(),
            to_node: 5,
            to_port: port.into(),
        });
    }
    Template {
        id: "builtin-h3-first-last".into(),
        builtin: true,
        name: "H3: First+Last Frame".into(),
        description:
            "Переход между двумя кадрами: у второй Keyframe-ноды уже выбран слот «последний \
             кадр». Модель поддерживает якоря только на первом и последнем кадрах, середина \
             отвергается на этапе сборки layout."
                .into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
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
        device_idx: 1,
        storage_idx: 0,
        compute_idx: 0,
        chunk_seconds: 30.0,
        overlap_seconds: 0.5,
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

/// Кавер: мелодия без аккордовых символов, аккомпанемент модель придумывает
/// под новый стиль (рекомендация релиза — `cot = melody`).
fn yue2_cover_template() -> Template {
    yue2_song_like(
        "builtin-yue2-cover",
        "YuE2: кавер по мелодии",
        "Мелодия без аккордов из ноды abc + новый стиль: аккомпанемент подстроится.",
        "english, acoustic folk, male vocal, fingerpicked guitar",
        "[verse]\nWe were younger then, the road was long",
        yue2_gen(1, 60.0),
        true,
    )
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

    /// Шаблоны FLUX: txt2img/img2img в разделе «Картинки», сцепки с видео —
    /// в «Видео»; кадр FLUX идёт в LTX Image / H3 Keyframe проводом, id нод
    /// после сдвига не пересекаются.
    #[test]
    fn flux_templates_wired_correctly() {
        use crate::templates::TemplateCategory;
        let all = all();
        let get = |id: &str| all.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("нет {id}"));
        for id in ["builtin-flux-text-to-image", "builtin-flux-image-to-image"] {
            assert_eq!(TemplateCategory::for_template(get(id)), TemplateCategory::Image, "{id}");
        }
        for (id, target) in [("builtin-ltx-flux-keyframe", NodeKind::LtxImage), ("builtin-h3-flux-keyframe", NodeKind::H3Keyframe)] {
            let t = get(id);
            assert_eq!(TemplateCategory::for_template(t), TemplateCategory::Video, "{id}");
            let mut ids: Vec<u64> = t.nodes.iter().map(|n| n.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), t.nodes.len(), "{id}: id нод пересекаются");
            assert!(
                t.connections.iter().any(|c| t.kind_of(c.from_node) == Some(NodeKind::FluxVaeDecode)
                    && c.from_port == "image"
                    && t.kind_of(c.to_node) == Some(target)
                    && c.to_port == "image"),
                "{id}: нет провода FLUX VAE Decode → {target:?}"
            );
        }
        // Апсемплинг: промпт идёт через LLM и правимый Text View в энкодер,
        // LLM — с инструкцией BFL и без размышлений, модель не резидентна.
        let up = get("builtin-flux2-llm-upsampling");
        assert_eq!(TemplateCategory::for_template(up), TemplateCategory::Image);
        let wired = |from: NodeKind, fp: &str, to: NodeKind, tp: &str| {
            up.connections.iter().any(|c| {
                up.kind_of(c.from_node) == Some(from) && c.from_port == fp && up.kind_of(c.to_node) == Some(to) && c.to_port == tp
            })
        };
        assert!(wired(NodeKind::TextView, "out", NodeKind::Llm, "prompt"));
        assert!(wired(NodeKind::SynCheckpoint, "model", NodeKind::Llm, "model"));
        assert!(wired(NodeKind::Llm, "answer", NodeKind::TextView, "in"));
        assert!(wired(NodeKind::TextView, "out", NodeKind::Flux2TextEncoder, "prompt"));
        let llm = up.nodes.iter().find(|n| n.kind == NodeKind::Llm).unwrap();
        match &llm.state {
            Some(NodeStateData::Llm(s)) => {
                assert!(s.system_prompt.contains("FLUX.2") && s.system_prompt.contains("Output only the revised prompt"));
                assert!(!s.think);
            }
            other => panic!("апсемплинг: у LLM нет state: {other:?}"),
        }
        let ck = up.nodes.iter().find(|n| n.kind == NodeKind::SynCheckpoint).unwrap();
        assert!(matches!(&ck.state, Some(NodeStateData::SynCheckpoint(s)) if !s.resident));

        let i2i = get("builtin-flux-image-to-image");
        let sampler = i2i.nodes.iter().find(|n| n.kind == NodeKind::FluxSampler).unwrap();
        match &sampler.state {
            Some(NodeStateData::FluxSampler(s)) => assert!((s.denoise - 0.6).abs() < 1e-6),
            other => panic!("img2img: у Sampler нет denoise: {other:?}"),
        }
    }

    /// Оба voice-clone TTS-шаблона зарегистрированы, попадают в раздел
    /// «Аудио» и все их связи ссылаются на валидные порты registry.
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
                TemplateCategory::Audio,
                "{}: должен попадать в раздел «Аудио»",
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
}
