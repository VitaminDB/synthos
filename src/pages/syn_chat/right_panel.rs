//! Правая панель Syn-чата: TabBar с тремя табами — «Инструменты» (tools/skills,
//! первая по умолчанию), «Параметры» (модель + sampling + контекст + thinking)
//! и «Детали» (статистика последней генерации).

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::{Slider, SpinBox, TextField, Toggle};

use crate::components::right_panel::tools_panel;
use crate::context::{
    SYN_RIGHT_PANEL_DETAILS, SYN_RIGHT_PANEL_PARAMS, SYN_RIGHT_PANEL_TOOLS,
};
use crate::icons::*;
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::{SynChatCtx, SynModelRegistry};

pub fn view() -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    let tab = ctx.right_panel_tab;

    let tabbar = TabBar::new()
        .tab(Tab::new("Инструменты", SYN_RIGHT_PANEL_TOOLS, &tab).icon(MI_AUTO_AWESOME))
        .tab(Tab::new("Параметры", SYN_RIGHT_PANEL_PARAMS, &tab).icon(MI_TUNE))
        .tab(Tab::new("Детали", SYN_RIGHT_PANEL_DETAILS, &tab).icon(MI_SPEED))
        .class("right-panel-tabbar-inner");

    let body = DecoratedBox::new().class("right-panel-body").child(move || {
        let child: Box<dyn Widget> = match tab.get() {
            SYN_RIGHT_PANEL_DETAILS => Box::new(details_tab()),
            SYN_RIGHT_PANEL_PARAMS => Box::new(params_tab()),
            _ => Box::new(tools_tab()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    });

    DecoratedBox::new()
        .class("right-panel syn-chat-right")
        .child(mgui! {
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("right-panel-tabbar").child(tabbar),
                body,
            ]
        })
}

// ─────────────────────── ТАБ «ИНСТРУМЕНТЫ» ───────────────────────

fn tools_tab() -> impl Widget {
    ScrollView::new().vertical().child(mgui! {
        Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                tools_panel::tools_section(),
                tools_panel::skills_section(),
            ]
    })
}

// ─────────────────────── ТАБ «ПАРАМЕТРЫ» ───────────────────────

fn params_tab() -> impl Widget {
    ScrollView::new().vertical().child(mgui! {
        Column::new()
            .gap(16.0)
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
                section_title("Модель"),
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
                "Загрузка модели…".to_string(),
                "5-15 секунд".to_string(),
                "model-status loading",
            )
        } else if let Some(err) = error.as_ref() {
            (
                MI_REPORT,
                "Ошибка".to_string(),
                err.clone(),
                "model-status error",
            )
        } else if let Some(loaded) = current.as_ref() {
            let name = loaded
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "—".to_string());
            (MI_CHECK, "Готово".to_string(), name, "model-status ready")
        } else {
            // Путь помним, но модель не поднимаем — показываем, что именно
            // поднимет кнопка «Загрузить модель».
            let hint = reg
                .last_path
                .get()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_else(|| "Выберите .syn-bundle".to_string());
            (MI_INFO, "Модель не загружена".to_string(), hint, "model-status idle")
        };

        // Бейдж мультимодальности: есть ли в бандле vision-башня. Отвечает
        // на вопрос «поймёт ли эта модель прикреплённую картинку» до того,
        // как пользователь потратит время на отправку.
        let media_badge = current.as_ref().map(|loaded| {
            if loaded.model.supports_media() {
                "Мультимодальная · картинки и видео".to_string()
            } else {
                "Текстовая · вложения уйдут описанием".to_string()
            }
        });

        // Quant-бейдж: подпись с текущей политикой квантования. Формат:
        //   "Balance · NVFP4 / FP8 KV / F16 lm_head"
        // Показывается только когда модель загружена (иначе политика
        // ещё может измениться).
        let quant_badge = if current.is_some() {
            let q = use_context::<crate::context::AppCtx>().syn_chat_quant.get();
            let preset_label = match q.preset.as_str() {
                "quality" => "Quality",
                "balance" => "Balance",
                "vram_saver" => "VRAM-Saver",
                _ => "Custom",
            };
            let kv_label = if q.kv_dtype == "fp8e4m3" {
                "FP8 KV".to_string()
            } else {
                format!("{} KV", q.kv_dtype.to_uppercase())
            };
            Some(format!(
                "{} · {} / {} / {} lm_head",
                preset_label,
                q.weights_storage.to_uppercase(),
                kv_label,
                q.lm_head_storage.to_uppercase()
            ))
        } else {
            None
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
            .tooltip("Выбрать .syn-bundle")
            .on_click(move || {
                if reg.loading.get_untracked() {
                    return;
                }
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter("Syn bundle", &["syn"])
                        .set_title("Выберите .syn с LLM (Qwen3.6/3.8, Muse Glimmer)")
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
            Button::new("Выгрузить модель")
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
                .class("right-unload-btn-full")
        } else {
            let last = reg.last_path.get();
            Button::new("Загрузить модель")
                .leading_icon(MI_POWER_SETTINGS)
                .disabled(loading || last.is_none())
                .on_click(move || {
                    if let Some(p) = last.clone() {
                        load_from_any_thread(p);
                    }
                })
                .class("right-unload-btn-full")
        };

        DecoratedBox::new()
            .child(mgui! {
                Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        DecoratedBox::new().class("grow").child(pick),
                    ],
                    toggle,
                ]
            })
            .class("right-pick-row")
    }
}

