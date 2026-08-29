//! Диалоги упаковки: быстрая карточка и мастер из трёх шагов.
//!
//! Оба работают над одним [`PackWizard`]: карточка показывает разобранный
//! план и кнопку «Собрать», мастер — тот же план с возможностью править
//! состав и метаданные. Переход между ними не теряет результат разбора —
//! это один и тот же `plan`, просто нарисованный подробнее.
//!
//! Смысл разделения: девять случаев из десяти план верен как есть, и
//! показывать ради них форму на шесть полей незачем. Мастер нужен там, где
//! автоматика честно не знает ответа: несколько наборов весов в одной папке,
//! пустая архитектура, нестандартная раскладка.

use std::path::Path;

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::trn;
use syngui::widgets::input::Toggle;
use syngui::widgets::scroll::ScrollView;
use syngui::widgets::{Checkbox, Dropdown, DropdownItem, ProgressBar, Stepper, TextField};
use synaptix_bundle::inspect::LayerRole;
use synaptix_bundle::pack_plan::{Guess, PackPlan};

use crate::icons::{
    MI_ADD_CIRCLE, MI_ARROW_FORWARD, MI_CHECK, MI_CLOSE, MI_DEPLOYED_CODE, MI_FOLDER_OPEN,
    MI_NOTE_ADD, MI_SAVE, MI_SETTINGS,
};

use super::actions;
use super::state::{
    allowed_quants, role_is_quantizable, ComponentLayers, CreateProgress, LoadState, PackWizard,
    QuantChoice, SynExplorerCtx,
};

// ── Быстрая карточка ───────────────────────────────────────────────────────

/// «Вот что распознано, вот куда положу» — и кнопка. Это и есть путь в один
/// клик: до неё пользователь нажал ровно один раз, на карточке модели.
pub fn confirm_card(wizard: PackWizard) -> impl Widget {
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if wizard.scanning.get() {
            return vec![Box::new(scanning_row())];
        }
        let Some(plan) = wizard.plan.get() else {
            return vec![Box::new(
                Text::new(tr!("explorer.pack.no_source")).class("syn-dialog-hint"),
            )];
        };
        vec![Box::new(plan_summary(&plan, wizard))]
    });

    let progress = progress_zone();
    let footer = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let busy = matches!(ctx.load_state.get(), LoadState::Creating);
        let ready = wizard.plan.get().is_some() && !wizard.scanning.get();
        let build_class = if ready && !busy {
            "syn-dialog-btn-primary"
        } else {
            "syn-dialog-btn-primary disabled"
        };
        let row = mgui! {
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Button::new(tr!("explorer.pack.customize"))
                        .leading_icon(MI_SETTINGS)
                        .on_click(move || {
                            if busy {
                                return;
                            }
                            let ctx = use_context::<SynExplorerCtx>();
                            actions::switch_to_wizard(ctx);
                        })
                        .class("syn-dialog-btn-secondary"),
                    DecoratedBox::new().class("syn-spacer grow"),
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(cancel)
                        .class("syn-dialog-btn-secondary"),
                    Button::new(tr!("explorer.pack.build"))
                        .leading_icon(MI_CHECK)
                        .on_click(move || {
                            if !ready || busy {
                                return;
                            }
                            let ctx = use_context::<SynExplorerCtx>();
                            actions::start_packing(ctx);
                        })
                        .class(build_class),
                ]
        };
        vec![Box::new(row)]
    });

    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-wide") => [
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.pack.confirm_title")).class("syn-dialog-title"),
                    body,
                    quick_quant_row(wizard),
                    total_size_row(wizard),
                    progress,
                    footer,
                ]
        ]
    }
}

/// Одна строка «Квантование» на быстром пути: выбор применяется сразу ко
/// всем квантуемым ролям. Разбирать модель по слоям здесь незачем — для
/// этого есть мастер и режим эксперта.
fn quick_quant_row(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if wizard.plan.get().is_none() || wizard.scanning.get() {
            return vec![];
        }
        let current = wizard.uniform_quant();
        let selected = current.map(|c| c.key()).unwrap_or("mixed");
        let mut items = vec![
            DropdownItem::new("dense", QuantChoice::Dense.label()),
            DropdownItem::new("nvfp4", "NVFP4"),
            DropdownItem::new("mxfp8", "MXFP8"),
        ];
        if current.is_none() {
            // Точность настроена по ролям в мастере — не затираем её молча
            // тем, что покажет дропдаун.
            items.push(DropdownItem::new("mixed", tr!("explorer.quant.mixed")));
        }
        let row = mgui! {
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(tr!("explorer.quant.label")).class("syn-quant-role"),
                    Text::new(tr!("explorer.quant.hint")).class("syn-pack-note grow"),
                    Dropdown::with_items(items)
                        .selected(selected)
                        .on_change(move |v| {
                            if v != "mixed" {
                                wizard.set_quant_for_all(QuantChoice::from_key(v));
                            }
                        })
                        .class("syn-quant-dropdown"),
                ]
        };
        vec![Box::new(row)]
    })
}

