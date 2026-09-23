//! Конвертация рантайм-структур графа (`NodeInstance` / `Connection` /
//! `FieldValue`) ↔ сериализуемых JSON-структур (`NodeData` / `ConnData` /
//! `FieldValueData`).
//!
//! Имена портов и полей в графе — `&'static str` из `registry.rs`.
//! При загрузке шаблона строковое имя резолвится через
//! [`resolve_port_name`] / [`resolve_field_name`] — неизвестные имена
//! пропускаются молча, чтобы шаблон, сохранённый в одной версии,
//! загружался в более новой даже если какие-то поля переименовались.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::{Color, Point, Size};
use syngui::prelude::*;

use super::model::{
    AceStepCheckpointStateData, AceStepGenerateStateData,
    Yue2CheckpointStateData, Yue2GenerateStateData, Yue2TranscribeStateData, Yue2VaeDecodeStateData,
    AceStepVaeStateData, AsrGigaamStateData,
    AudioFileStateData, AudioPlayerStateData, AudioRecorderStateData, ConnData, EqualizerStateData,
    FfmpegPlayerStateData, FieldValueData, FilterStateData, Flux2CheckpointStateData,
    Flux2SamplerStateData, FluxCheckpointStateData, QwenImageCheckpointStateData, QwenImageSamplerStateData,
    QwenImage21CheckpointStateData, QwenImage21SamplerStateData,
    SdxlCheckpointStateData, SdxlSamplerStateData,
    FluxEmptyLatentStateData, FluxSamplerStateData, FluxTextEncoderStateData, FluxVaeEncodeStateData,
    GainStateData, H3CheckpointStateData, ImageLoadStateData, ImageSaveStateData,
    H3EmptyLatentAvStateData, H3KeyframeStateData, H3ReferenceItemData, H3ReferencesStateData,
    H3SamplerStateData, LlmStateData,
    LtxA2VStateData, LtxAudioInputStateData, LtxCheckpointStateData, LtxIcLoraStateData,
    LtxImageStateData, LtxLipdubStateData, LtxNagPromptStateData, LtxRetakeStateData,
    LtxSamplerStage1StateData, LtxSamplerStage2StateData, LtxTextEncoderStateData,
    LtxVideoInputStateData, LtxVideoSaveStateData, SynCheckpointStateData,
    MarkdownViewStateData, MixerStateData, NodeData, NodeStateData, NodeStyleData,
    OmniVoiceStateData, PointData, ReverbStateData, SaveToFileStateData, Template,
    TextViewStateData, VibeVoiceStateData, ViewportData, VoxCpm2StateData,
};
use crate::pages::node_editor::registry::{self, default_fields, meta};
use crate::pages::node_editor::state::NodeEditorCtx;
use crate::pages::node_editor::types::{
    Connection, FieldValue, NodeId, NodeInstance, NodeKind, NodeRuntime, PortSide, MIXER_MAX_INPUTS,
};

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot: NodeEditorCtx → Template (без id/name/description/kind/builtin)
// ─────────────────────────────────────────────────────────────────────────────

/// Снять снимок текущего графа в сериализуемые `nodes`/`connections`/`viewport`.
/// Возвращает заготовку шаблона с пустыми `id/name/description/kind/builtin`
/// — caller заполняет эти поля сам (через UI-диалог или builtin-фабрику).
pub fn snapshot(ctx: &NodeEditorCtx) -> (Vec<NodeData>, Vec<ConnData>, Option<ViewportData>) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();

    let nodes_data: Vec<NodeData> = nodes.iter().map(node_to_data).collect();
    let conns_data: Vec<ConnData> = conns.iter().map(conn_to_data).collect();
    let viewport = Some(ViewportData {
        pan: ctx.pan.get_untracked().into(),
        zoom: ctx.zoom.get_untracked(),
    });
    (nodes_data, conns_data, viewport)
}

fn node_to_data(node: &NodeInstance) -> NodeData {
    let mut fields: std::collections::BTreeMap<String, FieldValueData> = Default::default();
    let map = node.fields.lock().unwrap();
    for (key, value) in map.iter() {
        fields.insert((*key).to_string(), field_value_to_data(value));
    }
    drop(map);
    let style = node.style.get_untracked();
    let state = {
        let rt = node.runtime.lock().unwrap();
        runtime_to_state(&rt)
    };
    NodeData {
        id: node.id.0,
        kind: node.kind,
        pos: node.pos.get_untracked().into(),
        fields,
        style: NodeStyleData {
            tint: style.tint.map(NodeStyleData::color_to_hex),
            shadow: style.shadow,
        },
        enabled: node.enabled.get_untracked(),
        state,
    }
}

fn conn_to_data(c: &Connection) -> ConnData {
    ConnData {
        from_node: c.from_node.0,
        from_port: c.from_port.to_string(),
        to_node: c.to_node.0,
        to_port: c.to_port.to_string(),
    }
}

