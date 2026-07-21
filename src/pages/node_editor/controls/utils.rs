use std::any::Any;

use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{
    DirtyFlags, Element, ElementId, ElementTree, LayoutHint, UpdateContext,
};

pub fn idx_in(arr: &[&str], s: &str) -> Option<usize> {
    arr.iter().position(|x| *x == s)
}

pub struct WrapPadded(pub Box<dyn Widget>, pub f32, pub f32);

impl Widget for WrapPadded {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(WrapPaddedElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            l: self.1,
            r: self.1,
            t: self.2,
            b: self.2,
            child_id: None,
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
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
        let child_element = self.0.create_element();
        let _ = tree.insert_with_type_id(child_element, Some(parent_id), self.0.as_any().type_id());
        self.0.mount(tree, parent_id);
    }

    fn child_widgets(&self) -> Vec<&dyn Widget> {
        vec![self.0.as_ref()]
    }
}

struct WrapPaddedElement {
    id: ElementId,
    bounds: Rect,
    l: f32,
    r: f32,
    t: f32,
    b: f32,
    child_id: Option<ElementId>,
    dirty: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
}

impl Element for WrapPaddedElement {
    fn update(&mut self, _w: &dyn Widget, _ctx: &mut UpdateContext) {}
    fn layout(&mut self, c: Constraints) -> Size {
        let w = if c.max_width.is_finite() { c.max_width } else { 0.0 };
        let h = if c.max_height.is_finite() { c.max_height } else { 0.0 };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
        Size::new(w, h)
    }
    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding {
            left: self.l,
            top: self.t,
            right: self.r,
            bottom: self.b,
        }
    }
    fn build_display_list(&self, _l: &mut DisplayList, _c: Rect) {}
    fn handle_event(&mut self, _e: &Event, _c: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }
    fn passthrough_hit_test(&self) -> bool {
        true
    }
    fn children(&self) -> &[ElementId] {
        static EMPTY: &[ElementId] = &[];
        match self.child_id {
            Some(ref i) => std::slice::from_ref(i),
            None => EMPTY,
        }
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, p: Point) {
        self.bounds.origin = p;
    }
    fn mark_dirty(&mut self, f: DirtyFlags) {
        self.dirty |= f;
    }
    fn clear_dirty(&mut self, f: DirtyFlags) {
        self.dirty.remove(f);
    }
    fn is_dirty(&self, f: DirtyFlags) -> bool {
        self.dirty.contains(f)
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, i: ElementId) {
        self.id = i;
    }
    fn mount(&mut self, _t: &mut ElementTree) {}
    fn element_type_name(&self) -> &str {
        "WrapPadded"
    }
    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, s: &ComputedStyle) {
        self.mss.apply(s);
    }
}
