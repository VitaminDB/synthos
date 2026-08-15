# MiniMax-H3 в `.syn`-бандлах (2026-08)

H3-нода `H3 Checkpoint` больше не берёт HF-каталог: модель и энкодер выбираются
как одиночные `.syn`-файлы. Веса читаются zero-copy из mmap бандла — без
распаковки во временный каталог и без промежуточных копий в RAM.

## Раскладка бандла

Бандл зеркалит дерево HF-каталога варианта (`MiniMax-H3/FL2VA`), поэтому
загрузчик читает конфиги по одним и тем же относительным путям независимо от
источника:

| Чанк | Источник | Размер |
|---|---|---|
| `tensors:transformer` | `transformer/*.safetensors` (13 шардов) | 66.3 ГБ |
| `tensors:video_vae` | `video_vae/source/model.safetensors` | 10.4 ГБ |
| `tensors:audio_vae` | `audio_vae/model.safetensors` | 0.6 ГБ |
| `tensors:text_encoder` | `text_encoder/*.safetensors` — опционально | 66.7 ГБ |

Вспомогательные файлы лежат под своими относительными путями:
`model_index.json`, `transformer/config.json`, `video_vae/config.json`,
`video_vae/source/config.json`, `audio_vae/config.json`,
`audio_vae/metadata.json`, `tokenizer/*`, `processor/*`.

Вариант партиции (FL2VA / Ref2VA) читается из `model_index.json`
(`_minimax_h3.partition`), так что дропдаун «Вариант» для бандла — лишь fallback.

## Энкодер

Qwen3-VL-32B (66.7 ГБ) вынесен в **отдельный** бандл: он переиспользуется между
партициями и его удобно менять независимо от DiT. Поле «Энкодер» в ноде
принимает `.syn`; если оставить его пустым, энкодер ищется внутри бандла модели
(компонент `text_encoder` + файлы `text_encoder/config.json`,
`text_encoder/tokenizer.json`) — так работает вариант «всё в одном файле».

## Упаковка

```sh
SRC=~/models/MiniMax-H3/FL2VA
OUT=/run/media/storage/syn_models
PACK=~/Projects/2027/synaptix/target/release/syn-pack

# модель: DiT + оба VAE + конфиги/токенайзер (77 ГБ)
$PACK -o $OUT/minimax-h3-fl2va.syn \
  --id minimax-h3-fl2va --version 1.0.0 --arch minimax-h3 --purpose video \
  --component transformer:$SRC/transformer \
  --component video_vae:$SRC/video_vae/source \
  --component audio_vae:$SRC/audio_vae \
  --files $SRC

# энкодер (67 ГБ)
$PACK $SRC/text_encoder -o $OUT/minimax-h3-qwen3vl-encoder.syn \
  --id minimax-h3-text-encoder --version 1.0.0 --arch qwen3-vl --purpose text-encoder
```

`syn-pack` пишет промежуточный `*.tensors_stage0.tmp` рядом с выходом — на время
упаковки нужно примерно 2× места под самый крупный компонент.

Чтобы положить энкодер внутрь модельного бандла, добавьте к первой команде
`--component text_encoder:$SRC/text_encoder` (итоговый файл ~144 ГБ).

## Проверка

```sh
cargo run --release -p synaptix-video-minimax-h3 --example inspect_source -- \
  $OUT/minimax-h3-fl2va.syn $OUT/minimax-h3-qwen3vl-encoder.syn
```

Печатает вариант, число тензоров и объём каждого компонента, разобранные
конфиги DiT/VAE/audio-VAE и размер `config.json`/`tokenizer.json` энкодера.
Тот же вызов с путём HF-каталога даёт эталонные значения для сравнения.

## Что изменилось в коде

- `synaptix-io`: `SafetensorsLoader::open_bundle` / `from_bundle` — индексация
  safetensors-потока прямо в mmap бандла (шард владеет `Arc<Bundle>`).
- `synaptix-video-minimax-h3`: `H3Source` (каталог | бандл) и `H3EncoderSource`;
  `H3Config`/`VaeConfig`/`AudioVaeConfig::from_source`,
  `H3Checkpoint::open_source`, `ComponentLoader::open_component`.
- `synaptix-vlm-qwen3`: `H3Encoder::from_parts` — сборка башен из уже
  прочитанных `config.json`/`tokenizer.json` и произвольного источника весов.
- `synthos`: `H3ModelHandle { model_path, encoder_path }`, `.syn`-пикеры в ноде,
  состояние чекпойнта сохраняется в workspace (`H3CheckpointStateData`).

HF-каталог продолжает работать: `H3Source::open` выбирает ветку по типу пути,
поэтому `h3_smoke <каталог>` и старые графы остаются рабочими.
