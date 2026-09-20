//! Ноды YuE2: реестр, тела карточек, резолв бандлов и состояние в шаблонах.
//!
//! Зачем тест: у ноды три независимых списка — порты в реестре, поля runtime и
//! состояние шаблона. Разойдись они, нода выглядит целой, но теряет настройки
//! при сохранении графа или молча не получает вход. Тела карточек проверяются
//! рендером: `body()` читает runtime и при несовпадении варианта возвращает
//! «invalid runtime» — здесь это видно сразу.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::path::Path;

use syngui::prelude::*;
use syngui::testing::*;

use synthos::pages::node_editor::nodes::yue2;
use synthos::pages::node_editor::registry;
use synthos::pages::node_editor::types::{
    NodeInstance, NodeKind, NodeRuntime, NodeStyle, PortsSpec, Yue2ModelHandle,
};
use synthos::templates::convert;
use synthos::templates::model::NodeStateData;

const KINDS: [NodeKind; 3] =
    [NodeKind::Yue2Checkpoint, NodeKind::Yue2Generate, NodeKind::Yue2VaeDecode];

/// Без каталогов `tr!` возвращает сам ключ, и проверка текста карточки
/// проверяла бы не то. Регистрация одна на процесс.
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
    let column = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![widget]);
    let mut h = TestHarness::new(Box::new(column));
    let engine = h.apply_mss(synthos::styles::styles());
    h.rebuild();
    h.apply_styles(&engine);
    h.layout(360.0, 1400.0);
    h
}

/// Нода вне полотна: реестр строит её поля и runtime так же, как редактор.
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

/// Весь текст, который карточка реально рисует.
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
fn registry_knows_all_three_nodes() {
    for kind in KINDS {
        let meta = registry::meta(kind);
        assert_eq!(meta.kind, kind, "meta({kind:?}) вернула чужую ноду");
        assert_eq!(meta.subcategory, Some("yue2"), "{kind:?}: подкатегория");
        assert!(meta.body.is_some(), "{kind:?}: нет тела карточки");
    }
    // Порты: у Generate три входа контента плюс модель и три выхода.
    let gen = registry::meta(NodeKind::Yue2Generate);
    assert_eq!(port_names(&gen.inputs), vec!["model", "style", "lyrics", "abc"]);
    assert_eq!(port_names(&gen.outputs), vec!["audio", "score", "latent"]);
    assert!(gen.on_run.is_some() && gen.busy_signal.is_some(), "Generate без запуска");

    let decode = registry::meta(NodeKind::Yue2VaeDecode);
    assert_eq!(port_names(&decode.inputs), vec!["model", "latent"]);
    assert_eq!(port_names(&decode.outputs), vec!["audio"]);

    let ckpt = registry::meta(NodeKind::Yue2Checkpoint);
    assert!(port_names(&ckpt.inputs).is_empty(), "у чекпойнта входов нет");
    assert_eq!(port_names(&ckpt.outputs), vec!["model"]);
    // Чекпойнт ничего не считает — кнопки запуска у него быть не должно.
    assert!(ckpt.on_run.is_none() && ckpt.busy_signal.is_none());
}

#[test]
fn node_bodies_render() {
    init_i18n();
    for kind in KINDS {
        let node = make_node(kind, 1);
        let body = registry::meta(kind).body.expect("тело карточки")(&node);
        let mut h = render(body);
        let text = texts(&mut h);
        assert!(
            !text.contains("invalid runtime") && !text.to_lowercase().contains("invalid"),
            "{kind:?}: тело собрано не тем вариантом runtime: {text}"
        );
        assert!(!text.trim().is_empty(), "{kind:?}: пустая карточка");
    }
    // На карточке Generate — режим партитуры, длительность и шаги решателя.
    let node = make_node(NodeKind::Yue2Generate, 1);
    let body = registry::meta(NodeKind::Yue2Generate).body.unwrap()(&node);
    let mut h = render(body);
    let text = texts(&mut h);
    for expect in ["full", "32", "Seed"] {
        assert!(text.contains(expect), "на карточке Generate нет `{expect}`: {text}");
    }
}

