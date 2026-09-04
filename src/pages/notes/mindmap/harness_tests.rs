//! Headless-тесты интеллект-карты на `TestHarness` syngui: карта живёт во
//! врезке редактора страницы (свободная раскладка), как в приложении.
//! Геометрия узлов считается той же [`layout::compute`] с моно-метрикой
//! харнесса (10 px на символ), поэтому клики адресуются точно. Запуск:
//! `cargo test --features testing mindmap::harness_tests`.

use std::collections::HashMap;
use std::sync::Arc;

use syngui::core::{Point, Rect};
use syngui::input::{Event, Key, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widget::context::TextMeasure;
use syngui::widgets::input::document_editor::{DocLayout, DocumentEditor, DocumentEditorHandle, EmbedCtx, EmbedFactory};

use super::layout::{self, MapGeometry, Metrics};
use super::model::MindmapDoc;
use super::view::{view, MapEnv};
use super::MindmapHandle;

struct Mono;
impl TextMeasure for Mono {
    fn measure_text_width(&self, _t: &str, _fs: f32, chars: usize) -> f32 {
        chars as f32 * 10.0
    }
    fn hit_test_char(&self, text: &str, _fs: f32, x: f32) -> usize {
        ((x / 10.0).round() as usize).min(text.chars().count())
    }
}

struct Factory {
    handles: HashMap<String, MindmapHandle>,
}

impl EmbedFactory for Factory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let id = target.trim().strip_prefix("mindmap:")?;
        let handle = self.handles.get(id)?.clone();
        let env = MapEnv { open_page: Arc::new(|_| {}), to_list: Arc::new(|_| {}) };
        let body = view(env, id.to_string(), handle);
        Some(Box::new(
            DecoratedBox::new()
                .style("height", StyleValue::px(ectx.height.unwrap_or(420.0)))
                .child(crate::components::workspace_frame::expand(Box::new(body))),
        ))
    }
    fn has_own_height(&self, _target: &str) -> bool {
        true
    }
}

struct World {
    h: TestHarness,
    page: DocumentEditorHandle,
    epoch: u64,
    handles: HashMap<String, MindmapHandle>,
    md: String,
}

impl World {
    fn new(md: &str, handles: HashMap<String, MindmapHandle>) -> Self {
        let page = DocumentEditorHandle::new();
        let mut h = TestHarness::new(Box::new(Self::editor(md, &page, &handles, 0)));
        h.tree.text_measure = Some(Arc::new(Mono));
        h.rebuild();
        h.apply_mss(".grow { flex-grow: 1; }");
        h.layout(1200.0, 800.0);
        Self { h, page, epoch: 0, handles, md: md.to_string() }
    }

    fn editor(md: &str, page: &DocumentEditorHandle, handles: &HashMap<String, MindmapHandle>, epoch: u64) -> DocumentEditor {
        DocumentEditor::new()
            .markdown(md)
            .handle(page)
            .embeds(Arc::new(Factory { handles: handles.clone() }))
            .model_epoch(epoch)
            .layout(DocLayout { free: true, ..DocLayout::default() })
    }

    /// Перестроить после правок (сигналы карты, очередь операций страницы).
    fn settle(&mut self) {
        self.epoch += 1;
        let w = Self::editor(&self.md, &self.page, &self.handles, self.epoch);
        self.h.update_widget(Box::new(w));
        self.h.rebuild();
        self.h.apply_mss(".grow { flex-grow: 1; }");
        self.h.layout(1200.0, 800.0);
    }

    fn canvas(&self) -> Rect {
        let ids = self.h.find_by_type_name("notes-mindmap-canvas");
        assert_eq!(ids.len(), 1, "ровно один холст карты");
        self.h.element_bounds(ids[0])
    }

    /// Фокус холсту — как делает приложение по клику (роль TextField).
    fn focus_canvas(&mut self) {
        let id = self.h.find_by_type_name("notes-mindmap-canvas")[0];
        self.h.tree.focused_element = Some(id);
        self.h.send_event(&Event::FocusGained);
    }

    fn click(&mut self, at: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: at });
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: at });
    }

    /// Нажать, сдвинуть за порог, довести до точки, отпустить.
    fn drag(&mut self, from: Point, to: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from });
        self.h.send_event(&Event::MouseMove(Point::new(from.x + 8.0, from.y + 8.0)));
        self.h.send_event(&Event::MouseMove(to));
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: to });
    }
}

fn mono(s: &str, _font: f32, _bold: bool) -> f32 {
    s.chars().count() as f32 * 10.0
}

/// Геометрия карты той же раскладкой, что у холста (моно-метрика).
fn geometry(doc: &MindmapDoc) -> MapGeometry {
    let m = Metrics {
        measure: &mono,
        font_size: doc.style.font_size,
        bold: doc.style.weight == "bold",
        padding: doc.style.padding,
        max_w: doc.style.max_node_w,
        show_icons: doc.style.show_icons,
    };
    layout::compute(doc, &doc.layout, &m)
}

fn center_of(canvas: Rect, r: Rect) -> Point {
    Point::new(canvas.origin.x + r.origin.x + r.size.width / 2.0, canvas.origin.y + r.origin.y + r.size.height / 2.0)
}

