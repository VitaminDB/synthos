//! Headless-тесты календаря на `TestHarness` syngui: виджет живёт во
//! врезке редактора страницы, как в приложении; геометрия ячеек и слотов
//! повторяет расчёты элементов (константы шапки/жёлоба). Запуск:
//! `cargo test --features testing calendar::harness_tests`.

use std::collections::HashMap;
use std::sync::Arc;

use syngui::core::{Point, Rect};
use syngui::input::{Event, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widget::context::TextMeasure;
use syngui::widgets::input::document_editor::{DocLayout, DocumentEditor, DocumentEditorHandle, EmbedCtx, EmbedFactory};

use super::model::{range_of, CalEvent, CalView, CalendarDoc, CalendarStore};
use super::view::view;
use super::{CalendarEnv, CalendarHandle, CalendarStoreHandle, ExternalItem, ExternalKind, ExternalRef};
use crate::pages::notes::gantt::calendar::parse_days;

struct Mono;
impl TextMeasure for Mono {
    fn measure_text_width(&self, _t: &str, _fs: f32, chars: usize) -> f32 {
        chars as f32 * 10.0
    }
    fn hit_test_char(&self, text: &str, _fs: f32, x: f32) -> usize {
        ((x / 10.0).round() as usize).min(text.chars().count())
    }
}

/// Слой задач досок для теста: что отдавать сетке и какие переносы она
/// попросила (доски здесь нет — календарь знает только `ExternalRef`).
#[derive(Clone, Default)]
struct External {
    items: Arc<std::sync::Mutex<Vec<ExternalItem>>>,
    shifts: Arc<std::sync::Mutex<Vec<(String, i64)>>>,
}

impl External {
    fn env_pair(&self) -> (Arc<dyn Fn(&super::ExternalQuery) -> Vec<ExternalItem> + Send + Sync>, Arc<dyn Fn(&ExternalRef, i64) -> bool + Send + Sync>) {
        let items = self.items.clone();
        let shifts = self.shifts.clone();
        (
            Arc::new(move |_| items.lock().unwrap().clone()),
            Arc::new(move |r, delta| {
                shifts.lock().unwrap().push((r.item.clone(), delta));
                true
            }),
        )
    }
}

struct Factory {
    handles: HashMap<String, CalendarHandle>,
    store: CalendarStoreHandle,
    external: External,
}

impl EmbedFactory for Factory {
    fn build(&self, target: &str, ectx: &EmbedCtx) -> Option<Box<dyn Widget>> {
        let id = target.trim().strip_prefix("calendar:")?;
        let handle = self.handles.get(id)?.clone();
        let (external, shift_external) = self.external.env_pair();
        let env = CalendarEnv {
            store: self.store.clone(),
            external,
            shift_external,
            boards: Arc::new(Vec::new),
            project_rev: syngui::signal::use_signal(0),
            open_page: Arc::new(|_| {}),
        };
        let body = view(env, id.to_string(), handle);
        Some(Box::new(
            DecoratedBox::new()
                .style("height", StyleValue::px(ectx.height.unwrap_or(480.0)))
                .child(crate::components::workspace_frame::expand(Box::new(body))),
        ))
    }
    fn has_own_height(&self, _target: &str) -> bool {
        true
    }
}

struct World {
    h: TestHarness,
    page: DocumentEditorHandle,
    epoch: u64,
    handles: HashMap<String, CalendarHandle>,
    store: CalendarStoreHandle,
    external: External,
    md: String,
}

impl World {
    fn new(md: &str, handles: HashMap<String, CalendarHandle>, store: CalendarStoreHandle, external: External) -> Self {
        let page = DocumentEditorHandle::new();
        let mut h = TestHarness::new(Box::new(Self::editor(md, &page, &handles, &store, &external, 0)));
        h.tree.text_measure = Some(Arc::new(Mono));
        h.rebuild();
        h.apply_mss(".grow { flex-grow: 1; }");
        h.layout(1200.0, 800.0);
        Self { h, page, epoch: 0, handles, store, external, md: md.to_string() }
    }

    fn editor(
        md: &str,
        page: &DocumentEditorHandle,
        handles: &HashMap<String, CalendarHandle>,
        store: &CalendarStoreHandle,
        external: &External,
        epoch: u64,
    ) -> DocumentEditor {
        DocumentEditor::new()
            .markdown(md)
            .handle(page)
            .embeds(Arc::new(Factory { handles: handles.clone(), store: store.clone(), external: external.clone() }))
            .model_epoch(epoch)
            .layout(DocLayout { free: true, ..DocLayout::default() })
    }

    fn settle(&mut self) {
        self.epoch += 1;
        let w = Self::editor(&self.md, &self.page, &self.handles, &self.store, &self.external, self.epoch);
        self.h.update_widget(Box::new(w));
        self.h.rebuild();
        self.h.apply_mss(".grow { flex-grow: 1; }");
        self.h.layout(1200.0, 800.0);
    }

    fn grid(&self, name: &str) -> Rect {
        let ids = self.h.find_by_type_name(name);
        assert_eq!(ids.len(), 1, "ровно одна сетка {name}");
        self.h.element_bounds(ids[0])
    }

    fn click(&mut self, at: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: at });
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: at });
    }

    fn drag(&mut self, from: Point, to: Point) {
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from });
        self.h.send_event(&Event::MouseMove(Point::new(from.x + 8.0, from.y + 8.0)));
        self.h.send_event(&Event::MouseMove(to));
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: to });
    }
}

