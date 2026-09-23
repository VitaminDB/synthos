//! Секция «Модели»: эмбеддер и реранкер — общие на все коллекции.
//!
//! Файлы ищет `kb::models` в каталоге из «AI модели»; строка показывает, что
//! нашлось (или где ждали), и что сейчас в памяти. Грузить руками не
//! обязательно — индексация и поиск поднимут модель сами; кнопки нужны, чтобы
//! прогреть заранее или освободить память.

use std::path::Path;

use rfd::AsyncFileDialog;
use syngui::prelude::*;
use syngui::widgets::feedback::tooltip::Tooltip;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};

use crate::config::{AppConfig, KbConfig};
use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::ctx::ModelState;
use crate::kb::loader;
use crate::kb::models::{FoundBy, FoundModel, ModelKind};

pub(super) fn view() -> Box<dyn Widget> {
    // Снимок KbConfig на время жизни карточки: правки пишутся и на диск, и
    // сюда — строки перерисовываются без чтения config.json на каждый кадр.
    let cfg = use_signal(AppConfig::load().kb);

    let body = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Reactive::new(move || vec![model_row(ModelKind::Embedder, cfg)]))
        .child(Reactive::new(move || vec![model_row(ModelKind::Reranker, cfg)]))
        .child(device_row(cfg));
    super::section(tr!("settings.knowledge_base.models"), None, Box::new(body))
}

fn model_row(kind: ModelKind, cfg: RwSignal<KbConfig>) -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let paths = ctx.kb.model_paths.get();
    let searched = ctx.kb.model_paths_ready.get();
    let state = ctx.kb.model_state(kind).get();
    let found = paths.get(kind).cloned();
    let enabled = kind == ModelKind::Embedder || cfg.get().reranker_enabled;

    let (icon, title) = match kind {
        ModelKind::Embedder => (MI_HUB, tr!("settings.knowledge_base.models.embedder")),
        ModelKind::Reranker => (MI_SWAP_VERT, tr!("settings.knowledge_base.models.reranker")),
    };

    // Вторая строка: что за файл / где искали / что сломалось.
    let (desc, desc_class) = match (&state, &found) {
        (ModelState::Failed(e), _) => (e.clone(), "settings-row-desc kb-desc-error"),
        (_, Some(f)) => (describe(f), "settings-row-desc"),
        (_, None) if !searched => (
            tr!("settings.knowledge_base.models.searching"),
            "settings-row-desc",
        ),
        (_, None) => (
            tr!(
                "settings.knowledge_base.models.not_found",
                file = format!("{}.syn", kind.stem()),
                dir = paths
                    .searched
                    .first()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
                repo = kind.hf_repo()
            ),
            "settings-row-desc kb-desc-warn",
        ),
    };
    let purpose = match kind {
        ModelKind::Embedder => tr!("settings.knowledge_base.models.embedder.tip"),
        ModelKind::Reranker => tr!("settings.knowledge_base.models.reranker.tip"),
    };

    let mut controls: Vec<Box<dyn Widget>> = Vec::new();
    if !enabled {
        controls.push(chip(tr!("settings.knowledge_base.models.state.off"), "kb-state-chip"));
    } else {
        match (&state, &found) {
            (ModelState::Loading, _) => {
                controls.push(Box::new(CircularProgress::new().indeterminate().size(16.0)));
                controls.push(chip(
                    tr!("settings.knowledge_base.models.state.loading"),
                    "kb-state-chip busy",
                ));
            }
            (ModelState::Ready, _) => {
                controls.push(chip(
                    tr!("settings.knowledge_base.models.state.ready"),
                    "kb-state-chip ready",
                ));
                controls.push(Box::new(
                    Button::new(tr!("settings.knowledge_base.models.unload"))
                        .on_click(move || loader::unload(&use_context::<AppCtx>().kb, kind))
                        .class("kb-btn"),
                ));
            }
            (_, None) if searched => controls.push(Box::new(
                Button::new(tr!("settings.knowledge_base.models.pick"))
                    .leading_icon(MI_FOLDER_OPEN)
                    .on_click(move || pick_model(kind, cfg))
                    .class("kb-btn"),
            )),
            (_, None) => {}
            (failed_or_idle, Some(_)) => {
                let label = if matches!(failed_or_idle, ModelState::Failed(_)) {
                    tr!("settings.knowledge_base.models.retry")
                } else {
                    tr!("settings.knowledge_base.models.load")
                };
                controls.push(Box::new(
                    Button::new(label).on_click(move || load(kind)).class("kb-btn kb-btn-soft"),
                ));
            }
        }
    }
    if kind == ModelKind::Reranker {
        controls.push(Box::new(Toggle::with_state(enabled).on_change(move |on| {
            update_cfg(cfg, |c| c.reranker_enabled = on);
            if !on {
                loader::unload(&use_context::<AppCtx>().kb, ModelKind::Reranker);
            }
        })));
    }

    let head = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![super::row_icon(icon)])
        .child(
            DecoratedBox::new().class("grow kb-min0").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Tooltip::new(Text::new(title).class("settings-row-title"), purpose))
                    .child(Text::new(desc).elide(Elide::Middle).class(desc_class)),
            ),
        )
        .children(controls);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 16.0).child(head)),
    )
}

