# Единый checkpoint-стиль всех пайплайнов + резидентность (2026-08-20)

Все семейства нод теперь работают через чекпойнт-ноду (ComfyUI-стиль:
нода-источник публикует конфиг-хэндл в порт `model`, веса грузит
потребитель через weak-кэш):

| Семейство | Чекпойнт | «Держать в памяти» |
|---|---|---|
| LTX-2.3 | LtxCheckpoint | да: hold DiT переживает VAE-decode (на 24 ГБ может не влезть — опт-ин) |
| MiniMax-H3 | H3Checkpoint | да: hold DiT переживает decode |
| ACE-Step | AceStepCheckpoint | да: MusicComponentCache в generate_music (LM/TE/DiT/VAE между прогонами) |
| LLM / VoxCPM2 / OmniVoice / ASR GigaAM / Sortformer | **SynCheckpoint** (новая) | да: выкл → слот очищается после прогона |

## SynCheckpoint (`nodes/syn_checkpoint.rs`)

Универсальный источник модели для слот-семейств: `.syn`-бандл или
HF-каталог + предпочтения Device (Auto/CUDA/CPU), Storage
(Auto/F16/BF16/FP8/NVFP4), Compute (Auto/F16/BF16/F32) + чекбокс
«Держать в памяти». Публикует `SynModelHandle` (POD, `handle_cache` для
стабильного ptr_eq) в порт `model`.

Потребители (у всех появился вход `model`) маппят предпочтения на свои
family-опции (`map_handle_*` в каждом узле; Auto → разумный дефолт:
GPU у ASR/TTS, дефолтные dtype семейства). Хэндл переопределяет
собственные поля ноды; **без подключённого хэндла нода работает от своих
полей — старые графы и шаблоны не ломаются**.

## Резидентность (чекбокс «Держать в памяти»)

- ACE-Step: движковый `MusicComponentCache` (synaptix) — владеет
  Generate-нода (`resident_cache()`-слот, виден в панели «Модели в
  памяти» как «ACE-Step · Resident»); ключ валидности — пути бандлов +
  device/dtype'ы, LM перезагружается при нехватке rope-ёмкости. Без
  чекбокса — прежний sequential-drop под 24 ГБ.
- Слот-семейства: по умолчанию ON (исторически слот жил всегда).
  OFF → после прогона воркер очищает слот (pipeline + loaded_cfg) и
  трим-ит пул — VRAM возвращается. Полезно перед тяжёлым видео-прогоном.
- LTX/H3: по умолчанию OFF (прежнее weak-поведение). ON → decode-ноды не
  вызывают `release_avdit_hold`/`release_dit_hold`, DiT остаётся в VRAM и
  следующий прогон не платит реквантизацию. Поле `resident` в ключи
  weak-кэшей не входит.
- Пре-чек агентского прогона (`pipeline_run::prepare`) не требует
  `model_path` у нод с подключённым входом `model`.

## Прочее из этого раунда

- Depth Anything V2 (IC-LoRA control) — weak-кэш в `ltx/shared.rs`
  (раньше грузился заново на каждый прогон).
- Встроенные voice-шаблоны (VoxCPM/OmniVoice Voice Clone) пересобраны на
  SynCheckpoint-ноды.
- Схема нового узла автоматически видна агенту (`pipelines action=nodes
  filter=syn_checkpoint` — пример state-JSON с дефолтами).

## Добавление нового слот-семейства (конвенция)

1. Вход `model:Data` первым в inputs registry.
2. В `start()`: `super::current_input_syn_model(ctx, node.id)` → cfg из
   хэндла (`map_handle_*`) или собственных полей; `resident` из хэндла.
3. В воркере после финала: `if !resident` → очистить слот + trim.
