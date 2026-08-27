//! Страница «Настройки» — трёхпанельный каркас под общей шапкой.
//!
//! * Левая панель — `sidebar` с разделами.
//! * Центр — вложенный `RouterView` по подмаршрутам.
//! * Правая панель — на вкладке Скилы отрисовывает список скилов, на
//!   моделях/базах знаний — их панели, на остальных — подсказку-заглушку.

pub mod about;
pub mod ai_models;
pub mod archive;
pub mod audio_models;
pub mod general;
pub mod knowledge_base;
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

use crate::components::page_header::{self, HeaderSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::MI_SETTINGS;

pub fn view() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let (left_visible, right_visible) = ctx.panels.settings;

    let identity: Box<dyn Widget> = Box::new(DecoratedBox::new().child(|| {
        let ctx = use_context::<AppCtx>();
        let key = ctx.selected_settings_tab.get();
        expand(page_header::identity_text(
            MI_SETTINGS,
            tr!("settings.title"),
            tr!(&format!("settings.tabs.{key}.title")),
        ))
    }));
    let header = page_header::view(
        HeaderSpec::new(identity).toggles(Some(left_visible), Some(right_visible)),
    );

    let spec = FrameSpec::new("settings-h-split", || {
        let ctx = use_context::<AppCtx>();
        let content = RouterView::new(ctx.settings_router.clone())
            .route("general",        || Box::new(general::view()))
            .route("themes",         || Box::new(themes::view()))
            .route("skills",         || Box::new(skills::view()))
            .route("audio_models",   || Box::new(audio_models::view()))
            .route("ai_models",      || Box::new(ai_models::view()))
            .route("knowledge_base", || Box::new(knowledge_base::view()))
            .route("terminal",       || Box::new(terminal::view()))
            .route("archive",        || Box::new(archive::view()))
            .route("about",          || Box::new(about::view()));
        Box::new(DecoratedBox::new().class("settings-content").child(content))
    })
    .left(Pane::new(left_visible, ctx.settings_left_split_ratio, 200.0, || {
        Box::new(sidebar::view())
    }))
    .right(Pane::new(right_visible, ctx.settings_right_split_ratio, 240.0, || {
        Box::new(right_panel())
    }));

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            DecoratedBox::new().class("settings-shell").child(workspace_frame::view(header, spec)),
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
/// Stack::Expand заставляет ребёнка занять всю высоту колонки,
/// избегая MSS-костыля `flex-grow` (движок его не поддерживает).
fn right_panel() -> impl Widget {
    DecoratedBox::new().class("settings-right").child(move || {
        let ctx = use_context::<AppCtx>();
        let child: Box<dyn Widget> = match ctx.selected_settings_tab.get().as_str() {
            "skills" => Box::new(skills_panel::view()),
            "audio_models" => Box::new(audio_models::audio_models_panel::view()),
            "knowledge_base" => Box::new(knowledge_base::collections_panel::view()),
            _ => Box::new(right_hint::view()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}
