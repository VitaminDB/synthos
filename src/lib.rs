//! Synthos app — frameless messenger-style shell.
//!
//! Весь контент правее nav-rail рендерится через `RouterView` по текущему
//! маршруту. Маршрут `chat` сохраняет исходную компоновку мессенджера,
//! `settings` — страница настроек со своим вложенным роутером,
//! остальные маршруты — временные заглушки.

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::navigation::router::{Router, RouterView};

pub mod agent;
pub mod components;
pub mod config;
pub mod context;
pub mod i18n;
pub mod icons;
pub mod kb;
pub mod logging;
pub mod metrics;
pub mod migrate;
pub mod models;
pub mod pages;
pub mod paths;
pub mod rail;
pub mod search;
pub mod skills;
pub mod styles;
pub mod syn_chat;
pub mod templates;

use components::titlebar;
use config::AppConfig;
use context::{
    AppCtx, GeneralCtx, ToolsCtx, VoiceFabCtx, INITIAL_ROUTE,
    INITIAL_SETTINGS_ROUTE, ROUTES, SETTINGS_ROUTES,
};
use metrics::MetricsState;
use pages::settings::theme_data;

pub fn run_desktop() {
    // Stderr (INFO+, ANSI, RUST_LOG override) и rolling daily файл
    // (DEBUG+, без ANSI) — для post-mortem отладки. Подробности —
    // в `logging.rs`.
    logging::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "Synthos starting"
    );

    // Регистрация backend'ов synaptix (CPU + CUDA) в глобальном registry.
    // Это две вставки в диспетчерскую таблицу — CUDA-контекст не
    // создаётся, `CudaBackend` unit-структура, — поэтому делать её на
    // старте дёшево и безопасно.
    //
    // Раньше регистрация висела на каждом потребителе отдельно
    // (`ensure_kernels_registered` в нодах LTX/H3/LLM/VoxCPM/ASR), и путь,
    // который её не звал, падал на первом же `cast`: глобальный голосовой
    // FAB грузил ASR через `agent::audio::load_selected_model` и получал
    // «backend not registered for device Cpu — did you call
    // synaptix::init()?». Одна регистрация на процесс убирает весь класс
    // таких промахов; локальные вызовы идемпотентны и остаются как есть.
    if let Err(e) = synaptix::init() {
        tracing::error!(error = %e, "synaptix::init: регистрация backend'ов не удалась");
    }

    // Регистрация sqlite-vec auto-extension: применяется глобально ко
    // всем последующим Connection'ам в процессе. Без feature
    // `kb-sqlite-vec` — no-op, KB остаётся на full-scan.
    kb::init_sqlite_vec();

    let (theme_mss, ctx) = build_context();
    i18n::install(ctx.general);

    App::new()
        .title(concat!("Synthos v", env!("CARGO_PKG_VERSION")))
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .min_size(1920, 1000)
        .maximized(true)
        .vsync(true)
        .gpu_backend(GpuBackend::Auto)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(styles::styles())
        .with_dynamic_theme(theme_mss)
        .with_system_appearance(ctx.appearance.system)
        .with_backdrop(ctx.appearance.backdrop)
        .with_window_state(ctx.appearance.window_state)
        .run(move |_| {
            provide_context(ctx.clone());
            // Снимок до автосейва: `install_config_autosave` пишет файл сразу
            // при установке, и `restore_last_view` увидел бы уже стартовые
            // значения вместо сохранённых при выходе.
            let startup_cfg = AppConfig::load();
            let code_ctx = build_code_editor_ctx();
            provide_context(code_ctx);
            // Семплер занятости терминалов — для бейджей на плитках сессий
            // в нав-рейле (см. pages::code_editor::terminal_activity).
            pages::code_editor::terminal_activity::start_sampler(code_ctx);
            provide_context(pages::node_editor::tabs::EditorWorkspace::new_or_restore());
            provide_context(pages::syn_explorer::SynExplorerCtx::new(&AppConfig::load()));
            provide_context(syn_chat::SynChatCtx::new());
            provide_context(pages::notes::NotesCtx::new_or_restore(&AppConfig::load()));
            provide_context(syn_chat::SynModelRegistry::new());
            let hf_ctx = pages::huggingface::HuggingFaceCtx::new(&AppConfig::load());
            provide_context(hf_ctx);
            // Восстановление прерванной очереди HF-загрузок с прошлого
            // запуска: Pending/Active → Pending, Done остаётся Done.
            // try_drain_queue запускает первые `concurrent_limit` файлов.
            let persisted = pages::huggingface::persist::load();
            pages::huggingface::download::resume_persisted(
                hf_ctx,
                ctx.notifications.clone(),
                persisted,
            );
            pages::huggingface::persist::install_autosave(hf_ctx);
            install_config_autosave(&ctx);
            install_syn_chat_autosave();
            install_workspace_autosave();
            pages::notes::autosave::install_notes_autosave();
            // Watcher sequencer'а нодового редактора ставится здесь, а не в
            // `run_controls::view`: страница пересобирается роутером, и
            // effect, заведённый внутри неё, умирал вместе с прогоном.
            pages::node_editor::run_controls::install_run_watcher();
            // Сигнал версии реестра моделей: заводится с main-thread'а,
            // дальше его дёргают загрузчики из воркеров.
            models::install();
            install_voice_auto_record(&ctx);
            metrics::system::start_sampler(ctx.metrics.clone());
            syn_chat::registry::load_all();
            restore_last_view(&startup_cfg);
            // Глобальный поиск: контекст и эффект сборки индекса. Ставится
            // после `load_all` — первый же индекс видит загруженные чаты.
            search::install();
            Box::new(DecoratedBox::new().class("grow").child(move || {
                syngui::i18n::subscribe();
                build_app()
            }))
        });
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: syngui::app::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    let (theme_mss, ctx) = build_context();

    App::new()
        .title(concat!("Synthos v", env!("CARGO_PKG_VERSION")))
        .vsync(true)
        .gpu_backend(GpuBackend::Gl)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_android_app(app)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(styles::styles())
        .with_dynamic_theme(theme_mss)
        .run(move |_| {
            provide_context(ctx.clone());
            // Снимок до автосейва: `install_config_autosave` пишет файл сразу
            // при установке, и `restore_last_view` увидел бы уже стартовые
            // значения вместо сохранённых при выходе.
            let startup_cfg = AppConfig::load();
            let code_ctx = build_code_editor_ctx();
            provide_context(code_ctx);
            // Семплер занятости терминалов — для бейджей на плитках сессий
            // в нав-рейле (см. pages::code_editor::terminal_activity).
            pages::code_editor::terminal_activity::start_sampler(code_ctx);
            provide_context(pages::node_editor::tabs::EditorWorkspace::new_or_restore());
            provide_context(pages::syn_explorer::SynExplorerCtx::new(&AppConfig::load()));
            provide_context(syn_chat::SynChatCtx::new());
            provide_context(pages::notes::NotesCtx::new_or_restore(&AppConfig::load()));
            provide_context(syn_chat::SynModelRegistry::new());
            let hf_ctx = pages::huggingface::HuggingFaceCtx::new(&AppConfig::load());
            provide_context(hf_ctx);
            // Восстановление прерванной очереди HF-загрузок с прошлого
            // запуска: Pending/Active → Pending, Done остаётся Done.
            // try_drain_queue запускает первые `concurrent_limit` файлов.
            let persisted = pages::huggingface::persist::load();
            pages::huggingface::download::resume_persisted(
                hf_ctx,
                ctx.notifications.clone(),
                persisted,
            );
            pages::huggingface::persist::install_autosave(hf_ctx);
            install_config_autosave(&ctx);
            install_syn_chat_autosave();
            install_workspace_autosave();
            pages::notes::autosave::install_notes_autosave();
            // Watcher sequencer'а нодового редактора ставится здесь, а не в
            // `run_controls::view`: страница пересобирается роутером, и
            // effect, заведённый внутри неё, умирал вместе с прогоном.
            pages::node_editor::run_controls::install_run_watcher();
            // Сигнал версии реестра моделей: заводится с main-thread'а,
            // дальше его дёргают загрузчики из воркеров.
            models::install();
            install_voice_auto_record(&ctx);
            metrics::system::start_sampler(ctx.metrics.clone());
            syn_chat::registry::load_all();
            restore_last_view(&startup_cfg);
            // Глобальный поиск: контекст и эффект сборки индекса. Ставится
            // после `load_all` — первый же индекс видит загруженные чаты.
            search::install();
            Box::new(DecoratedBox::new().class("grow").child(move || {
                syngui::i18n::subscribe();
                build_app()
            }))
        });
}

