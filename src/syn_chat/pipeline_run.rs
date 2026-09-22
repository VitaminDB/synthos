//! `pipelines action=run` — прогон графа служебной вкладки из agent-loop.
//!
//! Здесь живёт «графовая» половина запуска: валидация и автозаполнение
//! путей save-нод (prepare), старт секвенсера (start), отмена, сбор
//! артефактов с диска в CAS-вложения и итоговый envelope. Жизненный цикл
//! чат-LLM (free_vram: выгрузка перед прогоном, загрузка после) — НЕ здесь:
//! им управляет `session::run_pipeline_tool`, потому что владение
//! `Arc<LoadedSynModel>` и guard KV-слота принадлежат agent-loop'у.
//!
//! Почему run не в `agent::tools::pipelines`: обычные инструменты
//! исполняются через `tools::execute` и ничего не знают о модели. Прогон же
//! обязан уметь пережить выгрузку LLM — subagent'ам он поэтому недоступен.

use std::path::{Path, PathBuf};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;

use crate::agent::schema::ChatToolCall;
use crate::agent::state::MsgAttachment;
use crate::models;
use crate::pages::node_editor::registry;
use crate::pages::node_editor::run_controls::{self, RunEnd, RunOutcome, StartRunError};
use crate::pages::node_editor::state::NodeEditorCtx;
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::types::{NodeKind, NodeRuntime};
use crate::syn_chat::attach::ingest;
use crate::syn_chat::SynChatCtx;

/// Распарсенный вызов `pipelines action=run`.
pub struct RunRequest {
    pub free_vram: bool,
    /// Метка запуска — имя подкаталога результатов. Из `run_id` аргументов
    /// либо `run-<epoch>`.
    pub run_label: String,
}

/// `Some(RunRequest)` — если это вызов `pipelines` c `action=run`.
/// Канальные модели пишут квалифицированные имена (`x.pipelines`) — хвост
/// после точки тоже считается.
///
/// Аргументы прогоняются через [`normalize_args`] — тот же нормализатор, что
/// у `tools::execute`. Модели любят обрамлять значения переводами строк
/// (`{"action":"\nrun\n"}`); без trim'а такой вызов не опознавался здесь,
/// проваливался в `tools::execute`, где уже нормализованный `action` попадал
/// в subagent-ветку — и главный агент чата получал «run доступен только
/// основному агенту».
pub fn parse_run_call(call: &ChatToolCall) -> Option<RunRequest> {
    let name = call.function.name.as_deref().unwrap_or("").trim();
    if name != "pipelines" && !name.ends_with(".pipelines") {
        return None;
    }
    let args = crate::agent::tools::executor::normalize_args(
        call.function.arguments.as_deref().unwrap_or(""),
    );
    let v: serde_json::Value = serde_json::from_str(&args).ok()?;
    if v.get("action").and_then(|x| x.as_str()) != Some("run") {
        return None;
    }
    // Строковые «true»/«1» — та же манера моделей, что и JSON-в-строке у apply.
    let free_vram = v
        .get("free_vram")
        .map(|x| match x {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => {
                matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "да")
            }
            serde_json::Value::Number(n) => n.as_i64().is_some_and(|i| i != 0),
            _ => false,
        })
        .unwrap_or(false);
    let run_label = v
        .get("run_id")
        .and_then(|x| x.as_str())
        .map(sanitize_label)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("run-{}", epoch_secs()));
    Some(RunRequest { free_vram, run_label })
}

fn sanitize_label(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(48)
        .collect()
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Каталог результатов агентских прогонов:
/// `~/.local/share/synthos/outputs/<chat_id>/<run_label>/`.
fn outputs_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/synthos/outputs")
}

/// Save-нода прогона: куда она запишет файл. `pre` — (len, mtime) файла до
/// прогона: отличаем свежезаписанный результат от лежавшего там раньше.
pub struct PlannedArtifact {
    pub node_id: u64,
    pub title: &'static str,
    pub path: PathBuf,
    pre: Option<(u64, std::time::SystemTime)>,
}

