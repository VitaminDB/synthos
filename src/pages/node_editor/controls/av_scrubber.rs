//! Кадры со звуком в ноде (сохранение H3): общий плеер приложения
//! (`components::video_player`, компактный) и под ним стерео-волна в такт
//! позиции плеера, строка «кадр N/M · fps».

use std::sync::Arc;

use syngui::audio::AudioBuffer;
use syngui::prelude::*;
use syngui::widgets::Column;

use crate::components::video_player::{FramesSource, MediaSource, VideoPlayerView};

use super::stereo_waveform::StereoWaveform;

pub struct AvScrubber {
    frames: Option<Arc<Vec<Arc<syngui::video::VideoFrame>>>>,
    audio: Option<Arc<AudioBuffer>>,
    fps: f32,
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
        let source = FramesSource::shared(&frames, fps, self.audio.clone());
        let duration = source.duration().max(f64::EPSILON);
        // Секунды плеера: по ним идут волна и номер кадра.
        let position = use_signal(0.0_f32);

        let preview: Box<dyn Widget> = Box::new(
            DecoratedBox::new().class("vp-node-preview").child(
                VideoPlayerView::new(source)
                    .compact(true)
                    .position_signal(position)
                    .build(),
            ),
        );

        let audio = self.audio.clone();
        let wave_class = self.wave_class;
        let wave_height = self.wave_height;
        let wave: Box<dyn Widget> = Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let progress = (position.get() as f64 / duration).clamp(0.0, 1.0) as f32;
            match &audio {
                Some(a) => vec![Box::new(
                    StereoWaveform::new()
                        .pcm(a.clone())
                        .progress(progress)
                        .height(wave_height)
                        .into_canvas()
                        .class(wave_class),
                )],
                None => vec![],
            }
        }));

        let info: Box<dyn Widget> = Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let t = position.get() as f64;
            let frame_idx = ((t * fps as f64) as usize).min(total_frames - 1);
            vec![Box::new(
                Text::new(format!(
                    "{} {}/{}  ·  {:.0} fps",
                    tr!("nodes.unit.frame"),
                    frame_idx + 1,
                    total_frames,
                    fps
                ))
                .class("h3-node-info"),
            )]
        }));

        Box::new(Column::new().gap(4.0).children(vec![preview, wave, info]))
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
