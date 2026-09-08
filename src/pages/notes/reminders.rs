//! Напоминания режима «Заметки»: сроки карточек и события календаря.
//!
//! Ничего не программируется — напоминания выводятся из данных проекта:
//! карточка с `due` (не закрытая) напоминает о себе в день срока, за день
//! до него и, пока просрочена, каждый день; событие — за N минут до начала
//! (у события на весь день — в то же «время сроков»). Настройки и список
//! уже показанного — `notes/reminders.json` в бандле ([`ReminderState`]),
//! чтобы после перезапуска не показывать одно и то же дважды.
//!
//! Проверка ([`check`]) идёт на main-потоке при запуске и дальше раз в
//! минуту ([`install_reminders`]): собирает актуальный список для
//! колокольчика в шапке (`NotesCtx.reminders`) и показывает тосты
//! приложения тем, чьё время пришло сегодня и кто ещё не показан.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use syngui::async_runtime::run_on_main_thread;
use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::input::Toggle;
use syngui::widgets::{Dropdown, DropdownItem, GestureDetector, PopupAnchor, PopupPanel, ToolButton};

use crate::context::AppCtx;
use crate::icons::*;

use super::autosave;
use super::calendar::model::{fmt_hm, parse_hm};
use super::gantt::calendar::{days_to_iso, parse_days};
use super::project;
use super::state::{object_refs, LiveObject, NotesCtx};

/// Настройки напоминаний проекта.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Время дня для сроков карточек и событий на весь день, минуты с полуночи.
    #[serde(default = "default_card_time")]
    pub card_time: u32,
    /// Напоминать и за день до срока.
    #[serde(default = "default_true")]
    pub day_before: bool,
    /// За сколько минут до начала события напоминать.
    #[serde(default = "default_event_before")]
    pub event_before_min: u32,
}

fn default_true() -> bool {
    true
}
fn default_card_time() -> u32 {
    9 * 60
}
fn default_event_before() -> u32 {
    15
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self { enabled: true, card_time: default_card_time(), day_before: true, event_before_min: default_event_before() }
    }
}

/// Файл `notes/reminders.json`: настройки + что показано (`ключ → когда`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderState {
    #[serde(default)]
    pub settings: ReminderSettings,
    #[serde(default)]
    pub fired: BTreeMap<String, String>,
}

impl ReminderState {
    pub fn parse(json: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Забыть показанное старше месяца (ключи кончаются датой показа).
    pub fn prune(&mut self, today: i64) {
        self.fired.retain(|k, _| k.rsplit(':').next().and_then(parse_days).is_none_or(|d| today - d <= 31));
    }
}

#[derive(Clone)]
pub struct ReminderStateHandle {
    state: Arc<Mutex<ReminderState>>,
    pub revision: RwSignal<u64>,
}

impl PartialEq for ReminderStateHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl ReminderStateHandle {
    pub fn new(state: ReminderState) -> Self {
        Self { state: Arc::new(Mutex::new(state)), revision: use_signal(0) }
    }

    pub fn lock(&self) -> MutexGuard<'_, ReminderState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    pub fn settings(&self) -> ReminderSettings {
        self.lock().settings.clone()
    }

    pub fn edit(&self, f: impl FnOnce(&mut ReminderState)) {
        f(&mut self.lock());
        self.revision.set(self.revision.get_untracked() + 1);
    }

    pub fn set_settings(&self, f: impl FnOnce(&mut ReminderSettings)) {
        let mut s = self.settings();
        f(&mut s);
        if self.lock().settings == s {
            return;
        }
        self.edit(|st| st.settings = s);
    }
}

/// Настройки и показанное: из пула, из бандла либо по умолчанию.
pub fn state(ctx: NotesCtx) -> ReminderStateHandle {
    if let Some(s) = ctx.reminder_state.get_untracked() {
        return s;
    }
    let path = ctx.project_path.get_untracked();
    let state = project::read_text(&path, project::REMINDERS_PATH)
        .and_then(|t| ReminderState::parse(&t).ok())
        .unwrap_or_default();
    autosave::mark_saved(project::REMINDERS_PATH, 0);
    let handle = ReminderStateHandle::new(state);
    ctx.reminder_state.set(Some(handle.clone()));
    handle
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReminderKind {
    Overdue,
    DueToday,
    DueTomorrow,
    /// Событие сегодня.
    Event,
    /// Событие завтра — в списке, без тоста.
    EventTomorrow,
}

/// Одно напоминание: что, где и когда его время.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// Ключ показа: `card:<id>:<вид>:<день>` / `event:<id>:<день>`.
    pub key: String,
    pub kind: ReminderKind,
    pub title: String,
    /// Подпись: доска / страница, время события, с какого дня просрочено.
    pub subtitle: String,
    /// Страница, которую открыть по клику.
    pub page: Option<String>,
    /// Когда показывать: день и минута.
    pub day: i64,
    pub at: u32,
}

