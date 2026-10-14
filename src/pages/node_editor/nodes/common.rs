//! Общие части карточек нод: кнопка остановки, прогресс с оценкой
//! оставшегося времени, выбор каталога, запись WAV и mp4 из памяти.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::audio::AudioBuffer;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Reactive, Row, ToolButton};

use super::super::types::MemFrames;
use crate::icons::MI_FOLDER_OPEN;

/// Picker директории (Gemma-3-12B — каталог HF-модели, не файл).
pub fn dir_picker_row(
    tooltip: impl Into<String>,
    sig: RwSignal<Option<PathBuf>>,
) -> Box<dyn Widget> {
    let tooltip = tooltip.into();
    let tooltip_for_dialog = tooltip.clone();
    let pick_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tooltip)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(&tooltip_for_dialog);
            if let Some(p) = dlg.pick_folder() {
                sig.set(Some(p));
            }
        })
        .class("node-file-picker-btn");
    let dirname_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match sig.get() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Box::new(Text::new(name).class("node-file-picker-name"))
            }
            None => Box::new(Text::new(tr!("nodes.common.no_folder_selected")).class("node-file-picker-empty")),
        };
        vec![widget]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-file-picker-row")
            .children(vec![Box::new(pick_btn) as Box<dyn Widget>, Box::new(dirname_text)]),
    )
}

/// Кнопка остановки денойза: видна только при running, ставит cancel-флаг —
/// `denoise_av` вернёт `LtxError::Cancelled` на границе шага.
pub fn cancel_button(
    running: RwSignal<bool>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !running.get() {
            return vec![];
        }
        let cancel = cancel.clone();
        vec![
            Box::new(
                ToolButton::new(crate::icons::MI_STOP)
                    .tooltip(tr!("node.ltx.common.stop_generation"))
                    .on_click(move || {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    })
                    .class("ltx-cancel-btn"),
            ) as Box<dyn Widget>,
        ]
    }))
}

/// Оценщик оставшегося времени генерации: EMA-скорость прогресса по
/// событиям изменения `progress_pct`. Первый замер появляется после
/// второго события (интервал «загрузка модели → первый шаг» намеренно
/// отброшен — он не характеризует скорость шага денойза).
#[derive(Default)]
struct EtaState {
    /// Время и pct последнего изменения прогресса.
    last: Option<(std::time::Instant, f32)>,
    /// Сглаженная скорость, прогресс-единиц (0..1) в секунду.
    rate_ema: Option<f32>,
}

impl EtaState {
    fn observe(&mut self, now: std::time::Instant, pct: f32) {
        match self.last {
            None => {
                if pct > 0.0 {
                    self.last = Some((now, pct));
                }
            }
            Some((t0, p0)) => {
                if pct < p0 {
                    // Рестарт генерации — старые замеры невалидны.
                    self.rate_ema = None;
                    self.last = if pct > 0.0 { Some((now, pct)) } else { None };
                } else if pct > p0 {
                    let dt = (now - t0).as_secs_f32();
                    if dt > 0.0 {
                        let inst = (pct - p0) / dt;
                        self.rate_ema = Some(match self.rate_ema {
                            Some(ema) => ema * 0.6 + inst * 0.4,
                            None => inst,
                        });
                    }
                    self.last = Some((now, pct));
                }
            }
        }
    }

    /// Секунд до конца; между шагами убывает по часам (countdown), не
    /// уходя ниже нуля.
    fn remaining_secs(&self, now: std::time::Instant, pct: f32) -> Option<f32> {
        let rate = self.rate_ema?;
        let (t0, _) = self.last?;
        if rate <= f32::EPSILON || pct >= 1.0 {
            return None;
        }
        Some(((1.0 - pct) / rate - (now - t0).as_secs_f32()).max(0.0))
    }
}

/// `95 с` → `"1:35"`, `3723 с` → `"1:02:03"`.
fn fmt_eta(secs: f32) -> String {
    let s = secs.round().max(0.0) as u64;
    if s < 60 {
        tr!("nodes.common.eta_seconds", n = s)
    } else if s < 3600 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    }
}

/// Секундный тик для countdown'а ETA: пока нода работает, бампает сигнал
/// `tick` раз в секунду (Reactive-строка на него подписана), между шагами
/// денойза оценка «тает» по часам, а не замирает до следующего callback'а.
struct EtaTicker {
    running: RwSignal<bool>,
    tick: RwSignal<u64>,
    acc: syngui::core::sync::Mutex<std::time::Duration>,
}

impl super::super::controls::ProgressTicker for EtaTicker {
    fn active(&self) -> bool {
        self.running.get_untracked()
    }
    fn active_tracked(&self) -> bool {
        self.running.get()
    }
    fn tick(&self, dt: std::time::Duration) -> bool {
        if !self.running.get_untracked() {
            return false;
        }
        let due = match self.acc.lock() {
            Ok(mut acc) => {
                *acc += dt;
                if *acc >= std::time::Duration::from_secs(1) {
                    *acc = std::time::Duration::ZERO;
                    true
                } else {
                    false
                }
            }
            Err(_) => true,
        };
        if due {
            self.tick.update(|v| *v = v.wrapping_add(1));
        }
        true
    }
}

/// Прогресс-бар ноды: [`syngui::widgets::ProgressBar`] (MSS-класс
/// `.ltx-progress`) + строка «47% · ≈ 5:32» (процент и оценка оставшегося
/// времени). Виден только при running.
pub fn progress_row(running: RwSignal<bool>, progress_pct: RwSignal<f32>) -> Box<dyn Widget> {
    let eta = Arc::new(syngui::core::sync::Mutex::new(EtaState::default()));
    let tick = use_signal(0_u64);
    let ticker = super::super::controls::node_progress_animator(Arc::new(EtaTicker {
        running,
        tick,
        acc: syngui::core::sync::Mutex::new(std::time::Duration::ZERO),
    }));
    let row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !running.get() {
            if let Ok(mut st) = eta.lock() {
                *st = EtaState::default();
            }
            return vec![];
        }
        let _ = tick.get(); // подписка на секундный countdown-тик
        let pct = progress_pct.get().clamp(0.0, 1.0);
        let now = std::time::Instant::now();
        let label = match eta.lock() {
            Ok(mut st) => {
                st.observe(now, pct);
                match st.remaining_secs(now, pct) {
                    Some(rem) => format!("{}% · ≈ {}", (pct * 100.0) as i32, fmt_eta(rem)),
                    None => format!("{}%", (pct * 100.0) as i32),
                }
            }
            Err(_) => format!("{}%", (pct * 100.0) as i32),
        };
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("ltx-progress-row")
                .children(vec![
                    Box::new(syngui::widgets::ProgressBar::with_value(pct).class("ltx-progress"))
                        as Box<dyn Widget>,
                    Box::new(Text::new(label).class("ltx-progress-label")),
                ]),
        ) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("ltx-progress-row")
            .children(vec![ticker, Box::new(row) as Box<dyn Widget>]),
    )
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
    frames: &MemFrames,
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
