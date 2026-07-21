# llama-server — справочник ключей командной строки

> Сгенерировано из вывода `llama-server --help` (llama.cpp).
> Используется в synthos как источник метаданных для чипов настроек моделей.

Ключи разбиты по логическим категориям. Для каждого указаны короткая форма, длинное имя (и алиасы), синопсис аргумента, описание из `--help`, переменная окружения (если есть) и значение по умолчанию.

---

## 1. Основные (служебные) ключи

| Ключ | Описание |
|------|----------|
| `-h, --help, --usage` | Вывести справку и выйти. |
| `--version` | Показать версию и сведения о сборке. |
| `--license` | Показать лицензию исходного кода и зависимостей. |
| `-cl, --cache-list` | Показать список моделей в кэше. |
| `--completion-bash` | Напечатать bash-скрипт автодополнения для llama.cpp. |
| `--list-devices` | Показать список доступных устройств и выйти. |

---

## 2. Потоки и CPU

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-t, --threads` | `N` | `LLAMA_ARG_THREADS` | `-1` | Число CPU-потоков при генерации. |
| `-tb, --threads-batch` | `N` | — | = `--threads` | Число потоков при батчинге и обработке промпта. |
| `-C, --cpu-mask` | `M` | — | `""` | CPU affinity mask (произвольно длинный hex). Дополняет `cpu-range`. |
| `-Cr, --cpu-range` | `lo-hi` | — | — | Диапазон CPU для affinity. Дополняет `--cpu-mask`. |
| `--cpu-strict` | `<0\|1>` | — | `0` | Строгое размещение потоков по CPU. |
| `--prio` | `N` | — | `0` | Приоритет процесса/потока: `-1` low, `0` normal, `1` medium, `2` high, `3` realtime. |
| `--poll` | `<0...100>` | — | `50` | Уровень polling для ожидания работы (0 = без polling). |
| `-Cb, --cpu-mask-batch` | `M` | — | = `--cpu-mask` | Affinity mask для batch-фазы. |
| `-Crb, --cpu-range-batch` | `lo-hi` | — | — | Диапазоны CPU для batch-фазы. |
| `--cpu-strict-batch` | `<0\|1>` | — | = `--cpu-strict` | Строгое размещение для batch-фазы. |
| `--prio-batch` | `N` | — | `0` | Приоритет для batch-фазы (0 normal, 1 medium, 2 high, 3 realtime). |
| `--poll-batch` | `<0\|1>` | — | = `--poll` | Polling для batch-фазы. |

---

## 3. Контекст и батчинг

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-c, --ctx-size` | `N` | `LLAMA_ARG_CTX_SIZE` | `0` (из модели) | Размер промпт-контекста. |
| `-n, --predict, --n-predict` | `N` | `LLAMA_ARG_N_PREDICT` | `-1` (∞) | Число генерируемых токенов. |
| `-b, --batch-size` | `N` | `LLAMA_ARG_BATCH` | `2048` | Логический максимальный размер батча. |
| `-ub, --ubatch-size` | `N` | `LLAMA_ARG_UBATCH` | `512` | Физический максимальный размер батча. |
| `--keep` | `N` | — | `0` | Сколько токенов из начального промпта оставить (-1 = все). |
| `--swa-full` | — | `LLAMA_ARG_SWA_FULL` | `false` | Использовать полноразмерный SWA-кэш. |
| `-fa, --flash-attn` | `[on\|off\|auto]` | `LLAMA_ARG_FLASH_ATTN` | `auto` | Flash Attention. |
| `--perf, --no-perf` | — | `LLAMA_ARG_PERF` | `false` | Внутренние тайминги libllama. |
| `-e, --escape, --no-escape` | — | — | `true` | Обрабатывать escape-последовательности (`\n`, `\t`, ...). |

---

## 4. RoPE / YaRN (attention scaling)

