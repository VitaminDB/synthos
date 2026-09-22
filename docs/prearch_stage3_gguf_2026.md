# Этап 3: полный GGUF — прямая загрузка, мапперы, токенизатор, Keep-конверсия (22.09.2026)

Четвёртый этап плана `quant_lowbit_gguf_plan_2026.md`. После него `.gguf`
llama.cpp — такой же файл модели, как `.syn`: фасад `load_llm*`, CLI
`run/chat/inspect/convert`, чат и ноды synthos открывают его напрямую, веса
исполняются блоками ggml теми же портируемыми ядрами этапа 2 (на любой карте
sm_80+, на Blackwell тоже — нативных ядер под форматы ggml нет и не нужно).
Конвертация в `.syn` больше не раздувает Q4 до F16: блоки копируются байт в
байт (`OutDtype::Keep`).

## Что сделано

**Реестр мапперов** `synaptix-gguf/src/arch/` по `general.architecture`:
`llama`, `qwen2`, `qwen3`, `qwen3moe` (`dense.rs` — один модуль, отличия по
флагам: bias q/k/v, q/k-нормы, стопки экспертов `StackConcat`), `gemma3.rs`
(нормы `SubOne`, `query_pre_attn_scalar` по правилу 27B), `gemma4.rs` (типы
слоёв из `sliding_window_pattern`, `K = V` на полных слоях по отсутствию
`attn_v`, доля вращаемых измерений из `rope_freqs`, роутер/эксперты/
`layer_scalar`), `qwen35.rs` (гибрид, как раньше). Каждый маппер синтезирует
`config.json` с `model_type` движка, `generation_config.json`,
`tokenizer_config.json`, `chat_template.jinja`. Общие ключи и продюсеры — в
`common.rs`; `rope_scaling` Llama-3 восстанавливается подбором профиля по
`rope_freqs.weight` (factor 8/32, low 1, high 4).

**Перестановка q/k у `llama`.** Конвертер llama.cpp переставляет строки
`attn_q`/`attn_k` под чередующийся RoPE (`permute`: (heads, 2, hd/2) →
(heads, hd/2, 2)); движок считает RoPE половинами, как HF. Маппер возвращает
исходный порядок `Producer::PermuteRows` — на уровне строк блоков, квант не
трогается. Без этого Llama-3.2 отвечала «a country, and the only».

**Токенизатор** `tokenizer.rs`: byte-BPE (`gpt2`) с таблицей
`tokenizer.ggml.pre` → регэксп предтокенизации (все значения llama.cpp,
неизвестное — ошибка, не тихий Qwen); SentencePiece (`llama`) — merges
генерируются из score'ов в порядке transformers, пользовательские куски
со score 0 (llama.cpp пишет −1000), `▁` вместо пробела, все токены в
vocab (иначе tokenizers переназначает id добавленных). Дубли добавленных
токенов схлопываются к первому id. BOS — `TemplateProcessing`, когда
`add_bos_token`. `eos_ids()` собирает `eos`/`eot` плюс управляющие токены с
именами из списка `special_eog_ids` llama.cpp (`<|eot_id|>`, `<|eom_id|>`,
`<|im_end|>`, `<end_of_turn>`, …): без `<|eom_id|>` Llama-3 после вызова
инструмента не останавливалась. Тест `tokenizer_matches_llama_cpp` — парити
с `llama-tokenize` на четырёх моделях.

**`GgufSource`** (`source.rs`) — источник весов поверх mmap: квант-тензоры
отдаются как `QuantWeight::new_block` (zero-copy для `Direct`; перестановки
строк/столбцов и стопки из частей собираются на уровне блоков), плавающие —
как есть; плотное чтение кванта — деквант ядром на карте или
`materialize_f32` на хосте. `SynBundleLoader` получил второй бэкенд
(`open` нюхает магию GGUF): `load_quant*`, `quant_dims/kind`, `names`,
`contains`, `read_file` (синтезированные файлы) — модели не знают, что под
ними. Свободные функции `is_model_file`/`read_model_file` — «файл модели» =
`.syn` или `.gguf`; на них переведены загрузчики gemma3/gemma4/гибрида/
qwen3/llama и `facade::arch::read_model_file`. Загрузчики qwen3/llama стали
ленивыми (`Option<SynBundleLoader>` + `quant()`), `Qwen3Config` знает
`model_type` (`qwen2` — без q/k-норм, с bias q/k/v в `FullAttn`).

