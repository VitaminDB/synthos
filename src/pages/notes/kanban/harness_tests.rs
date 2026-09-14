//! Headless-тесты доски на `TestHarness` syngui: доска живёт во врезке
//! редактора страницы (свободная раскладка), как в приложении, а цепочка
//! drag-and-drop повторяет ту, что делает `AppHandler` (MouseDown →
//! MouseMove → DragMove → Drop → DragEnd). Запуск:
//! `cargo test --features testing kanban::harness_tests`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, Key, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::testing::TestHarness;
use syngui::widget::context::TextMeasure;
use syngui::widget::ElementId;
use syngui::widgets::input::document_editor::{
    BlockId, DocLayout, DocOp, DocumentEditor, DocumentEditorHandle, EmbedCtx, EmbedFactory,
};

use super::clip::CopyKind;
use super::model::{DropSpot, KanbanCard, KanbanDoc};
use super::sinks::DRAG_TYPE_BLOCK;
use super::view::view;
use super::{BoardEnv, KanbanHandle};

struct Mono;
impl TextMeasure for Mono {
    fn measure_text_width(&self, _t: &str, _fs: f32, chars: usize) -> f32 {
        chars as f32 * 10.0
    }
    fn hit_test_char(&self, text: &str, _fs: f32, x: f32) -> usize {
        ((x / 10.0).round() as usize).min(text.chars().count())
    }
}

struct Factory {
    handles: HashMap<String, KanbanHandle>,
    page: DocumentEditorHandle,
}

thread_local! {
    /// Что доска клала в буфер: `(карточка, вид)`.
    static COPIED: RefCell<Vec<(String, CopyKind)>> = const { RefCell::new(Vec::new()) };
    /// Что отдаст «буфер» при вставке.
    static PASTE: RefCell<Option<KanbanCard>> = const { RefCell::new(None) };
}

impl Factory {
    fn env(&self) -> BoardEnv {
        let map = self.handles.clone();
        let page = self.page.clone();
        let mut env = BoardEnv::detached(
            Arc::new(move |id| map.get(id).cloned()),
            // Как `sinks::take_page_block`: markdown блока + удаление.
            Arc::new(move |payload| {
                let id = BlockId(payload.parse().ok()?);
                let md = page.block_markdown(id)?;
                page.queue_op(DocOp::DeleteBlock(id));
                Some(md)
            }),
        );
        env.copy_card = Arc::new(|card, _column, kind| COPIED.with(|c| c.borrow_mut().push((card.id.clone(), kind))));
        env.paste_card = Arc::new(|| PASTE.with(|p| p.borrow_mut().take()));
        env
    }
}

impl EmbedFactory for Factory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let id = target.trim().strip_prefix("kanban:")?;
        let handle = self.handles.get(id)?.clone();
        let body = view(self.env(), id.to_string(), handle);
        Some(Box::new(
            DecoratedBox::new()
                .style("height", StyleValue::px(ectx.height.unwrap_or(crate::pages::notes::embeds::default_object_h("kanban"))))
                .child(crate::components::workspace_frame::expand(Box::new(body))),
        ))
    }
    fn has_own_height(&self, _target: &str) -> bool {
        true
    }
}

fn card(id: &str, column: &str, title: &str) -> KanbanCard {
    let mut c = KanbanCard::new(id.into(), column.into());
    c.title = title.into();
    c
}

fn doc_with_cards(titles: &[&str]) -> KanbanDoc {
    let mut d = KanbanDoc::template(["Todo", "Doing", "Done"]);
    for (i, t) in titles.iter().enumerate() {
        let col = d.columns[0].id.clone();
        d.cards.push(card(&format!("k{i}"), &col, t));
    }
    d
}

struct World {
    h: TestHarness,
    page: DocumentEditorHandle,
    epoch: u64,
    factory_handles: HashMap<String, KanbanHandle>,
    md: String,
}

impl World {
    fn new(md: &str, handles: HashMap<String, KanbanHandle>) -> Self {
        let page = DocumentEditorHandle::new();
        let mut h = TestHarness::new(Box::new(Self::editor(md, &page, &handles, 0)));
        h.tree.text_measure = Some(Arc::new(Mono));
        h.rebuild();
        h.apply_mss(".grow { flex-grow: 1; }");
        h.layout(1200.0, 800.0);
        Self { h, page, epoch: 0, factory_handles: handles, md: md.to_string() }
    }

