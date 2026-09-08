//! Виджет диаграммы Ганта: тулбар (новая задача, масштаб, «сегодня»),
//! слева колонка задач из обычных виджетов (цветная метка, название,
//! удаление), справа — шкала времени ([`super::chart`]). Строки колонки и
//! шкалы одной высоты, поэтому подписи и бары совпадают.
//!
//! После своих задач идут строки запланированных карточек выбранных досок
//! (`GanttDoc::boards`): подпись read-only с кнопкой «открыть страницу», а
//! бар живой — тянется и пишет новые даты в карточку.

use syngui::core::Color;
use syngui::input::CursorIcon;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::icons::*;

use super::chart::{GanttChart, HEADER_H, ROW_H};
use super::model::GanttTask;
use super::{BoardTask, GanttEnv, GanttHandle};

const LABEL_W: f32 = 200.0;

pub fn view(env: GanttEnv, handle: GanttHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.revision.get();
        let _ = env.project_rev.get();
        let go_today = handle.go_today.get();
        vec![build(env.clone(), handle.clone(), go_today)]
    })
}

fn build(env: GanttEnv, handle: GanttHandle, go_today: u64) -> Box<dyn Widget> {
    let (tasks, boards): (Vec<GanttTask>, Vec<String>) = {
        let doc = handle.lock();
        (doc.tasks.clone(), doc.boards.clone())
    };
    let cards = (env.cards)(&boards);

    let mut labels = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-gantt-labels")
        .child(
            DecoratedBox::new()
                .style("height", StyleValue::px(HEADER_H))
                .class("notes-gantt-labels-header")
                .child(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(tr!("notes.gantt.task")).class("notes-gantt-labels-title")),
                ),
        );
    for t in &tasks {
        labels = labels.child(task_row(&handle, t));
    }
    for c in &cards {
        labels = labels.child(card_row(&env, c));
    }
    if tasks.is_empty() && cards.is_empty() {
        labels = labels.child(
            DecoratedBox::new()
                .style("height", StyleValue::px(ROW_H))
                .child(Text::new(tr!("notes.gantt.empty")).max_lines(2).class("notes-empty-hint")),
        );
    }

    let body = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(DecoratedBox::new().style("width", StyleValue::px(LABEL_W)).child(labels))
        .child(
            DecoratedBox::new()
                .class("grow")
                .child(GanttChart { handle: handle.clone(), env: env.clone(), cards, go_today }.class("notes-gantt-chart")),
        );

    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(toolbar(&handle))
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(body))),
    )
}

/// Строка карточки доски: метка цвета, название read-only, «открыть
/// страницу доски» — сама карточка правится там, а не здесь.
fn card_row(env: &GanttEnv, c: &BoardTask) -> impl Widget {
    let open = env.open_page.clone();
    let page = c.page.clone();
    let mut dot = DecoratedBox::new().class("notes-kanban-dot");
    if c.color.is_empty() {
        dot = dot.class("notes-kanban-dot empty");
    } else {
        dot = dot.style("background-color", Color::from_hex(&c.color));
    }
    DecoratedBox::new()
        .style("height", StyleValue::px(ROW_H))
        .class("notes-gantt-row")
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(dot)
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(Text::new(c.name.clone()).max_lines(1).class("notes-gantt-task-name")),
                )
                .child(
                    ToolButton::new(MI_OPEN_IN_NEW)
                        .tooltip(tr!("notes.embed.open"))
                        .on_click(move || open(&page))
                        .class("notes-kanban-lane-btn"),
                ),
        )
}

fn toolbar(handle: &GanttHandle) -> impl Widget {
    let h_add = handle.clone();
    let h_out = handle.clone();
    let h_in = handle.clone();
    let h_today = handle.clone();
    Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-gantt-toolbar")
        .child(
            ToolButton::new(MI_ADD)
                .text(tr!("notes.gantt.add_task"))
                .tooltip(tr!("notes.gantt.add_task"))
                .on_click(move || {
                    h_add.add_task(&tr!("notes.gantt.new_task"));
                }),
        )
        .child(DecoratedBox::new().class("grow"))
        .child(
            ToolButton::new(MI_ZOOM_OUT)
                .tooltip(tr!("notes.gantt.zoom_out"))
                .on_click(move || h_out.set_zoom(h_out.zoom() / 1.25)),
        )
        .child(
            ToolButton::new(MI_ZOOM_IN)
                .tooltip(tr!("notes.gantt.zoom_in"))
                .on_click(move || h_in.set_zoom(h_in.zoom() * 1.25)),
        )
        .child(
            ToolButton::new(MI_TODAY)
                .tooltip(tr!("notes.gantt.today"))
                .on_click(move || h_today.show_today()),
        )
}

/// Строка задачи: метка цвета (клик — следующий), название, удалить.
fn task_row(handle: &GanttHandle, t: &GanttTask) -> impl Widget {
    let h_color = handle.clone();
    let id_color = t.id.clone();
    let h_name = handle.clone();
    let id_name = t.id.clone();
    let h_del = handle.clone();
    let id_del = t.id.clone();
    let mut dot = DecoratedBox::new().class("notes-kanban-dot");
    if t.color.is_empty() {
        dot = dot.class("notes-kanban-dot empty");
    } else {
        dot = dot.style("background-color", Color::from_hex(&t.color));
    }
    DecoratedBox::new()
        .style("height", StyleValue::px(ROW_H))
        .class("notes-gantt-row")
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    GestureDetector::new()
                        .cursor(CursorIcon::Pointer)
                        .on_click(move || h_color.cycle_task_color(&id_color))
                        .child(dot),
                )
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .text(t.name.clone())
                            .placeholder(tr!("notes.gantt.new_task"))
                            .submit_on_focus_lost(true)
                            .on_submit(move |v: &str| h_name.rename_task(&id_name, v))
                            .class("notes-gantt-task-name"),
                    ),
                )
                .child(
                    ToolButton::new(MI_CLOSE)
                        .tooltip(tr!("notes.gantt.delete_task"))
                        .on_click(move || h_del.delete_task(&id_del))
                        .class("notes-kanban-lane-btn"),
                ),
        )
}
