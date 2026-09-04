//! Интеллект-карта — примитив страницы: живая врезка `![[mindmap:<id>]]`
//! над объектом `notes/objects/<id>.mindmap.json`.
//!
//! [`MindmapHandle`] держит документ (Mutex) и сигналы: `revision` — на
//! каждую правку (автосейв), `structure_rev` — правка, меняющая вид
//! (перестройка виджета); `selected`/`editing` — выбранный узел и узел в
//! правке; `pan`/`zoom` — положение вида (`PanZoomViewport`), уходит в
//! документ без перестройки; `fit_request` — «вписать» из тулбара.

pub mod canvas;
pub mod layout;
pub mod model;
pub mod view;

#[cfg(all(test, feature = "testing"))]
mod harness_tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use model::{Curve, Direction, MapLayout, MindmapDoc, MindmapStyle, NodeShape};

/// Открыть страницу проекта по id (ссылка узла).
pub type OpenPage = Arc<dyn Fn(&str) + Send + Sync>;

pub const ZOOM_MIN: f32 = 0.25;
pub const ZOOM_MAX: f32 = 3.0;

#[derive(Clone)]
pub struct MindmapHandle {
    doc: Arc<Mutex<MindmapDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    pub selected: RwSignal<Option<String>>,
    pub editing: RwSignal<Option<String>>,
    pub pan: RwSignal<Point>,
    pub zoom: RwSignal<f32>,
    /// Бамп — вписать карту в окно.
    pub fit_request: RwSignal<u64>,
    /// Редакторы заметок узлов (панель свойств): ручка + исходник.
    editors: Arc<Mutex<HashMap<String, (DocumentEditorHandle, Arc<String>)>>>,
}