#[test]
fn checkpoint_defaults_to_app_models_dir() {
    let node = make_node(NodeKind::Yue2Checkpoint, 1);
    let guard = node.runtime.lock().unwrap();
    match &*guard {
        NodeRuntime::Yue2Checkpoint { models_dir, device_idx, vae_dtype_idx, .. } => {
            assert!(
                models_dir.get_untracked().is_some(),
                "каталог моделей должен быть заполнен — иначе нода из меню требует ручного выбора"
            );
            // GPU по умолчанию: 3,6B на CPU считались бы минутами.
            assert_eq!(device_idx.get_untracked(), 1);
            // Эталонный тракт декодера — F32.
            assert_eq!(vae_dtype_idx.get_untracked(), 0);
        }
        other => panic!("не тот runtime: {other:?}"),
    }
}

/// Состояние шаблона должно возвращаться в ноду как есть: иначе граф
/// «сохранился и поехал» при следующем открытии.
#[test]
fn generate_state_round_trips_through_template() {
    let node = make_node(NodeKind::Yue2Generate, 1);
    {
        let guard = node.runtime.lock().unwrap();
        let NodeRuntime::Yue2Generate { cot_idx, seconds, ode_steps, top_k, .. } = &*guard else {
            panic!("не тот runtime");
        };
        cot_idx.set(1);
        seconds.set(45.0);
        ode_steps.set(16);
        top_k.set(64);
    }
    let state = {
        let guard = node.runtime.lock().unwrap();
        convert::runtime_to_state(&guard).expect("состояние ноды")
    };
    let NodeStateData::Yue2Generate(data) = &state else {
        panic!("ожидался Yue2Generate, а получен {state:?}");
    };
    assert_eq!((data.cot_idx, data.seconds, data.ode_steps, data.top_k), (1, 45.0, 16, 64));

    // Применяем в свежую ноду — значения должны совпасть.
    let fresh = make_node(NodeKind::Yue2Generate, 2);
    {
        let guard = fresh.runtime.lock().unwrap();
        convert::apply_state_to_runtime(&guard, &state);
        let NodeRuntime::Yue2Generate { cot_idx, seconds, ode_steps, top_k, .. } = &*guard else {
            panic!("не тот runtime");
        };
        assert_eq!(cot_idx.get_untracked(), 1);
        assert_eq!(seconds.get_untracked(), 45.0);
        assert_eq!(ode_steps.get_untracked(), 16);
        assert_eq!(top_k.get_untracked(), 64);
    }
}

fn handle(dir: Option<&Path>, model: Option<&str>) -> Yue2ModelHandle {
    Yue2ModelHandle {
        models_dir: dir.map(|d| d.to_path_buf()),
        model_path: model.map(std::path::PathBuf::from),
        vae_path: None,
        device_idx: 1,
        quant_idx: 0,
        compute_idx: 0,
        vae_dtype_idx: 0,
        resident: false,
    }
}

