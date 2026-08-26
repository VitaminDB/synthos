# LTX-2.3 в `.syn`-бандлах (2026-08)

`LTX Checkpoint` принимает `.syn`-бандлы для всех четырёх путей — чекпойнт,
Gemma-энкодер, spatial-upscaler и LoRA. Сырые `.safetensors` и HF-каталог
Gemma продолжают работать: раскладка определяется по расширению пути, а не
настройкой.

## Почему бандлы

До этого LTX был единственным семейством, чьи веса жили вне каталога моделей
(`Настройки → AI-модели`): чекпойнт — отдельный `.safetensors`, Gemma — HF-
каталог из кэша HF-загрузок. Агент Syn-чата берёт пути из инвентаря
`pipelines list` и, не видя LTX там, честно докладывал «LTX-моделей в каталоге
нет» — и уходил на MiniMax-H3. Один `.syn` на модель кладёт LTX в тот же
каталог, что H3/ACE-Step/Qwen, и делает его видимым и для агента, и для
пикеров ноды.

## Раскладка

Каждая модель — отдельный бандл (поля ноды независимы: dev и distilled
взаимозаменяемы, upscaler общий, LoRA опциональна):

| Бандл | Источник | Размер |
|---|---|---|
| `ltx-2.3-22b-dev.syn` | `ltx-2.3-22b-dev.safetensors` | 46.1 ГБ |
| `ltx-2.3-22b-distilled-1.1.syn` | `ltx-2.3-22b-distilled-1.1.safetensors` | 46.1 ГБ |
| `ltx-2.3-spatial-upscaler-x2-1.1.syn` | одноимённый `.safetensors` | 1.0 ГБ |
| `ltx-2.3-22b-distilled-lora-384-1.1.syn` | одноимённый `.safetensors` | 7.6 ГБ |
| `ltx-2.3-22b-ic-lora-*.syn` | lipdub / motion-track / union-control | 0.3–2.5 ГБ |
| `ltx-gemma-3-12b-qat.syn` | HF-каталог `gemma-3-12b-qat` (5 шардов) | 24.4 ГБ |

Однофайловые модели пакуются fast-path'ом `syn-pack` — побайтовая копия
потока, поэтому `__metadata__["config"]` (конфиг transformer/vae/scheduler)
доезжает до `Ltx23Config::from_metadata` как есть. Gemma — merge-путь (5
шардов в один tensors-чанк); конфиг и токенайзер лежат файловыми чанками
(`config.json`, `tokenizer.json`, …) и читаются `Bundle::read_file`.

## Упаковка

```sh
PACK=~/Projects/2027/synaptix/target/release/syn-pack
SRC=~/models/ltx2.3_v1.1
OUT=/path/to/syn_models
STAGE=~/models/.syn_pack_staging      # hardlink'и, та же ФС — копий нет

mkdir -p "$STAGE/ltx-2.3-22b-dev"
ln "$SRC/ltx-2.3-22b-dev.safetensors" "$STAGE/ltx-2.3-22b-dev/model.safetensors"
"$PACK" "$STAGE/ltx-2.3-22b-dev" -o "$OUT/ltx-2.3-22b-dev.syn" \
  --id ltx-2.3-22b-dev --version 1.1.0 --arch ltx-2.3 --purpose video

# Gemma — HF-каталог целиком, staging не нужен
"$PACK" ~/models/gemma-3-12b-qat -o "$OUT/ltx-gemma-3-12b-qat.syn" \
  --id ltx-gemma-3-12b-qat --version 1.0.0 --arch gemma3 --purpose text-encoder
```

`syn-pack` берёт `.safetensors` из каталога, поэтому одиночный файл заводится
через staging-каталог с hardlink'ом `model.safetensors` — так срабатывает
fast-path (один шард, без префикса) и бандл получается побайтовой копией.

## Загрузка

| Что | Где | `.syn` |
|---|---|---|
| Чекпойнт | `LtxCheckpoint::open` | `SafetensorsLoader::open_bundle(path, None)` |
| LoRA | `LoraWeights::open` | то же (общий `loader::open_weights`) |
| Upscaler | `Upsampler::load` | то же |
| Gemma | `GemmaWeights::load` | `Bundle` + `SafetensorsLoader::from_bundle` |
| Токенайзер Gemma | `pipeline::load_tokenizer` | `read_aux` → `HfTokenizer::from_bytes` |

Все пути zero-copy: тензоры отдаются прямо из mmap бандла, распаковки во
временный файл нет.

Проверка на локальных бандлах — ignore-тесты:

```sh
SYN_LTX_BUNDLE=$OUT/ltx-2.3-22b-dev.syn \
  cargo test -p synaptix-video-ltx23 --lib -- --ignored bundle_open
SYN_GEMMA_BUNDLE=$OUT/ltx-gemma-3-12b-qat.syn \
  cargo test -p synaptix-llm-gemma3 --lib -- --ignored bundle_load
```

## Нода

Строки пикеров `LTX Checkpoint`: «Чекпойнт» (`.syn` | `.safetensors`),
«Gemma» (`.syn`), «Gemma HF-каталог» (старый путь — как у `SynCheckpoint`),
«Upscaler» и «LoRA» (`.syn` | `.safetensors`). Значение поля одно и то же —
меняется только способ выбора.
