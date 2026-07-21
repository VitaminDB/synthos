//! Универсальный декодер аудиофайлов через `symphonia`. Поддерживает
//! WAV / MP3 / FLAC / OGG / AAC / M4A — общий путь для AudioFile (открытие
//! пользовательского файла) и AudioRecorder (decode WAV-байтов после stop).
//!
//! Возвращает `AudioBuffer` (interleaved f32 PCM, native sample rate,
//! native channels). Никакой ресемплинг — `AudioPlayer` сделает его сам.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use syngui::audio::AudioBuffer;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Декодировать файл с диска в `AudioBuffer`. Расширение используется как
/// hint для symphonia (быстрее выбирает demuxer'а), но не обязательно.
pub fn decode_file(path: &Path) -> std::result::Result<AudioBuffer, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("не удалось открыть файл: {e}"))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    decode_inner(Box::new(file), Some(ext))
}

/// Декодировать байты (например, WAV-payload от `AudioRecorder::stop_and_encode_wav`).
pub fn decode_bytes(bytes: Vec<u8>) -> std::result::Result<AudioBuffer, String> {
    let cursor = Cursor::new(bytes);
    decode_inner(Box::new(cursor), Some("wav".to_string()))
}

fn decode_inner(
    source: Box<dyn MediaSource>,
    ext_hint: Option<String>,
) -> std::result::Result<AudioBuffer, String> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ref ext) = ext_hint {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("не удалось определить формат: {e}"))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "файл не содержит audio-треков".to_string())?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| "неизвестный sample rate".to_string())?;
    let channels_count = codec_params
        .channels
        .ok_or_else(|| "неизвестное число каналов".to_string())?
        .count() as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("codec не поддерживается: {e}"))?;

    let mut interleaved: Vec<f32> = Vec::new();
    // Reserve по estimated n_frames * channels (если известно).
    if let Some(n_frames) = codec_params.n_frames {
        interleaved.reserve(n_frames as usize * channels_count as usize);
    }

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("read packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => append_interleaved(&decoded, &mut interleaved, channels_count as usize),
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                // recoverable: skip пакет
                continue;
            }
            Err(e) => return Err(format!("decode error: {e}")),
        }
    }

    if interleaved.is_empty() {
        return Err("файл декодирован, но не содержит сэмплов".to_string());
    }

    Ok(AudioBuffer::new(
        Arc::from(interleaved.into_boxed_slice()),
        sample_rate,
        channels_count,
    ))
}

/// Скопировать decoded buffer (любой sample type) в interleaved f32-вектор.
/// Symphonia отдаёт planar layout (per-channel), мы интерливим в L0,R0,L1,R1...
fn append_interleaved(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>, channels: usize) {
    let frames = buf.frames();
    out.reserve(frames * channels);
    match buf {
        AudioBufferRef::F32(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push(plane[f]);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push(plane[f] as f32 / i16::MAX as f32);
                }
            }
        }
        AudioBufferRef::S24(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    let s = plane[f].inner();
                    out.push(s as f32 / 8388607.0); // 2^23 - 1
                }
            }
        }
        AudioBufferRef::S32(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push(plane[f] as f32 / i32::MAX as f32);
                }
            }
        }
        AudioBufferRef::F64(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push(plane[f] as f32);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push((plane[f] as f32 - 128.0) / 128.0);
                }
            }
        }
        AudioBufferRef::U16(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push((plane[f] as f32 - 32768.0) / 32768.0);
                }
            }
        }
        AudioBufferRef::U24(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    let s = plane[f].inner() as f32;
                    out.push((s - 8388608.0) / 8388608.0);
                }
            }
        }
        AudioBufferRef::U32(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push((plane[f] as f32 - 2147483648.0) / 2147483648.0);
                }
            }
        }
        AudioBufferRef::S8(b) => {
            for f in 0..frames {
                for c in 0..channels {
                    let plane = b.chan(c.min(b.spec().channels.count() - 1));
                    out.push(plane[f] as f32 / i8::MAX as f32);
                }
            }
        }
    }
}
