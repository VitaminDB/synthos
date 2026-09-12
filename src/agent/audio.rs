//! Подсистема голосового ввода: запись с микрофона через
//! [`syngui::audio::RecordingSession`] + локальная транскрипция через
//! [`synaptix::facade::asr::Transcriber`] (synaptix, без llama-server).
//!
//! ## UI-flow
//!
//! 1. Клик MI_MIC на input-panel → [`toggle_recording`].
//! 2. Если запись не идёт — стартует [`RecordingSession`], публикует
//!    `vis_handle` для waveform-виджета, поднимает `state=Recording`.
//! 3. Повторный клик → [`RecordingSession::stop`] → WAV-байты, далее
//!    `tokio::task::spawn_blocking(transcribe_wav)` → текст в `ChatCtx.input`
//!    через [`syngui::async_runtime::run_on_main_thread`].
//!
//! Загрузка модели (`Transcriber::load`) — отдельная блокирующая операция;
//! пользователь явно нажимает «Загрузить модель» в Settings → Аудио модели.
//! Если модель не загружена — клик MI_MIC показывает ошибку через
//! `session.error()`.
//!
//! ## Backward-compat: `is_recording` / `vis_handle` / `error`
//!
//! Существующие callsite'ы (input_panel, voice_fab/*) читают
//! `app.audio.is_recording.get()` и пр. Эти поля сохранены и обновляются
//! через `create_effect` из `session.state() / session.vis_handle() /
//! session.error()` — переход на `RecordingSession` для них прозрачен.
//! **Запись** в `audio.error.set(...)` извне больше не работает (эффект
//! перепишет): для очистки ошибки используйте `audio.session.error().set(None)`.

use syngui::tr;
use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::audio::{RecordingOptions, RecordingSession, RecordingState, VisHandle};
use syngui::context_provider::use_context;
use syngui::core::sync::Mutex;
use syngui::signal::{create_effect, use_signal, RwSignal};

use synaptix::facade::asr::{AsrConfig, AsrModelKind, Transcriber};

use crate::config::{AsrEngineKind, AudioModelConfig};
use crate::context::AppCtx;

/// Куда отправлять распознанный текст после транскрипции.
///
/// Один и тот же `stop_and_send_with_sink` обрабатывает оба сценария:
/// классический mic-toggle на input panel чата (вставка в `chat.input`)
/// и новую глобальную панель распознавания (накопление в `voice.accumulated`).
/// Поведение разделено только в `on_transcription_done`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptSink {
    /// Старый mic-toggle: после транскрипции текст добавляется в `chat.input`
    /// через пробел и бамп `input_gen` (визуальный re-render редактора).
    ChatInputAppend,
    /// Глобальная FAB-панель: текст уходит в `voice.accumulated`. Если
    /// `final_chunk=true` (финальный Stop, а не Pause) — поднимаем
    /// `voice.awaiting_actions=true`, чтобы UI переключил кнопки на
    /// Copy/Paste/Restart/Close.
    VoicePanel { final_chunk: bool },
}

/// Заявка на финализацию streaming-ASR. Выставляется в
/// `stop_and_send_with_sink` перед остановкой сессии. Дренер `streaming_asr.rs`
/// видит её при получении `StreamingAsrEvent::Final` (или `Error`) и
/// вызывает [`on_transcription_done`].
#[derive(Clone)]
pub struct PendingFinalize {
    pub sink: TranscriptSink,
}

/// Глобальный аудио-контекст: единая `RecordingSession` + ASR-модель +
/// зеркала session-сигналов для существующих callsite'ов.
#[derive(Clone)]
pub struct AudioCtx {
    /// Единый lifecycle записи (см. [`syngui::audio::RecordingSession`]).
    pub session: RecordingSession,
    /// Идёт ли транскрипция (после стопа записи) — отдельный сигнал,
    /// потому что transcribe blocking и может занимать секунды.
    pub transcribing: RwSignal<bool>,
    /// Загруженная ASR-модель. `None` пока пользователь не нажал
    /// «Загрузить модель» в Settings → Аудио модели.
    pub asr: Arc<Mutex<Option<Transcriber>>>,
    /// Имя текущей загруженной модели — для статус-чипа в UI настроек.
    pub asr_loaded_name: RwSignal<Option<String>>,
    /// Идёт ли загрузка модели (Transcriber::load в spawn_blocking).
    pub asr_loading: RwSignal<bool>,
    /// Микрофоны системы. `None` — опрос ещё не закончился: он идёт в
    /// фоне, потому что `cpal`/ALSA перебирает устройства около секунды и
    /// в сборке виджета вешал бы весь интерфейс.
    pub input_devices: RwSignal<Option<Vec<String>>>,