/// Вернуть пользователя туда, где он закрыл приложение.
///
/// Стартовый маршрут `INITIAL_ROUTE` — `syn_chat`, и без восстановления
/// запуск всегда открывал страницу чата: у кого в рейле только code-сессии,
/// тот получал пустое «Select or create a chat» вместо своего проекта.
/// Активные сущности внутри страниц восстанавливают свои владельцы
/// (`active_code_session` в конфиге, `active` в workspace.json), здесь —
/// только страница и активный чат.
///
/// Вызывается после `registry::load_all()`: список чатов уже загружен, и
/// сохранённый id можно проверить на существование (чат мог быть удалён или
/// заархивирован в прошлой сессии). Конфиг приходит снимком, снятым до
/// `install_config_autosave`: тот effect выполняется сразу при создании и
/// успевает переписать файл стартовыми значениями (`syn_chat`, чат не
/// выбран) — перечитывать его здесь было бы поздно.
/// Маршрут для восстановления: сохранённый, если он всё ещё существует в
/// `ROUTES` (набор меняется между версиями — выкинутая страница не должна
/// уводить старт в никуда).
fn restorable_route(saved: &AppConfig) -> Option<&str> {
    saved
        .last_route
        .as_deref()
        .filter(|r| context::ROUTES.contains(r))
}

fn restore_last_view(saved: &AppConfig) {
    let chat = use_context::<syn_chat::SynChatCtx>();
    if let Some(id) = saved.last_chat_id.as_deref() {
        if chat.chats.get_untracked().iter().any(|m| m.id == id && !m.archived) {
            syn_chat::registry::select(id);
        }
    }
    if let Some(route) = restorable_route(saved) {
        rail::navigate(route);
    }
    // Показывать на чат-странице нечего (чат не восстановился или его
    // удалили) — открываем первую плитку рейла, обычно code-сессию. Иначе
    // приложение встречает пустым плейсхолдером при живом рабочем окружении.
    let app = use_context::<AppCtx>();
    if app.current_route.get_untracked() == "syn_chat"
        && chat.active_chat_id.get_untracked().is_none()
    {
        if let Some(entry) = rail::entries()
            .into_iter()
            .find(|e| !matches!(e, rail::RailEntry::Separator(_)))
        {
            rail::open(&entry);
        }
    }
}

