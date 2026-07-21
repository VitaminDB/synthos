# Mixer node + динамические порты в node-editor (2026-05-11)

Краткая записка: что делает `NodeKind::Mixer`, как устроены вариативные
порты и куда смотреть при правках.

## Что добавлено

1. **`PortsSpec`** в `pages/node_editor/types.rs` — enum с двумя вариантами:
   - `Static(&'static [PortSchema])` — старый дефолт.
   - `Dynamic { pool: &'static [PortSchema], runtime: fn(&NodeInstance) -> &'static [PortSchema] }`
     — порты вычисляются на лету по экземпляру (например, по `n_inputs` в
     `NodeRuntime`). `pool` нужен для schema-операций без instance
     (загрузка шаблона, валидация имён портов в `templates::convert`).
2. **`NodeKindMeta.inputs / .outputs: PortsSpec`** в `pages/node_editor/registry.rs`.
   Все существующие меты обёрнуты в `PortsSpec::Static(...)`. Helper-методы
   `PortsSpec::resolve(node)` / `PortsSpec::pool()` дают нужный срез.
3. **`NodeKind::Mixer` + `NodeRuntime::Mixer`** (в `types.rs`):
   - `n_inputs: RwSignal<usize>` (2..=`MIXER_MAX_INPUTS`=16) — UI-spinbox растит/режет.
   - `gains_db: Vec<RwSignal<f32>>` (длина 16, активен префикс) — UI-state.
   - `live_gains: Vec<Arc<AtomicU32>>` (linear bits) — read worker'ом без локов.
   - `last_input_kinds: Vec<PrevSource>` — детект смены сигнатуры входов.
   - Стандартные `out_stream` / `worker` / `buffer_cache` (как у Gain/Filter).
4. **Executor** в `nodes/audio_mixer.rs`:
   - Streaming-режим: на изменение любой `last_input_kinds[i]` пересоздаёт
     worker. Worker per-tick тянет 1 chunk с каждого активного receiver'а,
     умножает на `live_gains[i].linear` и суммирует в общий out-chunk.
     Несовпадающие sr/ch у источников → log::warn, берём параметры первого.
   - Buffer-режим: `mix_buffers(&[(idx, buf, gain)])` — простая offline-сумма
     с длиной первого буфера (короткие добиваются нулями, длинные обрезаются).
     Кэшируется через общий `audio_gain::process_buffer_cached`.
5. **Body** (`audio_mixer::body`) — Column из header (SpinBox 2..16 + out-dot)
   + Reactive-блок channel-strip'ов (по N) с `inline_port_dot(in_i)` +
   Slider -24..+12 dB + readout. Reactive подписан на `n_inputs` —
   изменение spinbox'а перерисовывает body, evaluate сразу видит новый
   набор портов через `PortsSpec::Dynamic.runtime`.
6. **Меню «Add Node»** (`pages/node_editor/mod.rs::build_bg_menu`):
   - В DSP-эффектах появилось submenu **«Аудио микшеры»** с пресетами
     `2 источника / 3 / 4 / 5 / Custom`. Каждый пресет создаёт
     `NodeKind::Mixer` и сразу выставляет `n_inputs`.
   - В DSP-эффектах появилось submenu **«Эквалайзеры»** — четыре существующих
     `Equalizer{6,10,20,30}` сгруппированы через `subcategory: Some("Эквалайзеры")`.
7. **`NodeEditorCtx::set_mixer_n_inputs(id, n)`** (state.rs) — публичный
   helper: меняет `n_inputs` и удаляет connections к портам, которых
   больше нет в активном наборе. Применяется и из меню, и в spinbox.
8. **Шаблоны** (`templates/builtin.rs`): `Mix 2 Sources` и `Mic + File`.

## Имена портов микшера

`MIXER_PORT_SCHEMAS_FULL: [PortSchema; 16]` — статический пул. Имена
`in_1..in_16` строятся const'ным `match`-маппингом (`port_name(i)`),
поэтому `&'static str` гарантированы. `mixer_inputs(node)` возвращает
`&MIXER_PORT_SCHEMAS_FULL[..n_inputs]`. Output — отдельный
`MIXER_OUTPUT_SCHEMAS: [PortSchema; 1] = ["out"]`.

## Кэш-ключ buffer-режима

В offline-сложении ключ `(input_ptr, params)` для
`process_buffer_cached` строится как XOR указателей всех активных
буферов и их слот-индексов. Это спасает от случайной коллизии «два
разных набора входов с одинаковой суммой ptr». Параметры — текущий
вектор линейных gain'ов (одной длины с активным N).

## Что НЕ сделано (на будущее)

- **Pan / per-канальный mute / solo**: сейчас только mono-gain. Стоит
  добавить отдельный `pan: f32 [-1..+1]` и `mute: bool` в runtime.
- **VU-метры на каждом канале**: live_gains есть, нужен ещё RMS-bin
  через `compute_rms_bins` + миниатюрная StaticWaveform-полоска.
- **Resample-on-mismatch**: сейчас если у входов разный sample_rate,
  один из них «играет с искажением». Решение — подключить `rubato` resampler
  на входной chunk до сложения. Скрытое поле в `NodeRuntime::Mixer`.