fn map_with(children: &[&str]) -> (World, MindmapHandle) {
    let mut doc = MindmapDoc::template("Центр");
    let root = doc.root_id();
    for c in children {
        doc.add_node(&root, c, None);
    }
    let handle = MindmapHandle::new(doc);
    let mut handles = HashMap::new();
    handles.insert("m1".to_string(), handle.clone());
    (World::new("![[mindmap:m1]]{h=420}\n", handles), handle)
}

#[test]
fn click_selects_node_and_tab_adds_child_with_inline_editor() {
    let (mut w, handle) = map_with(&["Идея"]);
    let canvas = w.canvas();
    assert!(canvas.size.width > 100.0 && canvas.size.height > 100.0, "холст без размера: {canvas:?}");
    let g = geometry(&handle.lock());
    let root_id = handle.root_id();
    let root = g.rect_of(&root_id).unwrap();
    w.click(center_of(canvas, root));
    assert_eq!(handle.selected.get_untracked().as_deref(), Some(root_id.as_str()));

    // Tab — дочерний узел сразу в правке: над ним монтируется TextField.
    w.focus_canvas();
    w.h.send_event(&Event::KeyDown(Key::Tab));
    let editing = handle.editing.get_untracked().expect("новый узел в правке");
    assert_eq!(handle.lock().children_of(&root_id).len(), 2);
    assert_eq!(handle.selected.get_untracked().as_deref(), Some(editing.as_str()));
    w.settle();
    let fields = w.h.find_by_type_name("TextField");
    assert_eq!(fields.len(), 1, "редактор узла должен смонтироваться");
    let field = w.h.element_bounds(fields[0]);
    let node = geometry(&handle.lock()).rect_of(&editing).unwrap();
    assert!((field.origin.x - (canvas.origin.x + node.origin.x)).abs() < 1.0, "поле над узлом: {field:?} vs {node:?}");

    // Набор и Enter — текст в узле, правка закрыта, поле исчезло.
    for c in "План".chars() {
        w.h.send_event(&Event::CharInput(c));
    }
    w.h.send_event(&Event::KeyDown(Key::Enter));
    assert_eq!(handle.editing.get_untracked(), None);
    assert_eq!(handle.lock().node(&editing).unwrap().text, "План");
    w.settle();
    assert!(w.h.find_by_type_name("TextField").is_empty(), "после правки поля нет");

    // Delete удаляет выбранный узел с поддеревом.
    w.focus_canvas();
    w.h.send_event(&Event::KeyDown(Key::Delete));
    assert_eq!(handle.lock().children_of(&root_id).len(), 1);
    assert_eq!(handle.selected.get_untracked().as_deref(), Some(root_id.as_str()), "выбор переходит на родителя");
}

#[test]
fn double_click_starts_editing() {
    let (mut w, handle) = map_with(&["Идея"]);
    let canvas = w.canvas();
    let g = geometry(&handle.lock());
    let idea = handle.lock().children_of(&handle.root_id())[0].id.clone();
    let at = center_of(canvas, g.rect_of(&idea).unwrap());
    w.click(at);
    w.click(at);
    assert_eq!(handle.editing.get_untracked().as_deref(), Some(idea.as_str()));
}

#[test]
fn dragging_a_node_onto_another_reparents_it() {
    let (mut w, handle) = map_with(&["А", "Б"]);
    let canvas = w.canvas();
    let g = geometry(&handle.lock());
    let kids: Vec<String> = handle.lock().children_of(&handle.root_id()).iter().map(|n| n.id.clone()).collect();
    let (a, b) = (kids[0].clone(), kids[1].clone());
    let from = center_of(canvas, g.rect_of(&b).unwrap());
    let to = center_of(canvas, g.rect_of(&a).unwrap());
    w.drag(from, to);
    let doc = handle.lock();
    assert_eq!(doc.node(&b).unwrap().parent.as_deref(), Some(a.as_str()), "Б стал ребёнком А");
    assert_eq!(doc.children_of(&doc.root_id()).len(), 1);
}

#[test]
fn dragging_a_node_into_empty_space_offsets_its_subtree() {
    let (mut w, handle) = map_with(&["А", "Б"]);
    let canvas = w.canvas();
    let g = geometry(&handle.lock());
    let a = handle.lock().children_of(&handle.root_id())[0].id.clone();
    let r = g.rect_of(&a).unwrap();
    let from = center_of(canvas, r);
    // Далеко вправо-вниз, где узлов нет.
    let to = Point::new(from.x + 203.0, from.y + 147.0);
    w.drag(from, to);
    let doc = handle.lock();
    let n = doc.node(&a).unwrap();
    assert_eq!((n.dx, n.dy), (205.0, 145.0), "сдвиг с привязкой к 5 px");
    drop(doc);
    // Сброс раскладки снимает сдвиг.
    handle.reset_offsets();
    let n = handle.lock().node(&a).cloned().unwrap();
    assert_eq!((n.dx, n.dy), (0.0, 0.0));
}

#[test]
fn escape_and_empty_text_drop_a_fresh_node() {
    let (mut w, handle) = map_with(&[]);
    w.focus_canvas();
    let root = handle.root_id();
    handle.select(Some(root.clone()));
    w.h.send_event(&Event::KeyDown(Key::Tab));
    assert_eq!(handle.lock().nodes.len(), 2);
    w.settle();
    // Esc в пустом поле — узел без текста выбрасывается.
    w.h.send_event(&Event::KeyDown(Key::Escape));
    assert_eq!(handle.editing.get_untracked(), None);
    assert_eq!(handle.lock().nodes.len(), 1, "пустой узел не остаётся");
}