    fn editor(
        md: &str,
        page: &DocumentEditorHandle,
        handles: &HashMap<String, KanbanHandle>,
        epoch: u64,
    ) -> DocumentEditor {
        DocumentEditor::new()
            .markdown(md)
            .handle(page)
            .embeds(Arc::new(Factory { handles: handles.clone(), page: page.clone() }))
            .block_drag_type(DRAG_TYPE_BLOCK)
            .model_epoch(epoch)
            .layout(DocLayout { free: true, ..DocLayout::default() })
    }

    /// Перестроить после правок (сигналы досок, очередь операций страницы).
    fn settle(&mut self) {
        self.epoch += 1;
        let w = Self::editor(&self.md, &self.page, &self.factory_handles, self.epoch);
        self.h.update_widget(Box::new(w));
        self.h.rebuild();
        self.h.apply_mss(".grow { flex-grow: 1; }");
        self.h.layout(1200.0, 800.0);
    }

    fn draggables(&self) -> Vec<ElementId> {
        self.h.find_by_type_name("Draggable")
    }

    fn bounds(&self, id: ElementId) -> Rect {
        self.h.element_bounds(id)
    }

    /// Прямоугольники всех DropArea, слева направо.
    fn drop_areas(&self) -> Vec<Rect> {
        let mut v: Vec<Rect> =
            self.h.find_by_type_name("DropArea").iter().map(|id| self.h.element_bounds(*id)).collect();
        v.sort_by(|a, b| a.origin.x.partial_cmp(&b.origin.x).unwrap().then(a.origin.y.partial_cmp(&b.origin.y).unwrap()));
        v
    }

    /// Тело колонки с индексом `idx` (0-based) доски: самая высокая DropArea
    /// среди тех, что правее карточек первой колонки на `idx` ширин.
    fn lane_body(&self, board_x0: f32, idx: usize) -> Rect {
        let lane_w = super::model::DEFAULT_COLUMN_WIDTH;
        let x_min = board_x0 + lane_w * idx as f32;
        let x_max = x_min + lane_w;
        self.drop_areas()
            .into_iter()
            .filter(|r| r.origin.x >= x_min && r.origin.x < x_max && r.size.height > 100.0 && r.size.width < lane_w + 20.0)
            .max_by(|a, b| a.size.height.partial_cmp(&b.size.height).unwrap())
            .unwrap_or_else(|| panic!("нет тела колонки {idx} среди {:?}", self.drop_areas()))
    }

    /// Как `AppHandler`: нажать, сдвинуть за порог, довести до точки.
    fn drag_to(&mut self, from: Point, to: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from });
        self.h.send_event(&Event::MouseMove(Point::new(from.x + 12.0, from.y + 12.0)));
        assert!(self.h.tree.drag_state.is_some(), "drag не начался из {from:?}");
        self.h.send_event(&Event::MouseMove(to));
        let data = self.h.tree.drag_state.as_ref().unwrap().data.clone();
        self.h.tree.dispatch_drag_event(&Event::DragMove { position: to, data });
    }

    /// Отпустить в точке — как приложение на MouseUp при drag'е.
    fn release(&mut self, at: Point) {
        assert!(self.h.tree.end_drag(self.h.root_id, at, false), "переноса не было");
    }

    fn drag(&mut self, from: Point, to: Point) {
        self.drag_to(from, to);
        self.release(to);
    }
}

fn center(r: Rect) -> Point {
    Point::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0)
}

fn one_board(titles: &[&str]) -> (World, KanbanHandle) {
    let handle = KanbanHandle::new(doc_with_cards(titles));
    let mut handles = HashMap::new();
    handles.insert("b1".to_string(), handle.clone());
    (World::new("![[kanban:b1]]{h=340}\n", handles), handle)
}

fn column_ids(handle: &KanbanHandle, column: usize) -> Vec<String> {
    let doc = handle.lock();
    let col = doc.columns[column].id.clone();
    doc.cards_of(&col).iter().map(|c| c.id.clone()).collect()
}

