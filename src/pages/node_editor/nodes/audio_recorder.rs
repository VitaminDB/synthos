//! Body для NodeKind::AudioRecorder.
//!
//! Layout — горизонтальный, в стилистике Demo Node:
//!   [Mic/Stop btn] [elapsed + status] [waveform — live или preview]
//!
//! Lifecycle полностью на [`syngui::audio::RecordingSession`]:
//! - `session.start_with_device(device)` поднимает cpal и публикует
//!   `vis_handle` + `audio_stream` + фоновый elapsed-таймер.
//! - `session.stop()` декодирует WAV → `AudioBuffer` (потому что в
//!   `RecordingOptions` стоит `decode_on_stop = true`) и кладёт его
//!   в `session.last_result()`.
//!
//! Раньше всё это размазывалось по 8 сигналам в NodeRuntime плюс
//! отдельный `ElapsedAnimator`-Element ради тика elapsed. Теперь —
//! одна сессия наружу.

use syngui::audio::RecordingState;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::visual::audio_waveform::AudioWaveform;
use syngui::widgets::visual::StaticWaveform;
use syngui::widgets::{Column, DecoratedBox, Reactive, Row, ToolButton};

use crate::icons::{MI_MIC, MI_STOP};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime, PortValue};

/// Executor для AudioRecorder.
///
/// Семантика output-port'а `out`:
/// - `session` в Recording/Paused и стрим открыт → `PortValue::AudioStream`;
/// - иначе если в `last_result` есть декодированный буфер → `PortValue::Audio`;
/// - иначе → `Empty`.
pub struct AudioRecorderExec;

impl NodeExecutor for AudioRecorderExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AudioRecorder { session, .. } => {
                    let state_sig = session.state();
                    let state = if track { state_sig.get() } else { state_sig.get_untracked() };
                    let active = matches!(state, RecordingState::Recording | RecordingState::Paused);
                    if active {
                        let stream_sig = session.audio_stream();
                        let s = if track { stream_sig.get() } else { stream_sig.get_untracked() };
                        s.map(PortValue::AudioStream).unwrap_or(PortValue::Empty)
                    } else {
                        let last_sig = session.last_result();
                        let last = if track { last_sig.get() } else { last_sig.get_untracked() };
                        last.and_then(|r| r.audio_buffer.clone())
                            .map(PortValue::Audio)
                            .unwrap_or(PortValue::Empty)
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("out", pv);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (session, device) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AudioRecorder { session, device } => (session.clone(), *device),
            _ => return error_widget("AudioRecorder: некорректный runtime"),
        },
        Err(_) => return error_widget("AudioRecorder: lock error"),
    };

    let state_sig = session.state();
    let vis_sig = session.vis_handle();
    let elapsed_sig = session.elapsed_secs();
    let error_sig = session.error();
    let last_sig = session.last_result();

    let toggle_session = session.clone();
    let toggle_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let state = state_sig.get();
        let active = matches!(state, RecordingState::Recording | RecordingState::Paused);
        let (icon, tooltip, class) = if active {
            (
                MI_STOP,
                "Остановить запись",
                "audio-node-transport-btn audio-node-record-active",
            )
        } else {
            (
                MI_MIC,
                "Начать запись",
                "audio-node-transport-btn audio-node-record",
            )
        };
        let s_click = toggle_session.clone();
        let btn = ToolButton::new(icon)
            .tooltip(tooltip)
            .on_click(move || {
                let state_now = s_click.state().get_untracked();
                if matches!(
                    state_now,
                    RecordingState::Recording | RecordingState::Paused
                ) {
                    let _ = s_click.stop();
                } else {
                    let dev = device.get_untracked();
                    let _ = s_click.start_with_device(dev.as_deref());
                }
            })
            .class(class);
        vec![Box::new(btn)]
    });

    let elapsed_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let secs = elapsed_sig.get();
        vec![Box::new(Text::new(fmt_mmss(secs)).class("audio-node-timecode")) as Box<dyn Widget>]
    });

    let status_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(err) = error_sig.get() {
            return vec![
                Box::new(Text::new(format!("Ошибка: {err}")).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        let (label, class) = match state_sig.get() {
            RecordingState::Recording => (
                "Идёт запись…".to_string(),
                "audio-node-status audio-node-status-active",
            ),
            RecordingState::Paused => (
                "Пауза".to_string(),
                "audio-node-status audio-node-status-active",
            ),
            RecordingState::Completed => ("Запись готова".to_string(), "audio-node-status"),
            RecordingState::Failed => ("Ошибка".to_string(), "audio-node-status"),
            RecordingState::Idle => ("Готов к записи".to_string(), "audio-node-status"),
        };
        vec![Box::new(Text::new(label).class(class)) as Box<dyn Widget>]
    });

    let waveform_area = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let active = matches!(
            state_sig.get(),
            RecordingState::Recording | RecordingState::Paused
        );
        if active {
            if let Some(handle) = vis_sig.get() {
                return vec![Box::new(
                    AudioWaveform::new(handle)
                        .bars(64)
                        .height(40.0)
                        .class("audio-node-waveform-live"),
                )];
            }
            return vec![Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(
                            Text::new("Инициализация микрофона…")
                                .class("audio-node-waveform-placeholder"),
                        ),
                    )
                    .class("audio-node-waveform-empty"),
            )];
        }
        let buf = last_sig.get().and_then(|r| r.audio_buffer);
        let widget: Box<dyn Widget> = match buf {
            Some(b) => Box::new(
                StaticWaveform::new()
                    .pcm(Some(b))
                    .height(40.0)
                    .class("audio-node-waveform"),
            ),
            None => Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(
                            Text::new("Нет записи").class("audio-node-waveform-placeholder"),
                        ),
                    )
                    .class("audio-node-waveform-empty"),
            ),
        };
        vec![widget]
    });

    let info_col = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(elapsed_label)
        .child(status_label);

    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().child(toggle_btn).class("audio-node-slot-btn"),
                DecoratedBox::new().child(info_col).class("audio-node-info"),
                DecoratedBox::new().child(waveform_area).class("audio-node-waveform-host"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-recorder-host"),
    )
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}

fn fmt_mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let cs = ((secs - s as f64) * 100.0) as u64;
    format!("{:02}:{:02}.{:02}", s / 60, s % 60, cs)
}
