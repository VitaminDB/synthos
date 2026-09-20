//! `Yue2VaeDecode` — латенты YuE2 → звук.
//!
//! Нужна, когда генерация уже сделана, а звук хочется получить другим
//! декодером: `yue2-vae.syn` (по умолчанию, для прослушивания) или
//! `yue2-vae-legacy.syn` (декодер протокола бенчмарка). Генерация при этом не
//! повторяется — латенты приходят портом.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use synaptix_core::tensor::Tensor;
use synaptix_music_yue2::vae::Yue2Vae;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{NodeInstance, NodeRuntime, PortValue, Yue2ModelHandle};
use super::shared::{load_vae, resolve_paths};
use super::{
    current_input_latent, current_input_model, field_row, make_int_slider_row, status_row,
};
use crate::pages::node_editor::controls::file_picker::node_file_picker_placeholder;

const SYN_FILTER: &[(&str, &[&str])] = &[("Syn bundle", &["syn"])];

pub struct VaeDecodeExec;

impl NodeExecutor for VaeDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _latent = ctx.read_input("latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Yue2VaeDecode { output_buf_audio, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match output_buf_audio.lock() {
                        Ok(b) => b.clone().map(PortValue::Audio).unwrap_or(PortValue::Empty),
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("audio", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2VaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

type Snapshot = (
    RwSignal<Option<PathBuf>>,
    RwSignal<u32>,
    RwSignal<bool>,
    RwSignal<Option<String>>,
    RwSignal<Option<String>>,
    RwSignal<f32>,
    Arc<std::sync::atomic::AtomicBool>,
);

fn snapshot(node: &NodeInstance) -> Option<Snapshot> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2VaeDecode {
                vae_path,
                vae_core_frames,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                ..
            } => Some((
                *vae_path,
                *vae_core_frames,
                *running,
                *error,
                *loaded_name,
                *progress_pct,
                cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some((vae_path, core_frames, running, error, loaded_name, progress, cancel)) =
        snapshot(node)
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some(tr!("node.yue2_generate.error.no_checkpoint")));
        return;
    };
    let Some(latents) = current_input_latent(ctx, node.id, "latent") else {
        error.set(Some(tr!("node.yue2_vae_decode.error.no_latent")));
        return;
    };
    let override_path = vae_path.get_untracked();
    let frames = core_frames.get_untracked().max(16) as usize;

    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    running.set(true);
    error.set(None);
    progress.set(0.0);

    let (output_buf_audio, output_version) = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2VaeDecode { output_buf_audio, output_version, .. } => {
                (output_buf_audio.clone(), *output_version)
            }
            _ => return,
        },
        Err(_) => return,
    };

    let _ = thread::Builder::new()
        .name("synthos-yue2-decode".into())
        .spawn(move || {
            worker(
                handle,
                override_path,
                latents,
                frames,
                cancel,
                running,
                error,
                loaded_name,
                progress,
                output_buf_audio,
                output_version,
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn worker(
    handle: Arc<Yue2ModelHandle>,
    override_path: Option<PathBuf>,
    latents: Tensor,
    core_frames: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    progress: RwSignal<f32>,
    output_buf_audio: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    output_version: RwSignal<u32>,
) {
    let finish_err = |msg: String| {
        error.set(Some(msg));
        running.set(false);
    };
    let path = match override_path {
        // Голое имя (так их печатает схема ноды) — от каталога моделей.
        Some(p) if p.is_relative() => match &handle.models_dir {
            Some(d) => d.join(p),
            None => p,
        },
        Some(p) => p,
        None => match resolve_paths(&handle) {
            Ok((_, vae)) => vae,
            Err(e) => return finish_err(e),
        },
    };
    if !path.exists() {
        return finish_err(tr!(
            "node.yue2.error.bundle_not_found",
            label = "vae",
            path = path.display()
        ));
    }
    let vae = match load_vae(&path, handle.device_idx, handle.vae_dtype_idx) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    loaded_name.set(Some(tr!("node.yue2_vae_decode.status.busy")));

    let audio = match decode(&vae, &latents, core_frames, &cancel, progress) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    if !handle.resident {
        drop(vae);
        crate::models::trim_all();
    }
    crate::models::changed();

    let seconds = audio.len() as f32 / (2.0 * 48000.0);
    loaded_name.set(Some(tr!(
        "node.yue2_vae_decode.status.done",
        seconds = format!("{seconds:.0}"),
        name = name
    )));
    if let Ok(mut g) = output_buf_audio.lock() {
        *g = Some(Arc::new(AudioBuffer::new(
            Arc::from(audio.into_boxed_slice()),
            48_000,
            2,
        )));
    }
    syngui::prelude::run_on_main_thread(move || {
        output_version.update(|v| *v = v.wrapping_add(1));
    });
    progress.set(100.0);
    error.set(None);
    running.set(false);
}

/// Латенты `[кадры, 64]` (так их отдаёт Generate) или `[1, 64, кадры]` — в звук.
fn decode(
    vae: &Yue2Vae,
    latents: &Tensor,
    core_frames: usize,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
    progress: RwSignal<f32>,
) -> std::result::Result<Vec<f32>, String> {
    let dims = latents.dims().to_vec();
    let z = match dims.as_slice() {
        [_, 64] => latents
            .transpose(0, 1)
            .and_then(|t| t.contiguous())
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| e.to_string())?,
        [1, 64, _] => latents.clone(),
        other => {
            return Err(tr!(
                "node.yue2_vae_decode.error.bad_latent",
                dims = format!("{other:?}")
            ))
        }
    };
    let z = z.to_device(vae.device()).map_err(|e| e.to_string())?;
    let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
    let audio = vae
        .decode_tiled(&z, core_frames, 16, &cancelled, &|done, total| {
            let part = if total == 0 { 0.0 } else { done as f32 / total as f32 };
            progress.set(100.0 * part.min(1.0));
        })
        .map_err(|e| e.to_string())?;
    synaptix_music_yue2::pipeline::interleave(&audio).map_err(|e| e.to_string())
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some((vae_path, core_frames, running, error, loaded_name, _, cancel)) = snapshot(node)
    else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "Yue2VaeDecode"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(
            &tr!("node.yue2_vae_decode.field.vae"),
            node_file_picker_placeholder(
                tr!("node.yue2_vae_decode.tooltip.vae"),
                vae_path,
                SYN_FILTER,
                || tr!("node.yue2_vae_decode.default_vae"),
                |_| {},
            ),
        ),
        field_row(
            &tr!("node.yue2_generate.field.vae_core_frames"),
            make_int_slider_row(core_frames, 64, 4096, 64),
        ),
        field_row(
            &tr!("nodes.common.status"),
            status_row(
                running,
                error,
                loaded_name,
                tr!("node.yue2_vae_decode.status.busy"),
                "acestep-node-running",
            ),
        ),
        field_row(&tr!("app.cancel"), super::super::ltx::cancel_button(running, cancel)),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
