//! Попап события: название, даты, время, «весь день», календарь, цвет,
//! повтор, заметка; кнопки «сохранить» / «сделано» / «удалить».
//!
//! Черновик живёт в `CalendarHandle::draft` и правится полями без
//! перестройки попапа (иначе поле теряло бы каретку на каждом символе);
//! запись в хранилище — по «Сохранить» (новое событие — `add_event`,
//! существующее — `update_event`).

use syngui::core::Rect;
use syngui::prelude::*;
use syngui::widgets::input::{DatePicker, Date, TimePicker, Toggle};
use syngui::widgets::input::document_editor::{DocumentEditor, DocumentEditorHandle};
use syngui::widgets::input::time_picker::Time;
use syngui::widgets::overlay::PopupPanel;
use syngui::widgets::{Dropdown, DropdownItem, GestureDetector, ToolButton};

use crate::icons::*;
use crate::pages::notes::gantt::calendar::{civil_from_days, days_from_civil, days_to_iso, parse_days};

use super::model::{CalEvent, Repeat};
use super::{CalendarEnv, CalendarHandle};

const PRESET_COLORS: &[&str] = &["", "#EE5E48", "#E8A33D", "#4FBF7A", "#4F8CFF", "#C08FE8", "#8B95A6", "#2EC4B6"];

fn date_of(iso: &str) -> Option<Date> {
    let days = parse_days(iso)?;
    let (y, m, d) = civil_from_days(days);
    Some(Date::new(y as i32, m, d))
}

fn iso_of(date: &Date) -> String {
    days_to_iso(days_from_civil(date.year as i64, date.month, date.day))
}

pub fn event_popup(env: CalendarEnv, handle: CalendarHandle) -> impl Widget {
    let h_close = handle.clone();
    PopupPanel::new()
        .is_open(handle.popup_open)
        .anchor_rect(handle.popup_anchor)
        // Ширина с запасом под пару полей в строке: «дата — конечная
        // дата» и «повтор — до» иначе не влезали, и плейсхолдер ломался
        // на две строки.
        .min_width(340.0)
        .max_width(420.0)
        .on_close(move || h_close.draft.set(None))
        .class("notes-calendar-popup")
        .child(body(env, handle))
}

