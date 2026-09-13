//! Правая панель Syn-чата: TabBar с двумя табами — «Параметры» (модель +
//! sampling + контекст + thinking, первая по умолчанию) и «Детали»
//! (метрики основного цикла и живых субагентов). TabBar ([`header`]) живёт в строке
//! заголовков каркаса, тело ([`body`]) — под ним. Инструменты и скилы — в
//! левой панели (`left_panel`).

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{Dropdown, DropdownItem, SegmentedButton, Slider, SpinBox, TextField, Toggle};

use crate::context::{SYN_RIGHT_PANEL_DETAILS, SYN_RIGHT_PANEL_PARAMS};
use crate::icons::*;
use crate::syn_chat::params::{SamplingMode, SamplingParams};
use crate::syn_chat::prompt_presets::{self, PromptDialog};
use crate::syn_chat::telemetry::{AgentRun, RunKind, RunState, RunStats, ROOT_RUN};
use crate::syn_chat::{SynChatCtx, SynModelRegistry};

pub fn header() -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    let tab = ctx.right_panel_tab;
    let tabbar = TabBar::new()
        .tab(Tab::new(tr!("chat.right.tab.params"), SYN_RIGHT_PANEL_PARAMS, &tab).icon(MI_TUNE))
        .tab(Tab::new(tr!("chat.right.tab.details"), SYN_RIGHT_PANEL_DETAILS, &tab).icon(MI_SPEED))
        .class("right-panel-tabbar-inner");
    DecoratedBox::new().class("right-panel-tabbar").child(tabbar)
}

pub fn body() -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    let tab = ctx.right_panel_tab;
    let body = DecoratedBox::new().class("right-panel-body").child(move || {
        let child: Box<dyn Widget> = match tab.get() {
            SYN_RIGHT_PANEL_DETAILS => Box::new(details_tab()),
            _ => Box::new(params_tab()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    });
    DecoratedBox::new().class("right-panel syn-chat-right").child(body)
}

// ─────────────────────── ТАБ «ПАРАМЕТРЫ» ───────────────────────

fn params_tab() -> impl Widget {
    ScrollView::new().vertical().child(mgui! {
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                model_card(),
                sampling_card_reactive(),
                context_card_reactive(),
                thinking_card_reactive(),
                system_prompt_card_reactive(),
                reset_button(),
            ]
    })
}

// ── Карточка «Модель» ────────────────────────────────────────────────

fn model_card() -> impl Widget {
    DecoratedBox::new().class("sampling-card").child(mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title(tr!("chat.right.model.title")),
                model_status_reactive(),
                pick_button_reactive(),
            ]
    })
}

fn model_status_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let reg = use_context::<SynModelRegistry>();
        let current = reg.current.get();
        let loading = reg.loading.get();
        let error = reg.error.get();

        let (icon, title, subtitle, status_class) = if loading {
            (
                MI_HOURGLASS_TOP,
                tr!("chat.model.status.loading"),
                tr!("chat.right.model.loading_hint"),
                "model-status loading",
            )
        } else if let Some(err) = error.as_ref() {
            (
                MI_REPORT,
                tr!("chat.right.model.error_title"),
                err.clone(),
                "model-status error",
            )
        } else if let Some(loaded) = current.as_ref() {
            let name = loaded
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "—".to_string());
            (MI_CHECK, tr!("chat.right.model.ready_title"), name, "model-status ready")
        } else {
            // Путь помним, но модель не поднимаем — показываем, что именно
            // поднимет кнопка «Загрузить модель».
            let hint = reg
                .last_path
                .get()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_else(|| tr!("chat.right.model.pick_hint"));
            (MI_INFO, tr!("chat.model.not_loaded"), hint, "model-status idle")
        };

        // Бейдж мультимодальности: есть ли в бандле vision-башня. Отвечает
        // на вопрос «поймёт ли эта модель прикреплённую картинку» до того,
        // как пользователь потратит время на отправку.
        let media_badge = current.as_ref().map(|loaded| {
            // Кэш из LoadedSynModel, а не Llm::supports_media(): последний
            // берёт мьютекс пайплайна, занятый на всё время генерации.
            if loaded.supports_media {
                tr!("chat.right.model.badge.multimodal")
            } else {
                tr!("chat.right.model.badge.text_only")
            }
        });

        // Бейдж режима: что за настройки сейчас у загруженной модели.
        //   "Оптимальный · NVFP4 / MXFP8 KV"
        let quant_badge = match current.as_ref() {
            Some(model) => {
                let app = use_context::<crate::context::AppCtx>();
                let profiles = app.model_profiles.get();
                let profile = crate::config::model_profile_of(&profiles, &model.path);
                let resolved = profile.resolve(&model.path);
                let mode = if profile.is_custom() {
                    tr!("settings.ai_models.mode.custom")
                } else {
                    tr!("settings.ai_models.mode.optimal")
                };
                Some(format!(
                    "{mode} · {} / {} KV",
                    crate::config::dtype_name(resolved.policy.weights_storage).to_uppercase(),
                    resolved.policy.kv_dtype.name().to_uppercase(),
                ))
            }
            None => None,
        };

        DecoratedBox::new()
            .child(mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(icon).class("model-status-icon"),
                    DecoratedBox::new().class("grow").child(mgui! {
                        Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            Text::new(title).class("model-status-title"),
                            Text::new(subtitle).max_lines(2).class("model-status-subtitle"),
                            Reactive::new(move || -> Vec<Box<dyn Widget>> {
                                match quant_badge.as_ref() {
                                    Some(s) => vec![Box::new(Text::new(s.clone()).class("model-status-quant"))],
                                    None => vec![],
                                }
                            }),
                            Reactive::new(move || -> Vec<Box<dyn Widget>> {
                                match media_badge.as_ref() {
                                    Some(s) => vec![Box::new(Text::new(s.clone()).class("model-status-quant"))],
                                    None => vec![],
                                }
                            }),
                        ]
                    }),
                ]
            })
            .class(status_class)
    }
}

