//! Календарь — виджет страницы: живая врезка `![[calendar:<id>]]` над
//! объектом `notes/objects/<id>.calendar.json` (вид, якорь, фильтр, стиль)
//! и единым хранилищем событий проекта `notes/calendar.json`.
//!
//! [`CalendarStoreHandle`] — хранилище событий (Mutex + `revision` для
//! автосейва); один на проект, живёт в `NotesCtx.calendar`.
//! [`CalendarHandle`] — документ виджета: `revision` (автосейв),
//! `structure_rev` (перестройка), выбранное событие/день и состояние
//! попапа события.

/// Общая часть элементов-сеток: id, границы, флаги, классы, MSS.
pub struct ElementBase {
    pub id: syngui::widget::ElementId,
    pub bounds: syngui::core::Rect,
    pub dirty: syngui::widget::DirtyFlags,
    pub classes: Vec<String>,
    pub mss: syngui::mss::MssFields,
}

impl ElementBase {
    pub fn new() -> Self {
        Self {
            id: syngui::widget::ElementId::new(),
            bounds: syngui::core::Rect::zero(),
            dirty: syngui::widget::DirtyFlags::LAYOUT | syngui::widget::DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: syngui::mss::MssFields::new(),
        }
    }
}

/// Тривиальные методы `Element` через поле `base: ElementBase`.
macro_rules! element_boilerplate {
    ($name:literal) => {
        fn element_type_name(&self) -> &str {
            $name
        }
        fn set_classes(&mut self, c: Vec<String>) {
            self.base.classes = c;
        }
        fn get_classes(&self) -> &[String] {
            &self.base.classes
        }
        fn reset_mss_styles(&mut self) {
            self.base.mss.reset();
        }
        fn mss(&self) -> Option<&syngui::mss::MssFields> {
            Some(&self.base.mss)
        }
        fn apply_computed_style(&mut self, s: &syngui::mss::ComputedStyle) {
            self.base.mss.apply(s);
            self.base.dirty |= syngui::widget::DirtyFlags::RENDER;
        }
        fn id(&self) -> syngui::widget::ElementId {
            self.base.id
        }
        fn set_id(&mut self, id: syngui::widget::ElementId) {
            self.base.id = id;
        }
        fn bounds(&self) -> syngui::core::Rect {
            self.base.bounds
        }
        fn set_position(&mut self, pos: syngui::core::Point) {
            self.base.bounds.origin = pos;
        }
        fn set_content_size(&mut self, size: syngui::core::Size) {
            self.base.bounds.size = size;
        }
        fn children(&self) -> &[syngui::widget::ElementId] {
            &[]
        }
        fn mark_dirty(&mut self, f: syngui::widget::DirtyFlags) {
            self.base.dirty |= f;
        }
        fn clear_dirty(&mut self, f: syngui::widget::DirtyFlags) {
            self.base.dirty.remove(f);
        }
        fn is_dirty(&self, f: syngui::widget::DirtyFlags) -> bool {
            self.base.dirty.contains(f)
        }
        fn animate(&mut self, _dt: std::time::Duration) -> bool {
            false
        }
    };
}

pub mod layout;
pub mod model;
pub mod month;
pub mod paint;
pub mod popup;
pub mod timegrid;
pub mod view;
pub mod year;

#[cfg(all(test, feature = "testing"))]
mod harness_tests;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

use model::{CalEvent, CalView, CalendarDoc, CalendarStore, CalendarStyle};

use super::activity::{ActivityLogHandle, LogEntry};

/// Откуда взялся внешний элемент: срок карточки (точка), плановая полоса
/// карточки или задача Ганта.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalKind {
    Due,
    Card,
    Task,
}

/// Адрес внешнего элемента: объект проекта и его элемент — по нему
/// календарь двигает карточку или задачу, не зная про доски.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalRef {
    pub kind: ExternalKind,
    /// id объекта (`kanban` / `gantt`).
    pub object: String,
    /// id карточки либо задачи.
    pub item: String,
}

/// Внешний элемент на календаре: срок карточки доски, её плановая полоса
/// либо задача Ганта.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalItem {
    pub day: i64,
    /// Последний день полосы; у срока — тот же день.
    pub end_day: i64,
    /// Минуты начала/конца — полоса в сетке часов; `None` — «весь день».
    pub time: Option<(u32, u32)>,
    pub title: String,
    pub color: String,
    /// Страница, где живёт объект.
    pub page: String,
    pub source: ExternalRef,
}

