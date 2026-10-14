//! Ноды ядра synaptix-flow — общий нодовый движок synshell (ассистент,
//! рецепты, Synthos). Генерация идёт через службу synaptixd (LLM, картинки,
//! видео, музыка, речь, расшифровка), монтаж — ffmpeg (склейка, наложение
//! звука, кадр, обрезка, картинка в ролик), данные — текст, файл, шаблон,
//! строки, список.
//!
//! Стадии моделей картинок (FLUX.1, FLUX.2, Qwen-Image, SDXL) и MiniMax-H3:
//! модель → условие → латент → сэмплер → декодер; модели грузит служба,
//! дескрипторы между стадиями живут в её памяти.
//!
//! Метаданные (порты, поля) строятся по схемам ядра; исполняет граф служба
//! (`synaptixd::flow`), тем же ядром, что рецепты ассистента: список на
//! входе — пакет, нода выполняется для каждого элемента. Запуск ноды
//! отправляет её вместе с нодами ядра, от которых к ней идут дескрипторы
//! (их служба берёт из кэша стадий, если входы не менялись); остальные
//! входы — готовые значения: с нод ядра — их последние выходы, с родных
//! нод Synthos — через файлы (картинка на выходе — [`ImageData`], звук —
//! `AudioBuffer`, на входе картинка и звук пишутся во временные файлы).

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
    DataBlob, FieldSchema, FieldType, FieldValue, ImageData, NodeId, NodeInstance, NodeKind, NodeRuntime, PortKind,
    PortSchema, PortValue, PortsLayout, PortsSpec,
};
use super::acestep::field_row;
use super::{log_worker_done, log_worker_start};
use crate::icons::{
    MI_ARTICLE, MI_ASPECT_RATIO, MI_AUDIOTRACK, MI_AUTO_AWESOME, MI_CROP_SQUARE, MI_EDIT_NOTE, MI_FOLDER_OPEN, MI_GRAPHIC_EQ,
    MI_IMAGE_ICON, MI_INVENTORY_2, MI_LANGUAGE, MI_LAYERS, MI_LIBRARY_MUSIC, MI_MERGE_TYPE, MI_MIC, MI_MOVIE, MI_PALETTE,
    MI_PSYCHOLOGY, MI_RECORD_VOICE_OVER, MI_TRANSLATE, MI_TUNE,
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
    (NodeKind::FlowImageModel, "image_model", "Image Model", MI_INVENTORY_2),
    (NodeKind::FlowImageEncode, "image_encode", "Prompt Encoder", MI_TRANSLATE),
    (NodeKind::FlowImageRefs, "image_refs", "References", MI_LAYERS),
    (NodeKind::FlowImageLatent, "image_latent", "Empty Latent", MI_ASPECT_RATIO),
    (NodeKind::FlowImageToLatent, "image_to_latent", "Image to Latent", MI_IMAGE_ICON),
    (NodeKind::FlowImageSample, "image_sample", "Sampler", MI_AUTO_AWESOME),
    (NodeKind::FlowImageDecode, "image_decode", "Image Decoder", MI_PALETTE),
    (NodeKind::FlowH3Model, "h3_model", "H3 Model", MI_INVENTORY_2),
    (NodeKind::FlowH3Latent, "h3_latent", "H3 Video Size", MI_CROP_SQUARE),
    (NodeKind::FlowH3Encode, "h3_encode", "H3 Encoder", MI_TRANSLATE),
    (NodeKind::FlowH3Sample, "h3_sample", "H3 Sampler", MI_AUTO_AWESOME),
    (NodeKind::FlowH3DecodeVideo, "h3_decode_video", "H3 Video Decoder", MI_MOVIE),
    (NodeKind::FlowH3DecodeAudio, "h3_decode_audio", "H3 Audio Decoder", MI_GRAPHIC_EQ),
    (NodeKind::FlowLtxModel, "ltx_model", "LTX Model", MI_INVENTORY_2),
    (NodeKind::FlowLtxEncode, "ltx_encode", "LTX Encoder", MI_TRANSLATE),
    (NodeKind::FlowLtxNag, "ltx_nag", "LTX NAG", MI_TUNE),
    (NodeKind::FlowLtxImage, "ltx_image", "LTX Frame", MI_IMAGE_ICON),
    (NodeKind::FlowLtxStage1, "ltx_stage1", "LTX Stage1", MI_AUTO_AWESOME),
    (NodeKind::FlowLtxUpscale, "ltx_upscale", "LTX Upscale ×2", MI_ASPECT_RATIO),
    (NodeKind::FlowLtxStage2, "ltx_stage2", "LTX Stage2", MI_AUTO_AWESOME),
    (NodeKind::FlowLtxRetake, "ltx_retake", "LTX Retake", MI_MOVIE),
    (NodeKind::FlowLtxIcLora, "ltx_ic_lora", "LTX IC-LoRA", MI_MOVIE),
    (NodeKind::FlowLtxLipdub, "ltx_lipdub", "LTX LipDub", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowLtxA2v, "ltx_a2v", "LTX Audio to Video", MI_AUDIOTRACK),
    (NodeKind::FlowLtxDecodeVideo, "ltx_decode_video", "LTX Video Decoder", MI_MOVIE),
    (NodeKind::FlowLtxDecodeAudio, "ltx_decode_audio", "LTX Audio Decoder", MI_GRAPHIC_EQ),
];