/// Загружает persist-слой (sessions + active index) и собирает менеджер
/// сессий редактора кода. При отсутствии новых полей в конфиге выполняет
/// миграцию из legacy `last_code_folder` — одна-единственная сессия с
/// этим путём; на следующем save'е legacy-поле затирается в `None`.
fn build_code_editor_ctx() -> pages::code_editor::state::CodeEditorCtx {
    let saved = AppConfig::load();
    let mut sessions_cfg = saved.code_sessions.clone();
    if sessions_cfg.is_empty() {
        if let Some(legacy) = saved.last_code_folder.clone() {
            sessions_cfg.push(config::CodeSessionConfig {
                root_folder: Some(legacy),
                open_files: Vec::new(),
                active_file: None,
                split_ratio: None,
                left_split_ratio: None,
                right_split_ratio: None,
                soft_wrap: false,
                editor_states: std::collections::HashMap::new(),
                created_at: None,
            });
        }
    }
    let active_idx = saved
        .active_code_session
        .filter(|&i| i < sessions_cfg.len())
        .or_else(|| if sessions_cfg.is_empty() { None } else { Some(0) });
    pages::code_editor::state::CodeEditorCtx::new(sessions_cfg, active_idx)
}

/// Активная тема: в режиме «следовать системе» её выбирает светлота системной
/// схемы, иначе — ручной выбор пользователя.
pub(crate) fn active_theme(
    appearance: context::AppearanceCtx,
    theme_key: RwSignal<String>,
) -> theme_data::SynthosTheme {
    if appearance.follow_system.get() {
        let dark = appearance.system.get().is_dark();
        let key = if dark { appearance.theme_dark.get() } else { appearance.theme_light.get() };
        theme_data::find_or_default(&key, dark)
    } else {
        theme_data::find(&theme_key.get()).unwrap_or_else(theme_data::default_theme)
    }
}

/// Настройка «стекла» для фреймворка. Контраст просим вместе с размытием —
/// иначе на светлом рабочем столе полупрозрачные панели теряют читаемость.
///
/// Область эффекта повторяет `.shell` из `styles/layout/shell.mss`: в
/// восстановленном окне вокруг него 30px прозрачного воздуха под тень и
/// resize-захват, углы скруглены на `--radius-shell`; в развёрнутом и то и
/// другое обнуляется. Без этого композитор размывает всю поверхность, и вокруг
/// окна повисает мутный прямоугольник.
fn backdrop_config(blur: bool, maximized: bool) -> syngui::window::BackdropConfig {
    if !blur {
        return syngui::window::BackdropConfig::disabled();
    }
    let (inset, radius) = if maximized { (0.0, 0.0) } else { (30.0, 20.0) };
    syngui::window::BackdropConfig::frosted().with_shell(inset, radius)
}

