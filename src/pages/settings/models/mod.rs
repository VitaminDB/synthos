//! Подстраница «Модели» — редактор пресетов llama.cpp.
//!
//! Главная колонка содержит:
//! 1. Поле имени модели.
//! 2. Два поля-пути: к GGUF-файлу и к mmproj.
//! 3. Карточка «Активные настройки» — ctx_size (неудаляемая) + активные чипы
//!    с контролами SpinBox / Toggle / TextField / Dropdown.
//! 4. Divider.
//! 5. Пул «Доступные настройки» — сетка чипов, сгруппированных по категориям,
//!    с Info Tooltip для каждого; клик — перенос в «Активные».

pub mod llama_params;
pub mod models_panel;

use syngui::mgui;
use syngui::prelude::*;

use crate::config::{ActiveParam, ModelConfig, ParamValue};
use crate::context::AppCtx;
use crate::icons::*;

use llama_params::{
    default_value, LlamaParam, ParamCategory, ParamKind, LLAMA_PARAMS,
};

// ─────────────────────────────────────────────────────────────────────────────
// Корневой view
// ─────────────────────────────────────────────────────────────────────────────

pub fn view() -> impl Widget {
    DecoratedBox::new().class("settings-page models-page").child(move || {
        let ctx = use_context::<AppCtx>();
        let models = ctx.models.get();
        let selected = ctx.selected_model.get();

        let idx = selected
            .as_ref()
            .and_then(|name| models.iter().position(|m| &m.name == name));

        let child: Box<dyn Widget> = match idx {
            Some(i) => Box::new(editor(i, models[i].clone())),
            None => Box::new(empty_state()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn empty_state() -> impl Widget {
    mgui! {
        Center::new() => [
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("models-empty-bubble") => [
                    Center::new().child(Icon::new(MI_SMART_TOY).class("models-empty-icon")),
                ],
                Text::new("Выберите модель").class("models-empty-title"),
                Padding::symmetric(32.0, 0.0).child(
                    Text::new("Справа — список пресетов llama.cpp. Нажмите «+», чтобы добавить новую модель, или выберите существующую.")
                        .class("models-empty-text"),
                ),
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Редактор
// ─────────────────────────────────────────────────────────────────────────────

fn editor(idx: usize, model: ModelConfig) -> impl Widget {
    let name_initial = model.name.clone();
    let path_initial = model.model_path.clone();
    let mmproj_initial = model.mmproj_path.clone();

    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    // ── заголовок: имя модели ──
                    DecoratedBox::new().class("models-card models-card-header") => [
                        Padding::all(20.0) => [
                            Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                                DecoratedBox::new().class("models-header-icon-wrap") => [
                                    Center::new().child(Icon::new(MI_SMART_TOY).class("models-header-icon")),
                                ],
                                DecoratedBox::new().class("grow") => [
                                    Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                        Text::new("Имя модели").class("models-field-label"),
                                        TextField::with_text(name_initial)
                                            .placeholder("Например: Qwen 2.5 Coder 7B")
                                            .on_change(move |s| {
                                                let s = s.to_string();
                                                let ctx = use_context::<AppCtx>();
                                                let old_name = ctx.models.get_untracked()
                                                    .get(idx).map(|m| m.name.clone());
                                                ctx.models.update(|list| {
                                                    if let Some(m) = list.get_mut(idx) {
                                                        m.name = s.clone();
                                                    }
                                                });
                                                // держим selected_model в синхроне
                                                if ctx.selected_model.get_untracked() == old_name {
                                                    ctx.selected_model.set(Some(s));
                                                }
                                            })
                                            .class("models-name-field"),
                                    ]
                                ],
                                Button::new("Удалить")
                                    .icon(MI_DELETE)
                                    .on_click(move || {
                                        let ctx = use_context::<AppCtx>();
                                        let removed_name = ctx.models.get_untracked()
                                            .get(idx).map(|m| m.name.clone());
                                        ctx.models.update(|list| {
                                            if idx < list.len() { list.remove(idx); }
                                        });
                                        if ctx.selected_model.get_untracked() == removed_name {
                                            ctx.selected_model.set(None);
                                        }
                                    })
                                    .class("models-delete-btn"),
                            ]
                        ]
                    ],

                    // ── секция: пути ──
                    section_card("Пути", vec![
                        path_row(MI_FOLDER_OPEN, "Путь к модели",
                            "Локальный .gguf или URL — используется как --model",
                            path_initial, true, idx),
                        path_row(MI_IMAGE_ICON, "Путь к mmproj",
                            "Multimodal projector (vision) — опционально",
                            mmproj_initial, false, idx),
                    ]),

                    // ── секция: активные настройки (ctx_size + чипы) ──
                    active_section(idx, &model),

                    Divider::horizontal().class("models-divider"),

                    // ── секция: доступные настройки (пул чипов) ──
                    available_section(&model),
                ]
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Секция путей (model_path / mmproj_path)
// ─────────────────────────────────────────────────────────────────────────────

fn path_row(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    initial: String,
    is_model: bool,
    idx: usize,
) -> Box<dyn Widget> {
    let field = TextField::with_text(initial)
        .placeholder("/path/to/model.gguf")
        .on_change(move |s| {
            let s = s.to_string();
            write_path(idx, is_model, s);
        })
        .class("models-path-field");

    let browse = ToolButton::new(MI_FOLDER_OPEN)
        .on_click(move || {
            // Нативный file picker через xdg portal (Linux) — блокирует
            // event loop на время диалога, аналогично modal dialog'у,
            // поведение ожидаемое.
            let mut dlg = rfd::FileDialog::new();
            if is_model {
                dlg = dlg.add_filter("GGUF model", &["gguf"]);
            } else {
                dlg = dlg.add_filter("mmproj", &["gguf", "bin"]);
            }
            if let Some(path) = dlg.pick_file() {
                write_path(idx, is_model, path.display().to_string());
            }
        })
        .class("models-path-browse");

    let control = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(field) as Box<dyn Widget>,
            Box::new(browse) as Box<dyn Widget>,
        ]);

    row_frame(icon, title, desc, Box::new(control))
}

fn write_path(idx: usize, is_model: bool, s: String) {
    let ctx = use_context::<AppCtx>();
    ctx.models.update(|list| {
        if let Some(m) = list.get_mut(idx) {
            if is_model { m.model_path = s.clone(); }
            else        { m.mmproj_path = s.clone(); }
        }
    });
}

fn row_frame(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    control: Box<dyn Widget>,
) -> Box<dyn Widget> {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);

    Box::new(
        DecoratedBox::new().class("settings-row").child(
            Padding::symmetric(24.0, 18.0).child(inner),
        ),
    )
}

fn section_card(title: &'static str, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );
    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("settings-section-title"))
        .child(card)
}

// ─────────────────────────────────────────────────────────────────────────────
// Активные настройки
// ─────────────────────────────────────────────────────────────────────────────

fn active_section(idx: usize, model: &ModelConfig) -> impl Widget {
    let mut rows: Vec<Box<dyn Widget>> = Vec::new();

    // ctx_size — всегда первая, всегда неудаляемая.
    if let Some(param) = llama_params::by_key("ctx_size") {
        rows.push(ctx_size_row(idx, model.ctx_size, param));
    }

    for ap in &model.active_params {
        if let Some(param) = llama_params::by_key(&ap.key) {
            rows.push(active_param_row(idx, param, ap.value.clone()));
        }
    }

    section_card("Активные настройки", rows)
}

fn ctx_size_row(idx: usize, current: u32, param: &'static LlamaParam) -> Box<dyn Widget> {
    let control: Box<dyn Widget> = Box::new(
        SpinBox::new()
            .value(current as f64)
            .range(0.0, 1_000_000.0)
            .step(512.0)
            .decimal_places(0)
            .on_change(move |v| {
                let new_val = v.max(0.0) as u32;
                let ctx = use_context::<AppCtx>();
                ctx.models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.ctx_size = new_val;
                    }
                });
            })
            .width(140.0)
            .class("models-active-control"),
    );

    // ctx_size — без кнопки удаления (неудаляемый чип).
    let chip = Chip::new(param.label).selected(true).class("models-active-chip");
    let chip_cell = wrap_chip_with_tooltip(param, chip);

    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            chip_cell,
            Box::new(DecoratedBox::new().class("grow")),
            control,
        ]);

    Box::new(
        DecoratedBox::new().class("models-active-row").child(
            Padding::symmetric(20.0, 14.0).child(inner),
        ),
    )
}

