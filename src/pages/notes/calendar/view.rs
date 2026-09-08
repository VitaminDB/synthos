//! Виджет календаря внутри врезки: тулбар (виды, навигация, «сегодня»,
//! «+ событие»), тело — сетка выбранного вида, попап события.
//!
//! Снимок данных для сеток ([`GridData`]) собирается здесь на каждую
//! перестройку: документ виджета, хранилище событий, вхождения диапазона,
//! внешний слой (сроки досок / задачи Ганта), «сегодня» и «сейчас» по
//! локальному времени, локаль названий.

use syngui::core::{Color, Rect};
use syngui::mss::MssFields;
use syngui::prelude::*;
use syngui::widgets::{CalendarLocale, ToolButton};

use crate::icons::*;

use super::model::{days_in_month, range_of, CalView, CalendarDoc, CalendarStore, CalendarStyle, Occurrence};
use super::month::MonthGrid;
use super::timegrid::TimeGrid;
use super::year::YearGrid;
use super::{CalendarEnv, CalendarHandle, ExternalItem, ExternalQuery};
use crate::pages::notes::gantt::calendar::civil_from_days;

/// Снимок для сеток (клонируется в виджет на каждую перестройку).
#[derive(Clone)]
pub struct GridData {
    pub doc: CalendarDoc,
    pub store: CalendarStore,
    pub occurrences: Vec<Occurrence>,
    pub external: Vec<ExternalItem>,
    pub today: i64,
    pub now_min: u32,
    pub range: (i64, i64),
    pub locale: CalendarLocale,
    pub selected: Option<String>,
    pub selected_day: Option<i64>,
}

pub fn locale() -> CalendarLocale {
    syngui::widgets::default_locale()
}

pub fn snapshot(env: &CalendarEnv, handle: &CalendarHandle) -> GridData {
    let doc = handle.lock().clone();
    let store = env.store.lock().clone();
    let (today, now_min) = crate::agent::time::local_now();
    let range = range_of(doc.view, doc.anchor_days(), doc.style.first_weekday);
    let occurrences = store.occurrences(range.0, range.1, &doc.calendars);
    let external = (env.external)(&ExternalQuery {
        from: range.0,
        to: range.1,
        boards: doc.boards.clone(),
        due: doc.style.show_kanban_due,
        spans: doc.style.show_kanban_spans,
        gantt: doc.style.show_gantt,
    });
    GridData {
        doc,
        store,
        occurrences,
        external,
        today,
        now_min,
        range,
        locale: locale(),
        selected: handle.selected.get_untracked(),
        selected_day: handle.selected_day.get_untracked(),
    }
}

/// Цвета сетки: стиль виджета поверх MSS класса `notes-calendar-grid`.
pub struct Palette {
    pub text: Color,
    pub muted: Color,
    pub grid: Color,
    pub header_bg: Color,
    pub cell_bg: Color,
    pub weekend: Color,
    pub today: Color,
    pub accent: Color,
}

pub fn color_of(hex: &str) -> Option<Color> {
    let t = hex.trim();
    (t.starts_with('#') && (t.len() == 7 || t.len() == 9)).then(|| Color::from_hex(t))
}

impl Palette {
    pub fn resolve(style: &CalendarStyle, mss: &MssFields) -> Self {
        let text = color_of(&style.text_color).or(mss.color).unwrap_or_else(|| Color::from_hex("#E6E8EE"));
        let accent = mss.accent_color.unwrap_or_else(|| Color::from_hex("#4F8CFF"));
        let grid = color_of(&style.grid_color).or(mss.border_color).unwrap_or_else(|| text.with_alpha(0.12));
        let cell_bg = color_of(&style.cell_bg).or(mss.background_color).unwrap_or_else(|| Color::from_hex("#000000").with_alpha(0.0));
        Self {
            text,
            muted: text.with_alpha(0.55),
            grid,
            header_bg: color_of(&style.header_bg).unwrap_or_else(|| text.with_alpha(0.05)),
            cell_bg,
            weekend: color_of(&style.weekend_tint).unwrap_or_else(|| text.with_alpha(0.04)),
            today: color_of(&style.today_color).unwrap_or(accent),
            accent,
        }
    }
}

pub fn view(env: CalendarEnv, id: String, handle: CalendarHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let _ = env.store.revision.get();
        let _ = env.project_rev.get();
        let _ = handle.selected.get();
        let _ = handle.selected_day.get();
        let _ = handle.tick.get();
        let popup_open = handle.popup_open.get();
        let day_open = handle.day_popup_open.get();
        vec![build(&env, &id, handle.clone(), popup_open, day_open)]
    })
}