| Ключ | Аргумент | Env | Описание |
|------|----------|-----|----------|
| `--rope-scaling` | `{none,linear,yarn}` | `LLAMA_ARG_ROPE_SCALING_TYPE` | Метод RoPE frequency scaling (по умолч. linear или из модели). |
| `--rope-scale` | `N` | `LLAMA_ARG_ROPE_SCALE` | RoPE context scaling factor — расширяет контекст в N раз. |
| `--rope-freq-base` | `N` | `LLAMA_ARG_ROPE_FREQ_BASE` | RoPE base frequency (NTK-aware scaling). |
| `--rope-freq-scale` | `N` | `LLAMA_ARG_ROPE_FREQ_SCALE` | RoPE frequency scaling factor — расширяет контекст в 1/N раз. |
| `--yarn-orig-ctx` | `N` | `LLAMA_ARG_YARN_ORIG_CTX` | YaRN: исходный размер контекста модели (0 = training ctx). |
| `--yarn-ext-factor` | `N` | `LLAMA_ARG_YARN_EXT_FACTOR` | YaRN: коэффициент смешивания экстраполяции (default -1.00). |
| `--yarn-attn-factor` | `N` | `LLAMA_ARG_YARN_ATTN_FACTOR` | YaRN: scale sqrt(t) или магнитуда attention. |
| `--yarn-beta-slow` | `N` | `LLAMA_ARG_YARN_BETA_SLOW` | YaRN: high correction dim / alpha. |
| `--yarn-beta-fast` | `N` | `LLAMA_ARG_YARN_BETA_FAST` | YaRN: low correction dim / beta. |

---

