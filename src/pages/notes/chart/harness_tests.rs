//! Headless-тесты графика на `TestHarness` syngui: виджет живёт во врезке
//! редактора страницы, как в приложении. Запуск:
//! `cargo test --features testing chart::harness_tests`.

use std::collections::HashMap;
use std::sync::Arc;

use syngui::core::Point;
use syngui::input::{Event, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widgets::input::document_editor::{DocLayout, DocumentEditor, DocumentEditorHandle, EmbedCtx, EmbedFactory};

use super::model::{ChartDoc, ChartKind};
use super::view::view;
use super::ChartHandle;

/// Стили, без которых врезка не работает: `.grow` — растяжка колонок
/// каркаса, `.notes-chart` — размер графика по блоку (в приложении это
/// `styles/components/notes.mss`).
const MSS: &str = ".grow { flex-grow: 1; } .notes-chart { width: 100%; height: 100%; }";

struct Factory {
    handles: HashMap<String, ChartHandle>,
}

impl EmbedFactory for Factory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let id = target.trim().strip_prefix("chart:")?;
        let handle = self.handles.get(id)?.clone();
        Some(Box::new(
            DecoratedBox::new()
                .style("height", StyleValue::px(ectx.height.unwrap_or(320.0)))
                .child(crate::components::workspace_frame::expand(Box::new(view(handle)))),
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
    handles: HashMap<String, ChartHandle>,
    md: String,
}

impl World {
    fn new(md: &str, handles: HashMap<String, ChartHandle>) -> Self {
        let page = DocumentEditorHandle::new();
        let mut h = TestHarness::new(Box::new(Self::editor(md, &page, &handles, 0)));
        h.rebuild();
        h.apply_mss(MSS);
        h.layout(1200.0, 800.0);
        Self { h, page, epoch: 0, handles, md: md.to_string() }
    }

    fn editor(md: &str, page: &DocumentEditorHandle, handles: &HashMap<String, ChartHandle>, epoch: u64) -> DocumentEditor {
        DocumentEditor::new()
            .markdown(md)
            .handle(page)
            .embeds(Arc::new(Factory { handles: handles.clone() }))
            .model_epoch(epoch)
            .layout(DocLayout { free: true, ..DocLayout::default() })
    }

    fn settle(&mut self) {
        self.epoch += 1;
        let w = Self::editor(&self.md, &self.page, &self.handles, self.epoch);
        self.h.update_widget(Box::new(w));
        self.h.rebuild();
        self.h.apply_mss(MSS);
        self.h.layout(1200.0, 800.0);
    }

    fn chart(&self, name: &str) -> syngui::core::Rect {
        let ids = self.h.find_by_type_name(name);
        assert_eq!(ids.len(), 1, "ровно один виджет {name}");
        self.h.element_bounds(ids[0])
    }

    fn click(&mut self, at: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: at });
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: at });
    }
}

fn world(doc: ChartDoc) -> (World, ChartHandle) {
    let handle = ChartHandle::new(doc);
    let mut handles = HashMap::new();
    handles.insert("g1".to_string(), handle.clone());
    // Закреплённый блок: ширина 800 больше встроенного умолчания графика
    // (600) — по ней и видно, что размер пришёл из врезки.
    (World::new("![[chart:g1]]{x=40 y=40 w=800 h=320}\n", handles), handle)
}

#[test]
fn chart_fills_its_block() {
    let (w, _handle) = world(ChartDoc::template(ChartKind::Line, "Ряд"));
    let bounds = w.chart("LineChart");
    assert!(bounds.size.width > 700.0, "график ужался до своего умолчания: {bounds:?}");
    assert!(bounds.size.height > 250.0, "график не занял высоту врезки: {bounds:?}");
}

#[test]
fn switching_the_kind_swaps_the_widget() {
    let (mut w, handle) = world(ChartDoc::template(ChartKind::Line, "Ряд"));
    assert_eq!(w.h.find_by_type_name("LineChart").len(), 1);
    handle.set_kind(ChartKind::Pie);
    w.settle();
    assert!(w.h.find_by_type_name("LineChart").is_empty(), "старый виджет остался");
    assert_eq!(w.h.find_by_type_name("PieChart").len(), 1);
    // Данные при смене вида не теряются — их правят в панели свойств.
    assert_eq!(handle.lock().categories.len(), 5);
}

#[test]
fn a_click_on_the_chart_selects_its_block() {
    let (mut w, _handle) = world(ChartDoc::template(ChartKind::Bar, "Ряд"));
    let bounds = w.chart("BarChart");
    // Клик мимо легенды: график его не поглощает, и блок становится
    // текущим — иначе панель свойств графика не открыть.
    let at = Point::new(bounds.origin.x + bounds.size.width * 0.5, bounds.origin.y + 20.0);
    w.click(at);
    assert!(w.page.selected().get_untracked().is_some(), "клик по графику не выбрал блок врезки");
}

#[test]
fn every_kind_mounts_its_widget() {
    for (kind, name) in [
        (ChartKind::Line, "LineChart"),
        (ChartKind::Bar, "BarChart"),
        (ChartKind::Pie, "PieChart"),
        (ChartKind::Radar, "RadarChart"),
        (ChartKind::Gauge, "GaugeChart"),
    ] {
        let (w, _handle) = world(ChartDoc::template(kind, "Ряд"));
        assert_eq!(w.h.find_by_type_name(name).len(), 1, "вид {} не смонтировался", kind.key());
    }
}

/// Панель свойств строится для каждого вида: у графика своего редактора
/// нет, и если она падает или встаёт в дедлок на вложенном `lock()`, менять
/// данные становится нечем.
#[test]
fn the_property_panel_builds_for_every_kind() {
    for kind in ChartKind::ALL {
        let handle = ChartHandle::new(ChartDoc::template(kind, "Ряд"));
        if kind == ChartKind::Gauge {
            handle.set_options(|o| o.zones.push(super::model::GaugeZone { from: 0.0, to: 50.0, color: "#4FBF7A".into() }));
        }
        let mut h = TestHarness::new(Box::new(crate::pages::notes::right_panel::chart_props(handle.clone())));
        h.rebuild();
        h.apply_mss(MSS);
        h.layout(280.0, 900.0);
        assert!(h.element_count() > 20, "панель вида {} почти пуста", kind.key());
        // Ручка отвечает после построения — значит мьютекс свободен.
        assert_eq!(handle.kind(), kind);
    }
}
