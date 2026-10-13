//! Ноды ядра synaptix-flow — общий нодовый движок synshell (ассистент,
//! рецепты, Synthos). Генерация идёт через службу synaptixd (LLM, картинки,
//! видео, музыка, речь, расшифровка), монтаж — ffmpeg (склейка, наложение
//! звука, кадр, обрезка, картинка в ролик), данные — текст, файл, шаблон,
//! строки, список.
//!
//! Метаданные (порты, поля) строятся по схемам ядра; исполнение — тот же
//! `synaptix_flow::run_node`, что у рецептов ассистента: список на входе —
//! пакет, нода выполняется для каждого элемента. Значения между нодами ядра
//! передаются как есть ([`DataBlob::Flow`]); с родными нодами Synthos —
//! через файлы: картинка на выходе — [`ImageData`], звук — `AudioBuffer`,
//! на входе картинка и звук пишутся во временные файлы.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value as Json;
use synaptix_flow::{self as sf, FieldType as FT, PortType as PT, Val};
use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::input::dropdown::DropdownItem;
use syngui::widgets::input::MultilineTextEdit;
use syngui::widgets::{Column, Reactive, Row, ToolButton};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::registry::{NodeCategory, NodeKindMeta};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    DataBlob, FieldSchema, FieldType, FieldValue, ImageData, NodeInstance, NodeKind, NodeRuntime, PortKind,
    PortSchema, PortValue, PortsLayout, PortsSpec,
};
use super::acestep::field_row;
use super::{log_worker_done, log_worker_start};
use crate::icons::{
    MI_ARTICLE, MI_AUDIOTRACK, MI_EDIT_NOTE, MI_FOLDER_OPEN, MI_IMAGE_ICON, MI_LANGUAGE, MI_LAYERS, MI_LIBRARY_MUSIC,
    MI_MERGE_TYPE, MI_MIC, MI_MOVIE, MI_PSYCHOLOGY, MI_RECORD_VOICE_OVER, MI_TUNE,
};

/// Вид ноды Synthos ↔ вид ноды ядра.
const KINDS: &[(NodeKind, &str, &str, &str)] = &[
    (NodeKind::FlowText, "text", "Flow Text", MI_ARTICLE),
    (NodeKind::FlowFile, "file", "Flow File", MI_FOLDER_OPEN),
    (NodeKind::FlowFormat, "format", "Flow Template", MI_EDIT_NOTE),
    (NodeKind::FlowLines, "lines", "Flow Lines", MI_TUNE),
    (NodeKind::FlowList, "list", "Flow List", MI_LAYERS),
    (NodeKind::FlowLlm, "llm", "Flow LLM", MI_PSYCHOLOGY),
    (NodeKind::FlowImage, "image", "Flow Image", MI_IMAGE_ICON),
    (NodeKind::FlowVideo, "video", "Flow Video", MI_MOVIE),
    (NodeKind::FlowMusic, "music", "Flow Music", MI_LIBRARY_MUSIC),
    (NodeKind::FlowSpeak, "speak", "Flow Speech", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowTranscribe, "transcribe", "Flow Transcribe", MI_MIC),
    (NodeKind::FlowConcat, "concat", "Flow Concat", MI_MERGE_TYPE),
    (NodeKind::FlowMux, "mux", "Flow Mux Audio", MI_AUDIOTRACK),
    (NodeKind::FlowFrame, "frame", "Flow Frame", MI_IMAGE_ICON),
    (NodeKind::FlowTrim, "trim", "Flow Trim", MI_MOVIE),
    (NodeKind::FlowStill, "still", "Flow Still Video", MI_LANGUAGE),
];

impl NodeKind {
    /// Вид ноды ядра synaptix-flow, если это нода ядра.
    pub fn flow_kind(self) -> Option<&'static str> {
        KINDS.iter().find(|k| k.0 == self).map(|k| k.1)
    }

    pub fn from_flow_kind(kind: &str) -> Option<NodeKind> {
        KINDS.iter().find(|k| k.1 == kind).map(|k| k.0)
    }
}

fn spec(kind: NodeKind) -> Option<&'static sf::NodeSpec> {
    kind.flow_kind().and_then(sf::ops::spec)
}

fn port_kind(t: PT) -> PortKind {
    match t {
        PT::Text => PortKind::Text,
        PT::Number => PortKind::Data,
        PT::Image => PortKind::Image,
        PT::Audio => PortKind::Audio,
        PT::Video | PT::Media => PortKind::Video,
        PT::List | PT::Any => PortKind::Data,
    }
}

