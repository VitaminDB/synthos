use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, UpdateContext};

pub trait ProgressTicker: Send + Sync + 'static {
    fn tick(&self, dt: Duration) -> bool;
}

pub fn node_progress_animator(ticker: Arc<dyn ProgressTicker>) -> Box<dyn Widget> {
    Box::new(ProgressAnimatorWidget { ticker })
}

struct ProgressAnimatorWidget {
    ticker: Arc<dyn ProgressTicker>,
}

impl Widget for ProgressAnimatorWidget {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ProgressAnimatorElement {
            id: ElementId::new(),
            ticker: self.ticker.clone(),
            bounds: Rect::zero(),
            classes: Vec::new(),
            dirty: DirtyFlags::LAYOUT,
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
    fn mount(&self, _t: &mut ElementTree, _p: ElementId) {}
}

struct ProgressAnimatorElement {
    id: ElementId,
    ticker: Arc<dyn ProgressTicker>,
    bounds: Rect,
    classes: Vec<String>,
    dirty: DirtyFlags,
    mss: MssFields,
}

impl Element for ProgressAnimatorElement {
    fn update(&mut self, w: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(a) = w.as_any().downcast_ref::<ProgressAnimatorWidget>() {
            self.ticker = a.ticker.clone();
        }
    }
    fn layout(&mut self, _c: Constraints) -> Size {
        self.bounds = Rect::new(self.bounds.origin, Size::new(0.0, 0.0));
        Size::new(0.0, 0.0)
    }
    fn build_display_list(&self, _l: &mut DisplayList, _c: Rect) {}
    fn handle_event(&mut self, _e: &Event, _c: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }
    fn animate(&mut self, dt: Duration) -> bool {
        self.ticker.tick(dt)
    }
    fn children(&self) -> &[ElementId] {
        &[]
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
        "ProgressAnimator"
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
    fn passthrough_hit_test(&self) -> bool {
        true
    }
}