const D: &str = "2026-09-03";

fn day(iso: &str) -> i64 {
    parse_days(iso).unwrap()
}

fn world(view: CalView, events: Vec<CalEvent>) -> (World, CalendarHandle, CalendarStoreHandle) {
    let (w, h, s, _) = world_with(view, events, Vec::new());
    (w, h, s)
}

/// То же плюс слой задач досок (`external`).
fn world_with(
    view: CalView,
    events: Vec<CalEvent>,
    external: Vec<ExternalItem>,
) -> (World, CalendarHandle, CalendarStoreHandle, External) {
    let mut store = CalendarStore::template("Личное");
    let cal = store.calendars[0].id.clone();
    for mut e in events {
        e.calendar = cal.clone();
        store.add_event(e);
    }
    let store = CalendarStoreHandle::new(store);
    let handle = CalendarHandle::new(CalendarDoc::template(view, day(D)));
    let mut handles = HashMap::new();
    handles.insert("c1".to_string(), handle.clone());
    let ext = External::default();
    *ext.items.lock().unwrap() = external;
    (World::new("![[calendar:c1]]{h=480}\n", handles, store.clone(), ext.clone()), handle, store, ext)
}

/// Полоса задачи доски на календаре.
fn card_bar(title: &str, from: i64, to: i64, time: Option<(u32, u32)>) -> ExternalItem {
    ExternalItem {
        day: from,
        end_day: to,
        time,
        title: title.to_string(),
        color: "#4F8CFF".to_string(),
        page: "p1".to_string(),
        source: ExternalRef { kind: ExternalKind::Card, object: "b1".to_string(), item: "k1".to_string() },
    }
}

/// Центр ячейки дня в месячной сетке (шапка 22 px, 6 строк).
fn month_cell(grid: Rect, handle: &CalendarHandle, d: i64) -> Point {
    let (from, _) = range_of(CalView::Month, handle.anchor(), 0);
    let idx = d - from;
    let cw = grid.size.width / 7.0;
    let ch = (grid.size.height - 22.0) / 6.0;
    Point::new(grid.origin.x + (idx % 7) as f32 * cw + cw / 2.0, grid.origin.y + 22.0 + (idx / 7) as f32 * ch + ch / 2.0)
}