    /// **Mirror** `session.state()` → True при `RecordingState::Recording`.
    /// Backward-compat для `input_panel`, `voice_fab/*`. Обновляется
    /// `create_effect`'ом из `session.state()`.
    pub is_recording: RwSignal<bool>,
    /// **Mirror** `session.vis_handle()`. Backward-compat.
    pub vis_handle: RwSignal<Option<VisHandle>>,
    /// **Mirror** `session.error()`. Backward-compat. Запись в это поле
    /// извне неэффективна (эффект перепишет): используйте `session.error().set`.
    pub error: RwSignal<Option<String>>,

    /// Накопленный текст streaming-ASR: дельты во время записи + Final
    /// после её окончания. UI input-panel может подписаться на этот сигнал,
    /// чтобы показывать «черновик» транскрипции в реальном времени.
    pub live_text: RwSignal<String>,
    /// Запрос на финализацию: выставляется `stop_and_send_with_sink` и
    /// потребляется streaming-ASR-дренером при получении `Final`/`Error`.
    pub streaming_pending: Arc<Mutex<Option<PendingFinalize>>>,
}

impl AudioCtx {
    pub fn new() -> Self {
        let session = RecordingSession::new(RecordingOptions {
            decode_on_stop: false,
            // open_stream_on_start: true — задел под streaming-ASR, который
            // в следующей итерации (см. synaptix::facade::asr::stream) будет подписываться
            // на тот же поток PCM без дополнительной перезаписи.
            open_stream_on_start: true,
            ..Default::default()
        });

        let is_recording = use_signal(false);
        let vis_handle: RwSignal<Option<VisHandle>> = use_signal(None);
        let error: RwSignal<Option<String>> = use_signal(None);

        // Зеркала: подписываемся на session-сигналы и пушим в mirror'ы.
        {
            let s = session.state();
            create_effect(move || {
                is_recording.set(s.get() == RecordingState::Recording);
            });
        }
        {
            let v = session.vis_handle();
            create_effect(move || {
                vis_handle.set(v.get());
            });
        }
        {
            let e = session.error();
            create_effect(move || {
                error.set(e.get());
            });
        }

        Self {
            session,
            transcribing: use_signal(false),
            asr: Arc::new(Mutex::new(None)),
            asr_loaded_name: use_signal(None),
            asr_loading: use_signal(false),
            input_devices: use_signal(None),
            is_recording,
            vis_handle,
            error,
            live_text: use_signal(String::new()),
            streaming_pending: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for AudioCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Toggle-обработчик MI_MIC на input panel чата. Вызывается с главного потока.
/// Sink — `ChatInputAppend`: текст уходит в `chat.input`, как и до introduction
/// глобального голосового FAB.
pub fn toggle_recording() {
    let app = use_context::<AppCtx>();
    let actx = app.audio.clone();
    if actx.session.is_active() {
        stop_and_send_with_sink(actx, app, TranscriptSink::ChatInputAppend);
    } else {
        start_recording(actx, app);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Voice FAB: внешние обёртки над start_recording / stop_and_send_with_sink
// ─────────────────────────────────────────────────────────────────────────────

/// Старт записи из глобального FAB-окна.
///
/// Если ASR-модель ещё не загружена — пытаемся загрузить её **автоматически**:
/// 1. Если `selected_audio_model=None` и в `audio_models` есть пресеты — берём
///    первый и помечаем его как выбранный (persist'ится через autosave).
/// 2. Помечаем `voice.pending_record_start=true` и вызываем
///    [`load_selected_model`]. UI показывает «Загружаю модель…» в статусе
///    панели (см. `panel.rs::status_text_reactive`).
/// 3. Effect в `lib.rs::install_voice_auto_record` ловит завершение загрузки
///    (`asr_loading=false` + `asr_loaded_name=Some(_)`) и сам вызывает
///    `start_recording`.
///
/// Если модель уже загружена — стартуем сразу, как раньше.
pub fn voice_start() {
    let app = use_context::<AppCtx>();
    let actx = app.audio.clone();
    if actx.session.is_active() {
        return;
    }

    let asr_loaded = actx
        .asr
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    if asr_loaded {
        start_recording(actx, app);
        return;
    }

    // Загрузка уже идёт — оставляем pending=true чтобы effect подхватил.
    if actx.asr_loading.get_untracked() {
        app.voice.pending_record_start.set(true);
        return;
    }

    // Auto-select первой модели если ничего не выбрано — иначе load_selected_model
    // упадёт на «Не выбрана ASR-модель».
    if app.selected_audio_model.get_untracked().is_none() {
        let models = app.audio_models.get_untracked();
        if let Some(first) = models.first() {
            app.selected_audio_model.set(Some(first.name.clone()));
        } else {
            actx.session.error().set(Some(
                tr!("voice.asr.add_model_first"),
            ));
            return;
        }
    }

    app.voice.pending_record_start.set(true);
    load_selected_model();
}

/// «Пауза» в FAB-окне: stop+транскрипция с `final_chunk=false`. Транскрипция
/// чанка добавится в `voice.accumulated`, `awaiting_actions` НЕ поднимается —
/// пользователь может ткнуть Resume и продолжить запись, а накопленный текст
/// уже виден в окне.
pub fn voice_pause() {
    let app = use_context::<AppCtx>();
    let actx = app.audio.clone();
    if !actx.session.is_active() {
        return;
    }
    stop_and_send_with_sink(actx, app, TranscriptSink::VoicePanel { final_chunk: false });
}

/// Возобновить запись после Pause. Создаётся новая запись (предыдущая уже
/// застопирована и транскрибирована в `voice_pause`). Если в будущем потребуется
/// gapless pause без новой записи — переключить эту обёртку на `session.resume()`.
pub fn voice_resume() {
    let app = use_context::<AppCtx>();
    let actx = app.audio.clone();
    if actx.session.is_active() {
        return;
    }
    start_recording(actx, app);
}

/// Финальный Stop в FAB-окне: stop+транскрипция с `final_chunk=true`. После
/// успешной транскрипции `voice.awaiting_actions=true` — actions_row меняется
/// на Copy/Paste/Restart/Close.
pub fn voice_stop() {
    let app = use_context::<AppCtx>();
    let actx = app.audio.clone();
    if !actx.session.is_active() {
        return;
    }
    stop_and_send_with_sink(actx, app, TranscriptSink::VoicePanel { final_chunk: true });
}

fn start_recording(actx: AudioCtx, app: AppCtx) {
    // 1. Модель должна быть загружена (Settings → Аудио модели → «Загрузить»).
    let asr_loaded = actx
        .asr
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    if !asr_loaded {
        let msg = tr!("voice.asr.load_model_first");
        eprintln!("[synthos/audio] {msg}");
        actx.session.error().set(Some(msg));
        return;
    }

    // 2. Очистим live-буфер от прошлой записи: пользователь начинает новую
    // фразу, накопленный черновик уже отдан в input через предыдущий stop.
    actx.live_text.set(String::new());

    // 3. Старт через RecordingSession (cpal + vis_handle + audio_stream + elapsed).
    let preferred = app.general.audio_input_device.get_untracked();
    let preferred_opt = if preferred.trim().is_empty() {
        None
    } else {
        Some(preferred.as_str())
    };
    if let Err(e) = actx.session.start_with_device(preferred_opt) {
        let msg = tr!("voice.asr.record_start_failed", error = e);
        eprintln!("[synthos/audio] {msg}");
        actx.session.error().set(Some(msg));
        return;
    }

    // 4. Auto-сохранение реально использованного устройства.
    if let Some(actual) = actx.session.actual_device_name().get_untracked() {
        if !actual.is_empty() && actual != preferred {
            eprintln!(
                "[synthos/audio] устройство '{preferred}' недоступно, сохраняю '{actual}'"
            );
            app.general.audio_input_device.set(actual);
        }
    }

    // 5. Streaming-ASR: подписываемся на live PCM-стрим записи и запускаем
    // chunked-транскрипцию. Дельты приходят в `live_text` в реальном времени,
    // финал — после `session.stop()` и Disconnect Sender'а в рекордере.
    if let Some(stream) = actx.session.audio_stream().get_untracked() {
        let language = current_audio_model(&app).and_then(|m| {
            if m.language.is_empty() || m.language.eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(m.language)
            }
        });
        crate::agent::streaming_asr::start(stream, actx.clone(), language);
    }
}

fn stop_and_send_with_sink(actx: AudioCtx, app: AppCtx, sink: TranscriptSink) {
    let _ = app; // pending_finalize смотрит app через use_context в on_transcription_done.

    // Если streaming-ASR работает — выставляем pending_finalize, он подберёт
    // финальный текст и вызовет on_transcription_done. Без streaming-ASR
    // (или если модель выгружена в процессе записи) делаем classic-путь:
    // synchronously transcribe WAV через spawn_blocking.
    let streaming_active = actx.session.audio_stream().get_untracked().is_some();

    let result = match actx.session.stop() {
        Ok(r) => r,
        Err(e) => {
            let msg = tr!("voice.asr.record_stop_failed", error = e);
            eprintln!("[synthos/audio] {msg}");
            actx.session.error().set(Some(msg));
            return;
        }
    };
    let wav: Vec<u8> = (*result.wav_bytes).to_vec();
    actx.transcribing.set(true);

    if streaming_active {
        // Выставляем pending — дренер (streaming_asr.rs) увидит Final и
        // дёрнет on_transcription_done. После session.stop() Sender в
        // RecorderState уже дропнут → ASR-loop делает финальный inference
        // и эмитит Final. Это занимает ~1-3с в зависимости от размера остатка.
        if let Ok(mut g) = actx.streaming_pending.lock() {
            *g = Some(PendingFinalize { sink });
        }
        return;
    }

    fallback_transcribe(actx, sink, wav);
}

/// Fallback на классический blocking-путь: `Transcriber::transcribe_wav`
/// через `spawn_blocking`. Используется когда streaming-ASR не был запущен
/// (пустой стрим — модель выгрузилась в процессе записи) или после ошибки
/// streaming-ASR (`PendingFinalize` сам по себе не гарантирует Final).
fn fallback_transcribe(actx: AudioCtx, sink: TranscriptSink, wav: Vec<u8>) {
    let actx_send = actx.clone();
    let asr_holder = actx.asr.clone();
    spawn(async move {
        let wav_for_asr = wav;
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = match asr_holder.lock() {
                Ok(g) => g,
                Err(e) => return Err(format!("ASR mutex poisoned: {e}")),
            };
            let Some(t) = guard.as_mut() else {
                return Err(tr!("voice.asr.model_unloaded"));
            };
            t.transcribe_wav(&wav_for_asr).map_err(|e| e.to_string())
        })
        .await;

        let final_result: Result<String, String> = match result {
            Ok(inner) => inner,
            Err(join_err) => Err(format!("spawn_blocking join: {join_err}")),
        };
        run_on_main_thread(move || on_transcription_done(actx_send, final_result, sink));
    });
}

pub fn on_transcription_done(
    actx: AudioCtx,
    result: Result<String, String>,
    sink: TranscriptSink,
) {
    actx.transcribing.set(false);
    let app = use_context::<AppCtx>();
    match result {
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                actx.session
                    .error()
                    .set(Some(tr!("voice.asr.recognize_failed")));
                return;
            }
            match sink {
                TranscriptSink::ChatInputAppend => {
                    let syn = use_context::<crate::syn_chat::state::SynChatCtx>();
                    let prev = syn.input.get_untracked();
                    let merged = if prev.trim().is_empty() {
                        trimmed.to_string()
                    } else {
                        format!("{} {}", prev.trim_end(), trimmed)
                    };
                    syn.input.set(merged);
                    // Бампаем поколение, чтобы MultilineTextEdit пересобрался с новым текстом.
                    syn.input_gen.update(|n| *n = n.wrapping_add(1));
                }
                TranscriptSink::VoicePanel { final_chunk } => {
                    let voice = app.voice;
                    let prev = voice.accumulated.get_untracked();
                    let merged = if prev.trim().is_empty() {
                        trimmed.to_string()
                    } else {
                        format!("{} {}", prev.trim_end(), trimmed)
                    };
                    voice.accumulated.set(merged.clone());
                    voice.last_transcript.set(trimmed.to_string());
                    // Внешнее обновление raw-поля → бамп gen, чтобы Reactive
                    // пересобрал MultilineTextEdit с новым текстом.
                    voice.raw_gen.update(|n| *n = n.wrapping_add(1));
                    // Сбрасываем refined: новый исходник делает старую
                    // постобработку неактуальной.
                    voice.refined.set(String::new());
                    voice.refine_error.set(None);
                    voice.refined_gen.update(|n| *n = n.wrapping_add(1));
                    if final_chunk {
                        voice.awaiting_actions.set(true);
                    }
                }
            }
        }
        Err(e) => {
            let msg = tr!("voice.asr.recognize_error", error = e);
            eprintln!("[synthos/audio] {msg}");
            actx.session.error().set(Some(msg));
        }
    }
}

