//! Правая колонка вкладки «Аудио модели» — список ASR-пресетов + кнопка «+».
//!
//! Структура повторяет [`crate::pages::settings::models::models_panel`],
//! но против сигнала `audio_models` / `selected_audio_model` и с иконкой
//! микрофона в шапке. ASR-серверы запускаются в редакторе модели
//! (см. `audio_models/mod.rs`) — здесь только выбор/создание.

use syngui::mgui;
use syngui::prelude::*;

use crate::config::AudioModelConfig;
use crate::context::AppCtx;
use crate::icons::*;

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("skills-panel models-panel")
        .child(mgui! {
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
                    Center::new().child(Icon::new(MI_HEADSET_MIC).class("skills-panel-header-icon")),
                ],
                DecoratedBox::new().class("grow").child(
                    Text::new("Аудио модели").class("skills-panel-header-title"),
                ),
                ToolButton::new(MI_ADD)
                    .on_click(add_audio_model)
                    .class("models-add-btn"),
            ]
        ]
    }
}

fn list_reactive() -> impl Widget {
    DecoratedBox::new().class("grow").child(move || {
        let ctx = use_context::<AppCtx>();
        let models = ctx.audio_models.get();
        let selected = ctx.selected_audio_model.get();

        let body: Box<dyn Widget> = if models.is_empty() {
            Box::new(Center::new().child(
                Padding::all(24.0).child(
                    Text::new("Нет ASR-моделей — нажмите «+»")
                        .class("models-empty-list"),
                ),
            ))
        } else {
            let items: Vec<ListItem> = models
                .iter()
                .map(|m| {
                    ListItem::new(m.name.clone())
                        .secondary(subtitle(m))
                        .icon(MI_HEADSET_MIC)
                })
                .collect();

            let selected_idx = selected
                .as_ref()
                .and_then(|name| models.iter().position(|m| &m.name == name));

            let models_clone = models.clone();
            let models_for_widget = models.clone();
            let selected_name = selected.clone();
            let list = ListView::new(items)
                .selection_mode(SelectionMode::Single)
                .item_height(82.0)
                .item_widget(move |idx, item, is_sel, _hover| {
                    let is_default = models_for_widget
                        .get(idx)
                        .map(|m| selected_name.as_deref() == Some(m.name.as_str()))
                        .unwrap_or(false);
                    row(item, is_sel, is_default)
                })
                .on_select(move |idx| {
                    if let Some(m) = models_clone.get(idx) {
                        let ctx = use_context::<AppCtx>();
                        ctx.selected_audio_model.set(Some(m.name.clone()));
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

fn row(item: &ListItem, is_selected: bool, is_default: bool) -> Box<dyn Widget> {
    let class = if is_selected {
        "skill-list-row selected"
    } else {
        "skill-list-row"
    };
    let title = item.text.clone();
    let subtitle = item.secondary_text.clone().unwrap_or_default();
    let icon = item
        .icon
        .clone()
        .unwrap_or_else(|| MI_HEADSET_MIC.to_string());

    let trailing: Box<dyn Widget> = if is_default {
        Box::new(
            DecoratedBox::new().class("models-active-chip").child(
                Padding::symmetric(8.0, 3.0).child(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(MI_CHECK))
                        .child(Text::new("default")),
                ),
            ),
        )
    } else {
        Box::new(DecoratedBox::new())
    };

    let icon_wrap: Box<dyn Widget> = Box::new(
        DecoratedBox::new()
            .class("skill-list-icon-wrap")
            .child(Center::new().child(Icon::new(icon).class("skill-list-icon"))),
    );
    let title_col: Box<dyn Widget> = Box::new(DecoratedBox::new().class("grow").child(
        Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Text::new(title).class("skill-list-title"))
            .child(Text::new(subtitle).class("skill-list-subtitle")),
    ));

    Box::new(Padding::symmetric(4.0, 5.0).child(
        DecoratedBox::new().class(class).child(
            Padding::symmetric(12.0, 10.0).child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![icon_wrap, title_col, trailing]),
            ),
        ),
    ))
}

fn subtitle(m: &AudioModelConfig) -> String {
    let lang = if m.language.is_empty() {
        "auto"
    } else {
        m.language.as_str()
    };
    let kind = m.kind.display_name();
    if m.model_path.is_empty() {
        format!("{kind} · {lang}")
    } else {
        let short = std::path::Path::new(&m.model_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&m.model_path)
            .to_string();
        format!("{short} · {kind} · {lang}")
    }
}

fn add_audio_model() {
    let ctx = use_context::<AppCtx>();
    let existing: std::collections::HashSet<String> = ctx
        .audio_models
        .get_untracked()
        .iter()
        .map(|m| m.name.clone())
        .collect();

    let mut name = "Новая аудио-модель".to_string();
    if existing.contains(&name) {
        let mut n = 2;
        loop {
            let candidate = format!("Новая аудио-модель {n}");
            if !existing.contains(&candidate) {
                name = candidate;
                break;
            }
            n += 1;
        }
    }

    let new_model = AudioModelConfig {
        name: name.clone(),
        ..Default::default()
    };

    ctx.audio_models.update(|list| list.push(new_model));
    ctx.selected_audio_model.set(Some(name));
}