#[test]
fn month_click_selects_day_and_double_click_opens_new_event() {
    let (mut w, handle, _store) = world(CalView::Month, vec![]);
    let grid = w.grid("notes-calendar-month");
    assert!(grid.size.width > 300.0 && grid.size.height > 200.0, "сетка без размера: {grid:?}");
    let target = day("2026-09-10");
    let at = month_cell(grid, &handle, target);
    w.click(at);
    assert_eq!(handle.selected_day.get_untracked(), Some(target));
    assert!(!handle.popup_open.get_untracked());
    w.click(at);
    assert!(handle.popup_open.get_untracked(), "двойной клик открывает попап");
    let draft = handle.draft.get_untracked().expect("черновик");
    assert_eq!(draft.event.date, "2026-09-10");
    assert!(draft.event.all_day);
    w.settle();
    assert!(!w.h.find_by_type_name("PopupPanel").is_empty(), "попап смонтирован");
}

#[test]
fn month_chip_drag_moves_event_to_another_day() {
    let e = CalEvent::new("", "Дедлайн", day(D));
    let (mut w, handle, store) = world(CalView::Month, vec![e]);
    let id = store.lock().events[0].id.clone();
    let grid = w.grid("notes-calendar-month");
    let from_cell = month_cell(grid, &handle, day(D));
    // Чип — под номером дня: верх ячейки + 20 + 2 + половина чипа.
    let ch = (grid.size.height - 22.0) / 6.0;
    let chip = Point::new(from_cell.x, from_cell.y - ch / 2.0 + 22.0 + 10.0);
    let to = month_cell(grid, &handle, day("2026-09-08"));
    w.drag(chip, to);
    assert_eq!(store.lock().event(&id).unwrap().date, "2026-09-08", "событие переехало");
    w.settle();
    // Клик по чипу (без переноса) — попап правки того же события.
    let to_cell = month_cell(grid, &handle, day("2026-09-08"));
    let chip = Point::new(to_cell.x, to_cell.y - ch / 2.0 + 22.0 + 10.0);
    w.click(chip);
    assert!(handle.popup_open.get_untracked());
    assert_eq!(handle.draft.get_untracked().and_then(|d| d.id), Some(id));
}

#[test]
fn week_drag_moves_event_by_day_and_resize_extends_it() {
    let mut e = CalEvent::new("", "Встреча", day(D));
    e.start = Some(10 * 60);
    e.end = Some(11 * 60);
    e.all_day = false;
    let (mut w, handle, store) = world(CalView::Week, vec![e]);
    let id = store.lock().events[0].id.clone();
    let grid = w.grid("notes-calendar-timegrid");
    // Геометрия сетки часов: шапка 32, ряд «весь день» 22, жёлоб 48, слот 30 мин × 24 px.
    let (from, _) = range_of(CalView::Week, handle.anchor(), 0);
    let col_w = (grid.size.width - 48.0) / 7.0;
    let col_x = |d: i64| grid.origin.x + 48.0 + (d - from) as f32 * col_w;
    let y_of = |min: u32| grid.origin.y + 32.0 + 22.0 + (min as f32 - 8.0 * 60.0) / 30.0 * 24.0;
    let body = Point::new(col_x(day(D)) + col_w / 2.0, y_of(10 * 60) + 20.0);
    // На день вправо и на час позже.
    w.drag(body, Point::new(body.x + col_w, body.y + 2.0 * 24.0));
    {
        let s = store.lock();
        let ev = s.event(&id).unwrap();
        assert_eq!(ev.date, "2026-09-04", "перенос на следующий день");
        assert_eq!((ev.start, ev.end), (Some(11 * 60), Some(12 * 60)), "время сдвинулось на час, длительность прежняя");
    }
    w.settle();
    // Нижняя кромка — растянуть ещё на час.
    let edge = Point::new(col_x(day("2026-09-04")) + col_w / 2.0, y_of(12 * 60) - 3.0);
    w.drag(edge, Point::new(edge.x, edge.y + 2.0 * 24.0));
    let s = store.lock();
    let ev = s.event(&id).unwrap();
    assert_eq!(ev.end, Some(13 * 60), "конец растянут");
    assert_eq!(ev.start, Some(11 * 60));
}

