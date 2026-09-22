# Этап 0: карта решает, под что компилировать (22.09.2026)

Первый этап плана `quant_lowbit_gguf_plan_2026.md`: движок должен подниматься
на любой карте sm_80+, а не только на Blackwell. До этого ни один кванто-
ванный вес не работал нигде, кроме RTX 50xx: GEMM/GEMV NVFP4 и MXFP8 собирались
только под `sm_120a`, а модуль, не собравшийся под карту, ронял загрузку.

## Что сделано

**`synaptix-kernels-cuda::caps::DeviceCaps`** — один запрос атрибутов на контекст:
compute capability, число SM, opt-in shared memory, VRAM, и выведенные из них
возможности (`fp4_mma`, `mxfp8_mma`, `fp8_mma`, `fp8_cvt`, `sm90`, `bf16_mma`,
`dp4a`) и цель NVRTC (`sm_80` … `sm_89`, `sm_90a`, `sm_100a`, `sm_120a`).
`quant_native(dtype)` отвечает, исполняется ли NVFP4/MXFP8 тензорными ядрами.

**Компиляция** (`kernels::compile`): `compile_module` и `compile_module_with_opts(.., None)`
берут цель из `DeviceCaps`; в опции NVRTC всегда идут `-DSYN_CC=<major*10+minor>`
и `-DSYN_ARCH_A=0|1`. Новая `compile_module_req(.., Feature)` возвращает
`Unsupported` с понятным текстом вместо лога NVRTC, если возможности нет. Из
22 мест с зашитой целью не осталось ни одного: портируемые модули (`gemm_f16`,
`gemm_f32`, `gemv_mxfp8`, `implicit_conv`, `topk`, `flash_*mxfp8*`, `mma_gemv`,
квант-ядра, `flash_blocks`) собираются под карту, Blackwell-модули (`gemm_nvfp4`,
`gemv_nvfp4`, `gemm_mxfp8`, `nvfp4_{qkv_proj,swiglu,geglu}_shuf`) требуют
`Feature::Fp4Mma`/`Mxfp8Mma`, `flash_splitq6` — `Feature::Sm90`. `flash_blocks`
больше не собирает два модуля (sm_80 + sm_120a) — один под цель карты.

**TMA-семейство `gemm_bf16.cu`** (`gn_{bf16,f16}_tma_*`: setmaxnreg,
cp.async.bulk.tensor, mbarrier.try_wait) обёрнуто в `#if SYN_ARCH_A && SYN_CC >= 90`;
под `sm_80`…`sm_89` и под «плоскую» `sm_120` его нет, Rust-сторона грузит эти
функции через `.ok()` и остаётся на cp.async-семействе.

**Обходы без Blackwell** (`cuda_backend.rs`, `gemm/dispatch.rs`):
- `linear_quant` NVFP4 с F16-активацией идёт путём «`nvfp4_dequant_f16` → плотный
  f16 GEMM» (раньше только по `SYNAPTIX_NVFP4_WO=1`); BF16-активация → `Unsupported`,
  `QLinear::forward` берёт F16-обход.
- MXFP8: `mxfp8_linear_tiled` отвечает `Ok(false)` без block-scale MMA, а
  `mxfp8_linear_dequant_fallback` не пытается считать «голову» tiled-ядром —
  весь вес деквантуется полосами; декод M=1 — SIMT `gemv_mxfp8`, он портируемый.
- Производители пред-квантованных активаций NVFP4 (`nvfp4_quantize_act`,
  `silu/gelu_mul_quant_nvfp4`, `rms_mod_quant_nvfp4`, `dec_geglu_quant_nvfp4`)
  возвращают `Unsupported` без FP4 MMA — модель идёт обычным `forward`.
- `Backend::quant_native(dtype, device)` (default `false`, CUDA — по `DeviceCaps`).
  `QuantWeight::ensure_shuffled` без FP4 MMA — no-op (линейный `packed` остаётся,
  копию не строим); зеркало экспертов Qwen4Exp не перепаковывает NVFP4; батчевый
  путь MoE проверяет наличие копии, а не только ошибку.

**Фасад и CLI**: `synaptix::facade::device::{cuda_caps, cuda_device_count,
quant_native, cuda_summary}`; `synaptix devices` печатает по строке на карту.

## Эмуляция старых карт на 5090

`SYN_FORCE_ARCH=sm_80` (`sm_86`, `sm_89`, `sm_90`, `sm_120` — без `a`) компилирует
всё под указанную цель: PTX совместим вперёд, драйвер JIT'ит его под 5090, а
инструкции новее цели NVRTC не пропускает. Возможности считаются по цели, не по
карте: под `sm_80` `fp4_mma() == false`, и движок идёт обходами. Скорость при этом
не показательна; корректность — да.

## Проверено

- `tests/cuda_arch_matrix.rs`: 68 портируемых `.cu` собираются под sm_80/86/89/90a/120a;
  6 Blackwell-файлов собираются под sm_120a и не собираются ниже; `flash_splitq6` —
  только sm_90a+; в PTX `gemm_bf16.cu` под sm_80 и `sm_120` нет `setmaxnreg`.
- Под `SYN_FORCE_ARCH=sm_80` проходят: gemm f16/bf16, mma_gemv, flash_splitq,
  flash_bf16, flash_blocks (+MXFP8-KV), flash_decode, mxfp8_kv_decode, gemv_mxfp8,
  linear_quant_mxfp8 (все формы через деквант-обход), matmul_quant_dispatch (NVFP4
  через WO-обход), mxfp8_split_n, mxfp8_linear_shapes, nvfp4_roundtrip, rms_mod_quant,
  embed, embed_gather_mxfp8, topk. Тесты Blackwell-ядер пропускаются с сообщением.
- Нативный прогон тех же наборов — без регрессий. `mxfp8_linear_shapes` в нативном
  режиме падает (L2 = 0,183 на m=2) и до этих правок — старая проблема tiled-ядра
  на малом M, под эмуляцией тот же тест проходит через деквант.

## Чего этап 0 не даёт

- NVFP4 prefill без Blackwell материализует весь вес в F16 (`N·K·2` байт) на каждый
  вызов — медленно и прожорливо; MoE-эксперты NVFP4 (Gemma-4, Qwen4Exp) без FP4 MMA
  не считаются вовсе (группового/индексированного GEMV нет). Это этап 2:
  `TileDecoder<Fmt>` с деквантом в плитке и dp4a-GEMV.
- `optimal_profile` всё ещё не смотрит на карту (этап 4).
