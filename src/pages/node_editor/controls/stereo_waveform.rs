use std::sync::Arc;

use syngui::audio::AudioBuffer;
use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::visual::Canvas;

const DEFAULT_BINS: usize = 256;
const DEFAULT_HEIGHT: f32 = 96.0;
const LANE_GAP: f32 = 6.0;
const MIN_BAR: f32 = 1.0;
const FALLBACK_COLOR: &str = "#E0A458";
const CURSOR_COLOR: &str = "#F2F2F2";

pub fn rms_bins_channel(pcm: &[f32], channels: u16, ch_index: usize, bins: usize) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if pcm.is_empty() || bins == 0 || ch_index >= ch {
        return vec![0.0; bins.max(1)];
    }
    let frames = pcm.len() / ch;
    if frames == 0 {
        return vec![0.0; bins];
    }
    let chunk = (frames / bins.max(1)).max(1);
    let mut out = Vec::with_capacity(bins);
    for i in 0..bins {
        let start = i * chunk;
        let end = (start + chunk).min(frames);
        if start >= end {
            out.push(0.0);
            continue;
        }
        let mut sum = 0.0_f64;
        for f in start..end {
            let s = pcm.get(f * ch + ch_index).copied().unwrap_or(0.0) as f64;
            sum += s * s;
        }
        out.push(((sum / (end - start) as f64).sqrt()).clamp(0.0, 1.0) as f32);
    }
    out
}

pub fn stereo_lanes(buf: &AudioBuffer, bins: usize) -> Vec<Vec<f32>> {
    let ch = buf.channels.max(1) as usize;
    let mut lanes: Vec<Vec<f32>> = (0..ch.min(2))
        .map(|c| rms_bins_channel(&buf.pcm, buf.channels, c, bins))
        .collect();
    if lanes.len() == 1 {
        let mono = lanes[0].clone();
        lanes.push(mono);
    }
    let peak = lanes.iter().flat_map(|l| l.iter()).fold(0.0_f32, |a, &b| a.max(b));
    if peak > f32::EPSILON {
        for lane in &mut lanes {
            for v in lane.iter_mut() {
                *v /= peak;
            }
        }
    }
    lanes
}

pub struct StereoWaveform {
    pcm: Option<Arc<AudioBuffer>>,
    progress: Option<f32>,
    bins: usize,
    height: f32,
    color: Option<Color>,
    right_color: Option<Color>,
}

impl Default for StereoWaveform {
    fn default() -> Self {
        Self::new()
    }
}

impl StereoWaveform {
    pub fn new() -> Self {
        Self {
            pcm: None,
            progress: None,
            bins: DEFAULT_BINS,
            height: DEFAULT_HEIGHT,
            color: None,
            right_color: None,
        }
    }

    pub fn pcm(mut self, buf: impl Into<Option<Arc<AudioBuffer>>>) -> Self {
        self.pcm = buf.into();
        self
    }

    pub fn progress(mut self, p: f32) -> Self {
        self.progress = Some(p.clamp(0.0, 1.0));
        self
    }

    pub fn bins(mut self, n: usize) -> Self {
        self.bins = n.max(1);
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h.max(16.0);
        self
    }

    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    pub fn right_color(mut self, c: Color) -> Self {
        self.right_color = Some(c);
        self
    }

    pub fn into_canvas(self) -> Canvas {
        let bins = self.bins;
        let height = self.height;
        let progress = self.progress;
        let explicit = self.color;
        let explicit_right = self.right_color;
        let lanes: Vec<Vec<f32>> = match &self.pcm {
            Some(buf) => stereo_lanes(buf, bins),
            None => vec![vec![0.0; bins], vec![0.0; bins]],
        };

        Canvas::new(move |ctx, _t| {
            let base = explicit
                .or_else(|| ctx.mss_accent())
                .or_else(|| ctx.mss_color())
                .unwrap_or_else(|| Color::from_hex(FALLBACK_COLOR));
            let right = explicit_right.unwrap_or(Color { a: base.a * 0.65, ..base });

            let w = ctx.width();
            let h = ctx.height();
            let lane_h = ((h - LANE_GAP) * 0.5).max(4.0);
            let n = lanes[0].len().max(1) as f32;
            let bar_w = (w / n).max(1.0);
            let gap = (bar_w * 0.25).min(bar_w - MIN_BAR).max(0.0);
            let inner = (bar_w - gap).max(MIN_BAR);

            for (idx, lane) in lanes.iter().enumerate().take(2) {
                ctx.set_color(if idx == 0 { base } else { right });
                let baseline = if idx == 0 { lane_h } else { lane_h + LANE_GAP };
                for (i, v) in lane.iter().enumerate() {
                    let bh = (v.clamp(0.0, 1.0) * lane_h).max(MIN_BAR);
                    let x = i as f32 * bar_w + gap * 0.5;
                    let y = if idx == 0 { baseline - bh } else { baseline };
                    ctx.fill_rect(x, y, inner, bh);
                }
            }

            if let Some(p) = progress {
                ctx.set_color(Color::from_hex(CURSOR_COLOR));
                ctx.fill_rect((p.clamp(0.0, 1.0) * w).min(w - 1.5), 0.0, 1.5, h);
            }
        })
        .height(height)
    }
}

pub fn node_stereo_waveform(
    buf: Option<Arc<AudioBuffer>>,
    class: &'static str,
) -> Box<dyn Widget> {
    let Some(b) = buf else {
        return Box::new(Column::new());
    };
    let frames = b.pcm.len() / (b.channels.max(1) as usize);
    let secs = frames as f32 / b.sample_rate.max(1) as f32;
    let label = format!(
        "{} кГц · {} · {:.1} с",
        b.sample_rate as f32 / 1000.0,
        if b.channels >= 2 { "стерео" } else { "моно" },
        secs
    );
    let wave: Box<dyn Widget> =
        Box::new(StereoWaveform::new().pcm(b).into_canvas().class(class));
    let caption: Box<dyn Widget> = Box::new(Text::new(label).class("h3-node-info"));
    Box::new(Column::new().gap(2.0).children(vec![wave, caption]))
}
