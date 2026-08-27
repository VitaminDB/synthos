//! Правая колонка на вкладке «Скилы» — список скилов с CRUD-actions.
//!
//! Источник данных — `AppCtx.skills` (грузится один раз при старте через
//! `crate::skills::load_all()`). Заголовок панели содержит кнопку «+» —
//! открывает диалог создания. Каждый row имеет на hover trailing-actions:
//! редактировать (rename) и удалить. Действия CRUD проходят через диалог
//! из `pages::settings::skills_dialog`, который монтируется один раз в
//! `pages::settings::view()`.

use syngui::mgui;
use syngui::prelude::*;

use crate::context::{AppCtx, SkillDialogKind};
use crate::icons::*;
use crate::skills::Skill;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("skills-panel").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("grow").child(move || {
                let body = list_body();
                Padding::symmetric(4.0, 4.0).child(
                    Stack::new().fit(StackFit::Expand).children(vec![body]),
                )
            }),
        ]
    })
}

/// Заголовок панели — в общей строке заголовков каркаса настроек.
pub fn header() -> impl Widget {
    DecoratedBox::new().class("skills-panel-header").child(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("skills-panel-header-icon-wrap") => [
                Center::new().child(Icon::new(MI_PSYCHOLOGY).class("skills-panel-header-icon")),
            ],
            DecoratedBox::new().class("grow").child(
                Text::new(tr!("settings.skills.panel.title")).class("skills-panel-header-title"),
            ),
            ToolButton::new(MI_ADD)
                .tooltip(tr!("settings.skills.panel.create"))
                .on_click(open_create_dialog)
                .class("skills-panel-add-btn"),
        ]
    })
}

fn open_create_dialog() {
    let ctx = use_context::<AppCtx>();
    ctx.skills_dialog.set(Some(SkillDialogKind::Create));
}

/// Реактивное тело панели — Reactive подписан на `skills` и
/// `skills_selected_id`. На пустой список рисуется placeholder.
fn list_body() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let skills: Vec<Skill> = ctx.skills.get();
    let selected = ctx.skills_selected_id.get();

    if skills.is_empty() {
        return Box::new(empty_state());
    }

    let items: Vec<ListItem> = skills
        .iter()
        .map(|s| ListItem::new(s.name.clone()).secondary(s.description.clone()).icon(MI_CODE))
        .collect();

    let initial_selection = selected
        .as_ref()
        .and_then(|id| skills.iter().position(|s| &s.id == id))
        .map(|idx| vec![idx])
        .unwrap_or_default();

    let skills_for_callback = skills.clone();
    let skills_for_widget = skills.clone();

    let list = ListView::new(items)
        .selection_mode(SelectionMode::Single)
        .selected(initial_selection)
        .item_height(78.0)
        .item_widget(move |idx, item, is_selected, _is_hover| {
            let id = skills_for_widget
                .get(idx)
                .map(|s| s.id.clone())
                .unwrap_or_default();
            let name = skills_for_widget
                .get(idx)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            skill_row(item, id, name, is_selected)
        })
        .on_select(move |idx| {
            if let Some(s) = skills_for_callback.get(idx) {
                let ctx = use_context::<AppCtx>();
                ctx.skills_selected_id.set(Some(s.id.clone()));
            }
        })
        .class("skills-list");

    Box::new(list)
}

fn empty_state() -> impl Widget {
    Center::new().child(mgui! {
        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("skill-empty-bubble") => [
                Center::new().child(Icon::new(MI_PSYCHOLOGY).class("skill-empty-icon")),
            ],
            Text::new(tr!("settings.skills.panel.empty.title")).class("skill-empty-title"),
            Padding::symmetric(20.0, 0.0).child(
                Text::new(tr!("settings.skills.panel.empty.text"))
                    .class("skill-empty-text"),
            ),
        ]
    })
}

fn skill_row(item: &ListItem, id: String, name: String, is_selected: bool) -> Box<dyn Widget> {
    let class = if is_selected {
        "skill-list-row selected"
    } else {
        "skill-list-row"
    };
    let title = item.text.clone();
    let subtitle = item.secondary_text.clone().unwrap_or_default();
    let icon = item.icon.clone().unwrap_or_else(|| MI_CODE.to_string());

    let id_for_edit = id.clone();
    let name_for_edit = name.clone();
    let desc_for_edit = subtitle.clone();
    let id_for_delete = id;
    let name_for_delete = name;

    Box::new(Padding::symmetric(4.0, 3.0).child(
        DecoratedBox::new().class(class).child(mgui! {
            Padding::symmetric(12.0, 10.0) => [
                Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    DecoratedBox::new().class("skill-list-icon-wrap") => [
                        Center::new().child(Icon::new(icon).class("skill-list-icon")),
                    ],
                    DecoratedBox::new().class("grow").child(mgui! {
                        Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                            Text::new(title).class("skill-list-title"),
                            Text::new(subtitle).class("skill-list-subtitle"),
                        ]
                    }),
                    ToolButton::new(MI_EDIT_NOTE)
                        .tooltip(tr!("settings.skills.edit_meta"))
                        .on_click(move || {
                            let ctx = use_context::<AppCtx>();
                            ctx.skills_dialog.set(Some(SkillDialogKind::Edit {
                                id: id_for_edit.clone(),
                                current_name: name_for_edit.clone(),
                                current_description: desc_for_edit.clone(),
                            }));
                        })
                        .class("skill-list-action"),
                    ToolButton::new(MI_DELETE)
                        .tooltip(tr!("app.delete"))
                        .on_click(move || {
                            let ctx = use_context::<AppCtx>();
                            ctx.skills_dialog.set(Some(SkillDialogKind::Delete {
                                id: id_for_delete.clone(),
                                name: name_for_delete.clone(),
                            }));
                        })
                        .class("skill-list-action danger"),
                ]
            ]
        }),
    ))
}