fn pick_button_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let reg = use_context::<SynModelRegistry>();
        let loading = reg.loading.get();
        let has_loaded = reg.current.get().is_some();

        let pick = ToolButton::new(MI_FOLDER_OPEN)
            .tooltip(tr!("chat.right.model.pick.tooltip"))
            .on_click(move || {
                if reg.loading.get_untracked() {
                    return;
                }
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter(tr!("chat.right.model.pick.filter_name"), &["syn"])
                        .set_title(tr!("chat.right.model.pick.dialog_title"))
                        .pick_file();
                    if let Some(p) = path {
                        load_from_any_thread(p);
                    }
                });
            })
            .class(if loading {
                "right-pick-btn disabled"
            } else {
                "right-pick-btn"
            });

        // Модель поднимается только руками (авто-загрузки при входе в чат
        // нет), поэтому кнопка двойная: пока ничего не загружено — поднимает
        // последний бандл, дальше — выгружает.
        let toggle = if has_loaded {
            Button::new(tr!("chat.right.model.unload"))
                .leading_icon(MI_POWER_SETTINGS)
                .disabled(loading)
                .on_click(|| {
                    let ctx = use_context::<SynChatCtx>();
                    // Если идёт генерация — сначала abort, чтобы worker
                    // увидел несовпадение счётчика и завершился, отпустив
                    // Arc<LoadedSynModel> и освободив VRAM.
                    if ctx.pending.get_untracked() {
                        ctx.abort.fetch_add(1, Ordering::Relaxed);
                    }
                    use_context::<SynModelRegistry>().unload();
                })
                .class("right-model-btn right-model-btn-unload")
        } else {
            let last = reg.last_path.get();
            Button::new(tr!("chat.right.model.load"))
                .leading_icon(MI_POWER_SETTINGS)
                .disabled(loading || last.is_none())
                .on_click(move || {
                    if let Some(p) = last.clone() {
                        load_from_any_thread(p);
                    }
                })
                .class("right-model-btn right-model-btn-load")
        };

        // Picker и load/unload — одна строка: под ToolButton не нужна
        // отдельная полоса, он занимает свою ширину, а основную кнопку
        // растягивает `.grow`.
        DecoratedBox::new()
            .child(mgui! {
                Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    pick,
                    DecoratedBox::new().class("grow").child(toggle),
                ]
            })
            .class("right-pick-row")
    }
}

// ── Карточка «Sampling» ──────────────────────────────────────────────

/// Кликабельная шапка сворачиваемой секции: заголовок + шеврон, как у
/// карточек таба «Детали».
fn collapsible_title(
    text: impl Into<String>,
    open: bool,
    toggle: impl Fn() + Send + Sync + 'static,
) -> impl Widget {
    GestureDetector::new().on_click(toggle).child(mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                section_title(text.into()),
                Icon::new(if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE })
                    .class("right-section-chevron"),
            ]
    })
}

