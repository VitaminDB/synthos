//! Body для NodeKind::AudioPlayer.
//!
//! Layout — горизонтальный (широкая плоская карточка):
//!   [Play/Pause] [Stop] [waveform с progress+seek] [Volume slider] [timecode]
//!
//! Long-lived runtime: `NodeRuntime::AudioPlayer { player, is_playing,
//! is_paused, progress, volume, last_input_ptr, pcm_view }`.

use std::sync::Arc;

use syngui::audio::AudioPlayer;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::visual::StaticWaveform;
use syngui::widgets::{DecoratedBox, Reactive, Row, Slider, ToolButton};

use crate::icons::{MI_PAUSE, MI_PLAY_ARROW, MI_STOP, MI_VOLUME_UP};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::controls::fmt_mmss;
use super::super::types::{NodeInstance, NodeRuntime, PortValue, PrevSource};

/// Executor для AudioPlayer (sink-нода без outputs).
///
/// Читает audio с input-port'а `in`, обновляет `pcm_view` и детектит смену
/// источника через [`PrevSource`] (kind + `Arc::as_ptr` без clone содержимого).
/// При смене источника:
/// - останавливает текущий плеер и сбрасывает progress;
/// - для `Audio(buf)` — обновляет `pcm_view = Some(buf)`, `is_streaming=false`;
/// - для `AudioStream(s)` — забирает receiver в `pending_stream` (single-sub),
///   `pcm_view=None`, `is_streaming=true`, `stream_sample_rate = s.sample_rate`;
/// - для `Empty/Float` — всё сбрасывается в `None`.
///
/// Пользователь должен явно нажать Play после смены — это сохраняет
/// контроль над запуском cpal-стрима.
pub struct AudioPlayerExec;