/// «bge-m3.syn · 2,1 ГБ · ~/Storage/syn_models» (+ «указан вручную»).
fn describe(found: &FoundModel) -> String {
    let name = found
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = found.path.parent().map(short_home).unwrap_or_default();
    let mut parts = vec![name];
    if found.bytes > 0 {
        parts.push(crate::models::human_bytes(found.bytes));
    }
    parts.push(dir);
    if found.by == FoundBy::Config {
        parts.push(tr!("settings.knowledge_base.models.manual"));
    }
    parts.join(" · ")
}

fn short_home(path: &Path) -> String {
    let full = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && full.starts_with(&home) => format!("~{}", &full[home.len()..]),
        _ => full,
    }
}

fn chip(text: String, class: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class(class.to_string())
            .child(Text::new(text).class("kb-state-chip-text")),
    )
}

fn device_row(cfg: RwSignal<KbConfig>) -> Box<dyn Widget> {
    let current = match cfg.get_untracked().embedder_device.as_str() {
        "cuda" => "cuda",
        _ => "cpu",
    };
    let control = Dropdown::with_items(vec![
        DropdownItem::new("cpu", tr!("settings.knowledge_base.models.device.cpu")),
        DropdownItem::new("cuda", tr!("settings.knowledge_base.models.device.gpu")),
    ])
    .selected(current.to_string())
    .on_change(move |value| {
        let device = value.to_string();
        if cfg.get_untracked().embedder_device == device {
            return;
        }
        // На GPU считаем в F16 (вдвое меньше VRAM, точность та же по тестам),
        // на CPU F16-ядра медленнее F32.
        let dtype = if device == "cuda" { "f16" } else { "f32" };
        update_cfg(cfg, |c| {
            c.embedder_device = device.clone();
            c.reranker_device = device.clone();
            c.embedder_dtype = dtype.into();
            c.reranker_dtype = dtype.into();
        });
        // Загруженные модели сидят на старом устройстве — следующая
        // надобность поднимет их на новом.
        let kb = use_context::<AppCtx>().kb;
        loader::unload(&kb, ModelKind::Embedder);
        loader::unload(&kb, ModelKind::Reranker);
    })
    .class("settings-row-dropdown");

    crate::pages::settings::widgets::row_frame(
        MI_DEVELOPER_BOARD,
        tr!("settings.knowledge_base.models.device"),
        tr!("settings.knowledge_base.models.device.desc"),
        Box::new(control),
    )
}

fn load(kind: ModelKind) {
    let app = use_context::<AppCtx>();
    let plan = loader::plan();
    match kind {
        ModelKind::Embedder => loader::ensure_loaded(app.kb.clone(), app.notifications.clone(), plan),
        ModelKind::Reranker => {
            loader::ensure_reranker_loaded(app.kb.clone(), app.notifications.clone(), plan)
        }
    }
}

/// Записать правку KbConfig на диск и в снимок карточки.
fn update_cfg(cfg: RwSignal<KbConfig>, edit: impl FnOnce(&mut KbConfig)) {
    let app_cfg = AppConfig::update(|c| edit(&mut c.kb));
    cfg.set(app_cfg.kb);
}

/// Указать файл модели вручную — когда бандл лежит вне каталогов поиска.
fn pick_model(kind: ModelKind, cfg: RwSignal<KbConfig>) {
    syngui::async_runtime::spawn(async move {
        let picked = AsyncFileDialog::new()
            .add_filter(tr!("settings.knowledge_base.models.pick.filter"), &["syn"])
            .set_title(tr!("settings.knowledge_base.models.pick.title"))
            .pick_file()
            .await;
        let Some(file) = picked else { return };
        let path = file.path().display().to_string();
        syngui::async_runtime::run_on_main_thread(move || {
            update_cfg(cfg, |c| match kind {
                ModelKind::Embedder => c.embedder_model_path = path,
                ModelKind::Reranker => c.reranker_model_path = path,
            });
            loader::refresh_paths(use_context::<AppCtx>().kb.clone());
        });
    });
}
