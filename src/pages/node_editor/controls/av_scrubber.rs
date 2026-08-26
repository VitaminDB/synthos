use std::sync::Arc;

use syngui::audio::AudioBuffer;
use syngui::prelude::*;
use syngui::widgets::visual::{FramesView, ImageFit};
use syngui::widgets::{Column, Row};

use super::stereo_waveform::StereoWaveform;
use super::timecode::fmt_mmss;
use super::transport::{node_transport_buttons, TransportState};

pub struct AvScrubber {
    frames: Option<Arc<Vec<Arc<syngui::video::VideoFrame>>>>,
    audio: Option<Arc<AudioBuffer>>,
    fps: f32,
    preview_class: &'static str,
    wave_class: &'static str,
    wave_height: f32,
}

impl Default for AvScrubber {
    fn default() -> Self {
        Self::new()
    }
}

impl AvScrubber {
    pub fn new() -> Self {
        Self {
            frames: None,
            audio: None,
            fps: 24.0,
            preview_class: "h3-preview-canvas",
            wave_class: "h3-stereo-wave",
            wave_height: 56.0,
        }
    }

    pub fn frames(mut self, f: Arc<Vec<Arc<syngui::video::VideoFrame>>>, fps: f32) -> Self {
        self.frames = Some(f);
        self.fps = fps.max(1.0);
        self
    }

    pub fn audio(mut self, a: impl Into<Option<Arc<AudioBuffer>>>) -> Self {
        self.audio = a.into();
        self
    }

    pub fn preview_class(mut self, c: &'static str) -> Self {
        self.preview_class = c;
        self
    }

    pub fn wave_class(mut self, c: &'static str) -> Self {
        self.wave_class = c;
        self
    }

    pub fn wave_height(mut self, h: f32) -> Self {
        self.wave_height = h.max(24.0);
        self
    }

    pub fn build(self) -> Box<dyn Widget> {
        let Some(frames) = self.frames else {
            return Box::new(Column::new());
        };
        let total_frames = frames.len().max(1);
        let fps = self.fps;
        let duration = total_frames as f64 / fps as f64;

        let playing = use_signal(false);
        let position = use_signal(0.0_f32);
        let transport = use_signal(TransportState::Idle);

        let preview: Box<dyn Widget> = Box::new(
            FramesView::new(frames, fps)
                .fit(ImageFit::Contain)
                .playing_signal(playing)
                .position_signal(position)
                .loop_playback(true)
                .class(self.preview_class),
        );

        let audio = self.audio.clone();
        let wave_class = self.wave_class;
        let wave_height = self.wave_height;
        let wave: Box<dyn Widget> = Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let p = position.get();
            match &audio {
                Some(a) => vec![Box::new(
                    StereoWaveform::new()
                        .pcm(a.clone())
                        .progress(p)
                        .height(wave_height)
                        .into_canvas()
                        .class(wave_class),
                )],
                None => vec![],
            }
        }));

        let clock: Box<dyn Widget> = Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let p = position.get().clamp(0.0, 1.0);
            let cur = p as f64 * duration;
            let frame_idx = ((p * total_frames as f32) as usize).min(total_frames - 1);
            vec![Box::new(
                Text::new(format!(
                    "{} / {}  ·  {} {}/{}  ·  {:.0} fps",
                    fmt_mmss(cur),
                    fmt_mmss(duration),
                    tr!("nodes.unit.frame"),
                    frame_idx + 1,
                    total_frames,
                    fps
                ))
                .class("h3-node-info"),
            )]
        }));

        let buttons = node_transport_buttons(
            transport,
            move || {
                let now = !playing.get_untracked();
                playing.set(now);
                transport.set(if now { TransportState::Playing } else { TransportState::Paused });
            },
            move || {
                playing.set(false);
                position.set(0.0);
                transport.set(TransportState::Idle);
            },
        );

        let bar: Box<dyn Widget> = Box::new(Row::new().gap(8.0).children(vec![buttons, clock]));
        Box::new(Column::new().gap(4.0).children(vec![preview, wave, bar]))
    }
}

pub fn node_av_scrubber(
    frames: Option<Arc<crate::pages::node_editor::types::LtxFrames>>,
    audio: Option<Arc<AudioBuffer>>,
) -> Box<dyn Widget> {
    match frames {
        Some(f) => AvScrubber::new()
            .frames(f.frames.clone(), f.fps as f32)
            .audio(audio)
            .build(),
        None => Box::new(Column::new()),
    }
}
