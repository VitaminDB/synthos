//! Потоковое распознавание речи ([`StreamingAsr`]) и его связь с UI.
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
    // Распознаёт служба: модель открыта в `actx.asr`.
    let asr = actx.asr.clone();
    let run: TranscribeFn = Arc::new(move |buf, sr, lang| {
        let guard = asr.lock().map_err(|_| "asr mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(t) => t.transcribe_pcm_text(buf, sr, lang),
            None => Err(syngui::tr!("voice.error.model_not_loaded")),
        }
    });
    let handle = StreamingAsr::start_with(rx, sr, run, cfg);

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

// ─────────────────────── Потоковое распознавание ───────────────────────

/// Распознавание целого буфера: (PCM, частота, язык) → текст. Исполнитель —
/// служба (модель открыта в `AudioCtx::asr`).
pub type TranscribeFn = Arc<dyn Fn(&[f32], u32, Option<&str>) -> Result<String, String> + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct StreamingAsrConfig {
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StreamingAsrEvent {
    Delta { text: String, partial: bool },
    Final { text: String },
    Error(String),
}

/// Рабочий поток буферизует PCM-куски, раз в ~[`CADENCE_S`] с распознаёт весь
/// буфер заново и шлёт `Delta` с новым хвостом (разница по словам). Канал PCM
/// закрылся — финальное распознавание → `Final`.
pub struct StreamingAsr {
    pub events_rx: std::sync::mpsc::Receiver<StreamingAsrEvent>,
}

/// Накопленного аудио (с) между распознаваниями живого превью.
const CADENCE_S: f32 = 1.0;
/// Минимум аудио (с) для первой попытки (короче — шум или пусто).
const MIN_S: f32 = 0.4;

/// Слова `new` за общим префиксом со `prev` (инкремент для дописывания).
fn suffix_after_common_prefix(prev: &[&str], new: &[&str]) -> String {
    let mut i = 0;
    while i < prev.len() && i < new.len() && prev[i] == new[i] {
        i += 1;
    }
    new[i..].join(" ")
}

impl StreamingAsr {
    pub fn start_with(pcm_rx: std::sync::mpsc::Receiver<Vec<f32>>, sample_rate: u32, transcribe: TranscribeFn, cfg: StreamingAsrConfig) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        thread::Builder::new()
            .name("syn-streaming-asr".into())
            .spawn(move || {
                let sr = sample_rate;
                let cadence = (CADENCE_S * sr as f32) as usize;
                let min_samples = (MIN_S * sr as f32) as usize;
                let lang = cfg.language.as_deref();
                let mut buf: Vec<f32> = Vec::new();
                let mut last_len = 0usize;
                let mut prev_text = String::new();
                let run = |buf: &[f32]| -> Result<String, String> { transcribe(buf, sr, lang) };
                loop {
                    match pcm_rx.recv() {
                        Ok(chunk) => {
                            buf.extend_from_slice(&chunk);
                            if buf.len() >= min_samples && buf.len() - last_len >= cadence {
                                match run(&buf) {
                                    Ok(full) => {
                                        let prev_w: Vec<&str> = prev_text.split_whitespace().collect();
                                        let new_w: Vec<&str> = full.split_whitespace().collect();
                                        let delta = suffix_after_common_prefix(&prev_w, &new_w);
                                        if !delta.trim().is_empty() {
                                            let _ = tx.send(StreamingAsrEvent::Delta { text: delta, partial: true });
                                        }
                                        prev_text = full;
                                        last_len = buf.len();
                                    }
                                    Err(e) => {
                                        let _ = tx.send(StreamingAsrEvent::Error(e));
                                        return;
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            if buf.is_empty() {
                                let _ = tx.send(StreamingAsrEvent::Final { text: String::new() });
                                return;
                            }
                            let _ = tx.send(match run(&buf) {
                                Ok(full) => StreamingAsrEvent::Final { text: full },
                                Err(e) => StreamingAsrEvent::Error(e),
                            });
                            return;
                        }
                    }
                }
            })
            .ok();
        Self { events_rx: rx }
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn delta_is_suffix_after_common_words() {
        assert_eq!(suffix_after_common_prefix(&["привет", "мир"], &["привет", "мир", "как", "дела"]), "как дела");
        assert_eq!(suffix_after_common_prefix(&[], &["раз"]), "раз");
    }

    #[test]
    fn final_after_channel_closes() {
        let (tx, rx) = std::sync::mpsc::channel();
        let run: TranscribeFn = Arc::new(|buf, _, _| Ok(format!("{} отсчётов", buf.len())));
        let asr = StreamingAsr::start_with(rx, 10, run, StreamingAsrConfig::default());
        tx.send(vec![0.0; 25]).unwrap();
        drop(tx);
        let evs: Vec<_> = asr.events_rx.iter().collect();
        assert!(matches!(evs.last(), Some(StreamingAsrEvent::Final { text }) if text == "25 отсчётов"), "{evs:?}");
    }
}