fn build_context() -> (RwSignal<String>, AppCtx) {
    let saved = AppConfig::load();

    // Тема: resolve id → MSS. Пустой id или неизвестный → дефолт.
    let (theme_id, theme_mss_value) = match theme_data::find(&saved.theme) {
        Some(t) => (t.id.to_string(), t.to_mss()),
        None => {
            let def = theme_data::default_theme();
            (def.id.to_string(), def.to_mss())
        }
    };

    let theme_key = use_signal(theme_id);
    let theme_mss = use_signal(theme_mss_value);

    // Системное оформление читаем до первого кадра: с `follow_system_theme`
    // приложение должно стартовать уже в системной схеме, а не мигать светлой
    // темой, пока фреймворк донесёт первое обновление сигнала.
    let appearance = context::AppearanceCtx {
        system: use_signal(syngui::appearance::read_system_appearance()),
        follow_system: use_signal(saved.follow_system_theme),
        theme_light: use_signal(saved.theme_light.clone()),
        theme_dark: use_signal(saved.theme_dark.clone()),
        use_system_accent: use_signal(saved.use_system_accent),
        system_window_controls: use_signal(saved.system_window_controls),
        window_blur: use_signal(saved.window_blur),
        window_opacity: use_signal(saved.window_opacity),
        backdrop: use_signal(backdrop_config(saved.window_blur, false)),
        window_state: use_signal(syngui::window::WindowState::default()),
    };

    let current_route = use_signal(INITIAL_ROUTE.to_string());
    let selected_settings_tab = use_signal(INITIAL_SETTINGS_ROUTE.to_string());
    let skills_loaded = crate::skills::load_all();
    let skills_signal = use_signal(skills_loaded);
    let skills_selected_id = use_signal(None::<String>);
    let skills_active = use_signal(saved.skills_active.clone());
    let skills_dialog = use_signal(None::<context::SkillDialogKind>);
    let kb_url_dialog = use_signal(None::<String>);

    let router = Arc::new(Mutex::new(Router::new(
        ROUTES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        INITIAL_ROUTE,
    )));
    let settings_router = Arc::new(Mutex::new(Router::new(
        SETTINGS_ROUTES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        INITIAL_SETTINGS_ROUTE,
    )));

    let general = GeneralCtx {
        display_name: use_signal(saved.general.display_name.clone()),
        language: use_signal(saved.general.language.clone()),
        system_prompt: use_signal(saved.general.system_prompt.clone()),
        voice_refine_prompt: use_signal(saved.general.voice_refine_prompt.clone()),
        tool_display_mode: use_signal(saved.general.tool_display_mode.clone()),
        tool_approval_default: use_signal(saved.general.tool_approval_default.clone()),
        tool_approval_overrides: use_signal(saved.general.tool_approval_overrides.clone()),
        audio_input_device: use_signal(saved.general.audio_input_device.clone()),
        voice_font_family: use_signal(saved.general.voice_font_family.clone()),
        voice_font_size: use_signal(saved.general.voice_font_size),
        agent_max_turns: use_signal(saved.general.agent_max_turns),
        subagent_max_turns: use_signal(saved.general.subagent_max_turns),
        autocompact_enabled: use_signal(saved.general.autocompact_enabled),
        autocompact_threshold_percent: use_signal(saved.general.autocompact_threshold_percent),
    };


    let audio_models = use_signal(saved.audio_models.clone());
    let selected_audio_model = use_signal(saved.selected_audio_model.clone());
    let audio = agent::audio::AudioCtx::new();
    let voice = VoiceFabCtx::new();
    let metrics = Arc::new(MetricsState::new());
    let tools = ToolsCtx::new(saved.tools_active.clone());

    // KB / RAG. Реестр сканируется сразу — это быстро (просто listdir).
    // Embedder грузится лениво при первом ingest/search.
    let kb_dir = saved.kb.kb_dir.as_str();
    let kb_dir_path = if kb_dir.is_empty() {
        config::default_kb_dir()
    } else {
        std::path::PathBuf::from(kb_dir)
    };
    let kb = kb::KbCtx::new(kb_dir_path);
    kb.auto_augment.set(saved.kb.auto_augment_default);

    // Notification ctx: 15s default duration по требованию TASK.md.
    let notifications = syngui::widgets::feedback::NotificationCtx::with_default_duration(15_000);
    let terminal_font_family = use_signal(saved.terminal_font_family.clone());
    let terminal_font_size = use_signal(saved.terminal_font_size);
    let code_editor_font_family = use_signal(saved.code_editor_font_family.clone());
    let code_editor_font_size = use_signal(saved.code_editor_font_size);
    let model_profiles = use_signal(saved.model_profiles.clone());
    let syn_chat_max_image_tokens = use_signal(saved.syn_chat_max_image_tokens);
    let acestep_xl_bundle_path = use_signal(saved.acestep_xl_bundle_path.clone());
    let acestep_vae_bundle_path = use_signal(saved.acestep_vae_bundle_path.clone());
    let models_dir = use_signal(saved.models_dir.clone());
    let panels = context::PanelsCtx::from_config(&saved.panels);
    let rail_separators = use_signal(saved.rail_separators.clone());
    let rail_order = use_signal(saved.rail_order.clone());
    let hf_left_split_ratio = use_signal(saved.hf_left_split_ratio);
    let hf_right_split_ratio = use_signal(saved.hf_right_split_ratio);
    let settings_left_split_ratio = use_signal(saved.settings_left_split_ratio);
    let settings_right_split_ratio = use_signal(saved.settings_right_split_ratio);
    let notes_left_split_ratio = use_signal(saved.notes_left_split_ratio);
    let notes_right_split_ratio = use_signal(saved.notes_right_split_ratio);

    // Рантайм-часть профиля выставляется не здесь, а при загрузке модели:
    // выверенные пути у каждой архитектуры свои, и глобальные тумблеры
    // ровно этим и мешали (`ModelProfileConfig::resolve`).

    let ctx = AppCtx {
        theme_key,
        theme_mss,
        appearance,
        router,
        current_route,
        settings_router,
        selected_settings_tab,
        skills: skills_signal,
        skills_selected_id,
        skills_active,
        skills_dialog,
        kb_url_dialog,
        general,
        audio_models,
        selected_audio_model,
        audio,
        voice,
        metrics,
        tools,
        terminal_font_family,
        terminal_font_size,
        code_editor_font_family,
        code_editor_font_size,
        kb,
        notifications,
        model_profiles,
        syn_chat_max_image_tokens,
        models_dir,
        acestep_xl_bundle_path,
        acestep_vae_bundle_path,
        panels,
        rail_separators,
        rail_order,
        hf_left_split_ratio,
        hf_right_split_ratio,
        settings_left_split_ratio,
        settings_right_split_ratio,
        notes_left_split_ratio,
        notes_right_split_ratio,
    };

    // Реактивные MSS-переменные шрифтов поверх палитры темы:
    //   - voice-overlay (FAB-окно):       `--voice-font-family / --voice-font-size`
    //   - CodeEditor (страница «code»):   `--code-editor-font-family / --code-editor-font-size`
    //
    // Effect перегенерирует blob `theme_mss` при смене темы или правке любого
    // из этих сигналов в Settings — изменения видны мгновенно, без перезапуска.
    {
        let theme_key = theme_key;
        let theme_mss = theme_mss;
        let voice_family = general.voice_font_family;
        let voice_size = general.voice_font_size;
        let editor_family = code_editor_font_family;
        let editor_size = code_editor_font_size;
        create_effect(move || {
            let theme = active_theme(appearance, theme_key);
            let base = theme.to_mss();

            // Порядок блоков = приоритет: переменные из последнего `:root`
            // перекрывают предыдущие (StyleSheet держит их в HashMap).
            let mut extra = String::new();
            if appearance.use_system_accent.get() {
                if let Some(accent) = appearance.system.get().accent {
                    extra.push_str(&theme_data::accent_override_mss(accent, theme.is_dark));
                }
            }
            let opacity = appearance.window_opacity.get();
            if opacity < 0.999 {
                extra.push_str(&theme_data::surface_alpha_mss(&theme, opacity));
            }

            let vf = voice_family.get();
            let vf = if vf.trim().is_empty() { "sans-serif".to_string() } else { vf };
            let vs = voice_size.get();
            let ef = editor_family.get();
            let ef = if ef.trim().is_empty() { "monospace".to_string() } else { ef };
            let es = editor_size.get();
            theme_mss.set(format!(
                "{base}\n{extra}\n:root {{\n  --voice-font-family: {vf};\n  --voice-font-size: {vs:.0}px;\n  --code-editor-font-family: {ef};\n  --code-editor-font-size: {es:.0}px;\n}}\n"
            ));
        });
    }

    // Размытие фона композитором включается/выключается на лету; форма области
    // пересобирается при разворачивании окна.
    {
        let blur = appearance.window_blur;
        let window_state = appearance.window_state;
        let backdrop = appearance.backdrop;
        create_effect(move || {
            backdrop.set(backdrop_config(blur.get(), window_state.get().maximized))
        });
    }

    (theme_mss, ctx)
}