/// Задача доски с часами рисуется отрезком в сетке часов дневного вида —
/// раньше «День» молчал о задачах: у карточки был только срок-точка.
#[test]
fn day_view_draws_a_timed_card_bar_in_the_hour_grid() {
    let bar = card_bar("Импорт", day(D), day(D), Some((10 * 60, 12 * 60)));
    let (mut w, _handle, _store, _ext) = world_with(CalView::Day, vec![], vec![bar]);
    let grid = w.grid("notes-calendar-timegrid");
    let y_of = |min: u32| grid.origin.y + 32.0 + 22.0 + (min as f32 - 8.0 * 60.0) / 30.0 * 24.0;
    let mut list = syngui::render::DisplayList::new();
    w.h.tree.build_display_list(w.h.root_id, &mut list, Rect::new(Point::zero(), syngui::core::Size::new(1200.0, 800.0)));
    let (top, height) = (y_of(10 * 60), y_of(12 * 60) - y_of(10 * 60));
    let found = list.iter_all_commands().any(|c| match c {
        syngui::render::DrawCommand::Rect { rect, .. } => {
            (rect.origin.y - top).abs() < 1.0 && (rect.size.height - height).abs() < 1.0 && rect.size.width > 100.0
        }
        _ => false,
    });
    assert!(found, "полоса задачи 10:00–12:00 не нарисована в сетке часов (top {top}, h {height})");
}

/// Полоса задачи доски переносится по дням: календарь просит сдвинуть её
/// на столько же дней, а сама карточка живёт в доске.
#[test]
fn week_card_bar_drag_shifts_the_task_by_days() {
    let bar = card_bar("Импорт", day(D), day(D), Some((10 * 60, 12 * 60)));
    let (mut w, handle, _store, ext) = world_with(CalView::Week, vec![], vec![bar]);
    let grid = w.grid("notes-calendar-timegrid");
    let (from, _) = range_of(CalView::Week, handle.anchor(), 0);
    let col_w = (grid.size.width - 48.0) / 7.0;
    let col_x = |d: i64| grid.origin.x + 48.0 + (d - from) as f32 * col_w;
    let y_of = |min: u32| grid.origin.y + 32.0 + 22.0 + (min as f32 - 8.0 * 60.0) / 30.0 * 24.0;
    let body = Point::new(col_x(day(D)) + col_w / 2.0, y_of(10 * 60) + 20.0);
    w.drag(body, Point::new(body.x + 2.0 * col_w, body.y));
    assert_eq!(*ext.shifts.lock().unwrap(), vec![("k1".to_string(), 2)], "перенос на два дня вперёд");

    // Многодневная полоса «весь день» тащится из своего ряда.
    ext.shifts.lock().unwrap().clear();
    *ext.items.lock().unwrap() = vec![card_bar("Отпуск", day(D), day("2026-09-05"), None)];
    w.settle();
    let grid = w.grid("notes-calendar-timegrid");
    let allday = Point::new(col_x(day(D)) + col_w / 2.0, grid.origin.y + 32.0 + 3.0 + 8.0);
    w.drag(allday, Point::new(allday.x - col_w, allday.y));
    assert_eq!(*ext.shifts.lock().unwrap(), vec![("k1".to_string(), -1)], "перенос на день назад");
}