// ── Карточка «Sampling» ──────────────────────────────────────────────

fn sampling_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let p = ctx.params.get();

        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title("Sampling"),
                slider_row("Temperature", p.temperature, 0.0, 2.0, 0.05, 2, |v, q| q.temperature = v),
                slider_row("top_p", p.top_p, 0.0, 1.0, 0.05, 2, |v, q| q.top_p = v),
                slider_row("top_k", p.top_k as f32, 0.0, 200.0, 1.0, 0, |v, q| q.top_k = v.round() as u32),
                slider_row("min_p", p.min_p, 0.0, 1.0, 0.01, 2, |v, q| q.min_p = v),
                slider_row(
                    "repeat_penalty",
                    p.repeat_penalty,
                    1.0,
                    2.0,
                    0.01,
                    2,
                    |v, q| q.repeat_penalty = v,
                ),
                spin_row("repeat_last_n", p.repeat_last_n as f64, 0.0, 512.0, 8.0, |v, q| {
                    q.repeat_last_n = v.round().max(0.0) as u32;
                }),
                slider_row(
                    "presence_penalty",
                    p.presence_penalty,
                    -2.0,
                    2.0,
                    0.05,
                    2,
                    |v, q| q.presence_penalty = v,
                ),
                slider_row(
                    "frequency_penalty",
                    p.frequency_penalty,
                    -2.0,
                    2.0,
                    0.05,
                    2,
                    |v, q| q.frequency_penalty = v,
                ),
                seed_row(p.seed),
            ]
        })
    }
}

fn slider_row(
    label: &'static str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    decimals: usize,
    apply: fn(f32, &mut SamplingParams),
) -> impl Widget {
    let value_text = format!("{value:.decimals$}");
    let slider = Slider::new()
        .range(min, max)
        .step(step)
        .value(value)
        .on_change(move |v| {
            let ctx = use_context::<SynChatCtx>();
            ctx.params.update(|q| apply(v, q));
        });
    DecoratedBox::new().class("sampling-row").child(mgui! {
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
    })
}

fn spin_row(
    label: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    apply: fn(f64, &mut SamplingParams),
) -> impl Widget {
    let spin = SpinBox::new()
        .range(min, max)
        .step(step)
        .decimal_places(0)
        .value(value)
        .on_change(move |v| {
            let ctx = use_context::<SynChatCtx>();
            ctx.params.update(|q| apply(v, q));
        });
    DecoratedBox::new().class("sampling-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(label).class("sampling-label"),
                spin,
            ]
    })
}

fn seed_row(seed: i64) -> impl Widget {
    let edit = TextField::new()
        .text(seed.to_string())
        .placeholder("-1 = случайный")
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
        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title("Контекст"),
                slider_row(
                    "max_new_tokens",
                    p.max_new_tokens as f32,
                    32.0,
                    262144.0,
                    256.0,
                    0,
                    |v, q| q.max_new_tokens = v.round() as u32,
                ),
                slider_row(
                    "max_seq_len",
                    p.max_seq_len as f32,
                    1024.0,
                    262144.0,
                    1024.0,
                    0,
                    |v, q| q.max_seq_len = v.round() as u32,
                ),
            ]
        })
    }
}

