# Этап 2: портируемые ядра умножения для любого квант-формата (22.09.2026)

Третий этап плана `quant_lowbit_gguf_plan_2026.md`. После него любой
квантованный вес — NVFP4/MXFP8 движка, SQ1…SQ8, все типы ggml — умножается на
любой карте sm_80+: декод и малый батч через GEMV с деквантом в регистрах,
префилл через деквант полосами и плотный cp.async GEMM. На Blackwell для
NVFP4/MXFP8 остаются нативные block-scale MMA ядра.

## Что сделано

**Общий заголовок декодеров** `cu/elementwise/blockq_decode.cuh`: все
`deq_*` под-блоков из этапа 1 плюс декодеры форматов движка —
`deq_nvfp4_syn` (линейный packed + тайл-мажорные E4M3-шкалы,
`tile_scale_offset` как в `nvfp4_quant.cu`) и `deq_mxfp8_syn` (E4M3 + E8M0
натурально). Rust склеивает `ggml_tables.cuh + blockq_decode.cuh + ядра` в
модуль.

**GEMV** `cu/elementwise/blockq_gemv.cu` + `elementwise/blockq.rs`
(`BlockqGemvKernels`, `blockq_gemv`, `blockq_gemv_batched`): варп на строку N,
лейны делят под-блоки по 32 вдоль K (чтение веса по варпу непрерывное),
активация f16/bf16 читается четырьмя `uint4` на под-блок, накопление f32,
редукция шаффлом; до 8 строк активации за один проход по весу (спекуляция,
малый батч). Батчевый вариант — таблицы device-адресов, `blockIdx.y` —
эксперт. Ядра на все 8 SQ + 27 ggml + NVFP4/MXFP8 движка, модули f16 и bf16
(`-DSYN_ACT_BF16`).

**Префилл** `gemm/dispatch.rs::quant_linear_generic`: M ≤ 8 → GEMV; иначе
деквант полосами строк (бюджет `SYN_MXFP8_DEQ_MB`, общий скретч) через
`blockq_dequant_raw` / `blockq_dequant_band_syn` (для NVFP4/MXFP8 — по
смещению строки внутри целого веса) и `best_gemm_{f16tn,bf16}_linear_u8`,
куски сшиваются pitched-копиями. Пик памяти `m·chunk·2 + chunk·K·2`.

**Диспетчер** (`cuda_backend.rs::linear_quant`): нативные ядра только при
`caps.quant_native(dtype)` (Blackwell, NVFP4/MXFP8); одноблобные форматы и
всё без FP4 MMA — `quant_linear_generic`. Прежний WO-обход остался только по
`SYNAPTIX_NVFP4_WO=1`.

**MoE**: `Backend::quant_gemv_batched` + `QuantWeight::gemv_batched_dense`
(веса любого формата, плотная f16/bf16-активация, строка на эксперта);
`batched_pairs` разделён на `batched_pairs_nvfp4` (нативный: пред-квант,
перемешанные копии, фьюз swiglu→квант) и `batched_pairs_generic` (два батчевых
GEMV и `swiglu` между ними). Групповой GEMM префилла и фьюзы NVFP4 включаются
только при нативном FP4 — иначе префилл идёт по экспертам обычным `forward`.
Пред-квант активаций MXFP8 (`mxfp8_quantize_act`, `rms_mod_quant_mxfp8`) без
block-scale MMA тоже выключен: generic-путь считает по неквантованной
активации и точнее (тест `quant_bf16_paths` под sm_80 это и показал —
5 % расхождения там, где эталон стал точнее).

## Проверено

- `cuda_blockq_gemv`: GEMV против CPU-эталона (деквант f32 · x в f64) для всех
  форматов, M = 1/5/8, F16 и BF16; NVFP4/MXFP8 движка — вес квантован штатным
  ядром, эталон восстановлен из packed+scales; батч трёх экспертов = поодиночке.
- `cuda_quant_linear_generic`: `Tensor::linear_quant` для SQ4/SQ2/Q8_0 на M = 1,
  8, 100 в F16 и BF16 (GEMV и полосный GEMM), перенос веса CPU↔GPU.
- Под `SYN_FORCE_ARCH=sm_80` проходят `cuda_matmul_quant_dispatch`,
  `cuda_linear_quant_mxfp8`, `mxfp8_linear_shapes`, `cuda_mxfp8_split_n`
  (теперь через `quant_linear_generic`), нативно — без регрессий.
- `cuda_arch_matrix`: модули GEMV (f16/bf16) собираются под sm_80…sm_120a.
- **Сквозные прогоны под sm_80** (JIT на 5090):

| Модель | Путь | Декод |
|---|---|---|
| qwen3.8-27b, `--quant nvfp4` | этап 0 (деквант всего веса на вызов) | 1,2 ток/с |
| qwen3.8-27b, `--quant nvfp4` | этап 2, GEMV с деквантом в регистрах | **17,2 ток/с** |
| qwen3.8-27b, `--quant nvfp4` | нативно (sm_120a, FP4 MMA) | 37,8 ток/с |
| gemma-4-26b-a4b (MoE NVFP4, attn MXFP8) | этап 2, батч экспертов generic | 8 ток. за 228 мс, ответ совпал с нативным |

  Ответы связные и совпадают по смыслу с нативными. Разрыв 17 → 38 на
  плотной модели — цена деквантования в SIMT против тензорных ядер и
  по-прежнему один варп на строку; см. «Дальше».

## Чего этап 2 не даёт (честно)

- **dp4a-GEMV** (int8-активация) не написан — GEMV считает f16-FMA. На
  Ampere/Ada это может дать ещё ×1,5–2 на декоде целочисленных форматов.
- **Деквант в плитке mainloop'а GEMM** нет: префилл — деквант полосами + плотный
  GEMM, то есть лишний проход по весу в f16 (для M ≫ 8 это не узкое место, для
  M ≈ 16–64 — заметно).
- **MXFP8 через fp8-MMA на sm_89/90** нет: MXFP8 везде без Blackwell идёт
  деквантом.
- **Фьюзы qkv/swiglu/geglu** для не-Blackwell не сделаны: модель считает
  проекции по отдельности (это уже работало с этапа 0).
- Скорость измерена только эмуляцией на 5090: SM-ов и полосы у настоящих
  RTX 30xx/40xx меньше, числа выше — ориентир по доле от нативного.
