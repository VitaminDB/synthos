//! Tool `pipelines` — доступ агента Syn-чата к нодовому редактору.
//!
//! Агент работает со СЛУЖЕБНОЙ вкладкой редактора (одна на чат, скрытая из
//! полосы — см. `EditorWorkspace::ensure_agent_tab`): открывает в ней шаблон,
//! декларативно правит граф (Template-JSON), читает snapshot и сохраняет
//! результат как кастомный шаблон. Запуск прогона (`action=run`) живёт в
//! `pipelines_run` — он завязан на жизненный цикл LLM и исполняется
//! обёрткой в agent-loop, а не здесь.
//!
//! Действия:
//! - `list` — шаблоны (builtin + custom), вложения текущего чата (для
//!   `attachment:`-ссылок) и состояние служебной вкладки.
//! - `nodes` — схемы видов нод из `registry::REGISTRY`: порты, поля и
//!   пример `state`-JSON (снятый с default_runtime). Без `filter` — компакт-
//!   список, с `filter` — детали совпавших.
//! - `open` — загрузить шаблон в служебную вкладку (replace).
//! - `graph` — snapshot служебной вкладки.
//! - `apply` — правки графа: `graph` (replace/merge Template-JSON),
//!   `set_state` (точечный state ноды), `connect`/`disconnect` (связи).
//!   Строки вида `attachment:<имя|sha-префикс|last>` в state резолвятся в
//!   путь blob'а вложения чата (`blobs::model_path` — формат, читаемый
//!   пайплайнами).
//! - `save_template` — сохранить граф вкладки кастомным шаблоном.
//!
//! Все сигналы — main-thread: каждое действие целиком исполняется в
//! `run_on_main_thread`-замыкании, результат уходит через oneshot
//! (паттерн `kb_search`).

use syngui::tr;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::core::Point;

use crate::agent::state::{AttachmentKind, MsgAttachment};
use crate::context::AppCtx;
use crate::pages::node_editor::registry::{self, NodeCategory};
use crate::pages::node_editor::state::NodeEditorCtx;
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::types::{Connection, NodeKind, PortKind, PortsSpec, PortSide};
use crate::syn_chat::attach::blobs;
use crate::syn_chat::SynChatCtx;
use crate::templates::{self, convert, model::NodeStateData, ConnData, NodeData, Template, TemplateKind};

use super::executor::ToolError;
use crate::agent::json_repair::repair_bracket_tail;

/// Главный entrypoint из `executor::execute`.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let action = v
        .get("action")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("action"))?
        .to_string();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    run_on_main_thread(move || {
        let result = match action.as_str() {
            "list" => list_impl(),
            "nodes" => nodes_impl(&v),
            "open" => open_impl(&v),
            "graph" => graph_impl(),
            "apply" => apply_impl(&v),
            "save_template" => save_template_impl(&v),
            // В основном чате run перехватывается agent-loop'ом ДО
            // tools::execute (`pipeline_run::parse_run_call` — он управляет
            // жизненным циклом LLM). Сюда run доходит только из subagent'а —
            // там прогоны запрещены.
            "run" => Err(
                "action=run is only available to the chat's main agent (not a subagent): \
                 ask the main agent to run the graph"
                    .to_string(),
            ),
            other => Err(format!(
                "unknown action \"{other}\" (list | nodes | open | graph | apply | save_template | run)"
            )),
        };
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| ToolError::Spawn(e.to_string()))?
        .map_err(ToolError::BadArgs)
}

// ─────────────────────────────────────────────────────────────────────────────
// Контекст: чат + служебная вкладка
// ─────────────────────────────────────────────────────────────────────────────

fn active_chat_id() -> Result<String, String> {
    let chat = use_context::<SynChatCtx>();
    chat.active_chat_id
        .get_untracked()
        .ok_or_else(|| "no active chat".to_string())
}

/// Ctx служебной вкладки текущего чата, если она уже создана.
fn agent_ctx() -> Result<Option<NodeEditorCtx>, String> {
    let chat_id = active_chat_id()?;
    let ws = use_context::<EditorWorkspace>();
    let Some(tab_id) = ws.agent_tab_for_chat(&chat_id) else {
        return Ok(None);
    };
    Ok(ws
        .tabs
        .get_untracked()
        .iter()
        .find(|t| t.id == tab_id)
        .map(|t| t.ctx))
}

/// Ctx служебной вкладки, создавая её при необходимости.
fn agent_ctx_ensure(title: &str) -> Result<NodeEditorCtx, String> {
    let chat_id = active_chat_id()?;
    let ws = use_context::<EditorWorkspace>();
    let tab_id = ws.ensure_agent_tab(&chat_id, title);
    let tabs = ws.tabs.get_untracked();
    let tab = tabs
        .iter()
        .find(|t| t.id == tab_id)
        .ok_or_else(|| "the service tab wasn't created".to_string())?;
    if !title.is_empty() {
        tab.title.set(title.to_string());
    }
    Ok(tab.ctx)
}

// ─────────────────────────────────────────────────────────────────────────────
// list
// ─────────────────────────────────────────────────────────────────────────────

/// Описание шаблона в одну строку: длинные тексты builtin-шаблонов (до 600
/// символов) раздували `list` за лимит истории хода — агент видел обрезок
/// без раздела моделей и уходил искать `.syn` через bash.
fn clip_desc(s: &str) -> String {
    const MAX: usize = 90;
    let s = s.replace('\n', " ");
    if s.chars().count() <= MAX {
        return s;
    }
    let cut: String = s.chars().take(MAX).collect();
    let cut = cut.rsplit_once(' ').map(|(h, _)| h.to_string()).unwrap_or(cut);
    format!("{cut}…")
}

fn list_impl() -> Result<String, String> {
    let mut out = String::new();

    out.push_str("--- Pipeline templates ---\n");
    for t in templates::list_all() {
        out.push_str(&format!(
            "{} · \"{}\"{} · nodes: {}{}\n",
            t.id,
            t.name,
            if t.builtin { "" } else { " · custom" },
            t.nodes.len(),
            if t.description.is_empty() {
                String::new()
            } else {
                format!(" · {}", clip_desc(&t.description))
            }
        ));
    }

    // Инвентарь каталогов моделей: отсюда агент берёт пути для чекпойнт-нод
    // (LtxCheckpoint.model_path/gemma_dir, H3Checkpoint.model_path, …) —
    // встроенные шаблоны путей не несут. Каталогов два: основной
    // (Настройки → AI-модели) и кэш HF-загрузок — модели, скачанные со
    // страницы Hugging Face, лежат там, и агент, знающий только первый,
    // честно докладывал «такой модели нет».
    for (label, dir) in models_dirs() {
        let dir_s = dir.display().to_string();
        out.push_str(&format!(
            "--- Models in directory {dir_s} ({label}) --- (paths: {dir_s}/<name>)\n"
        ));
        let inventory = models_inventory(&dir);
        if inventory.is_empty() {
            out.push_str(
                "(empty or the directory doesn't exist — the path is set in Settings → AI Models)\n",
            );
        } else {
            // Имена относительно каталога: полный путь в каждой строке — это
            // ~40 лишних символов × десятки моделей, а собрать его агент
            // может из шапки раздела.
            let prefix = format!("{dir_s}/");
            for l in &inventory {
                out.push_str(l.strip_prefix(&prefix).unwrap_or(l));
                out.push('\n');
            }
        }
    }

    out.push_str("--- Chat attachments (for attachment:<name|sha|last>) ---\n");
    let atts = chat_attachments();
    if atts.is_empty() {
        out.push_str("(none)\n");
    } else {
        for a in &atts {
            out.push_str(&format!(
                "{} · {} · sha:{}\n",
                a.original_name,
                attachment_kind_label(a.kind),
                &a.sha256[..12.min(a.sha256.len())]
            ));
        }
    }

    out.push_str("--- Service tab ---\n");
    match agent_ctx()? {
        // Без state и связей: полный снимок — action=graph. В list он дублировал
        // ответ open и съедал бюджет, вытесняя инвентарь моделей.
        Some(ctx) => out.push_str(&graph_brief(&ctx)?),
        None => out.push_str("(not created yet — open a template via action=open)\n"),
    }
    Ok(out)
}

/// Каталоги, где живут модели: основной (`AppConfig.models_dir`) и кэш
/// HF-загрузок (`AppConfig.hf_cache_dir`). Второй отбрасывается, если
/// совпадает с первым или лежит внутри него — дублировать инвентарь незачем.
fn models_dirs() -> Vec<(&'static str, std::path::PathBuf)> {
    let app = use_context::<AppCtx>();
    let main = crate::config::resolve_models_dir(&app.models_dir.get_untracked());
    // try_: в headless-раннерах (smoke-бинари) HF-контекста нет — тогда
    // второй каталог просто не показываем.
    let Some(hf) =
        syngui::context_provider::try_use_context::<crate::pages::huggingface::HuggingFaceCtx>()
    else {
        return vec![("main", main)];
    };
    let hf_dir = crate::config::resolve_hf_cache_dir(&hf.cache_dir.get_untracked());
    let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (main_c, hf_c) = (canon(&main), canon(&hf_dir));
    let mut dirs = vec![("main", main)];
    if hf_c != main_c && !hf_c.starts_with(&main_c) {
        dirs.push(("HF cache", hf_dir));
    }
    dirs
}