/// Запрос внешнего слоя: диапазон дней и фильтр объектов (пусто — все).
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalQuery {
    pub from: i64,
    pub to: i64,
    /// id досок (`kanban`); пусто — все доски проекта.
    pub boards: Vec<String>,
    /// Сроки карточек точками.
    pub due: bool,
    /// Плановые полосы карточек.
    pub spans: bool,
    /// Задачи диаграмм Ганта.
    pub gantt: bool,
}

/// Окружение виджета: хранилище, внешние элементы, их перенос, открытие
/// страницы.
#[derive(Clone)]
pub struct CalendarEnv {
    pub store: CalendarStoreHandle,
    pub external: Arc<dyn Fn(&ExternalQuery) -> Vec<ExternalItem> + Send + Sync>,
    /// Сдвинуть внешний элемент на `delta` дней; `false` — не вышло.
    pub shift_external: Arc<dyn Fn(&ExternalRef, i64) -> bool + Send + Sync>,
    /// Доски проекта для выбора в панели свойств: `(id, название страницы)`.
    pub boards: Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>,
    /// Ревизия объектов проекта: правка чужой карточки перерисовывает
    /// календарь (её виджет живёт на другой странице).
    pub project_rev: RwSignal<u64>,
    pub open_page: Arc<dyn Fn(&str) + Send + Sync>,
}

#[derive(Clone)]
pub struct CalendarStoreHandle {
    store: Arc<Mutex<CalendarStore>>,
    pub revision: RwSignal<u64>,
    /// Журнал проекта: добавление, удаление, «сделано», перенос события.
    log: Option<ActivityLogHandle>,
}

impl PartialEq for CalendarStoreHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.store, &other.store)
    }
}

impl CalendarStoreHandle {
    pub fn new(store: CalendarStore) -> Self {
        Self { store: Arc::new(Mutex::new(store)), revision: use_signal(0), log: None }
    }

    pub fn with_log(mut self, log: ActivityLogHandle) -> Self {
        self.log = Some(log);
        self
    }

    pub fn lock(&self) -> MutexGuard<'_, CalendarStore> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    /// Правка хранилища; разница до/после по событиям уходит в журнал
    /// проекта (добавлено, удалено, сделано/снова открыто, перенесено).
    pub fn edit(&self, f: impl FnOnce(&mut CalendarStore)) {
        let entries = {
            let mut store = self.lock();
            let before = if self.log.is_some() { Some(store.events.clone()) } else { None };
            f(&mut store);
            before.map(|b| event_changes(&b, &store.events)).unwrap_or_default()
        };
        self.revision.set(self.revision.get_untracked() + 1);
        if let Some(log) = &self.log {
            log.record_all(entries);
        }
    }

    pub fn add_event(&self, event: CalEvent) -> String {
        let mut id = String::new();
        self.edit(|s| id = s.add_event(event));
        id
    }

    pub fn update_event(&self, id: &str, f: impl FnOnce(&mut CalEvent)) -> bool {
        let mut ok = false;
        self.edit(|s| {
            if let Some(e) = s.event_mut(id) {
                f(e);
                if e.start.is_none() {
                    e.all_day = true;
                }
                e.fix_done();
                ok = true;
            }
        });
        ok
    }

    /// «Сделано» вхождения, накрывающего `day`: у повтора — только этот
    /// повтор, у разового — всё событие. `false` — вхождения в этот день нет.
    pub fn set_event_done(&self, id: &str, day: i64, done: bool) -> bool {
        let mut marked = false;
        self.update_event(id, |e| marked = e.set_done_at(day, done));
        marked
    }

    pub fn remove_event(&self, id: &str) -> bool {
        let mut ok = false;
        self.edit(|s| ok = s.remove_event(id));
        ok
    }

    /// Перенести событие на другой день (и время, если задано).
    pub fn move_event(&self, id: &str, day: i64, start: Option<u32>) -> bool {
        self.update_event(id, |e| {
            let len = e.span().map(|(s, en)| en - s).unwrap_or(0);
            e.date = super::gantt::calendar::days_to_iso(day);
            if len > 0 {
                e.end_date = Some(super::gantt::calendar::days_to_iso(day + len));
            }
            if let (Some(s), Some((old_s, old_e))) = (start, e.time_span()) {
                let dur = old_e - old_s;
                e.start = Some(s);
                e.end = Some((s + dur).min(24 * 60));
            }
        })
    }

    pub fn add_calendar(&self, name: &str, color: &str) -> String {
        let mut id = String::new();
        self.edit(|s| id = s.add_calendar(name, color));
        id
    }

    pub fn remove_calendar(&self, id: &str) -> bool {
        let mut ok = false;
        self.edit(|s| ok = s.remove_calendar(id));
        ok
    }
}