fn scanning_row() -> impl Widget {
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("explorer.pack.scanning")).class("syn-dialog-hint"),
                ProgressBar::new().indeterminate().class("syn-dialog-progress"),
            ]
    }
}

/// Сводка плана: имя, чем распознано, состав и куда ляжет результат.
fn plan_summary(plan: &PackPlan, wizard: PackWizard) -> impl Widget {
    let id = wizard.id.get_untracked();
    let title = if id.is_empty() { plan.meta.id.clone() } else { id };
    let subtitle = summary_line(plan);
    let source = crate::paths::pretty(&plan.root);

    let out_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![Box::new(out_path_row(wizard))]
    });

    let col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(move || Text::new(title.clone()).class("syn-pack-summary-title"))
        .child(move || Text::new(subtitle.clone()).class("syn-pack-summary-line"))
        .child(move || {
            Text::new(tr!("explorer.pack.source", path = source.clone()))
                .elide(Elide::Middle)
                .class("syn-dialog-path")
        });

    let warnings = plan.warnings.clone();
    let notes: Vec<String> = plan
        .components
        .iter()
        .filter(|c| !c.enabled && !c.note.is_empty())
        .map(|c| tr!("explorer.pack.skipped", name = c.name.clone(), note = c.note.clone()))
        .collect();

    mgui! {
        DecoratedBox::new().class("syn-pack-summary") => [
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    col,
                    composition_block(wizard),
                    out_row,
                    notes_block(warnings, notes),
                ]
        ]
    }
}

fn composition_block(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(layers) = wizard.layers.get() else {
            return vec![];
        };
        if layers.by_role.is_empty() {
            return vec![];
        }
        vec![Box::new(role_bar(&layers))]
    })
}

fn notes_block(warnings: Vec<String>, notes: Vec<String>) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if warnings.is_empty() && notes.is_empty() {
            return vec![];
        }
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for w in warnings.iter() {
            let w = w.clone();
            col = col.child(move || Text::new(w.clone()).class("syn-pack-warning"));
        }
        for n in notes.iter() {
            let n = n.clone();
            col = col.child(move || Text::new(n.clone()).class("syn-pack-note"));
        }
        vec![Box::new(col)]
    })
}

/// «qwen3_5 · text-generation · 1 компонент · 11 файлов · 43.7 ГБ».
fn summary_line(plan: &PackPlan) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !plan.meta.arch.is_empty() {
        parts.push(plan.meta.arch.clone());
    }
    if !plan.meta.purpose.is_empty() {
        parts.push(plan.meta.purpose.clone());
    }
    let comps = plan.components.iter().filter(|c| c.enabled).count();
    parts.push(trn!("explorer.pack.components_n", comps));
    let files = plan.aux.iter().filter(|f| f.enabled).count();
    if files > 0 {
        parts.push(trn!("explorer.pack.files_n", files));
    }
    parts.push(humanize_bytes(plan.payload_bytes()));
    parts.join(" · ")
}

/// Куда ляжет бандл и сколько на этом разделе свободно. Место показываем
/// заранее: узнать о нехватке через десять минут упаковки — худшее, что
/// может случиться.
fn out_path_row(wizard: PackWizard) -> impl Widget {
    let out = wizard.out_path.get();
    let label = match &out {
        Some(p) => crate::paths::pretty(p),
        None => tr!("explorer.dialog.new_package.not_selected"),
    };
    let space = out
        .as_deref()
        .and_then(|p| p.parent())
        .map(free_space_label)
        .unwrap_or_default();

    mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Button::new(tr!("explorer.dialog.new_package.pick_out"))
                    .leading_icon(MI_SAVE)
                    .on_click(move || actions::pick_out_path(wizard))
                    .class("syn-dialog-btn-secondary"),
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("grow") => [
                        Text::new(label).elide(Elide::Middle).class("syn-dialog-path"),
                        Text::new(space).class("syn-pack-space"),
                    ],
            ]
    }
}

