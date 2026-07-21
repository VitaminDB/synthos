//! Portal-диалог выбора cache-dir перед первым скачиванием.

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use rfd::AsyncFileDialog;
use std::path::PathBuf;

use crate::config;
use crate::context::AppCtx;
use crate::icons::{MI_CHECK, MI_CLOSE, MI_FOLDER_OPEN};

use super::download;
use super::state::HuggingFaceCtx;

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<HuggingFaceCtx>();
        let has = ctx.cache_dir_dialog_open.get();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            if !ctx.cache_dir_dialog_open.get() {
                return vec![Box::new(DecoratedBox::new().class("hf-dialog-empty"))];
            }
            vec![Box::new(card())]
        }))
}

fn card() -> impl Widget {
    let default_path = config::resolve_hf_cache_dir("").display().to_string();
    let hint = format!(
        "Будут сохраняться в {}. Можно выбрать другую папку.",
        default_path
    );

    mgui! {
        DecoratedBox::new().class("hf-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new("Куда сохранять модели HuggingFace?")
                        .class("hf-dialog-title"),
                    Text::new(hint).class("hf-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new("Отмена")
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("hf-dialog-btn-secondary"),
                            Button::new("Выбрать папку…")
                                .leading_icon(MI_FOLDER_OPEN)
                                .on_click(pick_folder)
                                .class("hf-dialog-btn-secondary"),
                            Button::new("По умолчанию")
                                .leading_icon(MI_CHECK)
                                .on_click(accept_default)
                                .class("hf-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn cancel() {
    let ctx = use_context::<HuggingFaceCtx>();
    ctx.cache_dir_dialog_open.set(false);
    ctx.pending_download.set(Vec::new());
}

fn accept_default() {
    let ctx = use_context::<HuggingFaceCtx>();
    let app = use_context::<AppCtx>();
    let default_path = config::resolve_hf_cache_dir("").display().to_string();
    ctx.cache_dir.set(default_path);
    ctx.cache_dir_dialog_open.set(false);
    download::resume_pending(ctx, app.notifications.clone());
}

fn pick_folder() {
    let ctx = use_context::<HuggingFaceCtx>();
    let app = use_context::<AppCtx>();
    let notifications = app.notifications.clone();
    spawn(async move {
        let result = AsyncFileDialog::new()
            .set_title("Папка для моделей HuggingFace")
            .pick_folder()
            .await;
        let Some(folder) = result else { return };
        let path: PathBuf = folder.path().to_path_buf();
        let path_str = path.display().to_string();
        run_on_main_thread(move || {
            ctx.cache_dir.set(path_str);
            ctx.cache_dir_dialog_open.set(false);
            download::resume_pending(ctx, notifications);
        });
    });
}