fn sampling_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let open = ctx.sampling_open.get();
        let head = collapsible_title(tr!("chat.right.sampling.title"), open, move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.sampling_open.set(!ctx.sampling_open.get_untracked());
        });
        if !open {
            return DecoratedBox::new()
                .class("sampling-card")
                .child(Stack::new().children(vec![Box::new(head) as Box<dyn Widget>]));
        }
        let p = ctx.params.get();
        let profile = use_context::<SynModelRegistry>().sampling.get();
        let custom = p.mode() == SamplingMode::Custom;
        // В режиме `default` слайдеры показывают то, с чем пойдёт ход, —
        // пресет модели, — и не правятся.
        let shown = match (&profile, custom) {
            (Some(prof), false) => p.effective(prof),
            _ => p.clone(),
        };
        let edit = custom;

        let mode_switch = SegmentedButton::new(vec![
            tr!("chat.right.sampling.mode.default"),
            tr!("chat.right.sampling.mode.custom"),
        ])
        .selected(usize::from(custom))
        .on_change(|i| {
            let ctx = use_context::<SynChatCtx>();
            let profile = use_context::<SynModelRegistry>().sampling.get_untracked();
            ctx.params.update(|q| {
                if i == 1 {
                    // «Свои» начинаются с того, что модель и так давала, —
                    // иначе слайдеры прыгнули бы на давно забытые значения.
                    if q.mode() == SamplingMode::Default {
                        if let Some(prof) = &profile {
                            *q = q.effective(prof);
                        }
                    }
                    q.mode = Some(SamplingMode::Custom);
                } else {
                    q.mode = Some(SamplingMode::Default);
                }
            });
        })
        .class("sampling-mode-switch");

        let mut rows: Vec<Box<dyn Widget>> = vec![
            Box::new(head),
            Box::new(Tooltip::new(mode_switch, tr!("chat.right.sampling.mode.tooltip"))),
            preset_row(&p, profile.as_deref()),
        ];
        rows.extend([
            slider_row("Temperature", shown.temperature, 0.0, 2.0, 0.05, 2, edit, |v, q| q.temperature = v),
            slider_row("top_p", shown.top_p, 0.0, 1.0, 0.05, 2, edit, |v, q| q.top_p = v),
            slider_row("top_k", shown.top_k as f32, 0.0, 200.0, 1.0, 0, edit, |v, q| {
                q.top_k = v.round() as u32;
            }),
            slider_row("min_p", shown.min_p, 0.0, 1.0, 0.01, 2, edit, |v, q| q.min_p = v),
            slider_row("repeat_penalty", shown.repeat_penalty, 1.0, 2.0, 0.01, 2, edit, |v, q| {
                q.repeat_penalty = v;
            }),
        ]);
        rows.push(spin_row("repeat_last_n", shown.repeat_last_n as f64, 0.0, 512.0, 8.0, edit, |v, q| {
            q.repeat_last_n = v.round().max(0.0) as u32;
        }));
        rows.extend([
            slider_row("presence_penalty", shown.presence_penalty, -2.0, 2.0, 0.05, 2, edit, |v, q| {
                q.presence_penalty = v;
            }),
            slider_row("frequency_penalty", shown.frequency_penalty, -2.0, 2.0, 0.05, 2, edit, |v, q| {
                q.frequency_penalty = v;
            }),
        ]);
        rows.push(Box::new(seed_row(p.seed)));

        DecoratedBox::new().class("sampling-card").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        )
    }
}

/// Комбобокс пресетов модели. В режиме `default` выбирает, какой пресет
/// работает (пресет режима размышлений заодно переключает и их); в `custom`
/// — заливает значения пресета в слайдеры как отправную точку.
fn preset_row(p: &SamplingParams, profile: Option<&synaptix::facade::llm::SamplingProfile>) -> Box<dyn Widget> {
    let Some(profile) = profile else {
        return Box::new(Text::new(tr!("chat.right.sampling.preset.no_model")).class("sampling-hint"));
    };
    let selected = match p.mode() {
        SamplingMode::Default => profile.pick(&p.preset, p.enable_thinking).map(|x| x.id),
        SamplingMode::Custom => profile.preset(&p.preset).map(|x| x.id),
    };
    let items: Vec<DropdownItem> = profile
        .presets
        .iter()
        .map(|x| DropdownItem::new(x.id, preset_label(x.id)))
        .collect();
    let presets = profile.presets.clone();
    let picker = Dropdown::with_items(items)
        .selected(selected.unwrap_or_default().to_string())
        .leading_icon(MI_TUNE)
        .on_change(move |id| {
            let Some(preset) = presets.iter().find(|x| x.id == id) else { return };
            use_context::<SynChatCtx>().params.update(|q| {
                q.preset = preset.id.to_string();
                if let Some(thinking) = preset.thinking {
                    q.enable_thinking = thinking;
                }
                if q.mode() == SamplingMode::Custom {
                    *q = q.with_preset(preset);
                }
            });
        })
        .class("sampling-preset-picker");
    Box::new(DecoratedBox::new().class("sampling-row").child(mgui! {
        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Text::new(tr!("chat.right.sampling.preset.label")).class("sampling-label"),
            picker,
        ]
    }))
}