#[test]
fn now_line_is_drawn_in_todays_column() {
    crate::agent::time::override_offset_secs(Some(0));
    let (today, now_min) = crate::agent::time::local_now();
    let mut e = CalEvent::new("", "x", today);
    e.start = Some(0);
    e.end = Some(30);
    e.all_day = false;
    let (mut w, handle, _store) = world(CalView::Day, vec![]);
    handle.set_anchor(today);
    handle.set_style(|s| {
        s.from_min = 0;
        s.to_min = 24 * 60;
    });
    w.settle();
    let _ = e;
    let grid = w.grid("notes-calendar-timegrid");
    let mut list = syngui::render::DisplayList::new();
    w.h.tree.build_display_list(w.h.root_id, &mut list, Rect::new(Point::zero(), syngui::core::Size::new(1200.0, 800.0)));
    // Линия «сейчас»: прямоугольник высотой 2 на ожидаемом y.
    let expected_y = grid.origin.y + 32.0 + 22.0 + now_min as f32 / 30.0 * 24.0 - 1.0;
    let found = list.iter_all_commands().any(|c| match c {
        syngui::render::DrawCommand::Rect { rect, .. } => (rect.size.height - 2.0).abs() < 0.01 && (rect.origin.y - expected_y).abs() < 1.0,
        _ => false,
    });
    assert!(found, "линия «сейчас» на y={expected_y}");
    crate::agent::time::override_offset_secs(None);
}