// ── Карточка «Thinking» ──────────────────────────────────────────────

fn thinking_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let on = ctx.params.get().enable_thinking;
        let toggle = Toggle::new().on(on).on_change(|v| {
            use_context::<SynChatCtx>().params.update(|q| q.enable_thinking = v);
        });
        DecoratedBox::new().class("sampling-card").child(mgui! {
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Text::new("Thinking-режим").class("sampling-label"),
                    toggle,
                ]
        })
    }
}

// ── Карточка «Системный prompt» ──────────────────────────────────────

fn system_prompt_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let cur = ctx.system_prompt.get();
        let edit = syngui::widgets::MultilineTextEdit::new()
            .text(cur)
            .placeholder("Системный prompt (опционально)")
            .rows(2)
            .max_rows(8)
            .auto_height(true)
            .on_change(|s| {
                let ctx = use_context::<SynChatCtx>();
                ctx.system_prompt.set_always(s.to_string());
            })
            .class("system-prompt-edit");
        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title("Система"),
                edit,
            ]
        })
    }
}

// ── Кнопка «Сбросить к дефолтам» ─────────────────────────────────────

fn reset_button() -> impl Widget {
    let btn = ToolButton::new(MI_AUTORENEW)
        .tooltip("Сбросить к дефолтам")
        .on_click(|| {
            let ctx = use_context::<SynChatCtx>();
            let defaults = crate::config::AppConfig::load().syn_chat_defaults;
            ctx.params.set(defaults);
        })
        .class("right-reset-btn");
    DecoratedBox::new().class("right-reset-wrap").child(btn)
}

// ─────────────────────── ТАБ «ДЕТАЛИ» ───────────────────────

fn details_tab() -> impl Widget {
    ScrollView::new().vertical().child(mgui! {
        Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                stats_card_reactive(),
                chat_size_card_reactive(),
            ]
    })
}

fn stats_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let prompt_t = ctx.last_prompt_tokens.get();
        let gen_t = ctx.last_gen_tokens.get();
        let prefill_ms = ctx.last_prefill_ms.get();
        let tps = ctx.last_decode_tps.get();

        DecoratedBox::new().class("details-card").child(mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title("Последняя генерация"),
                metric_row("prompt_tokens", prompt_t.to_string()),
                metric_row("gen_tokens", gen_t.to_string()),
                metric_row("prefill_ms", prefill_ms.to_string()),
                metric_row("decode_tps", format!("{tps:.1}")),
            ]
        })
    }
}

fn chat_size_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let msgs = ctx.messages.get();
        let n = msgs.len();
        let chars: usize = msgs.iter().map(|m| m.body.chars().count() + m.thinking.chars().count()).sum();
        DecoratedBox::new().class("details-card").child(mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title("Размер чата"),
                metric_row("messages", n.to_string()),
                metric_row("characters", chars.to_string()),
            ]
        })
    }
}

fn metric_row(label: &'static str, value: String) -> impl Widget {
    DecoratedBox::new().class("details-metric-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(label).class("details-metric-label"),
                Text::new(value).class("details-metric-value"),
            ]
    })
}

// ─────────────────────── Общие хелперы ───────────────────────

fn section_title(text: &'static str) -> impl Widget {
    Text::new(text).class("right-panel-section-title")
}

/// Запуск `SynModelRegistry::load` из любого потока — get/set сигналов
/// сами маршализуются в main thread. policy читается ВНУТРИ run_on_main_thread,
/// потому что signal-runtime thread-local (use_context в spawn thread недоступен).
fn load_from_any_thread(path: PathBuf) {
    syngui::async_runtime::run_on_main_thread(move || {
        let app_ctx = use_context::<crate::context::AppCtx>();
        let policy = app_ctx.syn_chat_quant.get_untracked().to_policy();
        use_context::<SynModelRegistry>().load(path, policy);
    });
}
