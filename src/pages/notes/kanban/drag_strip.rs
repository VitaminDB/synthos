//! Узкая полоса-хваталка у правой кромки колонки: тянет её ширину.
//!
//! Свой элемент, потому что `GestureDetector` знает только клики. Сообщает
//! приращение по X на каждый сдвиг мыши (не суммарное): доска
//! перестраивается на каждый шаг, и замыкание берёт текущую ширину из
//! документа, а не из снимка на момент захвата.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use syngui::core::{Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree};

pub const STRIP_W: f32 = 6.0;

pub struct DragStrip {
    on_drag: Arc<dyn Fn(f32) + Send + Sync>,
}

impl DragStrip {
    pub fn new(on_drag: impl Fn(f32) + Send + Sync + 'static) -> Self {
        Self { on_drag: Arc::new(on_drag) }
    }
}

impl Widget for DragStrip {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(DragStripElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT,
            on_drag: self.on_drag.clone(),
            last_x: None,
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

struct DragStripElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    on_drag: Arc<dyn Fn(f32) + Send + Sync>,
    /// X последнего сдвига, пока тянем.
    last_x: Option<f32>,
}

impl Element for DragStripElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<DragStrip>() {
            self.on_drag = w.on_drag.clone();
        }
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 24.0 };
        self.bounds.size = Size::new(STRIP_W, h);
        self.bounds.size
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } if self.bounds.contains(*position) => {
                self.last_x = Some(position.x);
                ctx.capture();
                ctx.set_cursor(CursorIcon::ColResize);
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                if let Some(last) = self.last_x {
                    let dx = position.x - last;
                    if dx.abs() >= 1.0 {
                        self.last_x = Some(position.x);
                        (self.on_drag)(dx);
                    }
                    ctx.set_cursor(CursorIcon::ColResize);
                    return EventResult::Handled;
                }
                if self.bounds.contains(*position) {
                    ctx.set_cursor(CursorIcon::ColResize);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, .. } if self.last_x.is_some() => {
                self.last_x = None;
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }
    fn element_type_name(&self) -> &str {
        "notes-kanban-resize"
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
    fn set_content_size(&mut self, size: Size) {
        self.bounds.size = size;
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
