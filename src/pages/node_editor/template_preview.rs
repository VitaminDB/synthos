//! Мини-canvas-генератор preview-картинки шаблона.
//!
//! Каждая нода рисуется как маленький rect с цветом по `NodeKind`,
//! связи — как cubic-Bezier-кривые. Bbox масштабируется в фиксированный
//! размер карточки (`PREVIEW_W × PREVIEW_H`) с margin.
//!
//! Без сохранения PNG: рисуем напрямую при каждом render-проходе. Не-
//! `animated(false)` чтобы Canvas не запрашивал tick'и зря.

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::Canvas;

use crate::templates::{ConnData, NodeData, PointData, Template};
use super::types::NodeKind;

pub const PREVIEW_W: f32 = 220.0;
pub const PREVIEW_H: f32 = 88.0;
const MARGIN: f32 = 6.0;
const NODE_W: f32 = 16.0;
const NODE_H: f32 = 8.0;

/// Сборка widget'а preview для конкретного шаблона.
pub fn view(t: &Template) -> impl Widget {
    let nodes = t.nodes.clone();
    let conns = t.connections.clone();
    let bbox = t.bbox();
    Canvas::new(move |ctx, _t| {
        if nodes.is_empty() {
            // Empty preview: пунктирный «слот» по центру через одну
            // прямоугольную обводку. Никаких надписей — карточка-empty
            // и так подписана текстом снаружи.
            ctx.set_color(Color::new(1.0, 1.0, 1.0, 0.10));
            ctx.set_stroke_width(1.0);
            ctx.draw_rect(MARGIN, MARGIN, PREVIEW_W - 2.0 * MARGIN, PREVIEW_H - 2.0 * MARGIN);
            return;
        }

        // Подгоняем bbox под доступную область с сохранением aspect: scale =
        // min(avail_w / bw, avail_h / bh). NODE_W/H учитываем чтобы крайние
        // ноды не вылезали за рамку.
        let (min, max) = bbox.unwrap_or((PointData { x: 0.0, y: 0.0 }, PointData { x: 1.0, y: 1.0 }));
        let bw = (max.x - min.x).max(1.0);
        let bh = (max.y - min.y).max(1.0);
        let avail_w = PREVIEW_W - 2.0 * MARGIN - NODE_W;
        let avail_h = PREVIEW_H - 2.0 * MARGIN - NODE_H;
        let scale = (avail_w / bw).min(avail_h / bh);
        // Центрируем: смещаем на остаток.
        let used_w = bw * scale + NODE_W;
        let used_h = bh * scale + NODE_H;
        let off_x = MARGIN + (PREVIEW_W - 2.0 * MARGIN - used_w) * 0.5;
        let off_y = MARGIN + (PREVIEW_H - 2.0 * MARGIN - used_h) * 0.5;
        let project = |p: PointData| -> (f32, f32) {
            (off_x + (p.x - min.x) * scale, off_y + (p.y - min.y) * scale)
        };

        // Связи (под нодами, чтобы ноды визуально перекрывали концы).
        ctx.set_color(Color::new(0.45, 0.62, 0.95, 0.55));
        ctx.set_stroke_width(1.0);
        for c in &conns {
            let Some((a, b)) = endpoints(&nodes, c) else { continue };
            let (ax, ay) = project(a);
            let (bx, by) = project(b);
            // Output-порт — у правого края мини-ноды; input — у левого.
            let p0 = (ax + NODE_W, ay + NODE_H * 0.5);
            let p3 = (bx, by + NODE_H * 0.5);
            let dx = (p3.0 - p0.0).abs().max(8.0) * 0.45;
            let p1 = (p0.0 + dx, p0.1);
            let p2 = (p3.0 - dx, p3.1);
            draw_cubic_bezier(ctx, p0, p1, p2, p3, 16);
        }

        // Ноды.
        for n in &nodes {
            let (x, y) = project(n.pos);
            let c = node_color(n.kind);
            ctx.set_color(c);
            ctx.fill_rect(x, y, NODE_W, NODE_H);
        }
    })
    .size(PREVIEW_W, PREVIEW_H)
    .animated(false)
}

fn endpoints(nodes: &[NodeData], c: &ConnData) -> Option<(PointData, PointData)> {
    let from = nodes.iter().find(|n| n.id == c.from_node)?;
    let to = nodes.iter().find(|n| n.id == c.to_node)?;
    Some((from.pos, to.pos))
}