fn preset_label(id: &str) -> String {
    match id {
        "thinking" => tr!("chat.right.sampling.preset.thinking"),
        "thinking_coding" => tr!("chat.right.sampling.preset.thinking_coding"),
        "instruct" => tr!("chat.right.sampling.preset.instruct"),
        "recommended" => tr!("chat.right.sampling.preset.recommended"),
        "generation_config" => tr!("chat.right.sampling.preset.generation_config"),
        "generic" => tr!("chat.right.sampling.preset.generic"),
        other => other.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn slider_row(
    label: &'static str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    decimals: usize,
    enabled: bool,
    apply: fn(f32, &mut SamplingParams),
) -> Box<dyn Widget> {
    let value_text = format!("{value:.decimals$}");
    let slider = Slider::new()
        .range(min, max)
        .step(step)
        .value(value)
        .disabled(!enabled)
        .on_change(move |v| {
            let ctx = use_context::<SynChatCtx>();
            ctx.params.update(|q| apply(v, q));
        });
    Box::new(DecoratedBox::new().class("sampling-row").child(mgui! {
        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Text::new(label).class("sampling-label"),
                    Text::new(value_text).class("sampling-value"),
                ],
            slider,
        ]
    }))
}

#[allow(clippy::too_many_arguments)]
fn spin_row(
    label: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    enabled: bool,
    apply: fn(f64, &mut SamplingParams),
) -> Box<dyn Widget> {
    let spin = SpinBox::new()
        .range(min, max)
        .step(step)
        .decimal_places(0)
        .value(value)
        .disabled(!enabled)
        .on_change(move |v| {
            let ctx = use_context::<SynChatCtx>();
            ctx.params.update(|q| apply(v, q));
        });
    Box::new(DecoratedBox::new().class("sampling-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(label).class("sampling-label"),
                spin,
            ]
    }))
}

fn seed_row(seed: i64) -> impl Widget {
    let edit = TextField::new()
        .text(seed.to_string())
        .placeholder(tr!("chat.right.sampling.seed_placeholder"))
        .on_submit(move |s| {
            let parsed: i64 = s.trim().parse().unwrap_or(-1);
            let ctx = use_context::<SynChatCtx>();
            ctx.params.update(|q| q.seed = parsed);
        });
    DecoratedBox::new().class("sampling-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new("seed").class("sampling-label"),
                edit,
            ]
    })
}

// ── Карточка «Контекст» ──────────────────────────────────────────────

fn context_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let p = ctx.params.get();
        let rows: Vec<Box<dyn Widget>> = vec![
            Box::new(section_title(tr!("chat.right.context.title"))),
            slider_row(
                "max_new_tokens",
                p.max_new_tokens as f32,
                32.0,
                262144.0,
                256.0,
                0,
                true,
                |v, q| q.max_new_tokens = v.round() as u32,
            ),
            slider_row(
                "max_seq_len",
                p.max_seq_len as f32,
                1024.0,
                262144.0,
                1024.0,
                0,
                true,
                |v, q| q.max_seq_len = v.round() as u32,
            ),
        ];
        DecoratedBox::new().class("sampling-card").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        )
    }
}

// ── Карточка «Thinking» ──────────────────────────────────────────────

fn thinking_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let p = ctx.params.get();
        let on = p.enable_thinking;
        let toggle = Toggle::new().on(on).on_change(|v| {
            use_context::<SynChatCtx>().params.update(|q| q.enable_thinking = v);
        });
        let mut rows: Vec<Box<dyn Widget>> = vec![Box::new(mgui! {
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Text::new(tr!("chat.right.thinking.label")).class("sampling-label"),
                    toggle,
                ]
        })];

        // Глубина — только у моделей, чей шаблон её настраивает (Qwen3.8,
        // Muse Glimmer), и только при включённых размышлениях: без них
        // уровень шаблон не читает.
        let levels = use_context::<SynModelRegistry>()
            .sampling
            .get()
            .and_then(|prof| prof.reasoning.clone());
        if let (true, Some(levels)) = (on, levels) {
            let mut items = vec![DropdownItem::new(
                "",
                tr!("chat.right.thinking.level.model_default", level = level_label(&levels.default)),
            )];
            items.extend(levels.levels.iter().map(|l| DropdownItem::new(l.clone(), level_label(l))));
            let selected = levels.resolve(p.effort()).unwrap_or_default().to_string();
            let picker = Dropdown::with_items(items)
                .selected(selected)
                .leading_icon(MI_PSYCHOLOGY)
                .on_change(|id| {
                    let id = id.to_string();
                    use_context::<SynChatCtx>().params.update(|q| q.reasoning_effort = id.clone());
                })
                .class("sampling-preset-picker");
            rows.push(Box::new(DecoratedBox::new().class("sampling-row").child(mgui! {
                Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("chat.right.thinking.level.label")).class("sampling-label"),
                    picker,
                ]
            })));
        }

        DecoratedBox::new().class("sampling-card").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        )
    }
}

