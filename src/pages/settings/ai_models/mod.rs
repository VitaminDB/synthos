//! Settings → AI Models — конфигуратор квантования Qwen3.6 для Syn-чата.
//!
//! Состоит из:
//! - Preset dropdown: `Quality` / `Balance` / `VRAM-Saver` / `Custom`.
//! - 7 dtype dropdowns: weights/compute/kv/lm_head/embed/ssm_state/conv_state.
//! - Apply & Reload Model button: вызывает `SynModelRegistry::load(path, policy)`
//!   с текущим `last_syn_model`.
//! - Info-блок: текущая политика + примерная оценка экономии VRAM.
//!
//! Изменения любого dropdown'а переключают preset на `"custom"`. Выбор
//! пресета заполняет все dtype-поля из соответствующего helper'а в
//! `synaptix::facade::llm::QuantPolicy`.

use std::path::PathBuf;

use syngui::prelude::*;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};
use syngui::widgets::input::toggle::Toggle;

use crate::config::SynChatQuantConfig;
use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::settings::widgets::{
    compute_dtype_dropdown, kv_dtype_dropdown, row_frame, section_card, storage_dtype_dropdown,
    tied_embeddings_dropdown,
};
use crate::syn_chat::SynModelRegistry;

pub fn view() -> impl Widget {
    ScrollView::new().vertical().child(move || {
        let ctx = use_context::<AppCtx>();
        let q = ctx.syn_chat_quant.get();

        let header = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Text::new(tr!("settings.ai_models.title")).class("settings-section-title"))
            .child(Text::new(tr!("settings.ai_models.subtitle"))
                .class("settings-row-desc"));

        let body = Column::new()
            .gap(24.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(header) as Box<dyn Widget>,
                models_dir_card(),
                runtime_card(),
                preset_card(&q),
                components_card(&q),
                apply_card(),
                acestep_paths_card(),
            ]);

        Padding::all(32.0).child(
            DecoratedBox::new()
                .class("settings-page models-page ai-models-page")
                .child(body),
        )
    })
}

