//! `Yue2Transcribe` — запись → партитура ABC (SheetSage2).
//!
//! Первый шаг кавера: мелодия записи (вокал и инструментальная тема) в родном
//! диалекте YuE2 без аккордовых символов идёт во вход `abc` Generate-ноды с
//! `cot = melody` — аккомпанемент модель придумывает заново под новый стиль.
//! С аккордами (`full`) — полная партитура: её можно подать с `cot = full`
//! или просто посмотреть.
//!
//! SheetSage2 лежит компонентом в том же `yue2-3b.syn`, поэтому нода берёт
//! модель из того же чекпойнта, что и Generate. После прогона веса
//! отпускаются: на этапе генерации транскриптор в памяти не нужен.

use std::sync::Arc;
use std::thread;

use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use synaptix_music_sheetsage2::pipeline::{prepare_audio, Progress};
use synaptix_music_sheetsage2::{TranscribeOptions, VoiceSelect};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{NodeInstance, NodeRuntime, PortValue, Yue2ModelHandle};
use super::shared::{load_sheet, resolve_model_path};
use super::{current_input_model, field_row, make_dropdown, status_row};

/// Что писать в партитуру: только мелодии (план кавера) или с аккордами.
pub const MODE_OPTIONS: &[&str] = &["melody", "full"];
/// Какие мелодии оставить: обе, только вокал, только инструментальную.
pub const VOICE_OPTIONS: &[&str] = &["both", "vocal", "ins"];

pub fn voices_from_idx(i: usize) -> VoiceSelect {
    match VOICE_OPTIONS.get(i).copied() {
        Some("vocal") => VoiceSelect::Vocal,
        Some("ins") => VoiceSelect::Instrumental,
        _ => VoiceSelect::Both,
    }
}

pub struct TranscribeExec;

