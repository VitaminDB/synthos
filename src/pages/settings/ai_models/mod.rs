//! Settings → AI Models — режим работы каждой модели.
//!
//! У модели два режима. `Optimal` — по умолчанию: настройки берёт движок,
//! он знает, какие пути у какой архитектуры выверены замерами. `Custom` —
//! те же настройки, но поверх ложатся ручные правки; тронутое перебивает
//! выверенное, остальное остаётся как в `Optimal`.
//!
//! Глобальных тумблеров тут больше нет: ровно они и мешали — настройка,
//! хорошая для одной архитектуры, применялась ко всем.

use std::path::PathBuf;

use syngui::prelude::*;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};
use syngui::widgets::input::toggle::Toggle;

use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::settings::widgets::{
    compute_dtype_dropdown, kv_dtype_dropdown, row_tip, section_card, storage_dtype_dropdown,
};
use crate::syn_chat::model_registry::ServiceLlm;
use crate::syn_chat::SynModelRegistry;

pub fn view() -> impl Widget {
    ScrollView::new().vertical().child(move || {
        let header = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Text::new(tr!("settings.ai_models.title")).class("settings-section-title"))
            .child(Text::new(tr!("settings.ai_models.subtitle")).class("settings-row-desc"));

        let body = Column::new()
            .gap(24.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(header) as Box<dyn Widget>,
                models_dir_card(),
                models_card(),
                chat_card(),
            ]);

        Padding::all(32.0).child(
            DecoratedBox::new()
                .class("settings-page models-page ai-models-page")
                .child(body),
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Список моделей — языковые модели службы нейросетей
// ─────────────────────────────────────────────────────────────────────────────

fn models_card() -> Box<dyn Widget> {
    let reg = use_context::<SynModelRegistry>();

    // Строки уходят одной колонкой, а не списком виджетов: `Reactive` кладёт
    // всех своих детей в одну точку — несколько моделей нарисовались бы
    // друг поверх друга.
    let rows = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let models = reg.models.get();
        if models.is_empty() {
            let (title, tip) = if reg.online.get() {
                (tr!("settings.ai_models.models.empty"), tr!("settings.ai_models.models.empty.tip"))
            } else {
                (tr!("chat.right.model.service.offline"), tr!("settings.ai_models.service.offline.tip"))
            };
            return vec![row_tip(MI_FOLDER_OPEN, title, tip, storage_button())];
        }
        vec![Box::new(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(models.iter().map(model_block)),
        ) as Box<dyn Widget>]
    });

    section_card(tr!("settings.ai_models.section.models"), vec![Box::new(rows)])
}

/// Хранилище моделей службы — в «Параметрах» оболочки.
fn storage_button() -> Box<dyn Widget> {
    Box::new(
        Button::new(tr!("settings.ai_models.storage.button"))
            .icon(MI_FOLDER_OPEN)
            .on_click(|| {
                if let Err(e) = std::process::Command::new("synsettings").arg("models/storage").spawn() {
                    log::warn!("synsettings: {e}");
                }
            })
            .class("ai-models-apply-button"),
    )
}

/// Строка модели плюс — в режиме «свой» — блок переопределений под ней.
fn model_block(m: &ServiceLlm) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("optimal", tr!("settings.ai_models.mode.optimal")),
        DropdownItem::new("custom", tr!("settings.ai_models.mode.custom")),
    ];
    let id = m.id.clone();
    let mode: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(if m.own.custom { "custom" } else { "optimal" })
            .on_change(move |s| {
                let custom = s == "custom";
                update_engine(&id, move |p| p.custom = custom);
            })
            .class("models-active-dropdown"),
    );

    let load_path = m.path.clone();
    let load: Box<dyn Widget> = Box::new(
        Button::new(tr!("settings.ai_models.apply.button"))
            .icon(MI_AUTORENEW)
            .on_click(move || load_model(load_path.clone()))
            .class("ai-models-apply-button"),
    );

    let control = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![mode, load]);

    let mut subtitle = vec![crate::models::human_bytes(m.disk_bytes)];
    if let Some(f) = m.engine.as_ref().and_then(|e| e.source.first()) {
        subtitle.push(f.trim_start_matches("ggml:").to_uppercase());
    }
    if m.loaded {
        subtitle.push(tr!("chat.right.model.service.loaded"));
    }

    let head = row_tip(
        MI_MEMORY,
        format!("{} · {}", m.title, subtitle.join(" · ")),
        tr!("settings.ai_models.model.tip", path = m.path.display()),
        Box::new(control),
    );

    let Some(shown) = m.engine.clone().filter(|_| m.own.custom) else {
        return head;
    };

    let mut rows: Vec<Box<dyn Widget>> = vec![head];
    rows.extend(overrides(&m.id, &shown));
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