fn free_space_label(dir: &Path) -> String {
    match synaptix_bundle::available_space(dir) {
        Ok(n) => tr!("explorer.pack.free_space", value = humanize_bytes(n)),
        Err(_) => String::new(),
    }
}

/// Полоса «из чего состоит модель» плюс подписи ролей с долями.
fn role_bar(layers: &ComponentLayers) -> impl Widget {
    let total = layers.bytes.max(1);
    let widths = segment_widths(&layers.by_role, total);
    // Без зазора: он прибавляется к сумме процентов и полоса вылезает за
    // карточку. Границы ролей и так видны по цвету.
    let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for ((role, _), pct) in layers.by_role.iter().zip(widths) {
        let class = format!("syn-role-seg role-{}", role.key());
        row = row.child(move || {
            DecoratedBox::new()
                .class(class.clone())
                .style("width", StyleValue::percent(pct))
        });
    }

    let mut legend = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (role, est) in layers.by_role.iter().take(6) {
        let pct = (est.dense as f64 / total as f64) * 100.0;
        let text = format!("{} {:.0}%", role_label(*role), pct);
        let dot_class = format!("syn-role-dot role-{}", role.key());
        legend = legend.child(move || {
            mgui! {
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        DecoratedBox::new().class(dot_class.clone()),
                        Text::new(text.clone()).class("syn-role-legend"),
                    ]
            }
        });
    }

    mgui! {
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("syn-role-bar").child(row),
                legend,
            ]
    }
}

/// Ширины сегментов в процентах, нормированные так, чтобы сумма была ровно
/// 100. Крохотные роли получают видимый минимум, а излишек снимается с
/// самой крупной — иначе полоса шире карточки.
pub fn segment_widths(
    by_role: &[(LayerRole, synaptix_bundle::inspect::SizeEstimate)],
    total: u64,
) -> Vec<f32> {
    const MIN: f32 = 0.8;
    let mut w: Vec<f32> = by_role
        .iter()
        .map(|(_, est)| ((est.dense as f64 / total as f64) * 100.0) as f32)
        .map(|p| if p > 0.0 { p.max(MIN) } else { 0.0 })
        .collect();
    let sum: f32 = w.iter().sum();
    if sum > 100.0 {
        if let Some((i, _)) = w
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            w[i] -= sum - 100.0;
        }
    }
    w
}

pub fn role_label(role: LayerRole) -> String {
    match role {
        LayerRole::Embedding => tr!("explorer.role.embedding"),
        LayerRole::LmHead => tr!("explorer.role.lm_head"),
        LayerRole::Attention => tr!("explorer.role.attention"),
        LayerRole::Mlp => tr!("explorer.role.mlp"),
        LayerRole::Norm => tr!("explorer.role.norm"),
        LayerRole::Conv => tr!("explorer.role.conv"),
        LayerRole::Conditioning => tr!("explorer.role.conditioning"),
        LayerRole::Head => tr!("explorer.role.head"),
        LayerRole::Router => tr!("explorer.role.router"),
        LayerRole::Vision => tr!("explorer.role.vision"),
        LayerRole::Audio => tr!("explorer.role.audio"),
        LayerRole::Vae => tr!("explorer.role.vae"),
        LayerRole::Lora => tr!("explorer.role.lora"),
        LayerRole::Other => tr!("explorer.role.other"),
    }
}

// ── Мастер ─────────────────────────────────────────────────────────────────

pub fn wizard_card(wizard: PackWizard) -> impl Widget {
    let stepper = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let step = wizard.step.get();
        let s = Stepper::new()
            .step(tr!("explorer.wizard.step.source"), None)
            .step(tr!("explorer.wizard.step.content"), None)
            .step(tr!("explorer.wizard.step.meta"), None)
            .current(step)
            .allow_navigation(true)
            .on_step_click(move |i| actions::goto_step(wizard, i));
        vec![Box::new(s)]
    });

    let content = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match wizard.step.get() {
            0 => Box::new(step_source(wizard)),
            1 => Box::new(step_content(wizard)),
            _ => Box::new(step_meta(wizard)),
        };
        vec![widget]
    });

    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-wide syn-pack-wizard") => [
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Text::new(tr!("explorer.wizard.title")).class("syn-dialog-title grow"),
                            expert_toggle(wizard),
                        ],
                    stepper,
                    DecoratedBox::new().class("syn-wizard-body").child(content),
                    progress_zone(),
                    wizard_footer(wizard),
                ]
        ]
    }
}

