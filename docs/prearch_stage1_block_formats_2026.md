# Этап 1: одноблобные форматы в ядре — SQ и типы ggml (22.09.2026)

Второй этап плана `quant_lowbit_gguf_plan_2026.md`. Движок получил два
семейства форматов весов, у которых шкалы лежат внутри блока и тензор на
карте — один непрерывный блоб байт в байт как в файле: собственный **SQ**
(1…8 бит) и **типы ggml** (файлы llama.cpp). Ядер умножения для них ещё нет
(этап 2); есть деквант на карте, бит в бит совпадающий с CPU-эталоном, чтение
из бандла и сверка с gguf-py.

## Что сделано

**`synaptix-core::quant`** — новый модуль:
- `ggml::GgmlType` — все 35 живых типов `enum ggml_type` (включая новые
  `NVFP4 = 40` в раскладке ggml, `Q1_0 = 41`, `Q2_0 = 42`), геометрия блоков по
  `static_assert`'ам `ggml-common.h`, имена и ключи. Переехал из
  `synaptix-gguf` (там остался реэкспорт).
- `ggml_tables` — решётки IQ (`iq2xxs/iq2xs/iq2s/iq3xxs/iq3s/iq1s_grid`),
  `ksigns/kmask_iq2xs`, `kvalues_iq4nl/fp4` — сгенерированы скриптом из
  `ggml-common.h` (MIT), 90 КБ; та же таблица в `ggml_tables.cuh` для NVRTC.
- `ggml_dequant::dequantize` — CPU-эталон для всех типов: к прежним 21
  добавлены IQ1_S/M, IQ2_XXS/XS/S, IQ3_XXS/S, TQ1_0/TQ2_0, Q8_1, Q8_K, Q1_0,
  Q2_0, NVFP4(ggml). Порядок операций повторяет `dequantize_row_*` из
  `ggml-quants.c` — это контракт для GPU.
- `sq` — формат SQ: супер-блок 256 = 8 под-блоков по 32; заголовок 20 байт
  (`d: f16`, `dmin: f16`, `sub_scale[8]`, `sub_min[8]`), значения битовыми
  плоскостями (`b` слов `u32` на под-блок); `w = d·sub_scale·q − dmin·sub_min`,
  минимумы ≥ 0 как у Q4_K. Размер `20 + 32·b` байт на 256 → `b + 0,625` бит на
  вес. Эталонный энкодер RTN на CPU (`quantize_matrix`) и деквант.
- `DType::Sq { bits }` и `DType::Ggml(GgmlType)`; `is_block_quant()`,
  `bytes_for_numel` по строке, `format_key`/`format_from_key`
  (`sq4`, `ggml:q4_k`, …), `block_row_bytes`, `dequant_row_f32`.

**`QuantWeight`**: масштабы стали `Option` (у одноблобных — `None`),
`new_block(packed, dtype, n, k)`, `scales_opt()`, `block_row_bytes()`,
`dequantize(out_dt) -> Tensor` (одноблобные — через новый
`Backend::block_dequant`, MXFP8 — через `mxfp8_dequant`). `to_device`,
`device_address`, `from_shuffled` учитывают отсутствие масштабов.

**CUDA** — `cu/elementwise/blockq_dequant.cu` + `elementwise/blockq.rs`: одно
ядро на формат (8 SQ + 27 ggml), поток считает под-блок из 32 значений; выход
F16 или BF16 (`-DSYN_OUT_BF16`); модуль собирается с `--fmad=false`, поэтому
`a·b − c` не сливается в FMA и биты совпадают с CPU. Портируемый (sm_80+).

**Бандл** (`synaptix-bundle`): `QuantKind::{Sq(u8), Ggml(GgmlType)}`,
`dtype()/from_dtype()/has_scales()`, `quantized_bytes` и `QuantEntry::packed_bytes`
для одноблобных; ключи манифеста — из core. Крейт теперь зависит от
`synaptix-core` (раньше сознательно нет — геометрия блоков не должна жить в
двух местах). `synaptix-io::build_quant_slice` читает `.qpacked` без `.qscales`
и собирает `QuantWeight::new_block`; стопки экспертов — срезами, как прежде.

**synthos**: упаковщик (`quant_pack.rs`, `quant_pack_smoke`) знает о новых
вариантах и честно отказывается их писать до этапа 4.

## Проверено

- `synaptix-core`: юнит-тесты геометрии (размеры блоков = ggml), ключей,
  SQ (упаковка плоскостей для всех `b`, ошибка падает с ростом бит, хвостовой
  супер-блок, положительный блок → `min = 0`).
- `tests/ggml_dequant_ref.rs`: CPU-деквант 23 типов против `gguf.quants.dequantize`
  (gguf-py) на случайных валидных блоках — совпадение до 1e-6 (в основном
  бит в бит). Фикстура `reference_data/ggml_dequant_ref.bin` (55 КБ) сгенерирована
  скриптом с venv `gguf`; Q1_0/Q2_0/NVFP4(ggml) в gguf-py нет — они покрыты
  только юнит-тестами.
- `cuda_blockq_dequant`: GPU против CPU **бит в бит** для 27 форматов весов
  ggml, Q8_1/Q8_K и SQ1…SQ8, в F16 и BF16, включая неполный хвостовой
  супер-блок SQ и крайние блоки.
- `cuda_arch_matrix`: модуль собирается под sm_80/86/89/90a/120a.
- `syn_bundle_block_quant`: бандл с `sq4` и стопкой `ggml:q4_0` без блоба
  масштабов читается на CPU и на карте, `dequantize()` совпадает с эталоном.
- Регрессия: `syn_bundle_quant_stack`, `cuda_matmul_quant_dispatch`,
  `cuda_nvfp4_gemv_batched`, `cuda_linear_quant_mxfp8`, `synaptix-bundle`,
  `synaptix-gguf`; `cargo check` обоих репозиториев.

## Грабли

- NVRTC не знает `stdint.h` — типы `uint8_t`… объявлены в `ggml_tables.cuh`.
- Q3_K: бит маски `hmask` сквозной по всем 8 под-блокам (`1 << s`, а не
  `1 << (s % 4)`) — ловится только сравнением бит в бит, тест поймал.
- Q8_1 в актуальном ggml — 36 байт (`d`, `s` в f16), а `GGML_QUANT_SIZES`
  gguf-py всё ещё говорит 40; взят `static_assert` из `ggml-common.h`.
- gguf-py не умеет квантовать ничего из IQ, поэтому эталон — случайные байты
  с «разумными» f16-шкалами; для IQ1_M шкала собрана из ниббл `scales`, и
  старший ниббл фиксируется, чтобы f16 не ушёл в NaN.

## Чего этап 1 не даёт

- Умножений на этих форматах нет: `linear_quant` для `Sq`/`Ggml` → `Unsupported`.
  Это этап 2 (`TileDecoder<Fmt>`: GEMM W×A16, dp4a-GEMV, MoE, fused).
- Энкодера SQ на GPU и упаковки в мастере synthos нет (этап 4); прямой
  загрузки `.gguf` нет (этап 3).
