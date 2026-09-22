//! Ноды Qwen-Image 2.1: реестр и порты, тела карточек, состояние чекпойнта и
//! сэмплера в шаблонах, дефолты, связи встроенных шаблонов, альфа в ImageData.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::*;

use synthos::pages::node_editor::nodes::qwen_image21;
use synthos::pages::node_editor::registry;
use synthos::pages::node_editor::types::{ImageData, NodeInstance, NodeKind, NodeRuntime, NodeStyle, PortsSpec};
use synthos::templates::convert;
use synthos::templates::model::NodeStateData;

const KINDS: [NodeKind; 5] = [
    NodeKind::QwenImage21Checkpoint,
    NodeKind::QwenImage21TextEncoder,
    NodeKind::QwenImage21Reference,
    NodeKind::QwenImage21Sampler,
    NodeKind::QwenImage21VaeDecode,
];

fn init_i18n() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
        syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));
    });
}

fn port_names(spec: &PortsSpec) -> Vec<&'static str> {
    match spec {
        PortsSpec::Static(p) => p.iter().map(|p| p.name).collect(),
        _ => Vec::new(),
    }
}

fn render(widget: Box<dyn Widget>) -> TestHarness {
    let column = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).children(vec![widget]);
    let mut h = TestHarness::new(Box::new(column));
    let engine = h.apply_mss(synthos::styles::styles());
    h.rebuild();
    h.apply_styles(&engine);
    h.layout(360.0, 1400.0);
    h
}

fn make_node(kind: NodeKind, id: u64) -> NodeInstance {
    NodeInstance {
        id: synthos::pages::node_editor::types::NodeId(id),
        kind,
        pos: use_signal(Point::new(0.0, 0.0)),
        bounds: use_signal(Rect::zero()),
        fields: registry::default_fields(kind),
        runtime: registry::default_runtime(kind),
        style: use_signal(NodeStyle::default()),
        enabled: use_signal(true),
        timing: synthos::pages::node_editor::timing::Stopwatch::new(),
    }
}

