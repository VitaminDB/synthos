//! Что происходит по Enter: переходы между страницами, открытие чатов и
//! пакетов, тумблеры оформления.
//!
//! Все функции зовутся с main-thread'а — из обработчика клавиш панели или
//! из клика по строке.

use std::path::PathBuf;

use syngui::prelude::*;

use crate::context::AppCtx;
use crate::syn_chat::{registry, SynChatCtx, SynModelRegistry};

use super::model::{SearchAction, SearchCommand, SearchItem};
use super::SearchCtx;

pub fn run(search: &SearchCtx, item: &SearchItem, secondary: bool) {
    let action = if secondary {
        item.secondary
            .clone()
            .unwrap_or_else(|| item.action.clone())
    } else {
        item.action.clone()
    };
    search.remember(&item.key);
    // Переключение инструмента — единственное действие, ради которого
    // панель остаётся открытой: чипы удобно щёлкать пачкой.
    if !matches!(action, SearchAction::ToggleTool(_)) {
        search.close();
    }
    tracing::debug!(title = %item.title, ?action, "поиск: переход");
    match action {
        SearchAction::Page(route) => navigate(route),
        SearchAction::Note(id) => {
            let notes = use_context::<crate::pages::notes::NotesCtx>();
            notes.activate(&id);
            notes.open_tile();
            navigate("notes");
        }
        SearchAction::Settings(section) => open_settings(section),
        SearchAction::Chat(id) => open_chat(&id, None),
        SearchAction::Message { chat_id, index } => open_chat(&chat_id, Some(index)),
        SearchAction::Skill(id) => {
            let app = use_context::<AppCtx>();
            app.skills_selected_id.set(Some(id));
            open_settings("skills");
        }
        SearchAction::Tool(_) => {
            // Инструменты живут в левой панели чата — раскрыть её, если
            // пользователь её сворачивал.
            use_context::<AppCtx>().panels.syn_chat.0.set(true);
            navigate("syn_chat");
        }
        SearchAction::ToggleTool(key) => toggle_tool(&key),
        SearchAction::Template(id) => open_template(&id),
        SearchAction::Model(path) => open_bundle(path),
        SearchAction::LoadModelFile(path) => load_model(path),
        SearchAction::Command(command) => run_command(command),
    }
}

fn navigate(route: &str) {
    let app = use_context::<AppCtx>();
    if app.current_route.get_untracked() == route {
        return;
    }
    if let Ok(mut router) = app.router.lock() {
        router.navigate(route);
    }
    app.current_route.set(route.to_string());
}

fn open_settings(section: &str) {
    let app = use_context::<AppCtx>();
    if app.selected_settings_tab.get_untracked() != section {
        if let Ok(mut router) = app.settings_router.lock() {
            router.navigate(section);
        }
        app.selected_settings_tab.set(section.to_string());
    }
    navigate("settings");
}

/// Открыть чат и, если пришли из найденного сообщения, подсветить его.
/// Порядок важен: `select` сбрасывает подсветку прошлого перехода.
fn open_chat(id: &str, highlight: Option<usize>) {
    let ctx = use_context::<SynChatCtx>();
    if ctx.active_chat_id.get_untracked().as_deref() != Some(id) {
        registry::select(id);
    }
    ctx.highlight_msg.set(highlight);
    navigate("syn_chat");
}

fn toggle_tool(key: &str) {
    let app = use_context::<AppCtx>();
    app.tools.active.update(|list| {
        if let Some(pos) = list.iter().position(|k| k == key) {
            list.remove(pos);
        } else {
            list.push(key.to_string());
        }
    });
}

fn open_template(id: &str) {
    let Some(template) = crate::templates::list_all().into_iter().find(|t| t.id == id) else {
        return;
    };
    let ws = use_context::<crate::pages::node_editor::tabs::EditorWorkspace>();
    ws.open_template(&template);
    navigate("nodes");
}

fn open_bundle(path: PathBuf) {
    let ctx = use_context::<crate::pages::syn_explorer::SynExplorerCtx>();
    crate::pages::syn_explorer::actions::open_bundle(ctx, path);
    navigate("syn_explorer");
}

