# Qwen-Image-Edit и Qwen-Image-Edit-2511 (2026-09)

Правка картинки по инструкции: исходник видит и энкодер (Qwen2.5-VL читает
картинку вместе с промптом), и DiT (латент VAE идёт в ту же
последовательность токенов, что и генерируемая картинка).

## Что лежит в `~/Storage/syn_models`

| бандл | размер | исходник | пайплайн |
|---|---|---|---|
| `qwen-image-edit-2511.syn` | 57,7 ГБ | `Qwen-Image-Edit-2511/` (HF `Qwen/Qwen-Image-Edit-2511`) | `QwenImageEditPlusPipeline`, `zero_cond_t` |
| `qwen-image-edit.syn` | 57,7 ГБ | `Qwen-Image-Edit/` (HF `Qwen/Qwen-Image-Edit`, 08.2025) | `QwenImageEditPipeline` |

Оба — плотный BF16, `arch qwen-image-edit`, `purpose image`, лицензия Apache 2.0.
Энкодер (Qwen2.5-VL 7B) и VAE у двух выпусков байт в байт одинаковые (LFS-хэши
совпадают): скачаны один раз, в `Qwen-Image-Edit/` — жёсткие ссылки на файлы
`Qwen-Image-Edit-2511/`. SHA-256 всех весов обоих каталогов сверены с `lfs.oid`
HF. Квант NVFP4/MXFP8 делается при загрузке DiT.

Qwen-Image-Edit-2509 — тот же пайплайн, что у 2511, без `zero_cond_t`:
поддержан кодом (вариант `EditPlus` берётся из `model_index.json`, флаг — из
конфига трансформера), но не скачан.

```sh
cd ~/Storage/syn_models
~/Projects/2027/synaptix/target/release/syn-pack Qwen-Image-Edit-2511 -o qwen-image-edit-2511.syn \
  --id qwen-image-edit-2511 --version 2511.0.0 --arch qwen-image-edit --purpose image
syn-verify qwen-image-edit-2511.syn      # CRC32C всех 25 чанков — ок (у обоих)
QWEN_DIR=… QWEN_SYN=… cargo test --release -p synaptix-image-qwen \
  --test bundle_source -- --ignored --nocapture   # 2856 тензоров байт в байт
```

## synaptix: `synaptix-image-qwen`

- **Энкодер** (`text_encoder.rs`, `vision.rs`): Qwen2.5-VL 7B. Башня зрения
  (32 блока, окна 8×8 патчей, полное внимание в блоках 7/15/23/31, 2D-RoPE,
  слияние 2×2) считается в F32 — в BF16 её 7 % ошибки LLM размножает до
  косинуса 0,91. Голова 80 дополняется нулями до 128 для flash-ядра. LLM — BF16,
  слои, не влезшие в VRAM, читаются из mmap. Позиции — **M-RoPE** (текст подряд,
  токены картинки — (t, h, w)), выход — последнее скрытое состояние без 64
  токенов системной части шаблона.
- **Препроцессинг** (`preprocess.rs`) повторяет пиксели бит в бит: `VaeImageProcessor`
  — Lanczos PIL (веса ×2²²), процессор Qwen2-VL — бикубика torchvision на uint8
  (int16-веса, точность — наибольшая, при которой вес < 2¹⁵). Башня зрения очень
  чувствительна: 0,7 % разницы пикселей давали 20 % разницы эмбеддингов.
- **DiT** (`transformer.rs`): 60 double-stream блоков, у каждого своя модуляция —
  её веса (40 % блока) не лежат в VRAM, а модуляции всех шагов считаются один раз
  на расписание (`ModTable`). `zero_cond_t` (2511): токены картинок-референсов
  модулируются временем 0. RoPE центрирован по высоте/ширине, референсы — кадры
  1, 2, …
- **MXFP8**: GEMM с выходом в F16, а остаточный поток Qwen-Image доходит до
  ~2·10⁹ — вход приводится к максимуму 1, а не 64 (`QuantLinear::forward_target`),
  иначе 59-й блок давал inf (белая картинка).
- **VAE** (`vae.rs`): каузальный 3D-VAE Wan 2.1 в режиме одного кадра — 2D-свёртки
  из последнего временного среза ядра, RMS-нормы по каналам. F32.