## 5. KV cache

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-kvo, --kv-offload` / `-nkvo, --no-kv-offload` | — | `LLAMA_ARG_KV_OFFLOAD` | `enabled` | KV-cache offloading. |
| `--repack` / `-nr, --no-repack` | — | `LLAMA_ARG_REPACK` | `enabled` | Weight repacking. |
| `--no-host` | — | `LLAMA_ARG_NO_HOST` | `false` | Bypass host-буфер (разрешить доп. буферы). |
| `-ctk, --cache-type-k` | `TYPE` | `LLAMA_ARG_CACHE_TYPE_K` | `f16` | Тип данных KV-cache K. Значения: `f32, f16, bf16, q8_0, q4_0, q4_1, iq4_nl, q5_0, q5_1`. |
| `-ctv, --cache-type-v` | `TYPE` | `LLAMA_ARG_CACHE_TYPE_V` | `f16` | Тип данных KV-cache V. Значения как у `-ctk`. |
| `-dt, --defrag-thold` | `N` | `LLAMA_ARG_DEFRAG_THOLD` | — | **DEPRECATED**: порог дефрагментации KV-cache. |
| `-ctkd, --cache-type-k-draft` | `TYPE` | `LLAMA_ARG_CACHE_TYPE_K_DRAFT` | `f16` | KV-cache K для draft-модели. |
| `-ctvd, --cache-type-v-draft` | `TYPE` | `LLAMA_ARG_CACHE_TYPE_V_DRAFT` | `f16` | KV-cache V для draft-модели. |
| `-ctxcp, --ctx-checkpoints, --swa-checkpoints` | `N` | `LLAMA_ARG_CTX_CHECKPOINTS` | `32` | Максимум контекст-чекпоинтов на слот. |
| `-cpent, --checkpoint-every-n-tokens` | `N` | `LLAMA_ARG_CHECKPOINT_EVERY_NT` | `8192` | Создавать чекпоинт каждые N токенов (-1 = выкл). |
| `-cram, --cache-ram` | `N (MiB)` | `LLAMA_ARG_CACHE_RAM` | `8192` | Максимум кэша в MiB (-1 = без лимита, 0 = выкл). |
| `-kvu, --kv-unified` / `-no-kvu, --no-kv-unified` | — | `LLAMA_ARG_KV_UNIFIED` | `enabled if slots auto` | Единый KV-буфер для всех последовательностей. |
| `--cache-idle-slots, --no-cache-idle-slots` | — | `LLAMA_ARG_CACHE_IDLE_SLOTS` | `enabled` | Сохранять и очищать idle-слоты на новой задаче (требует unified KV + cache-ram). |
| `--context-shift, --no-context-shift` | — | `LLAMA_ARG_CONTEXT_SHIFT` | `disabled` | Context shift при бесконечной генерации. |

---

## 6. Память и NUMA

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `--mlock` | — | `LLAMA_ARG_MLOCK` | `false` | Не свопить модель из RAM. |
| `--mmap, --no-mmap` | — | `LLAMA_ARG_MMAP` | `enabled` | Memory-map модели. |
| `-dio, --direct-io` / `-ndio, --no-direct-io` | — | `LLAMA_ARG_DIO` | `disabled` | DirectIO, если доступно. |
| `--numa` | `TYPE` | `LLAMA_ARG_NUMA` | — | NUMA-оптимизация: `distribute`, `isolate`, `numactl`. |
| `--rpc` | `SERVERS` | `LLAMA_ARG_RPC` | — | Список RPC-серверов (host:port через запятую). |

---

## 7. Устройство / GPU

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-dev, --device` | `<dev1,dev2,..>` | `LLAMA_ARG_DEVICE` | — | Устройства для offload (`none` = без offload). |
| `-ot, --override-tensor` | `<pattern>=<buf-type>,...` | `LLAMA_ARG_OVERRIDE_TENSOR` | — | Переопределить buffer-type для тензоров. |
| `-cmoe, --cpu-moe` | — | `LLAMA_ARG_CPU_MOE` | — | MoE-веса хранить на CPU. |
| `-ncmoe, --n-cpu-moe` | `N` | `LLAMA_ARG_N_CPU_MOE` | — | MoE-веса первых N слоёв хранить на CPU. |
| `-ngl, --gpu-layers, --n-gpu-layers` | `N` | `LLAMA_ARG_N_GPU_LAYERS` | `auto` | Слоёв в VRAM: число, `auto` или `all`. |
| `-sm, --split-mode` | `{none,layer,row,tensor}` | `LLAMA_ARG_SPLIT_MODE` | `layer` | Как разбивать модель между GPU. `tensor` — экспериментально. |
| `-ts, --tensor-split` | `N0,N1,...` | `LLAMA_ARG_TENSOR_SPLIT` | — | Пропорции offload на каждую GPU (например `3,1`). |
| `-mg, --main-gpu` | `INDEX` | `LLAMA_ARG_MAIN_GPU` | `0` | Основная GPU (для `split-mode=none` или промежуточных результатов при `row`). |
| `-fit, --fit` | `[on\|off]` | `LLAMA_ARG_FIT` | `on` | Подгонять не заданные аргументы под память устройства. |
| `-fitt, --fit-target` | `MiB0,MiB1,...` | `LLAMA_ARG_FIT_TARGET` | `1024` | Целевой margin в MiB на устройство для `--fit`. |
| `-fitc, --fit-ctx` | `N` | `LLAMA_ARG_FIT_CTX` | `4096` | Минимальный ctx, устанавливаемый `--fit`. |
| `--check-tensors` | — | — | `false` | Проверить тензоры модели на недопустимые значения. |
| `--override-kv` | `KEY=TYPE:VALUE,...` | — | — | Переопределить метаданные модели (типы: `int`, `float`, `bool`, `str`). |
| `--op-offload, --no-op-offload` | — | — | `true` | Offload операций над host-тензорами на устройство. |

---

## 8. Загрузка модели и LoRA