/// Собрать напоминания на сегодня и завтра по всем доскам и календарю.
pub fn collect(ctx: NotesCtx, now: (i64, u32)) -> Vec<Reminder> {
    let settings = state(ctx).settings();
    let today = now.0;
    let mut out = Vec::new();
    let tree = ctx.tree.get_untracked();
    for pid in tree.all_ids() {
        let page_title = tree.title_of(&pid).unwrap_or_default();
        for (kind, oid) in object_refs(&ctx.page_markdown(&pid)) {
            if kind != "kanban" {
                continue;
            }
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &oid) else { continue };
            let doc = handle.lock();
            for c in doc.cards.iter().filter(|c| c.done.is_none()) {
                let Some(due) = c.due.as_deref().and_then(parse_days) else { continue };
                let title = if c.title.trim().is_empty() { c.md.lines().next().unwrap_or_default().to_string() } else { c.title.clone() };
                let (kind, sub) = if due < today {
                    (ReminderKind::Overdue, format!("{page_title} · {}", tr!("notes.reminders.since", date = short_date(due))))
                } else if due == today {
                    (ReminderKind::DueToday, page_title.clone())
                } else if due == today + 1 && settings.day_before {
                    (ReminderKind::DueTomorrow, page_title.clone())
                } else {
                    continue;
                };
                let slot = match kind {
                    ReminderKind::Overdue => "overdue",
                    ReminderKind::DueToday => "due",
                    _ => "before",
                };
                out.push(Reminder {
                    key: format!("card:{}:{slot}:{}", c.id, days_to_iso(today)),
                    kind,
                    title,
                    subtitle: sub,
                    page: Some(pid.clone()),
                    day: today,
                    at: settings.card_time,
                });
            }
        }
    }
    let store = ctx.calendar_store();
    let s = store.lock();
    for occ in s.occurrences(today, today + 1, &[]) {
        let Some(e) = s.event(&occ.event) else { continue };
        if e.done || !occ.first {
            continue;
        }
        let (kind, at) = if occ.day == today {
            (ReminderKind::Event, occ.time.map(|(from, _)| from.saturating_sub(settings.event_before_min)).unwrap_or(settings.card_time))
        } else {
            (ReminderKind::EventTomorrow, occ.time.map(|(from, _)| from).unwrap_or(settings.card_time))
        };
        let time = occ.time.map(|(from, to)| format!("{}–{}", fmt_hm(from), fmt_hm(to))).unwrap_or_else(|| tr!("notes.calendar.event.all_day"));
        let calendar = s.calendar(&e.calendar).map(|c| c.name.clone()).unwrap_or_default();
        out.push(Reminder {
            key: format!("event:{}:{}", e.id, days_to_iso(occ.day)),
            kind,
            title: e.title.clone(),
            subtitle: if calendar.is_empty() { time } else { format!("{time} · {calendar}") },
            page: e.link.clone(),
            day: occ.day,
            at,
        });
    }
    drop(s);
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.at.cmp(&b.at)).then(a.title.cmp(&b.title)));
    out
}

/// `ДД.ММ`.
fn short_date(days: i64) -> String {
    let (_, m, d) = super::gantt::calendar::civil_from_days(days);
    format!("{d:02}.{m:02}")
}

/// Сколько напоминаний «горит» (бейдж): просрочено, сегодня, события сегодня.
pub fn hot_count(list: &[Reminder]) -> usize {
    list.iter().filter(|r| matches!(r.kind, ReminderKind::Overdue | ReminderKind::DueToday | ReminderKind::Event)).count()
}

/// Проверка: обновить список и показать тосты тем, чьё время сегодня уже
/// пришло и кто ещё не показан. `notify` — как показать (в приложении —
/// тост, в тестах — сбор).
pub fn check(ctx: NotesCtx, now: (i64, u32), notify: &mut dyn FnMut(&Reminder)) {
    let list = collect(ctx, now);
    let state = state(ctx);
    let enabled = state.settings().enabled;
    let mut fired = Vec::new();
    if enabled {
        let known = state.lock().fired.clone();
        for r in list.iter().filter(|r| r.day == now.0 && r.at <= now.1 && !known.contains_key(&r.key)) {
            if matches!(r.kind, ReminderKind::EventTomorrow) {
                continue;
            }
            notify(r);
            fired.push(r.key.clone());
        }
    }
    if !fired.is_empty() {
        let stamp = super::activity::now_stamp();
        state.edit(|s| {
            for k in fired {
                s.fired.insert(k, stamp.clone());
            }
            s.prune(now.0);
        });
    }
    ctx.reminders.set(Arc::new(list));
}