/// Записи журнала по разнице списков событий.
fn event_changes(before: &[CalEvent], after: &[CalEvent]) -> Vec<LogEntry> {
    let mut out = Vec::new();
    let entry = |action: &str, e: &CalEvent| LogEntry::now("event", action).object("calendar").item(e.id.clone()).title(e.title.clone());
    for e in after {
        match before.iter().find(|b| b.id == e.id) {
            None => out.push(entry("add", e).from_to(String::new(), e.date.clone())),
            Some(b) => {
                // Смена формы (разовое ↔ повтор) снимает отметки сама — это
                // не «снова открыто».
                if b.is_repeating() == e.is_repeating() {
                    if b.done != e.done {
                        out.push(entry(if e.done { "done" } else { "reopen" }, e).from_to(String::new(), e.date.clone()));
                    }
                    for d in e.done_on.iter().filter(|d| !b.done_on.contains(d)) {
                        out.push(entry("done", e).from_to(String::new(), d.clone()));
                    }
                    for d in b.done_on.iter().filter(|d| !e.done_on.contains(d)) {
                        out.push(entry("reopen", e).from_to(String::new(), d.clone()));
                    }
                }
                if b.date != e.date {
                    out.push(entry("move", e).from_to(b.date.clone(), e.date.clone()));
                }
            }
        }
    }
    for b in before {
        if !after.iter().any(|e| e.id == b.id) {
            out.push(entry("delete", b).from_to(b.date.clone(), String::new()));
        }
    }
    out
}

/// Черновик события в попапе.
#[derive(Clone, Debug, PartialEq)]
pub struct EventDraft {
    /// `None` — новое событие.
    pub id: Option<String>,
    pub event: CalEvent,
    /// День вхождения, с которого открыт попап: «Сделано» у повтора
    /// отмечает этот повтор, а не всю серию.
    pub day: i64,
}

#[derive(Clone)]
pub struct CalendarHandle {
    doc: Arc<Mutex<CalendarDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    /// Выбранное событие (id) — поля в панели свойств.
    pub selected: RwSignal<Option<String>>,
    /// День выбранного вхождения: «сделано» повтора в панели свойств
    /// относится к нему.
    pub selected_at: RwSignal<Option<i64>>,
    /// Выбранный день (дни от эпохи) — подсветка в сетке.
    pub selected_day: RwSignal<Option<i64>>,
    /// Попап события: открыт ли, якорь, черновик.
    pub popup_open: RwSignal<bool>,
    pub popup_anchor: RwSignal<Rect>,
    pub draft: RwSignal<Option<EventDraft>>,
    /// Попап-список дня («ещё n»): открыт ли, день, якорь.
    pub day_popup_open: RwSignal<bool>,
    pub day_popup_day: RwSignal<Option<i64>>,
    pub day_popup_anchor: RwSignal<Rect>,
    /// Бамп — перерисовать сетки (внешний слой, «сейчас»).
    pub tick: RwSignal<u64>,
}

