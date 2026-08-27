//! Тонкая прослойка между [`synaptix::facade::asr::stream::StreamingAsr`] и UI.
//!
//! 1. `start(stream, actx, language)` — забирает `Receiver<Vec<f32>>` из
//!    [`syngui::audio::AudioStream`], запускает [`StreamingAsr`] и отдельный
//!    drainer-thread, который слушает `events_rx`.
//! 2. Drainer на каждое событие делает `run_on_main_thread` и применяет:
//!    - [`StreamingAsrEvent::Delta`] → конкатенация в `actx.live_text`.
//!    - [`StreamingAsrEvent::Final`] → перезапись `actx.live_text` финалом
//!      + (если есть pending) вызов [`crate::agent::audio::on_transcription_done`].
//!    - [`StreamingAsrEvent::Error`] → выставить ошибку в `session.error`.
//! 3. Когда `RecordingSession::stop` отрабатывает, Sender PCM-канала дропается
//!    → ASR-loop завершается → events_rx видит Disconnect → drainer выходит,
//!    Drop у `StreamingAsr` ждёт join рабочего треда.

use std::sync::Arc;
use std::thread;

use synaptix::facade::asr::stream::{StreamingAsr, StreamingAsrConfig, StreamingAsrEvent};
use syngui::async_runtime::run_on_main_thread;
use syngui::audio::AudioStream;

use super::audio::{on_transcription_done, AudioCtx};

/// Запустить streaming-ASR pipeline для свежеоткрытого live PCM-стрима записи.
///
/// Если `stream.take_receiver()` уже использован кем-то другим (single-sub
/// контракт `AudioStream`) — функция тихо ничего не делает: streaming-ASR
/// нельзя стартовать без эксклюзивного receiver'а.
pub fn start(stream: Arc<AudioStream>, actx: AudioCtx, language: Option<String>) {
    let Some(rx) = stream.take_receiver() else {
        eprintln!("[synthos/streaming_asr] receiver уже занят — пропускаем старт");
        return;
    };
    let sr = stream.sample_rate;
    let cfg = StreamingAsrConfig {
        language,
        ..StreamingAsrConfig::default()
    };
    let handle = StreamingAsr::start(rx, sr, actx.asr.clone(), cfg);

    let actx_drain = actx.clone();
    let spawn_result = thread::Builder::new()
        .name("synthos-streaming-asr-drainer".into())
        .spawn(move || {
            // Drainer владеет всем StreamingAsr — когда внутренний ASR-loop
            // завершится (через Final или Disconnect), цикл while-let выйдет,
            // handle дропнется, его Drop сделает join рабочего треда.
            while let Ok(ev) = handle.events_rx.recv() {
                let actx_for_ev = actx_drain.clone();
                run_on_main_thread(move || apply_event(actx_for_ev, ev));
            }
        });

    if let Err(e) = spawn_result {
        eprintln!("[synthos/streaming_asr] spawn drainer: {e}");
        // PCM-receiver уже забрали — без drainer'а данные просто упадут в
        // никуда. Корректнее всего: выставить ошибку в session.error,
        // пользователь увидит и нажмёт стоп.
        actx.session.error().set(Some(format!(
            "streaming-ASR: не удалось запустить drainer: {e}"
        )));
    }
}

fn apply_event(actx: AudioCtx, ev: StreamingAsrEvent) {
    match ev {
        StreamingAsrEvent::Delta { text, .. } => {
            // Whisper отдаёт дельту относительно прошлой полной транскрипции
            // окна — мы просто аппендим суффикс к live_text. Trim чтобы не
            // плодить ведущие/хвостовые пробелы; разделитель — одиночный пробел.
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                return;
            }
            actx.live_text.update(|s| {
                if !s.is_empty() && !s.ends_with(' ') {
                    s.push(' ');
                }
                s.push_str(&trimmed);
            });
        }
        StreamingAsrEvent::Final { text } => {
            // Финальная транскрипция полной записи. Может отличаться от
            // суммы дельт (модель видит целое) — перезаписываем live_text
            // вместо аппенда, это «авторитетный» результат.
            actx.live_text.set(text.clone());
            actx.transcribing.set(false);
            if let Some(pending) = actx
                .streaming_pending
                .lock()
                .ok()
                .and_then(|mut g| g.take())
            {
                on_transcription_done(actx, Ok(text), pending.sink);
            }
        }
        StreamingAsrEvent::Error(msg) => {
            actx.session.error().set(Some(msg.clone()));
            // Pending-finalize отменяем — без текста сэмплируем classic
            // fallback через тот же on_transcription_done (он обработает Err).
            if let Some(pending) = actx
                .streaming_pending
                .lock()
                .ok()
                .and_then(|mut g| g.take())
            {
                actx.transcribing.set(false);
                on_transcription_done(actx, Err(msg), pending.sink);
            }
        }
    }
}
