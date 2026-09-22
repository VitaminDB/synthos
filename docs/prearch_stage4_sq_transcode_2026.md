# Этап 4: SQ-энкодер, перекодировка при загрузке, форматы по карте (22.09.2026)

Пятый этап плана `quant_lowbit_gguf_plan_2026.md`. После него у движка есть
свой квант на любое число бит с энкодером на карте, любой квантованный
источник (NVFP4/MXFP8-бандл, блоки ggml из `.gguf`) перекодируется в формат
политики при загрузке — в памяти или через дисковый кэш, — а `optimal_profile`
считает возможности карты: без FP4-MMA плотная модель по умолчанию идёт в
SQ4, а не в NVFP4. Мастер упаковки synthos, настройки моделей и CLI видят
SQ1…SQ8.

## Что сделано

**Энкодер SQ.** CPU-эталон `synaptix_core::quant::sq::quant_super_block`
переписан с RTN на подбор шкалы: `fit_sub_block` перебирает 21 кандидата
вокруг min/max (как `make_qkx2_quants` в ggml), для каждого значения
квантуются и (шкала, минимум) пересчитываются наименьшими квадратами при
`min ≤ 0`; лучший по MSE. Затем под-шкалы квантуются в u8 относительно
`d`/`dmin` супер-блока и значения кодируются уже квантованными шкалами.
`quant_super_block_rtn` оставлен как эталон качества: на 2–4 битах подбор
даёт ≥10 % меньший MSE (тест `search_is_no_worse_than_rtn`). Ядро
`cu/elementwise/sq_quant.cu` (`elementwise/sq_quant.rs`): поток — под-блок,
восемь потоков супер-блока сводят максимумы шаффлом; `--fmad=false` и тот
же порядок операций — блоб совпадает с CPU бит в бит на всех SQ1…SQ8, входе
F16 и BF16, с хвостовым неполным супер-блоком (`cuda_sq_quant`).
`Backend::quantize_sq`, `Tensor::quantize_to_sq(bits)` и общий
`Tensor::quantize_to(dtype)`; `Backend::nvfp4_dequant` (линейный packed +
тайл-мажорные шкалы через `blockq_dequant_band_syn`) — теперь
`QuantWeight::dequantize` умеет и NVFP4, путь перекодировки.

**Движок.** `QLinear::build` принимает `DType::Sq` (K % 32); эмбеддинг
квантуется в SQ и читается тем же gather-ядром, что и блоки GGUF, связанная
голова делит с ним блоб. `parse_dtype("sq4")`, `PrecisionConfig::sq(bits)`,
пресет `--quant sq4…sq8` в CLI, `precision::dtype_name` для UI и ключей.

**Перекодировка** `synaptix-io/src/weights/transcode.rs`. `TranscodeSpec` —
формат по ролям (attn/mlp/эксперты/lm_head/embed, роли из
`synaptix_bundle::inspect::classify_in`), `requant` (двойной квант) и
`quantize_dense`. Каждый тензор: `QuantWeight` источника →
`dequantize(F16)` на карте (или плотная матрица из источника) →
`quantize_to(target)` → байты на хост; стопки экспертов — по срезу. Результат
— **оверлей** поверх источника: `SynBundleLoader` во всех квант-запросах
(`load_quant*`, `quant_dims/kind`, `quant_blob_slices`, плотное чтение)
сперва смотрит в него. Поэтому модели, pinned-зеркала экспертов
(`expert_blob_ranges` берёт срезы оверлея) и кэши о перекодировке не знают.
Размещение (`Placement::Auto`): целевой объём + 6 ГБ запаса ≤ `MemAvailable`
→ `Vec<u8>` на тензор в RAM; иначе дисковый кэш — компактный `.syn` только с
`.qpacked`/`.qscales` и `quant_manifest.json` в
`$XDG_CACHE_HOME/synaptix/transcode/<stem>-<hash>.syn` (`SYN_TRANSCODE_CACHE`),
пишется потоком тензор за тензором и открывается через mmap; ключ — путь,
размер, mtime, спецификация, версия энкодера; повторное открытие —
`disk-cached`. `SYN_TRANSCODE_DISK=1` форсирует диск. Заявка
`transcode::request(path, …)` живёт до сброса guard'а: все
`SynBundleLoader::open` этого пути (основной компонент, драфтер, башня
зрения) делят один оверлей; явный `SynBundleLoader::transcode` реестр не
трогает. `write_bundle` пишет полный самостоятельный `.syn`: перекодированные
матрицы, остальные тензоры и файлы как есть, все `tensors:<компонент>`.