/// Переключатель режима эксперта. Липкий: значение уходит в конфиг через
/// общий autosave, и включивший его однажды не включает снова.
fn expert_toggle(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let on = wizard.expert.get();
        let row = mgui! {
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(tr!("explorer.wizard.expert")).class("syn-pack-summary-line"),
                    Toggle::with_state(on).on_change(move |v| wizard.expert.set(v)),
                ]
        };
        vec![Box::new(row)]
    })
}

/// Шаг 1 — источник. Две кнопки вместо одной: нативные диалоги не умеют
/// предлагать файл и папку разом, а одиночный `.safetensors` — полноправная
/// модель (раньше её приходилось заводить через папку с хардлинком).
fn step_source(wizard: PackWizard) -> impl Widget {
    let picker = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Button::new(tr!("explorer.wizard.pick_dir"))
                    .leading_icon(MI_FOLDER_OPEN)
                    .on_click(|| {
                        let ctx = use_context::<SynExplorerCtx>();
                        actions::pick_source_dir(ctx);
                    })
                    .class("syn-dialog-btn-secondary"),
                Button::new(tr!("explorer.wizard.pick_file"))
                    .leading_icon(MI_NOTE_ADD)
                    .on_click(|| {
                        let ctx = use_context::<SynExplorerCtx>();
                        actions::pick_source_file(ctx);
                    })
                    .class("syn-dialog-btn-secondary"),
            ]
    };

    let result = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if wizard.scanning.get() {
            return vec![Box::new(scanning_row())];
        }
        let Some(plan) = wizard.plan.get() else {
            return vec![Box::new(
                Text::new(tr!("explorer.wizard.source_hint")).class("syn-dialog-hint"),
            )];
        };
        vec![Box::new(plan_summary(&plan, wizard))]
    });

    mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("explorer.wizard.source_desc")).class("syn-dialog-hint"),
                picker,
                result,
            ]
    }
}

/// Шаг 2 — состав: компоненты, файлы и инспектор слоёв с выбором точности.
fn step_content(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(plan) = wizard.plan.get() else {
            return vec![Box::new(
                Text::new(tr!("explorer.wizard.source_hint")).class("syn-dialog-hint"),
            )];
        };
        let enabled = wizard.components_enabled.get();
        let aux_enabled = wizard.aux_enabled.get();

        let mut col = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);

        col = col.child(|| {
            Text::new(tr!("explorer.wizard.components_label")).class("syn-dialog-section-label")
        });
        for (i, c) in plan.components.iter().enumerate() {
            let on = enabled.get(i).copied().unwrap_or(c.enabled);
            let label = format!(
                "{}  ·  {}  ·  {}",
                c.name,
                trn!("explorer.pack.shards_n", c.paths.len()),
                humanize_bytes(c.bytes)
            );
            let note = c.note.clone();
            let name = c.name.clone();
            let prefix = c.prefix.clone();
            col = col.child(move || {
                let label = label.clone();
                let note = note.clone();
                let name = name.clone();
                let prefix = prefix.clone();
                mgui! {
                    Column::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            Checkbox::checked(on)
                                .label(label)
                                .on_change(move |v| actions::toggle_component(wizard, i, v))
                                .class("syn-dialog-checkbox"),
                            note_line(note),
                            component_expert_row(wizard, i, name, prefix),
                        ]
                }
            });
        }

        let aux_total: u64 = plan.aux.iter().map(|f| f.bytes).sum();
        let aux_on = aux_enabled.iter().filter(|v| **v).count();
        let aux_label = tr!(
            "explorer.wizard.aux_label",
            on = aux_on,
            total = plan.aux.len(),
            size = humanize_bytes(aux_total)
        );
        let all_on = aux_on == plan.aux.len() && !plan.aux.is_empty();
        let aux_len = plan.aux.len();
        col = col.child(move || {
            Checkbox::checked(all_on)
                .label(aux_label.clone())
                .on_change(move |v| {
                    for i in 0..aux_len {
                        actions::toggle_aux(wizard, i, v);
                    }
                })
                .class("syn-dialog-checkbox")
        });

        col = col.child(move || aux_expert_list(wizard));
        col = col.child(move || layers_section(wizard));
        vec![Box::new(col)]
    })
}

