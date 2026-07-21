//! Страница «Настройки» — трёхколонная композиция.
//!
//! * Левая колонка — `sidebar` с пунктами Общие / Темы / Скилы.
//! * Центральная колонка — вложенный `RouterView` по трём подмаршрутам.
//! * Правая колонка — на вкладке Скилы отрисовывает список скилов,
//!   на остальных показывает центрированную подсказку-заглушку.

pub mod about;
pub mod ai_models;
pub mod audio_models;
pub mod general;
pub mod knowledge_base;
pub mod models;
pub mod right_hint;
pub mod sidebar;
pub mod skills;
pub mod skills_dialog;
pub mod skills_panel;
pub mod terminal;
pub mod theme_data;
pub mod themes;
pub mod widgets;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::navigation::router::RouterView;

use crate::context::AppCtx;

pub fn view() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let settings_router = ctx.settings_router.clone();

    let content = RouterView::new(settings_router)
        .route("general",        || Box::new(general::view()))
        .route("themes",         || Box::new(themes::view()))
        .route("skills",         || Box::new(skills::view()))
        .route("models",         || Box::new(models::view()))
        .route("audio_models",   || Box::new(audio_models::view()))
        .route("ai_models",      || Box::new(ai_models::view()))
        .route("knowledge_base", || Box::new(knowledge_base::view()))
        .route("terminal",       || Box::new(terminal::view()))
        .route("about",          || Box::new(about::view()));

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            DecoratedBox::new().class("settings-shell").child(mgui! {
                Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    sidebar::view(),
                    DecoratedBox::new().class("grow").child(content),
                    right_panel(),
                ]
            }),
            // Portal CRUD-диалогов скилов: создание / переименование / удаление.
            // Один инстанс, реактивно слушает `AppCtx.skills_dialog`.
            skills_dialog::view(),
            // Portal URL prompt для добавления URL-источника в KB-коллекцию.
            // Слушает `AppCtx.kb_url_dialog`.
            knowledge_base::url_dialog::view(),
        ]
    }
}

/// Правая колонка — переключается между списком скилов и подсказкой.
/// Stack::Expand заставляет ребёнка занять всю высоту 300px-колонки,
/// избегая MSS-костыля `flex-grow` (движок его не поддерживает).
fn right_panel() -> impl Widget {
    DecoratedBox::new().class("settings-right").child(move || {
        let ctx = use_context::<AppCtx>();
        let child: Box<dyn Widget> = match ctx.selected_settings_tab.get().as_str() {
            "skills" => Box::new(skills_panel::view()),
            "models" => Box::new(models::models_panel::view()),
            "audio_models" => Box::new(audio_models::audio_models_panel::view()),
            "knowledge_base" => Box::new(knowledge_base::collections_panel::view()),
            _ => Box::new(right_hint::view()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}
