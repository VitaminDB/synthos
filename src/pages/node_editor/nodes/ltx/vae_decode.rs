//! `LtxVaeDecode` — видео-VAE decode: `(model, video_latent) → frames`.
//! RGB `[1,3,F,H,W]` → RGBA-кадры (`LtxFrames`, шарится Arc'ами) + превью
//! `FramesView` прямо в body (клик — play/pause).
//!
//! Перед decode снимается hold AvDit (`shared::release_avdit_hold`) и
//! делается sync+hard-trim — CLI-паттерн «дроп DiT до VAE» (иначе на 24GB
//! HD-decode не помещается).

use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::video::VideoFrame;
use syngui::widgets::visual::FramesView;
use syngui::widgets::{Column, Reactive};
use synaptix_core::device::Device;
use synaptix_video_ltx23::pipeline::rgb_to_frames;
use synaptix_video_ltx23::vae::VaeDecoder;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, LtxBlob, LtxFrames, LtxModelHandle, LtxVideoLatent, NodeInstance, NodeRuntime,
    PortValue,
};
use super::super::acestep::{field_row, status_row};
use super::shared;
use super::{current_input_model, current_input_video_latent, device_from_idx, progress_row};

pub struct VaeDecodeExec;

impl NodeExecutor for VaeDecodeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("model");
        let _ = ctx.read_input("video_latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::LtxVaeDecode {
                    frames_out,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match frames_out.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(fr) => PortValue::Data(Arc::new(DataBlob::Ltx(
                                LtxBlob::Frames(fr.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("frames", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxVaeDecode { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxVaeDecode {
                running,
                error,
                progress_pct,
                frames_out,
                preview_version,
                ..
            } => Some((*running, *error, *progress_pct, frames_out.clone(), *preview_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, progress_pct, frames_out, preview_version)) = snapshot else {
        return Box::new(Text::new("LtxVaeDecode: некорректный runtime").class("node-card-field-error"));
    };
    let loaded_name = use_signal(None::<String>);
    let preview = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = preview_version.get();
        let fr = frames_out.lock().ok().and_then(|g| g.clone());
        match fr {
            Some(fr) => vec![
                Box::new(
                    FramesView::new(fr.frames.clone(), fr.fps as f32)
                        .fit(syngui::widgets::ImageFit::Contain)
                        .class("ltx-preview-canvas"),
                ) as Box<dyn Widget>,
            ],
            None => vec![
                Box::new(Text::new("Превью появится после декода").class("audio-node-empty"))
                    as Box<dyn Widget>,
            ],
        }
    });
    let rows: Vec<Box<dyn Widget>> = vec![
        Box::new(preview),
        field_row("Прогресс", progress_row(running, progress_pct)),
        field_row(
            "Статус",
            status_row(running, error, loaded_name, "VAE decode…", "ltx-node-running"),
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
            NodeRuntime::LtxVaeDecode {
                running,
                error,
                progress_pct,
                frames_out,
                output_version,
                preview_version,
            } => Some((
                *running,
                *error,
                *progress_pct,
                frames_out.clone(),
                *output_version,
                *preview_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((running, error, progress_pct, frames_out, output_version, preview_version)) =
        snapshot
    else {
        return;
    };

    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        error.set(Some("Подключите LTX Checkpoint на вход model".into()));
        return;
    };
    let Some(latent) = current_input_video_latent(ctx, node.id, "video_latent") else {
        error.set(Some("Подключите video_latent (Sampler/Upscale)".into()));
        return;
    };
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);

    let _ = thread::Builder::new()
        .name("synthos-ltx-vae-decode".into())
        .spawn(move || {
            match worker(&handle, &latent, progress_pct) {
                Ok(frames) => {
                    if let Ok(mut g) = frames_out.lock() {
                        *g = Some(Arc::new(frames));
                    }
                    run_on_main_thread(move || {
                        output_version.update(|v| *v = v.wrapping_add(1));
                        preview_version.update(|v| *v = v.wrapping_add(1));
                    });
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

fn worker(
    handle: &LtxModelHandle,
    latent: &LtxVideoLatent,
    progress_pct: RwSignal<f32>,
) -> std::result::Result<LtxFrames, String> {
    let dev = device_from_idx(handle.device_idx);
    // «Держать в памяти» на Checkpoint: hold DiT переживает decode. Иначе —
    // прежнее поведение: отпустить DiT, чтобы VAE влез в VRAM.
    if !handle.resident {
        shared::release_avdit_hold();
    }
    shared::sync_and_trim(dev);
    let set_pct = |pct: f32| {
        run_on_main_thread(move || progress_pct.set(pct));
    };

    let ckpt = shared::load_ckpt(handle)?;
    let vae = VaeDecoder::load(&ckpt, dev).map_err(|e| format!("VAE: {e}"))?;
    set_pct(0.1);
    let rgb = vae
        .decode(&latent.tensor)
        .map_err(|e| format!("VAE decode: {e}"))?;
    set_pct(0.5);
    let frames = tensor_frames_to_rgba(&rgb, latent.fps, |done, total| {
        set_pct(0.5 + 0.5 * done as f32 / total.max(1) as f32);
    })?;
    Ok(frames)
}

pub fn tensor_frames_to_rgba(
    rgb: &synaptix_core::tensor::Tensor,
    fps: f64,
    on_progress: impl FnMut(usize, usize),
) -> std::result::Result<LtxFrames, String> {
    frames_to_rgba_impl(rgb, fps, true, on_progress)
}

pub fn tensor_frames_to_rgba_unit(
    rgb: &synaptix_core::tensor::Tensor,
    fps: f64,
    on_progress: impl FnMut(usize, usize),
) -> std::result::Result<LtxFrames, String> {
    frames_to_rgba_impl(rgb, fps, false, on_progress)
}

fn unit_frames(
    rgb: &synaptix_core::tensor::Tensor,
) -> std::result::Result<Vec<synaptix_core::tensor::Tensor>, String> {
    let d = rgb.dims().to_vec();
    let scaled = rgb
        .to_dtype(synaptix_core::dtype::DType::F32)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(d[2]);
    for f in 0..d[2] {
        out.push(
            scaled
                .narrow(2, f, 1)
                .and_then(|t| t.contiguous())
                .and_then(|t| t.reshape(vec![d[1], d[3], d[4]]))
                .and_then(|t| t.clamp(0.0, 1.0))
                .and_then(|t| t.contiguous())
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(out)
}

fn frames_to_rgba_impl(
    rgb: &synaptix_core::tensor::Tensor,
    fps: f64,
    signed: bool,
    mut on_progress: impl FnMut(usize, usize),
) -> std::result::Result<LtxFrames, String> {
    let dims = rgb.dims().to_vec();
    if dims.len() != 5 {
        return Err(format!("ожидался RGB [1,3,F,H,W], получено {dims:?}"));
    }
    let (h, w) = (dims[3], dims[4]);
    let planes = if signed {
        rgb_to_frames(rgb).map_err(|e| format!("rgb_to_frames: {e}"))?
    } else {
        unit_frames(rgb)?
    };
    let total = planes.len();
    let mut frames: Vec<Arc<VideoFrame>> = Vec::with_capacity(total);
    for (i, fr) in planes.into_iter().enumerate() {
        let v = fr
            .to_device(Device::Cpu)
            .map_err(|e| format!("кадр {i} → CPU: {e}"))?
            .reshape(vec![3 * h * w])
            .map_err(|e| format!("кадр {i} reshape: {e}"))?
            .to_vec1::<f32>()
            .map_err(|e| format!("кадр {i} → vec: {e}"))?;
        let mut rgba = vec![0u8; h * w * 4];
        let plane = h * w;
        for p in 0..plane {
            rgba[p * 4] = (v[p] * 255.0 + 0.5) as u8;
            rgba[p * 4 + 1] = (v[plane + p] * 255.0 + 0.5) as u8;
            rgba[p * 4 + 2] = (v[2 * plane + p] * 255.0 + 0.5) as u8;
            rgba[p * 4 + 3] = 255;
        }
        frames.push(Arc::new(VideoFrame {
            width: w as u32,
            height: h as u32,
            rgba: Arc::from(rgba.into_boxed_slice()),
            pts_sec: i as f64 / fps,
        }));
        on_progress(i + 1, total);
    }
    Ok(LtxFrames {
        frames: Arc::new(frames),
        width: w as u32,
        height: h as u32,
        fps,
    })
}
