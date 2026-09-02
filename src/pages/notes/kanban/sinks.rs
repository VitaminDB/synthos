//! Приёмники дропа для блоков страницы: карточки и хвосты колонок досок
//! публикуют свои прямоугольники, редактор на отпускании блока после
//! переноса за ⋮⋮ спрашивает хост — и блок уезжает в карточку.
//!
//! Почему не drag-подсистема syngui: перенос блока — внутренний жест
//! редактора (в свободной раскладке блок едет живьём), а `DragEnd`
//! дерева уходит элементу в фокусе, не источнику. Прямоугольники —
//! глобальный реестр по ключу; свежесть — по номеру публикации: видимые
//! приёмники переиздаются на каждой раскладке и перекрывают устаревшие
//! записи удалённых карточек. Дроп ищется только среди досок активной
//! страницы.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use syngui::core::{Point, Rect, Size};
use syngui::input::{DragData, Event, EventResult};
use syngui::layout::Constraints;
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, LayoutHint};
use syngui::widgets::input::document_editor::{BlockId, DocOp};

use super::super::state::{object_refs, LiveObject, NotesCtx};
use super::view::DRAG_TYPE_CARD;
use super::KanbanHandle;

#[derive(Clone, Debug, PartialEq)]
pub enum Sink {
    Card { board: String, card: String },
    Tail { board: String, column: String },
}

impl Sink {
    fn board(&self) -> &str {
        match self {
            Sink::Card { board, .. } | Sink::Tail { board, .. } => board,
        }
    }

    fn key(&self) -> String {
        match self {
            Sink::Card { board, card } => format!("{board}|card|{card}"),
            Sink::Tail { board, column } => format!("{board}|tail|{column}"),
        }
    }
}

struct Entry {
    sink: Sink,
    rect: Rect,
    seq: u64,
}

fn registry() -> &'static Mutex<(HashMap<String, Entry>, u64)> {
    static R: OnceLock<Mutex<(HashMap<String, Entry>, u64)>> = OnceLock::new();
    R.get_or_init(|| Mutex::new((HashMap::new(), 0)))
}

fn publish(sink: &Sink, rect: Rect) {
    let mut g = registry().lock().unwrap_or_else(|e| e.into_inner());
    g.1 += 1;
    let seq = g.1;
    g.0.insert(sink.key(), Entry { sink: sink.clone(), rect, seq });
}

/// Приёмник под точкой среди досок `boards`: карточка предпочтительнее
/// хвоста, из равных — опубликованный позже.
pub fn sink_at(pos: Point, boards: &[String]) -> Option<Sink> {
    let g = registry().lock().unwrap_or_else(|e| e.into_inner());
    g.0.values()
        .filter(|e| e.rect.contains(pos) && boards.iter().any(|b| b == e.sink.board()))
        .max_by_key(|e| (matches!(e.sink, Sink::Card { .. }), e.seq))
        .map(|e| e.sink.clone())
}

/// Доски, врезанные в активную страницу.
fn active_boards(ctx: NotesCtx) -> Vec<String> {
    let Some(page) = ctx.active_page() else { return Vec::new() };
    object_refs(&page.markdown())
        .into_iter()
        .filter(|(kind, _)| kind == "kanban")
        .map(|(_, id)| id)
        .collect()
}

fn board(ctx: NotesCtx, id: &str) -> Option<KanbanHandle> {
    match ctx.object("kanban", id)? {
        LiveObject::Kanban { handle, .. } => Some(handle),
        _ => None,
    }
}

/// Блок страницы отпущен в точке: если под ней карточка — блок уходит в
/// её содержимое, если хвост колонки — становится новой карточкой.
pub fn take_block(ctx: NotesCtx, pos: Point, block: BlockId) -> bool {
    let sink = match sink_at(pos, &active_boards(ctx)) {
        Some(s) => s,
        None => return false,
    };
    let Some(page) = ctx.active_page() else { return false };
    let Some(md) = page.handle.block_markdown(block) else { return false };
    let Some(handle) = board(ctx, sink.board()) else { return false };
    match sink {
        Sink::Card { card, .. } => handle.append_card_md(&card, &md),
        Sink::Tail { column, .. } => handle.add_card_from_md(&column, &md).is_some(),
    }
}

/// Карточка доски отпущена на документ (мимо карточек и хвостов): она
/// становится блоками страницы в точке дропа и уходит с доски.
pub fn drop_on_page(ctx: NotesCtx, pos: Point, data: &DragData) -> bool {
    if data.drag_type != DRAG_TYPE_CARD {
        return false;
    }
    let boards = active_boards(ctx);
    if sink_at(pos, &boards).is_some() {
        return false;
    }
    let Some((board_id, card_id)) = data.payload.split_once('|') else { return false };
    let Some(handle) = board(ctx, board_id) else { return false };
    let Some(card) = handle.take_card(card_id) else { return false };
    ctx.doc_op(DocOp::InsertMarkdownAt { at: pos, md: KanbanHandle::card_markdown(&card) });
    true
}

// ─── RectProbe: обёртка, публикующая прямоугольник ребёнка ──────────────

pub struct RectProbe {
    sink: Sink,
    child: Box<dyn Widget>,
}

impl RectProbe {
    pub fn new(sink: Sink, child: Box<dyn Widget>) -> Self {
        Self { sink, child }
    }
}

impl Widget for RectProbe {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RectProbeElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            sink: self.sink.clone(),
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
        let element = self.child.create_element();
        let child_id = tree.insert_with_type_id(element, Some(parent_id), self.child.as_any().type_id());
        self.child.mount(tree, child_id);
    }
    fn child_widgets(&self) -> Vec<&dyn Widget> {
        vec![self.child.as_ref()]
    }
}

struct RectProbeElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    sink: Sink,
}

impl Element for RectProbeElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<RectProbe>() {
            self.sink = w.sink.clone();
        }
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        // С ребёнком дерево зовёт layout tight-размером — его и берём.
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 0.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 0.0 };
        self.bounds.size = Size::new(w, h);
        self.bounds.size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Stack { expand: false }
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, _event: &Event, _ctx: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        false
    }
    fn element_type_name(&self) -> &str {
        "notes-drop-sink"
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
        // Размер приходит после позиции — публикуем прямоугольник целиком.
        self.bounds.size = size;
        publish(&self.sink, self.bounds);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_publication_wins_and_cards_beat_tails() {
        let b = "b1".to_string();
        let tail = Sink::Tail { board: b.clone(), column: "c".into() };
        let card = Sink::Card { board: b.clone(), card: "k".into() };
        let r = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        publish(&tail, r);
        publish(&card, r);
        assert_eq!(sink_at(Point::new(10.0, 10.0), &[b.clone()]), Some(card.clone()));
        assert_eq!(sink_at(Point::new(10.0, 10.0), &["другая".into()]), None);
        // Карточка «уехала» — хвост снова сверху.
        publish(&card, Rect::new(Point::new(500.0, 500.0), Size::new(10.0, 10.0)));
        assert_eq!(sink_at(Point::new(10.0, 10.0), &[b]), Some(tail));
    }
}
