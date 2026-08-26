# VibeVoice — нода многоголосого синтеза (synthos, 2026-08-24)

`NodeKind::VibeVoice` — TTS-нода на нативном synaptix-порте **VibeVoice**
(Qwen2-LM + акустический/семантический σ-VAE + диффузионная голова).
Сценарий вида `Speaker N: текст` + до четырёх референс-голосов → сплошная
дорожка диалога 24 кГц.

Отличие от `VoxCpm2`/`OmniVoice`: те озвучивают одну реплику одним голосом,
VibeVoice держит контекст на десятки минут и сам переключает голоса между
репликами внутри одного прогона.

## Регистрация

- `pages/node_editor/types.rs` — `NodeKind::VibeVoice` (входит в
  `NodeKind::ALL`), `NodeRuntime::VibeVoice`, `VibeVoiceLoadedCfg`.
- `pages/node_editor/registry.rs` — `VIBEVOICE` (`NodeCategory::Neuro`,
  подкатегория «Синтез речи», иконка `MI_CAMPAIGN`), `VIBEVOICE_EXEC`,
  ветка в `default_runtime`.
- `pages/node_editor/nodes/vibevoice.rs` — body, executor, воркер.
- `pages/node_editor/node_view.rs` — класс `.vibevoice-node`.
- `styles/components/vibevoice_node.mss` (ширина карточки 400px + пульс
  `.vibevoice-node-running`), смонтирован в `src/styles.rs`.
- `templates/model.rs` / `templates/convert.rs` — `VibeVoiceStateData`
  (сохранение настроек в шаблонах и workspace).
- `agent/tools/pipelines.rs` — `enum_hints` для `device_idx`/`compute_idx`;
  остальное агент получает автоматически из `registry::REGISTRY`.

## Порты

| Сторона | Имя | Тип | Назначение |
|---|---|---|---|
| Вход | `model` | `Data` | Хэндл Syn Checkpoint (перебивает поля body). |
| Вход | `script` | `Text` | Сценарий; пусто → берётся textfield из body. |
| Вход | `voice1..voice4` | `Audio` | Референсы голосов: порядок = номер спикера. |
| Выход | `audio` | `Audio` | PCM 24 кГц mono. |

Голоса собираются по порядку до первого неподключённого порта: чтобы задать
голос второму спикеру, первый порт тоже должен быть занят.

Сценарий прогоняется через `plain_text_to_script`: строки без префикса
`Speaker N:` считаются репликами первого спикера, так что нода принимает и
обычный текст.

## Body

1. **Модель** — `.syn` picker (`vibevoice-1.5b.syn` / `vibevoice-7b.syn`).
2. **Device** — `CUDA` / `CPU`.
3. **Compute** — `bf16` / `f16` / `f32`.
4. **Сценарий** — `MultilineTextEdit`, fallback для порта `script`.
5. **CFG** — 0.5–3.0, дефолт 1.3 (guidance диффузионной головы).
6. **Steps** — 5–50, дефолт 20 (шаги DPM-Solver++ на кадр).
7. **Длина ×** — 1.0–6.0, дефолт 2.0: потолок числа шагов = множитель ×
   длина промпта. Если дорожка обрывается раньше конца сценария — поднять.
8. **Seed** — u64, дефолт 0.
9. **Статус** — `Загрузка модели…` → `Синтезирование… N%` → имя бандла /
   `Ошибка: …`. Процент приходит из колбэка `on_step` пайплайна.

## Поток данных

`start` (UI-поток) снимает сигналы, резолвит Syn Checkpoint (device/compute
из хэндла перебивают поля ноды), читает сценарий и голоса с портов и
запускает `synthos-vibevoice-worker`.

Воркер: сравнивает `VibeVoiceLoadedCfg` с загруженным, при расхождении
пересоздаёт `VibeVoicePipeline::from_syn` и регистрирует слот в панели
моделей (`TTS/VibeVoice`), затем `synthesize_with` с колбэком прогресса.
Результат кладётся в `output_buf`, `output_version` бампается через
`run_on_main_thread` (RwSignal нельзя трогать из воркера).

`VibeVoiceExec::evaluate` отдаёт содержимое `output_buf` в порт `audio`,
подписавшись на `output_version`.

Если у Syn Checkpoint выключено «Держать в памяти», слот очищается сразу
после прогона и VRAM возвращается (`crate::models::trim_device`).

## Встроенные пайплайны

`templates/builtin.rs`:

- **VibeVoice: диалог двух голосов** (`builtin-vibevoice-dialogue`) —
  Text View (сценарий) + два Audio File + Syn Checkpoint → VibeVoice →
  Audio Player и Save to File.
- **VibeVoice: клон одного голоса** (`builtin-vibevoice-single`) — один
  образец голоса + длинный текст.
- **VibeVoice: подкаст из темы (LLM → голос)**
  (`builtin-vibevoice-llm-podcast`) — тема → LLM пишет сценарий строками
  `Speaker N:` → Text View (правится вручную) → VibeVoice двумя голосами →
  плеер и файл. Нужны два Syn Checkpoint: LLM-бандл и `vibevoice-*.syn`.

## Бандлы

- `/path/to/syn_models/vibevoice-1.5b.syn` — 5.4 ГБ.
- `/path/to/syn_models/vibevoice-7b.syn` — 18.7 ГБ.
- `vibevoice-large.syn` — симлинк на 7B (веса microsoft/VibeVoice-Large и
  microsoft/VibeVoice-7B побайтово идентичны).

Внутри бандла: `config.json`, `preprocessor_config.json`, `tokenizer.json`
(Qwen2.5 — в HF-снапшоте VibeVoice его нет), компонент тензоров `main`.

## Скорость

RTX 5090 Laptop, compute `bf16`: RTF **0.214** у 1.5B и **0.649** у 7B —
генерация быстрее реального времени на обеих моделях. На `f32` — 1.68 (в
бэкенде нет fused-GEMM для f32), так что держать `bf16`.

## Ограничения

- Стриминг в плеер по мере генерации не подключён: нода публикует готовый
  буфер (колбэк `on_chunk` в пайплайне есть).
- Батч 1: один сценарий за прогон.
