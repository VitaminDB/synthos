# synthos: эффект-ноды Gain/Filter/Reverb + SaveToFile (2026-05-10)

Расширение свежего streaming-фундамента (`AudioStream` + trait-`NodeExecutor`,
см. [audio_streaming_2026.md](./audio_streaming_2026.md)) четырьмя новыми
NodeKind'ами:

- `Gain` — линейный gain в дБ (-24…+24).
- `Filter` — Biquad LP/HP/BP, RBJ-cookbook, Q=0.707.
- `Reverb` — Schroeder (4 параллельных comb + 2 series allpass), mix + room.
- `SaveToFile` — запись AudioStream в WAV PCM16 (streaming-write).

## Архитектура streaming-эффекта

Каждая effect-нода — однотипный pipeline:

1. **Detection источника**: executor сравнивает `PrevSource::Stream(ptr)` с
   `last_input_kind`. Смена → останавливаем старый worker (drop handle) и
   обнуляем `out_stream`.
2. **Захват receiver'а**: `s.take_receiver()` (single-sub). Если `None`
   (кто-то уже забрал) — выход с `Empty`.
3. **Создание output-канала**: `(tx, out_rx) = mpsc::channel()`,
   `AudioStream::from_channel(out_rx, sr, ch)` → новый `Arc<AudioStream>`,
   публикуется в output-port.
4. **Worker-поток**: `pull from rx → process chunk → push to tx`, выход
   на RecvError (upstream закрыт) или SendError (downstream отписался).

Live-параметры передаются через `Arc<AtomicU32>` (f32-bits) и
`Arc<AtomicBool>` для `dirty`-флага коэффициентов. UI пишет атомиками без
локов; worker читает/пересчитывает только при `dirty.swap(false)`.

## DSP-примитивы (syngui/src/audio/dsp/)

Pure-Rust, без новых крейтов:

- `LinearGain` — скалярный множитель (`from_db` для конверсии).
- `Biquad` — direct-form-I, RBJ-cookbook коэффициенты, smart cache (no-op
  на повторный `update_coeffs` с тем же набором параметров). LP/HP/BP.
- `SchroederReverb` — 4 freeverb-comb (1116/1188/1277/1356 sps @ 44.1 kHz,
  scale by sr) + 2 freeverb-allpass (225/556 sps), per-comb LP-damping.

Unit-тесты (`audio::dsp::tests`):
- `gain_unity_passthrough` / `gain_from_db_zero_is_unity` /
  `gain_from_db_minus_six_halves_amplitude`.
- `biquad_lowpass_attenuates_above_cutoff` (10 кГц через LP@1кГц → ratio < 0.05).
- `biquad_lowpass_passes_below_cutoff` (200 Гц через LP@5кГц → ratio > 0.9).
- `biquad_highpass_attenuates_below_cutoff`.
- `reverb_dry_zero_mix_is_identity`, `reverb_tail_decays_after_impulse`.

## SaveToFile — streaming-write через hound

Новый публичный модуль `syngui/src/audio/wav.rs`:

```rust
pub struct WavStreamWriter { ... }
impl WavStreamWriter {
    pub fn open(path, sr, ch) -> Result<Self, AudioError>;
    pub fn write_chunk(&mut self, samples: &[f32]) -> Result<(), AudioError>;
    pub fn finalize(&mut self) -> Result<(), AudioError>;
    pub fn samples_written(&self) -> u64;
    pub fn duration_seconds(&self) -> f64;
    pub fn is_open(&self) -> bool;
}
pub fn into_pcm16_bytes(samples, sr) -> Result<Vec<u8>, AudioError>;
```

Recorder.rs приватная `encode_wav_pcm16` рефакторится поверх
`into_pcm16_bytes` (без изменения публичного поведения).

### Поток записи в SaveToFile

1. **Executor** при detection нового AudioStream'а забирает receiver и
   кладёт в `runtime.pending_rx: Arc<Mutex<Option<(rx, sr, ch)>>>` +
   `has_upstream.set(true)`. Это нужно потому, что receiver — single-sub:
   взять его в момент Record-клика уже невозможно (Player мог его забрать).
