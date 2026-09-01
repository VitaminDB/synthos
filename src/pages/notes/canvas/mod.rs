//! Канвасы заметок: тонкий слепок node_editor без портов и registry.
//!
//! [`CanvasHandle`] держит документ (Mutex) и runtime-сигналы: позиции и
//! размеры карточек (их пишут drag/resize-жесты), выбор, режим
//! редактирования, pending-ребро. Эффекты, заведённые при создании handle,
//! стекают изменения сигналов обратно в документ (bump revision →
//! автосейв), не трогая structure_rev — холст не перестраивается на
//! каждый пиксель перетаскивания.

pub mod card;
pub mod model;
pub mod pane;
pub mod wires;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use syngui::core::{Point, Size};
use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use model::{CanvasDoc, CanvasEdge, CanvasNode};

#[derive(Clone)]
pub struct CanvasHandle {
    doc: Arc<Mutex<CanvasDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    pub pan: RwSignal<Point>,
    pub zoom: RwSignal<f32>,
    pub selected: RwSignal<Option<String>>,
    pub selected_edge: RwSignal<Option<String>>,
    /// Карточка в режиме редактирования текста.
    pub editing: RwSignal<Option<String>>,
    /// Тянущееся ребро: (карточка-источник, курсор в world-координатах).
    pub pending_wire: RwSignal<Option<(String, Point)>>,
    runtime: Arc<Mutex<RuntimeMaps>>,
}

#[derive(Default)]
struct RuntimeMaps {
    pos: HashMap<String, RwSignal<Point>>,
    size: HashMap<String, RwSignal<Size>>,
    editors: HashMap<String, DocumentEditorHandle>,
}

