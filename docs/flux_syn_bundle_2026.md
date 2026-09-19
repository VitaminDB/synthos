# FLUX.1 в synthos: бандл, ноды, шаблоны (2026-09)

## Что качать с HuggingFace

У `black-forest-labs/FLUX.1-dev` в репозитории лежат две раскладки одних и тех же
весов:

- **diffusers** — подкаталоги `transformer/` (3 шарда, 22 ГБ), `text_encoder/`
  (CLIP-L), `text_encoder_2/` (T5-XXL, 2 шарда, 8,9 ГБ), `vae/`, `tokenizer/`,
  `tokenizer_2/`, `scheduler/` и `model_index.json`. Её и читает synaptix.
- **исходный формат BFL** — `flux1-dev.safetensors` (только трансформер, другие
  имена тензоров) и `ae.safetensors` (VAE). Это дубликаты: без текстовых
  энкодеров модель из них не собрать, и упаковщик их пропускает.

Качать нужно подкаталоги; одиночные файлы в корне не нужны. Репозиторий gated:
без HF-токена API скрывает sha256 LFS-файлов (звёздочки вместо `lfs.oid`) —
сверять можно по открытым зеркалам (`camenduru/FLUX.1-dev-diffusers`) или с
токеном в настройках.

## Упаковка

```sh
cd ~/Storage/syn_models
~/Projects/2027/synaptix/target/release/syn-pack FLUX.1-dev -o flux.1-dev.syn \
  --id flux.1-dev --version 1.0.0 --arch flux.1 --purpose image
```

План распознаётся сам (`syn-scan FLUX.1-dev --plan`): компоненты
`transformer`, `text_encoder`, `text_encoder_2`, `vae` → чанки
`tensors:<имя>`; конфиги и токенайзеры — вспомогательными файлами под теми же
путями; `flux1-dev`/`ae` выключены как дубликаты. Бандл — плотный BF16,
33,7 ГБ; квант NVFP4/MXFP8 делается при загрузке трансформера (бит в бит как
квант на лету из каталога).

## Проверка

```sh
syn-verify flux.1-dev.syn                     # CRC32C всех чанков
FLUX_DIR=…/FLUX.1-dev FLUX_SYN=…/flux.1-dev.syn \
  cargo test --release -p synaptix-image-flux --test bundle_source -- --ignored --nocapture
FLUX_DIR=… FLUX_SYN=… cargo test --release -p synaptix-image-flux \
  --test bundle_e2e -- --ignored --nocapture --test-threads=1   # GPU
```

`bundle_source` сравнивает все 1 819 тензоров и конфиги с каталогом байт в
байт; `bundle_e2e` генерирует 512×384 из бандла и из каталога (бит в бит),
проверяет img2img (VAE encode → decode, MAE ≈ 0,0065) и отмену.

## synaptix: `synaptix-image-flux`

- `FluxSource` — каталог diffusers или `.syn` (как `H3Source`); компонент без
  чанка в бандле — явная ошибка, а не молча основной чанк.
- `FluxModel` — стадии для нод: `encode_prompt` (CLIP + T5, грузятся и
  отпускаются, всегда BF16 — T5 в F16 переполняется), `load_transformer`
  (квант резидентно; плотный — по свободной VRAM, частичный стриминг блоков),
  `sample` (txt2img / img2img с `denoise`, отмена через колбэк), `decode`,
  `encode_image`.
- Конфиги читаются из модели: `guidance_embeds` (dev / schnell),
  `scheduler_config.json` (динамический сдвиг dev, статический `shift` schnell).
- `FluxPipeline::txt2img` (CLI `synaptix imagine`) — те же стадии подряд;
  `imagine` понимает и `.syn`.
- Память: веса грузятся под `WeightsAllocGuard` (weights-пул), после каждой
  стадии — `model::release_pools` → `hard_trim_all_pools_device`. Без этого
  T5 оставлял ~10 ГБ в пуле активаций (порог освобождения — бесконечность),
  и их не видели ни DiT, ни следующая модель графа (LTX/H3). Запас
  auto-offload плотного DiT включает 1,5 ГБ под рабочий стол.

## synthos: ноды

Нейро → FLUX: **Checkpoint** (бандл/каталог, устройство, квант трансформера
`nvfp4 | mxfp8 | dense (bf16)`, по умолчанию mxfp8; память `auto | resident |
block_offload`; «Держать в памяти»), **Text Encoder** (T5 `auto/256/512`;
повтор с тем же промптом берёт готовое кондиционирование), **Empty Latent**
(размер, стороны кратны 16, пропорции как у H3), **VAE Encode** (картинка →
латент; размер — со входа `size` или сама картинка до 2 Мп), **Sampler**
(шаги 28, guidance 3.5, seed, denoise), **VAE Decode** (картинка + превью).

Картинки: **Image** (файл → порт `image`), **Image Save** (PNG/JPEG/WebP по
расширению). Новый тип порта `image` (оранжевый провод), `DataBlob::Image`.
LTX Image и H3 Keyframe получили необязательный вход `image`: провод важнее
выбранного файла.

Шаблоны: `builtin-flux-text-to-image`, `builtin-flux-image-to-image` (раздел
«Картинки»), `builtin-ltx-flux-keyframe`, `builtin-h3-flux-keyframe`
(«Видео»: кадр FLUX 1344×768 → image-to-video).

Агент: `pipelines list` показывает каталог diffusers как «HF pipeline
directory (FluxPipeline)» и архитектуру `.syn` (`arch flux.1, image`);
расшифровка `*_idx` полей FLUX; при открытии шаблона с FLUX Text Encoder
приходит формат промпта (`agent/tools/flux_prompt.md`). Прогон из чата
кладёт PNG из Image Save (или из VAE Decode, если сохранения нет) в
сообщение.

Смоук без GUI (настоящие `on_run` нод):

```sh
cargo run --release --bin flux_smoke -- ~/Storage/syn_models/flux.1-dev.syn out.png 768 768 20
# FLUX_IMAGE=src.png FLUX_DENOISE=0.6 — img2img; FLUX_REPEAT=2 FLUX_RESIDENT=1 — кэш промпта и DiT
```

## Малая карта (2026-09-19, synaptix dcc8c11)

FLUX.1 теперь работает при 7 ГБ VRAM: T5-XXL кладёт на карту столько блоков,
сколько влезает с запасом, остальные читает из бандла прямо в forward
(`T5Encoder::load_budgeted`); квантованный DiT больше не всегда резидентен —
в режиме памяти `auto`/`block_offload` не влезшие блоки уезжают пиннованной
копией на хост (`FluxTransformer::load_budgeted`), в forward следующий блок
едет на loader-стриме во время счёта текущего (`stream_each`, и для плотного
частичного оффлоада); перед VAE и энкодерами пулы отдаются драйверу. Замер
`tests/low_vram.rs` (балласт оставляет 7 ГБ), 1024² 20 шагов: NVFP4 — 13,2 с,
MXFP8 — 16,4 с, BF16 — 27,9 с, минимум свободной VRAM по фазам ≥ 0,67 ГБ.
См. также `docs/flux2_2026.md`.
