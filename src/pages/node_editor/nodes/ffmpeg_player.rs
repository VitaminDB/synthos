//! Универсальный видеоплеер: файл (ffmpeg `VideoPlayer`) ИЛИ кадры из
//! памяти со звуком. Режим выбирается по наличию входа `frames` (память
//! приоритетнее файла) — плеер ставится на выход LTX/H3-пайплайна
//! вместо/параллельно Video Save, либо открывает любой файл с диска.
//!
//! Кадр и управление — общий плеер приложения
//! ([`crate::components::video_player`]); у ноды свои только выбор файла,
//! аппаратное декодирование и сохранённая громкость.

use std::path::PathBuf;
use std::sync::Arc;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widget::WidgetExt;
use syngui::widgets::{Column, DecoratedBox, Dropdown, DropdownItem, Reactive, Row, ToolButton};

use crate::components::video_player::{FramesSource, MediaSource, VideoPlayerView};
use crate::icons::{MI_FOLDER_OPEN, MI_MOVIE, MI_PLAY_ARROW};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::registry;
use super::super::types::{LtxFrames, NodeInstance, NodeRuntime, PortValue};

pub struct FfmpegPlayerExec;

impl NodeExecutor for FfmpegPlayerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let new_path = match ctx.read_input("source") {
            PortValue::Text(s) if !s.trim().is_empty() => Some(PathBuf::from(s.trim())),
            _ => None,
        };
        let frames_pv = ctx.read_input("frames");
        let new_frames = frames_pv.as_ltx_frames().or_else(|| frames_pv.as_h3_frames());
        let new_audio = ctx.read_input("audio").as_audio();

        let Ok(g) = ctx.runtime().lock() else { return };
        let NodeRuntime::FfmpegPlayer {
            current_path,
            video_out,
            audio_out,
            out_version,
            frames_in,
            audio_in,
            duration,
            preview_version,
            ..
        } = &*g
        else {
            return;
        };

        // memory-источник: смена кадров → обновить буфер, duration, превью.
        if let Ok(mut fg) = frames_in.lock() {
            let changed = match (fg.as_ref(), &new_frames) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                if let Some(fr) = &new_frames {
                    let dur = fr.frames.len() as f32 / (fr.fps as f32).max(1.0);
                    duration.set(dur);
                }
                *fg = new_frames.clone();
                preview_version.set(preview_version.get_untracked().wrapping_add(1));
            }
        }
        if let Ok(mut ag) = audio_in.lock() {
            *ag = new_audio;
        }

        if new_path.is_some() && current_path.get_untracked() != new_path {
            current_path.set(new_path);
        }

        let _ = out_version.get();
        let video_v = video_out.lock().ok().and_then(|g| g.clone());
        let audio_v = audio_out.lock().ok().and_then(|g| g.clone());
        drop(g);
        if let Some(v) = video_v {
            ctx.write_output("video", PortValue::VideoStream(v));
        }
        if let Some(a) = audio_v {
            ctx.write_output("audio", PortValue::AudioStream(a));
        }
    }
}

#[derive(Clone)]
struct FfmpegPlayerHandles {
    player: Arc<Mutex<Option<Arc<Mutex<VideoPlayer>>>>>,
    current_path: RwSignal<Option<PathBuf>>,
    load_error: RwSignal<Option<String>>,
    is_playing: RwSignal<bool>,
    is_paused: RwSignal<bool>,
    progress: RwSignal<f32>,
    duration: RwSignal<f32>,
    position: RwSignal<f32>,
    volume: RwSignal<f32>,
    hwaccel_idx: RwSignal<usize>,
    video_out: Arc<Mutex<Option<Arc<syngui::video::VideoStream>>>>,
    audio_out: Arc<Mutex<Option<Arc<syngui::audio::AudioStream>>>>,
    out_version: RwSignal<u32>,
    frames_in: Arc<Mutex<Option<Arc<LtxFrames>>>>,
    audio_in: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    preview_version: RwSignal<u32>,
}