/// Тост приложения по напоминанию; больше четырёх за раз — одним общим.
fn toast_batch(batch: &[Reminder]) {
    let app = use_context::<AppCtx>();
    let show = |r: &Reminder| {
        let title = match r.kind {
            ReminderKind::Overdue => tr!("notes.reminders.toast.overdue", title = r.title.clone()),
            ReminderKind::DueToday => tr!("notes.reminders.toast.due_today", title = r.title.clone()),
            ReminderKind::DueTomorrow => tr!("notes.reminders.toast.due_tomorrow", title = r.title.clone()),
            ReminderKind::Event | ReminderKind::EventTomorrow => r.title.clone(),
        };
        let item = syngui::widgets::feedback::NotificationItem::info(title).message(r.subtitle.clone()).duration_ms(30_000);
        app.notifications.show(item);
    };
    if batch.len() <= 4 {
        batch.iter().for_each(show);
    } else {
        batch.iter().take(3).for_each(show);
        app.notifications.show(
            syngui::widgets::feedback::NotificationItem::info(tr!("notes.reminders.toast.more", n = batch.len() - 3))
                .duration_ms(30_000),
        );
    }
}

/// Проверка «сейчас» на main-потоке с тостами приложения.
pub fn check_now() {
    let ctx = use_context::<NotesCtx>();
    let mut batch = Vec::new();
    check(ctx, crate::agent::time::local_now(), &mut |r| batch.push(r.clone()));
    if !batch.is_empty() {
        toast_batch(&batch);
    }
}

/// Таймер: первая проверка через несколько секунд после запуска, дальше раз
/// в минуту. Зовётся один раз из lib.rs после автосейва заметок.
pub fn install_reminders() {
    syngui::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(6)).await;
        loop {
            run_on_main_thread(check_now);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

// ─── Колокольчик и попап ─────────────────────────────────────────────────

/// Кнопка-колокольчик с числом горящих напоминаний (шапка центра).
pub fn bell(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let list = ctx.reminders.get();
        let hot = hot_count(&list);
        let icon = if hot > 0 { MI_NOTIFICATIONS_ACTIVE } else { MI_NOTIFICATIONS };
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(
            ToolButton::new(icon)
                .tooltip(tr!("notes.reminders.title"))
                .on_click_with_bounds(move |_, bounds| {
                    ctx.reminders_anchor.set(bounds);
                    ctx.reminders_open.set(!ctx.reminders_open.get_untracked());
                })
                .class(if hot > 0 { "notes-reminders-bell hot" } else { "notes-reminders-bell" }),
        );
        if hot > 0 {
            row = row.child(Text::new(hot.to_string()).class("notes-reminders-badge"));
        }
        vec![Box::new(row)]
    })
}

/// Попап со списком и настройками.
pub fn popup(ctx: NotesCtx) -> impl Widget {
    PopupPanel::new()
        .is_open(ctx.reminders_open)
        .anchor_rect(ctx.reminders_anchor)
        .anchor(PopupAnchor::BottomEnd)
        .min_width(340.0)
        .max_width(380.0)
        .max_height(520.0)
        .class("notes-reminders-popup")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            if !ctx.reminders_open.get() {
                return vec![Box::new(DecoratedBox::new())];
            }
            vec![Box::new(body(ctx))]
        }))
}