/// Имя чанка и префикс имён тензоров — то, чем компонент адресуется в
/// загрузчиках. Обычному пользователю эти поля только мешают, эксперту без
/// них не собрать нестандартную раскладку.
fn component_expert_row(
    wizard: PackWizard,
    index: usize,
    name: String,
    prefix: String,
) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !wizard.expert.get() {
            return vec![];
        }
        let name_field = TextField::with_text(name.clone())
            .placeholder("main / transformer / vae …")
            .on_change(move |v| actions::set_component_name(wizard, index, v.to_string()))
            .class("syn-dialog-input");
        let prefix_field = TextField::with_text(prefix.clone())
            .placeholder(tr!("explorer.wizard.prefix_placeholder"))
            .on_change(move |v| actions::set_component_prefix(wizard, index, v.to_string()))
            .class("syn-dialog-input");
        let row = mgui! {
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("syn-expert-row") => [
                    Text::new(tr!("explorer.wizard.chunk_name")).class("syn-quant-role"),
                    name_field,
                    Text::new(tr!("explorer.wizard.tensor_prefix")).class("syn-quant-role"),
                    prefix_field,
                ]
        };
        vec![Box::new(row)]
    })
}

/// Пофайловый состав с назначением каждого файла. Тег важен по-настоящему:
/// загрузчики читают только `inference`, и README, попавший туда же, будет
/// им мешать.
fn aux_expert_list(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !wizard.expert.get() {
            return vec![];
        }
        let Some(plan) = wizard.plan.get() else {
            return vec![];
        };
        if plan.aux.is_empty() {
            return vec![];
        }
        let enabled = wizard.aux_enabled.get();
        let mut col = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, f) in plan.aux.iter().enumerate() {
            let on = enabled.get(i).copied().unwrap_or(f.enabled);
            let label = format!("{}  ·  {}", f.rel, humanize_bytes(f.bytes));
            let tag = f.tag;
            col = col.child(move || {
                let label = label.clone();
                mgui! {
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Checkbox::checked(on)
                                .label(label)
                                .on_change(move |v| actions::toggle_aux(wizard, i, v))
                                .class("syn-dialog-checkbox grow"),
                            Dropdown::with_items(vec![
                                DropdownItem::new("inference", tr!("explorer.tag.inference")),
                                DropdownItem::new("doc", tr!("explorer.tag.doc")),
                                DropdownItem::new("example", tr!("explorer.tag.example")),
                                DropdownItem::new("asset", tr!("explorer.tag.asset")),
                            ])
                            .selected(tag.as_str())
                            .on_change(move |v| actions::set_aux_tag(wizard, i, v))
                            .class("syn-quant-dropdown"),
                        ]
                }
            });
        }
        vec![Box::new(
            DecoratedBox::new().class("syn-expert-block syn-aux-wrap").child(
                ScrollView::new()
                    .vertical()
                    .class("syn-aux-scroll")
                    .child(col),
            ),
        )]
    })
}

/// Почему выбранный квант ничего не дал и что попробовать вместо него.
/// Пустая строка — квант применился нормально.
fn shape_hint(est: synaptix_bundle::inspect::SizeEstimate, current: QuantChoice) -> String {
    if current == QuantChoice::Dense || est.dense == 0 {
        return String::new();
    }
    let after = est.for_kind(current.kind());
    // «Почти не изменилось» — значит форма не подошла почти нигде.
    let helps = |v: u64| v * 100 < est.dense * 95;
    if helps(after) {
        return String::new();
    }
    let alt = match current {
        QuantChoice::Nvfp4 if helps(est.mxfp8) => Some("MXFP8"),
        QuantChoice::Mxfp8 if helps(est.nvfp4) => Some("NVFP4"),
        _ => None,
    };
    match alt {
        Some(other) => tr!(
            "explorer.quant.no_gain_try",
            format = current.label(),
            other = other
        ),
        None => tr!("explorer.quant.no_gain", format = current.label()),
    }
}

fn hint_line(hint: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if hint.is_empty() {
            return vec![];
        }
        vec![Box::new(Text::new(hint.clone()).class("syn-quant-hint"))]
    })
}

/// «→ 65.4 GB» — показывается, только когда квант действительно уменьшает вес.
fn size_after_line(after: Option<String>) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(v) = after.clone() else {
            return vec![];
        };
        vec![Box::new(mgui! {
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(MI_ARROW_FORWARD).class("syn-quant-arrow"),
                    Text::new(v).max_lines(1).class("syn-quant-size-after"),
                ]
        })]
    })
}

fn note_line(note: String) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if note.is_empty() {
            return vec![];
        }
        vec![Box::new(Text::new(note.clone()).class("syn-pack-note"))]
    })
}