fn body(env: CalendarEnv, handle: CalendarHandle) -> impl Widget {
    // Снимок черновика на момент открытия: поля правят сигнал, не перестраиваясь.
    let draft = handle.draft.get_untracked();
    let Some(draft) = draft else {
        return Column::new();
    };
    let e = draft.event.clone();
    let is_new = draft.id.is_none();
    let store = env.store.lock().clone();

    let edit = |handle: &CalendarHandle, f: Box<dyn Fn(&mut CalEvent) + Send + Sync>| {
        let h = handle.clone();
        move || h.draft.update(|d| if let Some(d) = d { f(&mut d.event) })
    };

    let h = handle.clone();
    let title = TextField::with_text(e.title.clone())
        .placeholder(tr!("notes.calendar.event.title_hint"))
        .autofocus(true)
        .on_change(move |t| {
            let t = t.to_string();
            h.draft.update(|d| {
                if let Some(d) = d {
                    d.event.title = t.clone();
                }
            });
        })
        .class("notes-calendar-popup-title");

    let h = handle.clone();
    let date = DatePicker::new()
        .width(140.0)
        .selected(date_of(&e.date).unwrap_or_else(Date::today))
        .on_change(move |d| {
            if let Some(d) = d {
                let iso = iso_of(&d);
                h.draft.update(|dr| {
                    if let Some(dr) = dr {
                        dr.event.date = iso.clone();
                    }
                });
            }
        });
    let h = handle.clone();
    let mut end_date = DatePicker::new().width(140.0).placeholder(tr!("notes.calendar.event.end_date")).on_change(move |d| {
        let iso = d.map(|d| iso_of(&d));
        h.draft.update(|dr| {
            if let Some(dr) = dr {
                dr.event.end_date = iso.clone();
            }
        });
    });
    if let Some(d) = e.end_date.as_deref().and_then(date_of) {
        end_date = end_date.selected(d);
    }

    let h = handle.clone();
    let all_day = Toggle::with_state(e.all_day).on_change(move |on| {
        h.draft.update(|dr| {
            if let Some(dr) = dr {
                dr.event.all_day = on;
                if on {
                    dr.event.start = None;
                    dr.event.end = None;
                } else if dr.event.start.is_none() {
                    dr.event.start = Some(9 * 60);
                    dr.event.end = Some(10 * 60);
                }
            }
        });
    });

    let time = |min: u32| Time::new(min / 60, min % 60);
    let h = handle.clone();
    let start = TimePicker::new()
        .use_24h(true)
        .width(96.0)
        .selected(time(e.start.unwrap_or(9 * 60)))
        .on_change(move |t| {
            if let Some(t) = t {
                let m = t.hour * 60 + t.minute;
                h.draft.update(|dr| {
                    if let Some(dr) = dr {
                        let dur = match (dr.event.start, dr.event.end) {
                            (Some(s), Some(e)) if e > s => e - s,
                            _ => 30,
                        };
                        dr.event.start = Some(m);
                        dr.event.end = Some((m + dur).min(24 * 60));
                        dr.event.all_day = false;
                    }
                });
            }
        });
    let h = handle.clone();
    let end = TimePicker::new()
        .use_24h(true)
        .width(96.0)
        .selected(time(e.end.unwrap_or(10 * 60)))
        .on_change(move |t| {
            if let Some(t) = t {
                let m = t.hour * 60 + t.minute;
                h.draft.update(|dr| {
                    if let Some(dr) = dr {
                        dr.event.end = Some(m.max(dr.event.start.unwrap_or(0) + 5));
                        dr.event.all_day = false;
                    }
                });
            }
        });

    let h = handle.clone();
    let calendar = Dropdown::new()
        .width(160.0)
        .items(store.calendars.iter().map(|c| DropdownItem::new(c.id.clone(), c.name.clone())).collect())
        .selected(e.calendar.clone())
        .on_change(move |v| {
            let v = v.to_string();
            h.draft.update(|dr| {
                if let Some(dr) = dr {
                    dr.event.calendar = v.clone();
                }
            });
        });

    let mut colors = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for preset in PRESET_COLORS {
        let value = preset.to_string();
        let selected = e.color == value;
        let mut dot = DecoratedBox::new().class(if selected { "notes-props-swatch selected" } else { "notes-props-swatch" });
        if value.is_empty() {
            dot = dot.class(if selected { "notes-props-swatch empty selected" } else { "notes-props-swatch empty" });
        } else {
            dot = dot.style("background-color", syngui::core::Color::from_hex(&value));
        }
        let h = handle.clone();
        colors = colors.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || {
                    let v = value.clone();
                    h.draft.update(|dr| {
                        if let Some(dr) = dr {
                            dr.event.color = v.clone();
                        }
                    });
                })
                .child(dot),
        );
    }

    let h = handle.clone();
    let repeat = Dropdown::new()
        .width(140.0)
        .items(Repeat::ALL.iter().map(|r| DropdownItem::new(r.key(), syngui::i18n::tr(&format!("notes.calendar.repeat.{}", r.key())))).collect())
        .selected(e.repeat.key())
        .on_change(move |v| {
            if let Some(r) = Repeat::parse(v) {
                h.draft.update(|dr| {
                    if let Some(dr) = dr {
                        dr.event.repeat = r;
                    }
                });
            }
        });
    let h = handle.clone();
    let mut until = DatePicker::new().width(140.0).placeholder(tr!("notes.calendar.event.until")).on_change(move |d| {
        let iso = d.map(|d| iso_of(&d));
        h.draft.update(|dr| {
            if let Some(dr) = dr {
                dr.event.until = iso.clone();
            }
        });
    });
    if let Some(d) = e.until.as_deref().and_then(date_of) {
        until = until.selected(d);
    }

    // Заметка — plain-редактор; текст стекает в черновик по ревизии ручки.
    let note_handle = DocumentEditorHandle::new();
    let h = handle.clone();
    let nh = note_handle.clone();
    create_effect(move || {
        if nh.revision().get() == 0 {
            return;
        }
        let md = nh.serialize();
        h.draft.update(|dr| {
            if let Some(dr) = dr {
                dr.event.note = md.clone();
            }
        });
    });
    let note = DecoratedBox::new().class("notes-calendar-note").child(
        DocumentEditor::new()
            .markdown(e.note.clone())
            .handle(&note_handle)
            .plain(true)
            .placeholder(tr!("notes.calendar.event.note_hint"))
            .class("notes-calendar-note-editor"),
    );

    // Кнопки.
    let h_save = handle.clone();
    let store_save = env.store.clone();
    let save = ToolButton::new(MI_CHECK).text(tr!("app.save")).on_click(move || {
        let Some(d) = h_save.draft.get_untracked() else { return };
        let mut ev = d.event.clone();
        if ev.title.trim().is_empty() {
            ev.title = tr!("notes.calendar.event.untitled");
        }
        match d.id {
            None => {
                let id = store_save.add_event(ev);
                h_save.select(Some(id));
            }
            Some(id) => {
                store_save.update_event(&id, |e| {
                    let keep = e.id.clone();
                    *e = ev.clone();
                    e.id = keep;
                });
            }
        }
        h_save.close_popup();
    });
    let h_done = handle.clone();
    let store_done = env.store.clone();
    let done_now = e.done;
    let id_done = draft.id.clone();
    let done = ToolButton::new(if done_now { MI_CHECK_BOX } else { MI_CHECK_BOX_OUTLINE_BLANK })
        .text(tr!("notes.calendar.event.done"))
        .on_click(move || {
            if let Some(id) = &id_done {
                store_done.update_event(id, |e| e.done = !done_now);
                h_done.close_popup();
            } else {
                h_done.draft.update(|dr| {
                    if let Some(dr) = dr {
                        dr.event.done = !dr.event.done;
                    }
                });
            }
        });
    let h_del = handle.clone();
    let store_del = env.store.clone();
    let id_del = draft.id.clone();
    let delete = ToolButton::new(MI_DELETE).tooltip(tr!("notes.calendar.event.delete")).on_click(move || {
        if let Some(id) = &id_del {
            store_del.remove_event(id);
        }
        h_del.select(None);
        h_del.close_popup();
    });
    let h_cancel = handle.clone();
    let cancel = ToolButton::new(MI_CLOSE).tooltip(tr!("app.cancel")).on_click(move || h_cancel.close_popup());

    let edit_once = edit(&handle, Box::new(|_| {}));
    let _ = edit_once;
    let mut buttons = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(save).child(done);
    if !is_new {
        buttons = buttons.child(delete);
    }
    buttons = buttons.child(DecoratedBox::new().class("grow")).child(cancel);

    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-calendar-popup-body")
        .child(title)
        .child(row(tr!("notes.calendar.event.date"), Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(date).child(end_date)))
        .child(row(tr!("notes.calendar.event.all_day"), all_day))
        .child(row(tr!("notes.calendar.event.time"), Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(start).child(Text::new("–").class("notes-calendar-popup-label")).child(end)))
        .child(row(tr!("notes.calendar.event.calendar"), calendar))
        .child(row(tr!("notes.props.color"), colors))
        .child(row(tr!("notes.calendar.event.repeat"), Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(repeat).child(until)))
        .child(note)
        .child(buttons)
}