fn active_param_row(
    model_idx: usize,
    param: &'static LlamaParam,
    current: ParamValue,
) -> Box<dyn Widget> {
    let key = param.key;
    let chip = Chip::new(param.label)
        .selected(true)
        .deletable()
        .on_delete(move || {
            let ctx = use_context::<AppCtx>();
            ctx.models.update(|list| {
                if let Some(m) = list.get_mut(model_idx) {
                    m.active_params.retain(|p| p.key != key);
                }
            });
        })
        .class("models-active-chip");

    let control = build_control(model_idx, param, current);
    let chip_cell = wrap_chip_with_tooltip(param, chip);

    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            chip_cell,
            Box::new(DecoratedBox::new().class("grow")),
            control,
        ]);

    Box::new(
        DecoratedBox::new().class("models-active-row").child(
            Padding::symmetric(20.0, 14.0).child(inner),
        ),
    )
}

fn build_control(
    model_idx: usize,
    param: &'static LlamaParam,
    current: ParamValue,
) -> Box<dyn Widget> {
    let key = param.key;
    match &param.kind {
        ParamKind::Int { min, max, step, .. } => {
            let current_num = match &current {
                ParamValue::Int(v) => *v as f64,
                _ => 0.0,
            };
            Box::new(
                SpinBox::new()
                    .value(current_num)
                    .range(*min as f64, *max as f64)
                    .step(*step as f64)
                    .decimal_places(0)
                    .on_change(move |v| update_param(model_idx, key, ParamValue::Int(v as i64)))
                    .width(140.0)
                    .class("models-active-control"),
            )
        }
        ParamKind::Float { min, max, step, decimals, .. } => {
            let current_num = match &current {
                ParamValue::Float(v) => *v,
                _ => 0.0,
            };
            Box::new(
                SpinBox::new()
                    .value(current_num)
                    .range(*min, *max)
                    .step(*step)
                    .decimal_places(*decimals)
                    .on_change(move |v| update_param(model_idx, key, ParamValue::Float(v)))
                    .width(140.0)
                    .class("models-active-control"),
            )
        }
        ParamKind::Bool { .. } => {
            let current_b = matches!(&current, ParamValue::Bool(true));
            Box::new(
                Toggle::with_state(current_b)
                    .on_change(move |v| update_param(model_idx, key, ParamValue::Bool(v))),
            )
        }
        ParamKind::Text { .. } => {
            let current_s = match &current {
                ParamValue::Text(v) => v.clone(),
                _ => String::new(),
            };
            Box::new(
                TextField::with_text(current_s)
                    .on_change(move |s| {
                        update_param(model_idx, key, ParamValue::Text(s.to_string()))
                    })
                    .class("models-active-textfield"),
            )
        }
        ParamKind::Enum { options, .. } => {
            let current_s = match &current {
                ParamValue::Enum(v) => v.clone(),
                _ => options.first().map(|s| (*s).to_string()).unwrap_or_default(),
            };
            let items: Vec<DropdownItem> = options
                .iter()
                .map(|o| DropdownItem::new(*o, *o))
                .collect();
            Box::new(
                Dropdown::with_items(items)
                    .selected(current_s)
                    .on_change(move |s| {
                        update_param(model_idx, key, ParamValue::Enum(s.to_string()))
                    })
                    .class("models-active-dropdown"),
            )
        }
    }
}

