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

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::core::Point;

use crate::agent::state::{AttachmentKind, ChatMsg, MsgAttachment};
use crate::context::AppCtx;
use crate::pages::node_editor::registry::{self, NodeCategory};
use crate::pages::node_editor::state::NodeEditorCtx;
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::types::{Connection, NodeKind, PortKind, PortsSpec, PortSide};
use crate::syn_chat::attach::blobs;
use crate::syn_chat::SynChatCtx;
use crate::templates::{self, convert, model::NodeStateData, ConnData, NodeData, Template, TemplateKind};

use super::executor::ToolError;

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
                "action=run доступен только основному агенту чата (не subagent): \
                 попроси главного агента запустить граф"
                    .to_string(),
            ),
            other => Err(format!(
                "неизвестный action «{other}» (list | nodes | open | graph | apply | save_template | run)"
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
        .ok_or_else(|| "нет активного чата".to_string())
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
        .ok_or_else(|| "служебная вкладка не создалась".to_string())?;
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

    out.push_str("--- Шаблоны пайплайнов ---\n");
    for t in templates::list_all() {
        out.push_str(&format!(
            "{} · «{}»{} · нод: {}{}\n",
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
            "--- Модели в каталоге {dir_s} ({label}) --- (пути: {dir_s}/<имя>)\n"
        ));
        let inventory = models_inventory(&dir);
        if inventory.is_empty() {
            out.push_str(
                "(пусто или каталог не существует — путь задаётся в Настройки → AI-модели)\n",
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

    out.push_str("--- Вложения чата (для attachment:<имя|sha|last>) ---\n");
    let atts = chat_attachments();
    if atts.is_empty() {
        out.push_str("(нет)\n");
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

    out.push_str("--- Служебная вкладка ---\n");
    match agent_ctx()? {
        // Без state и связей: полный снимок — action=graph. В list он дублировал
        // ответ open и съедал бюджет, вытесняя инвентарь моделей.
        Some(ctx) => out.push_str(&graph_brief(&ctx)?),
        None => out.push_str("(ещё не создана — открой шаблон через action=open)\n"),
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
        return vec![("основной", main)];
    };
    let hf_dir = crate::config::resolve_hf_cache_dir(&hf.cache_dir.get_untracked());
    let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (main_c, hf_c) = (canon(&main), canon(&hf_dir));
    let mut dirs = vec![("основной", main)];
    if hf_c != main_c && !hf_c.starts_with(&main_c) {
        dirs.push(("кэш HF", hf_dir));
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
                    out.push(format!("{} · HF-каталог модели", p.display()));
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
        lines.push(format!("… и ещё {extra} файлов"));
    }
    lines
}

fn attachment_kind_label(k: AttachmentKind) -> &'static str {
    match k {
        AttachmentKind::Image => "картинка",
        AttachmentKind::Video => "видео",
        AttachmentKind::Audio => "аудио",
        AttachmentKind::Document => "документ",
        AttachmentKind::Other => "файл",
    }
}

/// Все вложения user-сообщений активного чата, свежие в конце.
fn chat_attachments() -> Vec<MsgAttachment> {
    let chat = use_context::<SynChatCtx>();
    let msgs: Vec<ChatMsg> = chat.messages.get_untracked();
    msgs.iter()
        .flat_map(|m| m.attachments.iter().cloned())
        .collect()
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
        format!("{joined} (динамические)")
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
        syn_checkpoint, voxcpm2,
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
        NodeKind::AceStepGenerate => vec![
            ("mode_idx", acestep::generate::MODE_OPTIONS),
            ("keyscale_idx", acestep::generate::KEYSCALE_OPTIONS),
            ("timesig_idx", acestep::generate::TIMESIG_OPTIONS),
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
            "{} · {} · есть запуск: {}\n",
            kind_slug(*kind),
            meta.title,
            if meta.on_run.is_some() { "да" } else { "нет" }
        ));
    }
    out
}

const FILTER_HINT: &str = "filter ищет по kind/названию/категории (не по именам \
     полей — те смотри в примере state). Несколько значений через пробел или \
     запятую объединяются: «ltx_checkpoint ltx_sampler_stage1» вернёт обе ноды, \
     «ltx» — всё семейство.";

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
            "Все виды нод (kind · название · категория). Детали (порты, \
             state-JSON, расшифровка *_idx) — повтори с filter. {FILTER_HINT}\n"
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
        if !tokens.iter().any(|t| hay.contains(t)) {
            continue;
        }
        matched += 1;
        out.push_str(&format!(
            "--- {} · «{}» · {} ---\n",
            slug,
            meta.title,
            category_path(meta)
        ));
        out.push_str(&format!("входы: {}\n", ports_line(meta.inputs)));
        out.push_str(&format!("выходы: {}\n", ports_line(meta.outputs)));
        if !meta.fields.is_empty() {
            let fields = meta
                .fields
                .iter()
                .map(|f| format!("{} ({:?})", f.name, f.ty))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("поля: {fields}\n"));
        }
        out.push_str(&format!(
            "запуск (on_run): {}\n",
            if meta.on_run.is_some() { "да" } else { "нет (реактивная)" }
        ));
        if let Some(state) = state_example(*kind) {
            out.push_str(&format!("state (пример с дефолтами): {state}\n"));
        }
        for line in enum_hints_lines(*kind) {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if matched == 0 {
        // Не ошибка: пустой ответ гонит агента по кругу с вариациями фильтра.
        // Отдаём то, ради чего он и звал инструмент, — список видов нод.
        return Ok(format!(
            "по фильтру «{filter}» нод не найдено. {FILTER_HINT}\n{}",
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
        .ok_or("для action=open нужен параметр template (id из action=list)")?;
    let all = templates::list_all();
    let t = all
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("шаблона «{id}» нет (см. action=list)"))?;

    let ctx = agent_ctx_ensure(&t.name)?;
    convert::load_into_ctx(&ctx, t);
    let mut out = format!("шаблон «{}» загружен в служебную вкладку\n", t.name);
    out.push_str(&graph_summary(&ctx)?);
    // Чем заполнять граф — говорим сразу: встроенные шаблоны путей не несут,
    // и агент иначе выясняет это только на pre-check прогона, потратив ходы.
    let missing = missing_model_paths(&ctx);
    if !missing.is_empty() {
        out.push_str("не заполнены пути моделей (apply set_state, пути — из action=list):\n");
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
                "нода {} ({}): {}",
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
        None => Err("служебной вкладки ещё нет — открой шаблон (action=open) или собери граф (action=apply)".into()),
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
        "граф: нод {}, связей {} (полный снимок — action=graph)\n",
        nodes.len(),
        conns.len()
    );
    for n in &nodes {
        out.push_str(&format!(
            "[{}] {} · {}{}\n",
            n.id,
            kind_slug(n.kind),
            registry::meta(n.kind).title,
            if n.enabled { "" } else { " · ВЫКЛ" }
        ));
    }
    Ok(out)
}

fn graph_summary(ctx: &NodeEditorCtx) -> Result<String, String> {
    const STATE_CLIP: usize = 700;
    let (nodes, conns, _viewport) = convert::snapshot(ctx);
    let mut out = format!("граф: нод {}, связей {}\n", nodes.len(), conns.len());
    for n in &nodes {
        let meta = registry::meta(n.kind);
        let mut line = format!(
            "[{}] {} · {}{}",
            n.id,
            kind_slug(n.kind),
            meta.title,
            if n.enabled { "" } else { " · ВЫКЛ" }
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
        out.push_str("связи:\n");
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
    let ctx = agent_ctx_ensure("Агент")?;
    let mut notes: Vec<String> = Vec::new();

    // attachment:-ссылки резолвятся по всему payload'у до парсинга структур.
    let mut v = v.clone();
    resolve_attachment_uris(&mut v, &mut notes);

    if let Some(graph) = v.get("graph") {
        let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("replace");
        let graph = unwrap_json_string(graph, "graph")?;
        let template = template_from_value(&graph)?;
        match mode {
            "replace" => convert::load_into_ctx(&ctx, &template),
            "merge" => convert::apply_to_ctx(&ctx, &template, Point::new(0.0, 0.0)),
            other => return Err(format!("mode «{other}» неизвестен (replace | merge)")),
        }
        notes.push(format!(
            "graph применён ({mode}): нод {}, связей {}",
            template.nodes.len(),
            template.connections.len()
        ));
    }

    // Ошибки по отдельным элементам не отменяют весь apply: половина графа,
    // применённая из четырёх нод, — это прогресс, а полный откат заставлял
    // агента пересобирать вызов целиком и терять уже верные куски.
    let mut problems: Vec<String> = Vec::new();

    if let Some(items) = coerce_array(&v, "set_state")? {
        for (i, item) in items.iter().enumerate() {
            match apply_one_state(&ctx, item) {
                Ok(note) => notes.push(note),
                Err(e) => problems.push(format!("set_state[{i}]: {e}")),
            }
        }
    }

    if let Some(items) = coerce_array(&v, "connect")? {
        for (i, item) in items.iter().enumerate() {
            let res = conn_from_value(item, &ctx).and_then(|c| {
                add_connection(&ctx, &c)?;
                Ok(format!(
                    "связь {}.{} → {}.{}",
                    c.from_node, c.from_port, c.to_node, c.to_port
                ))
            });
            match res {
                Ok(note) => notes.push(note),
                Err(e) => problems.push(format!("connect[{i}]: {e}")),
            }
        }
    }

    if let Some(items) = coerce_array(&v, "disconnect")? {
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
                "разъединено {}: {}.{} → {}.{}",
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
            "apply без изменений — передано только [{got}]. Правки идут в \
             set_state / connect / disconnect / graph, формат set_state: \
             [{{\"node\": <id|имя вида>, \"state\": {{\"kind\": \"…\", \"data\": {{…}}}}}}]\n\
             Если граф уже собран — следующий шаг action=run.\n{}",
            graph_brief(&ctx)?
        ));
    }

    let mut out = notes.join("\n");
    out.push('\n');
    if !problems.is_empty() {
        out.push_str("⚠ не применено:\n");
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
        "Следующий шаг: action=run (free_vram=true, если по system status \
         VRAM не хватает). Проверить state целиком — action=graph.\n",
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

/// Собрать Template из agent-JSON `{nodes, connections}`. Послабления к
/// строгой схеме: отсутствующие `id` нумеруются по порядку, отсутствующие
/// `pos` раскладываются сеткой — LLM не обязан придумывать координаты.
fn template_from_value(graph: &serde_json::Value) -> Result<Template, String> {
    let mut graph = graph.clone();
    if let Some(nodes) = graph.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for (i, node) in nodes.iter_mut().enumerate() {
            let Some(obj) = node.as_object_mut() else {
                return Err(format!("nodes[{i}] — не объект"));
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
fn unwrap_json_string(v: &serde_json::Value, field: &str) -> Result<serde_json::Value, String> {
    let Some(raw) = v.as_str() else {
        return Ok(v.clone());
    };
    let text = raw.trim();
    serde_json::from_str(text).map_err(|e| {
        format!(
            "{field} пришёл строкой, и это не разбирается как JSON: {e}{}. \
             Передавай значение структурой (массивом/объектом), а не текстом.",
            json_error_context(text, &e)
        )
    })
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
    format!(" (около: …{frag}…)")
}

/// Массив-аргумент: принимает массив, одиночный объект и строку с JSON.
fn coerce_array(
    v: &serde_json::Value,
    field: &str,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    let Some(raw) = v.get(field) else {
        return Ok(None);
    };
    let val = unwrap_json_string(raw, field)?;
    match val {
        serde_json::Value::Array(a) => Ok(Some(a)),
        // Один элемент без обёртки — тоже понятное намерение.
        serde_json::Value::Object(_) => Ok(Some(vec![val])),
        other => Err(format!(
            "{field}: ожидался массив объектов, пришло {}",
            json_type_name(&other)
        )),
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "число",
        serde_json::Value::String(_) => "строка",
        serde_json::Value::Array(_) => "массив",
        serde_json::Value::Object(_) => "объект",
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
        return Err("id ноды — число (2) или имя вида ноды («h3_checkpoint»)".to_string());
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
            "ноды «{name}» нет в графе; есть: {}",
            nodes
                .iter()
                .map(|n| format!("{} ({})", n.id.0, kind_slug(n.kind)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => Err(format!(
            "«{name}» в графе не одна — укажи числовой id: {}",
            hits.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Связь: `from_node`/`to_node` принимают то же, что и `set_state[].node`.
/// Один элемент `set_state`: резолв ноды, разбор state, проверка kind,
/// применение. Ошибка описывает конкретный элемент, а не весь вызов.
fn apply_one_state(ctx: &NodeEditorCtx, item: &serde_json::Value) -> Result<String, String> {
    let node_ref = item
        .get("node")
        .ok_or("нужно поле node (id ноды или имя вида)")?;
    let node_id = resolve_node_ref(node_ref, ctx)?;
    let state_v = item.get("state").cloned().ok_or("нужно поле state")?;
    let state_v = unwrap_json_string(&state_v, "state")?;
    let state: NodeStateData = serde_json::from_value(state_v)
        .map_err(|e| format!("нода {node_id}: не разобрать state: {e}"))?;
    let nodes = ctx.nodes.get_untracked();
    let node = nodes
        .iter()
        .find(|n| n.id.0 == node_id)
        .ok_or_else(|| format!("ноды {node_id} нет в графе"))?;
    // Совпадение варианта state с kind ноды проверяем заранее:
    // apply_state_to_runtime при несовпадении молча no-op'ает, а агенту
    // нужна честная ошибка.
    let expected = registry::default_runtime(node.kind)
        .lock()
        .ok()
        .and_then(|g| convert::runtime_to_state(&g))
        .and_then(|s| variant_tag(&s));
    let got = variant_tag(&state);
    if expected != got {
        return Err(format!(
            "нода {node_id} ({}) ждёт state kind={}, а пришёл {}",
            kind_slug(node.kind),
            expected.unwrap_or_else(|| "—".into()),
            got.unwrap_or_else(|| "—".into())
        ));
    }
    if let Ok(rt) = node.runtime.lock() {
        convert::apply_state_to_runtime(&rt, &state);
    }
    Ok(format!("state ноды {node_id} обновлён"))
}

fn conn_from_value(v: &serde_json::Value, ctx: &NodeEditorCtx) -> Result<ConnData, String> {
    let mut v = v.clone();
    for field in ["from_node", "to_node"] {
        let Some(raw) = v.get(field) else {
            return Err(format!("связь: нужно поле {field}"));
        };
        let id = resolve_node_ref(raw, ctx).map_err(|e| format!("связь.{field}: {e}"))?;
        v[field] = serde_json::json!(id);
    }
    serde_json::from_value(v).map_err(|e| format!("связь: {e}"))
}

/// Добавить связь в граф вкладки с той же валидацией, что у `complete_wire`:
/// имена портов резолвятся в `&'static str` по registry, самосвязи и дубли
/// отбрасываются.
fn add_connection(ctx: &NodeEditorCtx, c: &ConnData) -> Result<(), String> {
    if c.from_node == c.to_node {
        return Err(format!("связь {}→{}: самосвязь запрещена", c.from_node, c.to_node));
    }
    let nodes = ctx.nodes.get_untracked();
    let from_kind = nodes
        .iter()
        .find(|n| n.id.0 == c.from_node)
        .map(|n| n.kind)
        .ok_or_else(|| format!("connect: ноды {} нет в графе", c.from_node))?;
    let to_kind = nodes
        .iter()
        .find(|n| n.id.0 == c.to_node)
        .map(|n| n.kind)
        .ok_or_else(|| format!("connect: ноды {} нет в графе", c.to_node))?;
    let from_port = convert::resolve_port_name(Some(from_kind), PortSide::Output, &c.from_port)
        .ok_or_else(|| {
            format!(
                "connect: у {} нет выхода «{}» (см. action=nodes filter={})",
                kind_slug(from_kind),
                c.from_port,
                kind_slug(from_kind)
            )
        })?;
    let to_port = convert::resolve_port_name(Some(to_kind), PortSide::Input, &c.to_port)
        .ok_or_else(|| {
            format!(
                "connect: у {} нет входа «{}» (см. action=nodes filter={})",
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
                        "attachment:{r} — вложение не найдено (см. action=list), строка оставлена как есть"
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

fn find_attachment(r: &str) -> Option<MsgAttachment> {
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
        .ok_or("для action=save_template нужен name")?;
    let ctx = agent_ctx()?
        .ok_or("служебной вкладки нет — нечего сохранять")?;
    let (nodes, connections, viewport) = convert::snapshot(&ctx);
    if nodes.is_empty() {
        return Err("граф пуст — нечего сохранять".into());
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
        "сохранён кастомный шаблон «{}» (id: {})",
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
        let items = coerce_array(&v, "set_state").expect("строка разбирается").expect("есть");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["node"], 13);

        // Одиночный объект без массива — тоже понятное намерение.
        let v = serde_json::json!({"set_state": {"node": 2, "state": {}}});
        assert_eq!(coerce_array(&v, "set_state").unwrap().unwrap().len(), 1);

        // Поля нет — не ошибка.
        assert!(coerce_array(&serde_json::json!({}), "set_state").unwrap().is_none());
    }

    /// Битый JSON внутри строки объясняется, а не прячется за «apply без
    /// изменений»: агенту нужна позиция ошибки, иначе он уходит в перебор.
    #[test]
    fn coerce_array_reports_broken_json_string() {
        let v = serde_json::json!({"set_state": "[{\"prompt\", \"\"}]"});
        let err = coerce_array(&v, "set_state").expect_err("битый JSON");
        assert!(err.contains("set_state пришёл строкой"), "{err}");
        assert!(err.contains("column"), "{err}");
        assert!(err.contains("около:"), "нужен фрагмент вокруг ошибки: {err}");
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
    /// моделей и сводкой графа весь ответ должен влезать в лимит истории
    /// хода (`session::HISTORY_TOOL_RESULT_CHARS` = 8000), иначе из середины
    /// вырезается именно то, ради чего инструмент звали.
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
        assert!(out.contains("«LTX Checkpoint»"), "{out}");
        assert!(out.contains("«LTX Sampler Stage1»"), "{out}");
        assert!(!out.contains("«H3 Checkpoint»"), "{out}");
    }

    /// Промах фильтра — не ошибка, а список видов нод: иначе агент крутит
    /// вариации фильтра до guard'а повторов.
    #[test]
    fn nodes_filter_miss_returns_catalog() {
        let out = nodes_impl(&serde_json::json!({"filter": "quant_dit_idx keep_gemma"}))
            .expect("промах не ошибка");
        assert!(out.contains("нод не найдено"), "{out}");
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

    /// Неизвестные ключи внутри state.data не валят парсинг (serde default).
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
        assert!(inv.contains("HF-каталог"), "{inv}");
        assert!(!inv.contains("readme.txt"), "{inv}");
        let _ = std::fs::remove_dir_all(&tmp);
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
}
