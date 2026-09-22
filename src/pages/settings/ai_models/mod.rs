//! Settings → AI Models — режим работы каждой модели.
//!
//! У модели два режима. `Optimal` — по умолчанию: настройки берёт движок,
//! он знает, какие пути у какой архитектуры выверены замерами. `Custom` —
//! те же настройки, но поверх ложатся ручные правки; тронутое перебивает
//! выверенное, остальное остаётся как в `Optimal`.
//!
//! Глобальных тумблеров тут больше нет: ровно они и мешали — настройка,
//! хорошая для одной архитектуры, применялась ко всем.

use std::path::{Path, PathBuf};

use syngui::prelude::*;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};
use syngui::widgets::input::toggle::Toggle;

use crate::config::ModelProfileConfig;
use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::settings::widgets::{
    compute_dtype_dropdown, kv_dtype_dropdown, row_tip, section_card, storage_dtype_dropdown,
};
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
                acestep_paths_card(),
            ]);

        Padding::all(32.0).child(
            DecoratedBox::new()
                .class("settings-page models-page ai-models-page")
                .child(body),
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Список моделей
// ─────────────────────────────────────────────────────────────────────────────

struct Found {
    path: PathBuf,
    name: String,
    arch: String,
    size: u64,
}

/// Определение архитектуры лезет в бандл за `config.json`, поэтому ответ
/// запоминается: список перерисовывается на каждое движение сигнала, а файлы
/// на диске за это время не меняются.
fn arch_label(path: &Path) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = cache.lock() {
        if let Some(v) = map.get(path) {
            return v.clone();
        }
    }
    let label = match synaptix::facade::arch::detect_llm_arch(path) {
        Ok(a) => format!("{a:?}"),
        Err(_) => String::new(),
    };
    if let Ok(mut map) = cache.lock() {
        map.insert(path.to_path_buf(), label.clone());
    }
    label
}

/// Бандлы и HF-каталоги, которые движок берётся грузить как LLM. Глубина 2:
/// каталог моделей обычно разложен по вендорам.
fn scan_llm_models(dir: &Path) -> Vec<Found> {
    fn scan(dir: &Path, depth: usize, out: &mut Vec<Found>) {
        if depth > 2 || out.len() > 64 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.join("config.json").exists() {
                    push(&p, 0, out);
                } else {
                    scan(&p, depth + 1, out);
                }
            } else if p
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("syn") || x.eq_ignore_ascii_case("gguf"))
            {
                // `.gguf` движок грузит напрямую (llama/qwen2/qwen3/gemma3/
                // gemma4): конфиг и токенизатор синтезируются из метаданных.
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                push(&p, size, out);
            }
        }
    }
    fn push(p: &Path, size: u64, out: &mut Vec<Found>) {
        let arch = arch_label(p);
        if arch.is_empty() {
            return;
        }
        out.push(Found {
            name: p
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string()),
            path: p.to_path_buf(),
            arch,
            size,
        });
    }
    let mut found = Vec::new();
    scan(dir, 0, &mut found);
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn models_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let dir_sig = ctx.models_dir;
    let profiles = ctx.model_profiles;

    // Строки уходят одной колонкой, а не списком виджетов: `Reactive` кладёт
    // всех своих детей в одну точку — несколько моделей нарисовались бы
    // друг поверх друга.
    let rows = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let dir = crate::config::resolve_models_dir(&dir_sig.get());
        let found = scan_llm_models(&dir);
        if found.is_empty() {
            return vec![row_tip(
                MI_FOLDER_OPEN,
                tr!("settings.ai_models.models.empty"),
                tr!("settings.ai_models.models.empty.tip"),
                Box::new(Text::new(dir.display().to_string()).class("settings-row-desc")),
            )];
        }
        let profs = profiles.get();
        vec![Box::new(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(found.iter().map(|m| model_block(m, &profs))),
        ) as Box<dyn Widget>]
    });

    section_card(tr!("settings.ai_models.section.models"), vec![Box::new(rows)])
}

/// Строка модели плюс — в режиме `custom` — блок переопределений под ней.
fn model_block(
    m: &Found,
    profiles: &std::collections::BTreeMap<String, ModelProfileConfig>,
) -> Box<dyn Widget> {
    let key = m.path.display().to_string();
    let profile = crate::config::model_profile_of(profiles, &m.path);
    let resolved = profile.resolve(&m.path);

    let items = vec![
        DropdownItem::new("optimal", tr!("settings.ai_models.mode.optimal")),
        DropdownItem::new("custom", tr!("settings.ai_models.mode.custom")),
    ];
    let mode_key = key.clone();
    let mode: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(profile.mode.clone())
            .on_change(move |s| {
                let mode = s.to_string();
                update_profile(&mode_key, move |p| p.mode = mode.clone());
            })
            .class("models-active-dropdown"),
    );

    let load_key = m.path.clone();
    let load: Box<dyn Widget> = Box::new(
        Button::new(tr!("settings.ai_models.apply.button"))
            .icon(MI_AUTORENEW)
            .on_click(move || load_model(load_key.clone()))
            .class("ai-models-apply-button"),
    );

    let control = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![mode, load]);

    let subtitle = if m.size > 0 {
        format!("{} · {}", m.arch, crate::models::human_bytes(m.size))
    } else {
        m.arch.clone()
    };

    let head = row_tip(
        MI_MEMORY,
        format!("{} · {subtitle}", m.name),
        tr!("settings.ai_models.model.tip", path = m.path.display()),
        Box::new(control),
    );

    if !profile.is_custom() {
        return head;
    }

    let mut rows: Vec<Box<dyn Widget>> = vec![head];
    rows.extend(overrides(&key, &profile, &resolved));
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