| Ключ | Аргумент | Env | Описание |
|------|----------|-----|----------|
| `-m, --model` | `FNAME` | `LLAMA_ARG_MODEL` | Путь к модели (.gguf). |
| `-mu, --model-url` | `MODEL_URL` | `LLAMA_ARG_MODEL_URL` | URL для скачивания модели. |
| `-dr, --docker-repo` | `[<repo>/]<model>[:quant]` | `LLAMA_ARG_DOCKER_REPO` | Docker Hub репозиторий моделей (`ai/` по умолчанию). |
| `-hf, -hfr, --hf-repo` | `<user>/<model>[:quant]` | `LLAMA_ARG_HF_REPO` | Hugging Face репозиторий. По умолч. `Q4_K_M`. mmproj подхватывается автоматически. |
| `-hfd, -hfrd, --hf-repo-draft` | `<user>/<model>[:quant]` | `LLAMA_ARG_HFD_REPO` | То же для draft-модели. |
| `-hff, --hf-file` | `FILE` | `LLAMA_ARG_HF_FILE` | Конкретный файл HF (перезаписывает quant из `--hf-repo`). |
| `-hfv, -hfrv, --hf-repo-v` | `<user>/<model>[:quant]` | `LLAMA_ARG_HF_REPO_V` | HF-репо vocoder-модели. |
| `-hffv, --hf-file-v` | `FILE` | `LLAMA_ARG_HF_FILE_V` | Файл vocoder-модели на HF. |
| `-hft, --hf-token` | `TOKEN` | `HF_TOKEN` | Access-token Hugging Face. |
| `--lora` | `FNAME` | — | Путь к LoRA-адаптеру (через запятую — несколько). |
| `--lora-scaled` | `FNAME:SCALE,...` | — | LoRA с user-defined scale. |
| `--control-vector` | `FNAME` | — | Control vector. |
| `--control-vector-scaled` | `FNAME:SCALE,...` | — | Control vector с scale. |
| `--control-vector-layer-range` | `START END` | — | Диапазон слоёв для control vector (start/end inclusive). |

---

## 9. Логирование

| Ключ | Аргумент | Env | Описание |
|------|----------|-----|----------|
| `--log-disable` | — | — | Отключить логи. |
| `--log-file` | `FNAME` | `LLAMA_LOG_FILE` | Писать логи в файл. |
| `--log-colors` | `[on\|off\|auto]` | `LLAMA_LOG_COLORS` | Цветные логи (`auto` — если вывод в терминал). |
| `-v, --verbose, --log-verbose` | — | — | Уровень verbosity в бесконечность (все сообщения). |
| `--offline` | — | `LLAMA_OFFLINE` | Offline-режим (принудительно из кэша, без сети). |
| `-lv, --verbosity, --log-verbosity` | `N` | `LLAMA_LOG_VERBOSITY` | Порог verbosity (0 output, 1 error, 2 warning, 3 info, 4 debug). По умолч. 3. |
| `--log-prefix` | — | `LLAMA_LOG_PREFIX` | Префикс в сообщениях. |
| `--log-timestamps` | — | `LLAMA_LOG_TIMESTAMPS` | Timestamps в сообщениях. |

---