fn body(ctx: NotesCtx) -> impl Widget {
    let list = ctx.reminders.get();
    let settings = state(ctx).settings();
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let today = crate::agent::time::local_today_days();
    let header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(tr!("notes.reminders.title")).class("notes-reminders-heading"))
        .child(DecoratedBox::new().class("grow"))
        .child(
            ToolButton::new(MI_TODAY)
                .text(tr!("notes.journal.today"))
                .tooltip(tr!("notes.journal.today"))
                .on_click(move || {
                    let id = ctx.journal_page(today);
                    ctx.activate(&id);
                    ctx.reminders_open.set(false);
                })
                .class("notes-reminders-journal"),
        );
    col = col.child(header);
    if list.is_empty() {
        col = col.child(Text::new(tr!("notes.reminders.empty")).class("notes-reminders-empty"));
    }
    let sections = [
        (ReminderKind::Overdue, tr!("notes.reminders.overdue")),
        (ReminderKind::DueToday, tr!("notes.reminders.today")),
        (ReminderKind::Event, tr!("notes.reminders.events")),
        (ReminderKind::DueTomorrow, tr!("notes.reminders.tomorrow")),
        (ReminderKind::EventTomorrow, tr!("notes.reminders.events_tomorrow")),
    ];
    for (kind, label) in sections {
        let items: Vec<&Reminder> = list.iter().filter(|r| r.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        col = col.child(Text::new(format!("{label} · {}", items.len())).class("notes-links-section"));
        for r in items {
            col = col.child(row(ctx, r));
        }
    }
    // Настройки.
    let st = state(ctx);
    let s1 = st.clone();
    let s2 = st.clone();
    let s3 = st.clone();
    let s4 = st.clone();
    col = col
        .child(Text::new(tr!("notes.reminders.settings")).class("notes-links-section"))
        .child(setting_row(tr!("notes.reminders.enabled"), Toggle::with_state(settings.enabled).on_change(move |on| s1.set_settings(|s| s.enabled = on))))
        .child(setting_row(
            tr!("notes.reminders.time"),
            TextField::with_text(fmt_hm(settings.card_time))
                .width(72.0)
                .submit_on_focus_lost(true)
                .on_submit(move |t: &str| {
                    if let Some(m) = parse_hm(t) {
                        s2.set_settings(|s| s.card_time = m);
                    }
                })
                .class("notes-props-field"),
        ))
        .child(setting_row(tr!("notes.reminders.day_before"), Toggle::with_state(settings.day_before).on_change(move |on| s3.set_settings(|s| s.day_before = on))))
        .child(setting_row(
            tr!("notes.reminders.before_event"),
            Dropdown::with_items([0u32, 5, 10, 15, 30, 60].iter().map(|m| DropdownItem::new(m.to_string(), m.to_string())).collect())
                .selected(settings.event_before_min.to_string())
                .width(72.0)
                .on_change(move |v| {
                    if let Ok(m) = v.parse::<u32>() {
                        s4.set_settings(|s| s.event_before_min = m);
                    }
                })
                .class("notes-props-field"),
        ));
    col.class("notes-reminders-body")
}

fn row(ctx: NotesCtx, r: &Reminder) -> impl Widget {
    let icon = match r.kind {
        ReminderKind::Event | ReminderKind::EventTomorrow => MI_CALENDAR_MONTH,
        _ => MI_TODAY,
    };
    let class = match r.kind {
        ReminderKind::Overdue => "notes-reminders-row overdue",
        _ => "notes-reminders-row",
    };
    let page = r.page.clone();
    let text = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(Text::new(r.title.clone()).max_lines(2).class("notes-reminders-row-title"))
        .child(Text::new(r.subtitle.clone()).max_lines(1).class("notes-reminders-row-sub"));
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            if let Some(p) = &page {
                ctx.activate(p);
                crate::rail::navigate("notes");
            }
            ctx.reminders_open.set(false);
        })
        .child(
            DecoratedBox::new().class(class).child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icon).class("notes-reminders-row-icon"))
                    .child(DecoratedBox::new().class("grow").child(text)),
            ),
        )
}