/// Runtime-настройки inference: per-shot контролы которые применяются
/// без перезагрузки модели (в отличие от quant-policy ниже).
fn runtime_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let value = ctx.qwen36_attn_mode;
    let current = value.get_untracked();
    let items = vec![
        DropdownItem::new("fa4", tr!("settings.ai_models.attn.fa4")),
        DropdownItem::new("fa2", tr!("settings.ai_models.attn.fa2")),
        DropdownItem::new("off", tr!("settings.ai_models.attn.off")),
    ];
    let dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| value.set(s.to_string()))
            .class("models-active-dropdown"),
    );

    let graph_signal = ctx.qwen36_graph_decode;
    let graph_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(graph_signal.get_untracked())
            .on_change(move |v| graph_signal.set(v)),
    );

    let mtp_signal = ctx.qwen36_mtp;
    let mtp_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(mtp_signal.get_untracked())
            .on_change(move |v| mtp_signal.set(v)),
    );

    let dflash_signal = ctx.muse_dflash;
    let dflash_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(dflash_signal.get_untracked())
            .on_change(move |v| dflash_signal.set(v)),
    );

    let la_signal = ctx.qwen36_la_fused;
    let la_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(la_signal.get_untracked())
            .on_change(move |v| la_signal.set(v)),
    );

    let gdr_signal = ctx.qwen36_gdr_fused;
    let gdr_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(gdr_signal.get_untracked())
            .on_change(move |v| gdr_signal.set(v)),
    );

    let mma_signal = ctx.qwen36_nvfp4_mma;
    let mma_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(mma_signal.get_untracked())
            .on_change(move |v| mma_signal.set(v)),
    );

    let gemv_signal = ctx.qwen36_nvfp4_gemv;
    let gemv_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(gemv_signal.get_untracked())
            .on_change(move |v| gemv_signal.set(v)),
    );

    let chunk_signal = ctx.qwen36_prefill_chunk;
    let chunk_items = vec![
        DropdownItem::new("64", tr!("settings.ai_models.prefill_chunk.64")),
        DropdownItem::new("128", "128"),
        DropdownItem::new("256", tr!("settings.ai_models.prefill_chunk.256")),
        DropdownItem::new("512", "512"),
        DropdownItem::new("1024", tr!("settings.ai_models.prefill_chunk.1024")),
        DropdownItem::new("2048", tr!("settings.ai_models.prefill_chunk.2048")),
    ];
    let chunk_current = chunk_signal.get_untracked().to_string();
    let chunk_dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(chunk_items)
            .selected(chunk_current)
            .on_change(move |s| {
                if let Ok(n) = s.parse::<usize>() {
                    chunk_signal.set(n);
                }
            })
            .class("models-active-dropdown"),
    );

    // Потолок vision-токенов на картинку-вложение. Влияет и на длину
    // промпта, и на время prefill: 4096 токенов с одной картинки — это
    // как приложить к сообщению небольшую статью.
    let img_signal = ctx.syn_chat_max_image_tokens;
    let img_items = vec![
        DropdownItem::new("256", tr!("settings.ai_models.image_tokens.256")),
        DropdownItem::new("512", tr!("settings.ai_models.image_tokens.512")),
        DropdownItem::new("1024", tr!("settings.ai_models.image_tokens.1024")),
        DropdownItem::new("2048", tr!("settings.ai_models.image_tokens.2048")),
        DropdownItem::new("4096", tr!("settings.ai_models.image_tokens.4096")),
        DropdownItem::new("0", tr!("settings.ai_models.image_tokens.unlimited")),
    ];
    let img_current = img_signal.get_untracked().to_string();
    let img_dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(img_items)
            .selected(img_current)
            .on_change(move |s| {
                if let Ok(n) = s.parse::<usize>() {
                    img_signal.set(n);
                }
            })
            .class("models-active-dropdown"),
    );

    let sync_signal = ctx.qwen36_layer_sync;
    let sync_items = vec![
        DropdownItem::new("auto", tr!("settings.ai_models.layer_sync.auto")),
        DropdownItem::new("on", tr!("settings.ai_models.layer_sync.on")),
        DropdownItem::new("off", tr!("settings.ai_models.layer_sync.off")),
    ];
    let sync_current = sync_signal.get_untracked();
    let sync_dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(sync_items)
            .selected(sync_current)
            .on_change(move |s| sync_signal.set(s.to_string()))
            .class("models-active-dropdown"),
    );

    section_card(
        tr!("settings.ai_models.section.runtime"),
        vec![
            row_frame(
                MI_SPEED,
                tr!("settings.ai_models.attn"),
                tr!("settings.ai_models.attn.desc"),
                dropdown,
            ),
            row_frame(
                MI_AUTO_AWESOME,
                tr!("settings.ai_models.cuda_graph"),
                tr!("settings.ai_models.cuda_graph.desc"),
                graph_toggle,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.mtp"),
                tr!("settings.ai_models.mtp.desc"),
                mtp_toggle,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.dflash"),
                tr!("settings.ai_models.dflash.desc"),
                dflash_toggle,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.la_fused"),
                tr!("settings.ai_models.la_fused.desc"),
                la_toggle,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.gdr_fused"),
                tr!("settings.ai_models.gdr_fused.desc"),
                gdr_toggle,
            ),
            row_frame(
                MI_TUNE,
                tr!("settings.ai_models.prefill_chunk"),
                tr!("settings.ai_models.prefill_chunk.desc"),
                chunk_dropdown,
            ),
            row_frame(
                MI_MEMORY,
                tr!("settings.ai_models.layer_sync"),
                tr!("settings.ai_models.layer_sync.desc"),
                sync_dropdown,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.nvfp4_mma"),
                tr!("settings.ai_models.nvfp4_mma.desc"),
                mma_toggle,
            ),
            row_frame(
                MI_BOLT,
                tr!("settings.ai_models.nvfp4_gemv"),
                tr!("settings.ai_models.nvfp4_gemv.desc"),
                gemv_toggle,
            ),
            row_frame(
                MI_IMAGE_ICON,
                tr!("settings.ai_models.image_tokens"),
                tr!("settings.ai_models.image_tokens.desc"),
                img_dropdown,
            ),
        ],
    )
}

fn preset_card(q: &SynChatQuantConfig) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("quality", tr!("settings.ai_models.preset.quality")),
        DropdownItem::new("balance", tr!("settings.ai_models.preset.balance")),
        DropdownItem::new("vram_saver", tr!("settings.ai_models.preset.vram_saver")),
        DropdownItem::new("custom", tr!("settings.ai_models.preset.custom")),
    ];
    let current = q.preset.clone();
    let preset_control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                if s == "custom" {
                    // Custom — только переключение имени, dtype-поля
                    // оставляем как есть (пользователь дальше крутит вручную).
                    ctx.syn_chat_quant.update(|cfg| {
                        cfg.preset = "custom".into();
                    });
                } else {
                    // Заменяем все поля из встроенного пресета.
                    let new_cfg = SynChatQuantConfig::from_preset(&s);
                    ctx.syn_chat_quant.set(new_cfg);
                }
            })
            .class("models-active-dropdown"),
    );

    section_card(
        tr!("settings.ai_models.section.preset"),
        vec![row_frame(
            MI_AUTO_AWESOME,
            tr!("settings.ai_models.preset"),
            tr!("settings.ai_models.preset.desc"),
            preset_control,
        )],
    )
}