impl NodeExecutor for AudioPlayerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::Audio(b) => PrevSource::Buffer(Arc::as_ptr(b) as usize),
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            _ => PrevSource::None,
        };
        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::AudioPlayer {
                pcm_view,
                last_input_kind,
                player,
                is_playing,
                is_paused,
                progress,
                pending_stream,
                stream_sample_rate,
                is_streaming,
                ..
            } = &mut *g
            {
                if new_kind != *last_input_kind {
                    *last_input_kind = new_kind;
                    if let Some(p) = player.take() {
                        p.stop();
                    }
                    is_playing.set(false);
                    is_paused.set(false);
                    progress.set(0.0);
                    match &in_pv {
                        PortValue::Audio(b) => {
                            pcm_view.set(Some(b.clone()));
                            is_streaming.set(false);
                            *stream_sample_rate = 0;
                            if let Ok(mut p) = pending_stream.lock() {
                                *p = None;
                            }
                        }
                        PortValue::AudioStream(s) => {
                            pcm_view.set(None);
                            is_streaming.set(true);
                            *stream_sample_rate = s.sample_rate;
                            if let Ok(mut p) = pending_stream.lock() {
                                *p = s.take_receiver();
                            }
                        }
                        _ => {
                            pcm_view.set(None);
                            is_streaming.set(false);
                            *stream_sample_rate = 0;
                            if let Ok(mut p) = pending_stream.lock() {
                                *p = None;
                            }
                        }
                    }
                }
            }
        }
        // Pseudo-output для согласованности структуры values.
        ctx.write_output("in", PortValue::Empty);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (pcm_view, is_playing, is_paused, progress_sig, volume_sig, is_streaming) =
        match runtime.lock() {
            Ok(g) => match &*g {
                NodeRuntime::AudioPlayer {
                    pcm_view,
                    is_playing,
                    is_paused,
                    progress,
                    volume,
                    is_streaming,
                    ..
                } => (
                    *pcm_view,
                    *is_playing,
                    *is_paused,
                    *progress,
                    *volume,
                    *is_streaming,
                ),
                _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "AudioPlayer")),
            },
            Err(_) => return error_widget("AudioPlayer: lock error"),
        };

    let runtime_for_seek = runtime.clone();
    let waveform = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Streaming-режим: waveform/seek не имеют смысла (длина неизвестна,
        // PCM не аккумулируется). Показываем пульсирующий «LIVE» бейдж.
        if is_streaming.get() {
            return vec![Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(Text::new(format!(
                            "{}  LIVE",
                            crate::icons::MI_GRAPHIC_EQ
                        ))
                            .class("audio-node-live-badge")),
                    )
                    .class("audio-node-waveform-live-host"),
            )];
        }
        let pcm = pcm_view.get();
        let runtime_seek = runtime_for_seek.clone();
        let widget: Box<dyn Widget> = match pcm {
            Some(buf) => {
                let duration = buf.duration_seconds();
                Box::new(
                    StaticWaveform::new()
                        .pcm(Some(buf))
                        .progress(progress_sig.get())
                        .on_seek(move |t| {
                            let secs = t as f64 * duration;
                            if let Ok(g) = runtime_seek.lock() {
                                if let NodeRuntime::AudioPlayer { player, .. } = &*g {
                                    if let Some(p) = player {
                                        let _ = p.seek_seconds(secs);
                                    }
                                }
                            }
                            progress_sig.set(t);
                        })
                        .height(40.0)
                        .class("audio-node-waveform"),
                )
            }
            None => Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(
                            Text::new(tr!("node.audio_player.waveform.connect_source"))
                                .class("audio-node-waveform-placeholder"),
                        ),
                    )
                    .class("audio-node-waveform-empty"),
            ),
        };
        vec![widget]
    });

    let runtime_btn = runtime.clone();
    let play_pause_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let playing = is_playing.get();
        let paused = is_paused.get();
        let runtime_btn = runtime_btn.clone();
        let icon = if playing && !paused { MI_PAUSE } else { MI_PLAY_ARROW };
        let tooltip = if playing && !paused { tr!("nodes.transport.pause") } else { tr!("nodes.transport.play") };
        let class = if playing && !paused {
            "audio-node-transport-btn audio-node-pause"
        } else {
            "audio-node-transport-btn audio-node-play"
        };
        let btn = ToolButton::new(icon)
            .tooltip(tooltip)
            .on_click(move || {
                toggle_play_pause(
                    &runtime_btn,
                    is_playing,
                    is_paused,
                    pcm_view,
                    progress_sig,
                    volume_sig,
                );
            })
            .class(class);
        vec![Box::new(btn)]
    });

    let runtime_stop = runtime.clone();
    let stop_btn = ToolButton::new(MI_STOP)
        .tooltip(tr!("nodes.common.stop"))
        .on_click(move || {
            stop_player(&runtime_stop, is_playing, is_paused, progress_sig);
        })
        .class("audio-node-transport-btn audio-node-stop");

    let runtime_vol = runtime.clone();
    let volume_slider = Slider::new()
        .range(0.0, 2.0)
        .step(0.01)
        .value(volume_sig.get_untracked())
        .on_change(move |v| {
            let v = (v * 100.0).round() / 100.0;
            volume_sig.set(v);
            if let Ok(g) = runtime_vol.lock() {
                if let NodeRuntime::AudioPlayer { player, .. } = &*g {
                    if let Some(p) = player {
                        p.set_volume(v);
                    }
                }
            }
        })
        .class("audio-node-volume-slider");

    let timecode = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // В streaming-режиме длительность неизвестна — показываем `--:--`
        // справа от элапсед-времени, чтобы пользователь видел live-running.
        let txt = if is_streaming.get() {
            "live  /  --:--".to_string()
        } else {
            let p = progress_sig.get();
            let pcm = pcm_view.get();
            let dur = pcm.as_ref().map(|b| b.duration_seconds()).unwrap_or(0.0);
            let cur = p as f64 * dur;
            format!("{} / {}", fmt_mmss(cur), fmt_mmss(dur))
        };
        vec![Box::new(Text::new(txt).class("audio-node-timecode")) as Box<dyn Widget>]
    });

    let runtime_anim = runtime.clone();
    let progress_animator =
        ProgressAnimator::new(runtime_anim, is_playing, is_paused, progress_sig);

    let row = mgui! {
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().child(play_pause_btn).class("audio-node-slot-btn"),
                stop_btn,
                DecoratedBox::new().child(waveform).class("audio-node-waveform-host"),
                Text::new(MI_VOLUME_UP).class("audio-node-volume-icon"),
                volume_slider,
                DecoratedBox::new().child(timecode).class("audio-node-slot-text"),
            ]
    };

    // Animator overlay'ится поверх body — Stack даёт ему 0×0, не влияя на
    // layout. Body-host с MSS height фиксирует высоту body точно.
    let body = syngui::widgets::Stack::new().children(vec![
        Box::new(row) as Box<dyn Widget>,
        Box::new(progress_animator) as Box<dyn Widget>,
    ]);
    Box::new(
        DecoratedBox::new()
            .child(body)
            .class("audio-node-body-host audio-player-host"),
    )
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}


