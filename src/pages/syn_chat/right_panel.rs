//! Правая панель Syn-чата: TabBar с двумя табами — «Параметры» (модель +
//! sampling + контекст + thinking, первая по умолчанию) и «Детали»
//! (статистика последней генерации). TabBar ([`header`]) живёт в строке
//! заголовков каркаса, тело ([`body`]) — под ним. Инструменты и скилы — в
//! левой панели (`left_panel`).

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::{Slider, SpinBox, TextField, Toggle};

use crate::context::{SYN_RIGHT_PANEL_DETAILS, SYN_RIGHT_PANEL_PARAMS};
use crate::icons::*;
use crate::syn_chat::params::SamplingParams;
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

fn sampling_card_reactive() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<SynChatCtx>();
        let p = ctx.params.get();

        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title(tr!("chat.right.sampling.title")),
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
        DecoratedBox::new().class("sampling-card").child(mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title(tr!("chat.right.context.title")),
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
                    Text::new(tr!("chat.right.thinking.label")).class("sampling-label"),
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
            .placeholder(tr!("chat.right.system.placeholder"))
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
                section_title(tr!("chat.right.system.title")),
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
        let turns = ctx.last_turns.get();
        let ring_tokens = ctx.last_ring_tokens.get();
        let ring_mb = ctx.kv_cache_bytes.get() / (1024 * 1024);
        let ctx_budget = ctx.ctx_budget_tokens.get();
        let vram_free = ctx.last_vram_free_mb.get();
        let reused = ctx.last_reused_tokens.get();

        DecoratedBox::new().class("details-card").child(mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                section_title(tr!("chat.right.details.last_gen.title")),
                // prompt_tokens — промпт ПОСЛЕДНЕГО хода agent-loop'а: после
                // tool-вызовов он в разы больше первого, и именно он определяет
                // размер KV-ринга.
                metric_row("prompt_tokens", prompt_t.to_string()),
                // Сколько из промпта взято из кэша прошлого хода (префикс-KV):
                // столько токенов не пришлось префиллить заново.
                metric_row(tr!("chat.right.details.from_cache"), reused.to_string()),
                metric_row("gen_tokens", gen_t.to_string()),
                metric_row("prefill_ms", prefill_ms.to_string()),
                metric_row("decode_tps", format!("{tps:.1}")),
                metric_row(tr!("chat.right.details.agent_turns"), turns.to_string()),
                metric_row(tr!("chat.right.details.kv_ring"), tr!("chat.right.details.kv_ring.value", tokens = ring_tokens, mb = ring_mb)),
                metric_row(tr!("chat.right.details.vram_context"), tr!("chat.right.details.vram_context.value", tokens = ctx_budget)),
                metric_row(tr!("chat.right.details.vram_free"), format!("{vram_free} MB")),
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
                section_title(tr!("chat.right.details.chat_size.title")),
                metric_row("messages", n.to_string()),
                metric_row("characters", chars.to_string()),
            ]
        })
    }
}

fn metric_row(label: impl Into<String>, value: String) -> impl Widget {
    DecoratedBox::new().class("details-metric-row").child(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(label.into()).class("details-metric-label"),
                Text::new(value).class("details-metric-value"),
            ]
    })
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