fn field_type(t: FT) -> FieldType {
    match t {
        FT::Text | FT::LongText | FT::File => FieldType::Text,
        FT::Number => FieldType::Float,
        FT::Bool => FieldType::Bool,
        FT::Choice(o) => FieldType::Choice(o),
    }
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

/// Метаданные всех нод ядра (строятся один раз).
pub fn metas() -> Vec<NodeKindMeta> {
    KINDS
        .iter()
        .filter_map(|&(kind, fk, title, icon)| {
            let s = sf::ops::spec(fk)?;
            let ports = |p: &'static [sf::PortSpec]| leak(p.iter().map(|p| PortSchema { name: p.name, label: p.label, kind: port_kind(p.ty) }).collect());
            Some(NodeKindMeta {
                kind,
                icon,
                title,
                category: NodeCategory::Service,
                subcategory: None,
                inputs: PortsSpec::Static(ports(s.inputs)),
                outputs: PortsSpec::Static(ports(s.outputs)),
                fields: leak(s.fields.iter().map(|f| FieldSchema { name: f.name, label: f.label, ty: field_type(f.ty) }).collect()),
                body: Some(body),
                ports_layout: PortsLayout::Rows,
                port_row_extra: None,
                executor: &FLOW_EXEC,
                on_run: Some(on_run),
                busy_signal: Some(busy_signal),
            })
        })
        .collect()
}

/// Поля свежей ноды — с умолчаниями схемы ядра.
pub fn default_fields(kind: NodeKind) -> Arc<Mutex<HashMap<&'static str, FieldValue>>> {
    let mut map = HashMap::new();
    for f in spec(kind).map(|s| s.fields).unwrap_or(&[]) {
        let v = match f.ty {
            FT::Text | FT::LongText | FT::File => FieldValue::Text(use_signal(f.default.to_string())),
            FT::Number => FieldValue::Float(use_signal(f.default.parse::<f32>().unwrap_or(0.0))),
            FT::Bool => FieldValue::Bool(use_signal(f.default == "true")),
            FT::Choice(o) => FieldValue::Choice(use_signal(o.iter().position(|x| *x == f.default).unwrap_or(0))),
        };
        map.insert(f.name, v);
    }
    Arc::new(Mutex::new(map))
}

/// Состояние ноды ядра.
#[derive(Clone)]
pub struct FlowRuntime {
    pub running: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Что получилось (или «элемент k из n» во время пакета).
    pub status: RwSignal<String>,
    pub progress: RwSignal<f32>,
    pub cancel: Arc<AtomicBool>,
    /// Выходы последнего прогона — значения ядра.
    pub outputs: Arc<Mutex<sf::Outputs>>,
    /// Те же выходы для проводов Synthos.
    pub ports: Arc<Mutex<HashMap<String, PortValue>>>,
    pub version: RwSignal<u32>,
}

impl FlowRuntime {
    pub fn new() -> Self {
        Self {
            running: use_signal(false),
            error: use_signal(None),
            status: use_signal(String::new()),
            progress: use_signal(0.0),
            cancel: Arc::new(AtomicBool::new(false)),
            outputs: Arc::new(Mutex::new(BTreeMap::new())),
            ports: Arc::new(Mutex::new(HashMap::new())),
            version: use_signal(0),
        }
    }
}

fn runtime(node: &NodeInstance) -> Option<FlowRuntime> {
    match &*node.runtime.lock().ok()? {
        NodeRuntime::Flow(f) => Some(f.clone()),
        _ => None,
    }
}

pub struct FlowExec;
pub static FLOW_EXEC: FlowExec = FlowExec;

impl NodeExecutor for FlowExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let Some(rt) = runtime(ctx.node) else { return };
        if ctx.track {
            rt.version.get();
        }
        let ports = rt.ports.lock().map(|p| p.clone()).unwrap_or_default();
        let Some(s) = spec(ctx.node.kind) else { return };
        for p in s.outputs {
            ctx.write_output(p.name, ports.get(p.name).cloned().unwrap_or(PortValue::Empty));
        }
    }
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    runtime(node).map(|r| r.running)
}