**Эмбеддинг и связанная голова из блоков.** `QuantWeight::embed_gather` для
`DType::Sq`/`DType::Ggml` — одно ядро `deqg_<fmt>` (`blockq_dequant.cu`):
строки таблицы по индексам с деквантом в F16, индекс вне таблицы — нули;
`Backend::block_gather_dequant`. (Первый вариант — gather байтов F16-ядром
эмбеддингов + деквант — давал битые строки; заменён.) Gather в F16 приводится
к рабочему типу модели (BF16 у Gemma). Связанная `lm_head` при упакованном
эмбеддинге — `QuantWeight::share()`: вторая ручка на тот же блоб без копии
и повторного кванта.

**Шаблон чата.** Фасад передаёт `tools = none`, когда инструментов нет (как
`apply_chat_template` в transformers): у Llama-3.2 `tools is not none` для
неопределённой переменной было истиной, и модель получала инструкцию
«ответь JSON-вызовом».

**Конвертация `OutDtype::Keep`** (по умолчанию в CLI `convert` и в
HF-браузере synthos): квант-тензоры, которые отдаются блоками
(`quant_capable`: формат весов, K кратен блоку и 32, без преобразований),
пишутся как `<имя>.qpacked` (U8, `[.., N, row_bytes]`) плюс
`quant_manifest.json` (`format = "ggml:q4_k"`, исходная форма) и
`required_caps = syn-quant-v1`; всё остальное (нормы, `interleave`,
`SubOne`) деквантуется как в Auto. `ConvertReport.kept_quant`. Читатель
бандла: `quant_blob_slices` без `.qscales` у одноблобных форматов,
`load_to` квант-веса без плотной копии — деквант на хосте (как у `.gguf`).

**CLI**: `synaptix inspect model.gguf` (метаданные, статистика по типам, маппер
и `model_type`, `-v` — тензоры), `convert --dtype keep|dequant|f16|bf16|f32`
(`keep` по умолчанию), фича `gguf` включена по умолчанию.

**synthos**: список LLM в настройках, пикер бандла, LLM-нода и поиск видят
`.gguf`; HF-браузер конвертирует в Keep.

## Проверено

- `synaptix-gguf/tests/real_gguf.rs`: план/`config.json`/квант-веса/деквант
  и парити токенизации с `llama-tokenize` на Qwen3-0.6B Q8_0, Gemma-3-1B
  Q8_0, Llama-3.2-1B Q4_K_M, Qwen2-7B Q4_0 (bias).
- `synaptix/tests/gguf_facade_smoke.rs`: те же четыре файла через
  `load_llm_with_policy` — сырой промпт «The capital of France is» → «Paris»
  (как `llama-completion`), чат-шаблон → «Paris». Нативно и под
  `SYN_FORCE_ARCH=sm_80`.
- `synaptix-io/tests/gguf_keep_roundtrip.rs`: GGUF → `.syn` (Keep) →
  `SynBundleLoader`: 197 тензоров блоками, байты блобов и плотный деквант
  совпадают с прямым чтением `.gguf`; Llama-3.2 Q4_K_M из keep-бандла отвечает
  так же, как из `.gguf`.
- Регрессии без изменений: `gemma4_smoke`, `qwen38_facade_smoke`,
  `quant_bf16_paths`, `cuda_arch_matrix` (новые `deqg_*` собираются под
  sm_80…sm_120a), `cuda_quant_linear_generic`, `cuda_matmul_quant_dispatch`,
  `cuda_blockq_dequant/gemv`, `syn_bundle_block_quant`, unit-тесты gguf.
- Отладочные обходы: `SYN_GGUF_DENSE=1|<подстрока>` — не отдавать блоки
  (модель читает плотно и квантует сама), `SYN_BLOCKQ_GEMV=0` — малые M через
  деквант+GEMM. Ими и локализовались gather и перестановка.

## Чего этап 3 не даёт (честно)

- `qwen3moe` и `qwen3next`/`qwen35` — маппер и конвертация есть, прямого
  исполнения GGUF нет: у движка нет ветки Qwen-MoE, а гибрид с `.gguf`
  сквозным прогоном не проверялся (эталонного файла на машине нет).
- Сверка **логитов** с llama.cpp не делалась — только ответы и токенизация.
- Форматы ggml на Blackwell идут портируемыми ядрами (GEMV с деквантом в
  регистрах / деквант полосами + GEMM): Q8_0 0.6B — 1,7 с загрузки, скорость
  декода не замерялась. Перекодировка ggml → NVFP4/SQ при загрузке — этап 4.
- Шаблон чата рендерится с `keep_trailing_newline` (так было и для `.syn`):
  у Llama-3.2 в конце промпта лишний `\n`, HF его срезает. Не трогал —
  общее поведение фасада, не GGUF.
- mmproj (башня зрения) из GGUF не грузится напрямую — только через
  конвертацию, как раньше.