/// Подписывается на все persistent-сигналы и сохраняет конфиг при любом
/// изменении. Эффект выполняется один раз при установке (запишет файл со
/// стартовым состоянием — это нормально, содержимое совпадает с загруженным).
///
/// Список code-сессий читается через `use_context::<CodeEditorCtx>()` — менеджер
/// уже provide_context'ится в `run` до этого вызова. Effect подписывается на
/// `mgr.sessions`, `mgr.active_id` и через iter — на `root_folder` каждой
/// сессии: правка любого из них триггерит save.
fn install_config_autosave(ctx: &AppCtx) {
    let theme_key = ctx.theme_key;
    let current_route = ctx.current_route;
    let active_chat_id = use_context::<syn_chat::SynChatCtx>().active_chat_id;
    let a = ctx.appearance;
    let g = ctx.general;
    let tools_active = ctx.tools.active;
    let skills_active = ctx.skills_active;
    let audio_models = ctx.audio_models;
    let selected_audio_model = ctx.selected_audio_model;
    let terminal_font_family = ctx.terminal_font_family;
    let terminal_font_size = ctx.terminal_font_size;
    let code_editor_font_family = ctx.code_editor_font_family;
    let code_editor_font_size = ctx.code_editor_font_size;
    let kb_auto_augment = ctx.kb.auto_augment;
    let model_profiles = ctx.model_profiles;
    let syn_chat_max_image_tokens = ctx.syn_chat_max_image_tokens;
    let acestep_xl_bundle_path = ctx.acestep_xl_bundle_path;
    let acestep_vae_bundle_path = ctx.acestep_vae_bundle_path;
    let models_dir_sig = ctx.models_dir;
    let panels = ctx.panels;
    let rail_separators = ctx.rail_separators;
    let rail_order = ctx.rail_order;
    let hf_left_split = ctx.hf_left_split_ratio;
    let hf_right_split = ctx.hf_right_split_ratio;
    let settings_left_split = ctx.settings_left_split_ratio;
    let settings_right_split = ctx.settings_right_split_ratio;
    let notes_left_split = ctx.notes_left_split_ratio;
    let notes_right_split = ctx.notes_right_split_ratio;
    let notes_ctx = use_context::<pages::notes::NotesCtx>();
    let code = use_context::<pages::code_editor::state::CodeEditorCtx>();
    let syn = use_context::<pages::syn_explorer::state::SynExplorerCtx>();
    let hf = use_context::<pages::huggingface::HuggingFaceCtx>();
    let syn_chat_ctx = use_context::<syn_chat::SynChatCtx>();
    let syn_chat_left_split = syn_chat_ctx.left_split_ratio;
    let syn_chat_right_split = syn_chat_ctx.right_split_ratio;

    create_effect(move || {
        let (notes_active_state, notes_expanded_state, notes_tile_state) = notes_ctx.persist();
        let sessions = code.sessions.get();
        let active_id = code.active_id.get();
        let sessions_cfg: Vec<config::CodeSessionConfig> = sessions
            .iter()
            .map(|s| config::CodeSessionConfig {
                root_folder: s.root_folder.get().map(|p| p.display().to_string()),
                open_files: s
                    .open_files
                    .get()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                active_file: s.active_file.get().map(|p| p.display().to_string()),
                // Per-session splitter ratio'ы. `.get()` подписывает effect:
                // drag дивайдера → set() сигнала → autosave перезаписывает
                // config.json с актуальными значениями для каждой сессии.
                split_ratio: Some(s.split_ratio.get()),
                left_split_ratio: Some(s.left_split_ratio.get()),
                right_split_ratio: Some(s.right_split_ratio.get()),
                soft_wrap: s.soft_wrap.get(),
                // Persisted-снимки cursor/scroll по открытым файлам. `.get()`
                // подписывает effect — каждое изменение в editor'е (через
                // `state_signal` → запись в `editor_states`) триггерит
                // повторный save конфига.
                editor_states: s
                    .editor_states
                    .get()
                    .iter()
                    .map(|(path, st)| {
                        (
                            path.display().to_string(),
                            config::EditorStateConfig {
                                cursor_offset: st.cursor_offset,
                                scroll_lines: st.scroll_lines,
                                scroll_x: st.scroll_x,
                            },
                        )
                    })
                    .collect(),
                created_at: Some(s.created_at),
            })
            .collect();
        let active_idx = active_id.and_then(|id| sessions.iter().position(|s| s.id == id));

        let cfg = AppConfig {
            theme: theme_key.get(),
            follow_system_theme: a.follow_system.get(),
            theme_light: a.theme_light.get(),
            theme_dark: a.theme_dark.get(),
            use_system_accent: a.use_system_accent.get(),
            system_window_controls: a.system_window_controls.get(),
            window_blur: a.window_blur.get(),
            window_opacity: a.window_opacity.get(),
            general: config::GeneralConfig {
                display_name: g.display_name.get(),
                language: g.language.get(),
                system_prompt: g.system_prompt.get(),
                voice_refine_prompt: g.voice_refine_prompt.get(),
                tool_display_mode: g.tool_display_mode.get(),
                tool_approval_default: g.tool_approval_default.get(),
                tool_approval_overrides: g.tool_approval_overrides.get(),
                audio_input_device: g.audio_input_device.get(),
                voice_font_family: g.voice_font_family.get(),
                voice_font_size: g.voice_font_size.get(),
                agent_max_turns: g.agent_max_turns.get(),
                subagent_max_turns: g.subagent_max_turns.get(),
                autocompact_enabled: g.autocompact_enabled.get(),
                autocompact_threshold_percent: g.autocompact_threshold_percent.get(),
            },
            tools_active: tools_active.get(),
            // Загруженный конфиг уже прошёл `introduce_notes_tool`.
            tools_notes_introduced: true,
            skills_active: skills_active.get(),
            audio_models: audio_models.get(),
            selected_audio_model: selected_audio_model.get(),
            audio_autostart: false,
            code_sessions: sessions_cfg,
            active_code_session: active_idx,
            // Что показывать на следующем старте: страница + активный чат
            // (у code/graph свои поля — active_code_session и workspace.json).
            last_route: Some(current_route.get()),
            last_chat_id: active_chat_id.get(),
            // Legacy-поле зачищаем явно: миграция один раз произошла в
            // build_code_editor_ctx, держать копию пути больше не нужно.
            last_code_folder: None,
            terminal_font_family: terminal_font_family.get(),
            terminal_font_size: terminal_font_size.get(),
            code_editor_font_family: code_editor_font_family.get(),
            code_editor_font_size: code_editor_font_size.get(),
            // KB persisted-поля. Остальные параметры KbConfig не редактируются
            // через autosave — пользователь меняет их в Settings → Базы знаний,
            // и тот код прямо записывает свежий AppConfig.
            kb: config::KbConfig {
                auto_augment_default: kb_auto_augment.get(),
                ..AppConfig::load().kb
            },
            // SynExplorer: закладки + layout-разделители.
            // `.get()` подписывают effect, drag дивайдера / add bookmark →
            // autosave перезаписывает config.json.
            syn_explorer_bookmarks: syn
                .bookmarks
                .get()
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            syn_explorer_left_split_ratio: syn.left_split_ratio.get(),
            syn_explorer_right_split_ratio: syn.right_split_ratio.get(),
            syn_explorer_expert: syn.wizard.expert.get(),
            hf_cache_dir: hf.cache_dir.get(),
            hf_concurrent_downloads: hf.concurrent_limit.get(),
            hf_segments_per_file: hf.segments_per_file.get(),
            hf_speed_limit_mbps: hf.speed_limit_mbps.get(),
            hf_skip_unwanted_formats: hf.skip_unwanted_formats.get(),
            hf_gguf_support: hf.gguf_support.get(),

            hf_token: hf.token.get(),
            model_profiles: model_profiles.get(),
            syn_chat_max_image_tokens: syn_chat_max_image_tokens.get(),
            acestep_xl_bundle_path: acestep_xl_bundle_path.get(),
            acestep_vae_bundle_path: acestep_vae_bundle_path.get(),
            models_dir: models_dir_sig.get(),
            // Layout-разделители страницы Syn-чата. `.get()` подписывает
            // effect: drag дивайдера → set() сигнала → autosave пишет новые
            // ширины в config.json.
            syn_chat_left_split_ratio: syn_chat_left_split.get(),
            syn_chat_right_split_ratio: syn_chat_right_split.get(),
            hf_left_split_ratio: hf_left_split.get(),
            hf_right_split_ratio: hf_right_split.get(),
            settings_left_split_ratio: settings_left_split.get(),
            settings_right_split_ratio: settings_right_split.get(),
            notes_left_split_ratio: notes_left_split.get(),
            notes_right_split_ratio: notes_right_split.get(),
            // Активная страница, раскрытые узлы и плитка заметок: `.get()`
            // внутри `persist` подписывают effect на их смену.
            notes_active: notes_active_state,
            notes_expanded: notes_expanded_state,
            notes_tile_opened_at: notes_tile_state,
            panels: panels.to_config(),
            rail_separators: rail_separators.get(),
            rail_order: rail_order.get(),
            // Syn-чат поля. Они автосохраняются отдельным
            // `install_syn_chat_autosave` (per-chat params), но дефолты для
            // новых чатов и last_syn_model — здесь, через AppConfig.
            //
            // Чтобы не сломать ранее сохранённые поля при сбросе config'а
            // в Settings, читаем текущий снепшот с диска и переиспользуем.
            ..AppConfig::load()
        };
        cfg.save();
    });

    // Sync HF-токен (Settings → HuggingFace) → api-слой (Authorization: Bearer).
    // Применяется к следующему HF-запросу без перезапуска. Стартовая установка —
    // в `HuggingFaceCtx::new`; этот эффект ловит правки поля в Settings.
    create_effect(move || {
        let t = hf.token.get();
        pages::huggingface::api::set_token(&t);
    });
    // Sync лимита скорости (тулбар / Settings) → глобальный token-bucket.
    // 0 МБ/с = без лимита. Применяется к активным загрузкам на лету.
    create_effect(move || {
        let mbps = hf.speed_limit_mbps.get();
        pages::huggingface::rate::set_limit_bps((mbps as u64) * 1024 * 1024);
    });
}

