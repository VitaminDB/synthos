//! Tool `system` — состояние системы и памяти для агента Syn-чата.
//!
//! Контракт:
//! - `{ "action": "status" }` — VRAM (всего / свободно по драйверу /
//!   доступно с учётом слабины пула активаций), RAM, список моделей в
//!   памяти: нодовые (`crate::models`, с id для выгрузки) и чат-LLM
//!   (`SynModelRegistry`).
//! - `{ "action": "unload", "id"?: N, "all"?: true }` — выгрузить нодовые
//!   модели из VRAM. Чат-LLM отсюда НЕ выгружается: worker-поток агента
//!   держит `Arc<LoadedSynModel>` весь ход, веса всё равно не освободятся —
//!   для этого есть `free_vram` у `pipelines action=run`.
//!
//! Формат результата — секционный plain-text (конвенция `web`/`kb_search`).
//! Все данные, кроме чат-LLM, читаются без main-thread: `models::list` —
//! Mutex + атомики, VRAM — прямые CUDA-вызовы, RAM — sysinfo. Снимок
//! чат-LLM берётся через `run_on_main_thread` + oneshot (сигналы).

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use crate::models;
use crate::syn_chat::model_registry::{act_pool_slack_mb, vram_available_mb, vram_free_mb};
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

    // VRAM. mem_info даёт (free, total); available добавляет слабину
    // пула активаций — по ней и надо принимать решения о памяти.
    out.push_str("--- VRAM (CUDA:0) ---\n");
    match synaptix_core::device::cuda::mem_info(0) {
        Ok((_free, total)) => {
            out.push_str(&format!("total: {} MB\n", total / (1024 * 1024)));
            out.push_str(&format!("free per driver: {} MB\n", vram_free_mb()));
            out.push_str(&format!(
                "available (free + activation pool slack {} MB): {} MB\n",
                act_pool_slack_mb(),
                vram_available_mb()
            ));
        }
        Err(_) => out.push_str("CUDA unavailable\n"),
    }

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
    out.push_str("--- Models in memory (node-graph) ---\n");
    let infos = models::list();
    if infos.is_empty() {
        out.push_str("(none)\n");
    } else {
        for m in &infos {
            out.push_str(&format!(
                "id={} · {} · {} · {} · {} · {}\n",
                m.id,
                m.family,
                m.component,
                m.label,
                m.device,
                models::human_bytes(m.bytes)
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
        "---\nUnload node-graph models: system {\"action\":\"unload\",\"id\":N} \
         or {\"all\":true}. The chat LLM is only unloaded via free_vram on \
         pipelines run.\n",
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
    let id = v
        .get("id")
        .and_then(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse::<u64>().ok()));

    let mut out = String::new();
    let before_mb = vram_available_mb();

    if all {
        let n = models::unload_all();
        models::trim_all();
        out.push_str(&format!("models unloaded: {n}\n"));
    } else if let Some(id) = id {
        let known = models::list().iter().any(|m| m.id == id);
        if !known {
            return Err(ToolError::BadArgs(format!(
                "no model with id={id} in the registry (see system status)"
            )));
        }
        let released = models::unload(id);
        models::trim_all();
        if released {
            out.push_str(&format!("model id={id} unloaded\n"));
        } else {
            out.push_str(&format!(
                "model id={id} is still held by a running worker — \
                 wait for the run to finish\n"
            ));
        }
    } else {
        return Err(ToolError::MissingField("id (or all=true)"));
    }

    out.push_str(&format!(
        "VRAM available: was {} MB, now {} MB\n",
        before_mb,
        vram_available_mb()
    ));
    Ok(out)
}