pub struct Prepared {
    pub planned: Vec<PlannedArtifact>,
    /// Сколько enabled-нод с on_run в графе.
    pub runnable: usize,
    /// Замечания, которые не мешают запуску, но объясняют плохой результат
    /// заранее (например, dev-чекпойнт LTX на distilled-расписании).
    pub warnings: Vec<String>,
}

/// Замечания по графу перед прогоном.
///
/// LTX-стадии в synaptix идут по **distilled**-расписанию (`DISTILLED_SIGMAS`
/// — 8 Euler-шагов на stage1, 3 на stage2). Недистиллированный чекпойнт
/// (`ltx-2.3-22b-dev`) на восьми шагах даёт мутную картинку — «как будто
/// шагов не хватает». Число шагов у ноды не настраивается, поэтому чекпойнт
/// и расписание обязаны совпадать; ловим несовпадение до прогона.
fn graph_warnings(nodes: &[crate::pages::node_editor::types::NodeInstance]) -> Vec<String> {
    let mut out = Vec::new();
    for n in nodes {
        if !n.enabled.get_untracked() {
            continue;
        }
        let Ok(rt) = n.runtime.lock() else { continue };
        let NodeRuntime::LtxCheckpoint { model_path, .. } = &*rt else {
            continue;
        };
        let Some(path) = model_path.get_untracked() else {
            continue;
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !name.contains("distilled") {
            out.push(tr!(
                "chat.pipeline.warning.ltx_non_distilled",
                node_id = n.id.0,
                name = name
            ));
        }
    }
    out
}

/// Ctx служебной вкладки текущего чата (main thread).
fn agent_tab_ctx() -> Result<NodeEditorCtx, String> {
    let chat = use_context::<SynChatCtx>();
    let chat_id = chat
        .active_chat_id
        .get_untracked()
        .ok_or_else(|| tr!("chat.pipeline.error.no_active_chat"))?;
    let ws = use_context::<EditorWorkspace>();
    let tab_id = ws
        .agent_tab_for_chat(&chat_id)
        .ok_or_else(|| tr!("chat.pipeline.error.no_agent_tab"))?;
    ws.tabs
        .get_untracked()
        .iter()
        .find(|t| t.id == tab_id)
        .map(|t| t.ctx)
        .ok_or_else(|| tr!("chat.pipeline.error.agent_tab_gone"))
}

/// Проверить граф и автозаполнить пути save-нод ДО выгрузки LLM — если
/// запускать нечего, незачем гонять модель туда-обратно.
pub async fn prepare(run_label: &str) -> Result<Prepared, String> {
    let label = run_label.to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let _ = tx.send(prepare_impl(&label));
    });
    rx.await.map_err(|e| e.to_string())?
}