## 10. Sampling (сэмплинг)

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `--samplers` | `SAMPLERS` | — | `penalties;dry;top_n_sigma;top_k;typ_p;top_p;min_p;xtc;temperature` | Сэмплеры через `;` в порядке применения. |
| `-s, --seed` | `SEED` | — | `-1` (random) | RNG seed. |
| `--sampler-seq, --sampling-seq` | `SEQUENCE` | — | `edskypmxt` | Упрощённая последовательность сэмплеров. |
| `--ignore-eos` | — | — | — | Игнорировать EOS-токен (эквивалент `--logit-bias EOS-inf`). |
| `--temp, --temperature` | `N` | — | `0.80` | Температура. |
| `--top-k` | `N` | `LLAMA_ARG_TOP_K` | `40` (0 = off) | Top-K сэмплинг. |
| `--top-p` | `N` | — | `0.95` (1.0 = off) | Top-P сэмплинг. |
| `--min-p` | `N` | — | `0.05` (0.0 = off) | Min-P сэмплинг. |
| `--top-nsigma, --top-n-sigma` | `N` | — | `-1.00` (off) | Top-N-Sigma. |
| `--xtc-probability` | `N` | — | `0.00` | XTC probability. |
| `--xtc-threshold` | `N` | — | `0.10` (1.0 = off) | XTC threshold. |
| `--typical, --typical-p` | `N` | — | `1.00` (off) | Locally typical sampling. |
| `--repeat-last-n` | `N` | — | `64` | Сколько последних токенов штрафовать (-1 = ctx_size, 0 = off). |
| `--repeat-penalty` | `N` | — | `1.00` (off) | Штраф за повтор последовательностей. |
| `--presence-penalty` | `N` | — | `0.00` (off) | Presence penalty. |
| `--frequency-penalty` | `N` | — | `0.00` (off) | Frequency penalty. |
| `--dry-multiplier` | `N` | — | `0.00` (off) | DRY multiplier. |
| `--dry-base` | `N` | — | `1.75` | DRY base. |
| `--dry-allowed-length` | `N` | — | `2` | DRY allowed length. |
| `--dry-penalty-last-n` | `N` | — | `-1` (ctx) | DRY penalty last N (0 = off). |
| `--dry-sequence-breaker` | `STRING` | — | `\n :"*` | Sequence breakers для DRY; `none` — пусто. |
| `--adaptive-target` | `N` | — | `-1.00` (off) | Adaptive-p: вероятность-цель (0.0–1.0). |
| `--adaptive-decay` | `N` | — | `0.90` | Adaptive-p: decay rate (0.0–0.99). |
| `--dynatemp-range` | `N` | — | `0.00` (off) | Dynamic temperature range. |
| `--dynatemp-exp` | `N` | — | `1.00` | Dynamic temperature exponent. |
| `--mirostat` | `N` | — | `0` (off) | Mirostat: `1` — v1, `2` — v2.0. Игнорирует Top-K/Nucleus/Typical. |
| `--mirostat-lr` | `N` | — | `0.10` | Mirostat learning rate (eta). |
| `--mirostat-ent` | `N` | — | `5.00` | Mirostat target entropy (tau). |
| `-l, --logit-bias` | `TOKEN_ID(+/-)BIAS` | — | — | Изменить likelihood токена. `15043+1` — повысить, `15043-1` — понизить. |
| `--grammar` | `GRAMMAR` | — | — | BNF-подобная грамматика для ограничения генерации. |
| `--grammar-file` | `FNAME` | — | — | Файл с грамматикой. |
| `-j, --json-schema` | `SCHEMA` | — | — | JSON Schema для ограничения генерации. |
| `-jf, --json-schema-file` | `FILE` | — | — | Файл с JSON Schema. |
| `-bs, --backend-sampling` | — | `LLAMA_ARG_BACKEND_SAMPLING` | `disabled` | Backend sampling (экспериментально). |

---

