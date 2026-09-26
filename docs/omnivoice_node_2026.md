# OmniVoice TTS-нода в node-editor (synthos, 2026-05-12)

`NodeKind::OmniVoice` — TTS-нода на базе движка **OmniVoice** (Qwen3-1B LM +
Higgs-Audio codec + discrete mask-diffusion). Text → Audio с автоматическим
выбором режима (Auto / Design / Clone) и поддержкой zero-shot voice cloning.

## Регистрация

- `app/synthos/src/pages/node_editor/registry.rs::OMNIVOICE` — `NodeKindMeta`
  с категорией `Neuro` и подкатегорией «Синтез речи». Иконка `MI_CAMPAIGN`.
- `app/synthos/src/pages/node_editor/types.rs::NodeKind::OmniVoice` —
  enum-вариант, входит в `NodeKind::ALL`.
- `app/synthos/src/pages/node_editor/types.rs::NodeRuntime::OmniVoice` —
  набор `RwSignal`'ов + `Arc<Mutex<Option<OmniVoicePipeline>>>` для lazy-load.

## Порты

| Сторона | Имя         | Тип        | Назначение                                   |
|---------|-------------|------------|----------------------------------------------|
| Вход    | `text`      | `Text`     | Основной текст для синтеза (обязателен).     |
| Вход    | `ref_audio` | `Audio`    | Reference-аудио для voice clone (опц.).      |
| Вход    | `ref_text`  | `Text`     | Транскрипт ref-аудио (опц., fallback ниже).  |
| Выход   | `audio`     | `Audio`    | Синтезированный PCM 24 kHz mono.             |

## Mode resolution

Mode выбирается автоматически по приоритету:

1. `ref_audio` подключен → **`Clone(VoiceClonePrompt)`** — голос имитирует
   референс. `ref_text` берётся из порта, если пуст — из textfield в body.
   `ref_lang` — значение поля `Language`.
2. иначе `instruct` непуст → **`Design { instruct }`** — голос задаётся
   описанием на естественном языке («женский низкий тембр, спокойный темп»).
3. иначе → **`Auto`** — модель сама выбирает голос.

Никаких явных переключателей режима в UI — пользователь подключает порт /
заполняет textfield, mode подбирается автоматически.

## UI body (13 field-rows)

1. **Модель** — `rfd::pick_file` с фильтром `.syn` + «Все файлы».
   `OmniVoicePipeline::load` принимает каталог; нода берёт `parent()` от
   выбранного файла как `model_dir`. В UI рядом — имя каталога.
2. **Device** — Dropdown `[CPU, GPU (auto)]`. `GPU (auto)` использует
   `OmniVoicePipeline::best_device()` (CUDA → Metal → CPU).
3. **Storage** — Dropdown `[f16, bf16, f32, nvfp4, mxfp8]`.
   В текущей реализации `OmniVoicePipeline::load` не принимает storage-
   параметр (весь pipeline в F32), но значение входит в `OmniLoadedCfg` для
   инвалидации кэша. Активируется когда synaptix-core квант дойдёт до diffusion/LLM.
4. **Compute** — Dropdown `[f16, bf16, f32, nvfp4, mxfp8]`. Та же логика.
5. **Instruct** — `MultilineTextEdit` (до 3 строк), placeholder «женский
   низкий тембр, спокойный темп». Активен только в режиме Design.
6. **Ref text** — `TextField` (single-line), fallback когда порт `ref_text`
   пуст. Используется в Clone-режиме.
7. **Language** — `TextField`, default `ru`. Передаётся как `ref_lang` в
   `VoiceClonePrompt`.
8. **CFG** — `Slider` 0.0–5.0, step 0.05, default 2.0. (`guidance_scale`)
9. **Steps** — `Slider` 8–64, step 1, default 32. (`num_step`)
10. **T-shift** — `Slider` 0.0–1.0, step 0.01, default 0.1. (`t_shift`)
11. **Speed** — `Slider` 0.5–2.0, step 0.01, default 1.0.
12. **Seed** — `TextField` (u64). Default 0. Парсится, некорректный ввод
    игнорируется.
13. **Статус** — Reactive строка: `Ошибка: …` / `Синтезирование…` (с
    пульсом `.omnivoice-node-running`) / имя загруженного каталога / «—».

`field_like_row` дублирован из `asr_gigaam.rs` (там private) — `[label |
spacer | control]` с MSS-классами `.node-card-field-*`. `WrapPadded`-обёртка
для `Box<dyn Widget>` (без неё `Padding::child` не принимает уже-боксированный
виджет) — копия из `asr_gigaam.rs`.

## Поток данных (start → worker)