fn setting_row(label: String, field: impl Widget + 'static) -> impl Widget {
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .class("notes-props-row")
        .child(Text::new(label).class("notes-props-row-label"))
        .child(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::pages::notes::calendar::model::CalEvent;
    use crate::pages::notes::kanban::model::KanbanCard;

    fn ctx() -> NotesCtx {
        let dir = std::env::temp_dir().join(format!("synthos-notes-reminders-{}", project::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = AppConfig {
            notes_project_path: dir.join("p.syn").display().to_string(),
            notes_vault_path: dir.join("no-vault").display().to_string(),
            ..AppConfig::default()
        };
        NotesCtx::new_or_restore(&cfg)
    }

    /// Просроченная, сегодняшняя и завтрашняя карточки плюс событие: список
    /// для колокольчика, тосты по времени и «не показывать дважды».
    #[test]
    fn collect_and_check_fire_once_per_day() {
        let ctx = ctx();
        let today = crate::agent::time::local_today_days();
        let page = ctx.insert_page(None, None, "Дела", false);
        let board = ctx.create_object("kanban").unwrap();
        ctx.set_page_markdown(&page, &format!("![[kanban:{board}]]"));
        let LiveObject::Kanban { handle, .. } = ctx.object("kanban", &board).unwrap() else { panic!() };
        let col = handle.lock().columns[0].id.clone();
        let done_col = handle.lock().columns[2].id.clone();
        for (title, due, column) in [("Просрочено", today - 2, &col), ("Сегодня", today, &col), ("Завтра", today + 1, &col), ("Закрыта", today, &done_col), ("Без срока", i64::MIN, &col)] {
            let mut c = KanbanCard::new(format!("k-{title}"), column.clone());
            c.title = title.to_string();
            if due != i64::MIN {
                c.due = Some(days_to_iso(due));
            }
            handle.edit(|d| d.cards.push(c));
        }
        let store = ctx.calendar_store();
        let cal = store.lock().calendars[0].id.clone();
        let mut ev = CalEvent::new(&cal, "Стендап", today);
        ev.start = Some(10 * 60);
        ev.end = Some(10 * 60 + 30);
        ev.all_day = false;
        store.add_event(ev);
        let mut tomorrow = CalEvent::new(&cal, "Врач", today + 1);
        tomorrow.start = Some(9 * 60);
        tomorrow.all_day = false;
        store.add_event(tomorrow);

        let list = collect(ctx, (today, 8 * 60));
        let kinds: Vec<(ReminderKind, &str)> = list.iter().map(|r| (r.kind, r.title.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (ReminderKind::Overdue, "Просрочено"),
                (ReminderKind::DueToday, "Сегодня"),
                (ReminderKind::DueTomorrow, "Завтра"),
                (ReminderKind::Event, "Стендап"),
                (ReminderKind::EventTomorrow, "Врач"),
            ]
        );
        assert_eq!(hot_count(&list), 3);
        assert!(list[0].subtitle.contains("Дела"), "{:?}", list[0]);
        assert_eq!(list[3].at, 10 * 60 - 15, "за 15 минут до начала");
        assert_eq!(list[1].at, 9 * 60);

        // В 08:00 ещё ничего не горит.
        let mut fired = Vec::new();
        check(ctx, (today, 8 * 60), &mut |r| fired.push(r.title.clone()));
        assert!(fired.is_empty(), "{fired:?}");
        assert_eq!(ctx.reminders.get_untracked().len(), 5);
        // В 09:50 — сроки (09:00) и событие (09:45); завтрашнее событие — нет.
        check(ctx, (today, 9 * 60 + 50), &mut |r| fired.push(r.title.clone()));
        assert_eq!(fired, ["Просрочено", "Сегодня", "Завтра", "Стендап"]);
        // Повторная проверка — тишина; состояние показанного сохранено.
        fired.clear();
        check(ctx, (today, 12 * 60), &mut |r| fired.push(r.title.clone()));
        assert!(fired.is_empty(), "{fired:?}");
        assert_eq!(state(ctx).lock().fired.len(), 4);
        // Выключено — список есть, тостов нет.
        state(ctx).set_settings(|s| s.enabled = false);
        check(ctx, (today + 1, 12 * 60), &mut |r| fired.push(r.title.clone()));
        assert!(fired.is_empty());
        assert!(!ctx.reminders.get_untracked().is_empty());
        // Настройка «за день до срока» выключена — завтрашней карточки в списке нет.
        state(ctx).set_settings(|s| s.day_before = false);
        let list = collect(ctx, (today, 8 * 60));
        assert!(list.iter().all(|r| r.kind != ReminderKind::DueTomorrow));
        // Файл состояния читается обратно.
        let json = state(ctx).serialize();
        let back = ReminderState::parse(&json).unwrap();
        assert_eq!(back.fired.len(), 4);
        assert!(!back.settings.enabled && !back.settings.day_before);
    }

    #[test]
    fn prune_keeps_a_month_of_fired_keys() {
        let mut st = ReminderState::default();
        st.fired.insert("card:k1:due:2026-08-01".into(), "x".into());
        st.fired.insert("card:k1:due:2026-09-07".into(), "x".into());
        st.fired.insert("event:e1:2026-09-08".into(), "x".into());
        st.prune(parse_days("2026-09-08").unwrap());
        let keys: Vec<&String> = st.fired.keys().collect();
        assert_eq!(keys, ["card:k1:due:2026-09-07", "event:e1:2026-09-08"]);
        // Дефолты и разбор пустого файла.
        let d = ReminderState::parse("{}").unwrap();
        assert_eq!(d.settings, ReminderSettings::default());
        assert_eq!(d.settings.card_time, 9 * 60);
    }
}