fn field_value_to_data(v: &FieldValue) -> FieldValueData {
    match v {
        FieldValue::Text(s) => FieldValueData::Text(s.get_untracked()),
        FieldValue::Float(s) => FieldValueData::Float(s.get_untracked()),
        FieldValue::Int(s) => FieldValueData::Int(s.get_untracked()),
        FieldValue::Bool(s) => FieldValueData::Bool(s.get_untracked()),
        FieldValue::Color(s) => FieldValueData::from_color(s.get_untracked()),
        FieldValue::Choice(s) => FieldValueData::Choice(s.get_untracked()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Apply: Template → NodeEditorCtx
// ─────────────────────────────────────────────────────────────────────────────

/// Заменяет содержимое `ctx` на содержимое шаблона. Используется при
/// открытии шаблона как новой вкладки (свежий пустой ctx).
pub fn load_into_ctx(ctx: &NodeEditorCtx, t: &Template) {
    ctx.nodes.set(Vec::new());
    ctx.connections.set(Vec::new());
    apply_to_ctx(ctx, t, Point::new(0.0, 0.0));
    if let Some(v) = t.viewport {
        ctx.pan.set(v.pan.into());
        ctx.zoom.set(v.zoom);
    }
}

/// Добавляет ноды/связи шаблона в существующий `ctx` со смещением
/// `offset` (для merge / drag-and-drop). Назначает свежие id, чтобы не
/// конфликтовать с существующими.
pub fn apply_to_ctx(ctx: &NodeEditorCtx, t: &Template, offset: Point) {
    let migrated = migrate_inline_models(t);
    let t = migrated.as_ref().unwrap_or(t);
    // local-id (внутри шаблона) → глобальный id (в ctx).
    let mut id_map: HashMap<u64, NodeId> = HashMap::new();

    let mut nodes_vec = ctx.nodes.get_untracked();
    let mut next_id = ctx.next_id.get_untracked();
    for nd in &t.nodes {
        let new_id = NodeId(next_id);
        next_id += 1;
        let pos = Point::new(nd.pos.x + offset.x, nd.pos.y + offset.y);
        let fields = build_fields(nd.kind, &nd.fields);
        // tint из hex-строки; неразобранный hex → no tint (graceful).
        let tint = nd.style.tint.as_deref()
            .map(syngui::core::Color::from_hex);
        let style = crate::pages::node_editor::types::NodeStyle {
            tint,
            shadow: nd.style.shadow,
        };
        let inst = NodeInstance {
            id: new_id,
            kind: nd.kind,
            pos: use_signal(pos),
            bounds: use_signal(syngui::core::Rect::zero()),
            fields,
            runtime: crate::pages::node_editor::registry::default_runtime(nd.kind),
            style: use_signal(style),
            enabled: use_signal(nd.enabled),
            timing: crate::pages::node_editor::timing::Stopwatch::new(),
        };
        // Per-kind state из шаблона → бампаем сигналы свежесозданного runtime'а.
        // Тяжёлые объекты (Transcriber/Pipeline) остаются дефолтными — модели
        // ленивятся на первый Play (см. `apply_state_to_runtime`).
        if let Some(state) = &nd.state {
            if let Ok(rt) = inst.runtime.lock() {
                apply_state_to_runtime(&rt, state);
            }
        }
        nodes_vec.push(inst);
        id_map.insert(nd.id, new_id);
    }
    ctx.nodes.set(nodes_vec);
    ctx.next_id.set(next_id);

    // Соединения — берём только те, у которых оба порта найдены в registry
    // и обе ноды — в id_map. Остальные пропускаем (не паника).
    let mut conns_vec = ctx.connections.get_untracked();
    for cd in &t.connections {
        let (Some(&from), Some(&to)) = (id_map.get(&cd.from_node), id_map.get(&cd.to_node)) else {
            continue;
        };
        let Some(from_port) = resolve_port_name(t.kind_of(cd.from_node), PortSide::Output, &cd.from_port) else {
            continue;
        };
        let Some(to_port) = resolve_port_name(t.kind_of(cd.to_node), PortSide::Input, &cd.to_port) else {
            continue;
        };
        let conn = Connection {
            from_node: from,
            from_port,
            to_node: to,
            to_port,
        };
        if !conns_vec.contains(&conn) {
            conns_vec.push(conn);
        }
    }
    ctx.connections.set(conns_vec);
}

/// Старые графы: слот-ноды (LLM/TTS/ASR/диаризация) держали путь модели и
/// device/точность у себя. Теперь модель приходит только из Syn Checkpoint —
/// для каждой такой ноды с путём и без входа `model` рядом ставится
/// чекпойнт с теми же настройками (резидентным: слот держал модель всегда).
/// Так же семейные ноды: ACE-Step VAE Encode получает вход `model` от
/// ACE-Step Checkpoint, свой декодер YuE2 VAE Decode уезжает в копию
/// YuE2 Checkpoint, depth-модель IC-LoRA — в подключённый LTX Checkpoint.
/// `None` — мигрировать нечего.
pub fn migrate_inline_models(t: &Template) -> Option<Template> {
    // Индексы предпочтений чекпойнта: device 0=Auto,1=CUDA,2=CPU;
    // storage 0=Auto,1=F16,2=BF16,3=FP8,4=NVFP4; compute 0=Auto,1=F16,2=BF16,3=F32.
    fn cuda_cpu(i: usize) -> usize {
        if i == 1 { 2 } else { 1 }
    }
    fn cpu_gpu(i: usize) -> usize {
        if i == 0 { 2 } else { 1 }
    }
    fn bf16_f16_f32(i: usize) -> usize {
        match i {
            0 => 2,
            1 => 1,
            2 => 3,
            _ => 0,
        }
    }
    // Семейства ASR/Omni/Sortformer: f16, bf16, f32, nvfp4, mxfp8.
    fn f16_list_storage(i: usize) -> usize {
        match i {
            0 => 1,
            1 => 2,
            3 => 4,
            4 => 3,
            _ => 0,
        }
    }
    fn f16_list_compute(i: usize) -> usize {
        match i {
            0 => 1,
            1 => 2,
            2 => 3,
            _ => 0,
        }
    }
    let ck = |path: &Option<String>, device_idx, storage_idx, compute_idx| {
        path.clone().map(|p| SynCheckpointStateData {
            model_path: Some(p),
            device_idx,
            storage_idx,
            compute_idx,
            resident: true,
        })
    };
    let mut out: Option<Template> = None;
    let mut next_id = t.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    for nd in &t.nodes {
        if !nd.kind.needs_syn_checkpoint()
            || t.connections.iter().any(|c| c.to_node == nd.id && c.to_port == "model")
        {
            continue;
        }
        let data = match &nd.state {
            Some(NodeStateData::Llm(d)) => ck(
                &d.model_path,
                cuda_cpu(d.device_idx),
                match d.quant_idx {
                    1 => 4,
                    2 => 3,
                    _ => 0,
                },
                bf16_f16_f32(d.compute_idx),
            ),
            Some(NodeStateData::VibeVoice(d)) => {
                ck(&d.model_path, cuda_cpu(d.device_idx), 0, bf16_f16_f32(d.compute_idx))
            }
            Some(NodeStateData::VoxCpm2(d)) => {
                ck(&d.model_path, cuda_cpu(d.device_idx), 0, bf16_f16_f32(d.compute_idx))
            }
            Some(NodeStateData::OmniVoice(d)) => ck(
                &d.model_path,
                cpu_gpu(d.device_idx),
                f16_list_storage(d.storage_idx),
                f16_list_compute(d.compute_idx),
            ),
            Some(NodeStateData::AsrGigaam(d)) => ck(
                &d.model_path,
                cpu_gpu(d.device_idx),
                f16_list_storage(d.storage_idx),
                f16_list_compute(d.compute_idx),
            ),
            Some(NodeStateData::SortformerDiarizer(d)) => ck(
                &d.model_path,
                cpu_gpu(d.device_idx),
                f16_list_storage(d.storage_idx),
                f16_list_compute(d.compute_idx),
            ),
            _ => None,
        };
        let Some(data) = data else { continue };
        let m = out.get_or_insert_with(|| t.clone());
        m.nodes.push(NodeData {
            id: next_id,
            kind: NodeKind::SynCheckpoint,
            pos: PointData {
                x: nd.pos.x - 320.0,
                y: nd.pos.y,
            },
            fields: Default::default(),
            style: Default::default(),
            enabled: nd.enabled,
            state: Some(NodeStateData::SynCheckpoint(data)),
        });
        m.connections.push(ConnData {
            from_node: next_id,
            from_port: "model".into(),
            to_node: nd.id,
            to_port: "model".into(),
        });
        next_id += 1;
    }

    // Семейные ноды: VAE/устройство/depth-модель переехали в свой чекпойнт.
    let node_at = |x: f32, y: f32, id: u64, kind: NodeKind, state: NodeStateData| NodeData {
        id,
        kind,
        pos: PointData { x, y },
        fields: Default::default(),
        style: Default::default(),
        enabled: true,
        state: Some(state),
    };
    for nd in &t.nodes {
        let model_src = t
            .connections
            .iter()
            .find(|c| c.to_node == nd.id && c.to_port == "model")
            .map(|c| c.from_node);
        match &nd.state {
            // ACE-Step VAE Encode брал VAE из глобальных настроек, device и
            // compute — свои. Теперь всё из ACE-Step Checkpoint графа.
            Some(NodeStateData::AceStepVaeEncode(d)) if model_src.is_none() => {
                let m = out.get_or_insert_with(|| t.clone());
                let from = match m.nodes.iter().find(|n| n.kind == NodeKind::AceStepCheckpoint) {
                    Some(ck) => ck.id,
                    None => {
                        let ck = node_at(
                            nd.pos.x - 320.0,
                            nd.pos.y,
                            next_id,
                            NodeKind::AceStepCheckpoint,
                            NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
                                device_idx: d.device_idx,
                                compute_idx: d.compute_idx,
                                ..Default::default()
                            }),
                        );
                        m.nodes.push(ck);
                        next_id += 1;
                        next_id - 1
                    }
                };
                m.connections.push(ConnData {
                    from_node: from,
                    from_port: "model".into(),
                    to_node: nd.id,
                    to_port: "model".into(),
                });
            }
            // YuE2 VAE Decode с собственным декодером → копия его чекпойнта
            // с override'ом VAE.
            Some(NodeStateData::Yue2VaeDecode(d)) if d.vae_path.is_some() => {
                let m = out.get_or_insert_with(|| t.clone());
                let base = model_src.and_then(|id| {
                    m.nodes
                        .iter()
                        .find(|n| n.id == id && n.kind == NodeKind::Yue2Checkpoint)
                        .and_then(|n| match &n.state {
                            Some(NodeStateData::Yue2Checkpoint(s)) => Some(s.clone()),
                            _ => None,
                        })
                });
                let state = Yue2CheckpointStateData {
                    vae_path: d.vae_path.clone(),
                    ..base.unwrap_or_default()
                };
                let ck = node_at(
                    nd.pos.x - 320.0,
                    nd.pos.y,
                    next_id,
                    NodeKind::Yue2Checkpoint,
                    NodeStateData::Yue2Checkpoint(state),
                );
                m.nodes.push(ck);
                m.connections.retain(|c| !(c.to_node == nd.id && c.to_port == "model"));
                m.connections.push(ConnData {
                    from_node: next_id,
                    from_port: "model".into(),
                    to_node: nd.id,
                    to_port: "model".into(),
                });
                next_id += 1;
            }
            // IC-LoRA: каталог depth-модели — в подключённый LTX Checkpoint.
            Some(NodeStateData::LtxIcLora(d)) if d.depth_model_path.is_some() => {
                let Some(src) = model_src else { continue };
                let m = out.get_or_insert_with(|| t.clone());
                let Some(ck) = m.nodes.iter_mut().find(|n| n.id == src && n.kind == NodeKind::LtxCheckpoint)
                else {
                    continue;
                };
                // Чекпойнт без state (рукописный шаблон) — serde-дефолты полей.
                let state = ck.state.get_or_insert_with(|| {
                    NodeStateData::LtxCheckpoint(serde_json::from_str("{}").expect("serde defaults"))
                });
                if let NodeStateData::LtxCheckpoint(s) = state {
                    if s.depth_model_path.is_none() {
                        s.depth_model_path = d.depth_model_path.clone();
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeRuntime ↔ NodeStateData
// ─────────────────────────────────────────────────────────────────────────────

/// Снимок per-kind state из runtime'а ноды. `None` для нод без
/// пользовательского state (`Number`, `Add`, `Output`, `Demo`).
/// pub — используется агентским инструментом `pipelines` (схема state
/// по default_runtime и точечный set_state).
pub fn runtime_to_state(rt: &NodeRuntime) -> Option<NodeStateData> {
    match rt {
        NodeRuntime::None => None,
        NodeRuntime::H3Checkpoint {
            model_path,
            encoder_path,
            lora_path,
            lora_strength,
            variant_idx,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            memory_mode_idx,
            resident,
            ..
        } => Some(NodeStateData::H3Checkpoint(H3CheckpointStateData {
            model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            encoder_path: encoder_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            lora_path: lora_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            lora_strength: lora_strength.get_untracked(),
            variant_idx: variant_idx.get_untracked(),
            device_idx: device_idx.get_untracked(),
            quant_dit_idx: quant_dit_idx.get_untracked(),
            quant_enc_idx: quant_enc_idx.get_untracked(),
            compute_idx: compute_idx.get_untracked(),
            memory_mode_idx: memory_mode_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::H3Sampler { steps, cfg_scale, seed, .. } => {
            Some(NodeStateData::H3Sampler(H3SamplerStateData {
                steps: steps.get_untracked(),
                cfg_scale: cfg_scale.get_untracked(),
                seed: seed.get_untracked(),
            }))
        }
        NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, aspect_idx } => {
            Some(NodeStateData::H3EmptyLatentAv(H3EmptyLatentAvStateData {
                width: width.get_untracked(),
                height: height.get_untracked(),
                duration_seconds: duration_seconds.get_untracked(),
                aspect_idx: aspect_idx.get_untracked(),
            }))
        }
        NodeRuntime::H3Keyframe { path, frame_slot_idx, resize_idx, .. } => {
            Some(NodeStateData::H3Keyframe(H3KeyframeStateData {
                image_path: path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                frame_slot_idx: frame_slot_idx.get_untracked(),
                resize_idx: resize_idx.get_untracked(),
            }))
        }
        NodeRuntime::H3References { items, image_size_idx, .. } => {
            Some(NodeStateData::H3References(H3ReferencesStateData {
                items: items
                    .get_untracked()
                    .iter()
                    .map(|e| H3ReferenceItemData {
                        path: e.path.to_string_lossy().to_string(),
                        use_audio: e.use_audio,
                    })
                    .collect(),
                image_size_idx: image_size_idx.get_untracked(),
            }))
        }
        NodeRuntime::H3TextEncoder { .. }
        | NodeRuntime::H3VaeDecode { .. }
        | NodeRuntime::H3AudioDecode { .. }
        | NodeRuntime::H3VideoSave { .. } => None,
        NodeRuntime::FluxCheckpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. } => {
            Some(NodeStateData::FluxCheckpoint(FluxCheckpointStateData {
                model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                device_idx: device_idx.get_untracked(),
                quant_idx: quant_idx.get_untracked(),
                memory_mode_idx: memory_mode_idx.get_untracked(),
                resident: resident.get_untracked(),
            }))
        }
        NodeRuntime::FluxTextEncoder { seq_len_idx, .. } => {
            Some(NodeStateData::FluxTextEncoder(FluxTextEncoderStateData {
                seq_len_idx: seq_len_idx.get_untracked(),
            }))
        }
        NodeRuntime::FluxEmptyLatent { width, height, aspect_idx } => {
            Some(NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData {
                width: width.get_untracked(),
                height: height.get_untracked(),
                aspect_idx: aspect_idx.get_untracked(),
            }))
        }
        NodeRuntime::FluxVaeEncode { resize_idx, .. } => {
            Some(NodeStateData::FluxVaeEncode(FluxVaeEncodeStateData { resize_idx: resize_idx.get_untracked() }))
        }
        NodeRuntime::FluxSampler { steps, guidance, seed, denoise, .. } => {
            Some(NodeStateData::FluxSampler(FluxSamplerStateData {
                steps: steps.get_untracked(),
                guidance: guidance.get_untracked(),
                seed: seed.get_untracked(),
                denoise: denoise.get_untracked(),
            }))
        }
        NodeRuntime::FluxVaeDecode { .. } => None,
        NodeRuntime::Flux2Checkpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. } => {
            Some(NodeStateData::Flux2Checkpoint(Flux2CheckpointStateData {
                model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                device_idx: device_idx.get_untracked(),
                quant_idx: quant_idx.get_untracked(),
                memory_mode_idx: memory_mode_idx.get_untracked(),
                resident: resident.get_untracked(),
            }))
        }
        NodeRuntime::Flux2VaeEncode { resize_idx, .. } => {
            Some(NodeStateData::Flux2VaeEncode(FluxVaeEncodeStateData { resize_idx: resize_idx.get_untracked() }))
        }
        NodeRuntime::Flux2Sampler { steps, guidance, seed, denoise, .. } => {
            Some(NodeStateData::Flux2Sampler(Flux2SamplerStateData {
                steps: steps.get_untracked(),
                guidance: guidance.get_untracked(),
                seed: seed.get_untracked(),
                denoise: denoise.get_untracked(),
            }))
        }
        NodeRuntime::Flux2TextEncoder { .. } | NodeRuntime::Flux2Reference { .. } | NodeRuntime::Flux2VaeDecode { .. } => None,
        NodeRuntime::QwenImageCheckpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. } => {
            Some(NodeStateData::QwenImageCheckpoint(QwenImageCheckpointStateData {
                model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                device_idx: device_idx.get_untracked(),
                quant_idx: quant_idx.get_untracked(),
                memory_mode_idx: memory_mode_idx.get_untracked(),
                resident: resident.get_untracked(),
            }))
        }
        NodeRuntime::QwenImageSampler { steps, cfg, seed, .. } => {
            Some(NodeStateData::QwenImageSampler(QwenImageSamplerStateData {
                steps: steps.get_untracked(),
                cfg: cfg.get_untracked(),
                seed: seed.get_untracked(),
            }))
        }
        NodeRuntime::QwenImageTextEncoder { .. }
        | NodeRuntime::QwenImageReference { .. }
        | NodeRuntime::QwenImageVaeDecode { .. } => None,
        NodeRuntime::QwenImage21Checkpoint {
            model_path, device_idx, quant_idx, memory_mode_idx, resolution_idx, resident, ..
        } => Some(NodeStateData::QwenImage21Checkpoint(QwenImage21CheckpointStateData {
            model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            device_idx: device_idx.get_untracked(),
            quant_idx: quant_idx.get_untracked(),
            memory_mode_idx: memory_mode_idx.get_untracked(),
            resolution_idx: resolution_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::QwenImage21Sampler { steps, cfg, seed, kv_cache, .. } => {
            Some(NodeStateData::QwenImage21Sampler(QwenImage21SamplerStateData {
                steps: steps.get_untracked(),
                cfg: cfg.get_untracked(),
                seed: seed.get_untracked(),
                kv_cache: kv_cache.get_untracked(),
            }))
        }
        NodeRuntime::QwenImage21TextEncoder { .. }
        | NodeRuntime::QwenImage21Reference { .. }
        | NodeRuntime::QwenImage21VaeDecode { .. } => None,
        NodeRuntime::SdxlCheckpoint { model_path, device_idx, quant_idx, resident, .. } => {
            Some(NodeStateData::SdxlCheckpoint(SdxlCheckpointStateData {
                model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                device_idx: device_idx.get_untracked(),
                quant_idx: quant_idx.get_untracked(),
                resident: resident.get_untracked(),
            }))
        }
        NodeRuntime::SdxlVaeEncode { resize_idx, .. } => {
            Some(NodeStateData::SdxlVaeEncode(FluxVaeEncodeStateData { resize_idx: resize_idx.get_untracked() }))
        }
        NodeRuntime::SdxlSampler { steps, guidance, seed, denoise, .. } => {
            Some(NodeStateData::SdxlSampler(SdxlSamplerStateData {
                steps: steps.get_untracked(),
                guidance: guidance.get_untracked(),
                seed: seed.get_untracked(),
                denoise: denoise.get_untracked(),
            }))
        }
        NodeRuntime::SdxlTextEncoder { .. } | NodeRuntime::SdxlVaeDecode { .. } => None,
        NodeRuntime::ImageLoad { path, .. } => Some(NodeStateData::ImageLoad(ImageLoadStateData {
            image_path: path.get_untracked().map(|p| p.to_string_lossy().to_string()),
        })),
        NodeRuntime::ImageSave { path, .. } => Some(NodeStateData::ImageSave(ImageSaveStateData {
            path: path.get_untracked().map(|p| p.to_string_lossy().to_string()),
        })),
        NodeRuntime::AudioFile { loaded_path, .. } => {
            Some(NodeStateData::AudioFile(AudioFileStateData {
                loaded_path: loaded_path
                    .get_untracked()
                    .map(|p| p.to_string_lossy().into_owned()),
            }))
        }
        NodeRuntime::FfmpegPlayer {
            current_path,
            volume,
            size,
            hwaccel_idx,
            ..
        } => Some(NodeStateData::FfmpegPlayer(FfmpegPlayerStateData {
            source: current_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            volume: volume.get_untracked(),
            size: {
                let s = size.get_untracked();
                (s.width, s.height)
            },
            hwaccel_idx: hwaccel_idx.get_untracked(),
        })),
        NodeRuntime::AsrGigaam {
            output_text,
            ..
        } => Some(NodeStateData::AsrGigaam(AsrGigaamStateData {
            output_text: output_text.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::OmniVoice {
            instruct,
            ref_text_field,
            language,
            num_step,
            guidance_scale,
            t_shift,
            speed,
            seed,
            ..
        } => Some(NodeStateData::OmniVoice(OmniVoiceStateData {
            instruct: instruct.get_untracked(),
            ref_text: ref_text_field.get_untracked(),
            language: language.get_untracked(),
            num_step: num_step.get_untracked(),
            guidance_scale: guidance_scale.get_untracked(),
            t_shift: t_shift.get_untracked(),
            speed: speed.get_untracked(),
            seed: seed.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::Llm {
            system_prompt,
            context,
            think,
            max_tokens,
            temperature,
            top_k,
            top_p,
            min_p,
            repetition_penalty,
            seed,
            ..
        } => Some(NodeStateData::Llm(LlmStateData {
            system_prompt: system_prompt.get_untracked(),
            context: context.get_untracked(),
            think: think.get_untracked(),
            max_tokens: max_tokens.get_untracked(),
            temperature: temperature.get_untracked(),
            top_k: top_k.get_untracked(),
            top_p: top_p.get_untracked(),
            min_p: min_p.get_untracked(),
            repetition_penalty: repetition_penalty.get_untracked(),
            seed: seed.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::VoxCpm2 {
            prompt_text_field,
            cfg_value,
            n_timesteps,
            max_len,
            seed,
            ..
        } => Some(NodeStateData::VoxCpm2(VoxCpm2StateData {
            prompt_text: prompt_text_field.get_untracked(),
            cfg_value: cfg_value.get_untracked(),
            n_timesteps: n_timesteps.get_untracked(),
            max_len: max_len.get_untracked(),
            seed: seed.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::VibeVoice {
            script_field,
            cfg_value,
            ddpm_steps,
            max_length_times,
            seed,
            ..
        } => Some(NodeStateData::VibeVoice(VibeVoiceStateData {
            script: script_field.get_untracked(),
            cfg_value: cfg_value.get_untracked(),
            ddpm_steps: ddpm_steps.get_untracked(),
            max_length_times: max_length_times.get_untracked(),
            seed: seed.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::MarkdownView {
            content,
            edit_mode,
            size,
            ..
        } => {
            let s = size.get_untracked();
            Some(NodeStateData::MarkdownView(MarkdownViewStateData {
                content: content.get_untracked(),
                width: s.width,
                height: s.height,
                edit_mode: edit_mode.get_untracked(),
            }))
        }
        NodeRuntime::TextView {
            output_text, size, ..
        } => {
            let s = size.get_untracked();
            Some(NodeStateData::TextView(TextViewStateData {
                output_text: output_text.get_untracked(),
                width: s.width,
                height: s.height,
            }))
        }
        NodeRuntime::Gain { gain_db, .. } => Some(NodeStateData::Gain(GainStateData {
            gain_db: gain_db.get_untracked(),
        })),
        NodeRuntime::Filter { mode, cutoff_hz, .. } => {
            Some(NodeStateData::Filter(FilterStateData {
                mode: mode.get_untracked(),
                cutoff_hz: cutoff_hz.get_untracked(),
            }))
        }
        NodeRuntime::Reverb { mix, room, .. } => Some(NodeStateData::Reverb(ReverbStateData {
            mix: mix.get_untracked(),
            room: room.get_untracked(),
        })),
        NodeRuntime::Equalizer { gains_db, .. } => {
            Some(NodeStateData::Equalizer(EqualizerStateData {
                gains_db: gains_db.iter().map(|s| s.get_untracked()).collect(),
            }))
        }
        NodeRuntime::Mixer {
            n_inputs, gains_db, ..
        } => Some(NodeStateData::Mixer(MixerStateData {
            n_inputs: n_inputs.get_untracked(),
            gains_db: gains_db.iter().map(|s| s.get_untracked()).collect(),
        })),
        NodeRuntime::AudioPlayer { volume, .. } => {
            Some(NodeStateData::AudioPlayer(AudioPlayerStateData {
                volume: volume.get_untracked(),
            }))
        }
        NodeRuntime::AudioRecorder { device, .. } => {
            Some(NodeStateData::AudioRecorder(AudioRecorderStateData {
                device: device.get_untracked(),
            }))
        }
        NodeRuntime::SaveToFile { path, .. } => {
            Some(NodeStateData::SaveToFile(SaveToFileStateData {
                path: path.get_untracked(),
            }))
        }
        NodeRuntime::SortformerDiarizer {
            threshold,
            allow_overlap,
            output_pretty,
            output_json,
            ..
        } => Some(NodeStateData::SortformerDiarizer(
            crate::templates::model::SortformerDiarizerStateData {
                threshold: threshold.get_untracked(),
                allow_overlap: allow_overlap.get_untracked(),
                output_pretty: output_pretty.get_untracked(),
                output_json: output_json.get_untracked(),
                ..Default::default()
            },
        )),
        // ACE-Step семейство. `loaded_cfg`/`output_buf`/`output_version`/
        // `running`/`error`/`loaded_name`/`progress_pct` НЕ сериализуем —
        // это операционное состояние и тяжёлые тензоры, восстанавливаются
        // лениво на следующий Play.
        NodeRuntime::AceStepVaeEncode {
            chunk_seconds,
            overlap_seconds,
            ..
        } => Some(NodeStateData::AceStepVaeEncode(AceStepVaeStateData {
            chunk_seconds: chunk_seconds.get_untracked(),
            overlap_seconds: overlap_seconds.get_untracked(),
            ..Default::default()
        })),
        NodeRuntime::SynCheckpoint {
            model_path,
            device_idx,
            storage_idx,
            compute_idx,
            resident,
            ..
        } => Some(NodeStateData::SynCheckpoint(SynCheckpointStateData {
            model_path: model_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            device_idx: device_idx.get_untracked(),
            storage_idx: storage_idx.get_untracked(),
            compute_idx: compute_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::LtxCheckpoint {
            model_path,
            gemma_dir,
            upscaler_path,
            depth_model_path,
            lora_path,
            lora_strength,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            resident,
            ..
        } => Some(NodeStateData::LtxCheckpoint(LtxCheckpointStateData {
            model_path: model_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            gemma_dir: gemma_dir
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            upscaler_path: upscaler_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            depth_model_path: depth_model_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            lora_path: lora_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            lora_strength: lora_strength.get_untracked(),
            device_idx: device_idx.get_untracked(),
            quant_dit_idx: quant_dit_idx.get_untracked(),
            quant_enc_idx: quant_enc_idx.get_untracked(),
            compute_idx: compute_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::LtxTextEncoder {
            prompt_field,
            keep_gemma,
            ..
        } => Some(NodeStateData::LtxTextEncoder(LtxTextEncoderStateData {
            prompt: prompt_field.get_untracked(),
            keep_gemma: keep_gemma.get_untracked(),
        })),
        NodeRuntime::LtxNagPrompt {
            prompt_field,
            scale,
            alpha,
            tau,
            ..
        } => Some(NodeStateData::LtxNagPrompt(LtxNagPromptStateData {
            prompt: prompt_field.get_untracked(),
            scale: scale.get_untracked(),
            alpha: alpha.get_untracked(),
            tau: tau.get_untracked(),
        })),
        NodeRuntime::LtxSamplerStage1 {
            width,
            height,
            duration_seconds,
            fps_idx,
            seed,
            ..
        } => Some(NodeStateData::LtxSamplerStage1(LtxSamplerStage1StateData {
            width: width.get_untracked(),
            height: height.get_untracked(),
            duration_seconds: duration_seconds.get_untracked(),
            fps_idx: fps_idx.get_untracked(),
            seed: seed.get_untracked(),
        })),
        NodeRuntime::LtxUpscale { .. } => None,
        NodeRuntime::LtxSamplerStage2 { seed, .. } => {
            Some(NodeStateData::LtxSamplerStage2(LtxSamplerStage2StateData {
                seed: seed.get_untracked(),
            }))
        }
        NodeRuntime::LtxVaeDecode { .. } => None,
        NodeRuntime::LtxAudioDecode { .. } => None,
        NodeRuntime::LtxVideoSave { path, .. } => {
            Some(NodeStateData::LtxVideoSave(LtxVideoSaveStateData {
                path: path.get_untracked(),
            }))
        }
        NodeRuntime::LtxImage { image_path, strength, frame_idx, .. } => {
            Some(NodeStateData::LtxImage(LtxImageStateData {
                image_path: image_path
                    .get_untracked()
                    .map(|p| p.to_string_lossy().to_string()),
                strength: strength.get_untracked(),
                frame_idx: frame_idx.get_untracked(),
            }))
        }
        NodeRuntime::LtxVideoInput { video_path } => {
            Some(NodeStateData::LtxVideoInput(LtxVideoInputStateData {
                video_path: video_path
                    .get_untracked()
                    .map(|p| p.to_string_lossy().to_string()),
            }))
        }
        NodeRuntime::LtxRetake {
            width, height, duration_seconds, fps_idx, retake_start, retake_end, seed, ..
        } => Some(NodeStateData::LtxRetake(LtxRetakeStateData {
            width: width.get_untracked(),
            height: height.get_untracked(),
            duration_seconds: duration_seconds.get_untracked(),
            fps_idx: fps_idx.get_untracked(),
            retake_start: retake_start.get_untracked(),
            retake_end: retake_end.get_untracked(),
            seed: seed.get_untracked(),
        })),
        NodeRuntime::LtxIcLora {
            width, height, duration_seconds, fps_idx, downscale, ref_strength,
            control_idx, canny_low, canny_high, seed, ..
        } => Some(NodeStateData::LtxIcLora(LtxIcLoraStateData {
            width: width.get_untracked(),
            height: height.get_untracked(),
            duration_seconds: duration_seconds.get_untracked(),
            fps_idx: fps_idx.get_untracked(),
            downscale: downscale.get_untracked(),
            ref_strength: ref_strength.get_untracked(),
            control_idx: control_idx.get_untracked(),
            canny_low: canny_low.get_untracked(),
            canny_high: canny_high.get_untracked(),
            depth_model_path: None,
            seed: seed.get_untracked(),
        })),
        NodeRuntime::LtxAudioInput { audio_path } => {
            Some(NodeStateData::LtxAudioInput(LtxAudioInputStateData {
                audio_path: audio_path
                    .get_untracked()
                    .map(|p| p.to_string_lossy().to_string()),
            }))
        }
        NodeRuntime::LtxLipdub {
            width, height, duration_seconds, fps_idx, seed, ..
        } => Some(NodeStateData::LtxLipdub(LtxLipdubStateData {
            width: width.get_untracked(),
            height: height.get_untracked(),
            duration_seconds: duration_seconds.get_untracked(),
            fps_idx: fps_idx.get_untracked(),
            seed: seed.get_untracked(),
        })),
        NodeRuntime::LtxA2V {
            width, height, duration_seconds, fps_idx, seed, ..
        } => Some(NodeStateData::LtxA2V(LtxA2VStateData {
            width: width.get_untracked(),
            height: height.get_untracked(),
            duration_seconds: duration_seconds.get_untracked(),
            fps_idx: fps_idx.get_untracked(),
            seed: seed.get_untracked(),
        })),
        NodeRuntime::Yue2Checkpoint {
            models_dir,
            model_path,
            vae_path,
            device_idx,
            quant_idx,
            compute_idx,
            vae_dtype_idx,
            resident,
            ..
        } => Some(NodeStateData::Yue2Checkpoint(Yue2CheckpointStateData {
            models_dir: models_dir.get_untracked().map(|p| p.to_string_lossy().to_string()),
            model_path: model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            vae_path: vae_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            device_idx: device_idx.get_untracked(),
            quant_idx: quant_idx.get_untracked(),
            compute_idx: compute_idx.get_untracked(),
            vae_dtype_idx: vae_dtype_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::Yue2Generate {
            cot_idx,
            seconds,
            ode_steps,
            cfg_scale,
            seed,
            temperature,
            top_p,
            top_k,
            repetition_penalty,
            vae_core_frames,
            ..
        } => Some(NodeStateData::Yue2Generate(Yue2GenerateStateData {
            cot_idx: cot_idx.get_untracked(),
            seconds: seconds.get_untracked(),
            ode_steps: ode_steps.get_untracked(),
            cfg_scale: cfg_scale.get_untracked(),
            seed: seed.get_untracked(),
            temperature: temperature.get_untracked(),
            top_p: top_p.get_untracked(),
            top_k: top_k.get_untracked(),
            repetition_penalty: repetition_penalty.get_untracked(),
            vae_core_frames: vae_core_frames.get_untracked(),
        })),
        NodeRuntime::Yue2VaeDecode { vae_core_frames, .. } => {
            Some(NodeStateData::Yue2VaeDecode(Yue2VaeDecodeStateData {
                vae_path: None,
                vae_core_frames: vae_core_frames.get_untracked(),
            }))
        }
        NodeRuntime::Yue2Transcribe { mode_idx, voices_idx, .. } => {
            Some(NodeStateData::Yue2Transcribe(Yue2TranscribeStateData {
                mode_idx: mode_idx.get_untracked(),
                voices_idx: voices_idx.get_untracked(),
            }))
        }
        NodeRuntime::AceStepCheckpoint {
            models_dir,
            lm_path,
            text_encoder_path,
            dit_path,
            vae_path,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            resident,
            ..
        } => Some(NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
            models_dir: models_dir.get_untracked().map(|p| p.to_string_lossy().to_string()),
            lm_path: lm_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            text_encoder_path: text_encoder_path
                .get_untracked()
                .map(|p| p.to_string_lossy().to_string()),
            dit_path: dit_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            vae_path: vae_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
            device_idx: device_idx.get_untracked(),
            quant_dit_idx: quant_dit_idx.get_untracked(),
            quant_enc_idx: quant_enc_idx.get_untracked(),
            compute_idx: compute_idx.get_untracked(),
            resident: resident.get_untracked(),
        })),
        NodeRuntime::AceStepGenerate {
            mode_idx,
            preset,
            duration_seconds,
            infer_steps,
            cfg_scale,
            flow_match_shift,
            seed,
            temperature,
            top_p,
            top_k,
            min_p,
            lm_cfg_scale,
            use_cot,
            use_ar,
            bpm,
            keyscale_idx,
            timesig_idx,
            norm_mode,
            enable_dcw,
            dcw_mode,
            dcw_scaler,
            dcw_high_scaler,
            dcw_wavelet,
            dcw_preset,
            retake_variance,
            retake_seed,
            repaint_start_sec,
            repaint_end_sec,
            repaint_strength,
            edit_n_min,
            edit_n_max,
            track_idx,
            ..
        } => {
            use crate::pages::node_editor::types::{
                DcwModeOption, DcwPresetOption, DcwWaveletOption, SamplerPreset,
            };
            use crate::templates::model::{
                DcwModeData, DcwPresetData, DcwWaveletData, SamplerPresetData,
            };
            let preset_data = match preset.get_untracked() {
                SamplerPreset::Auto => SamplerPresetData::Auto,
                SamplerPreset::Turbo => SamplerPresetData::Turbo,
                SamplerPreset::Base => SamplerPresetData::Base,
                SamplerPreset::Sft => SamplerPresetData::Sft,
            };
            let dcw_mode_data = match dcw_mode.get_untracked() {
                DcwModeOption::Low => DcwModeData::Low,
                DcwModeOption::High => DcwModeData::High,
                DcwModeOption::Double => DcwModeData::Double,
                DcwModeOption::Pix => DcwModeData::Pix,
            };
            let dcw_wavelet_data = match dcw_wavelet.get_untracked() {
                DcwWaveletOption::Haar => DcwWaveletData::Haar,
                DcwWaveletOption::Db4 => DcwWaveletData::Db4,
                DcwWaveletOption::Sym8 => DcwWaveletData::Sym8,
            };
            let dcw_preset_data = match dcw_preset.get_untracked() {
                DcwPresetOption::NoThink => DcwPresetData::NoThink,
                DcwPresetOption::Think => DcwPresetData::Think,
                DcwPresetOption::Custom => DcwPresetData::Custom,
            };
            Some(NodeStateData::AceStepGenerate(AceStepGenerateStateData {
                mode_idx: mode_idx.get_untracked(),
                preset: preset_data,
                duration_seconds: duration_seconds.get_untracked(),
                infer_steps: infer_steps.get_untracked(),
                cfg_scale: cfg_scale.get_untracked(),
                flow_match_shift: flow_match_shift.get_untracked(),
                seed: seed.get_untracked(),
                temperature: temperature.get_untracked(),
                top_p: top_p.get_untracked(),
                top_k: top_k.get_untracked(),
                min_p: min_p.get_untracked(),
                lm_cfg_scale: lm_cfg_scale.get_untracked(),
                use_cot: use_cot.get_untracked(),
                use_ar: use_ar.get_untracked(),
                bpm: bpm.get_untracked(),
                keyscale_idx: keyscale_idx.get_untracked(),
                timesig_idx: timesig_idx.get_untracked(),
                norm_mode: norm_mode.get_untracked(),
                enable_dcw: enable_dcw.get_untracked(),
                dcw_mode: dcw_mode_data,
                dcw_scaler: dcw_scaler.get_untracked(),
                dcw_high_scaler: dcw_high_scaler.get_untracked(),
                dcw_wavelet: dcw_wavelet_data,
                dcw_preset: dcw_preset_data,
                retake_variance: retake_variance.get_untracked(),
                retake_seed: retake_seed.get_untracked(),
                repaint_start_sec: repaint_start_sec.get_untracked(),
                repaint_end_sec: repaint_end_sec.get_untracked(),
                repaint_strength: repaint_strength.get_untracked(),
                edit_n_min: edit_n_min.get_untracked(),
                edit_n_max: edit_n_max.get_untracked(),
                track_idx: track_idx.get_untracked(),
                lm_defaults_v2: true,
            }))
        }
    }
}

/// Применить сохранённое `NodeStateData` к свежесозданному `NodeRuntime`
/// (полученному из `registry::default_runtime`). Несовпадение вариантов
/// (`kind` ноды поменялся между версиями) — silent skip; UI и executor
/// продолжают работать с дефолтным runtime'ом.
///
/// Стороны эффекта: `RwSignal::set` бампает subscribers, поэтому UI карточки
/// сразу видит загруженные значения. Тяжёлые объекты (Transcriber/Pipeline)
/// — НЕ трогаются: они грузятся лениво на первый Play.
/// pub — используется агентским инструментом `pipelines` (set_state).
pub fn apply_state_to_runtime(rt: &NodeRuntime, state: &NodeStateData) {
    match (rt, state) {
        (
            NodeRuntime::AsrGigaam {
                output_text,
                text_version,
                ..
            },
            NodeStateData::AsrGigaam(data),
        ) => {
            output_text.set(data.output_text.clone());
            // Bump version → Reactive в body перестраивает MultilineTextEdit
            // с новым initial-text (как после успешного транскрибата).
            text_version.set(text_version.get_untracked().wrapping_add(1));
        }
        (
            NodeRuntime::OmniVoice {
                instruct,
                ref_text_field,
                language,
                num_step,
                guidance_scale,
                t_shift,
                speed,
                seed,
                ..
            },
            NodeStateData::OmniVoice(data),
        ) => {
            instruct.set(data.instruct.clone());
            ref_text_field.set(data.ref_text.clone());
            language.set(data.language.clone());
            num_step.set(data.num_step);
            guidance_scale.set(data.guidance_scale);
            t_shift.set(data.t_shift);
            speed.set(data.speed);
            seed.set(data.seed);
        }
        (
            NodeRuntime::Llm {
                system_prompt,
                context,
                think,
                max_tokens,
                temperature,
                top_k,
                top_p,
                min_p,
                repetition_penalty,
                seed,
                ..
            },
            NodeStateData::Llm(data),
        ) => {
            system_prompt.set(data.system_prompt.clone());
            context.set(data.context);
            think.set(data.think);
            max_tokens.set(data.max_tokens);
            temperature.set(data.temperature);
            top_k.set(data.top_k);
            top_p.set(data.top_p);
            min_p.set(data.min_p);
            repetition_penalty.set(data.repetition_penalty);
            seed.set(data.seed);
        }
        (
            NodeRuntime::VoxCpm2 {
                prompt_text_field,
                cfg_value,
                n_timesteps,
                max_len,
                seed,
                ..
            },
            NodeStateData::VoxCpm2(data),
        ) => {
            prompt_text_field.set(data.prompt_text.clone());
            cfg_value.set(data.cfg_value);
            n_timesteps.set(data.n_timesteps);
            max_len.set(data.max_len);
            seed.set(data.seed);
        }
        (
            NodeRuntime::VibeVoice {
                script_field,
                cfg_value,
                ddpm_steps,
                max_length_times,
                seed,
                ..
            },
            NodeStateData::VibeVoice(data),
        ) => {
            script_field.set(data.script.clone());
            cfg_value.set(data.cfg_value);
            ddpm_steps.set(data.ddpm_steps);
            max_length_times.set(data.max_length_times);
            seed.set(data.seed);
        }
        (
            NodeRuntime::MarkdownView {
                content,
                edit_mode,
                size,
                ..
            },
            NodeStateData::MarkdownView(data),
        ) => {
            content.set(data.content.clone());
            edit_mode.set(data.edit_mode);
            if data.width > 0.0 && data.height > 0.0 {
                size.set(Size::new(data.width, data.height));
            }
        }
        (
            NodeRuntime::TextView {
                output_text,
                text_version,
                size,
                ..
            },
            NodeStateData::TextView(data),
        ) => {
            output_text.set(data.output_text.clone());
            text_version.set(text_version.get_untracked().wrapping_add(1));
            if data.width > 0.0 && data.height > 0.0 {
                size.set(Size::new(data.width, data.height));
            }
        }
        (NodeRuntime::Gain { gain_db, live_gain, .. }, NodeStateData::Gain(data)) => {
            gain_db.set(data.gain_db);
            // Снимок для realtime worker'а — UI-set обновляет live-gain
            // только через body-сигнал, но при load мы делаем sidewise-update,
            // body может ещё не быть смонтирован. Linear: 10^(dB/20).
            let lin = 10.0_f32.powf(data.gain_db / 20.0);
            live_gain.store(lin.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
        (
            NodeRuntime::Filter {
                mode,
                cutoff_hz,
                live_mode,
                live_cutoff,
                coeffs_dirty,
                ..
            },
            NodeStateData::Filter(data),
        ) => {
            mode.set(data.mode);
            cutoff_hz.set(data.cutoff_hz);
            live_mode.store(data.mode.as_index() as u8, std::sync::atomic::Ordering::Relaxed);
            live_cutoff.store(
                data.cutoff_hz.to_bits(),
                std::sync::atomic::Ordering::Relaxed,
            );
            coeffs_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        (
            NodeRuntime::Reverb {
                mix,
                room,
                live_mix,
                live_room,
                params_dirty,
                ..
            },
            NodeStateData::Reverb(data),
        ) => {
            mix.set(data.mix);
            room.set(data.room);
            live_mix.store(data.mix.to_bits(), std::sync::atomic::Ordering::Relaxed);
            live_room.store(data.room.to_bits(), std::sync::atomic::Ordering::Relaxed);
            params_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        (
            NodeRuntime::Equalizer {
                gains_db,
                live_gains,
                coeffs_dirty,
                ..
            },
            NodeStateData::Equalizer(data),
        ) => {
            // prefix-min: длина gains_db в шаблоне может не совпадать с
            // n_bands ноды (n_bands переехал между Equalizer6/10/20/30).
            let n = gains_db.len().min(data.gains_db.len());
            for i in 0..n {
                let v = data.gains_db[i];
                gains_db[i].set(v);
                live_gains[i].store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
            coeffs_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        (
            NodeRuntime::Mixer {
                n_inputs,
                gains_db,
                live_gains,
                ..
            },
            NodeStateData::Mixer(data),
        ) => {
            n_inputs.set(data.n_inputs.clamp(2, MIXER_MAX_INPUTS));
            let n = gains_db.len().min(data.gains_db.len());
            for i in 0..n {
                let v = data.gains_db[i];
                gains_db[i].set(v);
                // dB → linear, как в `default_runtime`.
                let lin = 10.0_f32.powf(v / 20.0);
                live_gains[i].store(lin.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
        (
            NodeRuntime::AudioFile {
                loaded_path,
                buffer,
                load_error,
                ..
            },
            NodeStateData::AudioFile(data),
        ) => {
            let path = data.loaded_path.as_ref().map(PathBuf::from);
            loaded_path.set(path.clone());
            buffer.set(None);
            load_error.set(None);
            if let Some(p) = path {
                crate::pages::node_editor::nodes::audio_file::spawn_load(p, *buffer, *load_error);
            }
        }
        (NodeRuntime::AudioPlayer { volume, .. }, NodeStateData::AudioPlayer(data)) => {
            volume.set(data.volume);
        }
        (NodeRuntime::AudioRecorder { device, .. }, NodeStateData::AudioRecorder(data)) => {
            device.set(data.device.clone());
        }
        (NodeRuntime::SaveToFile { path, .. }, NodeStateData::SaveToFile(data)) => {
            path.set(data.path.clone());
        }
        (
            NodeRuntime::SortformerDiarizer {
                threshold,
                allow_overlap,
                output_pretty,
                output_json,
                text_version,
                ..
            },
            NodeStateData::SortformerDiarizer(data),
        ) => {
            threshold.set(data.threshold);
            allow_overlap.set(data.allow_overlap);
            output_pretty.set(data.output_pretty.clone());
            output_json.set(data.output_json.clone());
            text_version.set(text_version.get_untracked().wrapping_add(1));
        }
        // ── ACE-Step семейство ──────────────────────────────────────
        (
            NodeRuntime::AceStepVaeEncode {
                chunk_seconds,
                overlap_seconds,
                ..
            },
            NodeStateData::AceStepVaeEncode(data),
        ) => {
            chunk_seconds.set(data.chunk_seconds);
            overlap_seconds.set(data.overlap_seconds);
        }
        (
            NodeRuntime::FfmpegPlayer {
                current_path,
                volume,
                size,
                hwaccel_idx,
                ..
            },
            NodeStateData::FfmpegPlayer(data),
        ) => {
            current_path.set(data.source.as_ref().map(PathBuf::from));
            volume.set(data.volume);
            let (w, h) = data.size;
            if w > 0.0 && h > 0.0 {
                size.set(Size::new(w, h));
            }
            hwaccel_idx.set(data.hwaccel_idx);
        }
        (
            NodeRuntime::H3Checkpoint {
                model_path,
                encoder_path,
                lora_path,
                lora_strength,
                variant_idx,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                memory_mode_idx,
                resident,
                ..
            },
            NodeStateData::H3Checkpoint(data),
        ) => {
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            encoder_path.set(data.encoder_path.as_ref().map(PathBuf::from));
            lora_path.set(data.lora_path.as_ref().map(PathBuf::from));
            lora_strength.set(data.lora_strength);
            variant_idx.set(data.variant_idx);
            device_idx.set(data.device_idx);
            quant_dit_idx.set(data.quant_dit_idx);
            quant_enc_idx.set(data.quant_enc_idx);
            compute_idx.set(data.compute_idx);
            memory_mode_idx.set(data.memory_mode_idx);
            resident.set(data.resident);
        }
        (
            NodeRuntime::H3Sampler { steps, cfg_scale, seed, .. },
            NodeStateData::H3Sampler(data),
        ) => {
            steps.set(data.steps);
            cfg_scale.set(data.cfg_scale);
            seed.set(data.seed);
        }
        (
            NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, aspect_idx },
            NodeStateData::H3EmptyLatentAv(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            aspect_idx.set(data.aspect_idx);
        }
        (
            NodeRuntime::FluxCheckpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. },
            NodeStateData::FluxCheckpoint(data),
        ) => {
            use crate::pages::node_editor::nodes::flux;
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx.min(flux::DEVICE_OPTIONS.len() - 1));
            quant_idx.set(data.quant_idx.min(flux::QUANT_OPTIONS.len() - 1));
            memory_mode_idx.set(data.memory_mode_idx.min(flux::MEMORY_MODE_OPTIONS.len() - 1));
            resident.set(data.resident);
        }
        (NodeRuntime::FluxTextEncoder { seq_len_idx, .. }, NodeStateData::FluxTextEncoder(data)) => {
            seq_len_idx.set(data.seq_len_idx.min(crate::pages::node_editor::nodes::flux::SEQ_LEN_OPTIONS.len() - 1));
        }
        (
            NodeRuntime::FluxEmptyLatent { width, height, aspect_idx },
            NodeStateData::FluxEmptyLatent(data),
        ) => {
            use crate::pages::node_editor::nodes::flux::latent::{ASPECT_OPTIONS, DIM_MAX, DIM_MIN};
            width.set(data.width.clamp(DIM_MIN, DIM_MAX));
            height.set(data.height.clamp(DIM_MIN, DIM_MAX));
            aspect_idx.set(data.aspect_idx.min(ASPECT_OPTIONS.len() - 1));
        }
        (NodeRuntime::FluxVaeEncode { resize_idx, .. }, NodeStateData::FluxVaeEncode(data)) => {
            resize_idx.set(data.resize_idx.min(1));
        }
        (
            NodeRuntime::FluxSampler { steps, guidance, seed, denoise, .. },
            NodeStateData::FluxSampler(data),
        ) => {
            steps.set(data.steps.max(1));
            guidance.set(data.guidance);
            seed.set(data.seed);
            denoise.set(data.denoise.clamp(0.0, 1.0));
        }
        (
            NodeRuntime::Flux2Checkpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. },
            NodeStateData::Flux2Checkpoint(data),
        ) => {
            use crate::pages::node_editor::nodes::flux2;
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx.min(flux2::DEVICE_OPTIONS.len() - 1));
            quant_idx.set(data.quant_idx.min(flux2::QUANT_OPTIONS.len() - 1));
            memory_mode_idx.set(data.memory_mode_idx.min(flux2::MEMORY_MODE_OPTIONS.len() - 1));
            resident.set(data.resident);
        }
        (NodeRuntime::Flux2VaeEncode { resize_idx, .. }, NodeStateData::Flux2VaeEncode(data)) => {
            resize_idx.set(data.resize_idx.min(1));
        }
        (NodeRuntime::Flux2Sampler { steps, guidance, seed, denoise, .. }, NodeStateData::Flux2Sampler(data)) => {
            steps.set(data.steps.min(100));
            guidance.set(data.guidance);
            seed.set(data.seed);
            denoise.set(data.denoise.clamp(0.0, 1.0));
        }
        (
            NodeRuntime::QwenImageCheckpoint { model_path, device_idx, quant_idx, memory_mode_idx, resident, .. },
            NodeStateData::QwenImageCheckpoint(data),
        ) => {
            use crate::pages::node_editor::nodes::qwen_image;
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx.min(qwen_image::DEVICE_OPTIONS.len() - 1));
            quant_idx.set(data.quant_idx.min(qwen_image::QUANT_OPTIONS.len() - 1));
            memory_mode_idx.set(data.memory_mode_idx.min(qwen_image::MEMORY_MODE_OPTIONS.len() - 1));
            resident.set(data.resident);
        }
        (NodeRuntime::QwenImageSampler { steps, cfg, seed, .. }, NodeStateData::QwenImageSampler(data)) => {
            steps.set(data.steps.min(100));
            cfg.set(data.cfg.max(1.0));
            seed.set(data.seed);
        }
        (
            NodeRuntime::QwenImage21Checkpoint {
                model_path, device_idx, quant_idx, memory_mode_idx, resolution_idx, resident, ..
            },
            NodeStateData::QwenImage21Checkpoint(data),
        ) => {
            use crate::pages::node_editor::nodes::qwen_image21;
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx.min(qwen_image21::DEVICE_OPTIONS.len() - 1));
            quant_idx.set(data.quant_idx.min(qwen_image21::QUANT_OPTIONS.len() - 1));
            memory_mode_idx.set(data.memory_mode_idx.min(qwen_image21::MEMORY_MODE_OPTIONS.len() - 1));
            resolution_idx.set(data.resolution_idx.min(qwen_image21::RESOLUTION_OPTIONS.len() - 1));
            resident.set(data.resident);
        }
        (
            NodeRuntime::QwenImage21Sampler { steps, cfg, seed, kv_cache, .. },
            NodeStateData::QwenImage21Sampler(data),
        ) => {
            steps.set(data.steps.min(100));
            cfg.set(data.cfg.max(1.0));
            seed.set(data.seed);
            kv_cache.set(data.kv_cache);
        }
        (
            NodeRuntime::SdxlCheckpoint { model_path, device_idx, quant_idx, resident, .. },
            NodeStateData::SdxlCheckpoint(data),
        ) => {
            use crate::pages::node_editor::nodes::sdxl;
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx.min(sdxl::DEVICE_OPTIONS.len() - 1));
            quant_idx.set(data.quant_idx.min(sdxl::QUANT_OPTIONS.len() - 1));
            resident.set(data.resident);
        }
        (NodeRuntime::SdxlVaeEncode { resize_idx, .. }, NodeStateData::SdxlVaeEncode(data)) => {
            resize_idx.set(data.resize_idx.min(1));
        }
        (NodeRuntime::SdxlSampler { steps, guidance, seed, denoise, .. }, NodeStateData::SdxlSampler(data)) => {
            steps.set(data.steps.clamp(1, 100));
            guidance.set(data.guidance);
            seed.set(data.seed);
            denoise.set(data.denoise.clamp(0.0, 1.0));
        }
        (NodeRuntime::ImageLoad { path, .. }, NodeStateData::ImageLoad(data)) => {
            path.set(data.image_path.as_ref().map(PathBuf::from));
        }
        (NodeRuntime::ImageSave { path, .. }, NodeStateData::ImageSave(data)) => {
            path.set(data.path.as_ref().map(PathBuf::from));
        }
        (
            NodeRuntime::H3Keyframe { path, frame_slot_idx, resize_idx, .. },
            NodeStateData::H3Keyframe(data),
        ) => {
            path.set(data.image_path.as_ref().map(PathBuf::from));
            frame_slot_idx.set(data.frame_slot_idx.min(1));
            resize_idx.set(data.resize_idx.min(1));
        }
        (
            NodeRuntime::H3References { items, image_size_idx, error, .. },
            NodeStateData::H3References(data),
        ) => {
            // Файл с незнакомым расширением не теряем молча: строка пропадёт
            // из списка, а сдвиг меток без объяснения хуже явной ошибки.
            let mut entries = Vec::with_capacity(data.items.len());
            let mut unknown = Vec::new();
            for it in &data.items {
                match crate::pages::node_editor::types::H3RefEntry::probe(&it.path, it.use_audio) {
                    Some(e) => entries.push(e),
                    None => unknown.push(it.path.clone()),
                }
            }
            items.set(entries);
            image_size_idx.set(data.image_size_idx.min(1));
            error.set((!unknown.is_empty()).then(|| {
                tr!("node.minimax_h3_references.unknown_type", path = unknown.join(", "))
            }));
        }
        (
            NodeRuntime::SynCheckpoint {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                resident,
                ..
            },
            NodeStateData::SynCheckpoint(data),
        ) => {
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx);
            storage_idx.set(data.storage_idx);
            compute_idx.set(data.compute_idx);
            resident.set(data.resident);
        }
        (
            NodeRuntime::LtxCheckpoint {
                model_path,
                gemma_dir,
                upscaler_path,
                depth_model_path,
                lora_path,
                lora_strength,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                resident,
                ..
            },
            NodeStateData::LtxCheckpoint(data),
        ) => {
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            gemma_dir.set(data.gemma_dir.as_ref().map(PathBuf::from));
            upscaler_path.set(data.upscaler_path.as_ref().map(PathBuf::from));
            depth_model_path.set(data.depth_model_path.as_ref().map(PathBuf::from));
            lora_path.set(data.lora_path.as_ref().map(PathBuf::from));
            lora_strength.set(data.lora_strength);
            device_idx.set(data.device_idx);
            quant_dit_idx.set(data.quant_dit_idx);
            quant_enc_idx.set(data.quant_enc_idx);
            compute_idx.set(data.compute_idx);
            resident.set(data.resident);
        }
        (
            NodeRuntime::LtxTextEncoder {
                prompt_field,
                keep_gemma,
                ..
            },
            NodeStateData::LtxTextEncoder(data),
        ) => {
            prompt_field.set(data.prompt.clone());
            keep_gemma.set(data.keep_gemma);
        }
        (
            NodeRuntime::LtxNagPrompt {
                prompt_field,
                scale,
                alpha,
                tau,
                ..
            },
            NodeStateData::LtxNagPrompt(data),
        ) => {
            prompt_field.set(data.prompt.clone());
            scale.set(data.scale);
            alpha.set(data.alpha);
            tau.set(data.tau);
        }
        (
            NodeRuntime::LtxSamplerStage1 {
                width,
                height,
                duration_seconds,
                fps_idx,
                seed,
                ..
            },
            NodeStateData::LtxSamplerStage1(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            fps_idx.set(data.fps_idx);
            seed.set(data.seed);
        }
        (
            NodeRuntime::LtxSamplerStage2 { seed, .. },
            NodeStateData::LtxSamplerStage2(data),
        ) => {
            seed.set(data.seed);
        }
        (
            NodeRuntime::LtxVideoSave { path, .. },
            NodeStateData::LtxVideoSave(data),
        ) => {
            path.set(data.path.clone());
        }
        (
            NodeRuntime::LtxImage { image_path, strength, frame_idx, .. },
            NodeStateData::LtxImage(data),
        ) => {
            image_path.set(data.image_path.as_ref().map(PathBuf::from));
            strength.set(data.strength);
            frame_idx.set(data.frame_idx);
        }
        (
            NodeRuntime::LtxVideoInput { video_path },
            NodeStateData::LtxVideoInput(data),
        ) => {
            video_path.set(data.video_path.as_ref().map(PathBuf::from));
        }
        (
            NodeRuntime::LtxRetake {
                width, height, duration_seconds, fps_idx, retake_start, retake_end, seed, ..
            },
            NodeStateData::LtxRetake(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            fps_idx.set(data.fps_idx);
            retake_start.set(data.retake_start);
            retake_end.set(data.retake_end);
            seed.set(data.seed);
        }
        (
            NodeRuntime::LtxIcLora {
                width, height, duration_seconds, fps_idx, downscale, ref_strength,
                control_idx, canny_low, canny_high, seed, ..
            },
            NodeStateData::LtxIcLora(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            fps_idx.set(data.fps_idx);
            downscale.set(data.downscale);
            ref_strength.set(data.ref_strength);
            control_idx.set(data.control_idx);
            canny_low.set(data.canny_low);
            canny_high.set(data.canny_high);
            seed.set(data.seed);
        }
        (
            NodeRuntime::LtxAudioInput { audio_path },
            NodeStateData::LtxAudioInput(data),
        ) => {
            audio_path.set(data.audio_path.as_ref().map(PathBuf::from));
        }
        (
            NodeRuntime::LtxLipdub {
                width, height, duration_seconds, fps_idx, seed, ..
            },
            NodeStateData::LtxLipdub(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            fps_idx.set(data.fps_idx);
            seed.set(data.seed);
        }
        (
            NodeRuntime::LtxA2V {
                width, height, duration_seconds, fps_idx, seed, ..
            },
            NodeStateData::LtxA2V(data),
        ) => {
            width.set(data.width);
            height.set(data.height);
            duration_seconds.set(data.duration_seconds);
            fps_idx.set(data.fps_idx);
            seed.set(data.seed);
        }
        (
            NodeRuntime::Yue2Checkpoint {
                models_dir,
                model_path,
                vae_path,
                device_idx,
                quant_idx,
                compute_idx,
                vae_dtype_idx,
                resident,
                ..
            },
            NodeStateData::Yue2Checkpoint(data),
        ) => {
            // `None` в шаблоне = «каталог приложения» (дефолт runtime'а), а не
            // «сбросить в пусто».
            if let Some(dir) = &data.models_dir {
                models_dir.set(Some(PathBuf::from(dir)));
            }
            model_path.set(data.model_path.as_ref().map(PathBuf::from));
            vae_path.set(data.vae_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx);
            quant_idx.set(data.quant_idx);
            compute_idx.set(data.compute_idx);
            vae_dtype_idx.set(data.vae_dtype_idx);
            resident.set(data.resident);
        }
        (
            NodeRuntime::Yue2Generate {
                cot_idx,
                seconds,
                ode_steps,
                cfg_scale,
                seed,
                temperature,
                top_p,
                top_k,
                repetition_penalty,
                vae_core_frames,
                ..
            },
            NodeStateData::Yue2Generate(data),
        ) => {
            cot_idx.set(data.cot_idx);
            seconds.set(data.seconds);
            ode_steps.set(data.ode_steps);
            cfg_scale.set(data.cfg_scale);
            seed.set(data.seed);
            temperature.set(data.temperature);
            top_p.set(data.top_p);
            top_k.set(data.top_k);
            repetition_penalty.set(data.repetition_penalty);
            vae_core_frames.set(data.vae_core_frames);
        }
        (
            NodeRuntime::Yue2VaeDecode { vae_core_frames, .. },
            NodeStateData::Yue2VaeDecode(data),
        ) => {
            vae_core_frames.set(data.vae_core_frames);
        }
        (NodeRuntime::Yue2Transcribe { mode_idx, voices_idx, .. }, NodeStateData::Yue2Transcribe(data)) => {
            mode_idx.set(data.mode_idx);
            voices_idx.set(data.voices_idx);
        }
        (
            NodeRuntime::AceStepCheckpoint {
                models_dir,
                lm_path,
                text_encoder_path,
                dit_path,
                vae_path,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                resident,
                ..
            },
            NodeStateData::AceStepCheckpoint(data),
        ) => {
            // `None` в шаблоне = «каталог приложения» (дефолт runtime'а из
            // `registry::default_runtime`), а не «сбросить в пусто».
            if let Some(dir) = &data.models_dir {
                models_dir.set(Some(PathBuf::from(dir)));
            }
            lm_path.set(data.lm_path.as_ref().map(PathBuf::from));
            text_encoder_path.set(data.text_encoder_path.as_ref().map(PathBuf::from));
            dit_path.set(data.dit_path.as_ref().map(PathBuf::from));
            vae_path.set(data.vae_path.as_ref().map(PathBuf::from));
            device_idx.set(data.device_idx);
            quant_dit_idx.set(data.quant_dit_idx);
            quant_enc_idx.set(data.quant_enc_idx);
            compute_idx.set(data.compute_idx);
            resident.set(data.resident);
        }
        (
            NodeRuntime::AceStepGenerate {
                mode_idx,
                preset,
                duration_seconds,
                infer_steps,
                cfg_scale,
                flow_match_shift,
                seed,
                temperature,
                top_p,
                top_k,
                min_p,
                lm_cfg_scale,
                use_cot,
                use_ar,
                bpm,
                keyscale_idx,
                timesig_idx,
                norm_mode,
                enable_dcw,
                dcw_mode,
                dcw_scaler,
                dcw_high_scaler,
                dcw_wavelet,
                dcw_preset,
                retake_variance,
                retake_seed,
                repaint_start_sec,
                repaint_end_sec,
                repaint_strength,
                edit_n_min,
                edit_n_max,
                track_idx,
                ..
            },
            NodeStateData::AceStepGenerate(data),
        ) => {
            use crate::pages::node_editor::types::{
                DcwModeOption, DcwPresetOption, DcwWaveletOption, SamplerPreset,
            };
            use crate::templates::model::{
                DcwModeData, DcwPresetData, DcwWaveletData, SamplerPresetData,
            };
            mode_idx.set(data.mode_idx);
            preset.set(match data.preset {
                SamplerPresetData::Auto => SamplerPreset::Auto,
                SamplerPresetData::Turbo => SamplerPreset::Turbo,
                SamplerPresetData::Base => SamplerPreset::Base,
                SamplerPresetData::Sft => SamplerPreset::Sft,
            });
            duration_seconds.set(data.duration_seconds);
            infer_steps.set(data.infer_steps);
            cfg_scale.set(data.cfg_scale);
            flow_match_shift.set(data.flow_match_shift);
            seed.set(data.seed);
            temperature.set(data.temperature);
            top_p.set(data.top_p);
            // Графы до pkgrel 219: 50/1.5 — дефолт старой ArLm-ноды, а не выбор
            // пользователя; на 0/2.0 вокал внятнее (CER 0,15 → 0,07 на 5 сидах).
            // Однократно — см. `lm_defaults_v2`.
            let legacy_lm = !data.lm_defaults_v2
                && data.top_k == 50
                && (data.lm_cfg_scale - 1.5).abs() < 1e-6;
            top_k.set(if legacy_lm { 0 } else { data.top_k });
            min_p.set(data.min_p);
            lm_cfg_scale.set(if legacy_lm { 2.0 } else { data.lm_cfg_scale });
            use_cot.set(data.use_cot);
            use_ar.set(data.use_ar);
            bpm.set(data.bpm);
            keyscale_idx.set(data.keyscale_idx);
            timesig_idx.set(data.timesig_idx);
            norm_mode.set(data.norm_mode);
            enable_dcw.set(data.enable_dcw);
            dcw_mode.set(match data.dcw_mode {
                DcwModeData::Low => DcwModeOption::Low,
                DcwModeData::High => DcwModeOption::High,
                DcwModeData::Double => DcwModeOption::Double,
                DcwModeData::Pix => DcwModeOption::Pix,
            });
            dcw_scaler.set(data.dcw_scaler);
            dcw_high_scaler.set(data.dcw_high_scaler);
            dcw_wavelet.set(match data.dcw_wavelet {
                DcwWaveletData::Haar => DcwWaveletOption::Haar,
                DcwWaveletData::Db4 => DcwWaveletOption::Db4,
                DcwWaveletData::Sym8 => DcwWaveletOption::Sym8,
            });
            dcw_preset.set(match data.dcw_preset {
                DcwPresetData::NoThink => DcwPresetOption::NoThink,
                DcwPresetData::Think => DcwPresetOption::Think,
                DcwPresetData::Custom => DcwPresetOption::Custom,
            });
            retake_variance.set(data.retake_variance);
            retake_seed.set(data.retake_seed);
            repaint_start_sec.set(data.repaint_start_sec);
            repaint_end_sec.set(data.repaint_end_sec);
            repaint_strength.set(data.repaint_strength);
            edit_n_min.set(data.edit_n_min);
            edit_n_max.set(data.edit_n_max);
            track_idx.set(data.track_idx);
        }
        _ => {}
    }
}

/// Создаёт `Arc<Mutex<HashMap<&'static str, FieldValue>>>` с дефолтными
/// сигналами по схеме (как `default_fields`), но потом проставляет
/// сохранённые значения по `data` в существующие сигналы — чтобы UI
/// видел те же RwSignal'ы (стабильные подписки) как у любой другой ноды.
fn build_fields(
    kind: NodeKind,
    data: &std::collections::BTreeMap<String, FieldValueData>,
) -> Arc<Mutex<HashMap<&'static str, FieldValue>>> {
    let arc = default_fields(kind);
    {
        let map = arc.lock().unwrap();
        for (raw_name, raw_val) in data {
            let Some(name_static) = resolve_field_name(kind, raw_name) else {
                continue;
            };
            let Some(slot) = map.get(name_static) else {
                continue;
            };
            apply_field_value(slot, raw_val);
        }
    }
    arc
}

fn apply_field_value(slot: &FieldValue, raw: &FieldValueData) {
    match (slot, raw) {
        (FieldValue::Text(s), FieldValueData::Text(v)) => s.set(v.clone()),
        (FieldValue::Float(s), FieldValueData::Float(v)) => s.set(*v),
        (FieldValue::Int(s), FieldValueData::Int(v)) => s.set(*v),
        (FieldValue::Bool(s), FieldValueData::Bool(v)) => s.set(*v),
        (FieldValue::Color(s), FieldValueData::Color(hex)) => {
            // Color::from_hex принимает `#RRGGBB` или `#RRGGBBAA`. Если
            // строка битая — игнорируем, оставляя дефолт из default_fields.
            let c = Color::from_hex(hex);
            // Color::from_hex не возвращает Result, отсутствие validate'а —
            // принимается на веру; пустые/мусорные строки дадут чёрный.
            s.set(c);
        }
        (FieldValue::Choice(s), FieldValueData::Choice(v)) => s.set(*v),
        // Несовпадение типов (например, в шаблоне `Float`, а схема говорит
        // `Int`) — пропускаем. Это может случиться если registry-схема
        // изменилась между версиями.
        _ => {}
    }
}

/// Резолвит динамическое имя порта обратно к `&'static str` из registry.
/// `None` — не найден среди inputs/outputs соответствующего side.
pub fn resolve_port_name(
    kind: Option<NodeKind>,
    side: PortSide,
    name: &str,
) -> Option<&'static str> {
    let kind = kind?;
    let m = meta(kind);
    // Шаблон загружается ДО создания NodeInstance — instance-aware resolve()
    // недоступен. Берём pool: для статических нод это сами inputs/outputs,
    // для динамических (Mixer) — полный список всех возможных портов.
    let list = match side {
        PortSide::Input => m.inputs.pool(),
        PortSide::Output => m.outputs.pool(),
    };
    list.iter().find(|p| p.name == name).map(|p| p.name)
}

/// Резолвит имя поля → `&'static str` из registry.
pub fn resolve_field_name(kind: NodeKind, name: &str) -> Option<&'static str> {
    let m = meta(kind);
    m.fields.iter().find(|f| f.name == name).map(|f| f.name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers on Template
// ─────────────────────────────────────────────────────────────────────────────

impl Template {
    /// Найти `NodeKind` ноды по её локальному id внутри шаблона.
    pub fn kind_of(&self, local_id: u64) -> Option<NodeKind> {
        self.nodes.iter().find(|n| n.id == local_id).map(|n| n.kind)
    }

    /// Прямоугольник, охватывающий все позиции нод (для preview-fit-to-bbox).
    /// Если нод нет — None.
    pub fn bbox(&self) -> Option<(PointData, PointData)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for n in &self.nodes {
            min_x = min_x.min(n.pos.x);
            min_y = min_y.min(n.pos.y);
            max_x = max_x.max(n.pos.x);
            max_y = max_y.max(n.pos.y);
        }
        Some((PointData { x: min_x, y: min_y }, PointData { x: max_x, y: max_y }))
    }
}

// silence unused-import warning when this module compiles standalone.
#[allow(dead_code)]
fn _use_registry(_: &registry::NodeKindMeta) {}

// ─────────────────────────────────────────────────────────────────────────────
// Roundtrip-тесты сохранения per-kind state ноды.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::node_editor::types::{FilterMode, PostNormMode};
    use crate::templates::model::TemplateKind;

    /// Подготавливает пустой ctx (без стартовой demo-ноды), снэпшотит шаблон,
    /// прогоняет через JSON serialize/deserialize и применяет в новый ctx.
    /// Возвращает второй ctx с восстановленным графом.
    fn roundtrip(t: &Template) -> NodeEditorCtx {
        let json = serde_json::to_string_pretty(t).expect("serialize");
        let restored: Template = serde_json::from_str(&json).expect("deserialize");

        let ctx = NodeEditorCtx::new();
        // NodeEditorCtx::new добавляет одну demo-ноду — убираем чтобы
        // снэпшот применился «на чистый граф».
        ctx.nodes.set(Vec::new());
        ctx.connections.set(Vec::new());
        super::load_into_ctx(&ctx, &restored);
        ctx
    }

    fn first_node(ctx: &NodeEditorCtx) -> NodeInstance {
        ctx.nodes.get_untracked().into_iter().next().expect("node")
    }

    fn make_template(nodes: Vec<NodeData>) -> Template {
        Template {
            id: String::new(),
            builtin: false,
            name: "test".into(),
            description: String::new(),
            kind: TemplateKind::Full,
            nodes,
            connections: Vec::new(),
            viewport: None,
        }
    }

    #[test]
    fn roundtrip_syn_checkpoint_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::SynCheckpoint,
            pos: PointData { x: 10.0, y: 20.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::SynCheckpoint(SynCheckpointStateData {
                model_path: Some("/tmp/qwen3.syn".into()),
                device_idx: 1,
                storage_idx: 4,
                compute_idx: 2,
                resident: false,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::SynCheckpoint {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                resident,
                ..
            } => {
                assert_eq!(
                    model_path.get_untracked().map(|p| p.to_string_lossy().to_string()),
                    Some("/tmp/qwen3.syn".into())
                );
                assert_eq!(device_idx.get_untracked(), 1);
                assert_eq!(storage_idx.get_untracked(), 4);
                assert_eq!(compute_idx.get_untracked(), 2);
                assert!(!resident.get_untracked());
            }
            other => panic!("не SynCheckpoint: {other:?}"),
        }
    }

    #[test]
    fn roundtrip_vibevoice_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::VibeVoice,
            pos: PointData { x: 10.0, y: 20.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::VibeVoice(VibeVoiceStateData {
                model_path: Some("/tmp/vibevoice-1.5b.syn".into()),
                device_idx: 1,
                compute_idx: 2,
                script: "Speaker 1: Привет.".into(),
                cfg_value: 1.7,
                ddpm_steps: 12,
                max_length_times: 3.5,
                seed: 99,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::VibeVoice {
                script_field,
                cfg_value,
                ddpm_steps,
                max_length_times,
                seed,
                ..
            } => {
                assert_eq!(script_field.get_untracked(), "Speaker 1: Привет.");
                assert!((cfg_value.get_untracked() - 1.7).abs() < 1e-6);
                assert_eq!(ddpm_steps.get_untracked(), 12);
                assert!((max_length_times.get_untracked() - 3.5).abs() < 1e-6);
                assert_eq!(seed.get_untracked(), 99);
            }
            other => panic!("не VibeVoice: {other:?}"),
        }
    }

    /// Старый JSON без поля resident получает дефолт true (слотовое
    /// поведение).
    #[test]
    fn syn_checkpoint_state_resident_defaults_true() {
        let json = r#"{ "kind": "SynCheckpoint", "data": { "model_path": "/x.syn" } }"#;
        let s: NodeStateData = serde_json::from_str(json).unwrap();
        match s {
            NodeStateData::SynCheckpoint(d) => assert!(d.resident),
            other => panic!("не SynCheckpoint: {other:?}"),
        }
    }

    /// Старый граф: LLM-нода с моделью в себе. При загрузке рядом появляется
    /// Syn Checkpoint с тем же путём и предпочтениями, подключённый к
    /// `model`; при сохранении путь у LLM больше не пишется.
    #[test]
    fn legacy_inline_model_migrates_to_syn_checkpoint() {
        let json = r#"{
            "name": "old", "kind": "full",
            "nodes": [{
                "id": 7, "kind": "llm", "pos": { "x": 500.0, "y": 100.0 },
                "state": { "kind": "Llm", "data": {
                    "model_path": "/m/qwen.syn", "device_idx": 0, "quant_idx": 1, "compute_idx": 1
                } }
            }]
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        let ctx = NodeEditorCtx::new();
        ctx.nodes.set(Vec::new());
        ctx.connections.set(Vec::new());
        super::load_into_ctx(&ctx, &t);
        let nodes = ctx.nodes.get_untracked();
        assert_eq!(nodes.len(), 2);
        let llm = nodes.iter().find(|n| n.kind == NodeKind::Llm).unwrap();
        let ck = nodes.iter().find(|n| n.kind == NodeKind::SynCheckpoint).unwrap();
        match &*ck.runtime.lock().unwrap() {
            NodeRuntime::SynCheckpoint {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                resident,
                ..
            } => {
                assert_eq!(model_path.get_untracked(), Some(PathBuf::from("/m/qwen.syn")));
                assert_eq!(device_idx.get_untracked(), 1); // CUDA
                assert_eq!(storage_idx.get_untracked(), 4); // NVFP4
                assert_eq!(compute_idx.get_untracked(), 1); // F16
                assert!(resident.get_untracked());
            }
            other => panic!("не SynCheckpoint: {other:?}"),
        }
        assert!(ctx
            .connections
            .get_untracked()
            .iter()
            .any(|c| c.from_node == ck.id && c.to_node == llm.id && c.to_port == "model"));

        // Повторная загрузка сохранённого графа второго чекпойнта не добавляет.
        let (nodes_data, conns_data, _) = super::snapshot(&ctx);
        let saved = serde_json::to_string(&nodes_data).unwrap();
        assert_eq!(saved.matches("/m/qwen.syn").count(), 1, "{saved}");
        let t2 = Template { nodes: nodes_data, connections: conns_data, ..t };
        let ctx2 = roundtrip(&t2);
        assert_eq!(ctx2.nodes.get_untracked().len(), 2);
    }

    /// Семейные ноды старых графов: VAE Encode подключается к чекпойнту
    /// ACE-Step, декодер YuE2 уезжает в копию чекпойнта, depth-модель
    /// IC-LoRA — в подключённый LTX Checkpoint.
    #[test]
    fn legacy_family_models_migrate_to_checkpoints() {
        let json = r#"{
            "name": "old", "kind": "full",
            "nodes": [
                { "id": 1, "kind": "ace_step_checkpoint", "pos": { "x": 0.0, "y": 0.0 } },
                { "id": 2, "kind": "ace_step_vae_encode", "pos": { "x": 400.0, "y": 0.0 },
                  "state": { "kind": "AceStepVaeEncode", "data": { "device_idx": 1, "compute_idx": 1 } } },
                { "id": 3, "kind": "yue2_checkpoint", "pos": { "x": 0.0, "y": 300.0 },
                  "state": { "kind": "Yue2Checkpoint", "data": { "quant_idx": 2 } } },
                { "id": 4, "kind": "yue2_vae_decode", "pos": { "x": 400.0, "y": 300.0 },
                  "state": { "kind": "Yue2VaeDecode", "data": { "vae_path": "/m/yue2-vae-legacy.syn" } } },
                { "id": 5, "kind": "ltx_checkpoint", "pos": { "x": 0.0, "y": 600.0 } },
                { "id": 6, "kind": "ltx_ic_lora", "pos": { "x": 400.0, "y": 600.0 },
                  "state": { "kind": "LtxIcLora", "data": { "control_idx": 2, "depth_model_path": "/m/depth" } } }
            ],
            "connections": [
                { "from_node": 3, "from_port": "model", "to_node": 4, "to_port": "model" },
                { "from_node": 5, "from_port": "model", "to_node": 6, "to_port": "model" }
            ]
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        let m = super::migrate_inline_models(&t).expect("есть что мигрировать");

        // VAE Encode — к существующему ACE-Step Checkpoint, новых чекпойнтов нет.
        assert!(m.connections.iter().any(|c| c.from_node == 1 && c.to_node == 2 && c.to_port == "model"));
        assert_eq!(m.nodes.iter().filter(|n| n.kind == NodeKind::AceStepCheckpoint).count(), 1);

        // YuE2: копия чекпойнта 3 с override'ом VAE, декодер переподключён к ней.
        let ck = m
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Yue2Checkpoint && n.id != 3)
            .expect("копия чекпойнта");
        match &ck.state {
            Some(NodeStateData::Yue2Checkpoint(s)) => {
                assert_eq!(s.vae_path.as_deref(), Some("/m/yue2-vae-legacy.syn"));
                assert_eq!(s.quant_idx, 2);
            }
            other => panic!("{other:?}"),
        }
        let to_decode: Vec<_> = m.connections.iter().filter(|c| c.to_node == 4 && c.to_port == "model").collect();
        assert_eq!(to_decode.len(), 1);
        assert_eq!(to_decode[0].from_node, ck.id);

        // IC-LoRA: depth-модель — в LTX Checkpoint 5.
        let ltx = m.nodes.iter().find(|n| n.id == 5).unwrap();
        match &ltx.state {
            Some(NodeStateData::LtxCheckpoint(s)) => assert_eq!(s.depth_model_path.as_deref(), Some("/m/depth")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn roundtrip_asr_gigaam_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AsrGigaam,
            pos: PointData { x: 50.0, y: 60.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AsrGigaam(AsrGigaamStateData {
                model_path: Some("/tmp/test.syn".into()),
                device_idx: 1,
                storage_idx: 2,
                compute_idx: 3,
                output_text: "hello world".into(),
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AsrGigaam {
                output_text,
                ..
            } => {
                assert_eq!(output_text.get_untracked(), "hello world");
            }
            other => panic!("Expected AsrGigaam runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_omnivoice_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::OmniVoice,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::OmniVoice(OmniVoiceStateData {
                model_path: Some("/tmp/omni.syn".into()),
                device_idx: 1,
                storage_idx: 4,
                compute_idx: 2,
                instruct: "soft female voice".into(),
                ref_text: "the quick fox".into(),
                language: "en".into(),
                num_step: 48,
                guidance_scale: 3.0,
                t_shift: 0.2,
                speed: 1.25,
                seed: 42,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::OmniVoice {
                instruct,
                ref_text_field,
                language,
                num_step,
                guidance_scale,
                t_shift,
                speed,
                seed,
                ..
            } => {
                assert_eq!(instruct.get_untracked(), "soft female voice");
                assert_eq!(ref_text_field.get_untracked(), "the quick fox");
                assert_eq!(language.get_untracked(), "en");
                assert_eq!(num_step.get_untracked(), 48);
                assert!((guidance_scale.get_untracked() - 3.0).abs() < 1e-6);
                assert!((t_shift.get_untracked() - 0.2).abs() < 1e-6);
                assert!((speed.get_untracked() - 1.25).abs() < 1e-6);
                assert_eq!(seed.get_untracked(), 42);
            }
            other => panic!("Expected OmniVoice runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_llm_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::Llm,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::Llm(LlmStateData {
                model_path: Some("/tmp/qwen3".into()),
                device_idx: 1,
                quant_idx: 2,
                compute_idx: 1,
                system_prompt: "Ты лаконичный ассистент.".into(),
                context: 8192,
                think: true,
                max_tokens: 256,
                temperature: 0.3,
                top_k: 40,
                top_p: 0.9,
                min_p: 0.05,
                repetition_penalty: 1.1,
                seed: 7,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::Llm {
                system_prompt,
                context,
                think,
                max_tokens,
                temperature,
                top_k,
                top_p,
                min_p,
                repetition_penalty,
                seed,
                ..
            } => {
                assert_eq!(system_prompt.get_untracked(), "Ты лаконичный ассистент.");
                assert_eq!(context.get_untracked(), 8192);
                assert!(think.get_untracked());
                assert_eq!(max_tokens.get_untracked(), 256);
                assert!((temperature.get_untracked() - 0.3).abs() < 1e-6);
                assert_eq!(top_k.get_untracked(), 40);
                assert!((top_p.get_untracked() - 0.9).abs() < 1e-6);
                assert!((min_p.get_untracked() - 0.05).abs() < 1e-6);
                assert!((repetition_penalty.get_untracked() - 1.1).abs() < 1e-6);
                assert_eq!(seed.get_untracked(), 7);
            }
            other => panic!("Expected Llm runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_flux_and_image_states() {
        let mk = |id: u64, kind: NodeKind, state: NodeStateData| NodeData {
            id,
            kind,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(state),
        };
        let nodes = vec![
            mk(
                1,
                NodeKind::FluxCheckpoint,
                NodeStateData::FluxCheckpoint(FluxCheckpointStateData {
                    model_path: Some("/models/flux.1-dev.syn".into()),
                    device_idx: 0,
                    quant_idx: 0,
                    memory_mode_idx: 2,
                    resident: true,
                }),
            ),
            mk(2, NodeKind::FluxTextEncoder, NodeStateData::FluxTextEncoder(FluxTextEncoderStateData { seq_len_idx: 1 })),
            mk(
                3,
                NodeKind::FluxEmptyLatent,
                NodeStateData::FluxEmptyLatent(FluxEmptyLatentStateData { width: 1344, height: 752, aspect_idx: 1 }),
            ),
            mk(4, NodeKind::FluxVaeEncode, NodeStateData::FluxVaeEncode(FluxVaeEncodeStateData { resize_idx: 1 })),
            mk(
                5,
                NodeKind::FluxSampler,
                NodeStateData::FluxSampler(FluxSamplerStateData { steps: 20, guidance: 4.5, seed: 77, denoise: 0.6 }),
            ),
            mk(6, NodeKind::ImageLoad, NodeStateData::ImageLoad(ImageLoadStateData { image_path: Some("/tmp/in.png".into()) })),
            mk(7, NodeKind::ImageSave, NodeStateData::ImageSave(ImageSaveStateData { path: Some("/tmp/out.png".into()) })),
        ];
        let want: Vec<Option<NodeStateData>> = nodes.iter().map(|n| n.state.clone()).collect();
        let ctx = roundtrip(&make_template(nodes));
        let got: Vec<Option<NodeStateData>> = ctx
            .nodes
            .get_untracked()
            .iter()
            .map(|n| runtime_to_state(&n.runtime.lock().unwrap()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn roundtrip_flux2_states() {
        let mk = |id: u64, kind: NodeKind, state: NodeStateData| NodeData {
            id,
            kind,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(state),
        };
        let nodes = vec![
            mk(
                1,
                NodeKind::Flux2Checkpoint,
                NodeStateData::Flux2Checkpoint(Flux2CheckpointStateData {
                    model_path: Some("/models/flux.2-klein-4b.syn".into()),
                    device_idx: 0,
                    quant_idx: 0,
                    memory_mode_idx: 2,
                    resident: true,
                }),
            ),
            mk(
                2,
                NodeKind::Flux2Sampler,
                NodeStateData::Flux2Sampler(Flux2SamplerStateData { steps: 28, guidance: 3.0, seed: 9, denoise: 0.55 }),
            ),
            mk(3, NodeKind::Flux2VaeEncode, NodeStateData::Flux2VaeEncode(FluxVaeEncodeStateData { resize_idx: 1 })),
        ];
        let want: Vec<Option<NodeStateData>> = nodes.iter().map(|n| n.state.clone()).collect();
        let ctx = roundtrip(&make_template(nodes));
        let got: Vec<Option<NodeStateData>> =
            ctx.nodes.get_untracked().iter().map(|n| runtime_to_state(&n.runtime.lock().unwrap())).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn roundtrip_qwen_image_and_sdxl_states() {
        let mk = |id: u64, kind: NodeKind, state: NodeStateData| NodeData {
            id,
            kind,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(state),
        };
        let nodes = vec![
            mk(
                1,
                NodeKind::QwenImageCheckpoint,
                NodeStateData::QwenImageCheckpoint(QwenImageCheckpointStateData {
                    model_path: Some("/models/qwen-image-edit-2511.syn".into()),
                    device_idx: 0,
                    quant_idx: 1,
                    memory_mode_idx: 2,
                    resident: true,
                }),
            ),
            mk(
                2,
                NodeKind::QwenImageSampler,
                NodeStateData::QwenImageSampler(QwenImageSamplerStateData { steps: 20, cfg: 2.5, seed: 7 }),
            ),
            mk(
                6,
                NodeKind::QwenImage21Checkpoint,
                NodeStateData::QwenImage21Checkpoint(QwenImage21CheckpointStateData {
                    model_path: Some("/models/qwen-image-2.1.syn".into()),
                    device_idx: 0,
                    quant_idx: 2,
                    memory_mode_idx: 1,
                    resolution_idx: 4,
                    resident: true,
                }),
            ),
            mk(
                7,
                NodeKind::QwenImage21Sampler,
                NodeStateData::QwenImage21Sampler(QwenImage21SamplerStateData {
                    steps: 12,
                    cfg: 1.0,
                    seed: 3,
                    kv_cache: false,
                }),
            ),
            mk(
                3,
                NodeKind::SdxlCheckpoint,
                NodeStateData::SdxlCheckpoint(SdxlCheckpointStateData {
                    model_path: Some("/models/sdxl-base-1.0.syn".into()),
                    device_idx: 0,
                    quant_idx: 2,
                    resident: true,
                }),
            ),
            mk(4, NodeKind::SdxlVaeEncode, NodeStateData::SdxlVaeEncode(FluxVaeEncodeStateData { resize_idx: 1 })),
            mk(
                5,
                NodeKind::SdxlSampler,
                NodeStateData::SdxlSampler(SdxlSamplerStateData { steps: 25, guidance: 7.0, seed: 3, denoise: 0.4 }),
            ),
        ];
        let want: Vec<Option<NodeStateData>> = nodes.iter().map(|n| n.state.clone()).collect();
        let ctx = roundtrip(&make_template(nodes));
        let got: Vec<Option<NodeStateData>> =
            ctx.nodes.get_untracked().iter().map(|n| runtime_to_state(&n.runtime.lock().unwrap())).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn roundtrip_h3_checkpoint_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::H3Checkpoint,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::H3Checkpoint(H3CheckpointStateData {
                model_path: Some("/models/minimax-h3-fl2va.syn".into()),
                encoder_path: Some("/models/minimax-h3-qwen3vl-encoder.syn".into()),
                lora_path: Some("/models/turbo.safetensors".into()),
                lora_strength: 0.85,
                variant_idx: 1,
                device_idx: 0,
                quant_dit_idx: 1,
                quant_enc_idx: 2,
                resident: false,
                compute_idx: 1,
                memory_mode_idx: 2,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::H3Checkpoint {
                model_path,
                encoder_path,
                lora_path,
                lora_strength,
                variant_idx,
                device_idx,
                quant_dit_idx,
                quant_enc_idx,
                compute_idx,
                memory_mode_idx,
                ..
            } => {
                let path = |s: Option<PathBuf>| s.map(|p| p.to_string_lossy().to_string());
                assert_eq!(
                    path(model_path.get_untracked()),
                    Some("/models/minimax-h3-fl2va.syn".into())
                );
                assert_eq!(
                    path(encoder_path.get_untracked()),
                    Some("/models/minimax-h3-qwen3vl-encoder.syn".into())
                );
                assert_eq!(
                    path(lora_path.get_untracked()),
                    Some("/models/turbo.safetensors".into())
                );
                assert!((lora_strength.get_untracked() - 0.85).abs() < 1e-6);
                assert_eq!(variant_idx.get_untracked(), 1);
                assert_eq!(device_idx.get_untracked(), 0);
                assert_eq!(quant_dit_idx.get_untracked(), 1);
                assert_eq!(quant_enc_idx.get_untracked(), 2);
                assert_eq!(compute_idx.get_untracked(), 1);
                assert_eq!(memory_mode_idx.get_untracked(), 2);
            }
            other => panic!("Expected H3Checkpoint runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_h3_empty_latent_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::H3EmptyLatentAv,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::H3EmptyLatentAv(H3EmptyLatentAvStateData {
                width: 768,
                height: 1344,
                duration_seconds: 10.0,
                aspect_idx: 2,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, aspect_idx } => {
                assert_eq!(width.get_untracked(), 768);
                assert_eq!(height.get_untracked(), 1344);
                assert!((duration_seconds.get_untracked() - 10.0).abs() < 1e-6);
                assert_eq!(aspect_idx.get_untracked(), 2);
            }
            other => panic!("Expected H3EmptyLatentAv runtime, got {other:?}"),
        }
    }

    /// Состояние H3 Keyframe раньше не сериализовалось вовсе: шаблон терял
    /// слот «последний кадр», а агент не мог подставить картинку.
    #[test]
    fn roundtrip_h3_keyframe_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::H3Keyframe,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::H3Keyframe(H3KeyframeStateData {
                image_path: Some("/tmp/last.png".into()),
                frame_slot_idx: 1,
                resize_idx: 1,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd.clone()]));
        let node = first_node(&ctx);
        assert_eq!(runtime_to_state(&node.runtime.lock().unwrap()), nd.state);
    }

    /// Порядок и флаг дорожки переживают круг; тип берётся по расширению, а
    /// файл с незнакомым расширением не теряется молча — нода показывает ошибку.
    #[test]
    fn roundtrip_h3_references_state() {
        use crate::pages::node_editor::nodes::minimax_h3::references::labels_of;
        let item = |path: &str, use_audio: bool| H3ReferenceItemData { path: path.into(), use_audio };
        let state = H3ReferencesStateData {
            items: vec![
                item("/nonexistent/dance.mp4", false),
                item("/nonexistent/hero.png", true),
                item("/nonexistent/voice.wav", true),
            ],
            image_size_idx: 1,
        };
        let nd = NodeData {
            id: 1,
            kind: NodeKind::H3References,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::H3References(state.clone())),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        assert_eq!(runtime_to_state(&rt), Some(NodeStateData::H3References(state)));
        let NodeRuntime::H3References { items, error, .. } = &*rt else {
            panic!("Expected H3References runtime, got {rt:?}");
        };
        assert_eq!(labels_of(&items.get_untracked()), ["<Video 1>", "<Picture 1>", "<Audio 1>"]);
        assert!(error.get_untracked().is_none());
        drop(rt);

        let bad = NodeData {
            id: 1,
            kind: NodeKind::H3References,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::H3References(H3ReferencesStateData {
                items: vec![item("/nonexistent/notes.txt", true)],
                image_size_idx: 0,
            })),
        };
        let ctx = roundtrip(&make_template(vec![bad]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        let NodeRuntime::H3References { items, error, .. } = &*rt else { panic!() };
        assert!(items.get_untracked().is_empty());
        assert!(error.get_untracked().is_some());
    }

    #[test]
    fn roundtrip_markdown_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::MarkdownView,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::MarkdownView(MarkdownViewStateData {
                content: "# Hello\n\nbody".into(),
                width: 480.0,
                height: 260.0,
                edit_mode: true,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::MarkdownView {
                content,
                edit_mode,
                size,
                ..
            } => {
                assert_eq!(content.get_untracked(), "# Hello\n\nbody");
                assert!(edit_mode.get_untracked());
                let s = size.get_untracked();
                assert!((s.width - 480.0).abs() < 1e-3);
                assert!((s.height - 260.0).abs() < 1e-3);
            }
            other => panic!("Expected MarkdownView runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_audio_file_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AudioFile,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AudioFile(AudioFileStateData {
                loaded_path: Some("/tmp/synthos_roundtrip_does_not_exist.wav".into()),
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AudioFile { loaded_path, .. } => {
                assert_eq!(
                    loaded_path.get_untracked(),
                    Some(PathBuf::from("/tmp/synthos_roundtrip_does_not_exist.wav"))
                );
            }
            other => panic!("Expected AudioFile runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_filter_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::Filter,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::Filter(FilterStateData {
                mode: FilterMode::HighPass,
                cutoff_hz: 2_500.0,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::Filter { mode, cutoff_hz, .. } => {
                assert_eq!(mode.get_untracked(), FilterMode::HighPass);
                assert!((cutoff_hz.get_untracked() - 2_500.0).abs() < 1e-3);
            }
            other => panic!("Expected Filter runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_equalizer_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::Equalizer10,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::Equalizer(EqualizerStateData {
                gains_db: vec![1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 9.0, -10.0],
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::Equalizer { gains_db, n_bands, .. } => {
                assert_eq!(*n_bands, 10);
                for (i, expected) in [1.0_f32, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 9.0, -10.0]
                    .iter()
                    .enumerate()
                {
                    assert!(
                        (gains_db[i].get_untracked() - expected).abs() < 1e-3,
                        "band {i}"
                    );
                }
            }
            other => panic!("Expected Equalizer runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_mixer_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::Mixer,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::Mixer(MixerStateData {
                n_inputs: 5,
                gains_db: vec![
                    0.5, 1.0, 1.5, 2.0, 2.5, // active 5
                    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // inactive 11
                ],
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::Mixer { n_inputs, gains_db, .. } => {
                assert_eq!(n_inputs.get_untracked(), 5);
                for (i, expected) in [0.5_f32, 1.0, 1.5, 2.0, 2.5].iter().enumerate() {
                    assert!(
                        (gains_db[i].get_untracked() - expected).abs() < 1e-3,
                        "ch {i}"
                    );
                }
            }
            other => panic!("Expected Mixer runtime, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_apply_roundtrip_preserves_state() {
        // Создаём ctx с ASR-нодой, выставляем runtime → snapshot → apply →
        // проверяем что во втором ctx значения те же.
        let ctx_src = NodeEditorCtx::new();
        ctx_src.nodes.set(Vec::new());
        ctx_src.connections.set(Vec::new());
        let id = ctx_src.add_node(NodeKind::AsrGigaam, Point::new(10.0, 20.0));
        {
            let nodes = ctx_src.nodes.get_untracked();
            let n = nodes.iter().find(|n| n.id == id).unwrap();
            let rt = n.runtime.lock().unwrap();
            if let NodeRuntime::AsrGigaam {
                output_text,
                ..
            } = &*rt
            {
                output_text.set("проверка".into());
            } else {
                panic!("expected AsrGigaam");
            }
        }

        let (nodes_data, conns_data, viewport) = super::snapshot(&ctx_src);
        let t = Template {
            id: String::new(),
            builtin: false,
            name: "snap".into(),
            description: String::new(),
            kind: TemplateKind::Full,
            nodes: nodes_data,
            connections: conns_data,
            viewport,
        };

        let ctx_dst = roundtrip(&t);
        let node = first_node(&ctx_dst);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AsrGigaam {
                output_text,
                ..
            } => {
                assert_eq!(output_text.get_untracked(), "проверка");
            }
            other => panic!("Expected AsrGigaam runtime, got {other:?}"),
        }
    }

    /// `models_dir: None` в шаблоне не затирает дефолт runtime'а (каталог
    /// моделей приложения) — иначе нода из builtin-шаблона приезжает пустой
    /// и Generate падает «укажите каталог моделей».
    #[test]
    fn acestep_checkpoint_none_dir_keeps_runtime_default() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AceStepCheckpoint,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
                models_dir: None,
                device_idx: 1,
                ..Default::default()
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AceStepCheckpoint { models_dir, .. } => {
                assert_eq!(
                    models_dir.get_untracked(),
                    Some(crate::pages::node_editor::nodes::acestep::app_models_dir()),
                    "None в шаблоне должен оставить дефолтный каталог моделей"
                );
            }
            other => panic!("Expected AceStepCheckpoint runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_acestep_vae_encode_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AceStepVaeEncode,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AceStepVaeEncode(AceStepVaeStateData {
                device_idx: 1,
                storage_idx: 0,
                compute_idx: 0,
                chunk_seconds: 24.0,
                overlap_seconds: 1.0,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AceStepVaeEncode {
                chunk_seconds,
                overlap_seconds,
                ..
            } => {
                assert!((chunk_seconds.get_untracked() - 24.0).abs() < 1e-3);
                assert!((overlap_seconds.get_untracked() - 1.0).abs() < 1e-3);
            }
            other => panic!("Expected AceStepVaeEncode runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_acestep_checkpoint_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AceStepCheckpoint,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
                models_dir: Some("/data/syn_models".into()),
                lm_path: None,
                text_encoder_path: Some("/data/te.syn".into()),
                dit_path: None,
                vae_path: None,
                device_idx: 1,
                quant_dit_idx: 1,
                quant_enc_idx: 2,
                compute_idx: 0,
                resident: false,
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AceStepCheckpoint {
                models_dir,
                text_encoder_path,
                quant_dit_idx,
                quant_enc_idx,
                ..
            } => {
                assert_eq!(
                    models_dir.get_untracked(),
                    Some(std::path::PathBuf::from("/data/syn_models"))
                );
                assert_eq!(
                    text_encoder_path.get_untracked(),
                    Some(std::path::PathBuf::from("/data/te.syn"))
                );
                assert_eq!(quant_dit_idx.get_untracked(), 1);
                assert_eq!(quant_enc_idx.get_untracked(), 2);
            }
            other => panic!("Expected AceStepCheckpoint runtime, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_acestep_generate_state() {
        let nd = NodeData {
            id: 1,
            kind: NodeKind::AceStepGenerate,
            pos: PointData { x: 0.0, y: 0.0 },
            fields: Default::default(),
            style: Default::default(),
            enabled: true,
            state: Some(NodeStateData::AceStepGenerate(AceStepGenerateStateData {
                mode_idx: 1,
                preset: crate::templates::model::SamplerPresetData::Turbo,
                duration_seconds: 30.0,
                infer_steps: 8,
                cfg_scale: 1.0,
                flow_match_shift: 3.0,
                seed: 777,
                temperature: 0.7,
                top_p: 0.95,
                top_k: 40,
                min_p: 0.05,
                lm_cfg_scale: 2.0,
                use_cot: true,
                use_ar: false,
                bpm: 128,
                keyscale_idx: 5,
                timesig_idx: 2,
                norm_mode: PostNormMode::Rms,
                retake_variance: 0.3,
                retake_seed: 9,
                repaint_start_sec: 2.0,
                repaint_end_sec: 8.0,
                repaint_strength: 0.5,
                edit_n_min: 0.1,
                edit_n_max: 0.9,
                track_idx: 2,
                ..Default::default()
            })),
        };
        let ctx = roundtrip(&make_template(vec![nd]));
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        match &*rt {
            NodeRuntime::AceStepGenerate {
                track_idx,
                mode_idx,
                preset,
                duration_seconds,
                infer_steps,
                use_cot,
                use_ar,
                bpm,
                keyscale_idx,
                timesig_idx,
                norm_mode,
                retake_variance,
                repaint_end_sec,
                edit_n_max,
                ..
            } => {
                assert_eq!(mode_idx.get_untracked(), 1);
                assert_eq!(preset.get_untracked(), crate::pages::node_editor::types::SamplerPreset::Turbo);
                assert!((duration_seconds.get_untracked() - 30.0).abs() < 1e-3);
                assert_eq!(infer_steps.get_untracked(), 8);
                assert!(use_cot.get_untracked());
                assert!(!use_ar.get_untracked());
                assert_eq!(bpm.get_untracked(), 128);
                assert_eq!(keyscale_idx.get_untracked(), 5);
                assert_eq!(timesig_idx.get_untracked(), 2);
                assert_eq!(norm_mode.get_untracked(), PostNormMode::Rms);
                assert!((retake_variance.get_untracked() - 0.3).abs() < 1e-3);
                assert!((repaint_end_sec.get_untracked() - 8.0).abs() < 1e-3);
                assert!((edit_n_max.get_untracked() - 0.9).abs() < 1e-3);
                assert_eq!(track_idx.get_untracked(), 2);
            }
            other => panic!("Expected AceStepGenerate runtime, got {other:?}"),
        }
    }

    /// Граф до pkgrel 219 (без `lm_defaults_v2`) с парой 50/1.5 от старой
    /// ArLm-ноды грузится с 0/2.0; с маркером 50/1.5 остаются как выбраны.
    #[test]
    fn acestep_generate_legacy_lm_defaults_migrate_once() {
        let load_lm = |state: AceStepGenerateStateData| -> (u32, f32) {
            let nd = NodeData {
                id: 1,
                kind: NodeKind::AceStepGenerate,
                pos: PointData { x: 0.0, y: 0.0 },
                fields: Default::default(),
                style: Default::default(),
                enabled: true,
                state: Some(NodeStateData::AceStepGenerate(state)),
            };
            let ctx = roundtrip(&make_template(vec![nd]));
            let node = first_node(&ctx);
            let rt = node.runtime.lock().unwrap();
            match &*rt {
                NodeRuntime::AceStepGenerate { top_k, lm_cfg_scale, .. } => {
                    (top_k.get_untracked(), lm_cfg_scale.get_untracked())
                }
                other => panic!("Expected AceStepGenerate runtime, got {other:?}"),
            }
        };
        let legacy: AceStepGenerateStateData =
            serde_json::from_value(serde_json::json!({ "top_k": 50, "lm_cfg_scale": 1.5 })).unwrap();
        assert!(!legacy.lm_defaults_v2);
        assert_eq!(load_lm(legacy), (0, 2.0));
        let legacy_custom: AceStepGenerateStateData =
            serde_json::from_value(serde_json::json!({ "top_k": 40, "lm_cfg_scale": 1.5 })).unwrap();
        assert_eq!(load_lm(legacy_custom), (40, 1.5));
        let chosen = AceStepGenerateStateData { top_k: 50, lm_cfg_scale: 1.5, ..Default::default() };
        assert_eq!(load_lm(chosen), (50, 1.5));
    }

    #[test]
    fn legacy_template_without_state_loads_with_defaults() {
        // Шаблон, сохранённый старой версией без `state`: парсер должен
        // принимать (#[serde(default)]) и нода получает дефолтный runtime.
        let json = r#"{
          "name": "legacy",
          "description": "",
          "kind": "full",
          "nodes": [
            {
              "id": 1,
              "kind": "gain",
              "pos": { "x": 0.0, "y": 0.0 },
              "fields": {},
              "style": { "shadow": true }
            }
          ],
          "connections": [],
          "viewport": null
        }"#;
        let t: Template = serde_json::from_str(json).expect("legacy parse");
        assert!(t.nodes[0].state.is_none());
        let ctx = roundtrip(&t);
        let node = first_node(&ctx);
        let rt = node.runtime.lock().unwrap();
        if let NodeRuntime::Gain { gain_db, .. } = &*rt {
            // Дефолт из `default_runtime` — 0.0 dB.
            assert!((gain_db.get_untracked() - 0.0).abs() < 1e-6);
        } else {
            panic!("expected Gain runtime");
        }
    }
}