/// Переопределения одной модели. Значение в контроле — действующее (своё,
/// если задано, иначе выверенное движком): видно, от чего отталкиваешься.
fn overrides(id: &str, e: &synaptixd::EngineProfile) -> Vec<Box<dyn Widget>> {
    let id = id.to_string();
    let text = |v: &Option<String>| v.clone().unwrap_or_default();
    let storage = |cur: String, set: fn(&mut synaptixd::EngineProfile, String)| {
        let id = id.clone();
        storage_dtype_dropdown(cur, move |s| {
            let s = s.to_string();
            update_engine(&id, move |p| set(p, s));
        })
    };
    let weights = storage(text(&e.weights), |p, s| p.weights = Some(s));
    let lm_head = storage(text(&e.lm_head), |p, s| p.lm_head = Some(s));
    let embed = storage(text(&e.embed), |p, s| p.embed = Some(s));
    let compute = {
        let id = id.clone();
        compute_dtype_dropdown(text(&e.compute), move |s| {
            let s = s.to_string();
            update_engine(&id, move |p| p.compute = Some(s));
        })
    };
    let kv = {
        let id = id.clone();
        kv_dtype_dropdown(text(&e.kv_dtype), move |s| {
            let s = s.to_string();
            update_engine(&id, move |p| p.kv_dtype = Some(s));
        })
    };
    let toggle = |on: bool, set: fn(&mut synaptixd::EngineProfile, bool)| {
        let id = id.clone();
        Box::new(Toggle::with_state(on).on_change(move |v| update_engine(&id, move |p| set(p, v)))) as Box<dyn Widget>
    };
    let graph = toggle(e.graph_decode == Some(true), |p, v| p.graph_decode = Some(v));
    let spec = toggle(e.speculation == Some(true), |p, v| p.speculation = Some(v));
    let transcode = toggle(e.transcode == Some(true), |p, v| p.transcode = Some(v));
    let sync = {
        let id = id.clone();
        let items = vec![
            DropdownItem::new("auto", tr!("settings.ai_models.layer_sync.auto")),
            DropdownItem::new("on", tr!("settings.ai_models.layer_sync.on")),
            DropdownItem::new("off", tr!("settings.ai_models.layer_sync.off")),
        ];
        Box::new(
            Dropdown::with_items(items)
                .selected(e.layer_sync.clone().unwrap_or_else(|| "auto".into()))
                .on_change(move |s| {
                    let v = s.to_string();
                    update_engine(&id, move |p| p.layer_sync = Some(v));
                })
                .class("models-active-dropdown"),
        ) as Box<dyn Widget>
    };
    let transcode_desc = if e.source.is_empty() {
        tr!("settings.ai_models.transcode.desc_dense")
    } else {
        tr!("settings.ai_models.transcode.desc", formats = e.source.join(", ").to_uppercase())
    };
    let reset: Box<dyn Widget> = {
        let id = id.clone();
        Box::new(
            Button::new(tr!("settings.ai_models.reset.button"))
                .on_click(move || update_engine(&id, |p| *p = synaptixd::EngineProfile { custom: true, ..Default::default() }))
                .class("ai-models-apply-button"),
        )
    };

    vec![
        row_tip(MI_TUNE, tr!("settings.ai_models.weights"), tr!("settings.ai_models.weights.desc"), weights),
        row_tip(MI_SPEED, tr!("settings.ai_models.compute"), tr!("settings.ai_models.compute.desc"), compute),
        row_tip(MI_MEMORY, tr!("settings.ai_models.kv"), tr!("settings.ai_models.kv.desc"), kv),
        row_tip(MI_TUNE, tr!("settings.ai_models.lm_head"), tr!("settings.ai_models.lm_head.desc"), lm_head),
        row_tip(MI_TUNE, tr!("settings.ai_models.embed"), tr!("settings.ai_models.embed.desc"), embed),
        row_tip(MI_AUTORENEW, tr!("settings.ai_models.transcode"), transcode_desc, transcode),
        row_tip(MI_AUTO_AWESOME, tr!("settings.ai_models.cuda_graph"), tr!("settings.ai_models.cuda_graph.desc"), graph),
        row_tip(MI_BOLT, tr!("settings.ai_models.speculation"), tr!("settings.ai_models.speculation.desc"), spec),
        row_tip(MI_MEMORY, tr!("settings.ai_models.layer_sync"), tr!("settings.ai_models.layer_sync.desc"), sync),
        row_tip(MI_AUTORENEW, tr!("settings.ai_models.reset"), tr!("settings.ai_models.reset.desc"), reset),
    ]
}