fn level_label(level: &str) -> String {
    match level {
        "low" => tr!("chat.right.thinking.level.low"),
        "medium" => tr!("chat.right.thinking.level.medium"),
        "high" => tr!("chat.right.thinking.level.high"),
        "xhigh" => tr!("chat.right.thinking.level.xhigh"),
        other => other.to_string(),
    }
}

// ── Карточка «Системный prompt» ──────────────────────────────────────
//
// Библиотека пресетов (`SynChatCtx.prompt_presets`): дропдаун переключает
// активный, кнопки в шапке — создать / переименовать / удалить и открыть
// текст в плавающем окне (`prompt_window`). Редактор внизу и окно правят
// один и тот же `system_prompt`; на диск текст уходит через эффект в
// `lib.rs::install_syn_chat_autosave`.

/// Карточка «Система» как самостоятельный виджет — для harness-тестов
/// (`tests/system_prompt_card_layout.rs`); панель собирает её через
/// [`system_prompt_card_reactive`].
pub fn system_prompt_card() -> impl Widget {
    DecoratedBox::new().child(system_prompt_card_reactive())
}

fn system_prompt_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let cur = ctx.system_prompt.get();
        let presets = ctx.prompt_presets.get();
        let active = ctx.prompt_active.get();
        let active_name = presets
            .iter()
            .find(|p| p.id == active)
            .map(|p| p.name.clone())
            .unwrap_or_default();

        let items: Vec<DropdownItem> = presets
            .iter()
            .map(|p| DropdownItem::new(p.id.clone(), p.name.clone()))
            .collect();
        let picker = Dropdown::with_items(items)
            .selected(active.clone())
            .leading_icon(MI_DESCRIPTION)
            .on_change(|id| {
                let ctx = use_context::<SynChatCtx>();
                prompt_presets::select(&ctx, id);
            })
            .class("system-prompt-picker");

        let add = ToolButton::new(MI_ADD)
            .tooltip(tr!("chat.right.system.preset.add"))
            .on_click(|| {
                use_context::<SynChatCtx>().prompt_dialog.set(Some(PromptDialog::Create));
            })
            .class("system-prompt-action");
        let rename = {
            let (id, name) = (active.clone(), active_name.clone());
            ToolButton::new(MI_DRIVE_FILE_RENAME_OUTLINE)
                .tooltip(tr!("chat.right.system.preset.rename"))
                .on_click(move || {
                    use_context::<SynChatCtx>().prompt_dialog.set(Some(PromptDialog::Rename {
                        id: id.clone(),
                        name: name.clone(),
                    }));
                })
                .class("system-prompt-action")
        };
        let delete = {
            let (id, name) = (active.clone(), active_name.clone());
            ToolButton::new(MI_DELETE)
                .tooltip(tr!("chat.right.system.preset.delete"))
                .on_click(move || {
                    use_context::<SynChatCtx>().prompt_dialog.set(Some(PromptDialog::Delete {
                        id: id.clone(),
                        name: name.clone(),
                    }));
                })
                .class("system-prompt-action")
        };
        let open_window = ToolButton::new(MI_OPEN_IN_NEW)
            .tooltip(tr!("chat.right.system.preset.open_window"))
            .active(ctx.prompt_window_open.get())
            .on_click(|| {
                use_context::<SynChatCtx>().prompt_window_open.set(true);
            })
            .class("system-prompt-action");

        let edit = syngui::widgets::MultilineTextEdit::new()
            .text(cur)
            .placeholder(tr!("chat.right.system.placeholder"))
            .rows(2)
            .max_rows(8)
            .auto_height(true)
            .on_change(|s| {
                let ctx = use_context::<SynChatCtx>();
                ctx.system_prompt.set(s.to_string());
            })
            .class("system-prompt-edit");

        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                        // Заголовок — flex-элемент (см. `.system-prompt-head-title`):
                        // кнопки справа держат размер, заголовок ужимается.
                        DecoratedBox::new().class("system-prompt-head-title") => [
                            Text::new(tr!("chat.right.system.title"))
                                .max_lines(1)
                                .class("right-panel-section-title"),
                        ],
                        Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            add,
                            rename,
                            delete,
                            open_window,
                        ],
                    ],
                picker,
                edit,
            ]
        })
    }
}