## 11. Сервер (HTTP)

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `--host` | `HOST` | `LLAMA_ARG_HOST` | `127.0.0.1` | IP-адрес (или UNIX-socket при `.sock`). |
| `--port` | `PORT` | `LLAMA_ARG_PORT` | `8080` | Порт. |
| `--reuse-port` | — | `LLAMA_ARG_REUSE_PORT` | `disabled` | Разрешить несколько сокетов на одном порту. |
| `--path` | `PATH` | `LLAMA_ARG_STATIC_PATH` | — | Путь к статике. |
| `--api-prefix` | `PREFIX` | `LLAMA_ARG_API_PREFIX` | — | Префикс пути сервера (без trailing slash). |
| `--webui-config` | `JSON` | `LLAMA_ARG_WEBUI_CONFIG` | — | JSON с дефолтными настройками WebUI. |
| `--webui-config-file` | `PATH` | `LLAMA_ARG_WEBUI_CONFIG_FILE` | — | JSON-файл с настройками WebUI. |
| `--webui-mcp-proxy, --no-webui-mcp-proxy` | — | `LLAMA_ARG_WEBUI_MCP_PROXY` | `disabled` | **экспериментально**: MCP CORS-proxy. |
| `--tools` | `TOOL1,TOOL2,...` | `LLAMA_ARG_TOOLS` | `no tools` | **экспериментально**: built-in tools для AI-агентов. Возможные: `read_file`, `file_glob_search`, `grep_search`, `exec_shell_command`, `write_file`, `edit_file`, `apply_diff`. `all` = всё. |
| `--webui, --no-webui` | — | `LLAMA_ARG_WEBUI` | `enabled` | Включать Web UI. |
| `--embedding, --embeddings` | — | `LLAMA_ARG_EMBEDDINGS` | `disabled` | Только embedding. |
| `--rerank, --reranking` | — | `LLAMA_ARG_RERANKING` | `disabled` | Reranking endpoint. |
| `--api-key` | `KEY` | `LLAMA_API_KEY` | — | API-ключ(и) (через запятую — несколько). |
| `--api-key-file` | `FNAME` | — | — | Файл с API-ключами. |
| `--ssl-key-file` | `FNAME` | `LLAMA_ARG_SSL_KEY_FILE` | — | PEM SSL private key. |
| `--ssl-cert-file` | `FNAME` | `LLAMA_ARG_SSL_CERT_FILE` | — | PEM SSL certificate. |
| `-to, --timeout` | `N` | `LLAMA_ARG_TIMEOUT` | `600` | Server read/write timeout (секунды). |
| `--threads-http` | `N` | `LLAMA_ARG_THREADS_HTTP` | `-1` | Потоков для HTTP. |
| `--cache-prompt, --no-cache-prompt` | — | `LLAMA_ARG_CACHE_PROMPT` | `enabled` | Prompt caching. |
| `--cache-reuse` | `N` | `LLAMA_ARG_CACHE_REUSE` | `0` | Мин. размер чанка для повторного использования через KV shifting. |
| `--metrics` | — | `LLAMA_ARG_ENDPOINT_METRICS` | `disabled` | Prometheus endpoint. |
| `--props` | — | `LLAMA_ARG_ENDPOINT_PROPS` | `disabled` | Разрешить изменять свойства через POST `/props`. |
| `--slots, --no-slots` | — | `LLAMA_ARG_ENDPOINT_SLOTS` | `enabled` | Slots monitoring endpoint. |
| `--slot-save-path` | `PATH` | — | `disabled` | Путь для сохранения KV-cache слотов. |
| `--media-path` | `PATH` | — | `disabled` | Директория для локальных медиафайлов (доступ через `file://`). |
| `--models-dir` | `PATH` | `LLAMA_ARG_MODELS_DIR` | `disabled` | Директория моделей для router-сервера. |
| `--models-preset` | `PATH` | `LLAMA_ARG_MODELS_PRESET` | `disabled` | INI с пресетами моделей. |
| `--models-max` | `N` | `LLAMA_ARG_MODELS_MAX` | `4` | Максимум одновременно загруженных моделей (0 = без лимита). |
| `--models-autoload, --no-models-autoload` | — | `LLAMA_ARG_MODELS_AUTOLOAD` | `enabled` | Автозагрузка моделей router-сервером. |

---

## 12. Chat template / Reasoning / Jinja

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `--jinja, --no-jinja` | — | `LLAMA_ARG_JINJA` | `enabled` | Jinja template engine для чата. |
| `--reasoning-format` | `FORMAT` | `LLAMA_ARG_THINK` | `auto` | Как возвращать thought tags: `none`, `deepseek`, `deepseek-legacy`. |
| `-rea, --reasoning` | `[on\|off\|auto]` | `LLAMA_ARG_REASONING` | `auto` | Reasoning/thinking в чате. |
| `--reasoning-budget` | `N` | `LLAMA_ARG_THINK_BUDGET` | `-1` (unrestricted) | Бюджет токенов на reasoning (0 = немедленный конец). |
| `--reasoning-budget-message` | `MESSAGE` | `LLAMA_ARG_THINK_BUDGET_MESSAGE` | — | Сообщение, вставляемое перед end-of-thinking при исчерпании бюджета. |
| `--chat-template` | `JINJA_TEMPLATE` | `LLAMA_ARG_CHAT_TEMPLATE` | из модели | Кастомный jinja-шаблон. См. список встроенных ниже. |
| `--chat-template-file` | `JINJA_TEMPLATE_FILE` | `LLAMA_ARG_CHAT_TEMPLATE_FILE` | из модели | Файл с jinja-шаблоном. |
| `--chat-template-kwargs` | `STRING` | `LLAMA_CHAT_TEMPLATE_KWARGS` | — | JSON-параметры для парсера шаблона (`{"key":"value"}`). |
| `--skip-chat-parsing, --no-skip-chat-parsing` | — | `LLAMA_ARG_SKIP_CHAT_PARSING` | `disabled` | Принудительно использовать pure content parser. |
| `--prefill-assistant, --no-prefill-assistant` | — | `LLAMA_ARG_PREFILL_ASSISTANT` | `enabled` | Prefill assistant-ответа, если последнее сообщение от ассистента. |
| `-sps, --slot-prompt-similarity` | `SIMILARITY` | — | `0.10` | Сходство промпта со слотом, чтобы использовать его (0.0 = off). |
| `--lora-init-without-apply` | — | — | `disabled` | Загрузить LoRA без применения (применить позже через POST `/lora-adapters`). |
| `--sleep-idle-seconds` | `SECONDS` | — | `-1` (off) | После скольких секунд idle сервер уходит в sleep. |
| `-a, --alias` | `STRING` | `LLAMA_ARG_ALIAS` | — | Имя-алиасы модели через запятую (используются API). |
| `--tags` | `STRING` | `LLAMA_ARG_TAGS` | — | Теги модели через запятую (informational). |