impl NodeExecutor for TranscribeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _audio = ctx.read_input("audio");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Yue2Transcribe { output_buf_score, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match output_buf_score.lock() {
                        Ok(b) => b.clone().map(PortValue::Text).unwrap_or(PortValue::Empty),
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("score", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2Transcribe { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

struct Snapshot {
    mode_idx: RwSignal<usize>,
    voices_idx: RwSignal<usize>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    progress_pct: RwSignal<f32>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    output_buf_score: Arc<Mutex<Option<String>>>,
    output_version: RwSignal<u32>,
}

fn snapshot(node: &NodeInstance) -> Option<Snapshot> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2Transcribe {
                mode_idx,
                voices_idx,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                output_buf_score,
                output_version,
            } => Some(Snapshot {
                mode_idx: *mode_idx,
                voices_idx: *voices_idx,
                running: *running,
                error: *error,
                loaded_name: *loaded_name,
                progress_pct: *progress_pct,
                cancel: cancel.clone(),
                output_buf_score: output_buf_score.clone(),
                output_version: *output_version,
            }),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(s) = snapshot(node) else { return };
    if s.running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        s.error.set(Some(tr!("node.yue2_generate.error.no_checkpoint")));
        return;
    };
    let Some(audio) = super::super::acestep::current_input_audio(ctx, node.id, "audio") else {
        s.error.set(Some(tr!("node.yue2_transcribe.error.no_audio")));
        return;
    };
    let options = TranscribeOptions {
        melody_only: MODE_OPTIONS.get(s.mode_idx.get_untracked()).copied() != Some("full"),
        voices: voices_from_idx(s.voices_idx.get_untracked()),
        ..Default::default()
    };

    s.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    s.running.set(true);
    s.error.set(None);
    s.progress_pct.set(0.0);
    s.loaded_name.set(Some(tr!("node.yue2_transcribe.status.busy")));

    let _ = thread::Builder::new()
        .name("synthos-yue2-transcribe".into())
        .spawn(move || worker(handle, audio, options, s));
}

fn worker(
    handle: Arc<Yue2ModelHandle>,
    audio: Arc<syngui::audio::AudioBuffer>,
    options: TranscribeOptions,
    s: Snapshot,
) {
    let (running, error, loaded_name, progress) = (s.running, s.error, s.loaded_name, s.progress_pct);
    let finish_err = |msg: String| {
        error.set(Some(msg));
        running.set(false);
    };
    let path = match resolve_model_path(&handle) {
        Ok(p) => p,
        Err(e) => return finish_err(e),
    };
    let sheet = match load_sheet(&path, handle.device_idx, handle.compute_idx) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    // Чередующийся PCM → каналы → моно 24 кГц (как `ffmpeg -ac 1 -ar 24000`).
    let ch = audio.channels.max(1) as usize;
    let channels: Vec<Vec<f32>> =
        (0..ch).map(|c| audio.pcm.iter().skip(c).step_by(ch).copied().collect()).collect();
    let samples = match prepare_audio(&channels, audio.sample_rate, sheet.sample_rate()) {
        Ok(v) => v,
        Err(e) => return finish_err(e.to_string()),
    };
    let seconds = samples.len() as f64 / sheet.sample_rate() as f64;
    // Событийных токенов — около дюжины на секунду; этого хватает, чтобы полоса
    // шла ровно.
    let window = sheet.config.input_audio_length.min(seconds).max(1.0);
    let mut on_progress = |p: Progress| match p {
        Progress::Encoding { window: w, windows } => {
            progress.set(100.0 * (w - 1) as f32 / windows as f32);
            loaded_name.set(Some(tr!(
                "node.yue2_transcribe.status.encoding",
                window = w.to_string(),
                windows = windows.to_string()
            )));
        }
        Progress::Decoding { window: w, windows, tokens } => {
            let part = (tokens as f64 / (12.0 * window)).min(0.95) as f32;
            progress.set(100.0 * ((w - 1) as f32 + part) / windows as f32);
            loaded_name.set(Some(tr!(
                "node.yue2_transcribe.status.decoding",
                window = w.to_string(),
                windows = windows.to_string(),
                tokens = tokens.to_string()
            )));
        }
        Progress::Notation => progress.set(99.0),
    };
    let cancel = s.cancel.clone();
    let cancelled = move || cancel.load(std::sync::atomic::Ordering::Relaxed);
    let result = sheet.transcribe(&samples, &options, &mut on_progress, &cancelled);
    // Транскриптор на этапе генерации не нужен: отпускаем и отдаём пул.
    drop(sheet);
    crate::models::trim_all();
    crate::models::changed();
    let result = match result {
        Ok(r) => r,
        Err(e) => return finish_err(e.to_string()),
    };
    for w in &result.warnings {
        tracing::warn!("[yue2] Transcribe: {w}");
    }
    let Some(abc) = result.abc.clone() else {
        let reason = result.abc_error.clone().unwrap_or_default();
        return finish_err(tr!("node.yue2_transcribe.error.no_score", reason = reason));
    };
    tracing::info!(
        "[yue2] Transcribe ✓ {:.0} с записи за {:.1} с: окон {}, событий {}, нот {} (вокал {}, инструмент {}), тактов {}",
        result.duration_seconds,
        result.seconds,
        result.windows.len(),
        result.events.len(),
        result.export.melody_notes,
        result.export.vocal_notes,
        result.export.instrumental_notes,
        result.export.measures
    );
    loaded_name.set(Some(tr!(
        "node.yue2_transcribe.status.done",
        measures = result.export.measures.to_string(),
        key = result.header_field("K").unwrap_or_default(),
        tempo = result.header_field("Q").map(|q| q.trim_start_matches("1/4=").to_string()).unwrap_or_default(),
        seconds = format!("{:.0}", result.duration_seconds)
    )));
    if let Ok(mut g) = s.output_buf_score.lock() {
        *g = Some(abc);
    }
    let version = s.output_version;
    syngui::prelude::run_on_main_thread(move || {
        version.update(|v| *v = v.wrapping_add(1));
    });
    progress.set(100.0);
    error.set(None);
    running.set(false);
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some(s) = snapshot(node) else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "Yue2Transcribe"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(&tr!("node.yue2_transcribe.field.mode"), make_dropdown(MODE_OPTIONS, s.mode_idx)),
        field_row(&tr!("node.yue2_transcribe.field.voices"), make_dropdown(VOICE_OPTIONS, s.voices_idx)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(
                s.running,
                s.error,
                s.loaded_name,
                tr!("node.yue2_transcribe.status.busy"),
                "acestep-node-running",
            ),
        ),
        field_row(&tr!("app.cancel"), super::super::ltx::cancel_button(s.running, s.cancel.clone())),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