impl MindmapHandle {
    pub fn new(doc: MindmapDoc) -> Self {
        let pan = use_signal(Point::new(doc.view.pan.0, doc.view.pan.1));
        let zoom = use_signal(doc.view.zoom);
        let h = Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            selected: use_signal(None),
            editing: use_signal(None),
            pan,
            zoom,
            fit_request: use_signal(0),
            editors: Arc::new(Mutex::new(HashMap::new())),
        };
        // Положение вида — в документ (автосейв дебаунсит), без перестройки.
        let hv = h.clone();
        create_effect(move || {
            let p = hv.pan.get();
            let z = hv.zoom.get();
            let changed = {
                let d = hv.lock();
                (d.view.pan.0 - p.x).abs() > 0.5 || (d.view.pan.1 - p.y).abs() > 0.5 || (d.view.zoom - z).abs() > 0.001
            };
            if changed {
                hv.edit_data(|d| d.view = model::MapView { pan: (p.x, p.y), zoom: z });
            }
        });
        h
    }

    pub fn lock(&self) -> MutexGuard<'_, MindmapDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    fn bump(&self) {
        self.revision.set(self.revision.get_untracked() + 1);
    }

    /// Правка, меняющая вид: автосейв + перестройка.
    pub fn edit(&self, f: impl FnOnce(&mut MindmapDoc)) {
        f(&mut self.lock());
        self.bump();
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    /// Правка данных без перестройки (положение вида, текст по ходу набора).
    pub fn edit_data(&self, f: impl FnOnce(&mut MindmapDoc)) {
        f(&mut self.lock());
        self.bump();
    }

    pub fn root_id(&self) -> String {
        self.lock().root_id()
    }

    pub fn select(&self, id: Option<String>) {
        if self.selected.get_untracked() != id {
            self.selected.set(id);
        }
    }

    // ─── Узлы ─────────────────────────────────────────────────────────────

    /// Дочерний узел (у выбранного либо корня); выбирается и открывается
    /// на правку.
    pub fn add_child(&self, parent: Option<&str>, text: &str) -> Option<String> {
        let parent = parent.map(str::to_string).unwrap_or_else(|| self.root_id());
        let mut id = None;
        self.edit(|d| {
            if let Some(p) = d.node_mut(&parent) {
                p.collapsed = false;
            }
            id = d.add_node(&parent, text, None);
        });
        if let Some(id) = &id {
            self.select(Some(id.clone()));
            self.editing.set(Some(id.clone()));
        }
        id
    }

    /// Соседний узел после `sibling` (у корня — дочерний).
    pub fn add_sibling(&self, sibling: &str, text: &str) -> Option<String> {
        // Решение принимается под мьютексом, действие — уже без него:
        // вызов `add_child`/`edit` с живым guard'ом — дедлок (mutex не
        // реентрантный), а у корня это ровно тот случай.
        let spot = {
            let d = self.lock();
            d.node(sibling).and_then(|n| n.parent.clone()).map(|p| {
                let idx = d.children_of(&p).iter().position(|n| n.id == sibling).map(|i| i + 1);
                (p, idx)
            })
        };
        let Some((parent, index)) = spot else {
            return self.add_child(Some(sibling), text);
        };
        let mut id = None;
        self.edit(|d| id = d.add_node(&parent, text, index));
        if let Some(id) = &id {
            self.select(Some(id.clone()));
            self.editing.set(Some(id.clone()));
        }
        id
    }

    pub fn set_text(&self, id: &str, text: &str) {
        let text = text.trim();
        let changed = self.lock().node(id).is_some_and(|n| n.text != text);
        if changed {
            let t = text.to_string();
            self.edit(|d| {
                if let Some(n) = d.node_mut(id) {
                    n.text = t;
                }
            });
        }
    }

    pub fn set_note(&self, id: &str, note: &str) {
        let changed = self.lock().node(id).is_some_and(|n| n.note != note);
        if changed {
            let t = note.to_string();
            self.edit_data(|d| {
                if let Some(n) = d.node_mut(id) {
                    n.note = t;
                }
            });
        }
    }

    pub fn set_link(&self, id: &str, page: Option<String>) {
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.link = page.filter(|p| !p.is_empty());
            }
        });
    }

    pub fn set_color(&self, id: &str, color: Option<String>) {
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.color = color.unwrap_or_default();
            }
        });
    }

    pub fn set_shape(&self, id: &str, shape: NodeShape) {
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.shape = shape;
            }
        });
    }

    pub fn set_icon(&self, id: &str, icon: &str) {
        let icon = icon.trim().to_string();
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.icon = icon;
            }
        });
    }

    pub fn set_collapsed(&self, id: &str, collapsed: bool) {
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.collapsed = collapsed;
            }
        });
    }

    pub fn toggle_collapsed(&self, id: &str) {
        let cur = self.lock().node(id).map(|n| n.collapsed).unwrap_or(false);
        let has_children = !self.lock().children_of(id).is_empty();
        if has_children || cur {
            self.set_collapsed(id, !cur);
        }
    }

    /// Ручной сдвиг поддерева (прибавляется к текущему).
    pub fn offset(&self, id: &str, dx: f32, dy: f32) {
        if dx.abs() < 0.5 && dy.abs() < 0.5 {
            return;
        }
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.dx += dx;
                n.dy += dy;
            }
        });
    }

    pub fn reset_offset(&self, id: &str) {
        self.edit(|d| {
            if let Some(n) = d.node_mut(id) {
                n.dx = 0.0;
                n.dy = 0.0;
            }
        });
    }

    pub fn reset_offsets(&self) {
        self.edit(|d| d.reset_offsets());
    }

    pub fn reparent(&self, id: &str, parent: &str, index: Option<usize>) -> bool {
        let mut ok = false;
        self.edit(|d| {
            ok = d.move_node(id, parent, index);
            if ok {
                if let Some(n) = d.node_mut(id) {
                    n.dx = 0.0;
                    n.dy = 0.0;
                }
                if let Some(p) = d.node_mut(parent) {
                    p.collapsed = false;
                }
            }
        });
        ok
    }

    /// Удалить поддерево; выбор переходит на родителя.
    pub fn delete(&self, id: &str) -> usize {
        let parent = self.lock().node(id).and_then(|n| n.parent.clone());
        let mut n = 0;
        self.edit(|d| n = d.remove_subtree(id));
        if n > 0 {
            self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            if self.editing.get_untracked().as_deref() == Some(id) {
                self.editing.set(None);
            }
            self.select(parent);
        }
        n
    }

    pub fn add_link(&self, from: &str, to: &str, label: &str) -> bool {
        let mut ok = false;
        self.edit(|d| ok = d.add_link(from, to, label));
        ok
    }

    pub fn delete_link(&self, from: &str, to: &str) -> bool {
        let mut ok = false;
        self.edit(|d| ok = d.remove_link(from, to));
        ok
    }

    // ─── Раскладка и оформление ───────────────────────────────────────────

    pub fn layout(&self) -> MapLayout {
        self.lock().layout.clone()
    }

    pub fn set_layout(&self, f: impl FnOnce(&mut MapLayout)) {
        let mut l = self.layout();
        f(&mut l);
        l.h_gap = l.h_gap.clamp(8.0, 400.0).round();
        l.v_gap = l.v_gap.clamp(0.0, 200.0).round();
        if self.lock().layout == l {
            return;
        }
        self.edit(|d| d.layout = l);
    }

    pub fn set_direction(&self, direction: Direction) {
        self.set_layout(|l| l.direction = direction);
    }

    pub fn set_curve(&self, curve: Curve) {
        self.set_layout(|l| l.curve = curve);
    }

    pub fn style(&self) -> MindmapStyle {
        self.lock().style.clone()
    }

    pub fn set_style(&self, f: impl FnOnce(&mut MindmapStyle)) {
        let mut s = self.style();
        f(&mut s);
        s.sanitize();
        if self.lock().style == s {
            return;
        }
        self.edit(|d| d.style = s);
    }

    // ─── Правка на месте ──────────────────────────────────────────────────

    pub fn start_editing(&self, id: &str) {
        self.select(Some(id.to_string()));
        self.editing.set(Some(id.to_string()));
    }

    /// Закончить правку: узел с пустым текстом (кроме корня) выбрасывается.
    pub fn finish_editing(&self, id: &str, text: Option<&str>) {
        if let Some(t) = text {
            self.set_text(id, t);
        }
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        let empty = self.lock().node(id).is_some_and(|n| n.text.trim().is_empty() && n.parent.is_some());
        if empty {
            self.delete(id);
        }
    }

    pub fn fit(&self) {
        self.fit_request.set(self.fit_request.get_untracked() + 1);
    }

    /// Редактор заметки узла (панель свойств): ручка и исходник на момент
    /// начала правки; правки стекают в документ эффектом по ревизии.
    pub fn note_editor(&self, id: &str) -> (DocumentEditorHandle, Arc<String>) {
        let mut editors = self.editors.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = editors.get(id) {
            return e.clone();
        }
        let source = Arc::new(self.lock().node(id).map(|n| n.note.clone()).unwrap_or_default());
        let editor = DocumentEditorHandle::new();
        editors.insert(id.to_string(), (editor.clone(), source.clone()));
        drop(editors);
        let h = self.clone();
        let node_id = id.to_string();
        let e = editor.clone();
        create_effect(move || {
            if e.revision().get() == 0 {
                return;
            }
            h.set_note(&node_id, &e.serialize());
        });
        (editor, source)
    }

    /// Забыть редактор заметки (узел удалён или выбор ушёл).
    pub fn drop_note_editor(&self, id: &str) {
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
}