/// Поля ноды → поля ядра (выбор — строкой варианта, 0 в числе — «не задано»).
fn fields_json(node: &NodeInstance) -> BTreeMap<String, Json> {
    let mut out = BTreeMap::new();
    let Some(s) = spec(node.kind) else { return out };
    let map = node.fields.lock().unwrap_or_else(|e| e.into_inner());
    for f in s.fields {
        let v = match (map.get(f.name), f.ty) {
            (Some(FieldValue::Text(sig)), _) => Json::String(sig.get_untracked()),
            (Some(FieldValue::Float(sig)), _) => serde_json::Number::from_f64(sig.get_untracked() as f64).map(Json::Number).unwrap_or(Json::Null),
            (Some(FieldValue::Bool(sig)), _) => Json::Bool(sig.get_untracked()),
            (Some(FieldValue::Choice(sig)), FT::Choice(o)) => Json::String(o.get(sig.get_untracked()).copied().unwrap_or("").to_string()),
            _ => continue,
        };
        out.insert(f.name.to_string(), v);
    }
    out
}

/// Каталог файлов прогонов нод ядра.
fn runs_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    base.join("synthos").join("flow")
}

/// Значение провода Synthos → значение ядра (картинка и звук — во временные файлы).
fn to_val(pv: &PortValue, dir: &Path, name: &str) -> Val {
    match pv {
        PortValue::Empty => Val::Empty,
        PortValue::Text(s) => Val::Text(s.clone()),
        PortValue::Float(f) => Val::Number(*f as f64),
        PortValue::Audio(buf) => {
            let p = dir.join(format!("{name}.wav"));
            match super::ltx::video_save::write_wav(&p, buf) {
                Ok(()) => Val::File(p),
                Err(_) => Val::Empty,
            }
        }
        PortValue::Data(blob) => match &**blob {
            DataBlob::Flow(v) => v.clone(),
            DataBlob::Image(img) => match &img.source {
                Some(src) => Val::File(src.clone()),
                None => {
                    let p = dir.join(format!("{name}.png"));
                    match super::image::write_image(img, &p) {
                        Ok(()) => Val::File(p),
                        Err(_) => Val::Empty,
                    }
                }
            },
            _ => Val::Empty,
        },
        _ => Val::Empty,
    }
}

/// Значение ядра → провод Synthos: одиночная картинка — `ImageData`, звук — `AudioBuffer`.
fn to_port(v: &Val, ty: PT) -> PortValue {
    match (ty, v) {
        (_, Val::Empty) => PortValue::Empty,
        (PT::Text, v) => PortValue::Text(v.text()),
        (PT::Number, v) => v.number().map(|n| PortValue::Float(n as f32)).unwrap_or(PortValue::Empty),
        (PT::Image, Val::File(p)) => match ImageData::load(p) {
            Ok(img) => PortValue::Data(Arc::new(DataBlob::Image(Arc::new(img)))),
            Err(_) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
        },
        (PT::Audio, Val::File(p)) => match super::decode::decode_file(p) {
            Ok(buf) => PortValue::Audio(Arc::new(buf)),
            Err(_) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
        },
        (_, v) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
    }
}

/// Входы ноды: с нод ядра — их значения как есть, с родных нод — через файлы.
fn gather(node: &NodeInstance, ctx: &NodeEditorCtx, dir: &Path) -> HashMap<String, Val> {
    let nodes = ctx.nodes.get_untracked();
    let values = ctx.values.get_untracked();
    let mut out = HashMap::new();
    for c in ctx.connections.get_untracked().iter().filter(|c| c.to_node == node.id) {
        let src = nodes.iter().find(|n| n.id == c.from_node);
        let from_flow = src.and_then(runtime).and_then(|r| r.outputs.lock().ok().and_then(|o| o.get(c.from_port).cloned()));
        let v = match from_flow {
            Some(v) => v,
            None => values.get(&(c.from_node, c.from_port)).map(|pv| to_val(pv, dir, c.to_port)).unwrap_or_default(),
        };
        out.insert(c.to_port.to_string(), v);
    }
    out
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(rt) = runtime(node) else { return };
    let Some(fk) = node.kind.flow_kind() else { return };
    if rt.running.get_untracked() {
        return;
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = runs_dir().join(format!("{fk}-{}-{stamp}", node.id.0));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        rt.error.set(Some(format!("{}: {e}", dir.display())));
        return;
    }
    let inputs = gather(node, ctx, &dir);
    let mut fnode = sf::FlowNode::new(node.id.0, fk);
    fnode.fields = fields_json(node);
    rt.cancel.store(false, Ordering::SeqCst);
    rt.running.set(true);
    rt.error.set(None);
    rt.progress.set(0.0);
    rt.status.set(String::new());
    let _ = std::thread::Builder::new().name(format!("synthos-flow-{fk}")).spawn(move || {
        let started = log_worker_start("flow", fk);
        let cancel = sf::Cancel::new();
        // Флаг Stop из run_controls → отмена ядра (рвёт запрос к службе).
        let done = Arc::new(AtomicBool::new(false));
        {
            let (flag, done, cancel) = (rt.cancel.clone(), done.clone(), cancel.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if flag.load(Ordering::SeqCst) {
                        cancel.cancel();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            });
        }
        let (progress, status) = (rt.progress, rt.status);
        let res = sf::run_node(&fnode, inputs, &dir, &cancel, &mut |e| match e {
            sf::Event::Progress { step, total, .. } if total > 0 => progress.set(step as f32 / total as f32),
            sf::Event::Started { item: Some((k, n)), .. } => {
                status.set(format!("{k} / {n}"));
                progress.set(0.0);
            }
            _ => {}
        });
        done.store(true, Ordering::SeqCst);
        log_worker_done("flow", started, &res);
        match res {
            Ok((outs, label)) => {
                let spec = sf::ops::spec(fk).unwrap();
                let ports: HashMap<String, PortValue> = spec.outputs.iter().map(|p| (p.name.to_string(), outs.get(p.name).map(|v| to_port(v, p.ty)).unwrap_or_default())).collect();
                let summary = summary(&outs, &label);
                if let Ok(mut o) = rt.outputs.lock() {
                    *o = outs;
                }
                if let Ok(mut p) = rt.ports.lock() {
                    *p = ports;
                }
                rt.status.set(summary);
                rt.version.update(|v| *v = v.wrapping_add(1));
            }
            Err(e) => rt.error.set(Some(e)),
        }
        rt.running.set(false);
    });
}