// ── Кнопка «Сбросить к дефолтам» ─────────────────────────────────────

fn reset_button() -> impl Widget {
    let btn = ToolButton::new(MI_AUTORENEW)
        .tooltip(tr!("chat.right.reset.tooltip"))
        .on_click(|| {
            let ctx = use_context::<SynChatCtx>();
            let defaults = crate::config::AppConfig::load().syn_chat_defaults;
            ctx.params.set(defaults);
        })
        .class("right-reset-btn");
    DecoratedBox::new().class("right-reset-wrap").child(btn)
}

// ─────────────────────── ТАБ «ДЕТАЛИ» ───────────────────────
//
// Панель — дашборд из карточек: сверху основной цикл чата, под ним
// карточки живых субагентов (в том же столбце, без отступа), внизу
// размер чата. Все карточки сворачиваются: цепочка из нескольких
// субагентов иначе не помещается в панель.

fn details_tab() -> impl Widget {
    ScrollView::new().vertical().child(mgui! {
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                main_card_reactive(),
                subagent_cards_reactive(),
                chat_size_card_reactive(),
            ]
    })
}

/// Данные одной карточки цикла. Основной чат и субагенты рисуются одним
/// [`run_card`] — иначе их метрики разъезжаются при первой же правке.
struct CardView {
    /// Ключ раскрытия в `SynChatCtx.details_open`.
    id: u64,
    icon: &'static str,
    title: String,
    /// Вторая строка заголовка: модель у основного чата, задача у субагента.
    subtitle: String,
    /// Строка под заголовком: ход, текущий инструмент, число вызовов.
    status: Option<String>,
    /// Плашка справа от заголовка: состояние цикла.
    badge: Option<(String, &'static str)>,
    /// Цикл работает прямо сейчас — иконка пульсирует, карточка подсвечена.
    live: bool,
    stats: RunStats,
    /// Ходов сделано — отдельной плиткой.
    turns: u32,
    /// Раскрыта ли карточка, пока пользователь не щёлкнул по ней сам.
    default_open: bool,
}

fn main_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let live = ctx.pending.get();
        let stats = RunStats {
            prompt_tokens: ctx.last_prompt_tokens.get(),
            reused_tokens: ctx.last_reused_tokens.get(),
            gen_tokens: ctx.last_gen_tokens.get(),
            prefill_ms: ctx.last_prefill_ms.get(),
            decode_tps: ctx.last_decode_tps.get(),
            ring_tokens: ctx.last_ring_tokens.get(),
            ring_bytes: ctx.kv_cache_bytes.get(),
            ctx_budget: ctx.ctx_budget_tokens.get(),
            vram_free_mb: ctx.last_vram_free_mb.get(),
            blocks_resident: ctx.last_blocks_resident.get(),
        };
        // Подпись — имя бандла: числа карточки принадлежат именно ему, а в
        // табе «Детали» модель больше нигде не видна.
        let model = use_context::<SynModelRegistry>()
            .current
            .get()
            .and_then(|m| m.path.file_name().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| tr!("chat.model.not_loaded"));
        run_card(CardView {
            id: ROOT_RUN,
            icon: MI_CHAT,
            title: tr!("chat.right.details.main.title"),
            subtitle: model,
            status: live.then(|| tr!("chat.right.details.status.generating")),
            badge: live.then(|| {
                (
                    tr!("chat.right.details.state.running"),
                    "details-badge live",
                )
            }),
            live,
            stats,
            turns: ctx.last_turns.get(),
            default_open: true,
        })
    }
}

/// Карточки вложенных циклов. Порядок — тот, в котором их запускали;
/// выровнены по основному чату, глубина — в заголовке.
fn subagent_cards_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let runs = ctx.agent_runs.get();
        let cards: Vec<Box<dyn Widget>> = runs
            .iter()
            .map(|r| Box::new(run_card(card_of(r))) as Box<dyn Widget>)
            .collect();
        DecoratedBox::new().class("details-runs").child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(cards),
        )
    }
}