**Встроенные chat-template имена**: `bailing, bailing-think, bailing2, chatglm3, chatglm4, chatml, command-r, deepseek, deepseek-ocr, deepseek2, deepseek3, exaone-moe, exaone3, exaone4, falcon3, gemma, gigachat, glmedge, gpt-oss, granite, granite-4.0, grok-2, hunyuan-dense, hunyuan-moe, hunyuan-ocr, kimi-k2, llama2, llama2-sys, llama2-sys-bos, llama2-sys-strip, llama3, llama4, megrez, minicpm, mistral-v1, mistral-v3, mistral-v3-tekken, mistral-v7, mistral-v7-tekken, monarch, openchat, orion, pangu-embedded, phi3, phi4, rwkv-world, seed_oss, smolvlm, solar-open, vicuna, vicuna-orca, yandex, zephyr`.

---

## 13. Multimodal (mmproj / Vision)

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-mm, --mmproj` | `FILE` | `LLAMA_ARG_MMPROJ` | — | Путь к multimodal projector-файлу. При использовании `-hf` — опционально. |
| `-mmu, --mmproj-url` | `URL` | `LLAMA_ARG_MMPROJ_URL` | — | URL к mmproj-файлу. |
| `--mmproj-auto, --no-mmproj, --no-mmproj-auto` | — | `LLAMA_ARG_MMPROJ_AUTO` | `enabled` | Использовать mmproj-файл, если доступен. |
| `--mmproj-offload, --no-mmproj-offload` | — | `LLAMA_ARG_MMPROJ_OFFLOAD` | `enabled` | GPU-offloading для mmproj. |
| `--image-min-tokens` | `N` | `LLAMA_ARG_IMAGE_MIN_TOKENS` | из модели | Мин. число токенов на изображение (dynamic-resolution vision). |
| `--image-max-tokens` | `N` | `LLAMA_ARG_IMAGE_MAX_TOKENS` | из модели | Макс. число токенов на изображение. |

---

## 14. Параллелизм / Continuous batching

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-np, --parallel` | `N` | `LLAMA_ARG_N_PARALLEL` | `-1` (auto) | Число server-slots. |
| `-cb, --cont-batching` / `-nocb, --no-cont-batching` | — | `LLAMA_ARG_CONT_BATCHING` | `enabled` | Continuous (dynamic) batching. |

---

## 15. Speculative decoding (draft-модель)

