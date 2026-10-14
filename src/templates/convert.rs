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

    AudioFileStateData,

    AudioPlayerStateData,

    AudioRecorderStateData,

    ConnData,

    EqualizerStateData,

    FfmpegPlayerStateData,

    FieldValueData,

    FilterStateData,

    GainStateData,

    ImageLoadStateData,

    ImageSaveStateData,

    MarkdownViewStateData,

    MixerStateData,

    NodeData,

    NodeStateData,

    NodeStyleData,

    PointData,

    ReverbStateData,

    SaveToFileStateData,

    Template,

    TextViewStateData,

    ViewportData,

};
use crate::pages::node_editor::registry::{default_fields, meta};
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
    let map = node.fields.lock().unwrap_or_else(|e| e.into_inner());
    for (key, value) in map.iter() {
        fields.insert((*key).to_string(), field_value_to_data(value));
    }
    drop(map);
    let style = node.style.get_untracked();
    let state = {
        let rt = node.runtime.lock().unwrap_or_else(|e| e.into_inner());
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

// ─────────────────────────────────────────────────────────────────────────────
// NodeRuntime ↔ NodeStateData
// ─────────────────────────────────────────────────────────────────────────────

/// Снимок per-kind state из runtime'а ноды. `None` для нод без
/// пользовательского state (`Number`, `Add`, `Output`, `Demo`).
/// pub — используется агентским инструментом `pipelines` (схема state
/// по default_runtime и точечный set_state).
pub fn runtime_to_state(rt: &NodeRuntime) -> Option<NodeStateData> {
    match rt {
        NodeRuntime::None | NodeRuntime::Flow(_) => None,
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
        // ACE-Step семейство. `loaded_cfg`/`output_buf`/`output_version`/
        // `running`/`error`/`loaded_name`/`progress_pct` НЕ сериализуем —
        // это операционное состояние и тяжёлые тензоры, восстанавливаются
        // лениво на следующий Play.
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
        (NodeRuntime::ImageLoad { path, .. }, NodeStateData::ImageLoad(data)) => {
            path.set(data.image_path.as_ref().map(PathBuf::from));
        }
        (NodeRuntime::ImageSave { path, .. }, NodeStateData::ImageSave(data)) => {
            path.set(data.path.as_ref().map(PathBuf::from));
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
        let map = arc.lock().unwrap_or_else(|e| e.into_inner());
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


// ─────────────────────────────────────────────────────────────────────────────
// Roundtrip-тесты сохранения per-kind state ноды.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::node_editor::types::FilterMode;
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
        // Создаём ctx с текстовой нодой, выставляем runtime → snapshot → apply →
        // проверяем что во втором ctx значения те же.
        let ctx_src = NodeEditorCtx::new();
        ctx_src.nodes.set(Vec::new());
        ctx_src.connections.set(Vec::new());
        let id = ctx_src.add_node(NodeKind::TextView, Point::new(10.0, 20.0));
        {
            let nodes = ctx_src.nodes.get_untracked();
            let n = nodes.iter().find(|n| n.id == id).unwrap();
            let rt = n.runtime.lock().unwrap();
            if let NodeRuntime::TextView { output_text, .. } = &*rt {
                output_text.set("проверка".into());
            } else {
                panic!("expected TextView");
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
            NodeRuntime::TextView { output_text, .. } => assert_eq!(output_text.get_untracked(), "проверка"),
            other => panic!("Expected TextView runtime, got {other:?}"),
        }
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
