# Qwen-Image 2.1 (2026-09)

Одна модель Qwen на три задачи: картинка по тексту, правка по референсам (до
10 картинок) и прозрачные RGBA. Энкодер — Qwen3-VL 8B (видит референсы вместе с
промптом), DiT — 32 single-stream блока (7 млрд параметров), VAE — 16× с
четырьмя каналами. Лицензия — Qwen Research License.

## Бандл

`~/Storage/syn_models/qwen-image-2.1.syn` — 30,9 ГБ, `arch qwen-image-2.1`,
`purpose image`, плотный BF16 (VAE — F32). Исходный каталог
`~/models/Qwen/Qwen-Image-2.1` (HF `Qwen/Qwen-Image-2.1`, 31 ГБ) **оставлен**:
удалять только по решению пользователя. Сверено: `syn-verify` (21 чанк CRC32C),
все 1285 тензоров и 11 вспомогательных файлов байт в байт
(`tests/bundle_source`).

```sh
cd ~/Storage/syn_models
~/Projects/2027/synaptix/target/release/syn-pack ~/models/Qwen/Qwen-Image-2.1 \
  -o qwen-image-2.1.syn --id qwen-image-2.1 --version 2.1.0 --arch qwen-image-2.1 --purpose image
QWEN21_DIR=~/models/Qwen/Qwen-Image-2.1 QWEN21_SYN=~/Storage/syn_models/qwen-image-2.1.syn \
  cargo test --release -p synaptix-image-qwen21 --test bundle_source -- --ignored --nocapture
```

## synaptix: `synaptix-image-qwen21`

Крейт `crates/synaptix-models/image/qwen_image21` (источник, память,
расписание и целочисленный ресэмплинг переиспользуются из
`synaptix-image-qwen`).

- **Картинки** (`image.rs`): RGBA в uint8, как у пайплайна. Ресайз референса —
  Lanczos PIL **с премультипликацией альфы** (`Image.resize` у RGBA идёт
  через RGBa: `MULDIV255` туда, `255·v/a` без округления обратно), композит на
  белом для башни зрения (`paste` с маской: `DIV255`), процессор Qwen3-VL —
  `smart_resize` кратно 32, бикубика torchvision на uint8, нормировка 0,5/0,5.
  Всё бит в бит с эталоном (`preprocess_matches`: max 0).
- **Башня зрения** (`vision.rs`): 27 блоков ViT с учёной таблицей позиций
  48×48 (билинейная интерполяция `linspace(0, 47, n)`, align_corners), 2D-RoPE
  на голове 72, LayerNorm с bias, MLP GELU-tanh; мерджеры — GELU точный, у
  deepstack норма после склейки 2×2. F32. Сверка: cos 1,000000, deepstack тоже.
- **Энкодер** (`text_encoder.rs`): LLM Qwen3-VL (q/k-нормы, M-RoPE с
  чередованием частот `[24, 20, 20]`), deepstack-отводы прибавляются к строкам
  картинок **после** слоёв 0–2, выход — последний слой **до финальной нормы**,
  без 14 токенов системной части. Остаточный поток и нормы — в F32, проекции
  BF16: в чистом BF16 выбросы Qwen3 (до сотен) роняли косинус коротких
  промптов до 0,98, так — 0,9993–0,9999. Слои сверх VRAM стримятся из mmap.
  Шаблон: `<|im_start|>system\nComprehend and analyze the provided
  prompt.<|im_end|>\n<|im_start|>user\n<image1><|vision_start|>…<|vision_end|>
  <image2>…{prompt}<|im_end|>\n<|im_start|>assistant\n`; пустой промпт → `" "`.
- **DiT** (`transformer.rs`): токены референсов подставляются в текстовый
  поток на слоты `<|image_pad|>` (слот = 2×2 латентных токена), генерируемая —
  в конец; одна общая модуляция `Linear(d → 4d)` на все блоки
  (`x·(1+scale)`, остаток `tanh(gate)·y`), текст и референсы — временем 0.
  Внимание блочно-причинное: текст видит прошлое (flash `causal`: диагональ
  выровнена по концу — как раз «префикс + свой треугольник»), картинка внутри
  себя двунаправленна, генерируемая видит всё. **KV-кэш**: первый шаг считает
  всю склейку и запоминает K/V префикса (32 × prefix × 4096 × 2 в BF16 —
  4200 токенов ≈ 2,2 ГБ), остальные шаги считают только генерируемую; если
  кэш не влезает — префикс на каждом шаге. RoPE: `build_rope` Qwen-Image по
  позициям `QwenImage21Rope` (текст двигает позицию, картинка замораживает
  кадр и центрирует h/w). MXFP8 через `forward_target(1.0)`, как у Qwen-Image.
  Сверка на 2 блоках: F32 — cos 1,000000 во всех режимах (prefill, decode из
  кэша, без кэша), BF16 0,99999, MXFP8 0,99998, NVFP4 0,99998.
- **VAE** (`vae.rs`): резидуальный Wan 2.2 в одном кадре. Шорткаты сведены к
  2D: `AvgDown3D` при сжатии по времени дополняет кадр нулевым спереди —
  половина каналов шортката нули; `DupUp3D` берёт из двух кадров последний
  (`first_chunk`) — пиксель `(sh, sw)` канала `o` идёт из входного канала
  `(o·factor + k)/repeats`. Плитки с перекрытием как `tiled_decode` diffusers
  (256/192 по умолчанию, 512/448, 1024/896 по бюджету VRAM). Сверка: encode и
  decode cos 1,000000; плитки 128/96 — 0,9947 (швы, как у diffusers).
