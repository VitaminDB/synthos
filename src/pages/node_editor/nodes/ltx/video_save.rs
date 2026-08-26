//! `LtxVideoSave` — sink: `(frames, audio?) → mp4`. Кадры RGBA стримятся в
//! stdin ffmpeg (`-f rawvideo`) без промежуточных PPM на диске; аудио — во
//! временный WAV (hound) рядом с выходным файлом, муксится `-c:a aac
//! -shortest`. Видео-кодек libx264 + yuv420p (как CLI `write_mp4`).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Column, Reactive, Row, ToolButton};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{LtxFrames, NodeInstance, NodeRuntime, PortValue, SaveStatus};
use super::super::acestep::{field_row, status_row};
use super::{current_input_audio, current_input_frames, progress_row};
use crate::icons::MI_SAVE;

pub struct VideoSaveExec;

impl NodeExecutor for VideoSaveExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("frames");
        let _ = ctx.read_input("audio");
        let _ = PortValue::Empty;
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxVideoSave { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn save_path_row(path: RwSignal<String>) -> Box<dyn Widget> {
    let pick_btn = ToolButton::new(MI_SAVE)
        .tooltip(tr!("nodes.common.save_mp4_path"))
        .on_click(move || {
            let dlg = rfd::FileDialog::new()
                .add_filter("MP4", &["mp4"])
                .set_file_name("ltx_video.mp4")
                .set_title(tr!("node.ltx_video_save.dialog.save_video_title"));
            if let Some(p) = dlg.save_file() {
                path.set(p.to_string_lossy().to_string());
            }
        })
        .class("node-file-picker-btn");
    let name_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let p = path.get();
        let widget: Box<dyn Widget> = if p.is_empty() {
            Box::new(Text::new(tr!("nodes.common.no_file_selected")).class("node-file-picker-empty"))
        } else {
            let name = PathBuf::from(&p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(p);
            Box::new(Text::new(name).class("node-file-picker-name"))
        };
        vec![widget]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-file-picker-row")
            .children(vec![Box::new(pick_btn) as Box<dyn Widget>, Box::new(name_text)]),
    )
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::LtxVideoSave {
                path,
                running,
                error,
                progress_pct,
                status,
            } => Some((*path, *running, *error, *progress_pct, *status)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, progress_pct, status)) = snapshot else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "LtxVideoSave"))
                .class("node-card-field-error"),
        );
    };
    let saved_name = use_signal(None::<String>);
    let status_view = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let label: Box<dyn Widget> = match status.get() {
            SaveStatus::Saved(p) => {
                let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                Box::new(
                    Text::new(tr!("node.ltx_video_save.status.saved", name = name))
                        .class("audio-node-meta"),
                )
            }
            _ => Box::new(Text::new("").class("audio-node-meta")),
        };
        vec![label]
    });
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(&tr!("node.ltx_video_save.field.file"), save_path_row(path)),
        field_row(&tr!("node.ltx.common.progress"), progress_row(running, progress_pct)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(running, error, saved_name, "ffmpeg mux…", "ltx-node-running"),
        ),
        Box::new(status_view),
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
            NodeRuntime::LtxVideoSave {
                path,
                running,
                error,
                progress_pct,
                status,
            } => Some((*path, *running, *error, *progress_pct, *status)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, progress_pct, status)) = snapshot else {
        return;
    };

    let Some(frames) = current_input_frames(ctx, node.id, "frames") else {
        error.set(Some(tr!("node.ltx_video_save.err.connect_frames")));
        return;
    };
    let audio = current_input_audio(ctx, node.id, "audio");
    let out_path = path.get_untracked();
    if out_path.trim().is_empty() {
        error.set(Some(tr!("node.ltx_video_save.err.choose_save_path")));
        return;
    }
    if running.get_untracked() {
        return;
    }
    running.set(true);
    error.set(None);
    progress_pct.set(0.0);
    status.set(SaveStatus::Writing);

    let _ = thread::Builder::new()
        .name("synthos-ltx-video-save".into())
        .spawn(move || {
            let out = PathBuf::from(out_path.trim());
            match encode_mp4(&frames, audio.as_deref(), &out, Some(progress_pct)) {
                Ok(()) => {
                    status.set(SaveStatus::Saved(out));
                    error.set(None);
                }
                Err(e) => {
                    status.set(SaveStatus::Error(e.clone()));
                    error.set(Some(e));
                }
            }
            running.set(false);
        });
}

pub fn write_wav(path: &PathBuf, buf: &AudioBuffer) -> std::result::Result<(), String> {
    let spec = hound::WavSpec {
        channels: buf.channels,
        sample_rate: buf.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("wav create: {e}"))?;
    for s in buf.pcm.iter() {
        writer.write_sample(*s).map_err(|e| format!("wav write: {e}"))?;
    }
    writer.finalize().map_err(|e| format!("wav finalize: {e}"))
}

/// Кодирование кадров (+ аудио) в mp4 через ffmpeg. `progress_pct` — None,
/// когда вызывающему нечего показывать (проброс результата плеера в чат).
pub fn encode_mp4(
    frames: &LtxFrames,
    audio: Option<&AudioBuffer>,
    out: &PathBuf,
    progress_pct: Option<RwSignal<f32>>,
) -> std::result::Result<(), String> {
    if frames.frames.is_empty() {
        return Err(tr!("node.ltx_video_save.err.no_frames"));
    }
    let wav_tmp = audio.map(|buf| {
        let p = out.with_extension("ltx_audio.tmp.wav");
        (p, buf)
    });
    if let Some((p, buf)) = &wav_tmp {
        write_wav(p, buf)?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", frames.width, frames.height)])
        .args(["-r", &format!("{}", frames.fps)])
        .args(["-i", "-"]);
    if let Some((p, _)) = &wav_tmp {
        cmd.arg("-i").arg(p);
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    if wav_tmp.is_some() {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    cmd.arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| tr!("node.ltx.shared.err_ffmpeg_run", error = e))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| tr!("node.ltx_video_save.err.ffmpeg_stdin_unavailable"))?;
    let total = frames.frames.len();
    let mut write_err: Option<String> = None;
    for (i, fr) in frames.frames.iter().enumerate() {
        if let Err(e) = stdin.write_all(&fr.rgba) {
            write_err = Some(format!("ffmpeg stdin: {e}"));
            break;
        }
        if let Some(sig) = progress_pct {
            let pct = (i + 1) as f32 / total as f32;
            run_on_main_thread(move || sig.set(pct));
        }
    }
    drop(stdin);
    let out_res = child.wait_with_output().map_err(|e| format!("ffmpeg wait: {e}"));
    if let Some((p, _)) = &wav_tmp {
        let _ = std::fs::remove_file(p);
    }
    let output = out_res?;
    if let Some(e) = write_err {
        return Err(e);
    }
    if !output.status.success() {
        let tail: String = String::from_utf8_lossy(&output.stderr)
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        let code = format!("{:?}", output.status.code());
        return Err(tr!("node.ltx_video_save.err.ffmpeg_exit_code", code = code, tail = tail));
    }
    Ok(())
}
