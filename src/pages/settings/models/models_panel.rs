//! Правая колонка на вкладке «Модели» — список пресетов + кнопка «+».
//!
//! Композиция повторяет `skills_panel`, но содержимое реактивно строится из
//! `ctx.models`. Клик по строке выбирает модель (`ctx.selected_model`), клик
//! по «+» добавляет новый пустой пресет с уникальным именем и сразу выделяет.

use syngui::mgui;
use syngui::prelude::*;

use crate::config::ModelConfig;
use crate::context::AppCtx;
use crate::icons::*;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("skills-panel models-panel").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            header(),
            DecoratedBox::new().class("grow").child(
                Padding::symmetric(8.0, 8.0).child(list_reactive()),
            ),
        ]
    })
}

fn header() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("skills-panel-header") => [
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("skills-panel-header-icon-wrap") => [
                    Center::new().child(Icon::new(MI_MEMORY).class("skills-panel-header-icon")),
                ],
                DecoratedBox::new().class("grow").child(
                    Text::new("Модели").class("skills-panel-header-title"),
                ),
                ToolButton::new(MI_ADD)
                    .on_click(add_model)
                    .class("models-add-btn"),
            ]
        ]
    }
}

fn list_reactive() -> impl Widget {
    DecoratedBox::new().class("grow").child(move || {
        let ctx = use_context::<AppCtx>();
        let models = ctx.models.get();
        let selected = ctx.selected_model.get();

        let body: Box<dyn Widget> = if models.is_empty() {
            Box::new(Center::new().child(
                Padding::all(24.0).child(
                    Text::new("Нет пресетов — нажмите «+»").class("models-empty-list"),
                ),
            ))
        } else {
            let items: Vec<ListItem> = models
                .iter()
                .map(|m| ListItem::new(m.name.clone()).secondary(subtitle(m)).icon(MI_MEMORY))
                .collect();

            let selected_idx = selected
                .as_ref()
                .and_then(|name| models.iter().position(|m| &m.name == name));

            let models_clone = models.clone();
            let list = ListView::new(items)
                .selection_mode(SelectionMode::Single)
                .item_height(82.0)
                .item_widget(move |_idx, item, is_sel, _hover| row(item, is_sel))
                .on_select(move |idx| {
                    if let Some(m) = models_clone.get(idx) {
                        let ctx = use_context::<AppCtx>();
                        ctx.selected_model.set(Some(m.name.clone()));
                    }
                })
                .class("skills-list");

            let list = match selected_idx {
                Some(i) => list.selected(vec![i]),
                None => list,
            };
            Box::new(list)
        };

        Stack::new().fit(StackFit::Expand).children(vec![body])
    })
}

fn row(item: &ListItem, is_selected: bool) -> Box<dyn Widget> {
    let class = if is_selected { "skill-list-row selected" } else { "skill-list-row" };
    let title = item.text.clone();
    let subtitle = item.secondary_text.clone().unwrap_or_default();
    let icon = item.icon.clone().unwrap_or_else(|| MI_MEMORY.to_string());

    // Наружный Padding даёт визуальный зазор между items — аналог
    // Column::gap у .settings-sidebar. ListView своего gap не имеет.
    Box::new(Padding::symmetric(4.0, 5.0).child(
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
                ]
            ]
        }),
    ))
}

fn subtitle(m: &ModelConfig) -> String {
    let n_params = m.active_params.len();
    if m.model_path.is_empty() {
        format!("ctx {} · {} парам.", m.ctx_size, n_params)
    } else {
        let short = std::path::Path::new(&m.model_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&m.model_path)
            .to_string();
        format!("{short} · ctx {} · {} парам.", m.ctx_size, n_params)
    }
}

fn add_model() {
    let ctx = use_context::<AppCtx>();
    let existing: std::collections::HashSet<String> = ctx
        .models
        .get_untracked()
        .iter()
        .map(|m| m.name.clone())
        .collect();

    // Уникальное имя: «Новая модель», затем «Новая модель 2», 3, ...
    let mut name = "Новая модель".to_string();
    if existing.contains(&name) {
        let mut n = 2;
        loop {
            let candidate = format!("Новая модель {n}");
            if !existing.contains(&candidate) {
                name = candidate;
                break;
            }
            n += 1;
        }
    }

    let new_model = ModelConfig {
        name: name.clone(),
        ..Default::default()
    };

    ctx.models.update(|list| list.push(new_model));
    ctx.selected_model.set(Some(name));
}
