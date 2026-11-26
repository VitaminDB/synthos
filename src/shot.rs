//! `synthos --screenshot <png> [--page <страница>[/<раздел>]]` — снимок окна
//! приложения в PNG без показа окна (`syngui::shot`): шапка, рейл и
//! страница — чат, редактор кода, обучение или настройки.
//!
//! Снимок только читает: конфиг, список чатов и сессии редактора берутся с
//! диска, но автосохранение не ставится, файлы чатов не дополняются и
//! открытые чаты не выбираются (выбор переписал бы их файлы) — так снимок
//! можно делать рядом с работающим synthos.

use syngui::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub out: String,
    /// Маршрут верхнего уровня (`context::ROUTES`), для настроек — с
    /// разделом через `/`: `settings/themes`.
    pub page: String,
    pub size: (u32, u32),
    pub scale: f64,
}

pub fn run(shot: &Shot) -> std::result::Result<(u32, u32), String> {
    let (route, section) = match shot.page.split_once('/') {
        Some((r, s)) => (r.to_string(), Some(s.to_string())),
        None => (shot.page.clone(), None),
    };
    if !crate::context::ROUTES.contains(&route.as_str()) {
        return Err(format!("нет страницы {route} (есть: {})", crate::context::ROUTES.join(", ")));
    }
    if let Some(s) = &section {
        if route != "settings" || !crate::context::SETTINGS_ROUTES.contains(&s.as_str()) {
            return Err(format!(
                "нет раздела {s} (разделы настроек: {})",
                crate::context::SETTINGS_ROUTES.join(", ")
            ));
        }
    }
    let (theme_mss, _) = crate::build_context();
    let mss = format!("{}\n{}", crate::styles::styles(), theme_mss.get_untracked());
    syngui::shot::render_png(
        move || {
            let (_, ctx) = crate::build_context();
            crate::i18n::install(ctx.general);
            provide_context(ctx.clone());
            provide_context(crate::components::quit_dialog::QuitCtx::new());
            provide_context(crate::build_code_editor_ctx());
            let chat = crate::syn_chat::SynChatCtx::new();
            // Плитки чатов в рейле — из файлов, без `registry::load_all`
            // (тот дописывает в старые файлы настройки).
            chat.chats.set(
                crate::syn_chat::storage::list_meta()
                    .into_iter()
                    .filter(|m| !m.archived)
                    .collect(),
            );
            provide_context(chat);
            provide_context(crate::syn_chat::SynModelRegistry::new());
            provide_context(crate::pages::training::TrainingCtx::new());
            crate::models::install();
            crate::search::provide();
            crate::rail::navigate(&route);
            if let Some(s) = &section {
                if let Ok(mut r) = ctx.settings_router.lock() {
                    r.navigate(s);
                }
                ctx.selected_settings_tab.set(s.clone());
            }
            Box::new(DecoratedBox::new().class("grow").child(crate::build_app()))
        },
        &mss,
        shot.size,
        shot.scale,
        &shot.out,
    )
}
