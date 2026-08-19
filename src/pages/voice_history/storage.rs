//! Хранение голосовых записей на диске.
//!
//! Структура каталога:
//! ```text
//! ~/.config/synthos/voice/
//!   index.json           # Vec<VoiceRecording>, сортирован по убыванию created_at
//!   {id}.wav             # сырые WAV-байты от AudioRecorder::stop_and_encode_wav
//! ```
//!
//! id = `{:016x}` от `time::unix_nanos()` — стабильный, сортируемый, не требует
//! внешних зависимостей. По образцу `chat::storage`.
//!
//! Все ошибки логируются через `eprintln!` и не панические — UI не должен
//! падать из-за проблем с файловой системой (правило TASK.md).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use syngui::audio::AudioError;
use serde::{Deserialize, Serialize};

use crate::context::AppCtx;

// ─────────────────────────────────────────────────────────────────────────────
// Тип
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceRecording {
    pub id: String,
    /// Unix-секунды (для отображения «N минут назад» / даты).
    pub created_at: u64,
    /// Длительность WAV в миллисекундах.
    pub duration_ms: u32,
    /// Распознанный текст (`voice.accumulated` в момент финального Stop).
    pub transcript: String,
    /// Имя ASR-модели, которой распознавалась запись (для показа в карточке).
    pub model_name: Option<String>,
    /// Sample rate WAV для AudioPlayer::start.
    pub sample_rate: u32,
}

impl Default for VoiceRecording {
    fn default() -> Self {
        Self {
            id: String::new(),
            created_at: 0,
            duration_ms: 0,
            transcript: String::new(),
            model_name: None,
            sample_rate: 16_000,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Пути
// ─────────────────────────────────────────────────────────────────────────────

pub fn voice_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/voice")
}

fn index_path() -> PathBuf {
    voice_dir().join("index.json")
}

fn wav_path(id: &str) -> PathBuf {
    voice_dir().join(format!("{id}.wav"))
}

// ─────────────────────────────────────────────────────────────────────────────
// I/O
// ─────────────────────────────────────────────────────────────────────────────

pub fn load_index() -> Vec<VoiceRecording> {
    let path = index_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("[synthos/voice] Не смог прочитать {:?}: {e}", path);
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<VoiceRecording>>(&text) {
        Ok(mut v) => {
            v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            v
        }
        Err(e) => {
            eprintln!("[synthos/voice] Битый index.json {:?}: {e}", path);
            Vec::new()
        }
    }
}

fn save_index(recs: &[VoiceRecording]) {
    let dir = voice_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[synthos/voice] Не удалось создать {:?}: {e}", dir);
        return;
    }
    let path = index_path();
    match serde_json::to_string_pretty(recs) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                eprintln!("[synthos/voice] Не удалось записать {:?}: {e}", path);
            }
        }
        Err(e) => eprintln!("[synthos/voice] Ошибка сериализации index: {e}"),
    }
}

pub fn delete_recording(id: &str) {
    let wav = wav_path(id);
    if let Err(e) = std::fs::remove_file(&wav) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[synthos/voice] Не удалось удалить {:?}: {e}", wav);
        }
    }
    let mut recs = load_index();
    recs.retain(|r| r.id != id);
    save_index(&recs);
}

/// Декодировать WAV-файл в моно f32-PCM для `AudioPlayer::start`.
/// Стерео сводится в моно усреднением. На любую ошибку — `Err`.
pub fn load_wav_pcm(id: &str) -> Result<(Arc<[f32]>, u32), AudioError> {
    let path = wav_path(id);
    let mut reader = hound::WavReader::open(&path).map_err(|e| {
        AudioError::Wav(format!("WAV open {:?}: {e}", path))
    })?;
    let spec = reader.spec();
    let sr = spec.sample_rate;
    let ch = spec.channels.max(1) as usize;

    let samples_i16: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|r| r.unwrap_or(0))
            .collect(),
        (hound::SampleFormat::Int, _) => reader
            .samples::<i32>()
            .map(|r| {
                let v = r.unwrap_or(0);
                let shift = spec.bits_per_sample as i32 - 16;
                if shift > 0 { (v >> shift) as i16 } else { (v << -shift) as i16 }
            })
            .collect(),
        (hound::SampleFormat::Float, _) => reader
            .samples::<f32>()
            .map(|r| (r.unwrap_or(0.0).clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect(),
    };

    let mono: Vec<f32> = if ch == 1 {
        samples_i16.iter().map(|s| *s as f32 / i16::MAX as f32).collect()
    } else {
        samples_i16
            .chunks(ch)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|s| *s as i32).sum();
                (sum as f32 / ch as f32) / i16::MAX as f32
            })
            .collect()
    };

    Ok((Arc::from(mono.into_boxed_slice()), sr))
}

// ─────────────────────────────────────────────────────────────────────────────
// Запись новой сессии
// ─────────────────────────────────────────────────────────────────────────────

/// Сохраняет голосовую сессию на диск: WAV-файл + добавление в индекс.
///
/// Вызывается из `chat::audio::on_transcription_done` при `final_chunk=true`
/// (sink=VoicePanel). Важно: WAV содержит ТОЛЬКО последний чанк (от Resume
/// до Stop) — старые чанки хранятся отдельными WAV'ами не сохраняются. Это
/// прагматичный компромисс: первый запуск истории будет хранить полный текст
/// (склейка всех чанков) и только последний WAV. В будущем — accumulator
/// рекордера, см. план §14.
///
/// Не паникует на ошибках I/O — логирует и продолжает.
pub fn save_session(app: &AppCtx, transcript: &str, wav_bytes: &[u8]) {
    if transcript.trim().is_empty() {
        return;
    }
    let dir = voice_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[synthos/voice] Не удалось создать {:?}: {e}", dir);
        return;
    }

    let id = generate_id();
    let path = wav_path(&id);
    if let Err(e) = std::fs::write(&path, wav_bytes) {
        eprintln!("[synthos/voice] Не удалось записать {:?}: {e}", path);
        return;
    }

    let (sample_rate, duration_ms) = probe_wav(wav_bytes);
    let model_name = current_model_name(app);

    let rec = VoiceRecording {
        id,
        created_at: now_unix_secs(),
        duration_ms,
        transcript: transcript.to_string(),
        model_name,
        sample_rate,
    };

    let mut recs = load_index();
    recs.insert(0, rec.clone());
    save_index(&recs);

    // Реактивно обновим signal истории — страница «История» сразу подхватит.
    app.voice_history.recordings.update(|v| {
        v.insert(0, rec);
    });
}

/// Извлечь sample_rate и duration_ms из WAV-байтов через минимальный hound-проход.
fn probe_wav(bytes: &[u8]) -> (u32, u32) {
    let cursor = std::io::Cursor::new(bytes);
    let reader = match hound::WavReader::new(cursor) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[synthos/voice] probe_wav: {e}");
            return (16_000, 0);
        }
    };
    let spec = reader.spec();
    let len = reader.duration() as f64; // frames
    let dur_ms = ((len / spec.sample_rate as f64) * 1000.0) as u32;
    (spec.sample_rate, dur_ms)
}

fn current_model_name(app: &AppCtx) -> Option<String> {
    app.audio.asr_loaded_name.get_untracked()
        .or_else(|| app.selected_audio_model.get_untracked())
}

fn generate_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:016x}", nanos as u64)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