fn card_of(run: &AgentRun) -> CardView {
    let title = match run.kind {
        RunKind::Subagent if run.depth > 1 => {
            tr!("chat.right.details.subagent.nested", depth = run.depth)
        }
        RunKind::Subagent => tr!("chat.right.details.subagent.title"),
    };

    // Живой цикл рассказывает, где он сейчас; завершённый — сколько всего
    // сделал. И то и другое короче, чем «карточка молчит».
    let status = if run.state.is_running() {
        let mut s = if run.max_turns > 0 {
            tr!(
                "chat.right.details.status.turn_of",
                turn = run.turn.max(1),
                max = run.max_turns
            )
        } else {
            tr!("chat.right.details.status.turn", turn = run.turn.max(1))
        };
        if let Some(tool) = run.tool.as_deref() {
            s.push_str(" · ");
            s.push_str(&tr!("chat.right.details.status.tool", tool = tool));
        }
        Some(s)
    } else {
        Some(tr!(
            "chat.right.details.status.summary",
            turns = run.turn,
            calls = run.tool_calls
        ))
    };

    let badge = Some(match run.state {
        RunState::Running => (
            tr!("chat.right.details.state.running"),
            "details-badge live",
        ),
        RunState::Done => (tr!("chat.right.details.state.done"), "details-badge ok"),
        RunState::Failed => (tr!("chat.right.details.state.failed"), "details-badge err"),
        RunState::Aborted => (
            tr!("chat.right.details.state.aborted"),
            "details-badge warn",
        ),
    });

    CardView {
        id: run.id,
        icon: MI_BOLT,
        title,
        subtitle: run.label.clone(),
        status,
        badge,
        live: run.state.is_running(),
        stats: run.stats,
        turns: run.turn,
        // Работающий цикл раскрыт — ради него панель и открывали;
        // завершённый сворачивается, чтобы не оттеснять живые.
        default_open: run.state.is_running(),
    }
}

/// Общий рендер карточки: кликабельный заголовок + свёрнутое тело
/// (строка состояния, полоса заполнения ринга, сетка метрик 2×5).
fn run_card(v: CardView) -> StyledWidget<DecoratedBox> {
    let ctx = use_context::<SynChatCtx>();
    let id = v.id;
    let open = ctx
        .details_open
        .get()
        .get(&id)
        .copied()
        .unwrap_or(v.default_open);
    let default_open = v.default_open;

    let icon_class = if v.live {
        "details-run-icon details-live-dot"
    } else {
        "details-run-icon"
    };
    let mut head: Vec<Box<dyn Widget>> = vec![
        Box::new(Icon::new(v.icon).class(icon_class)),
        Box::new(DecoratedBox::new().class("grow").child(mgui! {
            Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(v.title).class("details-run-title"),
                Text::new(v.subtitle).max_lines(2).class("details-run-sub"),
            ]
        })),
    ];
    if let Some((text, class)) = v.badge {
        head.push(Box::new(
            DecoratedBox::new()
                .class(class)
                .child(Text::new(text).class("details-badge-text")),
        ));
    }
    head.push(Box::new(
        Icon::new(if open { MI_EXPAND_LESS } else { MI_EXPAND_MORE })
            .class("details-run-chevron"),
    ));

    let header = GestureDetector::new()
        .on_click(move || {
            let ctx = use_context::<SynChatCtx>();
            ctx.details_open.update(|m| {
                let cur = m.get(&id).copied().unwrap_or(default_open);
                m.insert(id, !cur);
            });
        })
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(head),
        );

    let mut children: Vec<Box<dyn Widget>> = vec![Box::new(header)];
    if open {
        if let Some(status) = v.status {
            children.push(Box::new(
                DecoratedBox::new()
                    .class("details-status")
                    .child(Text::new(status).max_lines(2).class("details-status-text")),
            ));
        }
        children.push(Box::new(ring_usage(&v.stats)));
        children.push(Box::new(
            Grid::new(2)
                .gap(6.0)
                .children(stat_tiles(&v.stats, v.turns)),
        ));
    }

    // Карточки субагентов стоят в одном столбце с основным чатом, без
    // сдвига: глубину рекурсии называет заголовок («уровень N»).
    let class = if v.live {
        "details-card details-card-live"
    } else {
        "details-card"
    };
    DecoratedBox::new()
        .class(class)
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children),
        )
}

/// Полоса «сколько ринга занял промпт». Именно она предсказывает и обрезку
/// контекста, и OOM: цифры промпта и ринга рядом в сетке, но соотношение
/// глазами не считается.
fn ring_usage(s: &RunStats) -> impl Widget {
    let pct = if s.ring_tokens > 0 {
        (s.prompt_tokens as f32 / s.ring_tokens as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let caption = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(tr!("chat.right.details.ring_usage")).class("details-bar-label"),
                Text::new(format!("{}%", (pct * 100.0).round() as u32)).class("details-bar-value"),
            ]
    };
    DecoratedBox::new().class("details-bar").child(mgui! {
        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            caption,
            ProgressBar::with_value(pct).class("details-ctx-bar"),
        ]
    })
}

