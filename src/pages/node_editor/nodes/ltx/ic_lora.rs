//! `LtxIcLora` — IC-LoRA video→video (reference control): reference-видео даёт
//! структуру/движение, текст — содержание. `(model, v_enc, a_enc, ref_video) →
//! (video_latent, audio_tokens)`. Ref грузится на разрешении target/downscale,
//! энкодится и добавляется (append) как control-сигнал; IC-LoRA адаптер
//! мерджится в DiT через `lora_path` Checkpoint-ноды. Одностадийно.

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;
use synaptix_video_ltx23::pipeline::{
    denoise_av_append, frames_for_duration, latent_grid, out_frame_count, ref_video_positions,
    DenoiseHooks, DISTILLED_SIGMAS,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::{field_row, make_int_slider_row, make_seed_slider, make_slider_row, status_row};
use super::sampler_stage1::publish_latents;
use super::shared;
use super::{
    current_input_audio_encoding, current_input_model, current_input_video_encoding,
    current_input_video_input, device_from_idx, fps_from_idx, make_fps_dropdown, progress_row,
};

pub struct IcLoraExec;

impl NodeExecutor for IcLoraExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_encoding");
        let _ = ctx.read_input("audio_encoding");
        let _ = ctx.read_input("ref_video");
        let track = ctx.track;
        let (v_pv, a_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxIcLora { v_out, a_out, output_version, .. } => {
                    publish_latents(track, v_out, a_out, output_version)
                }
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("video_latent", v_pv);
        ctx.write_output("audio_tokens", a_pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxIcLora { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxIcLora {
                width, height, duration_seconds, fps_idx, downscale, ref_strength,
                control_idx, canny_low, canny_high, depth_model_path, seed,
                running, error, progress_pct, cancel, ..
            } => Some((
                *width, *height, *duration_seconds, *fps_idx, *downscale, *ref_strength,
                *control_idx, *canny_low, *canny_high, *depth_model_path, *seed,
                *running, *error, *progress_pct, cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, downscale, ref_strength, control_idx, canny_low, canny_high, depth_model_path, seed, running, error, progress_pct, cancel)) =
        snapshot
    else {
        return Box::new(Text::new("LtxIcLora: некорректный runtime").class("node-card-field-error"));
    };
    let loaded_name = use_signal(None::<String>);
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row("Ширина", make_int_slider_row(width, 256, 1920, 32)),
        field_row("Высота", make_int_slider_row(height, 256, 1088, 32)),
        field_row("Длительность, с", make_slider_row(duration_seconds, 1.0, 20.0, 0.5, 1)),
        field_row("FPS", make_fps_dropdown(fps_idx)),
        field_row("Downscale ref", make_int_slider_row(downscale, 1, 8, 1)),
        field_row("Сила ref", make_slider_row(ref_strength, 0.0, 1.0, 0.05, 2)),
        field_row("Control", super::super::acestep::make_dropdown(super::CONTROL_OPTIONS, control_idx)),
        field_row("Canny low", make_slider_row(canny_low, 0.0, 1.0, 0.01, 2)),
        field_row("Canny high", make_slider_row(canny_high, 0.0, 1.0, 0.01, 2)),
        field_row(
            "Depth-модель",
            super::dir_picker_row("Каталог Depth Anything V2 (для control=depth)", depth_model_path),
        ),
        field_row("Seed", make_seed_slider(seed)),
        field_row("Прогресс", progress_row(running, progress_pct)),
        field_row("Отмена", super::cancel_button(running, cancel)),
        field_row(
            "Статус",
            status_row(running, error, loaded_name, "IC-LoRA denoise…", "ltx-node-running"),
        ),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxIcLora {
                width, height, duration_seconds, fps_idx, downscale, ref_strength,
                control_idx, canny_low, canny_high, depth_model_path, seed,
                running, error, progress_pct, cancel, v_out, a_out, output_version,
            } => Some((
                *width, *height, *duration_seconds, *fps_idx, *downscale, *ref_strength,
                *control_idx, *canny_low, *canny_high, *depth_model_path, *seed,
                *running, *error, *progress_pct, cancel.clone(), v_out.clone(), a_out.clone(), *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration_seconds, fps_idx, downscale, ref_strength, control_idx, canny_low, canny_high, depth_model_path, seed, running, error, progress_pct, cancel, v_out, a_out, output_version)) =
        snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some("Подключите LTX Checkpoint на вход model".into()));
        return;
    };
    if handle.lora_path.is_none() {
        error.set(Some("IC-LoRA требует LoRA-адаптер в Checkpoint-ноде".into()));
        return;
    }
    let Some(v_enc) = current_input_video_encoding(ctx, node.id, "video_encoding") else {
        error.set(Some("Подключите video_encoding от Text Encoder".into()));
        return;
    };
    let Some(a_enc) = current_input_audio_encoding(ctx, node.id, "audio_encoding") else {
        error.set(Some("Подключите audio_encoding от Text Encoder".into()));
        return;
    };
    let Some(ref_path) = current_input_video_input(ctx, node.id, "ref_video") else {
        error.set(Some("Подключите LTX Video Input на вход ref_video".into()));
        return;
    };
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let w = width.get_untracked() as usize;
    let h = height.get_untracked() as usize;
    let dur = duration_seconds.get_untracked() as f64;
    let fps = fps_from_idx(fps_idx.get_untracked());
    let ds = (downscale.get_untracked() as usize).max(1);
    let strength = ref_strength.get_untracked();
    let seed_v = seed.get_untracked();
    let ctrl = control_idx.get_untracked();
    let c_low = canny_low.get_untracked();
    let c_high = canny_high.get_untracked();
    let depth_dir = depth_model_path.get_untracked();
    if ctrl == 2 && depth_dir.is_none() {
        error.set(Some("control=depth требует каталог Depth Anything V2".into()));
        running.set(false);
        return;
    }

    let _ = thread::Builder::new()
        .name("synthos-ltx-iclora".into())
        .spawn(move || {
            let r = worker(&handle, &v_enc, &a_enc, &ref_path, w, h, dur, fps, ds, strength, ctrl, c_low, c_high, depth_dir, seed_v, progress_pct, &cancel, &v_out, &a_out);
            match r {
                Ok(()) => {
                    run_on_main_thread(move || {
                        output_version.update(|v| *v = v.wrapping_add(1));
                    });
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

#[allow(clippy::too_many_arguments)]
fn worker(
    handle: &LtxModelHandle,
    v_enc: &synaptix_core::tensor::Tensor,
    a_enc: &synaptix_core::tensor::Tensor,
    ref_path: &std::path::Path,
    width: usize,
    height: usize,
    duration: f64,
    fps: f64,
    downscale: usize,
    ref_strength: f32,
    control: usize,
    canny_low: f32,
    canny_high: f32,
    depth_dir: Option<std::path::PathBuf>,
    seed: u64,
    progress_pct: RwSignal<f32>,
    cancel: &std::sync::atomic::AtomicBool,
    v_out: &Arc<syngui::core::sync::Mutex<Option<LtxVideoLatent>>>,
    a_out: &Arc<syngui::core::sync::Mutex<Option<synaptix_core::tensor::Tensor>>>,
) -> std::result::Result<(), String> {
    let dev = device_from_idx(handle.device_idx);
    let (hp, wp) = latent_grid(width, height);
    let fp = frames_for_duration(duration, fps);
    let out_frames = out_frame_count(fp);
    let tv_max = fp * hp * wp;

    // ref-видео на разрешении target/downscale → (опц. control-препроцессинг:
    // canny edges / depth map) → кадры → токены + позиции.
    let (rph, rpw) = (hp * 32 / downscale, wp * 32 / downscale);
    let mut ref_frames = shared::load_video_frames(ref_path, rph, rpw, out_frames, dev)?;
    ref_frames = match control {
        1 => shared::apply_canny_frames(&ref_frames, canny_low, canny_high)?,
        2 => {
            let dir = depth_dir.as_ref().ok_or("control=depth требует Depth-модель")?;
            shared::apply_depth_frames(&ref_frames, dir, dev)?
        }
        _ => ref_frames,
    };
    let (ref_tokens, fpr, hpr, wpr) = shared::ref_latent_tokens(handle, &ref_frames)?;
    let ref_pos = ref_video_positions(fpr, hpr, wpr, fps, downscale);

    let dit = shared::load_avdit(handle, tv_max)?;
    let hooks = DenoiseHooks {
        progress: Some(&|p| {
            let pct = p.step as f32 / p.total.max(1) as f32;
            run_on_main_thread(move || progress_pct.set(pct));
        }),
        cancel: Some(cancel),
    };
    let seed_opt = if seed == 0 { None } else { Some(seed) };
    let (l, a) = denoise_av_append(
        &dit.dit, v_enc, a_enc, fp, hp, wp, &DISTILLED_SIGMAS,
        None, None, false,
        Some((&ref_tokens, &ref_pos, ref_strength)), None,
        fps, dev, seed_opt, &hooks,
    )
    .map_err(|e| match e {
        synaptix_video_ltx23::LtxError::Cancelled => "Отменено".to_string(),
        other => format!("ic-lora denoise: {other}"),
    })?;

    shared::hold_avdit(dit);
    if let Ok(mut g) = v_out.lock() {
        *g = Some(LtxVideoLatent { tensor: l, fp, hp, wp, fps });
    }
    if let Ok(mut g) = a_out.lock() {
        *g = Some(a);
    }
    Ok(())
}