fn prepare_impl(run_label: &str) -> Result<Prepared, String> {
    let chat = use_context::<SynChatCtx>();
    let chat_id = chat
        .active_chat_id
        .get_untracked()
        .ok_or_else(|| tr!("chat.pipeline.error.no_active_chat"))?;
    let ctx = agent_tab_ctx()?;
    let nodes = ctx.nodes.get_untracked();
    let runnable = nodes
        .iter()
        .filter(|n| n.enabled.get_untracked() && registry::meta(n.kind).on_run.is_some())
        .count();
    if runnable == 0 {
        return Err(tr!("chat.pipeline.error.no_runnable_nodes"));
    }

    // Пустые чекпойнты ловим ДО прогона (и до выгрузки LLM): нода всё равно
    // упадёт, а агент потеряет ход и цикл unload/reload. Слот-нодам с
    // подключённым входом `model` (Syn Checkpoint) собственный model_path
    // не нужен — хэндл его переопределяет.
    let conns = ctx.connections.get_untracked();
    let mut missing: Vec<String> = Vec::new();
    for n in &nodes {
        if !n.enabled.get_untracked() {
            continue;
        }
        let has_model_input = conns
            .iter()
            .any(|c| c.to_node == n.id && c.to_port == "model");
        if let Ok(rt) = n.runtime.lock() {
            for field in rt.missing_model_paths() {
                if field == "model_path" && has_model_input {
                    continue;
                }
                missing.push(tr!(
                    "chat.pipeline.error.missing_field",
                    node_id = n.id.0,
                    node_title = registry::meta(n.kind).title,
                    field = field
                ));
            }
        }
    }
    // Upscaler нужен только графам со стадией Upscale — проверяем по факту
    // её наличия, а не в missing_model_paths (там он был бы ложной тревогой
    // для retake/a2v/lipdub). Ловим до прогона: иначе граф падает уже после
    // выгрузки LLM и загрузки 46-гигабайтного DiT.
    let needs_upscaler = nodes
        .iter()
        .any(|n| n.enabled.get_untracked() && n.kind == NodeKind::LtxUpscale);
    if needs_upscaler {
        for n in &nodes {
            if !n.enabled.get_untracked() {
                continue;
            }
            if n.runtime.lock().is_ok_and(|rt| rt.ltx_upscaler_missing()) {
                missing.push(tr!(
                    "chat.pipeline.error.missing_upscaler",
                    node_id = n.id.0,
                    node_title = registry::meta(n.kind).title
                ));
            }
        }
    }

    if !missing.is_empty() {
        return Err(tr!("chat.pipeline.error.missing_paths", list = missing.join("\n")));
    }

    let out_dir = outputs_dir().join(&chat_id).join(run_label);
    let mut planned = Vec::new();
    for n in &nodes {
        if !n.enabled.get_untracked() {
            continue;
        }
        // Сигналы путей у save-нод разнотипные: String у LTX/SaveToFile,
        // Option<PathBuf> у H3 — прячем за мини-обёрткой.
        enum SavePathSig {
            Str(syngui::signal::RwSignal<String>),
            OptPath(syngui::signal::RwSignal<Option<PathBuf>>),
        }
        impl SavePathSig {
            fn get(&self) -> String {
                match self {
                    SavePathSig::Str(s) => s.get_untracked(),
                    SavePathSig::OptPath(s) => s
                        .get_untracked()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                }
            }
            fn set(&self, p: &Path) {
                match self {
                    SavePathSig::Str(s) => s.set(p.display().to_string()),
                    SavePathSig::OptPath(s) => s.set(Some(p.to_path_buf())),
                }
            }
        }
        let (path_sig, default_ext) = {
            let Ok(rt) = n.runtime.lock() else { continue };
            match &*rt {
                NodeRuntime::LtxVideoSave { path, .. } => (SavePathSig::Str(*path), "mp4"),
                NodeRuntime::H3VideoSave { path, .. } => (SavePathSig::OptPath(*path), "mp4"),
                NodeRuntime::ImageSave { path, .. } => (SavePathSig::OptPath(*path), "png"),
                NodeRuntime::SaveToFile { path, .. } => (SavePathSig::Str(*path), "wav"),
                _ => continue,
            }
        };
        let meta = registry::meta(n.kind);
        let cur = path_sig.get();
        // Пустой или относительный путь (дефолт вроде «h3.mp4») заменяем
        // на каталог результатов — пользовательский абсолютный не трогаем.
        let needs_fill = cur.trim().is_empty() || !Path::new(cur.trim()).is_absolute();
        let final_path = if needs_fill {
            std::fs::create_dir_all(&out_dir).map_err(|e| {
                tr!("chat.pipeline.error.mkdir_failed", dir = out_dir.display(), error = e)
            })?;
            let p = out_dir.join(format!("node{}.{}", n.id.0, default_ext));
            path_sig.set(&p);
            p
        } else {
            PathBuf::from(cur.trim())
        };
        let pre = std::fs::metadata(&final_path)
            .ok()
            .and_then(|m| m.modified().ok().map(|t| (m.len(), t)));
        planned.push(PlannedArtifact {
            node_id: n.id.0,
            title: meta.title,
            path: final_path,
            pre,
        });
    }
    Ok(Prepared { planned, runnable, warnings: graph_warnings(&nodes) })
}

