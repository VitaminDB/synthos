//! Карточка канваса: перетаскиваемая плашка с markdown-содержимым.
//!
//! Не в режиме редактирования — MarkdownView (read-only, события не
//! перехватывает) + клик выделяет / повторный клик включает редактор.
//! В режиме редактирования — DocumentEditor с общей ручкой карточки
//! (правки стекают в `md` документа эффектом в CanvasHandle).
//! У выделенной карточки: ручка ресайза в правом нижнем углу и точка
//! протяжки ребра справа.

use std::any::Any;
use std::time::Duration;

use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};
use syngui::widgets::input::document_editor::DocumentEditor;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::GestureDetector;
use syngui::containers::Positioned;

use super::CanvasHandle;

pub fn view(handle: CanvasHandle, node_id: String) -> impl Widget {
    let pos = handle.node_pos(&node_id);
    let card_handle = handle.clone();
    let card_id = node_id.clone();
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![build_card(card_handle.clone(), card_id.clone())]
    });
    Positioned::new(Stack::new().clip(false).children(vec![Box::new(body) as Box<dyn Widget>]))
        .offset_signal(pos)
}

fn build_card(handle: CanvasHandle, node_id: String) -> Box<dyn Widget> {
    let size = handle.node_size(&node_id).get();
    let selected = handle.selected.get().as_deref() == Some(node_id.as_str());
    let editing = handle.editing.get().as_deref() == Some(node_id.as_str());
    let (md, color) = {
        let doc = handle.lock();
        doc.nodes
            .iter()
            .find(|n| n.id == node_id)
            .map(|n| (n.md.clone(), n.color.clone()))
            .unwrap_or_default()
    };

    let content: Box<dyn Widget> = if editing {
        let editor_handle = handle.node_editor(&node_id);
        Box::new(
            DocumentEditor::new()
                .markdown(md.clone())
                .handle(&editor_handle)
                .class("notes-canvas-editor"),
        )
    } else {
        Box::new(
            MarkdownView::new(md.clone())
                .selectable(false)
                .max_width(size.width - 24.0),
        )
    };

    let mut chrome_class = String::from("notes-canvas-card");
    if selected {
        chrome_class.push_str(" selected");
    }
    let mut chrome = DecoratedBox::new().class(chrome_class).child(
        syngui::widgets::Padding::all(10.0).clip(true).child(
            crate::components::workspace_frame::expand(content),
        ),
    );
    if !color.is_empty() {
        chrome = chrome.style("border-color", Color::from_hex(&color));
    }
    chrome = chrome
        .style("width", syngui::mss::StyleValue::px(size.width))
        .style("height", syngui::mss::StyleValue::px(size.height));

    // Клик: выделить; по выделенной — редактировать.
    let h_click = handle.clone();
    let id_click = node_id.clone();
    let clickable: Box<dyn Widget> = if editing {
        Box::new(chrome)
    } else {
        Box::new(
            CardGrab {
                handle: handle.clone(),
                node_id: node_id.clone(),
                child: Some(Box::new(GestureDetector::new().on_click(move || {
                    if h_click.selected.get_untracked().as_deref() == Some(id_click.as_str()) {
                        h_click.editing.set(Some(id_click.clone()));
                    } else {
                        h_click.selected.set(Some(id_click.clone()));
                        h_click.selected_edge.set(None);
                    }
                }).child(chrome))),
            },
        )
    };

    let mut stack = Stack::new().clip(false).children(vec![clickable]);
    if selected && !editing {
        // Ручка ресайза и точка ребра.
        stack = stack
            .child(
                Positioned::new(Grip::new(handle.clone(), node_id.clone(), GripKind::Resize))
                    .at(size.width - 14.0, size.height - 14.0),
            )
            .child(
                Positioned::new(Grip::new(handle.clone(), node_id.clone(), GripKind::Wire))
                    .at(size.width - 7.0, size.height / 2.0 - 7.0),
            );
    }
    Box::new(stack)
}

// ─── CardGrab: перетаскивание всей карточки ────────────────────────────────

struct CardGrab {
    handle: CanvasHandle,
    node_id: String,
    child: Option<Box<dyn Widget>>,
}

impl Widget for CardGrab {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(CardGrabElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            handle: self.handle.clone(),
            node_id: self.node_id.clone(),
            drag: None,
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        if let Some(child) = &self.child {
            let el = child.create_element();
            let child_id = tree.insert_with_type_id(el, Some(parent_id), child.as_any().type_id());
            child.mount(tree, child_id);
        }
    }
    fn child_widgets(&self) -> Vec<&dyn Widget> {
        self.child.as_ref().map(|c| vec![c.as_ref()]).unwrap_or_default()
    }
}