/// Подкатегория палитры по разделу ядра (ключ `node.subcategory.*`).
fn subcategory(spec: &sf::NodeSpec) -> Option<&'static str> {
    match spec.category {
        "Модели картинок" => Some("flow_image"),
        "MiniMax-H3" => Some("flow_h3"),
        "LTX-2.3" => Some("flow_ltx"),
        _ => None,
    }
}

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
        PT::List | PT::Any | PT::Model | PT::Cond | PT::Latent => PortKind::Data,
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
                subcategory: subcategory(s),
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
    /// Сид прошлого запуска при поле «Сид» = 0: прогон потомка (декодера)
    /// берёт тот же, и служба отдаёт результат сэмплера из кэша.
    pub seed: Arc<Mutex<Option<u64>>>,
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
            seed: Arc::new(Mutex::new(None)),
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
            match super::common::write_wav(&p, buf) {
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

/// Подграф запуска ноды: она сама и ноды ядра, от которых к ней (по цепочке)
/// идут дескрипторы — модель, условие, латент существуют только в службе.
/// Прочие входы — готовые значения: с нод ядра — последние выходы, с родных
/// нод — через файлы.
fn subflow(node: &NodeInstance, ctx: &NodeEditorCtx, dir: &Path) -> (sf::Flow, HashMap<(u64, String), Val>, HashMap<u64, FlowRuntime>) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let values = ctx.values.get_untracked();
    let by_id: HashMap<NodeId, &NodeInstance> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut included = vec![node.id];
    let mut i = 0;
    while i < included.len() {
        let id = included[i];
        i += 1;
        for c in conns.iter().filter(|c| c.to_node == id) {
            let Some(src) = by_id.get(&c.from_node) else { continue };
            let handle = spec(src.kind).and_then(|s| s.output(c.from_port)).is_some_and(|p| p.ty.is_handle());
            if handle && !included.contains(&src.id) {
                included.push(src.id);
            }
        }
    }
    let mut flow = sf::Flow::default();
    let mut rts = HashMap::new();
    for id in &included {
        let n = by_id[id];
        let (Some(fk), Some(rt)) = (n.kind.flow_kind(), runtime(n)) else { continue };
        let mut f = sf::FlowNode::new(id.0, fk);
        f.fields = fields_json(n);
        // Сид 0 — случайный: свой запуск — новый, у предков — прошлый.
        if f.fields.get("seed").and_then(Json::as_f64) == Some(0.0) {
            let mut last = rt.seed.lock().unwrap_or_else(|e| e.into_inner());
            let seed = match *last {
                Some(s) if *id != node.id => s,
                _ => random_seed(id.0),
            };
            *last = Some(seed);
            f.fields.insert("seed".into(), Json::from(seed));
        }
        flow.nodes.push(f);
        rts.insert(id.0, rt);
    }
    let mut inputs = HashMap::new();
    for c in conns.iter().filter(|c| rts.contains_key(&c.to_node.0)) {
        if rts.contains_key(&c.from_node.0) {
            flow.links.push(sf::Link { from_node: c.from_node.0, from_port: c.from_port.into(), to_node: c.to_node.0, to_port: c.to_port.into() });
            continue;
        }
        let src = by_id.get(&c.from_node);
        let from_flow = src.and_then(|n| runtime(n)).and_then(|r| r.outputs.lock().ok().and_then(|o| o.get(c.from_port).cloned()));
        let v = match from_flow {
            Some(v) => v,
            None => values.get(&(c.from_node, c.from_port)).map(|pv| to_val(pv, dir, &format!("{}-{}", c.to_node.0, c.to_port))).unwrap_or_default(),
        };
        inputs.insert((c.to_node.0, c.to_port.to_string()), v);
    }
    flow.outputs = vec![node.id.0];
    (flow, inputs, rts)
}