/// Скан каталога моделей (глубина ≤ 2): `.syn` / `.safetensors` / `.gguf`
/// файлы с размером; каталоги с `config.json` — как HF-модели целиком (в
/// них не спускаемся). Потолок — 80 строк, дальше «… и ещё N».
fn models_inventory(dir: &std::path::Path) -> Vec<String> {
    fn scan(dir: &std::path::Path, depth: usize, out: &mut Vec<String>) {
        if depth > 2 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.join("config.json").exists() {
                    out.push(format!("{} · HF model directory", p.display()));
                } else {
                    scan(&p, depth + 1, out);
                }
            } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if matches!(ext.to_lowercase().as_str(), "syn" | "safetensors" | "gguf") {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(format!(
                        "{} · {}",
                        p.display(),
                        crate::models::human_bytes(size)
                    ));
                }
            }
        }
    }
    let mut lines = Vec::new();
    scan(dir, 0, &mut lines);
    lines.sort();
    const CAP: usize = 80;
    if lines.len() > CAP {
        let extra = lines.len() - CAP;
        lines.truncate(CAP);
        lines.push(format!("… and {extra} more files"));
    }
    lines
}

fn attachment_kind_label(k: AttachmentKind) -> &'static str {
    match k {
        AttachmentKind::Image => "image",
        AttachmentKind::Video => "video",
        AttachmentKind::Audio => "audio",
        AttachmentKind::Document => "document",
        AttachmentKind::Other => "file",
    }
}