struct CardGrabElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    handle: CanvasHandle,
    node_id: String,
    /// (стартовый курсор, стартовая позиция карточки).
    drag: Option<(Point, Point)>,
}

impl Element for CardGrabElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<CardGrab>() {
            self.handle = w.handle.clone();
            self.node_id = w.node_id.clone();
        }
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 260.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 150.0 };
        self.bounds.size = Size::new(w, h);
        self.bounds.size
    }
    fn layout_hint(&self) -> syngui::widget::LayoutHint {
        syngui::widget::LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
    }
    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position }
                if self.bounds.contains(*position) =>
            {
                let pos = self.handle.node_pos(&self.node_id).get_untracked();
                self.drag = Some((*position, pos));
                ctx.set_cursor(CursorIcon::Grabbing);
                // Не съедаем событие: клик дойдёт до GestureDetector
                // (выделение), а drag начнётся с первого MouseMove.
                EventResult::Ignored
            }
            Event::MouseMove(p) => {
                if let Some((start, base)) = self.drag {
                    let moved = (p.x - start.x).abs() + (p.y - start.y).abs();
                    if moved > 3.0 {
                        self.handle
                            .node_pos(&self.node_id)
                            .set(Point::new(base.x + p.x - start.x, base.y + p.y - start.y));
                        ctx.set_cursor(CursorIcon::Grabbing);
                        return EventResult::Handled;
                    }
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, .. } if self.drag.is_some() => {
                self.drag = None;
                ctx.set_cursor(CursorIcon::Default);
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }
    fn element_type_name(&self) -> &str {
        "notes-canvas-grab"
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
}

// ─── Grip: ресайз и протяжка ребра ─────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum GripKind {
    Resize,
    Wire,
}

struct Grip {
    handle: CanvasHandle,
    node_id: String,
    kind: GripKind,
}

impl Grip {
    fn new(handle: CanvasHandle, node_id: String, kind: GripKind) -> Self {
        Self { handle, node_id, kind }
    }
}

impl Widget for Grip {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(GripElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            handle: self.handle.clone(),
            node_id: self.node_id.clone(),
            kind: self.kind,
            drag: None,
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct GripElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    handle: CanvasHandle,
    node_id: String,
    kind: GripKind,
    /// (стартовый курсор, стартовый размер карточки).
    drag: Option<(Point, Size)>,
}

impl Element for GripElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<Grip>() {
            self.handle = w.handle.clone();
            self.node_id = w.node_id.clone();
            self.kind = w.kind;
        }
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn layout(&mut self, _constraints: Constraints) -> Size {
        self.bounds.size = Size::new(14.0, 14.0);
        self.bounds.size
    }
    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let color = match self.kind {
            GripKind::Resize => Color::from_hex("#8b95a6"),
            GripKind::Wire => Color::from_hex("#EE5E48"),
        };
        list.push_rect(self.bounds, color, [7.0; 4]);
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position }
                if self.bounds.contains(*position) =>
            {
                let size = self.handle.node_size(&self.node_id).get_untracked();
                self.drag = Some((*position, size));
                if self.kind == GripKind::Wire {
                    self.handle.pending_wire.set(Some((self.node_id.clone(), *position)));
                }
                ctx.capture();
                EventResult::Handled
            }
            Event::MouseMove(p) => {
                let Some((start, base)) = self.drag else { return EventResult::Ignored };
                match self.kind {
                    GripKind::Resize => {
                        let w = (base.width + p.x - start.x).clamp(140.0, 900.0);
                        let h = (base.height + p.y - start.y).clamp(70.0, 900.0);
                        self.handle.node_size(&self.node_id).set(Size::new(w, h));
                    }
                    GripKind::Wire => {
                        self.handle.pending_wire.set(Some((self.node_id.clone(), *p)));
                    }
                }
                EventResult::Handled
            }
            Event::MouseUp { button: MouseButton::Left, position } if self.drag.is_some() => {
                self.drag = None;
                if self.kind == GripKind::Wire {
                    self.handle.pending_wire.set(None);
                    if let Some(target) = self.handle.node_at(*position) {
                        self.handle.add_edge(&self.node_id, &target);
                    }
                }
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }
    fn element_type_name(&self) -> &str {
        "notes-canvas-grip"
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
}