fn toggle_play_pause(
    runtime: &Arc<Mutex<NodeRuntime>>,
    is_playing: RwSignal<bool>,
    is_paused: RwSignal<bool>,
    pcm_view: RwSignal<Option<Arc<syngui::audio::AudioBuffer>>>,
    progress_sig: RwSignal<f32>,
    volume_sig: RwSignal<f32>,
) {
    let mut g = match runtime.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let NodeRuntime::AudioPlayer {
        player,
        pending_stream,
        stream_sample_rate,
        is_streaming,
        ..
    } = &mut *g
    else {
        return;
    };

    // Существующий плеер — pause/resume.
    if let Some(p) = player.as_ref() {
        if p.is_paused() {
            p.resume();
            is_paused.set(false);
            is_playing.set(true);
        } else {
            p.pause();
            is_paused.set(true);
        }
        return;
    }

    // Streaming-источник: забираем receiver, стартуем live-плеер.
    if is_streaming.get_untracked() {
        let rx = match pending_stream.lock() {
            Ok(mut p) => p.take(),
            Err(_) => None,
        };
        let sr = *stream_sample_rate;
        let Some(rx) = rx else {
            eprintln!("[AudioPlayer node] streaming: receiver уже забран или None");
            return;
        };
        if sr == 0 {
            eprintln!("[AudioPlayer node] streaming: sample_rate=0, источник не готов");
            return;
        }
        match AudioPlayer::start_streaming(rx, sr) {
            Ok(p) => {
                p.set_volume(volume_sig.get_untracked());
                *player = Some(p);
                is_playing.set(true);
                is_paused.set(false);
                progress_sig.set(0.0);
            }
            Err(e) => {
                eprintln!("[AudioPlayer node] start_streaming failed: {e}");
            }
        }
        return;
    }

    // Batch-источник (Audio buffer).
    let Some(buf) = pcm_view.get_untracked() else {
        return;
    };
    let pcm = buf.pcm.clone();
    let sr = buf.sample_rate;
    let new_player = if buf.channels >= 2 {
        AudioPlayer::start_stereo(pcm, sr)
    } else {
        AudioPlayer::start(pcm, sr)
    };
    match new_player {
        Ok(p) => {
            p.set_volume(volume_sig.get_untracked());
            *player = Some(p);
            is_playing.set(true);
            is_paused.set(false);
            progress_sig.set(0.0);
        }
        Err(e) => {
            eprintln!("[AudioPlayer node] start failed: {e}");
        }
    }
}

fn stop_player(
    runtime: &Arc<Mutex<NodeRuntime>>,
    is_playing: RwSignal<bool>,
    is_paused: RwSignal<bool>,
    progress_sig: RwSignal<f32>,
) {
    let mut g = match runtime.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let NodeRuntime::AudioPlayer { player, .. } = &mut *g {
        if let Some(p) = player.take() {
            p.stop();
        }
    }
    drop(g);
    is_playing.set(false);
    is_paused.set(false);
    progress_sig.set(0.0);
}

// ──────────────────────────────────────────────────────────────────────────
// ProgressAnimator: невидимый widget, тикает progress из живого player.
// ──────────────────────────────────────────────────────────────────────────

use std::any::Any;
use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, UpdateContext};

