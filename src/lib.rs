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
pub mod icons;
pub mod kb;
pub mod logging;
pub mod metrics;
pub mod migrate;
pub mod pages;
pub mod skills;
pub mod styles;
pub mod syn_chat;
pub mod templates;

use components::titlebar;
use config::AppConfig;
use context::{
    AppCtx, GeneralCtx, ToolsCtx, VoiceFabCtx, VoiceHistoryCtx, INITIAL_ROUTE,
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

    // Регистрация sqlite-vec auto-extension: применяется глобально ко
    // всем последующим Connection'ам в процессе. Без feature
    // `kb-sqlite-vec` — no-op, KB остаётся на full-scan.
    kb::init_sqlite_vec();

    let (theme_mss, ctx) = build_context();

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
        .run(move |_| {
            provide_context(ctx.clone());
            provide_context(build_code_editor_ctx());
            provide_context(pages::node_editor::tabs::EditorWorkspace::new_or_restore());
            provide_context(pages::syn_explorer::SynExplorerCtx::new(&AppConfig::load()));
            provide_context(syn_chat::SynChatCtx::new());
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
            install_voice_auto_record(&ctx);
            metrics::system::start_sampler(ctx.metrics.clone());
            syn_chat::registry::load_all();
            Box::new(build_app())
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
            provide_context(build_code_editor_ctx());
            provide_context(pages::node_editor::tabs::EditorWorkspace::new_or_restore());
            provide_context(pages::syn_explorer::SynExplorerCtx::new(&AppConfig::load()));
            provide_context(syn_chat::SynChatCtx::new());
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
            install_voice_auto_record(&ctx);
            metrics::system::start_sampler(ctx.metrics.clone());
            syn_chat::registry::load_all();
            Box::new(build_app())
        });
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
            });
        }
    }
    let active_idx = saved
        .active_code_session
        .filter(|&i| i < sessions_cfg.len())
        .or_else(|| if sessions_cfg.is_empty() { None } else { Some(0) });
    pages::code_editor::state::CodeEditorCtx::new(sessions_cfg, active_idx)
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
    let voice_history = VoiceHistoryCtx::new();
    // Загрузить индекс записей с диска (Sprint 2). Не паническая ошибка —
    // отсутствие файла или его повреждение → пустой список и работа продолжается.
    voice_history
        .recordings
        .set(pages::voice_history::storage::load_index());
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
    let syn_chat_quant = use_signal(saved.syn_chat_quant.clone());
    let qwen36_attn_mode = use_signal(saved.qwen36_attn_mode.clone());
    let qwen36_graph_decode = use_signal(saved.qwen36_graph_decode);
    let qwen36_mtp = use_signal(saved.qwen36_mtp);
    let muse_dflash = use_signal(saved.muse_dflash);
    let qwen36_la_fused = use_signal(saved.qwen36_la_fused);
    let qwen36_gdr_fused = use_signal(saved.qwen36_gdr_fused);
    let qwen36_prefill_chunk = use_signal(saved.qwen36_prefill_chunk);
    let qwen36_layer_sync = use_signal(saved.qwen36_layer_sync.clone());
    let qwen36_nvfp4_mma = use_signal(saved.qwen36_nvfp4_mma);
    let qwen36_nvfp4_gemv = use_signal(saved.qwen36_nvfp4_gemv);
    let acestep_xl_bundle_path = use_signal(saved.acestep_xl_bundle_path.clone());
    let acestep_vae_bundle_path = use_signal(saved.acestep_vae_bundle_path.clone());

    // Применить режим к глобальному runtime-state llm-qwen36 при старте.
    // Сигнал-driven sync с runtime — ниже в `create_effect` после `ctx`.
    synaptix::facade::llm::set_flash_attn_mode(parse_attn_mode(&saved.qwen36_attn_mode));
    synaptix::facade::llm::set_graph_decode_enabled(saved.qwen36_graph_decode);
    synaptix::facade::llm::set_mtp_enabled(saved.qwen36_mtp);
    synaptix::facade::llm::set_dflash_enabled(saved.muse_dflash);
    synaptix::facade::llm::set_la_prep_fused_disabled(!saved.qwen36_la_fused);
    synaptix::facade::llm::set_gdr_fused_disabled(!saved.qwen36_gdr_fused);
    synaptix::facade::llm::set_prefill_chunk_size(saved.qwen36_prefill_chunk);
    synaptix::facade::llm::set_layer_sync_mode(parse_layer_sync_mode(&saved.qwen36_layer_sync));

    let ctx = AppCtx {
        theme_key,
        theme_mss,
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
        voice_history,
        metrics,
        tools,
        terminal_font_family,
        terminal_font_size,
        code_editor_font_family,
        code_editor_font_size,
        kb,
        notifications,
        syn_chat_quant,
        qwen36_attn_mode,
        qwen36_graph_decode,
        qwen36_mtp,
        muse_dflash,
        qwen36_la_fused,
        qwen36_gdr_fused,
        qwen36_prefill_chunk,
        qwen36_layer_sync,
        qwen36_nvfp4_mma,
        qwen36_nvfp4_gemv,
        acestep_xl_bundle_path,
        acestep_vae_bundle_path,
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
            let base = match theme_data::find(&theme_key.get()) {
                Some(t) => t.to_mss(),
                None => theme_data::default_theme().to_mss(),
            };
            let vf = voice_family.get();
            let vf = if vf.trim().is_empty() { "sans-serif".to_string() } else { vf };
            let vs = voice_size.get();
            let ef = editor_family.get();
            let ef = if ef.trim().is_empty() { "monospace".to_string() } else { ef };
            let es = editor_size.get();
            theme_mss.set(format!(
                "{base}\n:root {{\n  --voice-font-family: {vf};\n  --voice-font-size: {vs:.0}px;\n  --code-editor-font-family: {ef};\n  --code-editor-font-size: {es:.0}px;\n}}\n"
            ));
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
    let syn_chat_quant = ctx.syn_chat_quant;
    let qwen36_attn_mode = ctx.qwen36_attn_mode;
    let qwen36_graph_decode = ctx.qwen36_graph_decode;
    let qwen36_mtp = ctx.qwen36_mtp;
    let muse_dflash = ctx.muse_dflash;
    let qwen36_la_fused = ctx.qwen36_la_fused;
    let qwen36_gdr_fused = ctx.qwen36_gdr_fused;
    let qwen36_prefill_chunk = ctx.qwen36_prefill_chunk;
    let qwen36_layer_sync = ctx.qwen36_layer_sync;
    let qwen36_nvfp4_mma = ctx.qwen36_nvfp4_mma;
    let qwen36_nvfp4_gemv = ctx.qwen36_nvfp4_gemv;
    let acestep_xl_bundle_path = ctx.acestep_xl_bundle_path;
    let acestep_vae_bundle_path = ctx.acestep_vae_bundle_path;
    let code = use_context::<pages::code_editor::state::CodeEditorCtx>();
    let syn = use_context::<pages::syn_explorer::state::SynExplorerCtx>();
    let hf = use_context::<pages::huggingface::HuggingFaceCtx>();

    create_effect(move || {
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
            })
            .collect();
        let active_idx = active_id.and_then(|id| sessions.iter().position(|s| s.id == id));

        let cfg = AppConfig {
            theme: theme_key.get(),
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
            skills_active: skills_active.get(),
            audio_models: audio_models.get(),
            selected_audio_model: selected_audio_model.get(),
            audio_autostart: false,
            code_sessions: sessions_cfg,
            active_code_session: active_idx,
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
            hf_cache_dir: hf.cache_dir.get(),
            hf_concurrent_downloads: hf.concurrent_limit.get(),
            hf_segments_per_file: hf.segments_per_file.get(),
            hf_speed_limit_mbps: hf.speed_limit_mbps.get(),
            hf_skip_unwanted_formats: hf.skip_unwanted_formats.get(),
            hf_gguf_support: hf.gguf_support.get(),

            hf_token: hf.token.get(),
            syn_chat_quant: syn_chat_quant.get(),
            qwen36_attn_mode: qwen36_attn_mode.get(),
            qwen36_graph_decode: qwen36_graph_decode.get(),
            qwen36_mtp: qwen36_mtp.get(),
            muse_dflash: muse_dflash.get(),
            qwen36_la_fused: qwen36_la_fused.get(),
            qwen36_gdr_fused: qwen36_gdr_fused.get(),
            qwen36_prefill_chunk: qwen36_prefill_chunk.get(),
            qwen36_layer_sync: qwen36_layer_sync.get(),
            qwen36_nvfp4_mma: qwen36_nvfp4_mma.get(),
            qwen36_nvfp4_gemv: qwen36_nvfp4_gemv.get(),
            acestep_xl_bundle_path: acestep_xl_bundle_path.get(),
            acestep_vae_bundle_path: acestep_vae_bundle_path.get(),
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

    // Sync UI dropdown → глобальный runtime-флаг llm-qwen36. Применяется
    // мгновенно к следующему `GatedAttention::forward`, без reload модели
    // и без пересоздания KV-кэша.
    create_effect(move || {
        let mode_str = qwen36_attn_mode.get();
        synaptix::facade::llm::set_flash_attn_mode(parse_attn_mode(&mode_str));
    });
    // Phase D toggle: применяется к следующему `generate(...)` /
    // `generate_streaming(...)` без reload. Требует device, созданный через
    // `CudaDevice::new_with_stream` (см. `syn_chat::model_registry`).
    create_effect(move || {
        let on = qwen36_graph_decode.get();
        synaptix::facade::llm::set_graph_decode_enabled(on);
    });
    create_effect(move || {
        let on = qwen36_mtp.get();
        synaptix::facade::llm::set_mtp_enabled(on);
    });
    // DFlash-драфтер Muse Glimmer: флаг читается и при загрузке модели
    // (подключать ли драфтер), и на каждом ответе (использовать ли его).
    create_effect(move || {
        let on = muse_dflash.get();
        synaptix::facade::llm::set_dflash_enabled(on);
    });
    // Phase B-1/B-2 fused kernels — runtime toggle для linear_attn слоёв
    // (применяется в `forward_raw_linear_attn_step`). Без reload.
    create_effect(move || {
        synaptix::facade::llm::set_la_prep_fused_disabled(!qwen36_la_fused.get());
    });
    create_effect(move || {
        synaptix::facade::llm::set_gdr_fused_disabled(!qwen36_gdr_fused.get());
    });
    // Prefill chunk: применяется к следующему `prefill_chunked` (тот читает
    // `runtime_flags::prefill_chunk_size()` на каждом запуске).
    create_effect(move || {
        synaptix::facade::llm::set_prefill_chunk_size(qwen36_prefill_chunk.get());
    });
    // Layer-sync: применяется к следующему `Qwen36Model::forward` (тот читает
    // `runtime_flags::layer_sync_should_apply(T)` на каждом запуске).
    create_effect(move || {
        let mode_str = qwen36_layer_sync.get();
        synaptix::facade::llm::set_layer_sync_mode(parse_layer_sync_mode(&mode_str));
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
/// `params`. Fingerprint включает params чтобы изменение слайдеров вызывало
/// disk write через snapshot_current (там params сохраняются в StoredChat).
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

        let fp_msgs = syn_chat::registry::fingerprint(&title, &msgs);
        // Подмешиваем хэш params в fingerprint, чтобы изменение слайдеров
        // тоже триггерило save.
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        fp_msgs.hash(&mut h);
        // serde_json::to_vec для f32/u32 — стабильный hash без NaN-issues.
        if let Ok(bytes) = serde_json::to_vec(&params) {
            bytes.hash(&mut h);
        }
        let fp = h.finish();
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
        }

        let mut tab_states = Vec::with_capacity(tabs.len());
        for tab in tabs.iter() {
            let (nodes, conns, viewport) = snapshot(&tab.ctx);
            tab_states.push(persist::TabState {
                id: tab.id.0,
                title: tab.title.get_untracked(),
                source: tab.source.get_untracked(),
                nodes,
                connections: conns,
                viewport,
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
fn parse_attn_mode(s: &str) -> synaptix::facade::llm::FlashAttnMode {
    s.parse().unwrap_or(synaptix::facade::llm::FlashAttnMode::Fa4)
}

/// Парсит строковый layer-sync режим из конфига (`"auto"`/`"on"`/`"off"`).
/// Невалидные значения → `Auto` (безопасный default).
fn parse_layer_sync_mode(s: &str) -> synaptix::facade::llm::LayerSyncMode {
    s.parse().unwrap_or(synaptix::facade::llm::LayerSyncMode::Auto)
}

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
        .route("voice_history", || Box::new(pages::voice_history::view()))
        .route("code", || Box::new(pages::code_editor::view()))
        .route("nodes", || Box::new(pages::node_editor::view()))
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
    mgui! {
        Stack::new().clip(false) => [
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
            ],
            components::template_picker::view(),
            components::voice_fab::view(),
            notification_view,
        ]
    }
}
