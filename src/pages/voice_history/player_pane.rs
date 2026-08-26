//! Правая панель — плеер выбранной записи.
//!
//! Показывает заголовок (дата/модель), полный текст распознавания, контролы
//! Play/Stop и прогресс. AudioPlayer создаётся при клике Play и хранится в
//! локальном `RwSignal<Option<Arc<Mutex<AudioPlayer>>>>`. Прогресс — Canvas
//! с `animated(true)`, опрашивает `player.position()` каждый кадр.

use std::sync::Arc;

use syngui::audio::AudioPlayer;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::signal::use_signal;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::ProgressBar;

use crate::icons::{MI_PLAY_ARROW, MI_STOP};

use super::storage::{self, VoiceRecording};

/// Ручка плеера: bool-сигнал «играет ли» + Arc<Mutex<Option<AudioPlayer>>>.
/// Сигнал в Mutex'е никак не сравнивается через PartialEq (AudioPlayer не Eq) —
/// поэтому держим отдельно: bool для UI, Arc для самого плеера.
#[derive(Clone)]
struct PlayerHandle {
    is_playing: RwSignal<bool>,
    inner: Arc<Mutex<Option<AudioPlayer>>>,
}

pub fn view(rec: VoiceRecording) -> impl Widget {
    let handle = PlayerHandle {
        is_playing: use_signal(false),
        inner: Arc::new(Mutex::new(None)),
    };

    let header = make_header(&rec);
    let transcript = make_transcript(&rec);
    let controls = make_controls(rec.clone(), handle.clone());
    let progress = make_progress(handle.is_playing);

    mgui! {
        DecoratedBox::new().class("voice-history-player") => [
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header,
                transcript,
                progress,
                controls
            ]
        ]
    }
}

fn make_header(rec: &VoiceRecording) -> impl Widget {
    let model = rec
        .model_name
        .clone()
        .unwrap_or_else(|| tr!("voice.history.player.no_model"));
    let date_label = format_date(rec.created_at);
    let dur = format_duration(rec.duration_ms);

    mgui! {
        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Text::new(date_label).class("voice-history-player-title"),
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(model).class("voice-history-player-model"),
                Text::new("·").class("voice-history-player-meta-sep"),
                Text::new(dur).class("voice-history-player-meta")
            ]
        ]
    }
}

fn make_transcript(rec: &VoiceRecording) -> impl Widget {
    DecoratedBox::new()
        .class("voice-history-player-transcript")
        .child(Text::new(rec.transcript.clone()).class("voice-history-player-text"))
}

fn make_controls(
    rec: VoiceRecording,
    handle: PlayerHandle,
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let is_playing = handle.is_playing.get();

        let inner = if is_playing {
            let h = handle.clone();
            mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    ToolButton::new(MI_STOP)
                        .tooltip(tr!("voice.history.player.stop_tooltip"))
                        .on_click(move || {
                            // Drop AudioPlayer внутри Mutex — освобождает stream.
                            if let Ok(mut g) = h.inner.lock() { *g = None; }
                            h.is_playing.set(false);
                        })
                        .class("voice-history-player-btn voice-history-player-stop")
                ]
            }
        } else {
            let rec_for_play = rec.clone();
            let h = handle.clone();
            mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    ToolButton::new(MI_PLAY_ARROW)
                        .tooltip(tr!("voice.history.player.play_tooltip"))
                        .on_click(move || {
                            start_playback(&rec_for_play, &h);
                        })
                        .class("voice-history-player-btn voice-history-player-play")
                ]
            }
        };
        DecoratedBox::new()
            .class("voice-history-player-controls")
            .child(inner)
    }
}

/// Прогресс-бар. Реактивен через `Canvas.animated(true)` — мы не делаем
/// отдельный signal-tick, чтобы не плодить request_redraw из background.
/// Используем Canvas внутри `.child(closure)` который пересобирает виджет
/// раз в кадр через подписку на `player` сигнал… Нет, лучше — ProgressBar
/// + closure-driver, читающий position() через own animation tick.
///
/// Для простоты MVP делаем «реактив на pause/play»: пока is_playing=true,
/// ProgressBar показывает indeterminate; после Stop — полный (1.0). В
/// следующем sprint можно подключить Canvas-driven прогресс по семплам.
fn make_progress(
    is_playing: RwSignal<bool>,
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let bar = if is_playing.get() {
            ProgressBar::new().indeterminate()
        } else {
            ProgressBar::new().value(0.0)
        };
        DecoratedBox::new()
            .class("voice-history-player-progress")
            .child(bar)
    }
}

fn start_playback(rec: &VoiceRecording, handle: &PlayerHandle) {
    let id = rec.id.clone();
    match storage::load_wav_pcm(&id) {
        Ok((pcm, sr)) => match AudioPlayer::start(pcm, sr) {
            Ok(p) => {
                if let Ok(mut g) = handle.inner.lock() {
                    *g = Some(p);
                }
                handle.is_playing.set(true);
            }
            Err(e) => {
                eprintln!("[synthos/voice] Не удалось запустить плеер: {e}");
            }
        },
        Err(e) => {
            eprintln!("[synthos/voice] Не удалось декодировать WAV {id}: {e}");
        }
    }
}

fn format_duration(ms: u32) -> String {
    let total = ms / 1000;
    let m = total / 60;
    let s = total % 60;
    if m > 0 {
        tr!(
            "voice.history.item.duration.min_sec",
            min = m.to_string(),
            sec = format!("{s:02}")
        )
    } else {
        tr!("voice.history.item.duration.sec", sec = s.to_string())
    }
}

fn format_date(unix_secs: u64) -> String {
    // Простая UTC-метка — без chrono, см. list_item.rs.
    if unix_secs == 0 {
        return String::new();
    }
    let secs_in_day = unix_secs % 86_400;
    let h = (secs_in_day / 3600) as u32;
    let m = ((secs_in_day % 3600) / 60) as u32;
    let days = unix_secs / 86_400;
    tr!(
        "voice.history.player.date",
        days = days.to_string(),
        time = format!("{h:02}:{m:02}")
    )
}
