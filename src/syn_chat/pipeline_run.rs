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

use crate::agent::schema::ChatToolCall;
use crate::agent::state::MsgAttachment;
use crate::models;
use crate::pages::node_editor::registry;
use crate::pages::node_editor::run_controls::{self, RunEnd, RunOutcome, StartRunError};
use crate::pages::node_editor::state::NodeEditorCtx;
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::types::NodeRuntime;
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
pub fn parse_run_call(call: &ChatToolCall) -> Option<RunRequest> {
    let name = call.function.name.as_deref().unwrap_or("");
    if name != "pipelines" && !name.ends_with(".pipelines") {
        return None;
    }
    let v: serde_json::Value =
        serde_json::from_str(call.function.arguments.as_deref().unwrap_or("")).ok()?;
    if v.get("action").and_then(|x| x.as_str()) != Some("run") {
        return None;
    }
    let free_vram = v.get("free_vram").and_then(|x| x.as_bool()).unwrap_or(false);
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
}

/// Ctx служебной вкладки текущего чата (main thread).
fn agent_tab_ctx() -> Result<NodeEditorCtx, String> {
    let chat = use_context::<SynChatCtx>();
    let chat_id = chat
        .active_chat_id
        .get_untracked()
        .ok_or("нет активного чата")?;
    let ws = use_context::<EditorWorkspace>();
    let tab_id = ws
        .agent_tab_for_chat(&chat_id)
        .ok_or("служебной вкладки нет — сначала pipelines open или apply")?;
    ws.tabs
        .get_untracked()
        .iter()
        .find(|t| t.id == tab_id)
        .map(|t| t.ctx)
        .ok_or_else(|| "служебная вкладка пропала".to_string())
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
        .ok_or("нет активного чата")?;
    let ctx = agent_tab_ctx()?;
    let nodes = ctx.nodes.get_untracked();
    let runnable = nodes
        .iter()
        .filter(|n| n.enabled.get_untracked() && registry::meta(n.kind).on_run.is_some())
        .count();
    if runnable == 0 {
        return Err(
            "в графе нет запускаемых нод — собери пайплайн (pipelines open/apply)".into(),
        );
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
                format!("не создать каталог результатов {}: {e}", out_dir.display())
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
    Ok(Prepared { planned, runnable })
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
                    "в графе нет запускаемых нод".to_string()
                }
                StartRunError::CycleInGraph => {
                    "в графе цикл — прогон невозможен".to_string()
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
            lines.push(format!(
                "{} (нода {}): файл не записан",
                p.title, p.node_id
            ));
            continue;
        }
        match ingest::ingest(&p.path) {
            Ok(a) => {
                lines.push(format!(
                    "{} → {} ({}) — приложено к сообщению",
                    p.title,
                    p.path.display(),
                    models::human_bytes(a.size_bytes)
                ));
                atts.push(a);
            }
            Err(e) => lines.push(format!(
                "{} → {}: файл есть, но не приложился: {e}",
                p.title,
                p.path.display()
            )),
        }
    }
    (atts, lines)
}

fn fmt_duration(ms: u64) -> String {
    let d = Duration::from_millis(ms);
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}ч {}м", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}м {}с", s / 60, s % 60)
    } else if s >= 10 {
        format!("{s}с")
    } else {
        format!("{:.1}с", d.as_secs_f32())
    }
}

/// Итоговый envelope прогона. Возвращает (текст, error-флаг).
pub fn format_envelope(
    outcome: Option<&RunOutcome>,
    artifact_lines: &[String],
    llm_note: Option<&str>,
    aborted: bool,
) -> (String, bool) {
    let mut out = String::new();
    let mut error = false;

    match outcome {
        Some(o) => {
            let end_label = match o.end {
                RunEnd::Completed => "завершён",
                RunEnd::Stopped => "остановлен",
                RunEnd::Superseded => "замещён другим запуском",
            };
            if o.end != RunEnd::Completed {
                error = true;
            }
            out.push_str(&format!(
                "--- Прогон {} за {} ---\n",
                end_label,
                fmt_duration(o.total_ms)
            ));
            for n in &o.nodes {
                match &n.error {
                    Some(e) => {
                        error = true;
                        out.push_str(&format!(
                            "- {} · {} · ОШИБКА: {e}\n",
                            n.title,
                            fmt_duration(n.elapsed_ms)
                        ));
                    }
                    None => out.push_str(&format!(
                        "- {} · {}\n",
                        n.title,
                        fmt_duration(n.elapsed_ms)
                    )),
                }
            }
        }
        None if aborted => {
            error = true;
            out.push_str("--- Прогон отменён пользователем (abort хода) ---\n");
        }
        None => {
            error = true;
            out.push_str("--- Итог прогона не получен ---\n");
        }
    }

    out.push_str("--- Артефакты ---\n");
    if artifact_lines.is_empty() {
        out.push_str("(save-нод в графе нет — результаты остались в памяти нод)\n");
    } else {
        for l in artifact_lines {
            out.push_str(&format!("- {l}\n"));
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
        assert!(parse_run_call(&mk("x.pipelines", r#"{"action":"run","free_vram":true}"#))
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
            }],
        };
        let (text, err) = format_envelope(Some(&o), &[], None, false);
        assert!(err);
        assert!(text.contains("ОШИБКА"));
        let (text2, err2) = format_envelope(None, &[], Some("LLM перезагружена"), true);
        assert!(err2);
        assert!(text2.contains("отменён"));
        assert!(text2.contains("LLM перезагружена"));
    }
}