/// Инспектор слоёв: роль, доля и выбор точности. Роли, которые движок не
/// квантует отдельно (свёртки, нормировки), помечены — притворяться, что
/// выбор там на что-то влияет, нельзя.
fn layers_section(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(layers) = wizard.layers.get() else {
            return vec![];
        };
        let quant = wizard.quant.get();
        let total = layers.bytes.max(1);

        let mut col = Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child({
            let n = layers.tensor_count;
            let comp = layers.component.clone();
            move || {
                Text::new(tr!(
                    "explorer.wizard.layers_label",
                    component = comp.clone(),
                    count = n
                ))
                .class("syn-dialog-section-label")
            }
        });
        col = col.child({
            let l = layers.clone();
            move || role_bar(&l)
        });

        let mut rows = Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("syn-quant-rows");
        for (role, est) in layers.by_role.iter() {
            let role = *role;
            let est = *est;
            let pct = (est.dense as f64 / total as f64) * 100.0;
            let current = quant
                .iter()
                .find(|(r, _)| *r == role)
                .map(|(_, q)| *q)
                .unwrap_or(QuantChoice::Dense);
            rows = rows.child(move || quant_row(wizard, role, pct, est, current));
        }
        // Ролей бывает больше десятка (диффузионные модели) — список
        // прокручивается, чтобы карточка не росла за край окна. Итог под
        // ним остаётся на виду.
        col = col.child(
            DecoratedBox::new().class("syn-quant-wrap").child(
                ScrollView::new().vertical().class("syn-quant-scroll").child(rows),
            ),
        );
        col = col.child(move || total_size_row(wizard));
        vec![Box::new(col)]
    })
}

/// Итог: сколько бандл весит сейчас и сколько будет весить с выбранной
/// точностью. Без этой строки выбор кванта делается вслепую.
fn total_size_row(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = wizard.quant.get();
        let Some((dense, quantized)) = wizard.estimated_payload() else {
            return vec![];
        };
        let changed = quantized < dense;
        let value = humanize_bytes(if changed { quantized } else { dense });

        let row = mgui! {
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(tr!("explorer.quant.total_label")).class("syn-pack-summary-line"),
                    Text::new(humanize_bytes(dense)).class(if changed {
                        "syn-quant-size-before"
                    } else {
                        "syn-pack-summary-line"
                    }),
                    size_after_line(changed.then(|| value.clone())),
                ]
        };
        vec![Box::new(
            DecoratedBox::new().class("syn-quant-total").child(row),
        )]
    })
}

fn quant_row(
    wizard: PackWizard,
    role: LayerRole,
    pct: f64,
    est: synaptix_bundle::inspect::SizeEstimate,
    current: QuantChoice,
) -> impl Widget {
    let quantizable = role_is_quantizable(role);
    // Размер после выбранного кванта — по раскладке ядер, а не прикидкой
    // «делим на четыре». Стрелка — из Material Icons: символа U+2192 в
    // текстовом шрифте нет, и вместо него получался пробел.
    let after = est.for_kind(current.kind());
    let dense_size = humanize_bytes(est.dense);
    let after_size = (after < est.dense).then(|| humanize_bytes(after));
    // Квант может не взять форму: NVFP4 требует обе размерности кратными
    // 64, MXFP8 — только последнюю кратной 32. У PLE-эмбеддингов
    // `[2500012, 160]` первое условие не выполняется, и размер не меняется.
    // Без объяснения это выглядит как сломанный подсчёт.
    let hint = shape_hint(est, current);
    // Контрол строим внутри `Reactive`: `mgui!` принимает виджеты, а не
    // `Box<dyn Widget>`, и обёртка заодно даёт замыкание, которое можно
    // вызывать повторно (все захваты — Copy).
    let control = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !quantizable {
            return vec![Box::new(
                Text::new(tr!("explorer.quant.not_applicable")).class("syn-quant-na"),
            )];
        }
        let items: Vec<DropdownItem> = allowed_quants(role)
            .iter()
            .map(|c| DropdownItem::new(c.key(), c.label()))
            .collect();
        vec![Box::new(
            Dropdown::with_items(items)
                .selected(current.key())
                .on_change(move |s| {
                    let choice = QuantChoice::from_key(s);
                    wizard.quant.update(|v| {
                        if let Some(slot) = v.iter_mut().find(|(r, _)| *r == role) {
                            slot.1 = choice;
                        } else {
                            v.push((role, choice));
                        }
                    });
                })
                .class("syn-quant-dropdown"),
        )]
    });

    mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("syn-quant-row") => [
                DecoratedBox::new().class(format!("syn-role-dot role-{}", role.key())),
                Column::new()
                    .gap(1.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("grow") => [
                        Text::new(role_label(role)).class("syn-quant-role"),
                        hint_line(hint),
                    ],
                Text::new(format!("{:.0}%", pct)).class("syn-quant-pct"),
                Row::new()
                    .gap(5.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .class("syn-quant-size") => [
                        Text::new(dense_size).max_lines(1).class("syn-quant-size-before"),
                        size_after_line(after_size),
                    ],
                control,
            ]
    }
}

