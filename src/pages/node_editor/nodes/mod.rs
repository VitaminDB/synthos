//! Тела (body builders) для функциональных нод node-editor'а.
//!
//! Каждый под-модуль реализует `pub fn body(node: &NodeInstance) -> Box<dyn Widget>`,
//! который регистрируется в `registry::NodeKindMeta::body`. Header и port-rows
//! рисуются стандартным `node_view`, body замещает дефолтные field-rows.
//!
//! Для long-lived state (плеер, рекордер, загруженный файл) используется
//! `NodeInstance.runtime: Arc<Mutex<NodeRuntime>>` — см. `types::NodeRuntime`.
//!
//! Логи воркеров нод идут в target [`WORKER_LOG`] (`node-editor.worker`) —
//! парный к `node-editor.run` из `run_controls`. Первый отвечает на вопрос
//! «что нода делала и чем кончила», второй — «в каком порядке sequencer их
//! запускал». Хелперы [`log_worker_start`] / [`log_worker_done`] держат
//! формат строк одинаковым между нодами.

use std::time::Instant;

use tracing::info;

/// Target логов воркеров нод. Уровень INFO: старт с параметрами, финиш с
/// длительностью и исходом. Прогресс внутри долгих воркеров — DEBUG.
pub const WORKER_LOG: &str = "node-editor.worker";

/// Старт воркера ноды. `params` — короткая строка вида
/// `"1344x768, 5.0s, 20 шагов, CFG 5.0"`; пустая допустима.
pub fn log_worker_start(node: &'static str, params: &str) -> Instant {
    if params.is_empty() {
        info!(target: WORKER_LOG, node, "воркер: старт");
    } else {
        info!(target: WORKER_LOG, node, params, "воркер: старт");
    }
    Instant::now()
}

/// Финиш воркера ноды. `res` — то, что воркер вернул; `Err` пишется
/// с текстом ошибки, чтобы отмена/OOM/сбой были видны в логе, а не
/// только в поле «Статус» на ноде.
pub fn log_worker_done<T, E: std::fmt::Display>(
    node: &'static str,
    started: Instant,
    res: &Result<T, E>,
) {
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(_) => info!(target: WORKER_LOG, node, elapsed_ms, "воркер: готово"),
        Err(e) => info!(
            target: WORKER_LOG,
            node,
            elapsed_ms,
            error = %e,
            "воркер: ошибка"
        ),
    }
}

pub mod acestep;
pub mod asr_gigaam;
pub mod audio_equalizer;
pub mod audio_file;
pub mod audio_filter;
pub mod audio_gain;
pub mod audio_mixer;
pub mod audio_player;
pub mod audio_recorder;
pub mod audio_reverb;
pub mod audio_save;
pub mod decode;
pub mod ffmpeg_player;
pub mod flux;
pub mod image;
pub mod llm;
pub mod ltx;
pub mod markdown_view;
pub mod minimax_h3;
pub mod omnivoice;
pub mod scalar;
pub mod sortformer_diarizer;
pub mod syn_checkpoint;
pub mod text_view;
pub mod vibevoice;
pub mod voxcpm2;

/// Хэндл «Syn Checkpoint», подключённый к входу `model` ноды `node_id`.
/// Общий для всех слот-семейств (LLM/TTS/ASR/диаризация): если порт не
/// подключён — None, нода работает от собственных полей (legacy-графы).
pub fn current_input_syn_model(
    ctx: &super::state::NodeEditorCtx,
    node_id: super::types::NodeId,
) -> Option<std::sync::Arc<super::types::SynModelHandle>> {
    let conns = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == "model")?;
    let values = ctx.values.get_untracked();
    values
        .get(&(src.from_node, src.from_port))
        .and_then(|pv| pv.as_syn_model())
}