/// Запустить прогон служебной вкладки. Возвращает receiver итога и число
/// стартовавших корней.
pub async fn start() -> Result<(tokio::sync::oneshot::Receiver<RunOutcome>, usize), String> {
    let (otx, orx) = tokio::sync::oneshot::channel::<RunOutcome>();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<usize, String>>();
    run_on_main_thread(move || {
        let res = (|| {
            let ctx = agent_tab_ctx()?;
            run_controls::start_run(ctx, Some(otx)).map_err(|e| match e {
                StartRunError::NoRunnableNodes => {
                    tr!("chat.pipeline.error.no_runnable_nodes_short")
                }
                StartRunError::CycleInGraph => {
                    tr!("chat.pipeline.error.cycle_in_graph")
                }
            })
        })();
        let _ = tx.send(res);
    });
    let started = rx.await.map_err(|e| e.to_string())??;
    Ok((orx, started))
}

/// Отменить текущий прогон (abort хода агента): очередь сбрасывается,
/// cancel-флаги активных нод взводятся.
pub async fn cancel_current() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        run_controls::cancel_active_run();
        let _ = tx.send(());
    });
    let _ = rx.await;
}

/// Что показывает нода-просмотрщик после прогона. Данные живут только в
/// памяти ноды (кадры от VAE Decode, буфер от TTS), файла на диске нет —
/// материализуем их сами, иначе граф без save-ноды не отдаёт в чат ничего,
/// хотя результат уже посчитан и играет в редакторе.
enum ViewerOutput {
    Video {
        frames: std::sync::Arc<crate::pages::node_editor::types::LtxFrames>,
        audio: Option<std::sync::Arc<syngui::audio::AudioBuffer>>,
    },
    Audio(std::sync::Arc<syngui::audio::AudioBuffer>),
    /// Картинка VAE Decode (FLUX, FLUX.2, Qwen-Image, SDXL), которую никто не сохранил.
    Image(std::sync::Arc<crate::pages::node_editor::types::ImageData>),
    /// Плеер, которому дали файл (а не память) — прикладываем как есть.
    File(PathBuf),
}

struct PlannedViewer {
    node_id: u64,
    title: &'static str,
    out: ViewerOutput,
}

/// Снять с нод-просмотрщиков то, что они показывают (main thread: сигналы).
/// `skip_video`/`skip_audio` — что уже пришло от save-нод: дублировать один
/// и тот же результат двумя вложениями незачем.
fn planned_viewers(
    skip_video: bool,
    skip_audio: bool,
    skip_image: bool,
) -> (Option<String>, Vec<PlannedViewer>) {
    let chat_id = use_context::<SynChatCtx>().active_chat_id.get_untracked();
    let Ok(ctx) = agent_tab_ctx() else {
        return (chat_id, Vec::new());
    };
    let mut out = Vec::new();
    for n in ctx.nodes.get_untracked().iter() {
        if !n.enabled.get_untracked() {
            continue;
        }
        let Ok(rt) = n.runtime.lock() else { continue };
        let title = registry::meta(n.kind).title;
        match &*rt {
            NodeRuntime::FfmpegPlayer { frames_in, audio_in, current_path, .. } => {
                if skip_video {
                    continue;
                }
                let frames = frames_in.lock().ok().and_then(|g| g.clone());
                if let Some(frames) = frames {
                    let audio = audio_in.lock().ok().and_then(|g| g.clone());
                    out.push(PlannedViewer {
                        node_id: n.id.0,
                        title,
                        out: ViewerOutput::Video { frames, audio },
                    });
                } else if let Some(p) = current_path.get_untracked() {
                    out.push(PlannedViewer { node_id: n.id.0, title, out: ViewerOutput::File(p) });
                }
            }
            NodeRuntime::FluxVaeDecode { out: image_out, .. }
            | NodeRuntime::Flux2VaeDecode { out: image_out, .. }
            | NodeRuntime::QwenImageVaeDecode { out: image_out, .. }
            | NodeRuntime::QwenImage21VaeDecode { out: image_out, .. }
            | NodeRuntime::SdxlVaeDecode { out: image_out, .. } => {
                if skip_image {
                    continue;
                }
                if let Some(img) = image_out.lock().ok().and_then(|g| g.clone()) {
                    out.push(PlannedViewer { node_id: n.id.0, title, out: ViewerOutput::Image(img) });
                }
            }
            NodeRuntime::AudioPlayer { pcm_view, .. } => {
                if skip_audio {
                    continue;
                }
                if let Some(buf) = pcm_view.get_untracked() {
                    out.push(PlannedViewer {
                        node_id: n.id.0,
                        title,
                        out: ViewerOutput::Audio(buf),
                    });
                }
            }
            _ => {}
        }
    }
    (chat_id, out)
}