/// Шаг 3 — метаданные. Поля предзаполнены догадками; у каждой видно, откуда
/// она взялась, чтобы «qwen3_5» не выглядел выдумкой программы.
fn step_meta(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let plan = wizard.plan.get();
        let arch_src = plan.as_ref().map(|p| p.meta.arch_from).unwrap_or(Guess::Unknown);
        let purpose_src = plan.as_ref().map(|p| p.meta.purpose_from).unwrap_or(Guess::Unknown);
        let version_src = plan.as_ref().map(|p| p.meta.version_from).unwrap_or(Guess::Unknown);

        let id_field = TextField::with_text(wizard.id.get_untracked())
            .placeholder(tr!("explorer.dialog.new_package.id_placeholder"))
            .on_change(move |s| wizard.id.set(s.to_string()))
            .class("syn-dialog-input");
        let version_field = TextField::with_text(wizard.version.get_untracked())
            .placeholder("1.0.0")
            .on_change(move |s| wizard.version.set(s.to_string()))
            .class("syn-dialog-input");
        let arch_field = TextField::with_text(wizard.arch.get_untracked())
            .placeholder("qwen3_5 / ltx-2.3 / …")
            .on_change(move |s| wizard.arch.set(s.to_string()))
            .class("syn-dialog-input");
        let purpose_field = TextField::with_text(wizard.purpose.get_untracked())
            .placeholder("text-generation / asr / video …")
            .on_change(move |s| wizard.purpose.set(s.to_string()))
            .class("syn-dialog-input");

        let delete_sig = wizard.delete_sources;
        let delete_checkbox = Checkbox::checked(delete_sig.get_untracked())
            .label(tr!("explorer.dialog.new_package.delete_sources"))
            .on_change(move |v| delete_sig.set(v))
            .class("syn-dialog-checkbox syn-dialog-warning");

        let col = mgui! {
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    field_row("id", Guess::Unknown, id_field),
                    field_row("version", version_src, version_field),
                    field_row("arch", arch_src, arch_field),
                    field_row("purpose", purpose_src, purpose_field),
                    out_path_row(wizard),
                    delete_checkbox,
                    expert_meta_block(wizard),
                ]
        };
        vec![Box::new(col)]
    })
}

/// Что ещё умеет формат `.syn`, кроме весов: контрольные суммы по каждому
/// чанку и читаемый глазами центральный каталог. Обычному пользователю это
/// лишний шум, эксперту — то, ради чего он включил режим.
fn expert_meta_block(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !wizard.expert.get() {
            return vec![];
        }
        let sha = wizard.sha256;
        let blake = wizard.blake3;
        let json = wizard.cdir_json;
        let col = mgui! {
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.wizard.checksums")).class("syn-dialog-section-label"),
                    Checkbox::checked(sha.get_untracked())
                        .label(tr!("explorer.wizard.sha256"))
                        .on_change(move |v| sha.set(v))
                        .class("syn-dialog-checkbox"),
                    Checkbox::checked(blake.get_untracked())
                        .label(tr!("explorer.wizard.blake3"))
                        .on_change(move |v| blake.set(v))
                        .class("syn-dialog-checkbox"),
                    Checkbox::checked(json.get_untracked())
                        .label(tr!("explorer.wizard.cdir_json"))
                        .on_change(move |v| json.set(v))
                        .class("syn-dialog-checkbox"),
                ]
        };
        vec![Box::new(DecoratedBox::new().class("syn-expert-block").child(col))]
    })
}

fn field_row<W: Widget + 'static>(label: &str, guess: Guess, field: W) -> impl Widget {
    let label = label.to_string();
    let hint = guess_label(guess);
    mgui! {
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Text::new(label).class("syn-dialog-label"),
                        Text::new(hint).class("syn-guess-badge"),
                    ],
                field,
            ]
    }
}

/// Откуда взялась догадка. Пустая строка — поле заполнял человек.
fn guess_label(guess: Guess) -> String {
    match guess {
        Guess::ConfigJson => tr!("explorer.guess.config_json"),
        Guess::ModelIndex => tr!("explorer.guess.model_index"),
        Guess::SafetensorsMeta => tr!("explorer.guess.safetensors_meta"),
        Guess::TensorNames => tr!("explorer.guess.tensor_names"),
        Guess::Name => tr!("explorer.guess.name"),
        Guess::Unknown => String::new(),
    }
}