/// Карточка «Каталог моделей» — общая папка с .syn-бандлами/LoRA/HF-
/// каталогами для всех пайплайнов. Агентский инструмент `pipelines`
/// (action=list) перечисляет её содержимое, чтобы LLM проставляла пути в
/// чекпойнт-ноды сама. `~` в начале пути раскрывается при использовании.
fn models_dir_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let sig = ctx.models_dir;

    let field = syngui::widgets::input::TextField::with_text(sig.get_untracked())
        .placeholder(crate::config::default_models_dir())
        .on_change(move |s| sig.set(s.to_string()))
        .class("models-path-field");

    let browse = Button::new(tr!("app.browse"))
        .icon(MI_FOLDER_OPEN)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(tr!("settings.ai_models.section.models_dir"));
            if let Some(p) = dlg.pick_folder() {
                sig.set(p.display().to_string());
            }
        })
        .class("ai-models-apply-button");

    let control = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(field) as Box<dyn Widget>,
            Box::new(browse) as Box<dyn Widget>,
        ]);

    section_card(
        tr!("settings.ai_models.section.models_dir"),
        vec![row_frame(
            MI_FOLDER_OPEN,
            tr!("settings.ai_models.models_dir"),
            tr!("settings.ai_models.models_dir.desc"),
            Box::new(control),
        )],
    )
}

fn components_card(q: &SynChatQuantConfig) -> Box<dyn Widget> {
    let mut rows: Vec<Box<dyn Widget>> = Vec::with_capacity(7);

    // 1. Weights storage
    rows.push({
        let control = storage_dtype_dropdown(q.weights_storage.clone(), |s| {
            update_quant(|cfg| cfg.weights_storage = s);
        });
        row_frame(
            MI_MEMORY,
            tr!("settings.ai_models.weights_storage"),
            tr!("settings.ai_models.weights_storage.desc"),
            control,
        )
    });

    // 2. Compute
    rows.push({
        let control = compute_dtype_dropdown(q.compute.clone(), |s| {
            update_quant(|cfg| cfg.compute = s);
        });
        row_frame(
            MI_BOLT,
            tr!("settings.ai_models.compute_dtype"),
            tr!("settings.ai_models.compute_dtype.desc"),
            control,
        )
    });

    // 3. KV-cache
    rows.push({
        let control = kv_dtype_dropdown(q.kv_dtype.clone(), |s| {
            update_quant(|cfg| cfg.kv_dtype = s);
        });
        row_frame(
            MI_DATA_OBJECT,
            tr!("settings.ai_models.kv_cache"),
            tr!("settings.ai_models.kv_cache.desc"),
            control,
        )
    });

    // 4. lm_head
    rows.push({
        let control = storage_dtype_dropdown(q.lm_head_storage.clone(), |s| {
            update_quant(|cfg| cfg.lm_head_storage = s);
        });
        row_frame(
            MI_ARTICLE,
            tr!("settings.ai_models.lm_head"),
            tr!("settings.ai_models.lm_head.desc"),
            control,
        )
    });

    // 5. Embed tokens — Phase F (NVFP4 gather) / FP8 gather
    rows.push({
        let control = storage_dtype_dropdown(q.embed_storage.clone(), |s| {
            update_quant(|cfg| cfg.embed_storage = s);
        });
        row_frame(
            MI_ARTICLE,
            tr!("settings.ai_models.embed_tokens"),
            tr!("settings.ai_models.embed_tokens.desc"),
            control,
        )
    });

    // 6. Tied embeddings (Phase F)
    rows.push({
        let control = tied_embeddings_dropdown(q.tied_embeddings.clone(), |s| {
            update_quant(|cfg| cfg.tied_embeddings = s);
        });
        row_frame(
            MI_HUB,
            tr!("settings.ai_models.tied_embeddings"),
            tr!("settings.ai_models.tied_embeddings.desc"),
            control,
        )
    });

    // 6. SSM state
    rows.push({
        let control = kv_dtype_dropdown(q.ssm_state_dtype.clone(), |s| {
            update_quant(|cfg| cfg.ssm_state_dtype = s);
        });
        row_frame(
            MI_MEMORY,
            tr!("settings.ai_models.ssm_state"),
            tr!("settings.ai_models.ssm_state.desc"),
            control,
        )
    });

    // 7. Conv state
    rows.push({
        let control = kv_dtype_dropdown(q.conv_state_dtype.clone(), |s| {
            update_quant(|cfg| cfg.conv_state_dtype = s);
        });
        row_frame(
            MI_TUNE,
            tr!("settings.ai_models.conv_state"),
            tr!("settings.ai_models.conv_state.desc"),
            control,
        )
    });

    section_card(tr!("settings.ai_models.section.components"), rows)
}

