use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxFrames, NodeInstance, NodeRuntime, PortValue};
use super::super::acestep::field_row;
use super::super::{log_worker_done, log_worker_start};
use super::{current_input_audio, current_input_frames};
use crate::pages::node_editor::controls::av_scrubber::node_av_scrubber;
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub struct VideoSaveExec;

impl NodeExecutor for VideoSaveExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("frames");
        let _ = ctx.read_input("audio");
        ctx.write_output("path", PortValue::Empty);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VideoSave {
                path,
                running,
                error,
                saved,
                preview,
                preview_version,
            } => Some((*path, *running, *error, *saved, preview.clone(), *preview_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, saved, preview, preview_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(frames) = current_input_frames(ctx, node.id, "frames") else {
        error.set(Some(tr!("node.minimax_h3_save.connect_frames")));
        return;
    };
    let audio = current_input_audio(ctx, node.id, "audio");
    let out = path.get_untracked().unwrap_or_else(|| PathBuf::from("h3.mp4"));

    if let Ok(mut g) = preview.lock() {
        *g = Some((frames.clone(), audio.clone()));
    }
    preview_version.update(|v| *v = v.wrapping_add(1));

    running.set(true);
    error.set(None);
    saved.set(None);

    let _ = thread::Builder::new()
        .name("synthos-h3-save".into())
        .spawn(move || {
            let started = log_worker_start(
                "h3-video-save",
                &format!(
                    "{}x{}, {} кадров @{:.1}fps, аудио {}, файл {}",
                    frames.width,
                    frames.height,
                    frames.frames.len(),
                    frames.fps,
                    if audio.is_some() { "есть" } else { "нет" },
                    out.display()
                ),
            );
            let res = write_mp4(&frames, audio.as_deref(), &out);
            log_worker_done("h3-video-save", started, &res);
            match res {
                Ok(()) => {
                    error.set(None);
                    saved.set(Some(out.display().to_string()));
                }
                Err(e) => error.set(Some(e)),
            }
            running.set(false);
        });
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VideoSave { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn write_wav(path: &PathBuf, buf: &AudioBuffer) -> std::result::Result<(), String> {
    let bits = 16u16;
    let channels = buf.channels;
    let byte_rate = buf.sample_rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * (bits / 8);
    let data_len = (buf.pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + buf.pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&buf.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in buf.pcm.iter() {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("wav: {e}"))
}

fn write_mp4(
    frames: &Arc<LtxFrames>,
    audio: Option<&AudioBuffer>,
    out: &PathBuf,
) -> std::result::Result<(), String> {
    // Свой каталог на прогон: параллельные Save-ноды иначе стирали кадры
    // друг друга. Удаляется при выходе из функции, в том числе по ошибке.
    let tmp = crate::fsutil::TempDir::new("synthos_h3_frames")
        .map_err(|e| tr!("node.minimax_h3_save.tmp_dir_failed", error = e))?;
    let dir = tmp.path();

    let (w, h) = (frames.width as usize, frames.height as usize);
    for (i, fr) in frames.frames.iter().enumerate() {
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.reserve(3 * w * h);
        for p in 0..w * h {
            ppm.push(fr.rgba[p * 4]);
            ppm.push(fr.rgba[p * 4 + 1]);
            ppm.push(fr.rgba[p * 4 + 2]);
        }
        std::fs::write(dir.join(format!("f{i:05}.ppm")), ppm)
            .map_err(|e| tr!("node.minimax_h3_save.frame_write_failed", n = i, error = e))?;
    }

    let wav = dir.join("audio.wav");
    if let Some(a) = audio {
        write_wav(&wav, a)?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-framerate")
        .arg(format!("{}", frames.fps))
        .arg("-i")
        .arg(dir.join("f%05d.ppm"));
    if audio.is_some() {
        cmd.arg("-i").arg(&wav).arg("-c:a").arg("aac").arg("-b:a").arg("192k").arg("-shortest");
    }
    let status = cmd
        .arg("-c:v")
        .arg("libx264")
        .arg("-crf")
        .arg("17")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(out)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !status.success() {
        return Err(tr!("node.minimax_h3_save.ffmpeg_failed"));
    }
    Ok(())
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3VideoSave {
                path,
                running,
                error,
                saved,
                preview,
                preview_version,
            } => Some((*path, *running, *error, *saved, preview.clone(), *preview_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, saved, preview, preview_version)) = snapshot else {
        return Box::new(Column::new());
    };
    let scrubber = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = preview_version.get();
        match preview.lock().ok().and_then(|g| g.clone()) {
            Some((frames, audio)) => vec![node_av_scrubber(Some(frames), audio)],
            None => vec![],
        }
    });
    let status = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error.get() {
            return vec![Box::new(Text::new(tr!("nodes.common.error", error = msg)).class("audio-node-error"))];
        }
        if running.get() {
            return vec![Box::new(Text::new(tr!("node.minimax_h3_save.saving")).class("h3-node-running"))];
        }
        match saved.get() {
            Some(p) => vec![Box::new(Text::new(tr!("node.minimax_h3_save.saved", path = p)).class("h3-node-info"))],
            None => vec![],
        }
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(scrubber),
                field_row(
                    &tr!("node.minimax_h3_save.file_label"),
                    node_file_picker(tr!("nodes.common.save_mp4_path"), path, &[("MP4", &["mp4"])], |_| {}),
                ),
                Box::new(status),
            ]),
    )
}