1. **start (UI-thread)** — снимок всех сигналов + Arc-clone handle'ов
   `pipeline` / `loaded_cfg` / `output_buf`. Lock не удерживается — worker
   обращается через свои клон-handle'ы. Валидации:
   - `model_path` обязан быть выбран (иначе error).
   - `text` обязан быть подключён к порту `text` (иначе error).
   - `running` уже true → early return (idempotent).
2. **synth_worker (`std::thread::spawn`)**:
   - Сравнение `loaded_cfg` со снимком. Если не совпадает — сброс
     `pipeline` + `OmniVoicePipeline::load(model_dir, device)` (sync,
     ~30 с на CPU при первой загрузке).
   - Сохранение `ref_audio_buf` (если есть) во временный WAV через
     `hound::WavWriter` (Float32, родная sample_rate + channels из
     `AudioBuffer`). `OmniVoicePipeline` декодирует через symphonia и
     ресэмплит в 24 kHz mono сам.
   - Mode resolution → `Clone(prompt)` / `Design{instruct}` / `Auto`.
   - `pipeline.synthesize(text, &mode, &gen_cfg)` (sync). Lock держится
     на всё время инференса — в одной ноде нет смысла запускать
     параллельные синтезы.
   - Удаление tmp WAV (best-effort, `std::fs::remove_file`).
   - Обёртка PCM в `AudioBuffer::new(Arc::from(pcm), pl.sample_rate(), 1)`
     и публикация в `output_buf` (Mutex).
   - `output_version.update(...)` маршалится через `run_on_main_thread` —
     `RwSignal::update` валится с не-main thread'а (signal-RUNTIME
     thread_local пуст в worker'е).
   - `running.set(false)`.
3. **Executor** (`OmniVoiceExec::evaluate`) — читает `output_buf` под
   lock'ом и пишет `PortValue::Audio(buf)` (или `Empty`) в порт `audio`.
   Подписка на `output_version` обеспечивает re-evaluate downstream.

## Lazy-load кэш

`pipeline: Arc<Mutex<Option<OmniVoicePipeline>>>` + `loaded_cfg:
Arc<Mutex<Option<OmniLoadedCfg>>>` живут в `NodeRuntime::OmniVoice`. Перед
synthesize воркер сравнивает snapshot (`OmniLoadedCfg`) с last-loaded — если
не совпадают, pipeline сбрасывается и перезагружается. Это значит:

- Смена model_path / device → перезагрузка (~30 с CPU, заметно меньше на CUDA).
- Изменение storage_idx / compute_idx сейчас тоже триггерит перезагрузку
  (хотя реально pipeline не использует эти dtype'ы) — заложено на будущее.
- Изменение параметров генерации (cfg, steps, t_shift, speed, seed) — НЕ
  триггерит перезагрузку. Они передаются в `synthesize` per-call.

## Зависимости (Cargo)

- `tts-omnivoice` (merged single crate, workspace dep) — public API через `tts_omnivoice::pipeline::OmniVoicePipeline::{load, synthesize, best_device, sample_rate}` и `tts_omnivoice::core::{GenerationMode, VoiceClonePrompt, GenerationConfig}`.
- `synaptix-core` — `Device`.
- `hound` — запись tmp-WAV для ref_audio.
- `rfd` — FilePicker.

## MSS

`app/synthos/styles/components/omnivoice_node.mss`:

- `.omnivoice-node-running` — pulse (opacity 0.6↔1.0, 1100ms, ease-in-out,
  alternate, infinite). Совпадает по тайму с `.asr-node-running` — визуально
  синхронизирует «работающие» нейро-ноды в одной сцене.

Mount: `src/styles.rs` — `include_str!("../styles/components/omnivoice_node.mss")`
между `asr_node.mss` и `syn_explorer.mss`.

## Ограничения

- `OmniVoicePipeline::load` — sync и медленный (~30 с CPU). Прогресс-бар
  загрузки пока не реализован; UI показывает «Синтезирование…» (общий статус),
  фактически в первые секунды идёт load. Можно добавить более точную
  индикацию через два состояния `LoadingModel` / `Synthesizing`, но это
  требует поддержки от worker'а.
- AudioStream live ref не поддержан — только snapshot `Arc<AudioBuffer>` с
  порта `ref_audio`.
- ~~Сериализация настроек ноды в шаблон/JSON отложена.~~ Есть
  (`OmniVoiceStateData`, `templates/convert.rs`).
- ~~`storage_idx` / `compute_idx` декоративные.~~ Идут в загрузку
  (`map_handle_storage`/`map_handle_compute`).
- Прогресс diffusion-шагов не виден (`IterativeSampler::run` без callback'а).

## Иконка

`MI_CAMPAIGN` (рупор, U+EF49) — визуально отличается от `MI_RECORD_VOICE_OVER`
у GigaAM ASR, при этом остаётся в семантическом поле «голос/звук». В палитре
для preview графов используется индиго `Color::new(0.55, 0.42, 0.92, 1.0)`.
