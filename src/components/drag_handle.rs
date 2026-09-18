//! Обёртка «потяни, чтобы переставить»: ребёнок едет за курсором, пока зажата
//! левая кнопка, короткое нажатие без движения — обычный щелчок.
//!
//! Зачем свой элемент: у `GestureDetector` нет жеста переноса, а
//! `syngui::Draggable` — это DnD с призраком и целью-`DropArea`, кнопка при нём
//! стоит на месте до отпускания. Здесь ребёнок двигается вживую: обёртка
//! сообщает смещение от точки нажатия, а владелец сам решает, что с ним делать
//! (у голосового FAB — пересчитывает отступы от угла окна).
//!
//! Нажатие перехватывается раньше ребёнка (`intercepts_event`): иначе кнопка
//! внутри забрала бы `MouseDown`, стала захватчиком мыши, и движение сюда бы
//! не дошло. Поэтому щелчок отдаёт сама обёртка (`on_click`), а не ребёнок.
//! Наведение не перехватывается — подсказка и `:hover` ребёнка работают.
//! Тот, кто обработал `MouseDown`, получает `MouseMove`/`MouseUp` и за
//! пределами своих границ (`mouse_captor` в дереве syngui).

use std::any::Any;
use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::{Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{
    DirtyFlags, Element, ElementId, ElementTree, LayoutHint, StyledElement, UpdateContext, Widget,
};
use syngui::widgets::containers::IntoWidget;

/// Фаза переноса. `bounds` — границы обёртки в момент нажатия (координаты
/// окна), `delta` — смещение курсора от точки нажатия.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragPhase {
    Move { bounds: Rect, delta: Point },
    End,
}

type ClickFn = Arc<Mutex<dyn FnMut() + Send>>;
type DragFn = Arc<Mutex<dyn FnMut(DragPhase) + Send>>;

pub struct DragHandle {
    child: Option<Box<dyn Widget>>,
    threshold: f32,
    on_click: Option<ClickFn>,
    on_drag: Option<DragFn>,
}

impl DragHandle {
    pub fn new<M>(child: impl IntoWidget<M>) -> Self {
        Self { child: Some(child.into_widget()), threshold: 5.0, on_click: None, on_drag: None }
    }

    /// Нажатие без движения дальше порога.
    pub fn on_click(mut self, cb: impl FnMut() + Send + 'static) -> Self {
        self.on_click = Some(Arc::new(Mutex::new(cb)));
        self
    }

    pub fn on_drag(mut self, cb: impl FnMut(DragPhase) + Send + 'static) -> Self {
        self.on_drag = Some(Arc::new(Mutex::new(cb)));
        self
    }
}

impl Widget for DragHandle {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(DragHandleElement {
            id: ElementId::new(),
            threshold: self.threshold,
            on_click: self.on_click.clone(),
            on_drag: self.on_drag.clone(),
            bounds: Rect::zero(),
            press: None,
            dragging: false,
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
        self.child.as_ref().map(|c| vec![c.as_ref() as &dyn Widget]).unwrap_or_default()
    }
}

struct DragHandleElement {
    id: ElementId,
    threshold: f32,
    on_click: Option<ClickFn>,
    on_drag: Option<DragFn>,
    bounds: Rect,
    /// Точка нажатия и границы на тот момент. Живёт в элементе, а не в
    /// виджете: владелец на каждое смещение пересобирает дерево, виджет
    /// приходит новый, элемент (и захват мыши) остаётся прежним.
    press: Option<(Point, Rect)>,
    dragging: bool,
    classes: Vec<String>,
    dirty_flags: DirtyFlags,
    mss: MssFields,
}

impl DragHandleElement {
    fn emit(&self, phase: DragPhase) {
        if let Some(cb) = &self.on_drag {
            if let Ok(mut f) = cb.lock() {
                f(phase);
            }
        }
    }
}

impl Element for DragHandleElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(h) = widget.as_any().downcast_ref::<DragHandle>() {
            self.threshold = h.threshold;
            self.on_click = h.on_click.clone();
            self.on_drag = h.on_drag.clone();
        }
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 0.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 0.0 };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
        Size::new(w, h)
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position }
                if self.bounds.contains(*position) =>
            {
                self.press = Some((*position, self.bounds));
                self.dragging = false;
                EventResult::Handled
            }
            Event::MouseMove(pos) => {
                let Some((down, bounds)) = self.press else {
                    return EventResult::Ignored;
                };
                let delta = Point::new(pos.x - down.x, pos.y - down.y);
                if !self.dragging {
                    if (delta.x * delta.x + delta.y * delta.y).sqrt() <= self.threshold {
                        return EventResult::Ignored;
                    }
                    self.dragging = true;
                }
                ctx.set_cursor(CursorIcon::Grabbing);
                self.emit(DragPhase::Move { bounds, delta });
                EventResult::Handled
            }
            Event::MouseUp { .. } => {
                if self.press.take().is_none() {
                    return EventResult::Ignored;
                }
                if std::mem::take(&mut self.dragging) {
                    self.emit(DragPhase::End);
                } else if let Some(cb) = &self.on_click {
                    if let Ok(mut f) = cb.lock() {
                        f();
                    }
                }
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn intercepts_event(&self, event: &Event) -> bool {
        matches!(event, Event::MouseDown { button: MouseButton::Left, .. })
    }

    fn children(&self) -> &[ElementId] {
        &[]
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }

    fn set_content_size(&mut self, size: Size) {
        self.bounds = Rect::new(self.bounds.origin, size);
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
        "DragHandle"
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
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

    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }

    fn apply_computed_style(&mut self, style: &ComputedStyle) {
        self.mss.apply(style);
        self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
    }
}

impl StyledElement for DragHandleElement {
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