/// Автосохранение Syn-чатов на диск + sync system_prompt с AppConfig.
///
/// Эффект 1 (per-chat): подписывается на `messages`/`chats`/`active_chat_id`/
/// `params`. Отпечаток считает `registry::state_fingerprint` — он включает
/// params, чтобы изменение слайдеров вызывало disk write через
/// snapshot_current (там params сохраняются в StoredChat), и он же кладётся
/// в `last_saved_fp` при выборе чата, чтобы простое переключение не
/// считалось правкой.
///
/// Эффект 2 (global): подписывается на `system_prompt`, пишет в
/// `AppConfig.syn_chat_system_prompt`.
fn install_syn_chat_autosave() {
    let ctx = use_context::<syn_chat::SynChatCtx>();

    // Изначально подтягиваем system_prompt из AppConfig (один раз на старте).
    let initial_cfg = config::AppConfig::load();
    ctx.system_prompt.set_always(initial_cfg.syn_chat_system_prompt.clone());

    // Эффект 1 — per-chat persistence.
    create_effect(move || {
        let _id = ctx.active_chat_id.get();
        let msgs = ctx.messages.get();
        let chats = ctx.chats.get();
        let params = ctx.params.get();

        if ctx.loading.get_untracked() {
            return;
        }
        let Some(active_id) = ctx.active_chat_id.get_untracked() else {
            return;
        };

        let title = chats
            .iter()
            .find(|m| m.id == active_id)
            .map(|m| m.title.clone())
            .unwrap_or_default();

        // Тот же расчёт, что `registry::select_internal` кладёт в
        // `last_saved_fp` при выборе чата — иначе выбор выглядит как
        // правка и двигает чат наверх списка.
        let fp = syn_chat::registry::state_fingerprint(&title, &msgs, &params);
        if ctx.last_saved_fp.get_untracked() == fp {
            return;
        }

        syn_chat::registry::refresh_active_preview(&msgs);
        if let Some(stored) = syn_chat::registry::snapshot_current() {
            syn_chat::storage::save(&stored);
            ctx.last_saved_fp.set(fp);
        }
    });

    // Эффект 2 — system_prompt → AppConfig.syn_chat_system_prompt.
    let ctx2 = ctx.clone();
    create_effect(move || {
        let prompt = ctx2.system_prompt.get();
        if ctx2.loading.get_untracked() {
            return;
        }
        let mut cfg = config::AppConfig::load();
        if cfg.syn_chat_system_prompt != prompt {
            cfg.syn_chat_system_prompt = prompt;
            cfg.save();
        }
    });
}