#[test]
fn card_dropped_on_empty_area_goes_to_the_end_of_that_column() {
    let (mut w, handle) = one_board(&["Задача"]);
    let drg = w.draggables();
    assert_eq!(drg.len(), 1);
    let card = w.bounds(drg[0]);
    assert!(card.size.width > 0.0 && card.size.height > 0.0, "карточка без размера: {card:?}");
    let board_x0 = w.lane_body(card.origin.x - 40.0, 0).origin.x - 8.0;
    let doing = w.lane_body(board_x0, 1);
    // Середина пустой колонки — не хвост и не карточка.
    let at = Point::new(doing.origin.x + doing.size.width / 2.0, doing.origin.y + doing.size.height / 2.0);
    w.drag_to(center(card), at);
    // Плейсхолдер — в конец второй колонки.
    let doing_id = handle.lock().columns[1].id.clone();
    assert_eq!(handle.hover.get_untracked(), Some(DropSpot::end(&doing_id)));
    w.release(at);
    assert_eq!(column_ids(&handle, 1), ["k0"]);
    assert!(column_ids(&handle, 0).is_empty());
    assert_eq!(handle.hover.get_untracked(), None, "после дропа плейсхолдер снимается");
}

#[test]
fn card_dropped_on_upper_or_lower_half_of_another_card_lands_before_or_after() {
    let (mut w, handle) = one_board(&["Первая", "Вторая", "Третья"]);
    let drg = w.draggables();
    assert_eq!(drg.len(), 3);
    let mut rects: Vec<Rect> = drg.iter().map(|id| w.bounds(*id)).collect();
    rects.sort_by(|a, b| a.origin.y.partial_cmp(&b.origin.y).unwrap());
    let (first, third) = (rects[0], rects[2]);
    // Третью — на верхнюю половину первой: перед первой.
    let upper = Point::new(center(first).x, first.origin.y + 3.0);
    w.drag(center(third), upper);
    assert_eq!(column_ids(&handle, 0), ["k2", "k0", "k1"]);
    w.settle();

    // Теперь k2 первая. Её — на нижнюю половину последней (k1): в конец.
    let drg = w.draggables();
    let mut rects: Vec<Rect> = drg.iter().map(|id| w.bounds(*id)).collect();
    rects.sort_by(|a, b| a.origin.y.partial_cmp(&b.origin.y).unwrap());
    let (top, last) = (rects[0], rects[2]);
    let lower = Point::new(center(last).x, last.origin.y + last.size.height - 3.0);
    w.drag_to(center(top), lower);
    let todo = handle.lock().columns[0].id.clone();
    assert_eq!(handle.hover.get_untracked(), Some(DropSpot::end(&todo)), "нижняя половина последней — конец колонки");
    w.release(lower);
    assert_eq!(column_ids(&handle, 0), ["k0", "k1", "k2"]);
}

#[test]
fn own_spot_shows_no_placeholder() {
    let (mut w, handle) = one_board(&["Первая", "Вторая"]);
    let drg = w.draggables();
    let mut rects: Vec<Rect> = drg.iter().map(|id| w.bounds(*id)).collect();
    rects.sort_by(|a, b| a.origin.y.partial_cmp(&b.origin.y).unwrap());
    let first = rects[0];
    // Первую — на её же нижнюю половину: место «после первой» = её место.
    let lower = Point::new(center(first).x, first.origin.y + first.size.height - 3.0);
    w.drag_to(center(first), lower);
    assert_eq!(handle.hover.get_untracked(), None);
    w.release(lower);
    assert_eq!(column_ids(&handle, 0), ["k0", "k1"]);
}

#[test]
fn card_moves_between_two_boards_on_one_page() {
    let a = KanbanHandle::new(doc_with_cards(&["Из A"]));
    let b = KanbanHandle::new(doc_with_cards(&[]));
    let mut handles = HashMap::new();
    handles.insert("a".to_string(), a.clone());
    handles.insert("b".to_string(), b.clone());
    // Две доски одна под другой на холсте.
    let md = "![[kanban:a]]{h=300}\n\n![[kanban:b]]{h=300}\n\n```doc-layout\n0 20 20 900\n1 20 360 900\n```\n";
    let mut w = World::new(md, handles);
    let drg = w.draggables();
    assert_eq!(drg.len(), 1);
    let card = w.bounds(drg[0]);
    // Тело первой колонки нижней доски: DropArea ниже y=360, самая высокая.
    let target = w
        .drop_areas()
        .into_iter()
        .filter(|r| r.origin.y > 340.0 && r.size.height > 100.0 && r.size.width < super::model::DEFAULT_COLUMN_WIDTH + 20.0)
        .min_by(|p, q| p.origin.x.partial_cmp(&q.origin.x).unwrap())
        .expect("тело колонки нижней доски");
    let at = center(target);
    w.drag_to(center(card), at);
    let b_todo = b.lock().columns[0].id.clone();
    assert_eq!(b.hover.get_untracked(), Some(DropSpot::end(&b_todo)));
    assert_eq!(a.hover.get_untracked(), None);
    w.release(at);
    assert!(a.lock().cards.is_empty(), "карточка должна уйти с доски A");
    assert_eq!(b.lock().cards.len(), 1);
    assert_eq!(b.lock().cards[0].title, "Из A");
    assert_eq!(b.lock().cards[0].column, b_todo);
}