fn node_color(kind: NodeKind) -> Color {
    match kind {
        NodeKind::H3Checkpoint => Color::new(0.55, 0.42, 0.92, 1.0),
        NodeKind::H3TextEncoder => Color::new(0.48, 0.55, 0.95, 1.0),
        NodeKind::H3EmptyLatentAv => Color::new(0.40, 0.62, 0.88, 1.0),
        NodeKind::H3Keyframe => Color::new(0.92, 0.58, 0.36, 1.0),
        NodeKind::H3References => Color::new(0.92, 0.58, 0.36, 1.0),
        NodeKind::H3Sampler => Color::new(0.86, 0.36, 0.62, 1.0),
        NodeKind::H3VaeDecode => Color::new(0.28, 0.76, 0.62, 1.0),
        NodeKind::H3AudioDecode => Color::new(0.34, 0.72, 0.86, 1.0),
        NodeKind::H3VideoSave => Color::new(0.18, 0.70, 0.52, 1.0),
        // FLUX — тёплая оранжевая гамма, картинки — в тон проводу `image`.
        NodeKind::FluxCheckpoint
        | NodeKind::FluxTextEncoder
        | NodeKind::FluxEmptyLatent
        | NodeKind::FluxVaeEncode
        | NodeKind::FluxSampler
        | NodeKind::FluxVaeDecode => Color::new(0.96, 0.52, 0.22, 1.0),
        NodeKind::ImageLoad | NodeKind::ImageSave => Color::new(0.98, 0.45, 0.09, 1.0),
        NodeKind::Number => Color::new(0.40, 0.65, 0.95, 1.0),       // blue
        NodeKind::Add => Color::new(0.93, 0.37, 0.28, 1.0),          // primary/red
        NodeKind::Output => Color::new(0.16, 0.74, 0.55, 1.0),       // emerald
        NodeKind::AudioFile => Color::new(0.96, 0.62, 0.28, 1.0),    // amber
        NodeKind::AudioPlayer => Color::new(0.32, 0.78, 0.74, 1.0),  // teal
        NodeKind::AudioRecorder => Color::new(0.92, 0.36, 0.40, 1.0),// rose
        NodeKind::Gain => Color::new(0.86, 0.78, 0.32, 1.0),         // gold
        NodeKind::Filter => Color::new(0.42, 0.66, 0.86, 1.0),       // sky
        NodeKind::Reverb => Color::new(0.62, 0.50, 0.92, 1.0),       // lavender
        NodeKind::SaveToFile => Color::new(0.30, 0.70, 0.45, 1.0),   // green
        NodeKind::Equalizer6
        | NodeKind::Equalizer10
        | NodeKind::Equalizer20
        | NodeKind::Equalizer30 => Color::new(0.95, 0.55, 0.78, 1.0), // pink
        NodeKind::Mixer => Color::new(0.55, 0.85, 0.62, 1.0),         // mint
        NodeKind::MarkdownView => Color::new(0.72, 0.74, 0.82, 1.0),  // slate (декоративная)
        NodeKind::AsrGigaam => Color::new(0.78, 0.46, 0.92, 1.0),      // orchid (ASR/нейро)
        NodeKind::TextView => Color::new(0.92, 0.70, 0.20, 1.0),       // amber (текст)
        NodeKind::OmniVoice => Color::new(0.55, 0.42, 0.92, 1.0),      // indigo (TTS/нейро)
        NodeKind::VoxCpm2 => Color::new(0.62, 0.50, 0.92, 1.0),        // lavender (TTS/нейро)
        NodeKind::VibeVoice => Color::new(0.44, 0.58, 0.96, 1.0),      // cornflower (диалоги/нейро)
        NodeKind::Llm => Color::new(0.40, 0.80, 0.66, 1.0),            // teal-green (LLM/нейро)
        NodeKind::SynCheckpoint => Color::new(0.68, 0.56, 0.90, 1.0),  // violet (источник модели)
        NodeKind::SortformerDiarizer => Color::new(0.31, 0.78, 0.94, 1.0), // cyan (диаризация)
        // ACE-Step семейство — единая палитра в фиолетово-розовой гамме,
        // так визуально граф «AceStep — Text2Music» легко выделить.
        NodeKind::AceStepCheckpoint
        | NodeKind::AceStepGenerate
        | NodeKind::AceStepVaeEncode => Color::new(0.85, 0.45, 0.95, 1.0),
        NodeKind::FfmpegPlayer => Color::new(0.93, 0.27, 0.60, 1.0),
        // LTX-2.3 семейство — единая палитра в сине-бирюзовой гамме,
        // граф «LTX — Text2Video» визуально отличим от ACE-Step.
        NodeKind::LtxCheckpoint
        | NodeKind::LtxTextEncoder
        | NodeKind::LtxNagPrompt
        | NodeKind::LtxSamplerStage1
        | NodeKind::LtxUpscale
        | NodeKind::LtxSamplerStage2
        | NodeKind::LtxVaeDecode
        | NodeKind::LtxAudioDecode
        | NodeKind::LtxVideoSave
        | NodeKind::LtxImage
        | NodeKind::LtxVideoInput
        | NodeKind::LtxRetake
        | NodeKind::LtxIcLora
        | NodeKind::LtxAudioInput
        | NodeKind::LtxLipdub
        | NodeKind::LtxA2V => Color::new(0.30, 0.68, 0.92, 1.0),
    }
}

/// 16-сегментный cubic-Bezier через draw_polyline. Достаточно для preview;
/// для основного wires.rs используется аналогичный самописный sampler.
fn draw_cubic_bezier(
    ctx: &mut syngui::core::canvas::CanvasContext,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    segments: usize,
) {
    let mut points: Vec<(f32, f32)> = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let mt = 1.0 - t;
        let b0 = mt * mt * mt;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let b3 = t * t * t;
        let x = b0 * p0.0 + b1 * p1.0 + b2 * p2.0 + b3 * p3.0;
        let y = b0 * p0.1 + b1 * p1.1 + b2 * p2.1 + b3 * p3.1;
        points.push((x, y));
    }
    ctx.draw_polyline(&points);
}
