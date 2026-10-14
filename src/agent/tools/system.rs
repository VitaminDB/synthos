//! Tool `system` — состояние системы и памяти для агента Syn-чата.
//!
//! Контракт:
//! - `{ "action": "status" }` — видеопамять (NVML), RAM, модели в памяти
//!   службы нейросетей (`crate::models`, с id для выгрузки и программами,
//!   которые их держат) и чат-LLM (`SynModelRegistry`).
//! - `{ "action": "unload", "id"?: "id", "all"?: true }` — выгрузить модели
//!   в службе (их память — у неё).
//!
//! Формат результата — секционный plain-text (конвенция `web`/`kb_search`).
//! Все данные, кроме чат-LLM, читаются без main-thread: `models::list` —
//! снимок службы, видеопамять — NVML, RAM — sysinfo. Снимок
//! чат-LLM берётся через `run_on_main_thread` + oneshot (сигналы).

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use crate::models;
use crate::syn_chat::SynModelRegistry;

use super::executor::ToolError;

/// Снимок состояния чат-LLM (сигналы — только main thread).
struct ChatLlmSnapshot {
    loaded_name: Option<String>,
    supports_media: bool,
    loading: bool,
    last_path: Option<String>,
}

fn chat_llm_snapshot() -> tokio::sync::oneshot::Receiver<ChatLlmSnapshot> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let reg = use_context::<SynModelRegistry>();
        let current = reg.current.get_untracked();
        let snap = ChatLlmSnapshot {
            loaded_name: current.as_ref().map(|m| {
                m.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| m.path.display().to_string())
            }),
            supports_media: current.as_ref().map(|m| m.supports_media).unwrap_or(false),
            loading: reg.loading.get_untracked(),
            last_path: reg
                .last_path
                .get_untracked()
                .map(|p| p.display().to_string()),
        };
        let _ = tx.send(snap);
    });
    rx
}

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: serde_json::Value =
        serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let action = v
        .get("action")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("action"))?;

    match action {
        "status" => status().await,
        "unload" => unload(&v),
        other => Err(ToolError::BadArgs(format!(
            "unknown action \"{other}\" (expected status | unload)"
        ))),
    }
}

async fn status() -> Result<String, ToolError> {
    let chat_rx = chat_llm_snapshot();

    let mut out = String::new();

    out.push_str("--- VRAM (GPU 0) ---\n");
    out.push_str(&vram_line());
    out.push('\n');

    // RAM — свежий замер sysinfo, только память (дёшево).
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    out.push_str("--- RAM ---\n");
    out.push_str(&format!(
        "used: {} of {}\n",
        models::human_bytes(sys.used_memory()),
        models::human_bytes(sys.total_memory())
    ));

    // Нодовые модели: id нужен для action=unload.
    out.push_str("--- Models in memory (synaptixd service) ---\n");
    let infos = models::list();
    if infos.is_empty() {
        out.push_str("(none)\n");
    } else {
        for m in &infos {
            out.push_str(&format!(
                "id={} · {} · {} · {} · {} · {}{} · used by: {}\n",
                m.id,
                m.family,
                m.component,
                m.label,
                m.device,
                models::human_bytes(m.bytes),
                if m.busy { " · busy" } else { "" },
                if m.apps.is_empty() { "-".to_string() } else { m.apps.join(", ") }
            ));
        }
    }

    out.push_str("--- Chat LLM ---\n");
    match chat_rx.await {
        Ok(snap) => {
            if let Some(name) = snap.loaded_name {
                out.push_str(&format!(
                    "loaded: {name}{}\n",
                    if snap.supports_media { " (vision)" } else { "" }
                ));
            } else if snap.loading {
                out.push_str("loading…\n");
            } else {
                out.push_str(&format!(
                    "not loaded{}\n",
                    snap.last_path
                        .map(|p| format!(" (last: {p})"))
                        .unwrap_or_default()
                ));
            }
        }
        Err(_) => out.push_str("(snapshot unavailable)\n"),
    }

    out.push_str(
        "---\nUnload models of the service: system {\"action\":\"unload\",\"id\":\"model-id\"} \
         or {\"all\":true}. The service also unloads idle models by itself when memory is needed.\n",
    );
    Ok(out)
}

fn unload(v: &serde_json::Value) -> Result<String, ToolError> {
    // Модели шлют флаги строками («true») и id — тоже строкой; принимаем оба.
    let all = v
        .get("all")
        .map(|x| match x {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => {
                matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "да")
            }
            serde_json::Value::Number(n) => n.as_i64().is_some_and(|i| i != 0),
            _ => false,
        })
        .unwrap_or(false);
    let id = v.get("id").and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_u64().map(|n| n.to_string())));

    let mut out = String::new();
    let before = vram_line();

    if all {
        let n = models::unload_all();
        out.push_str(&format!("models unloaded: {n}\n"));
    } else if let Some(id) = id {
        if !models::list().iter().any(|m| m.id == id) {
            return Err(ToolError::BadArgs(format!("no loaded model with id={id} (see system status)")));
        }
        match models::unload(&id) {
            Ok(()) => out.push_str(&format!("model id={id} unloaded\n")),
            Err(e) => out.push_str(&format!("model id={id} not unloaded: {e}\n")),
        }
    } else {
        return Err(ToolError::MissingField("id (or all=true)"));
    }

    out.push_str(&format!("VRAM before: {before}; now: {}\n", vram_line()));
    Ok(out)
}

/// «used 9.1 GB of 24.0 GB» — по NVML (без своего CUDA-контекста).
fn vram_line() -> String {
    match crate::metrics::gpu::GpuMonitor::try_new().and_then(|g| g.sample()) {
        Some(g) => format!("used {} of {}", models::human_bytes(g.mem_used), models::human_bytes(g.mem_total)),
        None => "GPU metrics unavailable".into(),
    }
}