fn wizard_footer(wizard: PackWizard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let busy = matches!(ctx.load_state.get(), LoadState::Creating);
        let step = wizard.step.get();
        let has_plan = wizard.plan.get().is_some();

        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if step > 0 {
            row = row.child(move || {
                Button::new(tr!("explorer.wizard.back"))
                    .on_click(move || actions::goto_step(wizard, step - 1))
                    .class("syn-dialog-btn-secondary")
            });
        }
        row = row.child(|| DecoratedBox::new().class("syn-spacer grow"));
        row = row.child(move || {
            Button::new(tr!("app.cancel"))
                .leading_icon(MI_CLOSE)
                .on_click(cancel)
                .class("syn-dialog-btn-secondary")
        });
        if step < 2 {
            let class = if has_plan {
                "syn-dialog-btn-primary"
            } else {
                "syn-dialog-btn-primary disabled"
            };
            row = row.child(move || {
                Button::new(tr!("explorer.wizard.next"))
                    .on_click(move || {
                        if has_plan {
                            actions::goto_step(wizard, step + 1);
                        }
                    })
                    .class(class)
            });
        } else {
            let class = if has_plan && !busy {
                "syn-dialog-btn-primary"
            } else {
                "syn-dialog-btn-primary disabled"
            };
            row = row.child(move || {
                Button::new(tr!("explorer.pack.build"))
                    .leading_icon(MI_CHECK)
                    .on_click(move || {
                        if !has_plan || busy {
                            return;
                        }
                        let ctx = use_context::<SynExplorerCtx>();
                        actions::start_packing(ctx);
                    })
                    .class(class)
            });
        }
        vec![Box::new(row)]
    })
}

// ── Общее ──────────────────────────────────────────────────────────────────

/// Во время упаковки диалог не закрывается: запись всё равно продолжится, а
/// полузакрытый диалог сбил бы реактивность прогресса.
fn cancel() {
    let ctx = use_context::<SynExplorerCtx>();
    if matches!(ctx.load_state.get_untracked(), LoadState::Creating) {
        return;
    }
    ctx.close_dialog();
}

/// Прогресс-зона: пуста в покое, активна во время упаковки. Подписана на
/// `create_progress_gen` — throttled-тик из worker'а.
pub fn progress_zone() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let state = ctx.load_state.get();
        let _gen = ctx.create_progress_gen.get();
        if !matches!(state, LoadState::Creating) {
            return vec![];
        }
        let handle = ctx.create_progress.get_untracked();
        let snapshot = handle.lock().ok().map(|g| g.clone()).unwrap_or_default();
        vec![Box::new(progress_panel(snapshot))]
    })
}

fn progress_panel(p: CreateProgress) -> impl Widget {
    let fraction = p.fraction();
    let percent = (fraction * 100.0).round() as i32;
    // Показываем реальный размер payload'а (без удвоения stage+pack):
    // пользователь видит «X из 51 ГБ», а не «X из 103 ГБ».
    let bytes_human = tr!(
        "explorer.dialog.new_package.progress_bytes",
        done = format!("{:.2}", p.display_done() as f64 / (1024.0 * 1024.0 * 1024.0)),
        total = format!("{:.2}", p.payload_total as f64 / (1024.0 * 1024.0 * 1024.0))
    );
    let stage = if p.stage_label.is_empty() {
        tr!("explorer.dialog.new_package.stage_preparing")
    } else {
        p.stage_label
    };
    let is_indet = p.finalizing || p.bytes_total == 0;
    let bar = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if is_indet {
            vec![Box::new(ProgressBar::new().indeterminate().class("syn-dialog-progress"))]
        } else {
            vec![Box::new(ProgressBar::with_value(fraction).class("syn-dialog-progress"))]
        }
    });
    mgui! {
        DecoratedBox::new().class("syn-dialog-progress-card") => [
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(stage).class("syn-dialog-progress-stage"),
                    bar,
                    Row::new()
                        .gap(8.0)
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                            Text::new(bytes_human).class("syn-dialog-progress-bytes"),
                            Text::new(format!("{percent}%"))
                                .class("syn-dialog-progress-percent"),
                        ],
                ]
        ]
    }
}

/// Кнопка «+» в шапке страницы открывает мастер без источника.
pub fn new_package_button_icon() -> &'static str {
    MI_ADD_CIRCLE
}

/// Иконка модели-источника — используется и в левой панели.
pub fn source_icon() -> &'static str {
    MI_DEPLOYED_CODE
}

pub fn humanize_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.0} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}