#[test]
fn resolve_paths_uses_directory_and_relative_overrides() {
    init_i18n();
    let dir = tempfile::tempdir().unwrap();
    for name in ["yue2-3b.syn", "yue2-vae.syn", "yue2-vae-legacy.syn"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    let (model, vae) = yue2::shared::resolve_paths(&handle(Some(dir.path()), None)).expect("резолв");
    assert_eq!(model, dir.path().join("yue2-3b.syn"));
    assert_eq!(vae, dir.path().join("yue2-vae.syn"));

    // Голое имя (так их печатает схема ноды агенту) — от каталога моделей,
    // а не от рабочей папки процесса.
    let (model, _) = yue2::shared::resolve_paths(&handle(Some(dir.path()), Some("yue2-3b.syn")))
        .expect("резолв с override");
    assert_eq!(model, dir.path().join("yue2-3b.syn"));

    // Ни каталога, ни override'а — понятная ошибка, а не пустой путь.
    assert!(yue2::shared::resolve_paths(&handle(None, None)).is_err());

    // Каталог есть, а бандла в нём нет — тоже ошибка с путём в тексте.
    let empty = tempfile::tempdir().unwrap();
    let err = yue2::shared::resolve_paths(&handle(Some(empty.path()), None)).unwrap_err();
    assert!(err.contains("yue2-3b.syn"), "в ошибке нет имени бандла: {err}");
}

/// Декодер по умолчанию — F32 (эталонный тракт), BF16 выбирается вторым
/// пунктом; порядок важен, потому что индекс хранится в графе.
#[test]
fn vae_dtype_options_order() {
    assert_eq!(yue2::VAE_DTYPE_OPTIONS, &["f32", "bf16"]);
    assert_eq!(yue2::vae_dtype_from_idx(0), synaptix_core::dtype::DType::F32);
    assert_eq!(yue2::vae_dtype_from_idx(1), synaptix_core::dtype::DType::BF16);
    // Индекс вне списка не должен менять тракт молча на что-то другое.
    assert_eq!(yue2::vae_dtype_from_idx(7), synaptix_core::dtype::DType::F32);
}

/// Режим партитуры хранится индексом — порядок пунктов тоже часть формата.
#[test]
fn cot_options_order() {
    use synaptix_music_yue2::protocol::Cot;
    assert_eq!(yue2::COT_OPTIONS, &["full", "melody", "off"]);
    assert_eq!(yue2::cot_from_idx(0), Cot::Full);
    assert_eq!(yue2::cot_from_idx(1), Cot::Melody);
    assert_eq!(yue2::cot_from_idx(2), Cot::Off);
}

/// Встроенные шаблоны ссылаются на порты строками: опечатка («tags» вместо
/// «style») заметна только тем, что нода молча не получает вход.
#[test]
fn builtin_templates_reference_existing_ports() {
    init_i18n();
    let all = synthos::templates::builtin::all();
    let ours: Vec<_> = all
        .iter()
        .filter(|t| t.id.starts_with("builtin-yue2-"))
        .collect();
    assert_eq!(ours.len(), 3, "ожидались три шаблона YuE2, найдено {}", ours.len());

    for t in &ours {
        let kind_of = |id: u64| t.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        assert!(
            t.nodes.iter().any(|n| n.kind == NodeKind::Yue2Generate),
            "{}: в шаблоне нет ноды Generate",
            t.id
        );
        for c in &t.connections {
            let from = kind_of(c.from_node).unwrap_or_else(|| panic!("{}: нет ноды {}", t.id, c.from_node));
            let to = kind_of(c.to_node).unwrap_or_else(|| panic!("{}: нет ноды {}", t.id, c.to_node));
            let outs = port_names(&registry::meta(from).outputs);
            let ins = port_names(&registry::meta(to).inputs);
            assert!(
                outs.contains(&c.from_port.as_str()),
                "{}: у {from:?} нет выхода `{}` (есть {outs:?})",
                t.id,
                c.from_port
            );
            assert!(
                ins.contains(&c.to_port.as_str()),
                "{}: у {to:?} нет входа `{}` (есть {ins:?})",
                t.id,
                c.to_port
            );
        }
    }

    // Партитура должна быть видна: выход `score` подключён в каждом шаблоне.
    for t in &ours {
        assert!(
            t.connections.iter().any(|c| c.from_port == "score"),
            "{}: выход score никуда не идёт — партитуру не видно",
            t.id
        );
    }
    // Два из трёх шаблонов принимают правленую партитуру во вход `abc`.
    let with_abc = ours.iter().filter(|t| t.connections.iter().any(|c| c.to_port == "abc")).count();
    assert_eq!(with_abc, 2, "шаблонов с входом abc должно быть два");
}