| Ключ | Аргумент | Env | По умолчанию | Описание |
|------|----------|-----|--------------|----------|
| `-otd, --override-tensor-draft` | `<pattern>=<buf-type>,...` | — | — | Override buffer-type для draft-модели. |
| `-cmoed, --cpu-moe-draft` | — | `LLAMA_ARG_CPU_MOE_DRAFT` | — | MoE-веса draft-модели на CPU. |
| `-ncmoed, --n-cpu-moe-draft` | `N` | `LLAMA_ARG_N_CPU_MOE_DRAFT` | — | Первые N слоёв draft-MoE на CPU. |
| `-td, --threads-draft` | `N` | — | = `--threads` | Потоки при генерации draft. |
| `-tbd, --threads-batch-draft` | `N` | — | = `--threads-draft` | Потоки при batch draft. |
| `--draft, --draft-n, --draft-max` | `N` | `LLAMA_ARG_DRAFT_MAX` | `16` | Число токенов для drafting. |
| `--draft-min, --draft-n-min` | `N` | `LLAMA_ARG_DRAFT_MIN` | `0` | Мин. токенов drafting. |
| `--draft-p-min` | `P` | `LLAMA_ARG_DRAFT_P_MIN` | `0.75` | Мин. greedy-probability draft. |
| `-cd, --ctx-size-draft` | `N` | `LLAMA_ARG_CTX_SIZE_DRAFT` | `0` (из модели) | ctx-size для draft. |
| `-devd, --device-draft` | `<dev1,dev2,..>` | — | — | Устройства для offload draft. |
| `-ngld, --gpu-layers-draft, --n-gpu-layers-draft` | `N` | `LLAMA_ARG_N_GPU_LAYERS_DRAFT` | `auto` | Слоёв draft-модели в VRAM. |
| `-md, --model-draft` | `FNAME` | `LLAMA_ARG_MODEL_DRAFT` | — | Путь к draft-модели. |
| `--spec-replace` | `TARGET DRAFT` | — | — | Транслировать строку TARGET в DRAFT, если draft несовместим. |
| `--spec-type` | `[none\|ngram-cache\|ngram-simple\|ngram-map-k\|ngram-map-k4v\|ngram-mod]` | `LLAMA_ARG_SPEC_TYPE` | `none` | Тип speculative-decoding, если draft-модель не указана. |
| `--spec-ngram-size-n` | `N` | — | `12` | Длина lookup n-gram. |
| `--spec-ngram-size-m` | `N` | — | `48` | Длина draft m-gram. |
| `--spec-ngram-min-hits` | `N` | — | `1` | Мин. хитов для ngram-map. |

---

## 16. TTS / Voice

| Ключ | Аргумент | Описание |
|------|----------|----------|
| `-mv, --model-vocoder` | `FNAME` | Vocoder-модель для генерации аудио. |
| `--tts-use-guide-tokens` | — | Guide tokens для улучшения TTS word recall. |

---

## 17. Другие параметры / infill / lookup

| Ключ | Аргумент | Env | Описание |
|------|----------|-----|----------|
| `-lcs, --lookup-cache-static` | `FNAME` | — | Статический lookup-cache для lookup-decoding (не обновляется). |
| `-lcd, --lookup-cache-dynamic` | `FNAME` | — | Динамический lookup-cache (обновляется генерацией). |
| `-r, --reverse-prompt` | `PROMPT` | — | Остановить генерацию на PROMPT. |
| `-sp, --special` | — | — | Вывод special tokens (default `false`). |
| `--warmup, --no-warmup` | — | — | Warmup пустым прогоном (default `enabled`). |
| `--spm-infill` | — | — | Suffix/Prefix/Middle для infill вместо Prefix/Suffix/Middle (default off). |
| `--pooling` | `{none,mean,cls,last,rank}` | `LLAMA_ARG_POOLING` | Тип pooling для embeddings. |

---

## 18. Default presets (готовые модели из сети)

| Ключ | Описание |
|------|----------|
| `--embd-gemma-default` | EmbeddingGemma (скачивает). |
| `--fim-qwen-1.5b-default` | Qwen 2.5 Coder 1.5B. |
| `--fim-qwen-3b-default` | Qwen 2.5 Coder 3B. |
| `--fim-qwen-7b-default` | Qwen 2.5 Coder 7B. |
| `--fim-qwen-7b-spec` | Qwen 2.5 Coder 7B + 0.5B draft. |
| `--fim-qwen-14b-spec` | Qwen 2.5 Coder 14B + 0.5B draft. |
| `--fim-qwen-30b-default` | Qwen 3 Coder 30B A3B Instruct. |
| `--gpt-oss-20b-default` | gpt-oss-20b. |
| `--gpt-oss-120b-default` | gpt-oss-120b. |
| `--vision-gemma-4b-default` | Gemma 3 4B QAT. |
| `--vision-gemma-12b-default` | Gemma 3 12B QAT. |
| `--spec-default` | Дефолтный speculative-decoding config. |