/// Материализовать результаты просмотрщиков в каталог прогона и приложить к
/// сообщению. Вызывается после [`collect_artifacts`]: `have` — то, что уже
/// собрано с save-нод.
pub async fn collect_viewer_outputs(
    run_label: &str,
    have: &[MsgAttachment],
) -> (Vec<MsgAttachment>, Vec<String>) {
    let skip_video = have.iter().any(|a| a.kind == crate::agent::state::AttachmentKind::Video);
    let skip_audio = have.iter().any(|a| a.kind == crate::agent::state::AttachmentKind::Audio);
    let skip_image = have.iter().any(|a| a.kind == crate::agent::state::AttachmentKind::Image);
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let _ = tx.send(planned_viewers(skip_video, skip_audio, skip_image));
    });
    let Ok((chat_id, planned)) = rx.await else {
        return (Vec::new(), Vec::new());
    };
    if planned.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let dir = outputs_dir()
        .join(chat_id.unwrap_or_else(|| "chat".to_string()))
        .join(run_label);
    let mut atts = Vec::new();
    let mut lines = Vec::new();
    for p in planned {
        let path = match &p.out {
            ViewerOutput::Video { frames, audio } => {
                let out = dir.join(format!("node{}_preview.mp4", p.node_id));
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    lines.push(tr!("chat.pipeline.viewer.mkdir_failed", title = p.title, node_id = p.node_id, error = e));
                    continue;
                }
                match crate::pages::node_editor::nodes::ltx::video_save::encode_mp4(
                    frames,
                    audio.as_deref(),
                    &out,
                    None,
                ) {
                    Ok(()) => out,
                    Err(e) => {
                        lines.push(tr!("chat.pipeline.viewer.encode_failed", title = p.title, node_id = p.node_id, error = e));
                        continue;
                    }
                }
            }
            ViewerOutput::Audio(buf) => {
                let out = dir.join(format!("node{}_preview.wav", p.node_id));
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    lines.push(tr!("chat.pipeline.viewer.mkdir_failed", title = p.title, node_id = p.node_id, error = e));
                    continue;
                }
                match crate::pages::node_editor::nodes::ltx::video_save::write_wav(&out, buf) {
                    Ok(()) => out,
                    Err(e) => {
                        lines.push(tr!("chat.pipeline.viewer.wav_write_failed", title = p.title, node_id = p.node_id, error = e));
                        continue;
                    }
                }
            }
            ViewerOutput::Image(img) => {
                let out = dir.join(format!("node{}_preview.png", p.node_id));
                match crate::pages::node_editor::nodes::image::write_image(img, &out) {
                    Ok(()) => out,
                    Err(e) => {
                        lines.push(tr!("chat.pipeline.viewer.encode_failed", title = p.title, node_id = p.node_id, error = e));
                        continue;
                    }
                }
            }
            ViewerOutput::File(p) => p.clone(),
        };
        match ingest::ingest(&path) {
            Ok(a) => {
                lines.push(tr!(
                    "chat.pipeline.viewer.attached",
                    title = p.title,
                    node_id = p.node_id,
                    path = path.display(),
                    size = models::human_bytes(a.size_bytes)
                ));
                atts.push(a);
            }
            Err(e) => lines.push(tr!(
                "chat.pipeline.viewer.attach_failed",
                title = p.title,
                node_id = p.node_id,
                path = path.display(),
                error = e
            )),
        }
    }
    (atts, lines)
}