/// Effect автосохранения сессии node-editor'а на диск.
///
/// Подписывается на `EditorWorkspace.tabs` / `active` +
/// все per-tab сигналы (через `persist::subscribe_tab_signals`). На любое
/// изменение пересобирает `WorkspaceState` через `templates::convert::snapshot`
/// для каждой вкладки и атомарно пишет `~/.config/synthos/workspace.json`.
///
/// Тяжёлые объекты (Transcriber/Pipeline/AudioPlayer/threads) сериализации
/// не подлежат — после restore они пересоздаются дефолтно (см.
/// `apply_to_ctx`), модели грузятся лениво на первый Play.
///
/// Fingerprint используется для skip-if-unchanged: вариант RwSignal'а,
/// который выглядит «изменённым», но даёт идентичный JSON, не вызывает
/// disk-write (пример — set с тем же значением через mouse-jitter в pan).
fn install_workspace_autosave() {
    use pages::node_editor::persist;
    use pages::node_editor::tabs::EditorWorkspace;
    use templates::convert::snapshot;

    let last_fp = use_signal(0_u64);

    create_effect(move || {
        let ws = use_context::<EditorWorkspace>();
        let tabs = ws.tabs.get();
        let active = ws.active.get();

        // Подписаться на все сигналы каждой вкладки. После этого собираем
        // снимок через get_untracked — повторных подписок не возникает.
        for tab in tabs.iter() {
            persist::subscribe_tab_signals(&tab.ctx);
            // Флаги агентской вкладки живут на OpenTab, не в ctx — reveal
            // из чата тоже должен пересохранить workspace.
            let _ = tab.hidden.get();
            let _ = tab.agent_chat.get();
            let _ = tab.created_at.get();
        }

        let mut tab_states = Vec::with_capacity(tabs.len());
        for tab in tabs.iter() {
            let (nodes, conns, viewport) = snapshot(&tab.ctx);
            tab_states.push(persist::TabState {
                id: tab.id.0,
                title: tab.title.get_untracked(),
                source: tab.source.get_untracked(),
                agent_chat: tab.agent_chat.get_untracked(),
                hidden: tab.hidden.get_untracked(),
                nodes,
                connections: conns,
                viewport,
                created_at: tab.created_at.get_untracked(),
            });
        }

        let state = persist::WorkspaceState {
            tabs: tab_states,
            active: active.map(|id| id.0),
            next_tab_id: ws.next_tab_id.get_untracked(),
        };

        // Skip-if-unchanged по hash JSON-байтов: при первом запуске сравним
        // с пустым (0) и сохраним; в худшем случае одна лишняя запись.
        let json = match serde_json::to_string_pretty(&state) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "workspace serialize failed");
                return;
            }
        };
        let fp = workspace_fingerprint(&json);
        if last_fp.get_untracked() == fp {
            return;
        }
        last_fp.set(fp);
        if let Err(e) = persist::save(&state) {
            tracing::warn!(error = %e, "workspace save failed");
        }
    });
}