fn apply_card() -> Box<dyn Widget> {
    let apply_control: Box<dyn Widget> = Box::new(
        Button::new(tr!("settings.ai_models.apply.button"))
            .icon(MI_AUTORENEW)
            .on_click(|| {
                let ctx = use_context::<AppCtx>();
                let policy = ctx.syn_chat_quant.get_untracked().to_policy();
                let cfg = crate::config::AppConfig::load();
                let Some(path_str) = cfg.last_syn_model.clone() else {
                    ctx.notifications.warning(
                        tr!("settings.ai_models.apply.no_bundle"),
                    );
                    return;
                };
                let path = PathBuf::from(path_str);
                if !path.exists() {
                    ctx.notifications
                        .warning(tr!("settings.ai_models.apply.not_found", path = path.display()));
                    return;
                }
                let registry = use_context::<SynModelRegistry>();
                registry.unload();
                registry.load(path, policy);
                ctx.notifications
                    .info(tr!("settings.ai_models.apply.reloading"));
            })
            .class("ai-models-apply-button"),
    );

    section_card(
        tr!("settings.ai_models.section.apply"),
        vec![row_frame(
            MI_PLAY_ARROW,
            tr!("settings.ai_models.apply"),
            tr!("settings.ai_models.apply.desc"),
            apply_control,
        )],
    )
}

/// Helper для атомарного апдейта `AppCtx.syn_chat_quant` + auto-detect
/// преcета. Если новые dtype-поля совпадают с одним из встроенных пресетов
/// — `preset` устанавливается соответствующим именем, иначе `"custom"`.
fn update_quant<F: FnOnce(&mut SynChatQuantConfig)>(f: F) {
    let ctx = use_context::<AppCtx>();
    ctx.syn_chat_quant.update(|cfg| {
        f(cfg);
        // Auto-detect preset через сравнение с известными вариантами.
        let detected = cfg.to_policy().detect_preset();
        cfg.preset = detected.to_string();
    });
}

/// Card «Пути к bundle'ам ACE-Step». Два file picker'а:
/// 1. xl-bundle (`acestep_v15_xl_base.syn` / `_turbo.syn`) — DiT + projector +
///    lyric/timbre encoders + null_cond + FSQ + Detokenizer.
/// 2. vae-bundle (`acestep_vae.syn`).
///
/// Эти пути используются всеми ACE-Step нодами семейства через
/// `nodes::acestep::shared::xl_bundle_path() / vae_bundle_path()`. Они
/// глобальные, потому что внутри одного workflow обычно используется один
/// набор весов на много нод (TextEncoder + LyricEncoder + TimbreEncoder +
/// Sampler тянут общий xl-bundle). На уровне ноды остаётся только Qwen-LLM
/// path (Qwen 0.6B для Text/Lyric или Qwen 4B AR для ArLm).
fn acestep_paths_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let xl_sig = ctx.acestep_xl_bundle_path;
    let vae_sig = ctx.acestep_vae_bundle_path;
    section_card(
        tr!("settings.ai_models.section.acestep"),
        vec![
            row_frame(
                MI_FOLDER_OPEN,
                tr!("settings.ai_models.acestep_xl"),
                tr!("settings.ai_models.acestep_xl.desc"),
                bundle_picker(xl_sig, tr!("settings.ai_models.acestep_xl.dialog_title")),
            ),
            row_frame(
                MI_FOLDER_OPEN,
                tr!("settings.ai_models.acestep_vae"),
                tr!("settings.ai_models.acestep_vae.desc"),
                bundle_picker(vae_sig, tr!("settings.ai_models.acestep_vae.dialog_title")),
            ),
        ],
    )
}

/// File picker для `.syn` bundle'а с привязкой к `RwSignal<Option<String>>`.
/// Показывает имя файла справа от кнопки (или «Не выбран»).
fn bundle_picker(sig: RwSignal<Option<String>>, title: impl Into<String>) -> Box<dyn Widget> {
    let title = title.into();
    let pick_btn: Box<dyn Widget> = Box::new(
        Button::new(tr!("app.browse"))
            .icon(MI_FOLDER_OPEN)
            .on_click(move || {
                let dlg = rfd::FileDialog::new()
                    .add_filter(tr!("settings.ai_models.bundle.filter"), &["syn"])
                    .set_title(title.clone());
                if let Some(p) = dlg.pick_file() {
                    sig.set(Some(p.to_string_lossy().to_string()));
                }
            })
            .class("ai-models-apply-button"),
    );
    let clear_btn: Box<dyn Widget> = Box::new(
        Button::new(tr!("settings.ai_models.bundle.clear"))
            .on_click(move || sig.set(None))
            .class("ai-models-apply-button"),
    );
    let filename = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let label: String = sig
            .get()
            .map(|s| {
                std::path::Path::new(&s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(s)
            })
            .unwrap_or_else(|| tr!("settings.ai_models.bundle.none"));
        vec![Box::new(Text::new(label).class("settings-row-desc")) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![pick_btn, clear_btn, Box::new(filename)]),
    )
}