- Расписание: sigmas `linspace(1, 1/N)`, экспоненциальный сдвиг по длине латента,
  растяжка хвоста к 0,02. true CFG с перенормировкой `comb·‖cond‖/‖comb‖`.

**Сверка с diffusers 0.40** (`scripts/reference/gen_qwen_image_edit.py`, CPU F32;
`tests/reference.rs`): VAE — cos 1,000000 (max 0); пиксели башни — бит в бит;
башня F32 — cos 1,000000; энкодер F32 на эталонных эмбеддингах картинки — cos
1,000000, весь путь в BF16 — 0,9955; DiT (3 блока) BF16 — 0,99996, с `zero_cond_t` —
0,99982, MXFP8 — 0,99938, NVFP4 — 0,9986; sigmas совпадают.

**Грабли эталона.** diffusers 0.40 на transformers 5.x не передаёт энкодеру
`mm_token_type_ids`, и transformers 5.12 тогда нумерует токены картинки
**линейно**, без M-RoPE (косинус к правильному выходу 0,845). Модель обучалась на
transformers 4.5x, где 3D-позиции считались по `input_ids`, — мы делаем так же;
эталон с M-RoPE (`prompt_embeds_mrope`) скрипт снимает прямым вызовом энкодера с `mm_token_type_ids`.
Для башни эталону нужен torchvision (иначе процессор не грузится).

## Замеры (RTX 5090 24 ГБ, 1024² + картинка 1024², CFG 4)

| | шаги | время денойза | пик VRAM |
|---|---|---|---|
| 2511 MXFP8 | 40 | 188 с | 17,8 ГБ |
| 2511 NVFP4 | 40 | 155 с | 13,2 ГБ |
| 2511 MXFP8, две картинки | 40 | 336 с | 19,0 ГБ |
| Edit (08.2025) NVFP4 | 50 | 219 с | 13,4 ГБ |
| 2511 MXFP8 на «карте 7 ГБ» | 8 | 38 с | 6,15 ГБ (15/60 блоков на карте) |

Энкодер: 2,3 с (5 с на две картинки; у старого Edit картинка ~1 Мп — 1369 токенов).
Проход DiT на 8,4 тыс. токенов — 1,9 с (было 4,7 с: скалярный broadcast шёл через
generic-ядро, а `max|x|` для подготовки входа квант-GEMM — через общую
редукцию; оба исправлены в ядре synaptix побитово эквивалентно — FLUX.2 даёт те
же PSNR 40,00/26,80 дБ). Треть оставшегося — flash-внимание.

## synthos: ноды Нейро → Qwen-Image

**Checkpoint** (бандл/каталог, устройство, веса DiT `nvfp4 | mxfp8 | dense (bf16)`,
по умолчанию mxfp8, память `auto | resident | block_offload`, «Держать в памяти»),
**Reference** (картинка → латент VAE ~1 Мп + исходник; цепочкой, до 4 у 2509/2511,
одна у Edit), **Text Encoder** (промпт, `negative` — по умолчанию `" "`, `references`
— те же картинки; повтор с тем же промптом и картинками берёт готовое),
**Sampler** (шаги 0 = по модели: Edit 50, 2509/2511 40; CFG 4, ≤ 1 — без CFG;
размер — FLUX Empty Latent или ~1 Мп по картинке; прогресс, отмена), **VAE Decode**.
Латент на проводе свой (`QwenImageBlob::Latent`) — ноды FLUX его не примут.

Шаблоны «Картинки»: **Qwen-Image: Edit Image**, **Qwen-Image: Multi-Image Edit**
(две картинки цепочкой, только 2509/2511). Агент: `qwen_image_prompt.md`
(инструкция «что изменить и что оставить», Picture 1/2), расшифровка `*_idx`.

Смоук `src/bin/qwen_image_smoke.rs` — настоящие `on_run` нод:
`qwen_image_smoke <бандл|каталог> <картинка> <out.png> [шаги [w h]]`,
env `QWEN_IMAGE2`, `QWEN_QUANT`, `QWEN_CFG`, `QWEN_VRAM_GB` (балласт «малой карты»).