fn build(env: &CalendarEnv, _id: &str, handle: CalendarHandle, popup_open: bool, day_open: bool) -> Box<dyn Widget> {
    let data = snapshot(env, &handle);
    let view = data.doc.view;
    let body: Box<dyn Widget> = match view {
        CalView::Month => Box::new(MonthGrid { env: env.clone(), handle: handle.clone(), data: data.clone() }.class("notes-calendar-grid")),
        CalView::Year => Box::new(YearGrid { env: env.clone(), handle: handle.clone(), data: data.clone() }.class("notes-calendar-grid")),
        CalView::Week | CalView::Day => Box::new(
            ScrollView::new()
                .vertical()
                .class("notes-calendar-scroll")
                .child(TimeGrid { env: env.clone(), handle: handle.clone(), data: data.clone() }.class("notes-calendar-grid")),
        ),
    };
    let mut stack = Stack::new().clip(false).child(DecoratedBox::new().class("grow").child(crate::components::workspace_frame::expand(body)));
    if popup_open {
        stack = stack.child(super::popup::event_popup(env.clone(), handle.clone()));
    }
    if day_open {
        stack = stack.child(super::popup::day_popup(env.clone(), handle.clone(), &data));
    }
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(toolbar(env, &handle, &data))
            .child(DecoratedBox::new().class("grow").child(stack)),
    )
}

/// Подпись периода в тулбаре.
pub fn period_title(data: &GridData) -> String {
    let anchor = data.doc.anchor_days();
    let (y, m, d) = civil_from_days(anchor);
    match data.doc.view {
        CalView::Year => format!("{y}"),
        CalView::Month => data.locale.month_year_title(y as i32, m),
        CalView::Week => {
            let (from, to) = data.range;
            let (fy, fm, fd) = civil_from_days(from);
            let (_, tm, td) = civil_from_days(to);
            if fm == tm {
                format!("{fd}–{td} {} {fy}", data.locale.month_name(fm))
            } else {
                format!("{fd} {} – {td} {} {fy}", data.locale.month_short(fm), data.locale.month_short(tm))
            }
        }
        CalView::Day => format!("{d} {} {y}, {}", data.locale.month_name(m), data.locale.weekday_short(day_weekday(anchor))),
    }
}

pub fn day_weekday(day: i64) -> u32 {
    crate::pages::notes::gantt::calendar::weekday_of(day) as u32
}

fn toolbar(env: &CalendarEnv, handle: &CalendarHandle, data: &GridData) -> impl Widget {
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).class("notes-calendar-toolbar");
    for (view, icon, key) in [
        (CalView::Year, MI_DATE_RANGE, "notes.calendar.view.year"),
        (CalView::Month, MI_CALENDAR_MONTH, "notes.calendar.view.month"),
        (CalView::Week, MI_CALENDAR_VIEW_WEEK, "notes.calendar.view.week"),
        (CalView::Day, MI_CALENDAR_VIEW_DAY, "notes.calendar.view.day"),
    ] {
        let h = handle.clone();
        let selected = data.doc.view == view;
        row = row.child(
            ToolButton::new(icon)
                .tooltip(syngui::i18n::tr(key))
                .on_click(move || h.set_view(view))
                .class(if selected { "notes-calendar-view-btn selected" } else { "notes-calendar-view-btn" }),
        );
    }
    let h_prev = handle.clone();
    let h_next = handle.clone();
    let h_today = handle.clone();
    let today = data.today;
    row = row
        .child(DecoratedBox::new().style("width", syngui::mss::StyleValue::px(8.0)))
        .child(ToolButton::new(MI_CHEVRON_LEFT).tooltip(tr!("notes.calendar.prev")).on_click(move || h_prev.step(-1)))
        .child(Text::new(period_title(data)).max_lines(1).class("notes-calendar-title"))
        .child(ToolButton::new(MI_CHEVRON_RIGHT).tooltip(tr!("notes.calendar.next")).on_click(move || h_next.step(1)))
        .child(ToolButton::new(MI_TODAY).tooltip(tr!("notes.calendar.today")).on_click(move || h_today.set_anchor(today)))
        .child(DecoratedBox::new().class("grow"));
    // «+ событие»: на выбранный день либо якорь, в первый видимый календарь.
    let h_add = handle.clone();
    let store = env.store.clone();
    let slot = data.doc.style.slot_min;
    let first_cal = data.doc.calendars.first().cloned().or_else(|| data.store.calendars.first().map(|c| c.id.clone())).unwrap_or_default();
    let day = data.selected_day.unwrap_or_else(|| data.doc.anchor_days());
    row = row.child(
        ToolButton::new(MI_ADD)
            .text(tr!("notes.calendar.add_event"))
            .tooltip(tr!("notes.calendar.add_event"))
            .on_click(move || {
                let _ = &store;
                h_add.open_new(&first_cal, day, None, slot, Rect::new(Point::new(0.0, 40.0), Size::new(1.0, 1.0)));
            }),
    );
    row
}

/// Число дней в месяце якоря (для подписей).
pub fn month_len(day: i64) -> u32 {
    let (y, m, _) = civil_from_days(day);
    days_in_month(y, m)
}