fn summary(outs: &sf::Outputs, label: &str) -> String {
    let files: usize = outs.values().map(|v| v.files().len()).sum();
    let what = if files > 0 { format!("{files} файл(ов)") } else { String::new() };
    [what, label.to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

// ── Тело карточки ─────────────────────────────────────────────────────────

fn field_control(f: &'static sf::FieldSpec, value: FieldValue) -> Box<dyn Widget> {
    match (f.ty, value) {
        (FT::LongText, FieldValue::Text(sig)) => Box::new(
            MultilineTextEdit::new()
                .text(sig.get_untracked())
                .soft_wrap(true)
                .auto_height(true)
                .on_change(move |s| sig.set(s.to_string()))
                .class("node-input-text flow-node-longtext"),
        ),
        (_, FieldValue::Text(sig)) => Box::new(
            TextField::new()
                .text(sig.get_untracked())
                .placeholder(if f.ty == FT::File { "путь к файлу" } else { "" })
                .on_change(move |s| sig.set(s.to_string()))
                .class("node-input-text"),
        ),
        (_, FieldValue::Float(sig)) => Box::new(
            SpinBox::new()
                .min(0.0)
                .max(4_294_967_295.0)
                .step(1.0)
                .decimal_places(if f.name == "volume" || f.name == "strength" || f.name == "guidance" { 2 } else { 0 })
                .value(sig.get_untracked() as f64)
                .on_change(move |v| sig.set(v as f32))
                .width(120.0)
                .class("node-input-spinbox"),
        ),
        (_, FieldValue::Bool(sig)) => Box::new(Checkbox::checked(sig.get_untracked()).on_change(move |v| sig.set(v)).class("node-input-checkbox")),
        (FT::Choice(opts), FieldValue::Choice(sig)) => {
            let label = |o: &str| if o.is_empty() { "—".to_string() } else { o.to_string() };
            let items: Vec<DropdownItem> = opts.iter().map(|o| DropdownItem::simple(label(o))).collect();
            Box::new(
                Dropdown::with_items(items)
                    .selected(label(opts.get(sig.get_untracked()).copied().unwrap_or("")))
                    .on_change(move |val| {
                        if let Some(i) = opts.iter().position(|o| label(o) == val) {
                            sig.set(i);
                        }
                    })
                    .class("node-input-dropdown"),
            )
        }
        _ => Box::new(Text::new("?").class("node-card-field-error")),
    }
}

/// Результат: текст, превью картинки, файлы с кнопкой «открыть».
fn result_view(rt: &FlowRuntime, spec: &'static sf::NodeSpec) -> Vec<Box<dyn Widget>> {
    let mut out: Vec<Box<dyn Widget>> = Vec::new();
    let outputs = rt.outputs.lock().map(|o| o.clone()).unwrap_or_default();
    let ports = rt.ports.lock().map(|p| p.clone()).unwrap_or_default();
    for p in spec.outputs {
        let Some(v) = outputs.get(p.name) else { continue };
        if let Some(PortValue::Data(blob)) = ports.get(p.name) {
            if let DataBlob::Image(img) = &**blob {
                out.push(super::image::image_preview(Some(img.clone())));
            }
        }
        let files = v.files();
        if files.is_empty() {
            let t = v.text();
            if !t.trim().is_empty() {
                let short: String = t.chars().take(600).collect();
                out.push(Box::new(Text::new(short).class("audio-node-meta flow-node-text")));
            }
        }
        for f in files.into_iter().take(8) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let open = f.clone();
            out.push(Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![
                        Box::new(
                            ToolButton::new(MI_FOLDER_OPEN)
                                .tooltip("Открыть")
                                .on_click(move || {
                                    let _ = crate::paths::open_with_system(&open);
                                })
                                .class("node-file-picker-btn"),
                        ) as Box<dyn Widget>,
                        Box::new(Text::new(name).class("audio-node-meta")),
                    ]),
            ));
        }
    }
    out
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let (Some(rt), Some(spec)) = (runtime(node), spec(node.kind)) else {
        return Box::new(Column::new());
    };
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let map = node.fields.lock().map(|m| m.clone()).unwrap_or_default();
    for f in spec.fields {
        if let Some(v) = map.get(f.name).cloned() {
            col = col.children(vec![field_row(f.label, field_control(f, v))]);
        }
    }
    let (running, error, status, progress, version) = (rt.running, rt.error, rt.status, rt.progress, rt.version);
    col = col.children(vec![super::ltx::progress_row(running, progress)]);
    let rt2 = rt.clone();
    col = col.children(vec![Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        version.get();
        let mut v: Vec<Box<dyn Widget>> = Vec::new();
        if let Some(e) = error.get() {
            v.push(Box::new(Text::new(tr!("nodes.common.error", error = e)).class("audio-node-error")));
        }
        let s = status.get();
        if running.get() {
            v.push(Box::new(Text::new(if s.is_empty() { "выполняется…".to_string() } else { s }).class("audio-node-meta")));
            return v;
        }
        if !s.is_empty() {
            v.push(Box::new(Text::new(s).class("audio-node-meta")));
        }
        v.extend(result_view(&rt2, spec));
        v
    })) as Box<dyn Widget>]);
    Box::new(col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::node_editor::types::PortSide;
    use crate::templates::convert::{resolve_field_name, resolve_port_name};
    use crate::templates::model::Template;

    /// Шаблон в форме экспорта synaptix-graph: виды, поля и порты резолвятся.
    #[test]
    fn exported_template_resolves() {
        let t: Template = serde_json::from_value(serde_json::json!({
            "name": "t", "kind": "full",
            "nodes": [
                {"id": 1, "kind": "flow_file", "pos": {"x": 0, "y": 0}, "fields": {"path": {"type": "Text", "value": "/tmp/a.png"}}, "state": null},
                {"id": 2, "kind": "flow_video", "pos": {"x": 0, "y": 0}, "fields": {"seconds": {"type": "Float", "value": 3.0}, "control": {"type": "Choice", "value": 2}, "two_stage": {"type": "Bool", "value": true}}, "state": null},
                {"id": 3, "kind": "flow_mux", "pos": {"x": 0, "y": 0}, "fields": {}, "state": null}
            ],
            "connections": [
                {"from_node": 1, "from_port": "file", "to_node": 2, "to_port": "first_frame"},
                {"from_node": 2, "from_port": "video", "to_node": 3, "to_port": "video"}
            ]
        }))
        .unwrap();
        assert_eq!(t.nodes[1].kind, NodeKind::FlowVideo);
        for n in &t.nodes {
            for f in n.fields.keys() {
                assert!(resolve_field_name(n.kind, f).is_some(), "{:?}.{f}", n.kind);
            }
        }
        for c in &t.connections {
            assert!(resolve_port_name(t.kind_of(c.from_node), PortSide::Output, &c.from_port).is_some(), "{}", c.from_port);
            assert!(resolve_port_name(t.kind_of(c.to_node), PortSide::Input, &c.to_port).is_some(), "{}", c.to_port);
        }
        // Все виды ядра есть в Synthos и наоборот.
        for s in synaptix_flow::ops::specs() {
            assert!(NodeKind::from_flow_kind(s.kind).is_some(), "{}", s.kind);
        }
        assert_eq!(metas().len(), synaptix_flow::ops::specs().count());
    }
}