/// Переопределения одной модели. Значение в контроле — своё, если задано,
/// иначе выверенное движком: пользователь видит, от чего отталкивается.
fn overrides(
    key: &str,
    profile: &ModelProfileConfig,
    resolved: &crate::config::ResolvedProfile,
) -> Vec<Box<dyn Widget>> {
    let p = &resolved.policy;
    let k = key.to_string();

    let weights = {
        let k = k.clone();
        let cur = profile
            .weights_storage
            .clone()
            .unwrap_or_else(|| crate::config::dtype_name(p.weights_storage).to_string());
        storage_dtype_dropdown(cur, move |s| {
            let k = k.clone();
            update_profile(&k, move |p| p.weights_storage = Some(s.clone()));
        })
    };
    let compute = {
        let k = k.clone();
        let cur = profile
            .compute
            .clone()
            .unwrap_or_else(|| crate::config::dtype_name(p.compute).to_string());
        compute_dtype_dropdown(cur, move |s| {
            let k = k.clone();
            update_profile(&k, move |p| p.compute = Some(s.clone()));
        })
    };
    let kv = {
        let k = k.clone();
        let cur = profile.kv_dtype.clone().unwrap_or_else(|| p.kv_dtype.name().to_string());
        kv_dtype_dropdown(cur, move |s| {
            let k = k.clone();
            update_profile(&k, move |p| p.kv_dtype = Some(s.clone()));
        })
    };
    let lm_head = {
        let k = k.clone();
        let cur = profile
            .lm_head_storage
            .clone()
            .unwrap_or_else(|| crate::config::dtype_name(p.lm_head_storage).to_string());
        storage_dtype_dropdown(cur, move |s| {
            let k = k.clone();
            update_profile(&k, move |p| p.lm_head_storage = Some(s.clone()));
        })
    };
    let embed = {
        let k = k.clone();
        let cur = profile
            .embed_storage
            .clone()
            .unwrap_or_else(|| crate::config::dtype_name(p.embed_storage).to_string());
        storage_dtype_dropdown(cur, move |s| {
            let k = k.clone();
            update_profile(&k, move |p| p.embed_storage = Some(s.clone()));
        })
    };

    let graph = {
        let k = k.clone();
        let on = profile.graph_decode.unwrap_or(resolved.graph_decode);
        Box::new(Toggle::with_state(on).on_change(move |v| {
            let k = k.clone();
            update_profile(&k, move |p| p.graph_decode = Some(v));
        })) as Box<dyn Widget>
    };
    let spec = {
        let k = k.clone();
        let on = profile.speculation.unwrap_or(resolved.speculation);
        Box::new(Toggle::with_state(on).on_change(move |v| {
            let k = k.clone();
            update_profile(&k, move |p| p.speculation = Some(v));
        })) as Box<dyn Widget>
    };
    let sync = {
        let k = k.clone();
        let items = vec![
            DropdownItem::new("auto", tr!("settings.ai_models.layer_sync.auto")),
            DropdownItem::new("on", tr!("settings.ai_models.layer_sync.on")),
            DropdownItem::new("off", tr!("settings.ai_models.layer_sync.off")),
        ];
        let cur = profile
            .layer_sync
            .clone()
            .unwrap_or_else(|| layer_sync_name(resolved.layer_sync).to_string());
        Box::new(
            Dropdown::with_items(items)
                .selected(cur)
                .on_change(move |s| {
                    let k = k.clone();
                    let v = s.to_string();
                    update_profile(&k, move |p| p.layer_sync = Some(v.clone()));
                })
                .class("models-active-dropdown"),
        ) as Box<dyn Widget>
    };

    let reset: Box<dyn Widget> = {
        let k = k.clone();
        Box::new(
            Button::new(tr!("settings.ai_models.reset.button"))
                .on_click(move || {
                    let k = k.clone();
                    update_profile(&k, |p| {
                        let mode = p.mode.clone();
                        *p = ModelProfileConfig { mode, ..Default::default() };
                    });
                })
                .class("ai-models-apply-button"),
        )
    };

    vec![
        row_tip(
            MI_TUNE,
            tr!("settings.ai_models.weights"),
            tr!("settings.ai_models.weights.desc"),
            weights,
        ),
        row_tip(
            MI_SPEED,
            tr!("settings.ai_models.compute"),
            tr!("settings.ai_models.compute.desc"),
            compute,
        ),
        row_tip(
            MI_MEMORY,
            tr!("settings.ai_models.kv"),
            tr!("settings.ai_models.kv.desc"),
            kv,
        ),
        row_tip(
            MI_TUNE,
            tr!("settings.ai_models.lm_head"),
            tr!("settings.ai_models.lm_head.desc"),
            lm_head,
        ),
        row_tip(
            MI_TUNE,
            tr!("settings.ai_models.embed"),
            tr!("settings.ai_models.embed.desc"),
            embed,
        ),
        row_tip(
            MI_AUTO_AWESOME,
            tr!("settings.ai_models.cuda_graph"),
            tr!("settings.ai_models.cuda_graph.desc"),
            graph,
        ),
        row_tip(
            MI_BOLT,
            tr!("settings.ai_models.speculation"),
            tr!("settings.ai_models.speculation.desc"),
            spec,
        ),
        row_tip(
            MI_MEMORY,
            tr!("settings.ai_models.layer_sync"),
            tr!("settings.ai_models.layer_sync.desc"),
            sync,
        ),
        row_tip(
            MI_AUTORENEW,
            tr!("settings.ai_models.reset"),
            tr!("settings.ai_models.reset.desc"),
            reset,
        ),
    ]
}

