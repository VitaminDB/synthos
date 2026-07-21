# Audio streaming в node-editor (2026-05-10)

Live PCM-чанки между нодами без full-buffer. Открывает дорогу для эффект-нод (Gain/Filter/Reverb), save-to-file, других стриминговых источников (TTS).

## Архитектура

### `syngui::audio::AudioStream` — single-sub канал

Файл `syngui/src/audio/stream.rs`. Контракт:

```rust
pub struct AudioStream {
    pub sample_rate: u32,
    pub channels: u16,
    rx: Mutex<Option<mpsc::Receiver<Vec<f32>>>>,
}
impl AudioStream {
    pub(crate) fn new(rx, sr, ch) -> Arc<Self>;
    pub fn take_receiver(&self) -> Option<Receiver<Vec<f32>>>;  // first call → Some, else None
}
```

- **Single-sub by design.** `mpsc::Receiver` передаётся одному consumer'у через `take_receiver`. Повторный вызов → `None`. Это согласовано с `AudioPlayer::start_streaming(rx, sr)`, который тоже single-consumer.
- **Lifetime.** Producer (recorder) держит `Sender` в собственном state'е. Когда producer останавливается — `Sender` дропается, `Receiver::recv()` возвращает `RecvError`, drainer-thread в плеере завершается.
- **`PartialEq` всегда `false`** — у live-стрима нет смысла «равенства»; `RwSignal::set` всегда триггерит перерисовку при `set_some`.

### Producer-side: `AudioRecorder::open_stream()`

`syngui/src/audio/recorder.rs`:

```rust
pub fn open_stream(&self) -> Option<Arc<AudioStream>>;
```

- Создаёт `mpsc::channel`, регистрирует Sender в `RecorderState.stream_tx`.
- Single-sub инвариант: повторное открытие на том же рекордере → `None`.
- В `RecorderState::push_frames` каждый callback'ный chunk моно-PCM (после downmix'а) клонируется и пушится в Sender, если он есть. Стоимость: `Vec<f32>::clone()` на ~480 samples = ~2KB на callback.

### `PortValue::AudioStream`

`app/synthos/src/pages/node_editor/types.rs`:

```rust
pub enum PortValue {
    Empty,
    Float(f32),
    Audio(Arc<AudioBuffer>),
    AudioStream(Arc<AudioStream>),
}
```

`PartialEq` для `AudioStream`-варианта — `Arc::ptr_eq`. Helper `as_audio_stream() -> Option<Arc<AudioStream>>`.

### `NodeRuntime` extensions

`AudioRecorder` runtime:
```rust
stream: RwSignal<Option<Arc<AudioStream>>>,  // Some пока recording, None после stop
```

`AudioPlayer` runtime (полностью переработан вокруг `PrevSource`):
```rust
last_input_kind: PrevSource,            // None | Buffer(usize) | Stream(usize)
pending_stream: Mutex<Option<Receiver<Vec<f32>>>>,  // забран в момент detection
stream_sample_rate: u32,                // snapshot для start_streaming(rx, sr)
is_streaming: RwSignal<bool>,           // UI: LIVE-бейдж и --:-- timecode
```

Старое поле `last_input_ptr: usize` удалено. `PrevSource` enum исключает ложные срабатывания, когда указатели Buffer и Stream нумерически совпадут.

### Executors

`AudioRecorderExec::evaluate`:
- `is_recording=true && stream=Some` → `PortValue::AudioStream(stream.clone())`
- `is_recording=false && last_buffer=Some` → `PortValue::Audio(buf.clone())`
- иначе → `Empty`

`AudioPlayerExec::evaluate` (sink):
- detect kind через `PrevSource::{Buffer,Stream}(Arc::as_ptr as usize)`
- при смене → stop текущего плеера, сбросить progress, обновить `pcm_view`/`pending_stream`/`is_streaming`
- для `AudioStream` источника: `s.take_receiver()` сразу при detection — receiver «зарезервирован» за этим Player'ом, не утечёт к другому потенциальному consumer'у

### UI Player body