2. **Record-клик** в body:
   - `runtime.pending_rx.take()` → если `Some((rx, sr, ch))`, открываем
     `WavStreamWriter::open(path, sr, ch)`.
   - Спавним worker `recv → write_chunk → update RwSignals` (counters).
   - `is_writing=true`, `status=Writing`.
3. **Stop-клик**: `cancel.store(true)`, drop worker handle.
   Worker завершает цикл, `finalize()`, обновляет `status=Saved(path)`
   или `Error(msg)`. После Stop `has_upstream=false` — receiver уже
   потреблён, для повторной записи нужно пересоединить провод.

## Builtin-шаблоны

В `templates/builtin.rs::all()` добавлены два новых:

- **«Audio Effects Chain»** — Recorder → Gain → Filter → Reverb → Player.
- **«Save Recording»** — Recorder → SaveToFile (PCM16 WAV).

Стартовые точки положений нод подобраны под widerasl-карточки эффектов
(каждая ~420px wide, шаг ~400-460px по горизонтали).

## MSS

`styles/components/audio_node.mss`:

- `.gain-node-slider`, `.filter-node-slider`, `.reverb-node-slider` —
  компактные 110px слайдеры с MSS-колорингом (audio-node-volume-slider стиль).
- `.filter-node-mode-host` — обёртка вокруг Dropdown (LP/HP/BP), фикс высоты.
- `.save-node-path-host` (280px), `.save-node-status-host` (140-220px).
- `.save-node-status-active` — pulse-keyframe (общая `audio-record-pulse`).
- `.save-node-status-saved` — emerald-зелёный текст (#34D399).

## Ограничения текущей итерации

- **Channel count**: SaveToFile уважает `s.channels` от upstream'а, но
  Recorder в synthos сейчас всегда mono. Stereo-pipeline появится когда
  Recorder начнёт публиковать stereo-stream.
- **Sample-rate mismatch**: эффект-ноды работают на native-rate входного
  потока. Mismatch с output device решает существующий drainer Player'а
  (rubato, см. `audio/player.rs::run_player_thread_streaming`).
- **Re-record SaveToFile**: после Stop receiver исчерпан; для повторной
  записи нужен reconnect-провод. Альтернатива (multi-record) потребует
  multi-sub fan-out (см. TODO в `stream.rs`).
- **Filter Q**: фиксирован 0.707 (Butterworth). Q-слайдер можно добавить
  позднее без изменения port-схемы.
- **Reverb алгоритм**: Schroeder. Freeverb (8+4 combs/allpasses) — апгрейд
  через `algorithm: Choice` field, без слома port-схемы.

## Файлы, добавленные/изменённые

### syngui (фреймворк)
- **NEW** `syngui/src/audio/wav.rs` — публичный `WavStreamWriter` + `into_pcm16_bytes`.
- **NEW** `syngui/src/audio/dsp/mod.rs` — `LinearGain` / `Biquad` / `SchroederReverb`.
- `syngui/src/audio/mod.rs` — `pub mod wav`, `pub mod dsp` + ре-экспорты.
- `syngui/src/audio/recorder.rs` — `encode_wav_pcm16` удалён, использует `wav::into_pcm16_bytes`.
- `syngui/src/audio/stream.rs` — `pub fn from_channel` (публичный конструктор для эффект-нод).

### synthos (приложение)
- `pages/node_editor/types.rs` — `NodeKind::{Gain, Filter, Reverb, SaveToFile}` + `FilterMode`/`SaveStatus` + `NodeRuntime` варианты.
- `pages/node_editor/registry.rs` — 4 новых `NodeKindMeta` + `default_runtime`.
- **NEW** `pages/node_editor/nodes/audio_gain.rs`
- **NEW** `pages/node_editor/nodes/audio_filter.rs`
- **NEW** `pages/node_editor/nodes/audio_reverb.rs`
- **NEW** `pages/node_editor/nodes/audio_save.rs`
- `pages/node_editor/nodes/mod.rs` — re-exports.
- `pages/node_editor/template_preview.rs` — colors для новых NodeKind'ов.
- `templates/builtin.rs` — «Audio Effects Chain» + «Save Recording».
- `styles/components/audio_node.mss` — новые классы.
- `icons.rs` — `MI_FILTER_ALT` (`\u{EF4F}`), `MI_BLUR_ON` (`\u{E3A5}`).