/// Прямоугольники дисплей-листа (после укладки) — для проверок геометрии.
fn rects(w: &mut World) -> Vec<Rect> {
    let mut list = syngui::render::DisplayList::new();
    w.h.tree.build_display_list(w.h.root_id, &mut list, Rect::new(Point::zero(), syngui::core::Size::new(1200.0, 800.0)));
    list.iter_all_commands()
        .filter_map(|c| match c {
            syngui::render::DrawCommand::Rect { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect()
}

/// Многодневное событие в месяце — одна полоса через три ячейки, а не три
/// чипа: раньше каждый день получал свой чип с «…», и полоса рвалась.
#[test]
fn month_multi_day_event_is_one_bar_across_cells() {
    let mut e = CalEvent::new("", "Отпуск", day(D));
    e.end_date = Some("2026-09-05".into());
    let (mut w, handle, _store) = world(CalView::Month, vec![e]);
    let grid = w.grid("notes-calendar-month");
    let cw = grid.size.width / 7.0;
    let ch = (grid.size.height - 22.0) / 6.0;
    let (from, _) = range_of(CalView::Month, handle.anchor(), 0);
    let col = (day(D) - from) % 7;
    let row = (day(D) - from) / 7;
    let top = grid.origin.y + 22.0 + row as f32 * ch + 22.0;
    let bar = rects(&mut w).into_iter().find(|r| (r.origin.y - top).abs() < 1.0 && r.size.height > 15.0 && r.size.width > cw * 2.0);
    let bar = bar.expect("полоса на три ячейки");
    assert!((bar.size.width - (cw * 3.0 - 6.0)).abs() < 1.5, "ширина полосы {} при ячейке {cw}", bar.size.width);
    assert!((bar.origin.x - (grid.origin.x + col as f32 * cw + 3.0)).abs() < 1.5, "полоса начинается в ячейке своего первого дня");
}

/// Что не влезло в ячейку — «ещё n», а не наезд на нижнюю кромку; клик
/// по нему открывает список дня.
#[test]
fn month_overflow_shows_more_and_click_opens_day_list() {
    let target = day("2026-09-10");
    let events: Vec<CalEvent> = (0..8).map(|i| CalEvent::new("", &format!("Дело {i}"), target)).collect();
    let (mut w, handle, _store) = world(CalView::Month, events);
    let grid = w.grid("notes-calendar-month");
    let cw = grid.size.width / 7.0;
    let ch = (grid.size.height - 22.0) / 6.0;
    let (from, _) = range_of(CalView::Month, handle.anchor(), 0);
    let (col, row) = ((target - from) % 7, (target - from) / 7);
    let cell_top = grid.origin.y + 22.0 + row as f32 * ch;
    let capacity = ((ch - 22.0) / 22.0).floor().max(1.0);
    assert!(capacity < 8.0, "ячейка вмещает всё — тест не про переполнение (ch {ch})");
    // Полос нарисовано не больше вместимости минус слот под «ещё n».
    let chips = rects(&mut w).into_iter().filter(|r| (r.origin.x - (grid.origin.x + col as f32 * cw + 3.0)).abs() < 1.5 && r.size.height > 15.0 && r.origin.y > cell_top && r.origin.y < cell_top + ch).count();
    assert_eq!(chips as f32, capacity - 1.0, "полос в ячейке при вместимости {capacity}");
    // Клик по строке «ещё n» — под последней видимой полосой.
    let more_y = cell_top + 22.0 + (capacity - 1.0) * 22.0 + 10.0;
    w.click(Point::new(grid.origin.x + col as f32 * cw + cw / 2.0, more_y));
    assert!(handle.day_popup_open.get_untracked(), "список дня открыт");
    assert_eq!(handle.day_popup_day.get_untracked(), Some(target));
    assert!(!handle.popup_open.get_untracked());
    w.settle();
    assert!(!w.h.find_by_type_name("PopupPanel").is_empty(), "попап списка дня смонтирован");
}

/// В ряду «весь день» недели многодневное событие — одна полоса через
/// колонки.
#[test]
fn week_all_day_multi_day_event_spans_columns() {
    let mut e = CalEvent::new("", "Конференция", day(D));
    e.end_date = Some("2026-09-05".into());
    let (mut w, handle, _store) = world(CalView::Week, vec![e]);
    let grid = w.grid("notes-calendar-timegrid");
    let (from, _) = range_of(CalView::Week, handle.anchor(), 0);
    let col_w = (grid.size.width - 48.0) / 7.0;
    let x0 = grid.origin.x + 48.0 + (day(D) - from) as f32 * col_w + 2.0;
    let top = grid.origin.y + 32.0 + 3.0;
    let bar = rects(&mut w).into_iter().find(|r| (r.origin.y - top).abs() < 1.0 && (r.origin.x - x0).abs() < 1.5 && r.size.width > col_w * 2.0);
    let bar = bar.expect("полоса на три колонки");
    assert!((bar.size.width - (col_w * 3.0 - 4.0)).abs() < 1.5, "ширина {} при колонке {col_w}", bar.size.width);
}

/// Событие за пределами часов вида не сплющивается в полоску у края:
/// диапазон часов расширяется под него, настройка стиля не меняется.
#[test]
fn hours_extend_to_fit_late_event() {
    let mut e = CalEvent::new("", "Отбой", day(D));
    e.start = Some(22 * 60 + 30);
    e.end = Some(23 * 60);
    e.all_day = false;
    let (mut w, handle, _store) = world(CalView::Day, vec![e]);
    let grid = w.grid("notes-calendar-timegrid");
    // 8:00..23:00 — 15 часов по два слота × 24 px.
    let expected_h = 32.0 + 22.0 + 15.0 * 2.0 * 24.0 + 8.0;
    assert!((grid.size.height - expected_h).abs() < 1.0, "высота сетки {} вместо {expected_h}", grid.size.height);
    let top = grid.origin.y + 32.0 + 22.0 + (22.5 - 8.0) * 2.0 * 24.0;
    let bar = rects(&mut w).into_iter().find(|r| (r.origin.y - top).abs() < 1.0 && (r.size.height - 24.0).abs() < 1.0 && r.size.width > 100.0);
    assert!(bar.is_some(), "полоса 22:30–23:00 в своём слоте");
    assert_eq!(handle.style().to_min, 20 * 60, "настройка часов не тронута");
}

/// Окно суток со сдвигом: сетка идёт 06:00 → 06:00, и колонка дня
/// показывает ночь следующей даты — событие в 02:00 стоит в её хвосте,
/// а не пропадает и не жмётся к кромке.
#[test]
fn shifted_day_window_shows_night_of_next_date() {
    let mut e = CalEvent::new("", "Ночная", day(D) + 1);
    e.start = Some(2 * 60);
    e.end = Some(3 * 60);
    e.all_day = false;
    let (mut w, handle, _store) = world(CalView::Day, vec![e]);
    handle.set_style(|s| {
        s.full_day = true;
        s.from_min = 6 * 60;
        s.to_min = 6 * 60;
    });
    w.settle();
    let grid = w.grid("notes-calendar-timegrid");
    // Сутки целиком: 48 слотов по 30 мин × 24 px плюс шапка, ряд «весь
    // день» и нижний отступ.
    let expected_h = 32.0 + 22.0 + 48.0 * 24.0 + 8.0;
    assert!((grid.size.height - expected_h).abs() < 1.0, "высота сетки {} вместо {expected_h}", grid.size.height);
    // 02:00 следующей даты — это 20 часов от начала окна.
    let top = grid.origin.y + 32.0 + 22.0 + 20.0 * 2.0 * 24.0;
    let bar = rects(&mut w).into_iter().find(|r| (r.origin.y - top).abs() < 1.0 && (r.size.height - 48.0).abs() < 1.0 && r.size.width > 100.0);
    assert!(bar.is_some(), "полоса 02:00–03:00 в ночном хвосте колонки");
}

/// Задача без времени остаётся в шапке и при окне со сдвигом: ряд «весь
/// день» живёт по календарным датам колонок.
#[test]
fn timeless_event_stays_in_allday_row_with_shifted_window() {
    let mut e = CalEvent::new("", "Без часов", day(D));
    e.all_day = true;
    let (mut w, handle, _store) = world(CalView::Day, vec![e]);
    handle.set_style(|s| {
        s.full_day = true;
        s.from_min = 6 * 60;
        s.to_min = 6 * 60;
    });
    w.settle();
    let grid = w.grid("notes-calendar-timegrid");
    let allday_top = grid.origin.y + 32.0 + 3.0;
    let rects = rects(&mut w);
    let bar = rects.iter().find(|r| (r.origin.y - allday_top).abs() < 1.0 && (r.size.height - 16.0).abs() < 1.0 && r.size.width > 100.0);
    assert!(bar.is_some(), "полоса «весь день» в шапке");
    let grid_top = grid.origin.y + 32.0 + 22.0;
    let in_grid = rects.iter().any(|r| r.origin.y > grid_top + 1.0 && (r.size.height - 48.0).abs() < 1.0 && r.size.width > 100.0);
    assert!(!in_grid, "в сетке часов её быть не должно");
}

/// Ночное окно 22:00 → 06:00 и перенос за полночь: сетка длиной восемь
/// часов, а событие, утащенное вниз через полночь, меняет дату — раньше
/// оно упиралось в 23:55.
#[test]
fn night_window_drag_past_midnight_moves_event_to_the_next_date() {
    let mut e = CalEvent::new("", "Поздняя", day(D));
    e.start = Some(23 * 60);
    e.end = Some(23 * 60 + 30);
    e.all_day = false;
    let (mut w, handle, store) = world(CalView::Day, vec![e]);
    handle.set_style(|s| {
        s.from_min = 22 * 60;
        s.to_min = 6 * 60;
    });
    w.settle();
    let id = store.lock().events[0].id.clone();
    let grid = w.grid("notes-calendar-timegrid");
    // Восемь часов по два слота × 24 px.
    let expected_h = 32.0 + 22.0 + 16.0 * 24.0 + 8.0;
    assert!((grid.size.height - expected_h).abs() < 1.0, "высота сетки {} вместо {expected_h}", grid.size.height);
    let col_w = grid.size.width - 48.0;
    // 23:00 — час от начала окна.
    let y = grid.origin.y + 32.0 + 22.0 + 2.0 * 24.0;
    let body = Point::new(grid.origin.x + 48.0 + col_w / 2.0, y + 12.0);
    // На четыре слота вниз — за полночь, в 01:00 следующей даты.
    w.drag(body, Point::new(body.x, body.y + 4.0 * 24.0));
    let s = store.lock();
    let ev = s.event(&id).unwrap();
    assert_eq!(ev.date, "2026-09-04", "событие переехало на следующую дату");
    assert_eq!((ev.start, ev.end), (Some(60), Some(90)), "01:00–01:30");
}
