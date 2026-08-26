//! Универсальный видеоплеер: файл (ffmpeg `VideoPlayer` + `VideoView`) ИЛИ
//! кадры из памяти (`FramesView` + `AudioPlayer`). Режим выбирается по
//! наличию входа `frames` (память приоритетнее файла) — плеер ставится на
//! выход LTX-пайплайна вместо/параллельно Video Save, либо открывает любой
//! файл с диска как раньше.

use std::path::PathBuf;
use std::sync::Arc;

use syngui::audio::{AudioBuffer, AudioPlayer};
use syngui::core::sync::Mutex;
use syngui::core::Size;
use syngui::mgui;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widget::WidgetExt;
use syngui::widgets::visual::{FramesView, VideoView};
use syngui::widgets::{
    Column, DecoratedBox, Dropdown, DropdownItem, Reactive, Row, Slider, ToolButton,
};

use crate::icons::{MI_FOLDER_OPEN, MI_MOVIE, MI_PAUSE, MI_PLAY_ARROW, MI_STOP, MI_VOLUME_UP};

use super::super::controls::fmt_mmss;
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
    mem_audio: Arc<Mutex<Option<AudioPlayer>>>,
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
        mem_audio,
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
            mem_audio: mem_audio.clone(),
            preview_version: *preview_version,
        })
    } else {
        None
    }
}

impl FfmpegPlayerHandles {
    /// memory-режим активен, если на входе `frames` есть кадры.
    fn is_memory(&self) -> bool {
        self.frames_in.lock().map(|g| g.is_some()).unwrap_or(false)
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
        // memory-режим: проигрываем кадры из памяти через FramesView.
        let mem_frames = h_canvas.frames_in.lock().ok().and_then(|g| g.clone());
        if let Some(fr) = mem_frames {
            let canvas: Box<dyn Widget> = Box::new(
                FramesView::new(fr.frames.clone(), fr.fps as f32)
                    .fit(syngui::widgets::ImageFit::Contain)
                    .playing_signal(h_canvas.is_playing)
                    .position_signal(h_canvas.position)
                    .class("ffmpeg-player-canvas"),
            );
            return vec![canvas];
        }
        let p_opt = h_canvas.player.lock().ok().and_then(|g| g.clone());
        let canvas: Box<dyn Widget> = match p_opt {
            Some(arc_player) => Box::new(
                VideoView::new(arc_player)
                    .fit(syngui::widgets::ImageFit::Contain)
                    .position_signal(h_canvas.position)
                    .class("ffmpeg-player-canvas"),
            ),
            None => Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(
                            Text::new(format!("{} {}", MI_MOVIE, tr!("node.ffmpeg_player.canvas.no_video")))
                                .class("ffmpeg-player-empty-label"),
                        ),
                    )
                    .class("ffmpeg-player-canvas-empty"),
            ),
        };
        vec![canvas]
    });

    let h_play = h.clone();
    let play_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let playing = h_play.is_playing.get();
        let paused = h_play.is_paused.get();
        let icon = if playing && !paused { MI_PAUSE } else { MI_PLAY_ARROW };
        let class = if playing && !paused {
            "audio-node-transport-btn audio-node-pause"
        } else {
            "audio-node-transport-btn audio-node-play"
        };
        let h_btn = h_play.clone();
        let btn = ToolButton::new(icon)
            .tooltip(if playing && !paused { tr!("nodes.transport.pause") } else { tr!("nodes.transport.play") })
            .on_click(move || {
                toggle_play_pause(&h_btn);
            })
            .class(class);
        vec![Box::new(btn)]
    });

    let h_stop = h.clone();
    let stop_btn = ToolButton::new(MI_STOP)
        .tooltip(tr!("nodes.common.stop"))
        .on_click(move || {
            stop(&h_stop);
        })
        .class("audio-node-transport-btn audio-node-stop");

    let h_vol = h.clone();
    let volume_slider = Slider::new()
        .range(0.0, 2.0)
        .step(0.01)
        .value(h_vol.volume.get_untracked())
        .on_change(move |v| {
            let v = (v * 100.0).round() / 100.0;
            h_vol.volume.set(v);
            if let Ok(g) = h_vol.mem_audio.lock() {
                if let Some(p) = g.as_ref() {
                    p.set_volume(v.clamp(0.0, 1.0));
                }
            }
            if let Ok(g) = h_vol.player.lock() {
                if let Some(p_arc) = g.as_ref() {
                    if let Ok(p) = p_arc.lock() {
                        p.set_volume(v.clamp(0.0, 1.0));
                    }
                }
            }
        })
        .class("audio-node-volume-slider");

    let h_seek = h.clone();
    let seek_slider = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let dur = h_seek.duration.get().max(0.001);
        let pos = h_seek.position.get();
        let h_seek_cb = h_seek.clone();
        let widget: Box<dyn Widget> = Box::new(
            Slider::new()
                .range(0.0, dur)
                .step(0.05)
                .value(pos.clamp(0.0, dur))
                .on_change(move |sec| {
                    seek(&h_seek_cb, sec, dur);
                })
                .class("ffmpeg-player-seek-slider"),
        );
        vec![widget]
    });

    let h_tc = h.clone();
    let timecode = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let dur = h_tc.duration.get();
        let pos = h_tc.position.get();
        let txt = format!("{} / {}", fmt_mmss(pos as f64), fmt_mmss(dur as f64));
        vec![Box::new(Text::new(txt).class("audio-node-timecode")) as Box<dyn Widget>]
    });

    let controls = mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                play_btn,
                stop_btn,
                DecoratedBox::new().child(seek_slider).class("ffmpeg-player-seek-host"),
                Text::new(MI_VOLUME_UP).class("audio-node-volume-icon"),
                volume_slider,
                DecoratedBox::new().child(timecode).class("audio-node-slot-text"),
            ]
    };

    let toolbar = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                path_picker,
                DecoratedBox::new().child(path_label).class("ffmpeg-player-path-host"),
                hwaccel_dropdown,
            ]
    };

    let progress_animator = ProgressAnimator::new(runtime.clone());

    let body_col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![
            Box::new(DecoratedBox::new().child(video_canvas).class("ffmpeg-player-canvas-host"))
                as Box<dyn Widget>,
            Box::new(toolbar),
            Box::new(controls),
            Box::new(progress_animator),
        ]);

    Box::new(
        DecoratedBox::new()
            .child(body_col)
            .class("ffmpeg-player-body-host ffmpeg-player-node"),
    )
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