fn row(label: String, field: impl Widget + 'static) -> impl Widget {
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().style("width", syngui::mss::StyleValue::px(88.0)).child(Text::new(label).max_lines(1).class("notes-calendar-popup-label")))
        .child(field)
}

#[allow(dead_code)]
fn anchor_default() -> Rect {
    Rect::zero()
}

// ─────────────────────────────────────────────────────────────────────────────
// Список дня
// ─────────────────────────────────────────────────────────────────────────────

/// Попап «ещё n»: все события и задачи дня списком — маркер цвета, время,
/// название; клик по событию открывает попап правки, по задаче — её
/// страницу; «+» — новое событие на этот день.
pub fn day_popup(env: CalendarEnv, handle: CalendarHandle, data: &super::view::GridData) -> impl Widget {
    let h_close = handle.clone();
    PopupPanel::new()
        .is_open(handle.day_popup_open)
        .anchor_rect(handle.day_popup_anchor)
        .anchor(syngui::widgets::overlay::menu::PopupAnchor::BottomStart)
        .min_width(260.0)
        .max_width(340.0)
        .on_close(move || h_close.day_popup_day.set(None))
        .class("notes-calendar-popup")
        .child(Stack::new().children(vec![day_body(env, handle, data)]))
}

fn day_body(env: CalendarEnv, handle: CalendarHandle, data: &super::view::GridData) -> Box<dyn Widget> {
    use super::layout::{segments, ItemRef};
    use super::model::fmt_hm;
    use super::view::{color_of, day_weekday};

    let Some(day) = handle.day_popup_day.get_untracked() else {
        return Box::new(Column::new());
    };
    let (_, m, d) = civil_from_days(day);
    let title = format!("{d} {}, {}", data.locale.month_name(m), data.locale.weekday_short(day_weekday(day)));
    let anchor = handle.day_popup_anchor.get_untracked();

    let h_add = handle.clone();
    let slot = data.doc.style.slot_min;
    let first_cal = data.doc.calendars.first().cloned().or_else(|| data.store.calendars.first().map(|c| c.id.clone())).unwrap_or_default();
    let add = ToolButton::new(MI_ADD).tooltip(tr!("notes.calendar.add_event")).on_click(move || h_add.open_new(&first_cal, day, None, slot, anchor));
    let h_x = handle.clone();
    let close = ToolButton::new(MI_CLOSE).tooltip(tr!("app.cancel")).on_click(move || h_x.close_day_popup());
    let header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title).max_lines(1).class("notes-calendar-day-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(add)
        .child(close);

    let accent = "#4F8CFF".to_string();
    let mut list = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let mut n = 0usize;
    for s in segments(&data.occurrences, &data.external).into_iter().filter(|s| s.covers(day)) {
        n += 1;
        let (color, name, done, is_event) = match &s.item {
            ItemRef::Event(id) => {
                let e = data.store.event(id);
                (e.map(|e| data.store.color_of(e)).unwrap_or_else(|| accent.clone()), e.map(|e| e.title.clone()).unwrap_or_default(), e.is_some_and(|e| e.done), true)
            }
            ItemRef::External(i) => {
                let ext = &data.external[*i];
                (ext.color.clone(), ext.title.clone(), false, false)
            }
        };
        let when = match s.time {
            Some((a, b)) => format!("{}–{}", fmt_hm(a), fmt_hm(b)),
            None => tr!("notes.calendar.day.all_day"),
        };
        let marker = DecoratedBox::new()
            .class(if is_event { "notes-calendar-day-marker" } else { "notes-calendar-day-marker task" })
            .style("background-color", color_of(&color).unwrap_or_else(|| syngui::core::Color::from_hex(&accent)));
        let row = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(marker)
            .child(Text::new(when).max_lines(1).class("notes-calendar-day-time"))
            .child(Text::new(name).max_lines(1).class(if done { "notes-calendar-day-name done" } else { "notes-calendar-day-name" }));
        let h = handle.clone();
        let env = env.clone();
        let item = s.item.clone();
        let event = match &s.item {
            ItemRef::Event(id) => data.store.event(id).cloned(),
            ItemRef::External(_) => None,
        };
        let page = match &s.item {
            ItemRef::External(i) => data.external.get(*i).map(|e| e.page.clone()),
            ItemRef::Event(_) => None,
        };
        list = list.child(
            GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || match (&item, &event, &page) {
                    (ItemRef::Event(_), Some(e), _) => h.open_edit(e.clone(), anchor),
                    (ItemRef::External(_), _, Some(p)) => {
                        h.close_day_popup();
                        (env.open_page)(p);
                    }
                    _ => {}
                })
                .child(DecoratedBox::new().class("notes-calendar-day-row").child(row)),
        );
    }
    if n == 0 {
        list = list.child(Text::new(tr!("notes.calendar.day.empty")).class("notes-calendar-popup-label"));
    }
    // Длинный список прокручивается, а не растягивает попап за экран.
    let body: Box<dyn Widget> = if n > 10 {
        Box::new(
            DecoratedBox::new()
                .style("height", syngui::mss::StyleValue::px(10.0 * 30.0))
                .child(ScrollView::new().vertical().child(list)),
        )
    } else {
        Box::new(list)
    };
    Box::new(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-calendar-popup-body")
            .child(header)
            .child(Stack::new().children(vec![body])),
    )
}