impl CalendarHandle {
    pub fn new(doc: CalendarDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            selected: use_signal(None),
            selected_at: use_signal(None),
            selected_day: use_signal(None),
            popup_open: use_signal(false),
            popup_anchor: use_signal(Rect::zero()),
            draft: use_signal(None),
            day_popup_open: use_signal(false),
            day_popup_day: use_signal(None),
            day_popup_anchor: use_signal(Rect::zero()),
            tick: use_signal(0),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, CalendarDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    /// Правка документа виджета: автосейв + перестройка.
    pub fn edit(&self, f: impl FnOnce(&mut CalendarDoc)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    pub fn view(&self) -> CalView {
        self.lock().view
    }

    pub fn set_view(&self, view: CalView) {
        if self.lock().view == view {
            return;
        }
        self.edit(|d| d.view = view);
    }

    pub fn anchor(&self) -> i64 {
        self.lock().anchor_days()
    }

    pub fn set_anchor(&self, day: i64) {
        if self.lock().anchor_days() == day {
            return;
        }
        self.edit(|d| d.anchor = super::gantt::calendar::days_to_iso(day));
    }

    /// Шаг вида: год / месяц / неделя / день.
    pub fn step(&self, delta: i64) {
        let (view, anchor) = {
            let d = self.lock();
            (d.view, d.anchor_days())
        };
        let next = match view {
            CalView::Day => anchor + delta,
            CalView::Week => anchor + 7 * delta,
            CalView::Month => {
                let (y, m, day) = super::gantt::calendar::civil_from_days(anchor);
                let (ny, nm) = model::add_months(y, m, delta);
                super::gantt::calendar::days_from_civil(ny, nm, day.min(model::days_in_month(ny, nm)))
            }
            CalView::Year => {
                let (y, m, day) = super::gantt::calendar::civil_from_days(anchor);
                super::gantt::calendar::days_from_civil(y + delta, m, day.min(model::days_in_month(y + delta, m)))
            }
        };
        self.set_anchor(next);
    }

    pub fn style(&self) -> CalendarStyle {
        self.lock().style.clone()
    }

    pub fn set_style(&self, f: impl FnOnce(&mut CalendarStyle)) {
        let mut s = self.style();
        f(&mut s);
        s.sanitize();
        if self.lock().style == s {
            return;
        }
        self.edit(|d| d.style = s);
    }

    pub fn set_calendars(&self, ids: Vec<String>) {
        if self.lock().calendars == ids {
            return;
        }
        self.edit(|d| d.calendars = ids);
    }

    pub fn toggle_calendar(&self, id: &str, all: &[String]) {
        let mut visible = {
            let d = self.lock();
            if d.calendars.is_empty() { all.to_vec() } else { d.calendars.clone() }
        };
        if let Some(pos) = visible.iter().position(|c| c == id) {
            visible.remove(pos);
        } else {
            visible.push(id.to_string());
        }
        // Все видны — фильтр пустой (новые календари попадают сами).
        let ids = if all.iter().all(|a| visible.contains(a)) { Vec::new() } else { visible };
        self.set_calendars(ids);
    }

    /// Доски-источники задач; пусто — все доски проекта.
    pub fn set_boards(&self, ids: Vec<String>) {
        if self.lock().boards == ids {
            return;
        }
        self.edit(|d| d.boards = ids);
    }

    pub fn toggle_board(&self, id: &str, all: &[String]) {
        let mut visible = {
            let d = self.lock();
            if d.boards.is_empty() { all.to_vec() } else { d.boards.clone() }
        };
        if let Some(pos) = visible.iter().position(|b| b == id) {
            visible.remove(pos);
        } else {
            visible.push(id.to_string());
        }
        // Видны все — фильтр пустой (новые доски попадают сами).
        let ids = if all.iter().all(|a| visible.contains(a)) { Vec::new() } else { visible };
        self.set_boards(ids);
    }

    pub fn select(&self, id: Option<String>) {
        if self.selected.get_untracked() != id {
            self.selected.set(id);
            if self.selected_at.get_untracked().is_some() {
                self.selected_at.set(None);
            }
        }
    }

    /// Выбрать событие вместе с днём вхождения (повтор в сетке).
    pub fn select_at(&self, id: String, day: i64) {
        self.select(Some(id));
        if self.selected_at.get_untracked() != Some(day) {
            self.selected_at.set(Some(day));
        }
    }

    /// Открыть попап нового события на день `day` (и слот `start`).
    pub fn open_new(&self, calendar: &str, day: i64, start: Option<u32>, slot_min: u32, anchor: Rect) {
        let mut e = CalEvent::new(calendar, "", day);
        if let Some(s) = start {
            e.start = Some(s);
            e.end = Some((s + slot_min).min(24 * 60));
            e.all_day = false;
        }
        self.close_day_popup();
        self.draft.set(Some(EventDraft { id: None, event: e, day }));
        self.popup_anchor.set(anchor);
        self.popup_open.set(true);
        self.select(None);
        self.selected_day.set(Some(day));
    }

    /// Открыть попап правки события с вхождения в день `day`.
    pub fn open_edit(&self, event: CalEvent, day: i64, anchor: Rect) {
        self.close_day_popup();
        self.select_at(event.id.clone(), day);
        self.draft.set(Some(EventDraft { id: Some(event.id.clone()), event, day }));
        self.popup_anchor.set(anchor);
        self.popup_open.set(true);
    }

    pub fn close_popup(&self) {
        if self.popup_open.get_untracked() {
            self.popup_open.set(false);
        }
        self.draft.set(None);
    }

    /// Открыть список дня (все события и задачи дня) у ячейки `anchor`.
    pub fn open_day(&self, day: i64, anchor: Rect) {
        self.close_popup();
        self.select(None);
        self.selected_day.set(Some(day));
        self.day_popup_day.set(Some(day));
        self.day_popup_anchor.set(anchor);
        self.day_popup_open.set(true);
    }

    pub fn close_day_popup(&self) {
        if self.day_popup_open.get_untracked() {
            self.day_popup_open.set(false);
        }
        if self.day_popup_day.get_untracked().is_some() {
            self.day_popup_day.set(None);
        }
    }

    pub fn bump_tick(&self) {
        self.tick.set(self.tick.get_untracked() + 1);
    }
}
