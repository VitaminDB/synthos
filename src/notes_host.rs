//! Режим «Заметки» — библиотека заметок synshell (`syn_notes`). Здесь её
//! связи с synthos: состояние заметок — поля `AppConfig` synthos (те же
//! имена), контекст приложения заметок — поверх сигналов `AppCtx`,
//! навигация и плитки — рейл synthos, вложения — чаты synthos, середина
//! шапки — кнопка поиска.

use syngui::prelude::*;

use crate::config::{AppConfig, NotesProjectConfig};
use crate::context::AppCtx;

/// Состояние заметок из конфига synthos.
pub fn state(cfg: &AppConfig) -> syn_notes::config::AppConfig {
    syn_notes::config::AppConfig {
        notes_left_split_ratio: cfg.notes_left_split_ratio,
        notes_right_split_ratio: cfg.notes_right_split_ratio,
        notes_vault_path: cfg.notes_vault_path.clone(),
        notes_project_path: cfg.notes_project_path.clone(),
        notes_active: cfg.notes_active.clone(),
        notes_expanded: cfg.notes_expanded.clone(),
        notes_tile_opened_at: cfg.notes_tile_opened_at,
        notes_projects: cfg.notes_projects.as_ref().map(|list| {
            list.iter()
                .map(|p| syn_notes::config::NotesProjectConfig {
                    path: p.path.clone(),
                    opened_at: p.opened_at,
                    active_page: p.active_page.clone(),
                    expanded: p.expanded.clone(),
                })
                .collect()
        }),
        notes_active_project: cfg.notes_active_project.clone(),
        notes_recent: cfg.notes_recent.clone(),
        notes_left: cfg.panels.notes_left,
        notes_right: cfg.panels.notes_right,
    }
}

/// Открытые проекты заметок — в конфиг synthos.
pub fn projects(list: Vec<syn_notes::config::NotesProjectConfig>) -> Vec<NotesProjectConfig> {
    list.into_iter()
        .map(|p| NotesProjectConfig { path: p.path, opened_at: p.opened_at, active_page: p.active_page, expanded: p.expanded })
        .collect()
}

/// Поставить связи до первого `NotesCtx`. Зовётся с главного потока.
pub fn install(ctx: &AppCtx) {
    provide_context(syn_notes::context::AppCtx {
        notifications: ctx.notifications.clone(),
        panels: syn_notes::context::PanelsCtx { notes: ctx.panels.notes },
        notes_left_split_ratio: ctx.notes_left_split_ratio,
        notes_right_split_ratio: ctx.notes_right_split_ratio,
        current_route: ctx.current_route,
    });
    syn_notes::config::set_loader(|| state(&AppConfig::load()));
    syn_notes::rail::set_host(syn_notes::rail::RailHost { navigate: crate::rail::navigate, rename_key: crate::rail::rename_key });
    syn_notes::attach::set_finder(|r| {
        let a = crate::agent::tools::pipelines::find_attachment(r)?;
        Some(syn_notes::attach::Attachment { sha256: a.sha256, ext: a.ext, original_name: a.original_name })
    });
    syn_notes::components::panel_header::set_center_widget(|| Box::new(crate::search::trigger::view()));
}