fn layer_sync_name(mode: synaptix::facade::llm::LayerSyncMode) -> &'static str {
    use synaptix::facade::llm::LayerSyncMode as M;
    match mode {
        M::Auto => "auto",
        M::Off => "off",
        M::On => "on",
    }
}

fn update_profile<F: FnOnce(&mut ModelProfileConfig)>(key: &str, f: F) {
    let ctx = use_context::<AppCtx>();
    let key = key.to_string();
    ctx.model_profiles.update(move |m| {
        f(m.entry(key).or_default());
    });
}

/// Загрузить модель с её профилем: рантайм-часть уходит в движок, политика
/// квантования — в загрузчик весов.
fn load_model(path: PathBuf) {
    let ctx = use_context::<AppCtx>();
    if !path.exists() {
        ctx.notifications
            .warning(tr!("settings.ai_models.apply.not_found", path = path.display()));
        return;
    }
    let policy =
        crate::config::resolve_model_profile(&ctx.model_profiles.get_untracked(), &path).policy;
    let registry = use_context::<SynModelRegistry>();
    registry.unload();
    registry.load(path, policy);
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

    let field = syngui::widgets::input::TextField::with_text(sig.get_untracked())
        .placeholder(crate::config::default_models_dir())
        .on_change(move |s| sig.set(s.to_string()))
        .class("models-path-field");

    let browse = Button::new(tr!("app.browse"))
        .icon(MI_FOLDER_OPEN)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(tr!("settings.ai_models.section.models_dir"));
            if let Some(p) = dlg.pick_folder() {
                sig.set(p.display().to_string());
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

/// Пути к бандлам ACE-Step: их тянут все ноды семейства, поэтому путь один
/// на приложение, а не на ноду.
fn acestep_paths_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let xl_sig = ctx.acestep_xl_bundle_path;
    let vae_sig = ctx.acestep_vae_bundle_path;
    section_card(
        tr!("settings.ai_models.section.acestep"),
        vec![
            row_tip(
                MI_FOLDER_OPEN,
                tr!("settings.ai_models.acestep_xl"),
                tr!("settings.ai_models.acestep_xl.desc"),
                bundle_picker(xl_sig, tr!("settings.ai_models.acestep_xl.dialog_title")),
            ),
            row_tip(
                MI_FOLDER_OPEN,
                tr!("settings.ai_models.acestep_vae"),
                tr!("settings.ai_models.acestep_vae.desc"),
                bundle_picker(vae_sig, tr!("settings.ai_models.acestep_vae.dialog_title")),
            ),
        ],
    )
}

fn bundle_picker(sig: RwSignal<Option<String>>, title: impl Into<String>) -> Box<dyn Widget> {
    let title = title.into();
    let pick_btn: Box<dyn Widget> = Box::new(
        Button::new(tr!("app.browse"))
            .icon(MI_FOLDER_OPEN)
            .on_click(move || {
                let dlg = rfd::FileDialog::new()
                    .add_filter(tr!("settings.ai_models.bundle.filter"), &["syn", "gguf"])
                    .set_title(title.clone());
                if let Some(p) = dlg.pick_file() {
                    sig.set(Some(p.to_string_lossy().to_string()));
                }
            })
            .class("ai-models-apply-button"),
    );
    let clear_btn: Box<dyn Widget> = Box::new(
        Button::new(tr!("settings.ai_models.bundle.clear"))
            .on_click(move || sig.set(None))
            .class("ai-models-apply-button"),
    );
    let filename = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let label: String = sig
            .get()
            .map(|s| {
                std::path::Path::new(&s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(s)
            })
            .unwrap_or_else(|| tr!("settings.ai_models.bundle.none"));
        vec![Box::new(Text::new(label).class("settings-row-desc")) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![pick_btn, clear_btn, Box::new(filename)]),
    )
}