fn random_seed(salt: u64) -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    (nanos ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)) % 1_000_000_007 + 1
}

/// Выходы ноды ядра → её состояние и провода Synthos.
fn publish(rt: &FlowRuntime, fk: &str, outs: sf::Outputs, summary: String) {
    let Some(spec) = sf::ops::spec(fk) else { return };
    let ports: HashMap<String, PortValue> = spec.outputs.iter().map(|p| (p.name.to_string(), outs.get(p.name).map(|v| to_port(v, p.ty)).unwrap_or_default())).collect();
    if let Ok(mut o) = rt.outputs.lock() {
        *o = outs;
    }
    if let Ok(mut p) = rt.ports.lock() {
        *p = ports;
    }
    rt.error.set(None);
    rt.status.set(summary);
    // update — только в главном потоке (set годится из любого).
    let version = rt.version;
    syngui::async_runtime::run_on_main_thread(move || version.update(|v| *v = v.wrapping_add(1)));
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
    let (flow, inputs, rts) = subflow(node, ctx, &dir);
    let kinds: HashMap<u64, String> = flow.nodes.iter().map(|n| (n.id, n.kind.clone())).collect();
    let target = node.id.0;
    rt.cancel.store(false, Ordering::SeqCst);
    rt.running.set(true);
    rt.error.set(None);
    rt.progress.set(0.0);
    rt.status.set(String::new());
    let _ = std::thread::Builder::new().name(format!("synthos-flow-{fk}")).spawn(move || {
        let started = log_worker_start("flow", fk);
        let cancel = sf::Cancel::new();
        // Флаг Stop из run_controls → отмена прогона (рвёт запрос к службе).
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
        let opts = sf::RunOpts { cancel, inputs, ..Default::default() };
        let res = synaptixd::flow::run_with(&flow, &dir, &opts, &mut |e| match e {
            sf::Event::Started { id, item, .. } => {
                if let Some(r) = rts.get(&id) {
                    r.progress.set(0.0);
                    r.status.set(match item {
                        Some((k, n)) => format!("{k} / {n}"),
                        None if id == target => String::new(),
                        None => "выполняется…".into(),
                    });
                }
            }
            sf::Event::Progress { id, step, total } if total > 0 => {
                if let Some(r) = rts.get(&id) {
                    r.progress.set(step as f32 / total as f32);
                    if id != target {
                        r.status.set(format!("шаг {step} / {total}"));
                    }
                }
            }
            sf::Event::Finished { id, outputs, summary, .. } => {
                if let (Some(r), Some(k)) = (rts.get(&id), kinds.get(&id)) {
                    publish(r, k, outputs, summary);
                }
            }
            _ => {}
        });
        done.store(true, Ordering::SeqCst);
        log_worker_done("flow", started, &res);
        if let Err(e) = res {
            rt.error.set(Some(e));
        }
        rt.running.set(false);
    });
}

// ── Тело карточки ─────────────────────────────────────────────────────────

/// Вид моделей службы для поля «Модель» ноды (и фильтр по id).
fn model_kind(fk: &str) -> Option<(synaptixd::ModelKind, Option<&'static str>)> {
    use synaptixd::ModelKind as K;
    Some(match fk {
        "image" | "image_model" => (K::Image, None),
        "video" => (K::Video, None),
        "h3_model" => (K::Video, Some("h3")),
        "ltx_model" => (K::Video, Some("ltx")),
        "music" => (K::Music, None),
        "llm" => (K::Llm, None),
        "speak" => (K::Tts, None),
        _ => return None,
    })
}