fn handles_from(rt: &NodeRuntime) -> Option<FfmpegPlayerHandles> {
    if let NodeRuntime::FfmpegPlayer {
        player,
        current_path,
        load_error,
        is_playing,
        is_paused,
        progress,
        duration,
        position,
        volume,
        hwaccel_idx,
        size: _,
        video_out,
        audio_out,
        out_version,
        frames_in,
        audio_in,
        // Звук кадров из памяти теперь ведёт `FramesSource` плеера.
        mem_audio: _,
        preview_version,
    } = rt
    {
        Some(FfmpegPlayerHandles {
            player: player.clone(),
            current_path: *current_path,
            load_error: *load_error,
            is_playing: *is_playing,
            is_paused: *is_paused,
            progress: *progress,
            duration: *duration,
            position: *position,
            volume: *volume,
            hwaccel_idx: *hwaccel_idx,
            video_out: video_out.clone(),
            audio_out: audio_out.clone(),
            out_version: *out_version,
            frames_in: frames_in.clone(),
            audio_in: audio_in.clone(),
            preview_version: *preview_version,
        })
    } else {
        None
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();
    let h = match runtime.lock().ok().as_deref().and_then(handles_from) {
        Some(h) => h,
        None => return error_widget(tr!("nodes.common.invalid_runtime", name = "FfmpegPlayer")),
    };

    let _ = registry::meta(node.kind);

    let h_pick = h.clone();
    let path_picker = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tr!("node.ffmpeg_player.tooltip.pick_video"))
        .on_click(move || {
            let path = rfd::FileDialog::new()
                .add_filter(
                    tr!("node.ffmpeg_player.filter.video"),
                    &["mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "wmv"],
                )
                .pick_file();
            if let Some(p) = path {
                close_player(&h_pick);
                h_pick.current_path.set(Some(p));
                h_pick.load_error.set(None);
                h_pick.is_playing.set(false);
                h_pick.is_paused.set(false);
                h_pick.progress.set(0.0);
                h_pick.position.set(0.0);
                h_pick.duration.set(0.0);
            }
        })
        .class("audio-node-open-btn");

    let current_path_sig = h.current_path;
    let load_error_sig = h.load_error;
    let frames_in_label = h.frames_in.clone();
    let path_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let p = current_path_sig.get();
        let err = load_error_sig.get();
        let mem = frames_in_label.lock().map(|g| g.is_some()).unwrap_or(false);
        let widget: Box<dyn Widget> = match (mem, p, err) {
            (_, _, Some(e)) => Box::new(Text::new(tr!("nodes.common.error", error = e)).class("audio-node-error")),
            (true, _, _) => {
                Box::new(Text::new(tr!("node.ffmpeg_player.status.from_memory")).class("audio-node-filename"))
            }
            (false, Some(path), _) => Box::new(
                Text::new(
                    path.file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string()),
                )
                .class("audio-node-filename"),
            ),
            _ => Box::new(Text::new(tr!("nodes.common.no_file_selected")).class("audio-node-empty")),
        };
        vec![widget]
    });

    let hwaccel_idx_sig = h.hwaccel_idx;
    let hwaccel_items: Vec<DropdownItem> = HWACCEL_LABELS
        .iter()
        .map(|name| DropdownItem::new(*name, *name))
        .collect();
    let initial_hwaccel = HWACCEL_LABELS
        .get(hwaccel_idx_sig.get_untracked().min(HWACCEL_LABELS.len() - 1))
        .copied()
        .unwrap_or("Auto");
    let hwaccel_dropdown = Dropdown::new()
        .items(hwaccel_items)
        .selected(initial_hwaccel)
        .on_change(move |v| {
            if let Some(idx) = HWACCEL_LABELS.iter().position(|n| *n == v) {
                hwaccel_idx_sig.set(idx);
            }
        })
        .class("node-input-dropdown");

    let h_canvas = h.clone();
    let out_version_canvas = h.out_version;
    let preview_version_canvas = h.preview_version;
    let video_canvas = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = out_version_canvas.get();
        let _ = preview_version_canvas.get();
        let has_path = h_canvas.current_path.get().is_some();
        vec![canvas(&h_canvas, has_path)]
    });

    let toolbar = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                path_picker,
                DecoratedBox::new().child(path_label).class("ffmpeg-player-path-host"),
                hwaccel_dropdown,
            ]
    };

    let body_col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![
            Box::new(DecoratedBox::new().child(video_canvas).class("ffmpeg-player-canvas-host"))
                as Box<dyn Widget>,
            Box::new(toolbar),
        ]);

    Box::new(
        DecoratedBox::new()
            .child(body_col)
            .class("ffmpeg-player-body-host ffmpeg-player-node"),
    )
}

/// Кадр ноды — общий плеер приложения (`components::video_player`): кадры
/// из памяти, если они пришли на вход, иначе открытый файл. Файл ещё не
/// открыт — заглушка с ⏵, которая его откроет.
fn canvas(h: &FfmpegPlayerHandles, has_path: bool) -> Box<dyn Widget> {
    if let Some(src) = memory_source(h) {
        return Box::new(VideoPlayerView::new(src).volume_signal(h.volume).build());
    }
    if let Some(p) = h.player.lock().ok().and_then(|g| g.clone()) {
        return Box::new(VideoPlayerView::file(p).volume_signal(h.volume).build());
    }
    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    if has_path {
        let h_play = h.clone();
        items.push(Box::new(
            ToolButton::new(MI_PLAY_ARROW)
                .tooltip(tr!("nodes.transport.play"))
                .on_click(move || toggle_play_pause(&h_play))
                .class("vp-big-play"),
        ));
    } else {
        items.push(Box::new(
            Text::new(format!("{} {}", MI_MOVIE, tr!("node.ffmpeg_player.canvas.no_video")))
                .class("ffmpeg-player-empty-label"),
        ));
    }
    Box::new(
        DecoratedBox::new().class("ffmpeg-player-canvas-empty").child(
            Center::new().child(
                Column::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(items),
            ),
        ),
    )
}