/// Все вложения user-сообщений активного чата, свежие в конце.
fn chat_attachments() -> Vec<MsgAttachment> {
    let chat = use_context::<SynChatCtx>();
    chat.messages.with_untracked(|msgs| {
        msgs.iter()
            .flat_map(|m| m.attachments.iter().cloned())
            .collect()
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// nodes — схемы видов нод
// ─────────────────────────────────────────────────────────────────────────────

fn kind_slug(kind: NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{kind:?}"))
}

fn port_kind_label(k: PortKind) -> &'static str {
    match k {
        PortKind::Data => "data",
        PortKind::Audio => "audio",
        PortKind::Control => "control",
        PortKind::Text => "text",
        PortKind::Video => "video",
    }
}

fn ports_line(spec: PortsSpec) -> String {
    let pool = spec.pool();
    if pool.is_empty() {
        return "—".to_string();
    }
    let dynamic = matches!(spec, PortsSpec::Dynamic { .. });
    let joined = pool
        .iter()
        .map(|p| format!("{}:{}", p.name, port_kind_label(p.kind)))
        .collect::<Vec<_>>()
        .join(", ");
    if dynamic {
        format!("{joined} (dynamic)")
    } else {
        joined
    }
}

/// Пример state-JSON вида ноды: default_runtime → runtime_to_state.
/// Именно эту форму принимает `apply.set_state[].state` и `NodeData.state`.
/// Расшифровка `*_idx`-полей state: сами дропдауны живут в UI-нодах, а в
/// state попадает голое число — по `{"quant_dit_idx": 0}` агент не мог знать,
/// что 0 это nvfp4, и оставлял выбор точности случайным. Таблица зеркалит
/// `*_OPTIONS`-константы нод.
fn enum_hints(kind: NodeKind) -> Vec<(&'static str, &'static [&'static str])> {
    use crate::pages::node_editor::nodes::{
        acestep, asr_gigaam, ffmpeg_player, llm, ltx, minimax_h3, omnivoice, sortformer_diarizer,
        syn_checkpoint, vibevoice, voxcpm2,
    };
    match kind {
        NodeKind::SynCheckpoint => vec![
            ("device_idx", syn_checkpoint::DEVICE_PREF_OPTIONS),
            ("storage_idx", syn_checkpoint::STORAGE_PREF_OPTIONS),
            ("compute_idx", syn_checkpoint::COMPUTE_PREF_OPTIONS),
        ],
        NodeKind::SortformerDiarizer => vec![
            ("device_idx", sortformer_diarizer::DEVICE_OPTIONS),
            ("storage_idx", sortformer_diarizer::STORAGE_OPTIONS),
            ("compute_idx", sortformer_diarizer::COMPUTE_OPTIONS),
        ],
        NodeKind::AsrGigaam => vec![
            ("device_idx", asr_gigaam::DEVICE_OPTIONS),
            ("storage_idx", asr_gigaam::STORAGE_OPTIONS),
            ("compute_idx", asr_gigaam::COMPUTE_OPTIONS),
        ],
        NodeKind::OmniVoice => vec![
            ("device_idx", omnivoice::DEVICE_OPTIONS),
            ("storage_idx", omnivoice::STORAGE_OPTIONS),
            ("compute_idx", omnivoice::COMPUTE_OPTIONS),
        ],
        NodeKind::VibeVoice => vec![
            ("device_idx", vibevoice::DEVICE_OPTIONS),
            ("compute_idx", vibevoice::COMPUTE_OPTIONS),
        ],
        NodeKind::VoxCpm2 => vec![
            ("device_idx", voxcpm2::DEVICE_OPTIONS),
            ("compute_idx", voxcpm2::COMPUTE_OPTIONS),
        ],
        NodeKind::Llm => vec![
            ("device_idx", llm::DEVICE_OPTIONS),
            ("quant_idx", llm::QUANT_OPTIONS),
            ("compute_idx", llm::COMPUTE_OPTIONS),
        ],
        NodeKind::AceStepCheckpoint => vec![
            ("device_idx", acestep::DEVICE_OPTIONS),
            ("quant_dit_idx", acestep::QUANT_OPTIONS),
            ("quant_enc_idx", acestep::QUANT_OPTIONS),
            ("compute_idx", acestep::COMPUTE_OPTIONS),
        ],
        NodeKind::AceStepVaeEncode => vec![
            ("device_idx", acestep::DEVICE_OPTIONS),
            ("storage_idx", acestep::QUANT_OPTIONS),
            ("compute_idx", acestep::COMPUTE_OPTIONS),
        ],
        NodeKind::AceStepGenerate => vec![
            ("mode_idx", acestep::generate::MODE_OPTIONS),
            ("keyscale_idx", acestep::generate::KEYSCALE_OPTIONS),
            ("timesig_idx", acestep::generate::TIMESIG_OPTIONS),
            ("track_idx", acestep::generate::TRACK_OPTIONS),
        ],
        NodeKind::FfmpegPlayer => vec![("hwaccel_idx", ffmpeg_player::HWACCEL_LABELS)],
        NodeKind::LtxCheckpoint => vec![
            ("device_idx", ltx::DEVICE_OPTIONS),
            ("quant_dit_idx", ltx::QUANT_DIT_OPTIONS),
            ("quant_enc_idx", ltx::QUANT_ENC_OPTIONS),
            ("compute_idx", ltx::COMPUTE_OPTIONS),
        ],
        NodeKind::LtxSamplerStage1
        | NodeKind::LtxRetake
        | NodeKind::LtxLipdub
        | NodeKind::LtxA2V => vec![("fps_idx", ltx::FPS_OPTIONS)],
        NodeKind::LtxIcLora => vec![
            ("fps_idx", ltx::FPS_OPTIONS),
            ("control_idx", ltx::CONTROL_OPTIONS),
        ],
        NodeKind::H3Checkpoint => vec![
            ("variant_idx", minimax_h3::VARIANT_OPTIONS),
            ("device_idx", minimax_h3::DEVICE_OPTIONS),
            ("quant_dit_idx", minimax_h3::QUANT_DIT_OPTIONS),
            ("quant_enc_idx", minimax_h3::QUANT_ENC_OPTIONS),
            ("compute_idx", minimax_h3::COMPUTE_OPTIONS),
            ("memory_mode_idx", minimax_h3::MEMORY_MODE_OPTIONS),
        ],
        NodeKind::H3EmptyLatentAv => vec![("aspect_idx", minimax_h3::latent::ASPECT_OPTIONS)],
        _ => Vec::new(),
    }
}

/// `quant_dit_idx: 0=nvfp4, 1=mxfp8, 2=dense (compute)` — одна строка на поле.
fn enum_hints_lines(kind: NodeKind) -> Vec<String> {
    enum_hints(kind)
        .into_iter()
        .map(|(field, opts)| {
            let vals = opts
                .iter()
                .enumerate()
                .map(|(i, o)| format!("{i}={o}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{field}: {vals}")
        })
        .collect()
}

/// Заметка про железные поля. Enum-таблица показывает агенту, что `1=nvfp4`,
/// и он трактует это как приглашение «оптимизировать»: увидев мало свободной
/// VRAM (её занимает сама чат-LLM), включал квант на DiT и энкодере и ронял
/// качество. VRAM под прогон освобождает `run free_vram=true`, а не квант.
fn hardware_note(kind: NodeKind) -> Option<&'static str> {
    let has_hw = enum_hints(kind)
        .iter()
        .any(|(f, _)| f.starts_with("quant") || *f == "storage_idx" || *f == "device_idx");
    has_hw.then_some(
        "note: device_idx / quant_* / compute_idx already carry the right defaults for \
         this machine — keep them unless the user asks. Quantization trades quality for \
         VRAM; to free VRAM use action=run with free_vram=true instead.",
    )
}

/// Подсказки для path-полей, у которых пустое значение = «взять бандл из
/// каталога по имени». Enum-таблица их не покрывает (это строки, не индексы),
/// а без списка кандидатов агент не знал, чем переключить связку на turbo или
/// на лёгкий LM, и оставлял дефолт. Первое имя в списке — то, что резолвится
/// при пустом поле.
fn path_hints_lines(kind: NodeKind) -> Vec<String> {
    use crate::pages::node_editor::nodes::acestep::generate;
    let named = |field: &str, names: &[&str]| {
        let vals = names
            .iter()
            .enumerate()
            .map(|(i, n)| if i == 0 { format!("{n} (default)") } else { (*n).to_string() })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{field} (null = pick from models_dir): {vals}")
    };
    match kind {
        NodeKind::AceStepCheckpoint => vec![
            named("lm_path", generate::LM_NAMES),
            named("text_encoder_path", generate::TEXT_ENC_NAMES),
            named("dit_path", generate::DIT_NAMES),
            named("vae_path", generate::VAE_NAMES),
        ],
        _ => Vec::new(),
    }
}

fn state_example(kind: NodeKind) -> Option<String> {
    let rt = registry::default_runtime(kind);
    let g = rt.lock().ok()?;
    let state = convert::runtime_to_state(&g)?;
    serde_json::to_string(&state).ok()
}

fn category_path(meta: &'static registry::NodeKindMeta) -> String {
    match meta.subcategory {
        Some(sub) => format!("{} / {}", meta.category.label(), sub),
        None => meta.category.label().to_string(),
    }
}

/// Компакт-список всех видов нод по категориям — ответ на пустой фильтр и на
/// фильтр без совпадений.
fn compact_kind_list() -> String {
    let mut out = String::new();
    let mut current_cat: Option<NodeCategory> = None;
    for kind in NodeKind::ALL {
        let meta = registry::meta(*kind);
        if current_cat != Some(meta.category) {
            current_cat = Some(meta.category);
            out.push_str(&format!("--- {} ---\n", meta.category.label()));
        }
        out.push_str(&format!(
            "{} · {} · has run: {}\n",
            kind_slug(*kind),
            meta.title,
            if meta.on_run.is_some() { "yes" } else { "no" }
        ));
    }
    out
}

/// Имя без разделителей: `ace_step_checkpoint` и `AceStepCheckpoint` дают
/// одно и то же `acestepcheckpoint`.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

const FILTER_HINT: &str = "filter matches kind/title/category (not field \
     names — see those in the state example). Multiple values joined by a \
     space or comma are combined: \"ltx_checkpoint ltx_sampler_stage1\" \
     returns both nodes, \"ltx\" returns the whole family.";

fn nodes_impl(v: &serde_json::Value) -> Result<String, String> {
    let filter = v
        .get("filter")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    // Фильтр — набор токенов, а не одна подстрока: модели пишут туда список
    // нужных нод (и заодно имена полей), и матч по всей строке целиком не
    // давал совпадений — инструмент отвечал ошибкой без данных, а агент
    // крутил вариации фильтра, пока не упирался в guard повторов.
    let tokens: Vec<&str> = filter
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|t| !t.is_empty())
        .collect();

    let mut out = String::new();
    if tokens.is_empty() {
        out.push_str(&format!(
            "All node kinds (kind · title · category). Details (ports, \
             state JSON, *_idx decoding) — repeat with filter. {FILTER_HINT}\n"
        ));
        out.push_str(&compact_kind_list());
        return Ok(out);
    }

    let mut matched = 0usize;
    for kind in NodeKind::ALL {
        let meta = registry::meta(*kind);
        let slug = kind_slug(*kind);
        let hay = format!(
            "{} {} {} {}",
            slug,
            meta.title.to_lowercase(),
            meta.category.label().to_lowercase(),
            meta.subcategory.unwrap_or("").to_lowercase()
        );
        // Сравниваем и «как есть», и без разделителей: модель берёт имя из
        // виденного ей state-JSON (`AceStepCheckpoint`) вместо slug'а
        // (`ace_step_checkpoint`), получала «no nodes found» и шла спрашивать
        // ноды по одной — лишние ходы на ровном месте.
        let hay_squashed = squash(&hay);
        if !tokens
            .iter()
            .any(|t| hay.contains(t) || hay_squashed.contains(&squash(t)))
        {
            continue;
        }
        matched += 1;
        out.push_str(&format!(
            "--- {} · \"{}\" · {} ---\n",
            slug,
            meta.title,
            category_path(meta)
        ));
        out.push_str(&format!("inputs: {}\n", ports_line(meta.inputs)));
        out.push_str(&format!("outputs: {}\n", ports_line(meta.outputs)));
        if !meta.fields.is_empty() {
            let fields = meta
                .fields
                .iter()
                .map(|f| format!("{} ({:?})", f.name, f.ty))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("fields: {fields}\n"));
        }
        out.push_str(&format!(
            "run (on_run): {}\n",
            if meta.on_run.is_some() { "yes" } else { "no (reactive)" }
        ));
        if let Some(state) = state_example(*kind) {
            out.push_str(&format!("state (example with defaults): {state}\n"));
        }
        for line in enum_hints_lines(*kind)
            .into_iter()
            .chain(path_hints_lines(*kind))
            .chain(hardware_note(*kind).map(str::to_string))
        {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if matched == 0 {
        // Не ошибка: пустой ответ гонит агента по кругу с вариациями фильтра.
        // Отдаём то, ради чего он и звал инструмент, — список видов нод.
        return Ok(format!(
            "no nodes found for filter \"{filter}\". {FILTER_HINT}\n{}",
            compact_kind_list()
        ));
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
// open / graph
// ─────────────────────────────────────────────────────────────────────────────

fn open_impl(v: &serde_json::Value) -> Result<String, String> {
    let id = v
        .get("template")
        .and_then(|x| x.as_str())
        .ok_or("action=open requires the template parameter (id from action=list)")?;
    let all = templates::list_all();
    let t = all
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("template \"{id}\" not found (see action=list)"))?;

    let ctx = agent_ctx_ensure(&t.name)?;
    convert::load_into_ctx(&ctx, t);
    let mut out = format!("template \"{}\" loaded into the service tab\n", t.name);
    out.push_str(&graph_summary(&ctx)?);
    // Чем заполнять граф — говорим сразу: встроенные шаблоны путей не несут,
    // и агент иначе выясняет это только на pre-check прогона, потратив ходы.
    let missing = missing_model_paths(&ctx);
    if !missing.is_empty() {
        out.push_str("model paths not filled in (apply set_state, paths — from action=list):\n");
        for m in &missing {
            out.push_str(m);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Ноды графа с пустыми путями моделей — то же, что проверяет pre-check
/// прогона (`pipeline_run::prepare`), но до `run`.
fn missing_model_paths(ctx: &NodeEditorCtx) -> Vec<String> {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let mut out = Vec::new();
    for n in &nodes {
        if !n.enabled.get_untracked() {
            continue;
        }
        // Слот-ноды с подключённым входом `model` берут путь от чекпойнта.
        let has_model_input = conns
            .iter()
            .any(|c| c.to_node == n.id && c.to_port == "model");
        let Ok(rt) = n.runtime.lock() else { continue };
        let mut fields: Vec<&str> = rt
            .missing_model_paths()
            .into_iter()
            .filter(|f| !(*f == "model_path" && has_model_input))
            .collect();
        // Upscaler требуется только графам со стадией Upscale ×2.
        let needs_upscaler = nodes
            .iter()
            .any(|x| x.enabled.get_untracked() && x.kind == NodeKind::LtxUpscale);
        if needs_upscaler && rt.ltx_upscaler_missing() {
            fields.push("upscaler_path");
        }
        if !fields.is_empty() {
            out.push(format!(
                "node {} ({}): {}",
                n.id.0,
                registry::meta(n.kind).title,
                fields.join(", ")
            ));
        }
    }
    out
}

fn graph_impl() -> Result<String, String> {
    match agent_ctx()? {
        Some(ctx) => graph_summary(&ctx),
        None => Err("there's no service tab yet — open a template (action=open) or build a graph (action=apply)".into()),
    }
}

/// Текстовый snapshot графа: ноды с id/kind/state и связи. `state`-строки
/// обрезаются — envelope должен оставаться компактным (в историю агента
/// tool-result уходит клипованным).
/// Сводка графа для `list`: ноды без state, связи числом. Полный снимок с
/// state и связями — `action=graph`.
fn graph_brief(ctx: &NodeEditorCtx) -> Result<String, String> {
    let (nodes, conns, _viewport) = convert::snapshot(ctx);
    let mut out = format!(
        "graph: {} nodes, {} connections (full snapshot — action=graph)\n",
        nodes.len(),
        conns.len()
    );
    for n in &nodes {
        out.push_str(&format!(
            "[{}] {} · {}{}\n",
            n.id,
            kind_slug(n.kind),
            registry::meta(n.kind).title,
            if n.enabled { "" } else { " · OFF" }
        ));
    }
    Ok(out)
}

fn graph_summary(ctx: &NodeEditorCtx) -> Result<String, String> {
    const STATE_CLIP: usize = 700;
    let (nodes, conns, _viewport) = convert::snapshot(ctx);
    let mut out = format!("graph: {} nodes, {} connections\n", nodes.len(), conns.len());
    for n in &nodes {
        let meta = registry::meta(n.kind);
        let mut line = format!(
            "[{}] {} · {}{}",
            n.id,
            kind_slug(n.kind),
            meta.title,
            if n.enabled { "" } else { " · OFF" }
        );
        if let Some(state) = &n.state {
            if let Ok(js) = serde_json::to_string(state) {
                let clipped = if js.len() > STATE_CLIP {
                    let mut end = STATE_CLIP;
                    while end > 0 && !js.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &js[..end])
                } else {
                    js
                };
                line.push_str(&format!(" · state: {clipped}"));
            }
        }
        line.push('\n');
        out.push_str(&line);
    }
    if !conns.is_empty() {
        out.push_str("connections:\n");
        for c in &conns {
            out.push_str(&format!(
                "{}.{} → {}.{}\n",
                c.from_node, c.from_port, c.to_node, c.to_port
            ));
        }
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
// apply
// ─────────────────────────────────────────────────────────────────────────────

fn apply_impl(v: &serde_json::Value) -> Result<String, String> {
    let ctx = agent_ctx_ensure(&tr!("nodes.agent_tab.title"))?;
    let mut notes: Vec<String> = Vec::new();

    // attachment:-ссылки резолвятся по всему payload'у до парсинга структур.
    let mut v = v.clone();
    resolve_attachment_uris(&mut v, &mut notes);

    if let Some(graph) = v.get("graph") {
        let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("replace");
        let (graph, note) = unwrap_json_string(graph, "graph")?;
        notes.extend(note);
        let template = template_from_value(&graph)?;
        match mode {
            "replace" => convert::load_into_ctx(&ctx, &template),
            "merge" => convert::apply_to_ctx(&ctx, &template, Point::new(0.0, 0.0)),
            other => return Err(format!("unknown mode \"{other}\" (replace | merge)")),
        }
        notes.push(format!(
            "graph applied ({mode}): {} nodes, {} connections",
            template.nodes.len(),
            template.connections.len()
        ));
        notes.extend(unknown_graph_state_fields(&graph));
    }

    // Ошибки по отдельным элементам не отменяют весь apply: половина графа,
    // применённая из четырёх нод, — это прогресс, а полный откат заставлял
    // агента пересобирать вызов целиком и терять уже верные куски.
    let mut problems: Vec<String> = Vec::new();

    if let Some(items) = coerce_array(&v, "set_state", &mut notes)? {
        for (i, item) in items.iter().enumerate() {
            match apply_one_state(&ctx, item) {
                Ok(note) => notes.push(note),
                Err(e) => problems.push(format!("set_state[{i}]: {e}")),
            }
        }
    }

    if let Some(items) = coerce_array(&v, "connect", &mut notes)? {
        for (i, item) in items.iter().enumerate() {
            let res = conn_from_value(item, &ctx).and_then(|c| {
                add_connection(&ctx, &c)?;
                Ok(format!(
                    "connection {}.{} → {}.{}",
                    c.from_node, c.from_port, c.to_node, c.to_port
                ))
            });
            match res {
                Ok(note) => notes.push(note),
                Err(e) => problems.push(format!("connect[{i}]: {e}")),
            }
        }
    }

    if let Some(items) = coerce_array(&v, "disconnect", &mut notes)? {
        for item in &items {
            let c = match conn_from_value(item, &ctx) {
                Ok(c) => c,
                Err(e) => {
                    problems.push(format!("disconnect: {e}"));
                    continue;
                }
            };
            let mut conns = ctx.connections.get_untracked();
            let before = conns.len();
            conns.retain(|e| {
                !(e.from_node.0 == c.from_node
                    && e.to_node.0 == c.to_node
                    && e.from_port == c.from_port
                    && e.to_port == c.to_port)
            });
            let removed = before - conns.len();
            ctx.connections.set(conns);
            notes.push(format!(
                "disconnected {}: {}.{} → {}.{}",
                removed, c.from_node, c.from_port, c.to_node, c.to_port
            ));
        }
    }

    if notes.is_empty() && !problems.is_empty() {
        return Err(problems.join("\n"));
    }

    if notes.is_empty() {
        // Не ошибка: `apply` без полей модели шлют как «подтверди правки»,
        // и отказ загонял их в цикл повторов. Отдаём состояние графа и
        // прямо называем следующий шаг.
        let got = v
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, val)| format!("{k}: {}", json_type_name(val)))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return Ok(format!(
            "apply made no changes — only [{got}] was passed. Edits go through \
             set_state / connect / disconnect / graph, set_state format: \
             [{{\"node\": <id|kind name>, \"state\": {{\"kind\": \"…\", \"data\": {{…}}}}}}]\n\
             If the graph is already assembled — the next step is action=run.\n{}",
            graph_brief(&ctx)?
        ));
    }

    let mut out = notes.join("\n");
    out.push('\n');
    if !problems.is_empty() {
        out.push_str("⚠ not applied:\n");
        for p in &problems {
            out.push_str(p);
            out.push('\n');
        }
    }
    // Сводка, а не полный снимок: state всех нод со связями — это ~2.5 тыс.
    // символов на каждый apply, из которых агенту нужна лишь строка «что
    // дальше». Полный снимок остаётся за action=graph.
    out.push_str(&graph_brief(&ctx)?);
    out.push_str(
        "Next step: action=run (free_vram=true if system status shows not \
         enough VRAM). To check the full state — action=graph.\n",
    );
    Ok(out)
}

/// Тег варианта NodeStateData («LtxTextEncoder», …) — из его serde-формы.
fn variant_tag(s: &NodeStateData) -> Option<String> {
    serde_json::to_value(s)
        .ok()?
        .get("kind")?
        .as_str()
        .map(str::to_string)
}

/// Неизвестные ключи `state.data` в agent-JSON графа. В отличие от
/// `set_state` здесь это предупреждение, а не отказ: граф из десятка нод не
/// стоит ронять целиком из-за одного лишнего ключа. Но и молчать нельзя —
/// serde их выбрасывает, и нода уезжает в прогон со старым значением.
fn unknown_graph_state_fields(graph: &serde_json::Value) -> Vec<String> {
    let Some(nodes) = graph.get("nodes").and_then(|n| n.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let Some(slug) = node.get("kind").and_then(|k| k.as_str()) else { continue };
        let Some(kind) = NodeKind::ALL.iter().find(|k| kind_slug(**k) == slug) else { continue };
        let Some(state) = node.get("state") else { continue };
        let reference = registry::default_runtime(*kind)
            .lock()
            .ok()
            .and_then(|g| convert::runtime_to_state(&g));
        if let Err(e) = check_state_fields(reference.as_ref(), state) {
            let id = node.get("id").and_then(|x| x.as_u64());
            let at = id.map(|n| format!("node {n}")).unwrap_or_else(|| format!("nodes[{i}]"));
            out.push(format!("⚠ {at} ({slug}): {e}"));
        }
    }
    out
}

/// Собрать Template из agent-JSON `{nodes, connections}`. Послабления к
/// строгой схеме: отсутствующие `id` нумеруются по порядку, отсутствующие
/// `pos` раскладываются сеткой — LLM не обязан придумывать координаты.
fn template_from_value(graph: &serde_json::Value) -> Result<Template, String> {
    let mut graph = graph.clone();
    if let Some(nodes) = graph.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for (i, node) in nodes.iter_mut().enumerate() {
            let Some(obj) = node.as_object_mut() else {
                return Err(format!("nodes[{i}] is not an object"));
            };
            obj.entry("id".to_string())
                .or_insert(serde_json::json!((i + 1) as u64));
            obj.entry("pos".to_string()).or_insert(serde_json::json!({
                "x": 80.0 + (i % 4) as f32 * 300.0,
                "y": 100.0 + (i / 4) as f32 * 260.0,
            }));
        }
    }
    let nodes: Vec<NodeData> = serde_json::from_value(
        graph.get("nodes").cloned().unwrap_or(serde_json::json!([])),
    )
    .map_err(|e| format!("graph.nodes: {e}"))?;
    let connections: Vec<ConnData> = serde_json::from_value(
        graph
            .get("connections")
            .cloned()
            .unwrap_or(serde_json::json!([])),
    )
    .map_err(|e| format!("graph.connections: {e}"))?;
    let mut t = Template::empty("agent", TemplateKind::Full);
    t.nodes = nodes;
    t.connections = connections;
    Ok(t)
}

/// JSON-аргумент, который модель могла завернуть в строку.
///
/// Qwen и родственники регулярно сериализуют вложенную структуру как текст:
/// `"set_state": "[{\"node\": 13, …}]"`. Раньше такой вызов молча пролетал
/// мимо `as_array()` и получал «apply без изменений: передай … set_state» —
/// ошибку, отрицающую то, что агент видит в собственном вызове.
fn unwrap_json_string(
    v: &serde_json::Value,
    field: &str,
) -> Result<(serde_json::Value, Option<String>), String> {
    let Some(raw) = v.as_str() else {
        return Ok((v.clone(), None));
    };
    let text = raw.trim();
    let err = |e: serde_json::Error| {
        format!(
            "{field} came as a string, and it doesn't parse as JSON: {e}{}. \
             Pass the value as a structure (array/object), not as text.",
            json_error_context(text, &e)
        )
    };
    let e = match serde_json::from_str(text) {
        Ok(val) => return Ok((val, None)),
        Err(e) => e,
    };
    // Второй типовой случай: в одну строку склеены несколько аргументов —
    // `"[{…}], \"connect\": []"`. Значение этого поля — первое в строке;
    // берём его и называем отброшенный хвост, вместо отказа целиком.
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>();
    let Some(Ok(val)) = stream.next() else {
        // Третий случай: скобки закрыты не в том порядке — `…"}}]}` вместо
        // `…"}}}]`. Пересобираем хвост и, если после этого JSON валиден,
        // работаем с ним: иначе агент видит «передай структурой», меняет
        // формулировку, а не скобки, и повторяет ту же ошибку.
        if let Some(fixed) = repair_bracket_tail(text) {
            if let Ok(val) = serde_json::from_str(&fixed) {
                return Ok((
                    val,
                    Some(format!(
                        "⚠ {field}: the closing brackets were out of order and got \
                         rebuilt. Check that the applied value is what you meant."
                    )),
                ));
            }
        }
        return Err(err(e));
    };
    let rest = text[stream.byte_offset().min(text.len())..].trim();
    let note = (!rest.is_empty()).then(|| {
        format!(
            "⚠ {field}: everything after the first JSON value was ignored ({}). \
             Pass each argument as its own field, not glued into one string.",
            clip(rest, 80)
        )
    });
    Ok((val, note))
}

/// Обрезка длинного фрагмента для сообщения агенту.
fn clip(s: &str, max: usize) -> String {
    match s.chars().count() > max {
        true => format!("{}…", s.chars().take(max).collect::<String>()),
        false => s.to_string(),
    }
}

/// Фрагмент текста вокруг места ошибки разбора: «expected `:` at column 132»
/// без самого куска модель чинит наугад, тратя ходы.
fn json_error_context(text: &str, e: &serde_json::Error) -> String {
    let col = e.column();
    if e.line() != 1 || col == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let idx = col.min(chars.len()).saturating_sub(1);
    let from = idx.saturating_sub(40);
    let to = (idx + 20).min(chars.len());
    let frag: String = chars[from..to].iter().collect();
    format!(" (around: …{frag}…)")
}

/// Массив-аргумент: принимает массив, одиночный объект и строку с JSON.
/// Предупреждения разбора (отброшенный хвост склеенной строки) складывает в
/// `notes` — они уходят агенту вместе с результатом apply.
fn coerce_array(
    v: &serde_json::Value,
    field: &str,
    notes: &mut Vec<String>,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    let Some(raw) = v.get(field) else {
        return Ok(None);
    };
    let (val, note) = unwrap_json_string(raw, field)?;
    notes.extend(note);
    match val {
        serde_json::Value::Array(a) => Ok(Some(a)),
        // Один элемент без обёртки — тоже понятное намерение.
        serde_json::Value::Object(_) => Ok(Some(vec![val])),
        other => Err(format!(
            "{field}: expected an array of objects, got {}",
            json_type_name(&other)
        )),
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Числовая часть ссылки на ноду: `2` или `"2"` (модели любят строки).
fn node_ref_as_number(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.trim().parse::<u64>().ok())
}

/// Ссылка на ноду: числовой id (`2`), строка с числом (`"2"`) или slug вида
/// ноды (`"h3_checkpoint"`). Слаг разрешён, пока такая нода в графе одна:
/// снимок графа печатает `[2] h3_checkpoint`, и модели цепляются за имя чаще,
/// чем за номер — отвечать им «нужно поле node» было ложью.
fn resolve_node_ref(v: &serde_json::Value, ctx: &NodeEditorCtx) -> Result<u64, String> {
    if let Some(n) = node_ref_as_number(v) {
        return Ok(n);
    }
    let Some(raw) = v.as_str() else {
        return Err("node id is a number (2) or a node kind name (\"h3_checkpoint\")".to_string());
    };
    let name = raw.trim();
    let nodes = ctx.nodes.get_untracked();
    let want = name.to_lowercase();
    let hits: Vec<_> = nodes
        .iter()
        .filter(|n| kind_slug(n.kind).eq_ignore_ascii_case(&want))
        .map(|n| n.id.0)
        .collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!(
            "no node \"{name}\" in the graph; there is: {}",
            nodes
                .iter()
                .map(|n| format!("{} ({})", n.id.0, kind_slug(n.kind)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => Err(format!(
            "\"{name}\" is not unique in the graph — specify the numeric id: {}",
            hits.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Связь: `from_node`/`to_node` принимают то же, что и `set_state[].node`.
/// Один элемент `set_state`: резолв ноды, разбор state, проверка kind,
/// применение. Ошибка описывает конкретный элемент, а не весь вызов.
/// Слить патч-`state` с текущим состоянием ноды: верхнеуровневые ключи `data`
/// из патча перекрывают текущие, остальные сохраняются. Патч без `data` или с
/// другим `kind` возвращается как есть — про несовпадение варианта вызывающий
/// выдаёт отдельную ошибку.
fn merge_state_patch(
    current: Option<&NodeStateData>,
    patch: serde_json::Value,
) -> serde_json::Value {
    let Some(cur) = current.and_then(|s| serde_json::to_value(s).ok()) else {
        return patch;
    };
    let (Some(cur_obj), Some(patch_obj)) = (cur.as_object(), patch.as_object()) else {
        return patch;
    };
    if cur_obj.get("kind") != patch_obj.get("kind") {
        return patch;
    }
    let Some(patch_data) = patch_obj.get("data").and_then(|d| d.as_object()) else {
        return patch;
    };
    let mut data = cur_obj
        .get("data")
        .and_then(|d| d.as_object())
        .cloned()
        .unwrap_or_default();
    for (k, v) in patch_data {
        data.insert(k.clone(), v.clone());
    }
    let mut merged = cur_obj.clone();
    merged.insert("data".to_string(), serde_json::Value::Object(data));
    serde_json::Value::Object(merged)
}

/// Проверить ключи `data` патча против полей варианта и вернуть список тех,
/// что реально применятся. Неизвестный ключ — ошибка, а не пустое место:
/// `serde(default)` молча проглатывал выдуманное поле (агент прислал
/// TextView `text` вместо `output_text`), tool рапортовал «state updated»,
/// и прогон уходил со старым текстом — про это никто не узнавал до
/// прослушивания результата.
fn check_state_fields(
    reference: Option<&NodeStateData>,
    patch: &serde_json::Value,
) -> Result<Vec<String>, String> {
    let Some(patch_data) = patch.get("data").and_then(|d| d.as_object()) else {
        return Ok(Vec::new());
    };
    let names: Vec<String> = patch_data.keys().cloned().collect();
    let Some(known) = reference
        .and_then(|s| serde_json::to_value(s).ok())
        .and_then(|v| v.get("data").and_then(|d| d.as_object()).cloned())
    else {
        return Ok(names);
    };
    let unknown: Vec<String> =
        names.iter().filter(|k| !known.contains_key(*k)).map(|k| format!("\"{k}\"")).collect();
    if !unknown.is_empty() {
        return Err(format!(
            "unknown state field(s) {} — they would be silently dropped. \
             Fields of this node: {}",
            unknown.join(", "),
            known.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(names)
}

/// Агент шлёт state и без обёртки: `{"dit_path": …}`, `{"data": {…}}` без
/// `kind` или поля рядом с `kind`. Вид ноды известен — достраиваем
/// `{kind, data}` сами. Раньше такой вызов отклонялся «expects state
/// kind=…, but got —», и агент тратил ход на повтор. Чужой `kind`
/// сохраняется — дальше честная ошибка несовпадения.
fn normalize_state_shape(expected: Option<&str>, state: serde_json::Value) -> serde_json::Value {
    let (Some(expected), serde_json::Value::Object(mut obj)) = (expected, state.clone()) else {
        return state;
    };
    if obj.contains_key("data") {
        obj.entry("kind").or_insert_with(|| expected.into());
        return serde_json::Value::Object(obj);
    }
    let kind = obj.remove("kind").unwrap_or_else(|| expected.into());
    serde_json::json!({ "kind": kind, "data": serde_json::Value::Object(obj) })
}

/// Строки применённых полей для ответа apply: у `*_idx` — значение и
/// подпись варианта (`device_idx=0 (CPU)`). Голый индекс агент читал по
/// аналогии с другими нодами: у LTX 0 = CUDA, у ACE-Step 0 = CPU — и
/// «вернув дефолт», отправил энкод трека на процессор. Второе значение —
/// предупреждение, если нода переехала на CPU.
fn describe_changes(
    kind: NodeKind,
    state: &serde_json::Value,
    changed: &[String],
) -> (Vec<String>, Option<String>) {
    let hints = enum_hints(kind);
    let data = state.get("data");
    let mut to_cpu = false;
    let lines = changed
        .iter()
        .map(|field| {
            let opts = hints.iter().find(|(name, _)| *name == field.as_str()).map(|(_, o)| *o);
            let idx = data.and_then(|d| d.get(field)).and_then(|v| v.as_u64());
            match (opts, idx) {
                (Some(opts), Some(i)) => {
                    let label = opts.get(i as usize).copied().unwrap_or("out of range");
                    if field.starts_with("device") && label.eq_ignore_ascii_case("cpu") {
                        to_cpu = true;
                    }
                    format!("{field}={i} ({label})")
                }
                _ => field.clone(),
            }
        })
        .collect();
    let warning = to_cpu.then(|| {
        format!(
            "warning: \"{}\" will now run on the CPU — neural models there are many \
             times slower (a multi-minute track or a video takes tens of minutes to \
             hours). Keep the GPU unless the user asked for CPU.",
            registry::meta(kind).title
        )
    });
    (lines, warning)
}

fn apply_one_state(ctx: &NodeEditorCtx, item: &serde_json::Value) -> Result<String, String> {
    let node_ref = item
        .get("node")
        .ok_or("the node field is required (node id or kind name)")?;
    let node_id = resolve_node_ref(node_ref, ctx)?;
    let state_v = item.get("state").cloned().ok_or("the state field is required")?;
    let (state_v, parse_note) = unwrap_json_string(&state_v, "state")?;
    let nodes = ctx.nodes.get_untracked();
    let node = nodes
        .iter()
        .find(|n| n.id.0 == node_id)
        .ok_or_else(|| format!("node {node_id} not found in the graph"))?;
    // Совпадение варианта state с kind ноды проверяем заранее:
    // apply_state_to_runtime при несовпадении молча no-op'ает, а агенту
    // нужна честная ошибка.
    let reference = registry::default_runtime(node.kind)
        .lock()
        .ok()
        .and_then(|g| convert::runtime_to_state(&g));
    let expected = reference.as_ref().and_then(variant_tag);
    let state_v = normalize_state_shape(expected.as_deref(), state_v);
    let got = state_v.get("kind").and_then(|x| x.as_str()).map(str::to_string);
    if expected != got {
        return Err(format!(
            "node {node_id} ({}) expects state kind={}, but got {}",
            kind_slug(node.kind),
            expected.unwrap_or_else(|| "—".into()),
            got.unwrap_or_else(|| "—".into())
        ));
    }
    let changed = check_state_fields(reference.as_ref(), &state_v)
        .map_err(|e| format!("node {node_id}: {e}"))?;
    let (changed_lines, cpu_warning) = describe_changes(node.kind, &state_v, &changed);
    // Патч, а не замена: агент правит одно-два поля («поставь turbo-DiT»),
    // а `#[serde(default)]` на *StateData добил бы остальные дефолтами —
    // device_idx уехал бы в CPU, кванты и sampler-параметры откатились бы
    // молча. Недостающие ключи берём из текущего state ноды.
    let current = node.runtime.lock().ok().and_then(|g| convert::runtime_to_state(&g));
    let state_v = merge_state_patch(current.as_ref(), state_v);
    let state: NodeStateData = serde_json::from_value(state_v)
        .map_err(|e| format!("node {node_id}: failed to parse state: {e}"))?;
    if let Ok(rt) = node.runtime.lock() {
        convert::apply_state_to_runtime(&rt, &state);
    }
    // Перечисляем применённые поля: «updated» без списка не отличить от
    // «принял вызов и ничего не поменял».
    let mut msg = match changed.is_empty() {
        true => format!("state of node {node_id}: nothing to change (data is empty)"),
        false => format!("state of node {node_id} updated: {}", changed_lines.join(", ")),
    };
    if let Some(warning) = cpu_warning {
        msg.push('\n');
        msg.push_str(&warning);
    }
    if let Some(note) = parse_note {
        msg.push('\n');
        msg.push_str(&note);
    }
    Ok(msg)
}

fn conn_from_value(v: &serde_json::Value, ctx: &NodeEditorCtx) -> Result<ConnData, String> {
    let mut v = v.clone();
    for field in ["from_node", "to_node"] {
        let Some(raw) = v.get(field) else {
            return Err(format!("connection: the {field} field is required"));
        };
        let id = resolve_node_ref(raw, ctx).map_err(|e| format!("connection.{field}: {e}"))?;
        v[field] = serde_json::json!(id);
    }
    serde_json::from_value(v).map_err(|e| format!("connection: {e}"))
}

/// Добавить связь в граф вкладки с той же валидацией, что у `complete_wire`:
/// имена портов резолвятся в `&'static str` по registry, самосвязи и дубли
/// отбрасываются.
fn add_connection(ctx: &NodeEditorCtx, c: &ConnData) -> Result<(), String> {
    if c.from_node == c.to_node {
        return Err(format!("connection {}→{}: self-connection is forbidden", c.from_node, c.to_node));
    }
    let nodes = ctx.nodes.get_untracked();
    let from_kind = nodes
        .iter()
        .find(|n| n.id.0 == c.from_node)
        .map(|n| n.kind)
        .ok_or_else(|| format!("connect: node {} not found in the graph", c.from_node))?;
    let to_kind = nodes
        .iter()
        .find(|n| n.id.0 == c.to_node)
        .map(|n| n.kind)
        .ok_or_else(|| format!("connect: node {} not found in the graph", c.to_node))?;
    let from_port = convert::resolve_port_name(Some(from_kind), PortSide::Output, &c.from_port)
        .ok_or_else(|| {
            format!(
                "connect: {} has no output \"{}\" (see action=nodes filter={})",
                kind_slug(from_kind),
                c.from_port,
                kind_slug(from_kind)
            )
        })?;
    let to_port = convert::resolve_port_name(Some(to_kind), PortSide::Input, &c.to_port)
        .ok_or_else(|| {
            format!(
                "connect: {} has no input \"{}\" (see action=nodes filter={})",
                kind_slug(to_kind),
                c.to_port,
                kind_slug(to_kind)
            )
        })?;

    let conn = Connection {
        from_node: crate::pages::node_editor::types::NodeId(c.from_node),
        from_port,
        to_node: crate::pages::node_editor::types::NodeId(c.to_node),
        to_port,
    };
    let mut conns = ctx.connections.get_untracked();
    let dup = conns.iter().any(|e| {
        e.from_node == conn.from_node
            && e.to_node == conn.to_node
            && e.from_port == conn.from_port
            && e.to_port == conn.to_port
    });
    if !dup {
        conns.push(conn);
        ctx.connections.set(conns);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// attachment:-ссылки
// ─────────────────────────────────────────────────────────────────────────────

/// Пройтись по payload'у и заменить строки `attachment:<ref>` на путь blob'а
/// вложения текущего чата. `<ref>` — имя файла (case-insensitive), префикс
/// sha256 (≥6 hex) или `last` (самое свежее вложение). Используется
/// `blobs::model_path` — derived-копия в формате, читаемом пайплайнами.
fn resolve_attachment_uris(v: &mut serde_json::Value, notes: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(r) = s.strip_prefix("attachment:") {
                match find_attachment(r) {
                    Some(a) => {
                        let path = blobs::model_path(&a).display().to_string();
                        notes.push(format!("attachment:{r} → {path}"));
                        *s = path;
                    }
                    None => notes.push(format!(
                        "attachment:{r} — attachment not found (see action=list), string left as is"
                    )),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| resolve_attachment_uris(x, notes)),
        serde_json::Value::Object(o) => {
            o.values_mut().for_each(|x| resolve_attachment_uris(x, notes))
        }
        _ => {}
    }
}

pub(super) fn find_attachment(r: &str) -> Option<MsgAttachment> {
    let atts = chat_attachments();
    let r_lower = r.to_lowercase();
    if r_lower == "last" || r_lower == "latest" {
        return atts.last().cloned();
    }
    // Точное имя файла.
    if let Some(a) = atts
        .iter()
        .rev()
        .find(|a| a.original_name.to_lowercase() == r_lower)
    {
        return Some(a.clone());
    }
    // Префикс sha256.
    if r_lower.len() >= 6 && r_lower.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(a) = atts.iter().rev().find(|a| a.sha256.starts_with(&r_lower)) {
            return Some(a.clone());
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// save_template
// ─────────────────────────────────────────────────────────────────────────────

fn save_template_impl(v: &serde_json::Value) -> Result<String, String> {
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("action=save_template requires name")?;
    let ctx = agent_ctx()?
        .ok_or("there's no service tab — nothing to save")?;
    let (nodes, connections, viewport) = convert::snapshot(&ctx);
    if nodes.is_empty() {
        return Err("the graph is empty — nothing to save".into());
    }
    let mut t = Template::empty(name, TemplateKind::Full);
    if let Some(desc) = v.get("description").and_then(|x| x.as_str()) {
        t.description = desc.to_string();
    }
    t.nodes = nodes;
    t.connections = connections;
    t.viewport = viewport;
    let created = templates::create(t).map_err(|e| e.to_string())?;
    crate::components::template_picker::bump_revision();
    Ok(format!(
        "custom template saved \"{}\" (id: {})",
        created.name, created.id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `set_state` строкой с JSON — типовая манера моделей; принимаем.
    #[test]
    fn coerce_array_unwraps_json_string_and_single_object() {
        let v = serde_json::json!({
            "set_state": "[{\"node\": 13, \"state\": {\"kind\": \"TextView\"}}]"
        });
        let mut notes = Vec::new();
        let items =
            coerce_array(&v, "set_state", &mut notes).expect("строка разбирается").expect("есть");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["node"], 13);

        // Одиночный объект без массива — тоже понятное намерение.
        let v = serde_json::json!({"set_state": {"node": 2, "state": {}}});
        assert_eq!(coerce_array(&v, "set_state", &mut notes).unwrap().unwrap().len(), 1);

        // Поля нет — не ошибка.
        assert!(coerce_array(&serde_json::json!({}), "set_state", &mut notes).unwrap().is_none());
    }

    /// Битый JSON внутри строки объясняется, а не прячется за «apply без
    /// изменений»: агенту нужна позиция ошибки, иначе он уходит в перебор.
    #[test]
    fn coerce_array_reports_broken_json_string() {
        let v = serde_json::json!({"set_state": "[{\"prompt\", \"\"}]"});
        let mut notes = Vec::new();
        let err = coerce_array(&v, "set_state", &mut notes).expect_err("битый JSON");
        assert!(err.contains("set_state came as a string"), "{err}");
        assert!(err.contains("column"), "{err}");
        assert!(err.contains("around:"), "нужен фрагмент вокруг ошибки: {err}");
    }

    /// Описания шаблонов в `list` подрезаются: полный `list` обязан влезать
    /// в лимит истории хода, иначе из него вырезается раздел моделей.
    #[test]
    fn template_descriptions_are_clipped() {
        let long = "с".repeat(400);
        let clipped = clip_desc(&long);
        assert!(clipped.chars().count() <= 91, "{}", clipped.chars().count());
        assert!(clipped.ends_with('…'));
        assert_eq!(clip_desc("коротко"), "коротко");
    }

    /// Раздел шаблонов в `list` держится в бюджете: вместе с инвентарём
    /// моделей и сводкой графа весь ответ должен влезать в окно даже при
    /// тесном контексте (`budget::fit` режет середину — а там ровно то,
    /// ради чего инструмент звали).
    #[test]
    fn templates_section_fits_history_budget() {
        let len: usize = templates::list_all()
            .iter()
            .map(|t| {
                format!(
                    "{} · «{}» · нод: {} · {}\n",
                    t.id,
                    t.name,
                    t.nodes.len(),
                    clip_desc(&t.description)
                )
                .chars()
                .count()
            })
            .sum();
        assert!(len < 4500, "раздел шаблонов раздулся до {len} символов");
    }

    /// Фильтр — набор токенов: перечисление нод возвращает их все.
    #[test]
    fn nodes_filter_matches_any_token() {
        let out = nodes_impl(&serde_json::json!({
            "filter": "ltx_checkpoint ltx_sampler_stage1 quant_dit_idx width"
        }))
        .expect("nodes");
        assert!(out.contains("\"LTX Checkpoint\""), "{out}");
        assert!(out.contains("\"LTX Sampler Stage1\""), "{out}");
        assert!(!out.contains("\"H3 Checkpoint\""), "{out}");
    }

    /// Промах фильтра — не ошибка, а список видов нод: иначе агент крутит
    /// вариации фильтра до guard'а повторов.
    #[test]
    fn nodes_filter_miss_returns_catalog() {
        let out = nodes_impl(&serde_json::json!({"filter": "quant_dit_idx keep_gemma"}))
            .expect("промах не ошибка");
        assert!(out.contains("no nodes found"), "{out}");
        assert!(out.contains("ltx_checkpoint · LTX Checkpoint"), "{out}");
    }

    #[test]
    fn node_ref_accepts_number_and_numeric_string() {
        assert_eq!(node_ref_as_number(&serde_json::json!(2)), Some(2));
        assert_eq!(node_ref_as_number(&serde_json::json!(" 3 ")), Some(3));
        assert_eq!(node_ref_as_number(&serde_json::json!("h3_checkpoint")), None);
    }

    /// `*_idx`-поля state расшифрованы: без этого агент выбирал квант вслепую.
    #[test]
    fn enum_hints_explain_quant_indices() {
        let lines = enum_hints_lines(NodeKind::LtxCheckpoint);
        let quant = lines
            .iter()
            .find(|l| l.starts_with("quant_dit_idx:"))
            .expect("quant_dit_idx расшифрован");
        assert!(quant.contains("0=nvfp4"), "{quant}");
        assert!(enum_hints_lines(NodeKind::Add).is_empty());
    }

    #[test]
    fn kind_slug_is_snake_case() {
        assert_eq!(kind_slug(NodeKind::LtxTextEncoder), "ltx_text_encoder");
        assert_eq!(kind_slug(NodeKind::AceStepGenerate), "ace_step_generate");
    }

    /// Agent-JSON без id/pos дополняется автоматически; state парсится.
    #[test]
    fn template_from_value_fills_defaults() {
        let graph = serde_json::json!({
            "nodes": [
                { "kind": "ltx_checkpoint" },
                {
                    "kind": "ltx_text_encoder",
                    "state": {
                        "kind": "LtxTextEncoder",
                        "data": { "prompt": "закат над морем" }
                    }
                }
            ],
            "connections": [
                { "from_node": 1, "from_port": "model", "to_node": 2, "to_port": "model" }
            ]
        });
        let t = template_from_value(&graph).expect("parse");
        assert_eq!(t.nodes.len(), 2);
        assert_eq!(t.nodes[0].id, 1);
        assert_eq!(t.nodes[1].id, 2);
        assert_eq!(t.nodes[0].kind, NodeKind::LtxCheckpoint);
        // pos проставлен сеткой.
        assert!(t.nodes[1].pos.x > t.nodes[0].pos.x);
        let state = t.nodes[1].state.as_ref().expect("state");
        assert_eq!(variant_tag(state).as_deref(), Some("LtxTextEncoder"));
        assert_eq!(t.connections.len(), 1);
    }

    /// Неизвестные ключи внутри state.data не валят парсинг (serde default),
    /// но apply о них предупреждает — иначе поле теряется беззвучно.
    #[test]
    fn template_from_value_tolerates_unknown_state_fields() {
        let graph = serde_json::json!({
            "nodes": [{
                "kind": "ltx_text_encoder",
                "state": {
                    "kind": "LtxTextEncoder",
                    "data": { "prompt": "x", "made_up_field": 42 }
                }
            }]
        });
        let t = template_from_value(&graph).expect("parse");
        assert!(t.nodes[0].state.is_some());
        let warns = unknown_graph_state_fields(&graph).join("\n");
        assert!(warns.contains("made_up_field"), "{warns}");
        assert!(warns.contains("ltx_text_encoder"), "{warns}");
    }

    /// Дефолтные чекпойнты без путей — pre-check прогона это ловит.
    #[test]
    fn default_checkpoints_report_missing_paths() {
        let rt = registry::default_runtime(NodeKind::LtxCheckpoint);
        let g = rt.lock().unwrap();
        let miss = g.missing_model_paths();
        assert!(miss.contains(&"model_path"), "{miss:?}");
        assert!(miss.contains(&"gemma_dir"), "{miss:?}");
        drop(g);
        let rt = registry::default_runtime(NodeKind::H3Checkpoint);
        assert_eq!(rt.lock().unwrap().missing_model_paths(), vec!["model_path"]);
        // Реактивные ноды ничего не требуют.
        let rt = registry::default_runtime(NodeKind::Add);
        assert!(rt.lock().unwrap().missing_model_paths().is_empty());
    }

    /// Инвентарь каталога моделей видит .syn на глубине и HF-каталоги.
    #[test]
    fn models_inventory_finds_bundles() {
        let tmp = std::env::temp_dir().join(format!(
            "synthos_models_inv_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("video")).unwrap();
        std::fs::create_dir_all(tmp.join("gemma-3-12b")).unwrap();
        std::fs::write(tmp.join("qwen3.8-27b.syn"), b"x").unwrap();
        std::fs::write(tmp.join("video/ltx23.syn"), b"x").unwrap();
        std::fs::write(tmp.join("video/turbo_lora.safetensors"), b"x").unwrap();
        std::fs::write(tmp.join("gemma-3-12b/config.json"), b"{}").unwrap();
        std::fs::write(tmp.join("readme.txt"), b"x").unwrap();

        let inv = models_inventory(&tmp).join("\n");
        assert!(inv.contains("qwen3.8-27b.syn"), "{inv}");
        assert!(inv.contains("ltx23.syn"), "{inv}");
        assert!(inv.contains("turbo_lora.safetensors"), "{inv}");
        assert!(inv.contains("HF model directory"), "{inv}");
        assert!(!inv.contains("readme.txt"), "{inv}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Патч из одного поля не откатывает остальные: `device_idx` (GPU) и
    /// кванты переживают точечную смену DiT на turbo.
    #[test]
    fn merge_state_patch_keeps_unmentioned_fields() {
        use crate::templates::model::AceStepCheckpointStateData;
        let current = NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
            models_dir: Some("/models".into()),
            device_idx: 1,
            quant_dit_idx: 2,
            resident: true,
            ..Default::default()
        });
        let patch = serde_json::json!({
            "kind": "AceStepCheckpoint",
            "data": { "dit_path": "/models/acestep_v15_xl_turbo.syn" }
        });
        let merged: NodeStateData =
            serde_json::from_value(merge_state_patch(Some(&current), patch)).expect("parse");
        let NodeStateData::AceStepCheckpoint(d) = merged else { panic!("wrong variant") };
        assert_eq!(d.dit_path.as_deref(), Some("/models/acestep_v15_xl_turbo.syn"));
        assert_eq!(d.models_dir.as_deref(), Some("/models"));
        assert_eq!(d.device_idx, 1);
        assert_eq!(d.quant_dit_idx, 2);
        assert!(d.resident);
    }

    /// Строка со склеенными аргументами: берём первое значение, про хвост
    /// говорим вслух. Раньше весь вызов отклонялся, и агент повторял его.
    #[test]
    fn coerce_array_recovers_value_glued_with_extra_args() {
        let v = serde_json::json!({
            "set_state": "[{\"node\": 3, \"state\": {}}], \"connect\": []"
        });
        let mut notes = Vec::new();
        let items = coerce_array(&v, "set_state", &mut notes).unwrap().expect("есть");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["node"], 3);
        assert!(notes.iter().any(|n| n.contains("connect")), "{notes:?}");
    }

    /// Скобки, закрытые не в том порядке (реальный вызов модели: `…"}}]}`
    /// вместо `…"}}}]`), чинятся, а не роняют apply.
    #[test]
    fn coerce_array_repairs_out_of_order_brackets() {
        let broken = "[{\"node\": 3, \"state\": {\"kind\": \"TextView\", \"data\": \
                      {\"output_text\": \"tags\"}}}, {\"node\": 4, \"state\": {\"kind\": \
                      \"TextView\", \"data\": {\"output_text\": \"lyrics\"}}]}";
        let v = serde_json::json!({ "set_state": broken });
        let mut notes = Vec::new();
        let items = coerce_array(&v, "set_state", &mut notes).unwrap().expect("есть");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1]["node"], 4);
        assert_eq!(items[1]["state"]["data"]["output_text"], "lyrics");
        assert!(notes.iter().any(|n| n.contains("brackets")), "{notes:?}");
    }

    /// Чинится только скобочный хвост: оборванная строка остаётся ошибкой.
    #[test]
    fn repair_bracket_tail_gives_up_on_truncated_string() {
        assert!(repair_bracket_tail("[{\"a\": \"unterminated").is_none());
        assert_eq!(repair_bracket_tail("[{\"a\": 1}").as_deref(), Some("[{\"a\": 1}]"));
        // Сбалансированному тексту чинить нечего.
        assert!(repair_bracket_tail("[{\"a\": 1}]").is_none());
    }

    /// Фильтр нод матчится и по CamelCase-имени варианта state: модель
    /// берёт его из state-JSON, а не из slug'а.
    #[test]
    fn nodes_filter_matches_camel_case_kind() {
        let v = serde_json::json!({ "filter": "AceStepCheckpoint AceStepGenerate" });
        let out = nodes_impl(&v).expect("ok");
        assert!(out.contains("--- ace_step_checkpoint"), "{out}");
        assert!(out.contains("--- ace_step_generate"), "{out}");
        assert!(!out.contains("no nodes found"), "{out}");
    }

    /// Выдуманное поле — ошибка со списком настоящих, а не молчаливая потеря
    /// (агент слал TextView `text` вместо `output_text`, и текст песни не
    /// доезжал до графа).
    #[test]
    fn check_state_fields_rejects_unknown_key() {
        use crate::templates::model::TextViewStateData;
        let reference = NodeStateData::TextView(TextViewStateData::default());
        let patch = serde_json::json!({ "kind": "TextView", "data": { "text": "песня" } });
        let err = check_state_fields(Some(&reference), &patch).expect_err("must fail");
        assert!(err.contains("\"text\""), "{err}");
        assert!(err.contains("output_text"), "{err}");

        let ok = serde_json::json!({ "kind": "TextView", "data": { "output_text": "песня" } });
        assert_eq!(check_state_fields(Some(&reference), &ok).unwrap(), vec!["output_text"]);
    }

    /// У нод с device/quant/compute есть заметка «не трогай без просьбы» —
    /// иначе агент включает квант, увидев мало свободной VRAM.
    #[test]
    fn hardware_note_covers_checkpoint_nodes() {
        assert!(hardware_note(NodeKind::AceStepCheckpoint).is_some());
        assert!(hardware_note(NodeKind::LtxCheckpoint).is_some());
        assert!(hardware_note(NodeKind::TextView).is_none());
    }

    /// Патч с другим `kind` не мержится — вызывающий должен увидеть чужой
    /// вариант и выдать ошибку, а не тихо применить поля текущего.
    #[test]
    fn merge_state_patch_leaves_foreign_kind_alone() {
        use crate::templates::model::AceStepCheckpointStateData;
        let current = NodeStateData::AceStepCheckpoint(AceStepCheckpointStateData {
            device_idx: 1,
            ..Default::default()
        });
        let patch = serde_json::json!({ "kind": "TextView", "data": { "output_text": "x" } });
        let merged = merge_state_patch(Some(&current), patch.clone());
        assert_eq!(merged, patch);
    }

    /// Агент видит, каким именем переключить связку на turbo/1.7b, и какое
    /// имя подставится при пустом path-поле.
    #[test]
    fn path_hints_list_acestep_bundles() {
        let lines = path_hints_lines(NodeKind::AceStepCheckpoint).join("\n");
        assert!(lines.contains("acestep_5hz_lm_4b.syn (default)"), "{lines}");
        assert!(lines.contains("acestep_5hz_lm_1.7b.syn"), "{lines}");
        assert!(lines.contains("acestep_v15_xl_base.syn (default)"), "{lines}");
        assert!(lines.contains("acestep_v15_xl_turbo.syn"), "{lines}");
        assert!(path_hints_lines(NodeKind::Add).is_empty());
    }

    /// Дефолтная связка ACE-Step: 4b-LM + xl_base, кванты выключены.
    #[test]
    fn acestep_checkpoint_defaults_are_4b_base_dense() {
        use crate::pages::node_editor::nodes::acestep::{self, generate};
        assert_eq!(generate::LM_NAMES[0], "acestep_5hz_lm_4b.syn");
        assert_eq!(generate::DIT_NAMES[0], "acestep_v15_xl_base.syn");
        assert_eq!(acestep::QUANT_OPTIONS[acestep::default_storage_idx()], "none");
    }

    /// Пример state снимается с default_runtime и несёт правильный тег.
    #[test]
    fn state_example_matches_kind() {
        let js = state_example(NodeKind::LtxSamplerStage1).expect("state");
        assert!(js.contains("\"LtxSamplerStage1\""), "{js}");
        assert!(js.contains("width"), "{js}");
        // Реактивные ноды без state.
        assert!(state_example(NodeKind::Add).is_none());
    }

    /// state без обёртки `{kind, data}` достраивается по виду ноды (первая
    /// попытка apply в чате «Vocal» пришла плоской и была отклонена).
    #[test]
    fn normalize_state_shape_wraps_flat_forms() {
        use serde_json::json;
        let flat = normalize_state_shape(Some("AceStepCheckpoint"), json!({"dit_path": "/m/x.syn"}));
        assert_eq!(flat, json!({"kind": "AceStepCheckpoint", "data": {"dit_path": "/m/x.syn"}}));
        let no_kind = normalize_state_shape(Some("TextView"), json!({"data": {"output_text": "a"}}));
        assert_eq!(no_kind, json!({"kind": "TextView", "data": {"output_text": "a"}}));
        let beside = normalize_state_shape(Some("TextView"), json!({"kind": "TextView", "output_text": "a"}));
        assert_eq!(beside, json!({"kind": "TextView", "data": {"output_text": "a"}}));
        let canon = json!({"kind": "TextView", "data": {"output_text": "a"}});
        assert_eq!(normalize_state_shape(Some("TextView"), canon.clone()), canon);
        let foreign = normalize_state_shape(Some("TextView"), json!({"kind": "AudioFile", "loaded_path": "/a"}));
        assert_eq!(foreign["kind"], "AudioFile");
    }

    /// Ответ apply называет вариант индекса и предупреждает о переезде на CPU.
    #[test]
    fn describe_changes_labels_indices_and_warns_on_cpu() {
        use serde_json::json;
        let cpu = json!({"kind": "AceStepVaeEncode", "data": {"device_idx": 0, "chunk_seconds": 20.0}});
        let fields = vec!["device_idx".to_string(), "chunk_seconds".to_string()];
        let (lines, warning) = describe_changes(NodeKind::AceStepVaeEncode, &cpu, &fields);
        assert_eq!(lines, vec!["device_idx=0 (CPU)", "chunk_seconds"]);
        assert!(warning.expect("warning").contains("CPU"));

        let gpu = json!({"kind": "AceStepVaeEncode", "data": {"device_idx": 1}});
        let (lines, warning) =
            describe_changes(NodeKind::AceStepVaeEncode, &gpu, &["device_idx".to_string()]);
        assert_eq!(lines, vec!["device_idx=1 (GPU (auto))"]);
        assert!(warning.is_none());
    }

    /// Каждое `*_idx`-поле state расшифровано в action=nodes — системный
    /// промпт это обещает. У VAE Encode таблицы не было, и агент решил, что
    /// `device_idx: 0` — «основной CUDA».
    #[test]
    fn every_idx_state_field_has_enum_hint() {
        // Не варианты дропдауна, а числа: номер кадра LTX Image (0 — i2v,
        // >0 — позиция ключевого кадра).
        const NOT_ENUMS: &[&str] = &["ltx_image.frame_idx"];
        let mut missing = Vec::new();
        for kind in NodeKind::ALL {
            let Some(js) = state_example(*kind) else { continue };
            let v: serde_json::Value = serde_json::from_str(&js).expect("state json");
            let Some(data) = v.get("data").and_then(|d| d.as_object()) else { continue };
            let hints = enum_hints(*kind);
            for key in data.keys().filter(|k| k.ends_with("_idx")) {
                let name = format!("{}.{key}", kind_slug(*kind));
                if !hints.iter().any(|(f, _)| *f == key.as_str()) && !NOT_ENUMS.contains(&name.as_str()) {
                    missing.push(name);
                }
            }
        }
        assert!(missing.is_empty(), "*_idx fields without decoding: {missing:?}");
    }

    /// VAE Encode по умолчанию на GPU, как ACE-Step Checkpoint.
    #[test]
    fn acestep_vae_encode_defaults_to_gpu() {
        use crate::pages::node_editor::nodes::acestep;
        let js = state_example(NodeKind::AceStepVaeEncode).expect("state");
        let v: serde_json::Value = serde_json::from_str(&js).expect("state json");
        let idx = v["data"]["device_idx"].as_u64().expect("device_idx") as usize;
        assert_eq!(acestep::DEVICE_OPTIONS[idx], "GPU (auto)");
    }
}