/// Десять плиток «значение + подпись» — тот же набор чисел, что панель
/// показывала строками, но вдвое плотнее и читается по вертикали.
fn stat_tiles(s: &RunStats, turns: u32) -> Vec<Box<dyn Widget>> {
    let (prefill_value, prefill_label) = if s.prefill_ms >= 1000 {
        (
            format!("{:.1}", s.prefill_ms as f32 / 1000.0),
            tr!("chat.right.details.tile.prefill_s"),
        )
    } else {
        (
            s.prefill_ms.to_string(),
            tr!("chat.right.details.tile.prefill_ms"),
        )
    };
    let mut rows: Vec<(String, String, bool)> = vec![
        (
            format!("{:.1}", s.decode_tps),
            tr!("chat.right.details.tile.tps"),
            true,
        ),
        (prefill_value, prefill_label, false),
        (
            group(s.prompt_tokens as u64),
            tr!("chat.right.details.tile.prompt"),
            false,
        ),
        (
            group(s.reused_tokens as u64),
            tr!("chat.right.details.tile.cached"),
            false,
        ),
        (
            group(s.gen_tokens as u64),
            tr!("chat.right.details.tile.answer"),
            false,
        ),
        (
            turns.to_string(),
            tr!("chat.right.details.tile.turns"),
            false,
        ),
        (
            group(s.ring_tokens as u64),
            tr!("chat.right.details.tile.ring_tokens"),
            false,
        ),
        (
            group(s.ring_bytes / (1024 * 1024)),
            tr!("chat.right.details.tile.ring_mb"),
            false,
        ),
        (
            group(s.ctx_budget as u64),
            tr!("chat.right.details.tile.ctx_budget"),
            false,
        ),
        (
            group(s.vram_free_mb as u64),
            tr!("chat.right.details.tile.vram_free"),
            false,
        ),
    ];
    // Только при частичном оффлоаде: при полной резидентности плитки нет,
    // а «62/64» рядом со скоростью объясняет, куда делась её треть.
    if let Some((on_card, total)) = s.blocks_resident {
        rows.push((
            format!("{on_card}/{total}"),
            tr!("chat.right.details.tile.blocks"),
            true,
        ));
    }
    rows.into_iter()
        .map(|(value, label, accent)| stat_tile(value, label, accent))
        .collect()
}

fn stat_tile(value: String, label: String, accent: bool) -> Box<dyn Widget> {
    let value_class = if accent {
        "details-tile-value details-tile-accent"
    } else {
        "details-tile-value"
    };
    Box::new(DecoratedBox::new().class("details-tile").child(mgui! {
        Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Text::new(value).class(value_class),
            Text::new(label).max_lines(2).class("details-tile-label"),
        ]
    }))
}

fn chat_size_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let (n, chars) = ctx.messages.with(|msgs| {
            let chars: usize = msgs
                .iter()
                .map(|m| m.body.chars().count() + m.thinking.chars().count())
                .sum();
            (msgs.len(), chars)
        });
        let tiles: Vec<Box<dyn Widget>> = vec![
            stat_tile(
                group(n as u64),
                tr!("chat.right.details.tile.messages"),
                false,
            ),
            stat_tile(
                group(chars as u64),
                tr!("chat.right.details.tile.characters"),
                false,
            ),
        ];
        DecoratedBox::new().class("details-card").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_ARTICLE).class("details-run-icon"),
                    Text::new(tr!("chat.right.details.chat_size.title")).class("details-run-title"),
                ],
                Grid::new(2).gap(6.0).children(tiles),
            ]
        })
    }
}

/// Разряды через неразрывный пробел: «353 098» читается с одного взгляда,
/// «353098» — нет.
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('\u{00A0}');
        }
        out.push(ch);
    }
    out
}

// ─────────────────────── Общие хелперы ───────────────────────

fn section_title(text: impl Into<String>) -> impl Widget {
    Text::new(text.into()).class("right-panel-section-title")
}

/// Запуск `SynModelRegistry::load` из любого потока — get/set сигналов
/// сами маршализуются в main thread. policy читается ВНУТРИ run_on_main_thread,
/// потому что signal-runtime thread-local (use_context в spawn thread недоступен).
fn load_from_any_thread(path: PathBuf) {
    syngui::async_runtime::run_on_main_thread(move || {
        let app_ctx = use_context::<crate::context::AppCtx>();
        let policy =
            crate::config::resolve_model_profile(&app_ctx.model_profiles.get_untracked(), &path)
                .policy;
        use_context::<SynModelRegistry>().load(path, policy);
    });
}