/// Собрать записанные save-нодами файлы в CAS-вложения. Возвращает
/// (вложения, строки отчёта). Файл считается результатом, если появился
/// или изменился относительно снимка `pre`.
pub fn collect_artifacts(planned: &[PlannedArtifact]) -> (Vec<MsgAttachment>, Vec<String>) {
    let mut atts = Vec::new();
    let mut lines = Vec::new();
    for p in planned {
        let now = std::fs::metadata(&p.path)
            .ok()
            .and_then(|m| m.modified().ok().map(|t| (m.len(), t)));
        let fresh = match (&p.pre, &now) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(a), Some(b)) => a != b,
        };
        if !fresh {
            lines.push(tr!(
                "chat.pipeline.artifact.not_written",
                title = p.title,
                node_id = p.node_id
            ));
            continue;
        }
        match ingest::ingest(&p.path) {
            Ok(a) => {
                lines.push(tr!(
                    "chat.pipeline.artifact.attached",
                    title = p.title,
                    path = p.path.display(),
                    size = models::human_bytes(a.size_bytes)
                ));
                atts.push(a);
            }
            Err(e) => lines.push(tr!(
                "chat.pipeline.artifact.attach_failed",
                title = p.title,
                path = p.path.display(),
                error = e
            )),
        }
    }
    (atts, lines)
}

fn fmt_duration(ms: u64) -> String {
    let d = Duration::from_millis(ms);
    let s = d.as_secs();
    if s >= 3600 {
        tr!("chat.pipeline.duration.hm", h = s / 3600, m = (s % 3600) / 60)
    } else if s >= 60 {
        tr!("chat.pipeline.duration.ms", m = s / 60, sec = s % 60)
    } else if s >= 10 {
        tr!("chat.pipeline.duration.s", sec = s)
    } else {
        tr!("chat.pipeline.duration.s_frac", sec = format!("{:.1}", d.as_secs_f32()))
    }
}