fn load_model(path: PathBuf) {
    let app = use_context::<AppCtx>();
    let reg = use_context::<SynModelRegistry>();
    if reg.loading.get_untracked() {
        return;
    }
    let policy =
        crate::config::resolve_model_profile(&app.model_profiles.get_untracked(), &path).policy;
    reg.load(path, policy);
    navigate("syn_chat");
}

fn run_command(command: SearchCommand) {
    let app = use_context::<AppCtx>();
    match command {
        SearchCommand::NewChat => {
            registry::create_new();
            navigate("syn_chat");
        }
        SearchCommand::ClearChat => {
            let ctx = use_context::<SynChatCtx>();
            ctx.messages.set(Vec::new());
            ctx.streaming_body.set(String::new());
            ctx.streaming_thinking.set(String::new());
            navigate("syn_chat");
        }
        SearchCommand::CompactNow => {
            crate::syn_chat::compact::compact_now();
            navigate("syn_chat");
        }
        SearchCommand::DeleteChat => {
            let ctx = use_context::<SynChatCtx>();
            // В архив — как и корзина в шапке, команда только поднимает
            // диалог подтверждения (`archive_dialog`).
            ctx.pending_archive.set(registry::active_meta());
            navigate("syn_chat");
        }
        SearchCommand::LoadModel => {
            let reg = use_context::<SynModelRegistry>();
            let Some(path) = reg.last_path.get_untracked() else {
                app.notifications.warning(tr!("search.notice.no_last_model"));
                return;
            };
            load_model(path);
        }
        SearchCommand::UnloadModel => {
            let ctx = use_context::<SynChatCtx>();
            if ctx.pending.get_untracked() {
                ctx.abort
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            use_context::<SynModelRegistry>().unload();
        }
        SearchCommand::NewNodeTab => {
            let ws = use_context::<crate::pages::node_editor::tabs::EditorWorkspace>();
            ws.new_untitled();
            navigate("nodes");
        }
        SearchCommand::TemplatePicker => {
            let ws = use_context::<crate::pages::node_editor::tabs::EditorWorkspace>();
            ws.template_picker_open.set(true);
            navigate("nodes");
        }
        SearchCommand::NewCodeSession => {
            let code = use_context::<crate::pages::code_editor::state::CodeEditorCtx>();
            code.create_empty();
            navigate("code");
        }
        SearchCommand::VoicePanel => crate::components::voice_fab::fab_button::open_panel_and_record(),
        SearchCommand::DarkTheme => apply_base_theme(true),
        SearchCommand::LightTheme => apply_base_theme(false),
        SearchCommand::SystemTheme => {
            let follow = !app.appearance.follow_system.get_untracked();
            app.appearance.follow_system.set(follow);
        }
        SearchCommand::NextLanguage => next_language(),
        SearchCommand::Fullscreen => syngui::signal::toggle_fullscreen(),
        SearchCommand::Glass => {
            let on = !app.appearance.window_blur.get_untracked();
            app.appearance.window_blur.set(on);
        }
    }
}

/// Явный выбор светлой или тёмной темы. В режиме «как в системе» ручной
/// выбор смысла не имеет, поэтому режим выключается, а активной становится
/// половина системной пары — или встроенная тема нужной светлоты.
fn apply_base_theme(dark: bool) {
    use crate::pages::settings::theme_data;
    let app = use_context::<AppCtx>();
    let a = app.appearance;
    if a.follow_system.get_untracked() {
        a.follow_system.set(false);
    }
    let paired = if dark {
        a.theme_dark.get_untracked()
    } else {
        a.theme_light.get_untracked()
    };
    let theme = theme_data::find(&paired)
        .filter(|t| t.is_dark == dark)
        .unwrap_or_else(|| theme_data::find_or_default("", dark));
    app.theme_key.set(theme.id.to_string());
    app.theme_mss.set(theme.to_mss());
}

/// Следующий язык интерфейса по кругу. `auto` — часть круга: с него
/// начинается список, на него же он и замыкается.
fn next_language() {
    let app = use_context::<AppCtx>();
    let mut tags: Vec<String> = vec![crate::i18n::AUTO.to_string()];
    tags.extend(
        syngui::i18n::languages()
            .into_iter()
            .map(|l| l.tag.tag().to_string()),
    );
    let current = app.general.language.get_untracked();
    let pos = tags.iter().position(|t| *t == current).unwrap_or(0);
    let next = tags[(pos + 1) % tags.len()].clone();
    app.general.language.set(next);
}
