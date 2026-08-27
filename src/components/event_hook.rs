use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use syngui::core::sync::Mutex;
use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult, Key, Modifiers};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{
    DirtyFlags, Element, ElementId, ElementTree, LayoutHint, StyledElement, UpdateContext, Widget,
};
use syngui::widgets::containers::IntoWidget;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyReply {
    Ignore,
    Handled,
    ScrollIntoView,
}

type KeyHandler = Arc<dyn Fn(Key, Modifiers) -> KeyReply + Send + Sync>;

/// Прозрачная обёртка: перехват клавиш и наблюдение за собственной геометрией.
///
/// Без ребёнка занимает нулевую высоту, с ребёнком отдаёт ему всё место.
/// Клавиши приходят сюда двумя путями: при открытом модальном оверлее дерево
/// обходит детей в обратном порядке, и обёртка, поставленная последним
/// ребёнком, видит нажатие раньше поля ввода; без оверлея события всплывают
/// от элемента с фокусом к предкам, и обёртка-предок получает всё, что поле
/// не поглотило.
pub struct EventHook {
    on_key_down: Option<KeyHandler>,
    on_key_up: Option<KeyHandler>,
    bounds_out: Option<Arc<Mutex<Rect>>>,
    child: Option<Box<dyn Widget>>,
}

impl EventHook {
    pub fn new() -> Self {
        Self {
            on_key_down: None,
            on_key_up: None,
            bounds_out: None,
            child: None,
        }
    }

    pub fn on_key_down(
        mut self,
        handler: impl Fn(Key, Modifiers) -> KeyReply + Send + Sync + 'static,
    ) -> Self {
        self.on_key_down = Some(Arc::new(handler));
        self
    }

    pub fn on_key_up(
        mut self,
        handler: impl Fn(Key, Modifiers) -> KeyReply + Send + Sync + 'static,
    ) -> Self {
        self.on_key_up = Some(Arc::new(handler));
        self
    }

    pub fn report_bounds(mut self, out: Arc<Mutex<Rect>>) -> Self {
        self.bounds_out = Some(out);
        self
    }

    pub fn child<M>(mut self, child: impl IntoWidget<M>) -> Self {
        self.child = Some(child.into_widget());
        self
    }
}

impl Default for EventHook {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for EventHook {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(EventHookElement {
            id: ElementId::new(),
            on_key_down: self.on_key_down.clone(),
            on_key_up: self.on_key_up.clone(),
            bounds_out: self.bounds_out.clone(),
            has_child: self.child.is_some(),
            bounds: Rect::zero(),
            classes: Vec::new(),
            dirty_flags: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
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
        if let Some(child) = &self.child {
            let element = child.create_element();
            let child_id =
                tree.insert_with_type_id(element, Some(parent_id), child.as_any().type_id());
            child.mount(tree, child_id);
        }
    }

    fn child_widgets(&self) -> Vec<&dyn Widget> {
        self.child
            .as_ref()
            .map(|c| vec![c.as_ref() as &dyn Widget])
            .unwrap_or_default()
    }
}

struct EventHookElement {
    id: ElementId,
    on_key_down: Option<KeyHandler>,
    on_key_up: Option<KeyHandler>,
    bounds_out: Option<Arc<Mutex<Rect>>>,
    has_child: bool,
    bounds: Rect,
    classes: Vec<String>,
    dirty_flags: DirtyFlags,
    mss: MssFields,
}

impl EventHookElement {
    fn publish_bounds(&self) {
        if let Some(out) = &self.bounds_out {
            if let Ok(mut rect) = out.lock() {
                *rect = self.bounds;
            }
        }
    }
}

impl Element for EventHookElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(hook) = widget.as_any().downcast_ref::<EventHook>() {
            self.on_key_down = hook.on_key_down.clone();
            self.on_key_up = hook.on_key_up.clone();
            self.bounds_out = hook.bounds_out.clone();
            self.has_child = hook.child.is_some();
            self.publish_bounds();
        }
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        if !self.has_child {
            self.bounds = Rect::new(self.bounds.origin, Size::zero());
            return Size::zero();
        }
        let w = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            0.0
        };
        let h = if constraints.max_height.is_finite() {
            constraints.max_height
        } else {
            0.0
        };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
        Size::new(w, h)
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        let (key, handler) = match event {
            Event::KeyDown(key) => (*key, self.on_key_down.as_ref()),
            Event::KeyUp(key) => (*key, self.on_key_up.as_ref()),
            _ => return EventResult::Ignored,
        };
        let Some(handler) = handler else {
            return EventResult::Ignored;
        };
        match handler(key, ctx.modifiers) {
            KeyReply::Ignore => EventResult::Ignored,
            KeyReply::Handled => EventResult::Handled,
            KeyReply::ScrollIntoView => {
                ctx.scroll_into_view(self.bounds);
                EventResult::Handled
            }
        }
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }

    fn needs_repaint(&self) -> bool {
        false
    }

    fn children(&self) -> &[ElementId] {
        &[]
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
        self.publish_bounds();
    }

    fn set_content_size(&mut self, size: Size) {
        self.bounds = Rect::new(self.bounds.origin, size);
        self.publish_bounds();
    }

    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty_flags |= flags;
    }

    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty_flags.remove(flags);
    }

    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty_flags.contains(flags)
    }

    fn id(&self) -> ElementId {
        self.id
    }

    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn element_type_name(&self) -> &str {
        "EventHook"
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        }
    }

    fn passthrough_hit_test(&self) -> bool {
        false
    }

    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
        self.mark_dirty(DirtyFlags::RENDER);
    }

    fn get_classes(&self) -> &[String] {
        &self.classes
    }

    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }

    fn apply_computed_style(&mut self, style: &ComputedStyle) {
        self.mss.apply(style);
        self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
    }

    fn apply_transition_styles(
        &mut self,
        base: &ComputedStyle,
        hover: Option<&ComputedStyle>,
        active: Option<&ComputedStyle>,
        focus: Option<&ComputedStyle>,
        selected: Option<&ComputedStyle>,
        _checked: Option<&ComputedStyle>,
    ) {
        self.mss
            .apply_transitions(base, hover, active, focus, selected);
    }
}

impl StyledElement for EventHookElement {
    fn apply_style(&mut self, _style: &ComputedStyle) {
        self.mark_dirty(DirtyFlags::RENDER);
    }

    fn classes(&self) -> &[String] {
        &self.classes
    }

    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
        self.mark_dirty(DirtyFlags::RENDER);
    }
}