В streaming-режиме (`is_streaming.get() == true`):
- waveform → пульсирующий LIVE-бейдж (`audio-node-live-badge` MSS-класс с `audio-live-pulse` keyframe)
- timecode → `live  /  --:--` (длительность неизвестна)
- on_seek → no-op (seek в streaming не поддерживается, см. `AudioPlayer::seek_seconds` → `SeekNotSupported`)
- toggle_play_pause → `AudioPlayer::start_streaming(rx, stream_sample_rate)` вместо `start`/`start_stereo`

## Сценарий end-to-end

1. Пользователь добавляет Recorder и Player, соединяет.
2. Пользователь жмёт Mic на Recorder:
   - `AudioRecorder::start_with_device` → cpal stream открыт.
   - `r.open_stream()` → создаёт `AudioStream`, кладёт в `runtime.stream`.
3. `AudioRecorderExec::evaluate` (через реактивный effect на `is_recording`/`stream`) → output `PortValue::AudioStream(s)`.
4. `AudioPlayerExec::evaluate` детектит `Stream(ptr)`:
   - stop старого плеера (если был);
   - `is_streaming = true`, `pcm_view = None`;
   - `pending_stream = s.take_receiver()`;
   - `stream_sample_rate = s.sample_rate`.
5. UI Player'а перерисовывается: LIVE-бейдж, `live / --:--`.
6. Пользователь жмёт Play:
   - `pending_stream.take()` → `rx`;
   - `AudioPlayer::start_streaming(rx, sr)` → cpal output stream живой.
7. cpal callback Recorder'а → `push_frames` → `Sender::send(mono.clone())` → drainer в Player'е → callback Player'а играет.
8. Пользователь жмёт Stop на Recorder:
   - `is_recording = false`, `stream = None` в runtime;
   - `AudioRecorder` дропается (через `stop_and_encode_wav`), `Sender` дропается, `Receiver::recv` → `RecvError`;
   - Player drainer-thread завершается → `is_done`;
   - после decode WAV — `last_buffer = Some(buf)`;
   - executor переключает output с `Empty` на `Audio(buf)`;
   - Player executor детектит `Buffer(ptr)` → `is_streaming=false`, `pcm_view=Some(buf)`, можно опять Play (батч-режим, теперь с seek и duration).

## Trait NodeExecutor

Этот этап шёл вместе со streaming'ом. Файл `app/synthos/src/pages/node_editor/eval.rs` теперь содержит:

```rust
pub trait NodeExecutor: Send + Sync + 'static {
    fn evaluate(&self, ctx: &mut EvalContext<'_>);
}
pub struct EvalContext<'a> { /* node, track, incoming, values */ }
impl EvalContext<'_> {
    pub fn read_input(port) -> PortValue;
    pub fn write_output(port, v: PortValue);
    pub fn read_float_field(name) -> f32;
    pub fn runtime() -> &Arc<Mutex<NodeRuntime>>;
    pub fn node_id() -> NodeId;
}
```

Каждая нода предоставляет ZST-executor, регистрируется как `&'static dyn NodeExecutor` в `NodeKindMeta::executor`. `evaluate_graph` диспатчит через `meta.executor.evaluate(ctx)` — match по NodeKind пропал.

Размещение:
- скаляры (Number/Add/Output/Demo) → `nodes/scalar.rs`
- audio (AudioFile/Player/Recorder) → внутри своих `nodes/audio_*.rs`

## Ограничения / TODO

- **Multi-sub** не реализован. Если нужен fan-out (один Recorder в два Player'а одновременно) — добавить `BroadcastAudioStream` или `AudioStream::tee()`. Сейчас второй consumer получит `None` от `take_receiver`.
- **Variable chunk-size** в стриме. cpal callback приходит с разной длиной (256–512 samples). Для эффект-цепочек может понадобиться `Chunker`-нода с фиксированным rebuffering. Решим по факту.
- **`AudioRecorder::set_paused`** не реализован — push в стрим продолжается / прекращается атомарно с записью; pause-режим у рекордера — отдельная задача.
- **Volume в streaming** — тот же per-callback gain через `AudioPlayer::set_volume` (atomic), уже работает.