/// Модели службы (список кэшируется на минуту: карточки строятся часто).
fn service_models(kind: synaptixd::ModelKind, filter: Option<&str>) -> Vec<String> {
    static CACHE: std::sync::OnceLock<Mutex<Option<(std::time::Instant, Vec<synaptixd::ModelEntry>)>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = g.as_ref().is_some_and(|(t, _)| t.elapsed() < std::time::Duration::from_secs(60));
    if !fresh {
        let list = synaptixd::Client::connect().and_then(|mut c| c.list()).unwrap_or_default();
        *g = Some((std::time::Instant::now(), list));
    }
    let mut ids: Vec<String> = g.as_ref().map(|(_, l)| l.iter().filter(|m| m.kind == kind && filter.is_none_or(|f| m.id.contains(f))).map(|m| m.id.clone()).collect()).unwrap_or_default();
    ids.sort();
    ids
}

/// Поле «Модель»: модели службы нужного вида; пусто — модель по умолчанию;
/// путь или чужой id из шаблона остаётся вариантом списка.
fn model_control(kind: synaptixd::ModelKind, filter: Option<&'static str>, sig: RwSignal<String>) -> Box<dyn Widget> {
    const AUTO: &str = "по умолчанию";
    let current = sig.get_untracked();
    let mut ids = service_models(kind, filter);
    if !current.is_empty() && !ids.contains(&current) {
        ids.insert(0, current.clone());
    }
    let mut items = vec![DropdownItem::simple(AUTO)];
    items.extend(ids.iter().map(|i| DropdownItem::simple(i.clone())));
    Box::new(
        Dropdown::with_items(items)
            .selected(if current.is_empty() { AUTO.to_string() } else { current })
            .on_change(move |val| sig.set(if val == AUTO { String::new() } else { val.to_string() }))
            .class("node-input-dropdown"),
    )
}

/// Путь к файлу: поле и кнопка выбора.
fn file_control(sig: RwSignal<String>) -> Box<dyn Widget> {
    let field = TextField::new().text(sig.get_untracked()).placeholder("путь к файлу").on_change(move |s| sig.set(s.to_string())).class("node-input-text");
    let pick = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip("Выбрать файл")
        .on_click(move || {
            if let Some(p) = rfd::FileDialog::new().pick_file() {
                sig.set(p.display().to_string());
            }
        })
        .class("node-file-picker-btn");
    Box::new(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).children(vec![Box::new(field) as Box<dyn Widget>, Box::new(pick)]))
}

fn field_control(fk: &str, f: &'static sf::FieldSpec, value: FieldValue) -> Box<dyn Widget> {
    if let (FieldValue::Text(sig), Some((kind, filter)), "model") = (&value, model_kind(fk), f.name) {
        return model_control(kind, filter, *sig);
    }
    if let (FT::File, FieldValue::Text(sig)) = (f.ty, &value) {
        return file_control(*sig);
    }
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
        (_, FieldValue::Float(sig)) => {
            // Дробные поля — два знака и мелкий шаг, остальные — целые.
            let (dp, step) = match f.name {
                "volume" | "strength" | "guidance" | "denoise" | "alpha" | "tau" | "canny_low" | "canny_high" | "lora_strength" => (2, 0.05),
                "seconds" | "start" | "end" => (1, 0.5),
                _ => (0, 1.0),
            };
            Box::new(
                SpinBox::new()
                    .min(0.0)
                    .max(4_294_967_295.0)
                    .step(step)
                    .decimal_places(dp)
                    .value(sig.get_untracked() as f64)
                    .on_change(move |v| sig.set(v as f32))
                    .width(120.0)
                    .class("node-input-spinbox"),
            )
        }
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
        // Дескриптор (модель, условие, латент) уже описан строкой статуса.
        if v.handle().is_some() {
            continue;
        }
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
            col = col.children(vec![field_row(f.label, field_control(spec.kind, f, v))]);
        }
    }
    let (running, error, status, progress, version) = (rt.running, rt.error, rt.status, rt.progress, rt.version);
    col = col.children(vec![super::common::progress_row(running, progress)]);
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