#[test]
fn page_block_dragged_by_its_handle_becomes_a_card() {
    let handle = KanbanHandle::new(doc_with_cards(&[]));
    let mut handles = HashMap::new();
    handles.insert("b1".to_string(), handle.clone());
    let md = "# Импорт Excel\n\n![[kanban:b1]]{h=340}\n";
    let mut w = World::new(md, handles);

    // Наведение на блок — ручка ⋮⋮ рисуется у блока под курсором.
    let row = w.bounds(w.h.find_by_type_name("doc-text-row")[0]);
    let inside = Point::new(row.origin.x + 5.0, row.origin.y + 5.0);
    w.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: inside });
    w.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: inside });
    w.h.send_event(&Event::MouseMove(inside));
    let mut list = DisplayList::new();
    w.h.tree.build_display_list(w.h.root_id, &mut list, Rect::new(Point::zero(), Size::new(1200.0, 800.0)));

    let board_x0 = w.drop_areas().iter().map(|r| r.origin.x).fold(f32::MAX, f32::min);
    let todo_body = w.lane_body(board_x0, 0);
    let at = center(todo_body);
    // Ручка — по центру первой строки (у заголовка строка выше обычной).
    let grip = Point::new(row.origin.x - 14.0, row.origin.y + row.size.height / 2.0);
    w.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: grip });
    w.h.send_event(&Event::MouseMove(Point::new(grip.x + 30.0, grip.y + 30.0)));
    let drag = w.h.tree.drag_state.as_ref().expect("перенос блока должен объявить drag дерева");
    assert_eq!(drag.data.drag_type, DRAG_TYPE_BLOCK);
    w.h.send_event(&Event::MouseMove(at));
    let data = w.h.tree.drag_state.as_ref().unwrap().data.clone();
    w.h.tree.dispatch_drag_event(&Event::DragMove { position: at, data });
    let todo = handle.lock().columns[0].id.clone();
    assert_eq!(handle.hover.get_untracked(), Some(DropSpot::end(&todo)), "плейсхолдер и для блока страницы");
    w.release(at);

    let cards = handle.lock().cards.clone();
    assert_eq!(cards.len(), 1, "блок должен стать карточкой");
    assert_eq!(cards[0].title, "Импорт Excel");
    assert!(cards[0].md.is_empty());
    w.settle();
    let page = w.page.serialize();
    assert!(!page.contains("Импорт Excel"), "блок должен уйти со страницы:\n{page}");
    assert!(page.contains("![[kanban:b1]]"), "доска на месте:\n{page}");
}

// ─── Буфер обмена ─────────────────────────────────────────────────────────

/// Прямоугольники карточек сверху вниз.
fn card_rects(w: &World) -> Vec<Rect> {
    let mut rects: Vec<Rect> = w.draggables().iter().map(|id| w.bounds(*id)).collect();
    rects.sort_by(|a, b| a.origin.y.partial_cmp(&b.origin.y).unwrap());
    rects
}

fn pasted(title: &str) -> KanbanCard {
    let mut c = KanbanCard::new("px".into(), String::new());
    c.title = title.into();
    c
}

/// Правый клик в точке и выбор пункта меню `index` (сверху, с нуля):
/// у меню отступ 4, пункты по 32.
fn menu_pick(w: &mut World, at: Point, index: usize) {
    w.h.send_event(&Event::MouseDown { button: MouseButton::Right, position: at });
    w.h.layout(1200.0, 800.0);
    // Меню узнаёт размер поверхности при отрисовке — до неё кликов нет.
    w.h.paint();
    let item = Point::new(at.x + 20.0, at.y + 4.0 + 32.0 * index as f32 + 16.0);
    w.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: item });
    w.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: item });
}