fn texts(h: &mut TestHarness) -> String {
    h.paint()
        .commands()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCommand::Text { text, .. } => Some(text.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn registry_knows_all_five_nodes() {
    for kind in KINDS {
        let meta = registry::meta(kind);
        assert_eq!(meta.kind, kind, "meta({kind:?}) вернула чужую ноду");
        assert_eq!(meta.subcategory, Some("qwen_image21"), "{kind:?}: подкатегория");
        assert!(meta.body.is_some(), "{kind:?}: нет тела карточки");
    }
    let te = registry::meta(NodeKind::QwenImage21TextEncoder);
    assert_eq!(port_names(&te.inputs), vec!["model", "prompt", "negative", "references"]);
    assert_eq!(port_names(&te.outputs), vec!["conditioning"]);
    let smp = registry::meta(NodeKind::QwenImage21Sampler);
    assert_eq!(port_names(&smp.inputs), vec!["model", "conditioning", "references", "latent"]);
    assert_eq!(port_names(&smp.outputs), vec!["latent"]);
    let r = registry::meta(NodeKind::QwenImage21Reference);
    assert_eq!(port_names(&r.inputs), vec!["model", "image", "references"]);
    assert_eq!(port_names(&r.outputs), vec!["references"]);
    let dec = registry::meta(NodeKind::QwenImage21VaeDecode);
    assert_eq!(port_names(&dec.inputs), vec!["model", "latent"]);
    assert_eq!(port_names(&dec.outputs), vec!["image"]);
    let ckpt = registry::meta(NodeKind::QwenImage21Checkpoint);
    assert!(port_names(&ckpt.inputs).is_empty());
    assert_eq!(port_names(&ckpt.outputs), vec!["model"]);
    assert!(ckpt.on_run.is_none() && ckpt.busy_signal.is_none());
    for k in [NodeKind::QwenImage21TextEncoder, NodeKind::QwenImage21Reference, NodeKind::QwenImage21Sampler, NodeKind::QwenImage21VaeDecode] {
        let m = registry::meta(k);
        assert!(m.on_run.is_some() && m.busy_signal.is_some(), "{k:?} без запуска");
    }
}

#[test]
fn node_bodies_render() {
    init_i18n();
    for kind in KINDS {
        let node = make_node(kind, 1);
        let body = registry::meta(kind).body.expect("тело карточки")(&node);
        let mut h = render(body);
        let text = texts(&mut h);
        assert!(!text.to_lowercase().contains("invalid"), "{kind:?}: тело собрано не тем вариантом runtime: {text}");
        assert!(!text.trim().is_empty(), "{kind:?}: пустая карточка");
    }
    // Чекпойнт показывает разрешение (1024 по умолчанию) и квант mxfp8.
    let node = make_node(NodeKind::QwenImage21Checkpoint, 1);
    let body = registry::meta(NodeKind::QwenImage21Checkpoint).body.unwrap()(&node);
    let mut h = render(body);
    let text = texts(&mut h);
    for expect in ["1024", "mxfp8", "Разрешение"] {
        assert!(text.contains(expect), "на карточке Checkpoint нет `{expect}`: {text}");
    }
    // Сэмплер: KV-кэш и Seed.
    let node = make_node(NodeKind::QwenImage21Sampler, 2);
    let body = registry::meta(NodeKind::QwenImage21Sampler).body.unwrap()(&node);
    let mut h = render(body);
    let text = texts(&mut h);
    for expect in ["KV-кэш", "Seed", "CFG"] {
        assert!(text.contains(expect), "на карточке Sampler нет `{expect}`: {text}");
    }
}

#[test]
fn defaults_follow_the_model_card() {
    let node = make_node(NodeKind::QwenImage21Checkpoint, 1);
    match &*node.runtime.lock().unwrap() {
        NodeRuntime::QwenImage21Checkpoint { quant_idx, resolution_idx, memory_mode_idx, resident, .. } => {
            assert_eq!(quant_idx.get_untracked(), qwen_image21::DEFAULT_QUANT_IDX);
            assert_eq!(qwen_image21::QUANT_OPTIONS[quant_idx.get_untracked()], "mxfp8");
            assert_eq!(qwen_image21::resolution_of(resolution_idx.get_untracked()), 1024);
            assert_eq!(memory_mode_idx.get_untracked(), 0);
            assert!(!resident.get_untracked());
        }
        other => panic!("не тот runtime: {other:?}"),
    }
    let node = make_node(NodeKind::QwenImage21Sampler, 2);
    match &*node.runtime.lock().unwrap() {
        NodeRuntime::QwenImage21Sampler { steps, cfg, kv_cache, .. } => {
            assert_eq!(steps.get_untracked(), 0, "0 — по модели (40)");
            assert_eq!(cfg.get_untracked(), 1.0, "модель идёт без CFG");
            assert!(kv_cache.get_untracked());
        }
        other => panic!("не тот runtime: {other:?}"),
    }
    assert_eq!(qwen_image21::sampler::resolve_steps(0), 40);
}

#[test]
fn states_round_trip_through_template() {
    let ckpt = make_node(NodeKind::QwenImage21Checkpoint, 1);
    if let NodeRuntime::QwenImage21Checkpoint { model_path, quant_idx, resolution_idx, resident, .. } = &*ckpt.runtime.lock().unwrap() {
        model_path.set(Some("/models/qwen-image-2.1.syn".into()));
        quant_idx.set(2);
        resolution_idx.set(4);
        resident.set(true);
    }
    let state = convert::runtime_to_state(&ckpt.runtime.lock().unwrap()).expect("state чекпойнта");
    match &state {
        NodeStateData::QwenImage21Checkpoint(d) => {
            assert_eq!(d.model_path.as_deref(), Some("/models/qwen-image-2.1.syn"));
            assert_eq!((d.quant_idx, d.resolution_idx, d.resident), (2, 4, true));
        }
        other => panic!("не то состояние: {other:?}"),
    }
    let fresh = make_node(NodeKind::QwenImage21Checkpoint, 3);
    {
        let rt = fresh.runtime.lock().unwrap();
        convert::apply_state_to_runtime(&rt, &state);
        if let NodeRuntime::QwenImage21Checkpoint { quant_idx, resolution_idx, resident, .. } = &*rt {
            assert_eq!((quant_idx.get_untracked(), resolution_idx.get_untracked(), resident.get_untracked()), (2, 4, true));
        }
    }

    let smp = make_node(NodeKind::QwenImage21Sampler, 2);
    if let NodeRuntime::QwenImage21Sampler { steps, cfg, seed, kv_cache, .. } = &*smp.runtime.lock().unwrap() {
        steps.set(20);
        cfg.set(3.0);
        seed.set(9);
        kv_cache.set(false);
    }
    let state = convert::runtime_to_state(&smp.runtime.lock().unwrap()).expect("state сэмплера");
    let fresh = make_node(NodeKind::QwenImage21Sampler, 4);
    let rt = fresh.runtime.lock().unwrap();
    convert::apply_state_to_runtime(&rt, &state);
    if let NodeRuntime::QwenImage21Sampler { steps, cfg, seed, kv_cache, .. } = &*rt {
        assert_eq!((steps.get_untracked(), cfg.get_untracked(), seed.get_untracked(), kv_cache.get_untracked()), (20, 3.0, 9, false));
    }
}

#[test]
fn builtin_templates_reference_existing_ports() {
    init_i18n();
    let all = synthos::templates::builtin::all();
    let ours: Vec<_> = all.iter().filter(|t| t.id.starts_with("builtin-qwen-image21-")).collect();
    assert_eq!(ours.len(), 5, "ожидались пять шаблонов Qwen-Image 2.1, найдено {}", ours.len());
    for t in &ours {
        let kind_of = |id: u64| t.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        assert!(t.nodes.iter().any(|n| n.kind == NodeKind::QwenImage21Sampler), "{}: нет Sampler", t.id);
        for c in &t.connections {
            let from = kind_of(c.from_node).unwrap_or_else(|| panic!("{}: нет ноды {}", t.id, c.from_node));
            let to = kind_of(c.to_node).unwrap_or_else(|| panic!("{}: нет ноды {}", t.id, c.to_node));
            let outs = port_names(&registry::meta(from).outputs);
            let ins = port_names(&registry::meta(to).inputs);
            assert!(outs.contains(&c.from_port.as_str()), "{}: у {from:?} нет выхода `{}`", t.id, c.from_port);
            assert!(ins.contains(&c.to_port.as_str()), "{}: у {to:?} нет входа `{}`", t.id, c.to_port);
        }
        // Референсы идут и в энкодер, и в сэмплер — иначе сэмплер откажет.
        let refs_to = |kind: NodeKind| {
            t.connections.iter().any(|c| c.to_port == "references" && kind_of(c.to_node) == Some(kind))
        };
        assert_eq!(refs_to(NodeKind::QwenImage21TextEncoder), refs_to(NodeKind::QwenImage21Sampler), "{}", t.id);
    }
    let edit = ours.iter().find(|t| t.id == "builtin-qwen-image21-edit").unwrap();
    assert!(edit.nodes.iter().any(|n| n.kind == NodeKind::QwenImage21Reference));
    assert!(!edit.nodes.iter().any(|n| n.kind == NodeKind::FluxEmptyLatent), "размер правки — с картинки");
    let multi = ours.iter().find(|t| t.id == "builtin-qwen-image21-multi-reference").unwrap();
    assert_eq!(multi.nodes.iter().filter(|n| n.kind == NodeKind::QwenImage21Reference).count(), 2);
    let llm = ours.iter().find(|t| t.id == "builtin-qwen-image21-llm-rewrite").unwrap();
    assert!(llm.nodes.iter().any(|n| n.kind == NodeKind::Llm));
}

#[test]
fn image_data_keeps_alpha() {
    synthos::pages::node_editor::nodes::flux::shared::ensure_kernels_registered();
    // RGBA 2×1: непрозрачный и полупрозрачный пиксель.
    let v = vec![1.0f32, 0.0, 0.5, 0.5, 0.0, 1.0, 1.0, 0.25];
    let t = synaptix_core::tensor::Tensor::from_vec(v, vec![4, 1, 2], synaptix_core::device::Device::Cpu).unwrap();
    let img = ImageData::from_rgba_tensor(t, None).unwrap();
    assert_eq!(img.tensor.dims(), &[3, 1, 2], "tensor остаётся RGB");
    assert_eq!(&img.rgba[..], &[255, 128, 0, 255, 0, 128, 255, 64]);
    assert!(img.has_alpha());
    let back = img.rgba_tensor().unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
    assert!((back[6] - 1.0).abs() < 1e-6 && (back[7] - 64.0 / 255.0).abs() < 1e-6);
    // RGB → альфы нет.
    let rgb = synaptix_core::tensor::Tensor::from_vec(vec![0.5f32; 6], vec![3, 1, 2], synaptix_core::device::Device::Cpu).unwrap();
    assert!(!ImageData::from_tensor(rgb, None).unwrap().has_alpha());
}