/// Источник кадров из памяти для входа `frames` (со звуком со входа
/// `audio`): тот же, что показывает плеер на кадре, пока он жив.
fn memory_source(h: &FfmpegPlayerHandles) -> Option<Arc<FramesSource>> {
    let fr = h.frames_in.lock().ok().and_then(|g| g.clone())?;
    let audio = h.audio_in.lock().ok().and_then(|g| g.clone());
    Some(FramesSource::shared(&fr.frames, fr.fps as f32, audio))
}

pub fn on_run(node: &NodeInstance, _ctx: &super::super::state::NodeEditorCtx) {
    let Some(h) = node.runtime.lock().ok().as_deref().and_then(handles_from) else {
        return;
    };
    toggle_play_pause(&h);
}

/// Плеер не «занят» для секвенсера: он показывает результат, а не считает
/// его. Раньше busy=`is_playing` держал прогон открытым всё время
/// воспроизведения — 8-секундный ролик стоил минуты в отчёте («Видео-плеер ·
/// 1м 1с»), а агентский ход всё это время ждал вместе с выгруженной LLM.
/// Воспроизведение продолжается само по себе, downstream-нод у плеера нет.
pub fn busy_signal(_node: &NodeInstance) -> Option<RwSignal<bool>> {
    None
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("ffmpeg-player-error")))
}

pub const HWACCEL_LABELS: &[&str] = &[
    "Auto",
    "Software",
    "VAAPI",
    "NVDEC",
    "VideoToolbox",
    "D3D11Va",
    "DXVA2",
    "Vulkan",
];

fn hwaccel_from_idx(i: usize) -> HwAccel {
    match i {
        0 => HwAccel::Auto,
        1 => HwAccel::None,
        2 => HwAccel::Vaapi,
        3 => HwAccel::Nvdec,
        4 => HwAccel::VideoToolbox,
        5 => HwAccel::D3D11Va,
        6 => HwAccel::Dxva2,
        7 => HwAccel::Vulkan,
        _ => HwAccel::Auto,
    }
}

/// ▶ ноды (секвенсер, `on_run`) и заглушка кадра: пауза/старт того же
/// плеера, что на кадре; файл ещё не открыт — открыть и запустить.
fn toggle_play_pause(h: &FfmpegPlayerHandles) {
    if let Some(src) = memory_source(h) {
        let start = src.is_paused();
        if start {
            src.play();
        } else {
            src.pause();
        }
        h.is_playing.set(start);
        h.is_paused.set(!start);
        return;
    }

    if let Ok(g) = h.player.lock() {
        if let Some(p_arc) = g.as_ref() {
            if let Ok(mut p) = p_arc.lock() {
                if p.is_paused() {
                    p.play();
                    h.is_paused.set(false);
                    h.is_playing.set(true);
                } else {
                    p.pause();
                    h.is_paused.set(true);
                }
            }
            return;
        }
    }

    let Some(path) = h.current_path.get_untracked() else {
        h.load_error.set(Some(tr!("nodes.common.no_file_selected")));
        return;
    };
    let path_str = path.to_string_lossy().to_string();
    let accel = hwaccel_from_idx(h.hwaccel_idx.get_untracked());
    match VideoPlayer::open_with_hwaccel(&path_str, accel) {
        Ok(p) => {
            p.set_volume(h.volume.get_untracked().clamp(0.0, 1.0));
            let m = p.meta();
            h.duration.set(m.duration_sec as f32);
            h.progress.set(0.0);

            let v_stream = p.install_video_tee();
            let a_stream = p.install_audio_tee();
            if let Ok(mut o) = h.video_out.lock() {
                *o = v_stream;
            }
            if let Ok(mut o) = h.audio_out.lock() {
                *o = a_stream;
            }
            h.out_version.set(h.out_version.get_untracked().wrapping_add(1));

            if let Ok(mut g) = h.player.lock() {
                *g = Some(Arc::new(Mutex::new(p)));
            }
            h.is_playing.set(true);
            h.is_paused.set(false);
            h.load_error.set(None);
        }
        Err(e) => {
            h.load_error.set(Some(format!("{e:?}")));
        }
    }
}

fn close_player(h: &FfmpegPlayerHandles) {
    if let Ok(mut g) = h.player.lock() {
        if let Some(p_arc) = g.take() {
            if let Ok(p) = p_arc.lock() {
                p.uninstall_tees();
            }
        }
    }
    if let Ok(mut o) = h.video_out.lock() {
        *o = None;
    }
    if let Ok(mut o) = h.audio_out.lock() {
        *o = None;
    }
    h.out_version.set(h.out_version.get_untracked().wrapping_add(1));
}