pub struct ProgressAnimator {
    runtime: Arc<Mutex<NodeRuntime>>,
    is_playing: RwSignal<bool>,
    is_paused: RwSignal<bool>,
    progress_sig: RwSignal<f32>,
}

impl ProgressAnimator {
    pub fn new(
        runtime: Arc<Mutex<NodeRuntime>>,
        is_playing: RwSignal<bool>,
        is_paused: RwSignal<bool>,
        progress_sig: RwSignal<f32>,
    ) -> Self {
        Self { runtime, is_playing, is_paused, progress_sig }
    }
}

impl Widget for ProgressAnimator {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ProgressAnimatorElement {
            id: ElementId::new(),
            runtime: self.runtime.clone(),
            is_playing: self.is_playing,
            is_paused: self.is_paused,
            progress_sig: self.progress_sig,
            bounds: Rect::zero(),
            classes: Vec::new(),
            dirty: DirtyFlags::LAYOUT,
            mss: MssFields::new(),
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool { other.is::<Self>() }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn mount(&self, _t: &mut ElementTree, _p: ElementId) {}
}

struct ProgressAnimatorElement {
    id: ElementId,
    runtime: Arc<Mutex<NodeRuntime>>,
    is_playing: RwSignal<bool>,
    is_paused: RwSignal<bool>,
    progress_sig: RwSignal<f32>,
    bounds: Rect,
    classes: Vec<String>,
    dirty: DirtyFlags,
    mss: MssFields,
}

impl Element for ProgressAnimatorElement {
    fn update(&mut self, w: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(a) = w.as_any().downcast_ref::<ProgressAnimator>() {
            self.runtime = a.runtime.clone();
            self.is_playing = a.is_playing;
            self.is_paused = a.is_paused;
            self.progress_sig = a.progress_sig;
        }
    }
    fn layout(&mut self, _c: Constraints) -> Size {
        self.bounds = Rect::new(self.bounds.origin, Size::new(0.0, 0.0));
        Size::new(0.0, 0.0)
    }
    fn build_display_list(&self, _l: &mut DisplayList, _c: Rect) {}
    fn handle_event(&mut self, _e: &Event, _c: &mut EventContext) -> EventResult { EventResult::Ignored }
    fn animate(&mut self, _dt: std::time::Duration) -> bool {
        if !self.is_playing.get_untracked() || self.is_paused.get_untracked() {
            return false;
        }
        let Ok(g) = self.runtime.lock() else { return false; };
        let NodeRuntime::AudioPlayer { player, .. } = &*g else { return false; };
        let Some(p) = player.as_ref() else { return false; };
        if !p.is_ready() {
            return true;
        }
        let pos = p.position();
        let prev = self.progress_sig.get_untracked();
        if (pos - prev).abs() > 1.0 / 1000.0 {
            self.progress_sig.set(pos);
        }
        if p.is_done() {
            self.is_playing.set(false);
            self.progress_sig.set(1.0);
            return false;
        }
        true
    }
    fn children(&self) -> &[ElementId] { &[] }
    fn bounds(&self) -> Rect { self.bounds }
    fn set_position(&mut self, p: Point) { self.bounds.origin = p; }
    fn mark_dirty(&mut self, f: DirtyFlags) { self.dirty |= f; }
    fn clear_dirty(&mut self, f: DirtyFlags) { self.dirty.remove(f); }
    fn is_dirty(&self, f: DirtyFlags) -> bool { self.dirty.contains(f) }
    fn id(&self) -> ElementId { self.id }
    fn set_id(&mut self, i: ElementId) { self.id = i; }
    fn mount(&mut self, _t: &mut ElementTree) {}
    fn element_type_name(&self) -> &str { "ProgressAnimator" }
    fn set_classes(&mut self, c: Vec<String>) { self.classes = c; }
    fn get_classes(&self) -> &[String] { &self.classes }
    fn reset_mss_styles(&mut self) { self.mss.reset(); }
    fn mss(&self) -> Option<&MssFields> { Some(&self.mss) }
    fn apply_computed_style(&mut self, s: &ComputedStyle) { self.mss.apply(s); }
    fn passthrough_hit_test(&self) -> bool { true }
}