/// Старт memory-аудио (interleaved stereo) синхронно с FramesView. Если уже
/// игралось — resume.
fn mem_audio_play(h: &FfmpegPlayerHandles) {
    let Ok(mut g) = h.mem_audio.lock() else { return };
    if let Some(p) = g.as_ref() {
        p.resume();
        return;
    }
    let buf = h.audio_in.lock().ok().and_then(|b| b.clone());
    let Some(buf) = buf else { return };
    match AudioPlayer::start_stereo(buf.pcm.clone(), buf.sample_rate) {
        Ok(p) => {
            p.set_volume(h.volume.get_untracked().clamp(0.0, 1.0));
            *g = Some(p);
        }
        Err(e) => h.load_error.set(Some(format!("audio: {e:?}"))),
    }
}

fn toggle_play_pause(h: &FfmpegPlayerHandles) {
    if h.is_memory() {
        let playing = h.is_playing.get_untracked() && !h.is_paused.get_untracked();
        if playing {
            h.is_paused.set(true);
            h.is_playing.set(false);
            if let Ok(g) = h.mem_audio.lock() {
                if let Some(p) = g.as_ref() {
                    p.pause();
                }
            }
        } else {
            h.is_paused.set(false);
            h.is_playing.set(true);
            mem_audio_play(h);
        }
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

fn stop(h: &FfmpegPlayerHandles) {
    if h.is_memory() {
        if let Ok(mut g) = h.mem_audio.lock() {
            if let Some(p) = g.take() {
                p.stop();
            }
        }
        h.is_playing.set(false);
        h.is_paused.set(false);
        h.progress.set(0.0);
        h.position.set(0.0);
        return;
    }
    close_player(h);
    h.is_playing.set(false);
    h.is_paused.set(false);
    h.progress.set(0.0);
    h.position.set(0.0);
}

fn seek(h: &FfmpegPlayerHandles, sec: f32, dur: f32) {
    if h.is_memory() {
        h.position.set(sec);
        h.progress.set(sec / dur);
        if let Ok(g) = h.mem_audio.lock() {
            if let Some(p) = g.as_ref() {
                let _ = p.seek_seconds(sec as f64);
            }
        }
        return;
    }
    if let Ok(g) = h.player.lock() {
        if let Some(p_arc) = g.as_ref() {
            if let Ok(mut p) = p_arc.lock() {
                let _ = p.seek(sec as f64);
                h.position.set(sec);
                h.progress.set(sec / dur);
            }
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

use std::any::Any;
use syngui::core::{Point, Rect};
use syngui::input::{Event, EventResult};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, UpdateContext};

pub struct ProgressAnimator {
    runtime: Arc<Mutex<NodeRuntime>>,
}

impl ProgressAnimator {
    pub fn new(runtime: Arc<Mutex<NodeRuntime>>) -> Self {
        Self { runtime }
    }
}

impl Widget for ProgressAnimator {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ProgressAnimatorElement {
            id: ElementId::new(),
            runtime: self.runtime.clone(),
            bounds: Rect::zero(),
            classes: Vec::new(),
            dirty: DirtyFlags::LAYOUT,
            mss: MssFields::new(),
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, _t: &mut ElementTree, _p: ElementId) {}
}

struct ProgressAnimatorElement {
    id: ElementId,
    runtime: Arc<Mutex<NodeRuntime>>,
    bounds: Rect,
    classes: Vec<String>,
    dirty: DirtyFlags,
    mss: MssFields,
}

impl Element for ProgressAnimatorElement {
    fn update(&mut self, w: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(a) = w.as_any().downcast_ref::<ProgressAnimator>() {
            self.runtime = a.runtime.clone();
        }
    }
    fn layout(&mut self, _c: Constraints) -> Size {
        self.bounds = Rect::new(self.bounds.origin, Size::new(0.0, 0.0));
        Size::new(0.0, 0.0)
    }
    fn build_display_list(&self, _l: &mut DisplayList, _c: Rect) {}
    fn handle_event(&mut self, _e: &Event, _c: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }
    fn animate(&mut self, _dt: std::time::Duration) -> bool {
        let Ok(g) = self.runtime.lock() else { return false };
        let NodeRuntime::FfmpegPlayer {
            player,
            is_playing,
            is_paused,
            progress,
            duration,
            position,
            frames_in,
            ..
        } = &*g
        else {
            return false;
        };
        // memory-режим: FramesView сам ведёт position; здесь только progress
        // для таймкода/seek-слайдера.
        if frames_in.lock().map(|f| f.is_some()).unwrap_or(false) {
            if !is_playing.get_untracked() {
                return false;
            }
            let dur = duration.get_untracked().max(0.001);
            let pct = (position.get_untracked() / dur).clamp(0.0, 1.0);
            if (pct - progress.get_untracked()).abs() > 0.005 {
                progress.set(pct);
            }
            return true;
        }
        if !is_playing.get_untracked() || is_paused.get_untracked() {
            return false;
        }
        let Ok(p_g) = player.lock() else { return true };
        let Some(p_arc) = p_g.as_ref() else { return false };
        let Ok(p) = p_arc.lock() else { return true };
        let pos = p.position_sec() as f32;
        let dur = duration.get_untracked().max(0.001);
        let prev_pos = position.get_untracked();
        if (pos - prev_pos).abs() > 0.05 {
            position.set(pos);
        }
        let pct = (pos / dur).clamp(0.0, 1.0);
        let prev_pct = progress.get_untracked();
        if (pct - prev_pct).abs() > 0.005 {
            progress.set(pct);
        }
        true
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, p: Point) {
        self.bounds.origin = p;
    }
    fn mark_dirty(&mut self, f: DirtyFlags) {
        self.dirty |= f;
    }
    fn clear_dirty(&mut self, f: DirtyFlags) {
        self.dirty.remove(f);
    }
    fn is_dirty(&self, f: DirtyFlags) -> bool {
        self.dirty.contains(f)
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, i: ElementId) {
        self.id = i;
    }
    fn mount(&mut self, _t: &mut ElementTree) {}
    fn element_type_name(&self) -> &str {
        "FfmpegPlayerAnimator"
    }
    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, s: &ComputedStyle) {
        self.mss.apply(s);
    }
    fn passthrough_hit_test(&self) -> bool {
        true
    }
}