/// Маппинг строки из `AppConfig.qwen36_attn_mode` в [`synaptix::facade::llm::FlashAttnMode`].
/// Невалидные/legacy-значения (включая старые bool `"true"`/`"false"`) трактуются
/// как `Fa4` — наибыстрейший путь.

/// Парсит строковый layer-sync режим из конфига (`"auto"`/`"on"`/`"off"`).
/// Невалидные значения → `Auto` (безопасный default).

fn workspace_fingerprint(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    let v = h.finish();
    // 0 зарезервирован как «ни разу не считали»: гарантируем ненулевой.
    if v == 0 {
        1
    } else {
        v
    }
}

/// Effect, отвечающий за авто-старт записи после загрузки ASR-модели.
///
/// Когда пользователь кликает FAB при выгруженной модели, `voice_start()`
/// ставит `voice.pending_record_start=true` и вызывает `load_selected_model()`.
/// Этот effect наблюдает за `asr_loading`/`asr_loaded_name` и при успешном
/// завершении загрузки автоматически дёргает `voice_start()` снова — на этот
/// раз он попадает в ветку «модель уже загружена» и стартует запись.
///
/// При ошибке загрузки (`asr_loaded_name` остаётся None и `asr_loading` стал
/// false) — просто сбрасываем pending; ошибка уже выставлена в `audio.error`.
fn install_voice_auto_record(ctx: &AppCtx) {
    let voice = ctx.voice;
    let audio = ctx.audio.clone();
    create_effect(move || {
        let pending = voice.pending_record_start.get();
        if !pending {
            return;
        }
        // Подписываемся на оба сигнала: загрузка идёт → не делаем ничего.
        let loading = audio.asr_loading.get();
        let loaded_name = audio.asr_loaded_name.get();
        if loading {
            return;
        }
        // Загрузка завершилась (успешно или с ошибкой). Сбрасываем pending,
        // и при наличии модели — стартуем запись.
        voice.pending_record_start.set(false);
        if loaded_name.is_some() {
            agent::audio::voice_start();
        }
    });
}

fn build_app() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let top_router = ctx.router.clone();

    let routes = RouterView::new(top_router)
        .route("syn_chat", || Box::new(pages::syn_chat::view()))
        .route("code", || Box::new(pages::code_editor::view()))
        .route("nodes", || Box::new(pages::node_editor::view()))
        .route("notes", || Box::new(pages::notes::view()))
        .route("syn_explorer", || Box::new(pages::syn_explorer::view()))
        .route("huggingface", || Box::new(pages::huggingface::view()))
        .route("settings", || Box::new(pages::settings::view()));

    // Padding и rounded окна управляются MSS-pseudo `:window-maximized`
    // (см. styles/layout/shell.mss). В restored — `.window-backdrop` имеет
    // padding 30px вокруг `.shell`, в maximize — padding/rounded обнуляются
    // с плавным transition.
    // Voice FAB живёт собственным `Portal::BottomEnd` поверх любой страницы;
    // окно распознавания — отдельный `Portal::Center, modal`. Оба позиционируются
    // overlay-слоем самим Portal'ом — обходя layout shell'а, нам достаточно
    // просто включить `voice_fab::view()` в общий Stack-уровень.
    let notification_view = components::notification::view(ctx.notifications.clone());
    // Оболочка обёрнута хоткей-скоупом поиска: Ctrl+K / Ctrl+F работают на
    // любой странице, а сама панель живёт отдельным overlay-слоем.
    let shell = search::hotkey_scope(mgui! {
        DecoratedBox::new().class("window-backdrop") => [
            DecoratedBox::new().clip(true).class("shell") => [
                Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    titlebar::view(),
                    DecoratedBox::new().class("grow").child(mgui! {
                        Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            components::nav_rail::view(),
                            DecoratedBox::new().class("grow").child(routes),
                        ]
                    }),
                    DecoratedBox::new().class("window-statusbar"),
                ]
            ]
        ]
    });
    // Диалоги закрытия плиток рейла живут здесь, а не на страницах:
    // «Закрыть» в контекстном меню плитки доступно с любого маршрута.
    mgui! {
        Stack::new().clip(false) => [
            shell,
            components::template_picker::view(),
            components::graph_close_dialog::view(),
            pages::syn_chat::archive_dialog::view(),
            pages::syn_chat::clear_dialog::view(),
            components::voice_fab::view(),
            search::panel::view(),
            notification_view,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Восстанавливаем только известный маршрут: имя из старой версии
    /// (страница переименована/удалена) не должно уводить старт в никуда.
    #[test]
    fn restorable_route_filters_unknown() {
        let mut cfg = AppConfig::default();
        assert_eq!(restorable_route(&cfg), None);
        cfg.last_route = Some("code".into());
        assert_eq!(restorable_route(&cfg), Some("code"));
        cfg.last_route = Some("music_studio_legacy".into());
        assert_eq!(restorable_route(&cfg), None);
    }
}
