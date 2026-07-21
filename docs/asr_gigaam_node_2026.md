# ASR-нода (GigaAM) в node-editor — synthos 2026-05-12

Нода `NodeKind::AsrGigaam` — Audio → Text транскрибация на базе модели **GigaAM**
(CTC). Категория **«Нейро» → «Транскрибация»** (новый top-level `NodeCategory::Neuro`).

## Порты

| Сторона | Имя | PortKind   | Что несёт              |
|---------|-----|------------|------------------------|
| input   | in  | Audio      | `PortValue::Audio(Arc<AudioBuffer>)` |
| output  | out | Text (new) | `PortValue::Text(String)`             |

`PortKind::Text` + `PortValue::Text(String)` — новые варианты, добавлены
в этой же итерации. Цвет провода — `#EAB308` (yellow-500), отличается
от Data/Audio/Control.

## Body / UI

Тело ноды — 4 строки:

1. **File picker** — `rfd` open-dialog с фильтром `.syn`, имя выбранного
   файла и зелёный бадж `✓ <model_name>` после успешной загрузки.
2. **Settings** — три dropdown'а:
   - `device`: `CPU` / `GPU (auto)` (Cuda → Metal → fallback Cpu).
   - `storage`: `f16 / bf16 / f32 / q8_0 / q4_0 / fp8e4m3 / nvfp4`.
   - `compute`: `f16 / bf16 / f32 / fp8e4m3 / nvfp4`.
3. **Play row** — большая Play-кнопка с pulse-анимацией во время работы,
   статус-строка (`Распознавание…` / `Ошибка: ...`), output-port-dot.
4. **Output** — `MultilineTextEdit` (auto-height, max_rows=8). Пользователь
   может редактировать текст; правки уходят downstream через тот же
   `output_text`.

## Runtime (`NodeRuntime::AsrGigaam`)

```rust
AsrGigaam {
    model_path:    RwSignal<Option<PathBuf>>,
    device_idx:    RwSignal<usize>,
    storage_idx:   RwSignal<usize>,
    compute_idx:   RwSignal<usize>,
    transcriber:   Arc<Mutex<Option<synaptix::facade::asr::Transcriber>>>,
    loaded_cfg:    Arc<Mutex<Option<AsrLoadedCfg>>>,
    running:       RwSignal<bool>,
    error:         RwSignal<Option<String>>,
    loaded_name:   RwSignal<Option<String>>,
    output_text:   RwSignal<String>,
    text_version:  RwSignal<u32>,
    last_input_kind: PrevSource,
}
```

`AsrLoadedCfg = { model_path, device_idx, storage_idx, compute_idx }` —
снимок настроек, с которыми `Transcriber` сейчас в памяти. При Play
сравниваем с текущими; если изменилось — модель перезагружается.

## Поток данных

```
[Audio in] ──▶ AsrGigaamExec.evaluate
                  └─▶ output_text.get() → PortValue::Text(...) на "out"

[Play btn] ──▶ click → snapshot cfg + current_input_audio(ctx, node_id)
                  └─▶ thread::spawn:
                          1. load model if needed (Transcriber::load)
                          2. downmix_to_mono → transcribe_pcm_text(&pcm, sr, None)
                          3. RwSignal::set(text), text_version.update(|v| ...)
                             running.set(false)
```

- `text_version` bump'ится **только** после транскрибации → `Reactive`
  пересоздаёт `MultilineTextEdit` c новым initial-text. На user-edit
  `text_version` не меняется → курсор не прыгает.
- Модель кэшируется в `transcriber: Arc<Mutex<Option<Transcriber>>>` до
  смены `.syn`/device/dtype или удаления ноды.
- `current_input_audio` отвечает понятной ошибкой если: нет связи, нет
  значения, на входе `AudioStream` (live ещё не поддерживается).

## Файлы (что изменилось)

- `app/synthos/src/pages/node_editor/types.rs` — `PortKind::Text`,
  `PortValue::Text(String)`, `NodeKind::AsrGigaam`, `NodeRuntime::AsrGigaam`,
  `AsrLoadedCfg`, Debug+PartialEq.
- `app/synthos/src/pages/node_editor/registry.rs` — `NodeCategory::Neuro`,
  `ORDER`/`label`/`icon`, `ASR_GIGAAM_EXEC`, schemas, `ASR_GIGAAM` meta,
  `REGISTRY`, `default_runtime` branch.
- `app/synthos/src/pages/node_editor/wires.rs` — `PortKind::Text` цвет
  `#EAB308` (yellow-500).
- `app/synthos/src/pages/node_editor/node_view.rs` — `value_display`
  ветка для `PortValue::Text`.
- `app/synthos/src/pages/node_editor/template_preview.rs` —
  `node_color` для `NodeKind::AsrGigaam` (orchid `#C775EB`).
- `app/synthos/src/pages/node_editor/nodes/asr_gigaam.rs` — new (~470 LOC).
- `app/synthos/src/pages/node_editor/nodes/mod.rs` — `pub mod asr_gigaam`.
- `app/synthos/styles/components/asr_node.mss` — новые MSS-классы.
- `app/synthos/src/styles.rs` — include `asr_node.mss`.

## MSS-классы

| Класс                    | Где / что                             |
|--------------------------|---------------------------------------|
| `asr-gigaam-host`        | хост-DecoratedBox body                |
| `asr-node-side-pad`      | пустые 10px-spacer'ы                  |
| `asr-node-pick-btn`      | rfd-кнопка                            |
| `asr-node-filename`      | имя выбранного `.syn`                 |
| `asr-node-empty`         | placeholder «Модель не выбрана»       |
| `asr-node-badge`         | зелёный `✓ <model_name>` бадж          |
| `asr-node-dd-label`      | uppercase подпись `device/storage/compute` |
| `asr-node-dd-device`     | dropdown устройств                    |
| `asr-node-dd-storage`    | dropdown storage dtype                |
| `asr-node-dd-compute`    | dropdown compute dtype                |
| `asr-node-play`          | Play-кнопка                           |
| `asr-node-running`       | модификатор: pulse-анимация при работе |
| `asr-node-status-running`| статус-текст «Распознавание…»         |
| `asr-node-error`         | красный текст ошибки                  |
| `asr-node-editor-host`   | контейнер MultilineTextEdit с focus-обводкой |
| `asr-node-output`        | сам MultilineTextEdit                  |

## Ограничения / задел на будущее

- **Live-stream ASR**: вход `PortValue::AudioStream(...)` пока возвращает
  ошибку «Live-stream не поддерживается». Шаблон расширения — использовать
  `synaptix::facade::asr::stream::StreamingAsr` (sliding-window 30s / step 1.5s).
- **Сериализация шаблонов**: `model_path: PathBuf` → надо будет
  `to_string_lossy()` при save-graph / `serde(skip)`. Сейчас сохранение
  графа пока не поддерживает AsrGigaam.
- **Языковой dropdown**: GigaAM моноязычная (RU), language hardcoded "ru".
  Когда добавим Whisper-multilingual ноду — нужен будет language-select.
- **Прогресс-бар**: модель загружается ~секунды/десятки секунд. UI
  сейчас показывает только pulse-анимацию + статус-текст. Долго —
  добавить indeterminate-progress.