/// Итоговый envelope прогона. Возвращает (текст, error-флаг).
pub fn format_envelope(
    outcome: Option<&RunOutcome>,
    artifact_lines: &[String],
    llm_note: Option<&str>,
    aborted: bool,
    warnings: &[String],
) -> (String, bool) {
    let mut out = String::new();
    let mut error = false;

    match outcome {
        Some(o) => {
            let end_label = match o.end {
                RunEnd::Completed => tr!("chat.pipeline.status.completed"),
                RunEnd::Stopped => tr!("chat.pipeline.status.stopped"),
                RunEnd::Superseded => tr!("chat.pipeline.status.superseded"),
            };
            if o.end != RunEnd::Completed {
                error = true;
            }
            out.push_str(&tr!(
                "chat.pipeline.envelope.header",
                status = end_label,
                duration = fmt_duration(o.total_ms)
            ));
            for n in &o.nodes {
                match &n.error {
                    Some(e) => {
                        error = true;
                        let cause = n
                            .after_failed
                            .map(|t| format!(" {}", tr!("chat.pipeline.node.after_failed", title = t)))
                            .unwrap_or_default();
                        out.push_str(&format!(
                            "- {} · {} · {}: {e}{cause}\n",
                            n.title,
                            fmt_duration(n.elapsed_ms),
                            tr!("chat.pipeline.error_label")
                        ));
                    }
                    None => out.push_str(&format!(
                        "- {} · {}\n",
                        n.title,
                        fmt_duration(n.elapsed_ms)
                    )),
                }
            }
            for u in &o.unfinished {
                let state = match u.running_ms {
                    Some(ms) => tr!("chat.pipeline.node.interrupted", duration = fmt_duration(ms)),
                    None => tr!("chat.pipeline.node.not_started"),
                };
                out.push_str(&format!("- {} · {state}\n", u.title));
            }
        }
        None if aborted => {
            error = true;
            out.push_str(&tr!("chat.pipeline.envelope.aborted"));
        }
        None => {
            error = true;
            out.push_str(&tr!("chat.pipeline.envelope.no_outcome"));
        }
    }

    out.push_str(&tr!("chat.pipeline.envelope.artifacts_header"));
    if artifact_lines.is_empty() {
        out.push_str(&tr!("chat.pipeline.envelope.artifacts_empty"));
    } else {
        for l in artifact_lines {
            out.push_str(&format!("- {l}\n"));
        }
    }
    if !warnings.is_empty() {
        out.push_str(&tr!("chat.pipeline.envelope.warnings_header"));
        for w in warnings {
            out.push_str(&format!("- {w}\n"));
        }
    }

    if let Some(note) = llm_note {
        out.push_str(&format!("---\n{note}\n"));
    }
    (out, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_label_keeps_safe_chars() {
        assert_eq!(sanitize_label("run-2"), "run-2");
        assert_eq!(sanitize_label("../evil path"), "---evil-path");
        assert!(sanitize_label(&"x".repeat(100)).len() <= 48);
    }

    #[test]
    fn parse_run_call_matches_only_run_action() {
        use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
        let mk = |name: &str, args: &str| ChatToolCall {
            id: "1".into(),
            kind: "function".into(),
            function: ChatToolCallFunction {
                name: Some(name.into()),
                arguments: Some(args.into()),
            },
        };
        assert!(parse_run_call(&mk("pipelines", r#"{"action":"run"}"#)).is_some());
        // Модель обрамила значения переводами строк — это всё ещё run.
        assert!(parse_run_call(&mk("pipelines", "{\"action\":\"\\nrun\\n\"}")).is_some());
        assert!(parse_run_call(&mk("x.pipelines", r#"{"action":"run","free_vram":true}"#))
            .map(|r| r.free_vram)
            .unwrap_or(false));
        // free_vram строкой — тоже да.
        assert!(parse_run_call(&mk("pipelines", r#"{"action":"run","free_vram":"true"}"#))
            .map(|r| r.free_vram)
            .unwrap_or(false));
        assert!(parse_run_call(&mk("pipelines", r#"{"action":"list"}"#)).is_none());
        assert!(parse_run_call(&mk("bash", r#"{"action":"run"}"#)).is_none());
    }

    #[test]
    fn envelope_flags_errors() {
        let o = RunOutcome {
            end: RunEnd::Completed,
            total_ms: 65_000,
            nodes: vec![crate::pages::node_editor::run_controls::NodeRunReport {
                id: 1,
                title: "LTX Sampler Stage1",
                elapsed_ms: 60_000,
                error: Some("нет входа".into()),
                after_failed: None,
            }],
            unfinished: Vec::new(),
        };
        let (text, err) = format_envelope(Some(&o), &[], None, false, &[]);
        assert!(err);
        assert!(text.contains(&tr!("chat.pipeline.error_label")));
        let (text2, err2) = format_envelope(None, &[], Some("LLM перезагружена"), true, &[]);
        assert!(err2);
        assert!(text2.contains(&tr!("chat.pipeline.envelope.aborted")));
        assert!(text2.contains("LLM перезагружена"));
    }
}