- **Модель** (`model.rs`): `encode_prompt(prompt, negative, images, res)`,
  `encode_references(images, res)`, `load_transformer(tokens)`, `sample`,
  `decode` → RGBA `[4, H, W]`. `res` — `output_resolution` пайплайна: сторона
  кратна 32, размер по умолчанию — квадрат или пропорции последнего референса.
  Сквозной прогон 4 шага 256² против diffusers BF16: латент cos 0,9999.
- **CLI**: `synaptix imagine qwen-image-2.1.syn "промпт" --device cuda
  --quant mxfp8 [--image a.png --image b.png] [--width 0 --height 0
  --resolution 1024]` — PNG с альфой, если модель нарисовала прозрачность.

Эталон: `scripts/reference/gen_qwen_image21.py` (diffusers main ≥ 0.41.dev,
transformers 5.17, torchvision CPU; venv из scratchpad сессии — пересоздать
`python -m venv --system-site-packages`, `pip install git+…/diffusers
"transformers>=5.17" torchvision --index-url …/cpu`, запускать с
`PYTHONNOUSERSITE=1 python -s` — в `~/.local` лежит битый torchaudio). Тесты:
`QWEN21_MODEL=… QWEN21_REF=… cargo test --release -p synaptix-image-qwen21
--test reference -- --ignored --test-threads=1`.

**Грабли:** `te.model.visual(...)` в transformers 5.17 возвращает
`last_hidden_state` **до** мерджера — эмбеддинги LLM лежат в `pooler_output`.
Выход VAE у непрозрачных генераций шумит альфой в 244…255 — порог
`RgbaImage::has_transparency` (альфа < 204) решает, RGB или RGBA. На CPU
`narrow(...).contiguous()` пустого тензора роняет `copy_strided`
(bytemuck на висячем указателе) — в плитках пустой хвост не режется.

## Замеры (RTX 5090 24 ГБ, 40 шагов, KV-кэш)

| | время | из них денойз | пик VRAM (nvidia-smi) |
|---|---|---|---|
| t2i 1024², MXFP8 | 30 с | 24 с | 13,6 ГБ |
| t2i 1024², NVFP4 | 23 с | 18 с | 9,3 ГБ |
| t2i 1024², BF16 | 41 с | 35 с | 14,8 ГБ |
| RGBA 1024², MXFP8 | 31 с | 24 с | 12,6 ГБ |
| правка 1024² + 1 референс, MXFP8 | 38 с | 29 с | 12,0 ГБ |
| t2i 2048², MXFP8 | 172 с | 158 с | 23,2 ГБ |

Энкодер: 1,9 с на промпт (36/36 слоёв на карте, башня F32 отдельно). На 2048²
пул подходит к потолку карты — с открытыми тяжёлыми окнами лучше NVFP4.

## synthos: ноды Нейро → Qwen-Image 2.1

**Checkpoint** (бандл/каталог, устройство, веса DiT `nvfp4 | mxfp8 | dense (bf16)`
— по умолчанию mxfp8, память, **разрешение** 512…2048 = `output_resolution`:
под него приводятся референсы и от него считается размер без Empty Latent,
«Держать в памяти»), **Reference** (картинка → латент VAE RGBA + исходник для
энкодера; цепочкой до 10, в промпте `<image1>`, `<image2>`…; альфа исходника
берётся из превью-буфера), **Text Encoder** (промпт, `negative` — пустой = без
CFG, `references` — необязательно), **Sampler** (шаги 0 = 40, CFG 1 = выкл,
KV-кэш, seed; размер — Empty Latent или разрешение чекпойнта), **VAE Decode**
(RGBA → `ImageData::from_rgba_tensor`, если есть настоящая прозрачность,
иначе чистый RGB). Хэндл свой (`QwenImage21ModelHandle` с `resolution_idx`),
латент свой (`QwenImage21Blob::Latent`, 64 канала на 16 px).

**Альфа в ImageData**: `tensor` остаётся RGB `[3, H, W]`, настоящая альфа живёт
в `rgba` (превью-буфер); Image Load читает файлы через `load_image_rgba`,
Image Save при `has_alpha()` пишет PNG с альфой из `rgba_tensor()`.

Шаблоны «Картинки»: **Text to Image**, **Edit Image**, **Multi-Reference**,
**Transparent RGBA**, **LLM Prompt Rewrite** (инструкция по мотивам Prompt
Enhancer из репо Qwen-Image-2.1: сохранить зафиксированное, додумать
остальное, надписи в кавычках, один абзац без JSON). Агент:
`qwen_image21_prompt.md`, опции чекпойнта (в т.ч. `resolution_idx`) в
`pipelines`. Строки на 14 языках. Тесты: `tests/qwen_image21_nodes` (реестр и
порты, рендер карточек, дефолты, round-trip состояния, связи шаблонов, альфа
в ImageData).

Смоук `src/bin/qwen_image21_smoke.rs` — настоящие `on_run` нод:
`qwen_image21_smoke <бандл> <out.png> [шаги [w h]]`, env `QWEN21_IMAGES`,
`QWEN21_PROMPT`, `QWEN21_QUANT`, `QWEN21_RESOLUTION`, `QWEN21_CFG`,
`QWEN21_KV_CACHE=0`, `QWEN21_VRAM_GB` (балласт «малой карты»).
