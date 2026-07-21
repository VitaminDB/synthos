# Persistence node-editor'а (2026-05-13)

Документ описывает два слоя сохранения в node-editor synthos:

1. **Workspace autosave** — все открытые вкладки и их содержимое восстанавливаются между запусками приложения через `~/.config/synthos/workspace.json`.
2. **NodeStateData в шаблонах** — per-kind runtime-state ноды (модель, файл, текст, sliders) сохраняется в шаблонах графа (`~/.config/synthos/templates/*.json`).

Оба слоя используют общую сериализуемую модель (`NodeData`/`ConnData`/`ViewportData`/`NodeStateData` из `templates::model`).

---

## 1. Workspace autosave / restore

**Файл:** `~/.config/synthos/workspace.json`

Содержит сессию редактора нод: список открытых вкладок, активную, открытость templates-панели, auto-increment id вкладок. Каждая вкладка хранит снимок графа (nodes + connections + viewport).

**Сохранение:** `install_workspace_autosave` в `app/synthos/src/lib.rs` — `create_effect`, подписанный на `EditorWorkspace.tabs/active/templates_open` + все per-tab сигналы (через `persist::subscribe_tab_signals`). На любое изменение пересобирает `WorkspaceState` через `templates::convert::snapshot` и атомарно пишет JSON (через `.tmp` + rename). Защита от дубль-записи — `workspace_fingerprint` (hash JSON-байтов): если состояние идентично последнему сохранённому — skip.

**Восстановление:** `EditorWorkspace::new_or_restore` в `tabs.rs`:
- Читает workspace.json через `persist::load` (None при отсутствии файла / битом JSON).
- При успехе строит вкладки через `make_tab_from_state` — каждая создаёт fresh `NodeEditorCtx`, очищает стартовую Demo-ноду, применяет сохранённый граф через `templates::convert::apply_to_ctx` (это тот же путь, по которому грузятся шаблоны, поэтому все `NodeStateData` корректно проставляются в свежий runtime).
- При отсутствии файла — fallback на `EditorWorkspace::new()` (одна Untitled-вкладка с Demo-нодой).