**Фасад.** `QuantPolicy.transcode: bool` (по умолчанию выключен),
`transcode_spec()` из полей политики (эмбеддинг — только в MXFP8/SQ: у
NVFP4-таблицы нет gather), заявка ставится в `load_llm_with_policy` на время
загрузки, `set_transcode_progress`. `optimal_profile(path)` =
`optimal_profile_for(path, caps карты 0)`: без FP4-MMA NVFP4 в политике →
SQ4 (веса, внимание, голова, эмбеддинг); квантованный NVFP4-бандл остаётся
как есть (портируемый путь этапа 2), MXFP8 внимания Gemma не трогается.
`facade::llm::transcode_bundle`, `facade::arch::bundle_quant_formats(path)`
(форматы весов файла для панелей и предупреждения о двойном кванте).

**CLI** `synaptix quantize <in.syn|in.gguf> <out.syn> --format sq4 [--attn
mxfp8] [--lm-head sq6|none] [--embed sq8|none] [--keep-quant] [--device
cuda:N]` — плотные и уже квантованные источники, стопки экспертов; HF-каталог
— сперва `convert`.

**synthos.** Мастер упаковки: `QuantChoice::Sq(b)`, SQ8/6/5/4/3/2 в быстром
выборе и по ролям (эмбеддинги — MXFP8/SQ8/SQ6/SQ4), упаковщик пишет SQ
(`quantize_to_sq`, без `.qscales`), подсказка «попробуйте SQ4», когда NVFP4
не берёт формы (N, K не кратны 64), `SizeEstimate` считает SQ по числу
супер-блоков. Настройки моделей: в списке хранения SQ8…SQ2 с подписью
«нативно / портируемые ядра» по возможностям карты; строка модели показывает
формат квантованного бандла; тумблер «Перекодировать квантованный бандл» с
описанием, из чего именно (двойной квант) — `ModelProfileConfig.transcode`;
бейдж чата показывает формат файла, пока перекодировка выключена. Подписи
на 14 языках.

## Проверено

- `cuda_sq_quant`: GPU-энкодер = CPU-эталон бит в бит (SQ1…SQ8, F16/BF16,
  хвостовой супер-блок, выбросы); NVFP4 → F16 → SQ4 round-trip.
- `transcode_overlay` (synaptix-io): оверлей в памяти и на диске поверх
  Qwen3-0.6B Q8_0 (размеры блобов, деквант ≈ источник, плотное чтение через
  оверлей), кэш пишется один раз и находится повторно, заявка действует на
  все открытия и снимается, полный бандл через `write_bundle` открывается
  без оверлея и несёт `syn-quant-v1`.
- `transcode_smoke` (фасад): GGUF Q8_0 и Q4_K_M → SQ8/SQ4 при загрузке,
  «The capital of France is» → «Paris»; NVFP4-бандл qwen3.8-27b → SQ4 в
  памяти (15 ГБ, 14,5 с загрузки) и → SQ8 (42 с) — ответ тот же; всё то же
  под `SYN_FORCE_ARCH=sm_80`.
- `synaptix quantize` Llama-3.2-1B Q4_K_M → SQ4 (голова SQ6): бандл
  открывается и отвечает «Paris».
- Регрессии без изменений: `gguf_facade_smoke`, `gemma4_smoke`,
  `quant_bf16_paths`, `qwen38_facade_smoke`, `gguf_keep_roundtrip`,
  `syn_bundle_*`, unit-тесты core/bundle; unit-тесты synthos.

## Чего этап 4 не даёт (честно)

- **Качество SQ на 0,6B**: Qwen3-0.6B из Q8_0 в SQ4 отвечает не «Paris»
  (SQ8 — отвечает). Четырёх бит такой модели мало и в llama.cpp; но сравнение
  SQ4 с Q4_K на равных (perplexity) — этап 5. Importance-взвешивания у
  энкодера нет (см. план, §9).
- Перекодировка считается на карте по одному тензору; на 7 ГБ VRAM самый
  большой тензор (эмбеддинг 262k × 5120 в F16 = 2,7 ГБ плюс источник и
  результат) должен влезать — специально не проверено.
- Дисковый кэш не чистится сам: `~/.cache/synaptix/transcode`.
- `optimal_profile` смотрит на карту 0; с несколькими разными картами
  профиль по-прежнему один.
- Прогресс перекодировки в GUI synthos не показывается (колбэк есть:
  `set_transcode_progress`), загрузка просто идёт дольше.
- Скорость SQ-путей — числа этапа 2 (GEMV с деквантом в регистрах); dp4a и
  деквант в плитке GEMM всё ещё отложены.