#[test]
fn right_click_on_card_opens_its_menu_and_paste_lands_after_it() {
    let (mut w, handle) = one_board(&["Первая", "Вторая"]);
    let first = card_rects(&w)[0];
    PASTE.with(|p| *p.borrow_mut() = Some(pasted("Из буфера")));
    // Пункты: «Копировать ▸», «Вставить карточку».
    menu_pick(&mut w, center(first), 1);
    assert_eq!(column_ids(&handle, 0), ["k0", "px", "k1"], "вставка — после карточки под курсором");
    assert_eq!(handle.selected.get_untracked().as_deref(), Some("px"), "вставленная выбрана");
}

#[test]
fn right_click_on_empty_column_area_pastes_to_its_end() {
    let (mut w, handle) = one_board(&["Первая"]);
    let card = card_rects(&w)[0];
    let board_x0 = w.lane_body(card.origin.x - 40.0, 0).origin.x - 8.0;
    let doing = w.lane_body(board_x0, 1);
    PASTE.with(|p| *p.borrow_mut() = Some(pasted("В конец")));
    menu_pick(&mut w, Point::new(doing.origin.x + 30.0, doing.origin.y + 40.0), 0);
    assert_eq!(column_ids(&handle, 1), ["px"]);
    assert_eq!(column_ids(&handle, 0), ["k0"]);
}

#[test]
fn ctrl_c_in_card_editor_without_selection_copies_whole_card() {
    let (mut w, handle) = one_board(&["Задача"]);
    handle.start_editing("k0");
    w.settle();
    COPIED.with(|c| c.borrow_mut().clear());
    w.h.tree.modifiers.ctrl = true;
    w.h.send_event(&Event::KeyDown(Key::C));
    w.h.tree.modifiers.ctrl = false;
    assert_eq!(COPIED.with(|c| c.borrow().clone()), [("k0".to_string(), CopyKind::Full)]);
}

// ─── Таймеры колонок ──────────────────────────────────────────────────────

/// Колонка с правилом: значок в её шапке и чип отсчёта на карточке. Такт
/// без срабатываний сдвигает только отсчёт (`tick`), доска не
/// перестраивается; просроченная карточка уезжает по такту сама, и в
/// колонке без правил чипа у неё нет.
#[test]
fn column_timer_shows_icon_and_countdown_and_moves_the_card_on_tick() {
    let mut d = doc_with_cards(&["Зависла"]);
    let doing = d.columns[1].id.clone();
    d.columns[0].move_after_hours = Some(72);
    d.columns[0].move_to = Some(doing);
    let handle = KanbanHandle::new(d);
    // Первый такт — штамп `entered` у карточки без него; записей журнала
    // нет, но ревизия растёт — штамп уйдёт на диск автосейвом.
    let rev0 = handle.revision.get_untracked();
    handle.sweep();
    assert!(handle.lock().cards[0].entered.is_some());
    assert!(handle.revision.get_untracked() > rev0, "штампы такта должны сохраняться");
    let mut handles = HashMap::new();
    handles.insert("b1".to_string(), handle.clone());
    let mut w = World::new("![[kanban:b1]]{h=340}\n", handles);
    assert_eq!(w.h.find_by_class("notes-kanban-lane-timer").len(), 1, "значок только у колонки с правилом");
    assert_eq!(w.h.find_by_class("timer").len(), 1, "чип отсчёта на карточке");

    let (rev, tick) = (handle.structure_rev.get_untracked(), handle.tick.get_untracked());
    handle.sweep();
    assert_eq!(handle.structure_rev.get_untracked(), rev, "такт без срабатываний не перестраивает доску");
    assert_eq!(handle.tick.get_untracked(), tick + 1, "а сдвигает отсчёт");

    {
        let mut doc = handle.lock();
        let entered = doc.cards[0].entered.unwrap();
        doc.cards[0].entered = Some(entered - 73 * 3600);
    }
    handle.sweep();
    assert_eq!(column_ids(&handle, 1), ["k0"], "73 ч в «Todo» — карточка в «Doing»");
    assert!(column_ids(&handle, 0).is_empty());
    w.settle();
    assert_eq!(w.h.find_by_class("timer").len(), 0, "в «Doing» правил нет — и чипа нет");
    assert_eq!(w.h.find_by_class("notes-kanban-lane-timer").len(), 1);
}