fn update_param(model_idx: usize, key: &'static str, value: ParamValue) {
    let ctx = use_context::<AppCtx>();
    ctx.models.update(|list| {
        if let Some(m) = list.get_mut(model_idx) {
            if let Some(p) = m.active_params.iter_mut().find(|p| p.key == key) {
                p.value = value;
            } else {
                m.active_params.push(ActiveParam { key: key.to_string(), value });
            }
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Пул доступных настроек (чипы с Tooltip)
// ─────────────────────────────────────────────────────────────────────────────

/// Ключи, которые уже представлены отдельными полями в UI —
/// не показываем их среди доступных чипов.
const HIDDEN_KEYS: &[&str] = &["ctx_size", "model", "mmproj"];

fn available_section(model: &ModelConfig) -> impl Widget {
    let active_keys: std::collections::HashSet<&str> = model
        .active_params
        .iter()
        .map(|p| p.key.as_str())
        .collect();

    let mut groups: Vec<Box<dyn Widget>> = Vec::new();
    groups.push(Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new("Доступные настройки").class("settings-section-title"))
            .child(
                Text::new("Кликните по чипу, чтобы добавить настройку в активные. Наведите — описание.")
                    .class("models-available-hint"),
            ),
    ));

    for cat in ParamCategory::ALL {
        let params: Vec<&'static LlamaParam> = LLAMA_PARAMS
            .iter()
            .filter(|p| {
                p.category == *cat
                    && !HIDDEN_KEYS.contains(&p.key)
                    && !active_keys.contains(p.key)
            })
            .collect();

        if params.is_empty() {
            continue;
        }

        groups.push(Box::new(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Text::new(cat.label()).class("models-available-category"))
                .child(chips_grid(params)),
        ));
    }

    DecoratedBox::new().class("models-available-wrap").child(
        Column::new()
            .gap(18.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(groups),
    )
}

/// Раскладывает чипы в сетку построчно по `PER_ROW` штук на строку.
/// syngui не поддерживает flex-wrap — делаем явную разбивку. `MainAxisAlignment::Start`
/// оставляет чипам их intrinsic-ширину (без растяжения Row до max_width) — иначе
/// Row-flex-контейнер пробрасывает узкие constraints, и шрифтовой движок
/// переносит текст чипа внутрь двух строк.
const PER_ROW: usize = 3;

fn chips_grid(params: Vec<&'static LlamaParam>) -> impl Widget {
    let mut rows: Vec<Box<dyn Widget>> = Vec::new();

    for chunk in params.chunks(PER_ROW) {
        let mut cells: Vec<Box<dyn Widget>> = Vec::new();
        for p in chunk {
            cells.push(available_chip(p));
        }
        let row = Row::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::Start)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(cells);
        rows.push(Box::new(row));
    }

    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(rows)
}

fn available_chip(param: &'static LlamaParam) -> Box<dyn Widget> {
    let key = param.key;
    let kind = param.kind;
    let chip = Chip::new(param.label)
        .icon(MI_ADD_CIRCLE)
        .on_click(move || {
            let ctx = use_context::<AppCtx>();
            let Some(current_name) = ctx.selected_model.get_untracked() else { return; };
            ctx.models.update(|list| {
                if let Some(m) = list.iter_mut().find(|m| m.name == current_name) {
                    if !m.active_params.iter().any(|p| p.key == key) {
                        m.active_params.push(ActiveParam {
                            key: key.to_string(),
                            value: default_value(&kind),
                        });
                    }
                }
            });
        })
        .class("models-available-chip");

    wrap_chip_with_tooltip(param, chip)
}

fn wrap_chip_with_tooltip<W: Widget + 'static>(param: &'static LlamaParam, chip: W) -> Box<dyn Widget> {
    // Собираем содержимое tooltip: title, cli, desc и (если есть) env в одной Column.
    let mut rows: Vec<Box<dyn Widget>> = vec![
        Box::new(Text::new(param.label).class("models-tip-title")),
        Box::new(Text::new(param.cli).class("models-tip-cli")),
        Box::new(Text::new(param.description).class("models-tip-desc")),
    ];
    if let Some(env) = param.env {
        rows.push(Box::new(
            Text::new(format!("env: {env}")).class("models-tip-env"),
        ));
    }
    let content = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(rows);

    Box::new(
        Tooltip::rich(chip, content)
            .position(TooltipPosition::Above)
            .max_width(360.0)
            .delay_ms(250)
            .class("models-chip-tooltip"),
    )
}