fn current_audio_model(app: &AppCtx) -> Option<AudioModelConfig> {
    let name = app.selected_audio_model.get_untracked()?;
    app.audio_models
        .get_untracked()
        .into_iter()
        .find(|m| m.name == name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Загрузка / выгрузка ASR-модели (вызывается из Settings → Аудио модели)
// ─────────────────────────────────────────────────────────────────────────────

fn config_kind_to_asr(kind: AsrEngineKind) -> AsrModelKind {
    match kind {
        AsrEngineKind::Whisper => AsrModelKind::Whisper,
        AsrEngineKind::GigaAm => AsrModelKind::GigaAm,
    }
}

/// Маппит строковый `device` из `AudioModelConfig` в `synaptix_core::Device`.
/// `"gpu_auto"` выбирает GPU только в GPU-сборке (`asr-cuda`/`asr-metal`),
/// иначе fallback на CPU (без паники).
fn config_device_to_synaptix(s: &str) -> synaptix::facade::asr::Device {
    use synaptix::facade::asr::Device;
    match s {
        "gpu_auto" => audio_gpu_device(),
        _ => Device::Cpu,
    }
}

fn audio_gpu_device() -> synaptix::facade::asr::Device {
    synaptix::facade::asr::Device::Cuda(0)
}

/// Маппит строковый `compute_dtype` из `AudioModelConfig` в [`ComputeDType`].
/// Принимает F32/BF16/F16 (standard) или FP8E4M3/NVFP4 (native quantized
/// paths). Unknown → F16.
fn config_compute_to_compute_dtype(s: &str) -> synaptix::facade::asr::ComputeDType {
    synaptix::facade::asr::ComputeDType::from_name(s).unwrap_or(synaptix::facade::asr::ComputeDType::F16)
}

/// Маппит строковый `storage_dtype` из `AudioModelConfig` в `StorageDType`.
/// На Stage 0 quantized форматы не реализованы — `Transcriber::load`
/// выдаст warn + downgrade до standard, поэтому здесь просто пробрасываем
/// что есть. Unknown → F16.
fn config_storage_to_storage_dtype(s: &str) -> synaptix::facade::asr::StorageDType {
    match s.to_ascii_lowercase().as_str() {
        "fp8e4m3" | "fp8" | "mxfp8" => synaptix::facade::asr::StorageDType::MXFP8,
        other => {
            synaptix_core::precision::parse_dtype(other).unwrap_or(synaptix::facade::asr::StorageDType::F16)
        }
    }
}

/// Запустить загрузку ASR-модели по выбранному пресету.
///
/// Долгая операция (десятки секунд для Whisper Large на CPU). Выполняется в
/// `spawn_blocking`. Пока идёт — `asr_loading=true`, кнопка в UI блокируется.
/// При успехе — `asr_loaded_name=Some(...)`, при ошибке — заполняется `error`.
pub fn load_selected_model() {
    let app = use_context::<AppCtx>();
    let Some(model) = current_audio_model(&app) else {
        app.audio
            .session
            .error()
            .set(Some(tr!("voice.asr.no_model_selected")));
        return;
    };
    if app.audio.asr_loading.get_untracked() {
        return;
    }
    if model.model_path.trim().is_empty() {
        app.audio
            .session
            .error()
            .set(Some(tr!("voice.asr.no_model_path")));
        return;
    }

    let cfg = AsrConfig {
        kind: config_kind_to_asr(model.kind),
        model_path: PathBuf::from(model.model_path.clone()),
        language: if model.language.is_empty() || model.language.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(model.language.clone())
        },
        device: config_device_to_synaptix(&model.device),
        storage_dtype: config_storage_to_storage_dtype(&model.storage_dtype),
        compute_dtype: config_compute_to_compute_dtype(&model.compute_dtype),
    };
    let model_name = model.name.clone();
    let actx = app.audio.clone();
    actx.session.error().set(None);
    actx.asr_loading.set(true);

    spawn(async move {
        let result = tokio::task::spawn_blocking(move || Transcriber::load(cfg)).await;
        let actx2 = actx.clone();
        run_on_main_thread(move || {
            actx2.asr_loading.set(false);
            match result {
                Ok(Ok(t)) => {
                    if let Ok(mut g) = actx2.asr.lock() {
                        *g = Some(t);
                    }
                    actx2.asr_loaded_name.set(Some(model_name));
                }
                Ok(Err(e)) => {
                    let msg = tr!("voice.asr.load_failed", error = e);
                    eprintln!("[synthos/audio] {msg}");
                    actx2.session.error().set(Some(msg));
                }
                Err(e) => {
                    let msg = format!("spawn_blocking join: {e}");
                    eprintln!("[synthos/audio] {msg}");
                    actx2.session.error().set(Some(msg));
                }
            }
        });
    });
}

/// Выгрузить текущую ASR-модель (освобождает память, сбрасывает имя).
pub fn unload_model() {
    let app = use_context::<AppCtx>();
    if let Ok(mut g) = app.audio.asr.lock() {
        *g = None;
    }
    app.audio.asr_loaded_name.set(None);
    app.audio.session.error().set(None);
}

/// Опросить микрофоны в фоне и положить в `AudioCtx::input_devices`.
/// Повторный вызов, пока опрос идёт, ничего не делает; `force` заставляет
/// перечитать (пользователь воткнул микрофон и нажал «обновить»).
pub fn scan_input_devices(ctx: &AudioCtx, force: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);

    if !force && ctx.input_devices.get_untracked().is_some() {
        return;
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let devices = ctx.input_devices;
    // Обычный поток, а не задача рантайма: перебор устройств блокирующий.
    std::thread::spawn(move || {
        let list = syngui::audio::list_input_devices();
        syngui::async_runtime::run_on_main_thread(move || {
            devices.set(Some(list));
            BUSY.store(false, Ordering::SeqCst);
        });
    });
}

#[cfg(test)]
mod input_devices_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Опрос микрофонов не должен занимать время вызывающего: `cpal`
    /// перебирает устройства около секунды, а зовут его из сборки виджета
    /// настроек — раньше на это вставал весь интерфейс.
    #[test]
    fn scan_does_not_block_the_caller() {
        syngui::signal::allow_signal_reads_on_this_thread();
        let ctx = AudioCtx::new();
        assert!(ctx.input_devices.get_untracked().is_none());

        let started = Instant::now();
        scan_input_devices(&ctx, false);
        let call = started.elapsed();
        assert!(
            call < Duration::from_millis(100),
            "вызов занял {call:?} — опрос идёт не в фоне"
        );

        // Ждём фонового ответа: он приходит колбэком на главный поток.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && ctx.input_devices.get_untracked().is_none() {
            syngui::async_runtime::drain_main_thread_callbacks();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            ctx.input_devices.get_untracked().is_some(),
            "список микрофонов так и не пришёл"
        );
    }
}