/// Правка профиля модели в службе (её настройки — общие для всех программ).
/// Смена весов, KV или спекуляции служба применит перезагрузкой модели.
fn update_engine(id: &str, f: impl FnOnce(&mut synaptixd::EngineProfile) + Send + 'static) {
    let reg = use_context::<SynModelRegistry>();
    let Some(m) = reg.models.get_untracked().into_iter().find(|m| m.id == id) else { return };
    let ctx = use_context::<AppCtx>();
    let notes = ctx.notifications;
    let mut e = m.own.clone();
    f(&mut e);
    let id = id.to_string();
    std::thread::spawn(move || {
        let r = synaptixd::Client::connect().and_then(|mut c| c.set(&id, synaptixd::SettingsChange { engine: Some(e), ..Default::default() }));
        if let Err(err) = r {
            let msg = if err.code == synaptixd::ErrorCode::Denied { tr!("settings.ai_models.service.denied") } else { err.to_string() };
            syngui::async_runtime::run_on_main_thread(move || notes.warning(msg));
        }
    });
}

/// Выбрать модель для чата (служба загрузит её со своим профилем).
fn load_model(path: PathBuf) {
    let ctx = use_context::<AppCtx>();
    let registry = use_context::<SynModelRegistry>();
    registry.unload();
    registry.load(path, synaptix::facade::llm::QuantPolicy::quality());
    ctx.notifications.info(tr!("settings.ai_models.apply.reloading"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Прочие карточки
// ─────────────────────────────────────────────────────────────────────────────

/// Каталог моделей — общая папка с бандлами и HF-каталогами для всех
/// пайплайнов. Отсюда же берётся список выше.
fn models_dir_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let sig = ctx.models_dir;

    // Перестраивается только по выбору через «Обзор» — см. `picked`.
    let picked = use_signal(0u64);
    let field = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        picked.get();
        vec![Box::new(
            syngui::widgets::input::TextField::with_text(sig.get_untracked())
                .placeholder(crate::config::default_models_dir())
                .on_change(move |s| sig.set(s.to_string()))
                .class("models-path-field"),
        )]
    });

    let browse = Button::new(tr!("app.browse"))
        .icon(MI_FOLDER_OPEN)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(tr!("settings.ai_models.section.models_dir"));
            if let Some(p) = dlg.pick_folder() {
                sig.set(p.display().to_string());
                picked.update(|n| *n += 1);
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
        vec![row_tip(
            MI_FOLDER_OPEN,
            tr!("settings.ai_models.models_dir"),
            tr!("settings.ai_models.models_dir.desc"),
            Box::new(control),
        )],
    )
}

/// Настройки чата, а не движка: потолок vision-токенов меняет длину промпта,
/// а не путь исполнения, поэтому живёт отдельно от режимов модели.
fn chat_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let img_signal = ctx.syn_chat_max_image_tokens;
    let items = vec![
        DropdownItem::new("256", tr!("settings.ai_models.image_tokens.256")),
        DropdownItem::new("512", tr!("settings.ai_models.image_tokens.512")),
        DropdownItem::new("1024", tr!("settings.ai_models.image_tokens.1024")),
        DropdownItem::new("2048", tr!("settings.ai_models.image_tokens.2048")),
        DropdownItem::new("4096", tr!("settings.ai_models.image_tokens.4096")),
        DropdownItem::new("0", tr!("settings.ai_models.image_tokens.unlimited")),
    ];
    let dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(img_signal.get_untracked().to_string())
            .on_change(move |s| {
                if let Ok(n) = s.parse::<usize>() {
                    img_signal.set(n);
                }
            })
            .class("models-active-dropdown"),
    );

    section_card(
        tr!("settings.ai_models.section.chat"),
        vec![row_tip(
            MI_IMAGE_ICON,
            tr!("settings.ai_models.image_tokens"),
            tr!("settings.ai_models.image_tokens.desc"),
            dropdown,
        )],
    )
}