impl CanvasHandle {
    pub fn new(doc: CanvasDoc) -> Self {
        let camera = doc.camera;
        let handle = Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            pan: use_signal(Point::new(camera.pan_x, camera.pan_y)),
            zoom: use_signal(camera.zoom.clamp(0.25, 4.0)),
            selected: use_signal(None),
            selected_edge: use_signal(None),
            editing: use_signal(None),
            pending_wire: use_signal(None),
            runtime: Arc::new(Mutex::new(RuntimeMaps::default())),
        };
        // Камера → документ (автосейв подхватит по ревизии).
        let h = handle.clone();
        create_effect(move || {
            let pan = h.pan.get();
            let zoom = h.zoom.get();
            let mut doc = h.lock();
            let cam = model::Camera { pan_x: pan.x, pan_y: pan.y, zoom };
            if doc.camera != cam {
                doc.camera = cam;
                drop(doc);
                h.bump();
            }
        });
        handle
    }

    pub fn lock(&self) -> MutexGuard<'_, CanvasDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    fn bump(&self) {
        self.revision.set(self.revision.get_untracked() + 1);
    }

    fn bump_structural(&self) {
        self.bump();
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    /// Перечитка с диска: документ заменяется, runtime-сигналы сбрасываются.
    pub fn replace(&self, doc: CanvasDoc) {
        self.pan.set(Point::new(doc.camera.pan_x, doc.camera.pan_y));
        self.zoom.set(doc.camera.zoom.clamp(0.25, 4.0));
        *self.lock() = doc;
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.pos.clear();
        rt.size.clear();
        rt.editors.clear();
        drop(rt);
        self.selected.set(None);
        self.editing.set(None);
        self.bump_structural();
    }

    /// Позиция карточки (runtime-сигнал; эффект стекает её в документ).
    pub fn node_pos(&self, id: &str) -> RwSignal<Point> {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sig) = rt.pos.get(id) {
            return *sig;
        }
        let node = self.lock().nodes.iter().find(|n| n.id == id).cloned();
        let p = node.map(|n| Point::new(n.x, n.y)).unwrap_or_default();
        let sig = use_signal(p);
        rt.pos.insert(id.to_string(), sig);
        drop(rt);
        let h = self.clone();
        let node_id = id.to_string();
        create_effect(move || {
            let p = sig.get();
            let mut doc = h.lock();
            if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == node_id) {
                if (n.x - p.x).abs() > 0.01 || (n.y - p.y).abs() > 0.01 {
                    n.x = p.x;
                    n.y = p.y;
                    drop(doc);
                    h.bump();
                }
            }
        });
        sig
    }

    /// Размер карточки (resize-ручка пишет сигнал).
    pub fn node_size(&self, id: &str) -> RwSignal<Size> {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sig) = rt.size.get(id) {
            return *sig;
        }
        let node = self.lock().nodes.iter().find(|n| n.id == id).cloned();
        let s = node.map(|n| Size::new(n.w, n.h)).unwrap_or(Size::new(260.0, 140.0));
        let sig = use_signal(s);
        rt.size.insert(id.to_string(), sig);
        drop(rt);
        let h = self.clone();
        let node_id = id.to_string();
        create_effect(move || {
            let s = sig.get();
            let mut doc = h.lock();
            if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == node_id) {
                if (n.w - s.width).abs() > 0.01 || (n.h - s.height).abs() > 0.01 {
                    n.w = s.width;
                    n.h = s.height;
                    drop(doc);
                    h.bump();
                }
            }
        });
        sig
    }

    /// Редактор текста карточки; правки стекают в `md` документа.
    pub fn node_editor(&self, id: &str) -> DocumentEditorHandle {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = rt.editors.get(id) {
            return h.clone();
        }
        let editor = DocumentEditorHandle::new();
        rt.editors.insert(id.to_string(), editor.clone());
        drop(rt);
        let h = self.clone();
        let node_id = id.to_string();
        let e = editor.clone();
        create_effect(move || {
            let rev = e.revision().get();
            if rev == 0 {
                return;
            }
            let md = e.serialize();
            let mut doc = h.lock();
            if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == node_id) {
                if n.md != md {
                    n.md = md;
                    drop(doc);
                    h.bump();
                }
            }
        });
        editor
    }

    // ─── Операции ───────────────────────────────────────────────────────────

    pub fn add_node(&self, world: Point) -> String {
        let id = super::base::model::new_id("n");
        self.lock().nodes.push(CanvasNode {
            id: id.clone(),
            x: world.x,
            y: world.y,
            w: 260.0,
            h: 150.0,
            color: String::new(),
            md: String::new(),
        });
        self.bump_structural();
        self.selected.set(Some(id.clone()));
        self.editing.set(Some(id.clone()));
        id
    }

    pub fn delete_node(&self, id: &str) {
        {
            let mut doc = self.lock();
            doc.nodes.retain(|n| n.id != id);
            doc.edges.retain(|e| e.from != id && e.to != id);
        }
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.pos.remove(id);
        rt.size.remove(id);
        rt.editors.remove(id);
        drop(rt);
        if self.selected.get_untracked().as_deref() == Some(id) {
            self.selected.set(None);
        }
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        self.bump_structural();
    }

    pub fn add_edge(&self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let mut doc = self.lock();
        let exists = doc.edges.iter().any(|e| e.from == from && e.to == to);
        if exists {
            return;
        }
        doc.edges.push(CanvasEdge {
            id: super::base::model::new_id("e"),
            from: from.to_string(),
            to: to.to_string(),
            label: String::new(),
        });
        drop(doc);
        self.bump_structural();
    }

    pub fn delete_edge(&self, id: &str) {
        self.lock().edges.retain(|e| e.id != id);
        if self.selected_edge.get_untracked().as_deref() == Some(id) {
            self.selected_edge.set(None);
        }
        self.bump_structural();
    }

    /// Карточка под точкой (world) — для завершения pending-ребра.
    pub fn node_at(&self, world: Point) -> Option<String> {
        let doc = self.lock();
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        for n in doc.nodes.iter().rev() {
            let pos = rt.pos.get(&n.id).map(|s| s.get_untracked()).unwrap_or(Point::new(n.x, n.y));
            let size = rt
                .size
                .get(&n.id)
                .map(|s| s.get_untracked())
                .unwrap_or(Size::new(n.w, n.h));
            if world.x >= pos.x
                && world.x <= pos.x + size.width
                && world.y >= pos.y
                && world.y <= pos.y + size.height
            {
                return Some(n.id.clone());
            }
        }
        None
    }

    /// Геометрия карточки (для проводов).
    pub fn node_rect(&self, id: &str) -> Option<(Point, Size)> {
        let doc = self.lock();
        let n = doc.nodes.iter().find(|n| n.id == id)?;
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let pos = rt.pos.get(id).map(|s| s.get_untracked()).unwrap_or(Point::new(n.x, n.y));
        let size = rt
            .size
            .get(id)
            .map(|s| s.get_untracked())
            .unwrap_or(Size::new(n.w, n.h));
        Some((pos, size))
    }
}