**Что НЕ восстанавливается:**
- Тяжёлые объекты (`Transcriber`, `OmniVoicePipeline`, `AudioPlayer`, `RecordingSession`, thread-handle'ы, audio-stream-receiver'ы, biquad'ы DSP-эффектов) — пересоздаются дефолтно через `registry::default_runtime`, грузятся лениво на первый Play.
- Транспорт-state аудио (is_playing / is_paused / progress) — плеер всегда стартует со Stopped.
- Buffer-кэши offline-обработки — пересчитываются на первый evaluate.

**Файлы:**
- `app/synthos/src/pages/node_editor/persist.rs` — `WorkspaceState`/`TabState` (serde) + `save`/`load`/`workspace_path` + `subscribe_tab_signals` (reactive-subscribe для autosave-эффекта).
- `app/synthos/src/pages/node_editor/tabs.rs` — `EditorWorkspace::new_or_restore` / `from_state` / `make_tab_from_state`.
- `app/synthos/src/lib.rs` — `install_workspace_autosave` + замена `EditorWorkspace::new()` на `new_or_restore` в `run`-closure.

**Тесты:**
- `pages::node_editor::persist::tests::empty_workspace_roundtrip`
- `pages::node_editor::persist::tests::tab_with_node_roundtrip`
- `pages::node_editor::persist::tests::parse_legacy_empty_file_defaults_in`
- `pages::node_editor::persist::tests::save_load_restore_full_flow` — полный path: save на disk → load → `EditorWorkspace::from_state` → проверка восстановленных вкладок и runtime ноды.

---

## 2. Сохранение per-node runtime-state в шаблонах графа

## Контекст

Шаблоны графа нод (`~/.config/synthos/templates/*.json`) до этой точки сохраняли только `fields: HashMap<&'static str, FieldValue>` каждой ноды — значения из `NodeKindMeta.fields`. Реальные «настройки» нод (выбранный `.syn`, dropdown'ы device/storage/compute, содержимое markdown'а, gain'ы EQ/Mixer, текст транскрибата) живут в `NodeRuntime`-варианте и при save/load терялись.

Эта правка расширяет JSON-схему шаблонов: каждая нода теперь дополнительно хранит per-kind `NodeStateData`-блок, зеркалящий runtime-сигналы пользовательского state.

## Архитектура

### Сериализуемая модель (`templates/model.rs`)

```text
NodeData
├── id, kind, pos
├── fields       BTreeMap<String, FieldValueData>  (как раньше)
├── style        NodeStyleData                     (как раньше)
├── enabled      bool                              (как раньше)
└── state        Option<NodeStateData>             ← НОВОЕ
```

`NodeStateData` — `#[serde(tag = "kind", content = "data")]` enum:

```text
NodeStateData
├── AsrGigaam(AsrGigaamStateData)       — model_path, device_idx, storage_idx, compute_idx, output_text
├── OmniVoice(OmniVoiceStateData)       — model_path, idx-tuples, instruct, ref_text, language, sampling params
├── MarkdownView(MarkdownViewStateData) — content, width, height, edit_mode
├── TextView(TextViewStateData)         — output_text, width, height
├── Gain(GainStateData)                 — gain_db
├── Filter(FilterStateData)             — mode (FilterMode), cutoff_hz
├── Reverb(ReverbStateData)             — mix, room
├── Equalizer(EqualizerStateData)       — gains_db: Vec<f32>
├── Mixer(MixerStateData)               — n_inputs, gains_db: Vec<f32>
├── AudioPlayer(AudioPlayerStateData)   — volume
├── AudioRecorder(AudioRecorderStateData) — device: Option<String>
└── SaveToFile(SaveToFileStateData)     — path
```

Все поля помечены `#[serde(default)]` — частичные шаблоны парсятся, отсутствующие ключи дают `Default`. Само поле `NodeData.state` — `Option<...> + #[serde(default)]`, поэтому старые шаблоны без `state` загружаются без ошибок (graceful upgrade).

### Конвертация (`templates/convert.rs`)

Два symmetry-helper'а:

- `runtime_to_state(&NodeRuntime) -> Option<NodeStateData>` — снимок runtime'а в `NodeStateData` (часть `snapshot()` → `node_to_data()`).
- `apply_state_to_runtime(&NodeRuntime, &NodeStateData)` — применяет state к свежесозданному runtime'у (после `default_runtime(kind)` в `apply_to_ctx`). Несовпадение `kind` ↔ вариант `NodeStateData` — silent skip (forward/backward compat).

### Что НЕ сериализуется

- Тяжёлые ленивые объекты — `Transcriber`, `OmniVoicePipeline`, `AudioPlayer`, `RecordingSession`, biquad'ы — пересоздаются дефолтно, реально грузятся на первый Play.
- Thread-handle'ы, mpsc-receiver'ы, atomic snapshot'ы — restart-ятся executor'ом при первом evaluate.
- AudioBuffer (PCM), AudioStream-receiver, buffer-cache — кэши, не state.
- `last_input_kind`/`last_input` — детектор изменения upstream, инициализируется `None`/`PrevSource::None`.

## Соглашения для будущих нод

При добавлении нового `NodeKind` с runtime-state:

1. В `templates/model.rs`: добавить вариант в `NodeStateData` + per-kind `*StateData` struct с `#[derive(Serialize, Deserialize, Default)]`. Все поля — `#[serde(default)]`, иногда `#[serde(default = "fn")]` для не-zero дефолтов.
2. В `templates/convert.rs::runtime_to_state` и `apply_state_to_runtime`: новые ветки match.
3. `PathBuf` → `Option<String>` через `to_string_lossy()`/`PathBuf::from`. `syngui::core::Color` → hex-строка `#RRGGBB[AA]` (см. `FieldValueData::Color`). `Size`/`Point` → отдельные `f32`-поля.
4. Roundtrip-тест в `templates/convert.rs::tests`.

## Тесты

`templates/convert.rs::tests` (`cargo test -p synthos --lib templates::convert::tests`):

- `roundtrip_asr_gigaam_state`, `roundtrip_omnivoice_state`, `roundtrip_markdown_state`, `roundtrip_filter_state`, `roundtrip_equalizer_state`, `roundtrip_mixer_state` — сериализация → JSON pretty → deserialize → load → проверка runtime значений.
- `snapshot_apply_roundtrip_preserves_state` — ставит state в `ctx_src`, гоняет через `snapshot()`/`apply_to_ctx()`, проверяет `ctx_dst`.
- `legacy_template_without_state_loads_with_defaults` — backward-compat: JSON без поля `state` загружается, нода получает дефолтный runtime.

## Связанные файлы

- `app/synthos/src/templates/model.rs` — JSON-модель.
- `app/synthos/src/templates/convert.rs` — snapshot/apply + roundtrip-тесты.
- `app/synthos/src/templates/builtin.rs` — все builtin-шаблоны проинициализированы `state: None` (state не используется во встроенных пресетах).
- `app/synthos/src/components/templates_panel/mod.rs` — UI «Save current as template» (вызывает `templates::convert::snapshot`), без изменений.
- `app/synthos/src/pages/node_editor/types.rs` — `NodeRuntime` + `FilterMode` (`Serialize`/`Deserialize`).
