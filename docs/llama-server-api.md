# HTTP API `llama-server`

Документ описывает весь публичный HTTP-API `llama-server` из проекта
[ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp).

Источник: `tools/server/README.md` и `tools/server/server.cpp` (регистрация
роутов строки 172–206) из ветки `master` на момент коммита
`c78fb909b23758f5e418cf98a69bc8a0ef142fb8`. Локально установленная версия —
`llama.cpp-cuda-git b8895.r7.550d684bd1`.

> Документ собран для написания типизированного клиента в `synthos` — поля и
> форматы сверены с кодом сервера, а не только с README.

---

## Оглавление

1. [Общие сведения](#общие-сведения)
2. [Аутентификация](#аутентификация)
3. [Форматы ошибок](#форматы-ошибок)
4. [Server-Sent Events](#server-sent-events-sse)
5. [Sampling-параметры](#sampling-параметры)
6. [Endpoints](#endpoints)
   - Мониторинг и метаданные
     - [`GET /health`](#get-health)
     - [`GET /v1/models`](#get-v1models)
     - [`GET /models`](#get-models-router-mode)
     - [`GET /props`](#get-props)
     - [`POST /props`](#post-props)
     - [`GET /slots`](#get-slots)
     - [`POST /slots/{id}?action=save|restore|erase`](#post-slotsidaction)
     - [`GET /metrics`](#get-metrics)
   - Inference
     - [`POST /completion`](#post-completion)
     - [`POST /v1/completions`](#post-v1completions)
     - [`POST /chat/completions`, `/v1/chat/completions`](#post-chatcompletions)
     - [`POST /v1/responses`](#post-v1responses)
     - [`POST /v1/messages`](#post-v1messages-anthropic)
     - [`POST /v1/messages/count_tokens`](#post-v1messagescount_tokens)
     - [`POST /v1/audio/transcriptions`](#post-v1audiotranscriptions)
     - [`POST /infill`](#post-infill)
   - Embeddings и rerank
     - [`POST /embedding`](#post-embedding)
     - [`POST /embeddings`](#post-embeddings)
     - [`POST /v1/embeddings`](#post-v1embeddings)
     - [`POST /rerank`, `/v1/rerank`](#post-rerank)
   - Токенизация
     - [`POST /tokenize`](#post-tokenize)
     - [`POST /detokenize`](#post-detokenize)
     - [`POST /apply-template`](#post-apply-template)
   - LoRA
     - [`GET /lora-adapters`](#get-lora-adapters)
     - [`POST /lora-adapters`](#post-lora-adapters)
   - Router mode (без модели)
     - [`POST /models/load`](#post-modelsload)
     - [`POST /models/unload`](#post-modelsunload)
7. [Приложение А. Матрица совместимости с OpenAI](#приложение-а-матрица-совместимости-с-openai)
8. [Приложение Б. Поля `generation_settings`](#приложение-б-поля-generation_settings)

---

## Общие сведения

- Базовый URL: `http://<host>:<port>` (по умолчанию `127.0.0.1:8080`).
- Все JSON-тела — UTF-8, `Content-Type: application/json`.
- Запросы без модели (router mode, `llama-server` без `-m`) поддерживают только
  часть API — см. `server.cpp:130–169`.
- Public endpoints (без проверки API-ключа): `GET /health`, `GET /v1/health`,
  `GET /models`, `GET /v1/models` (`server.cpp:172–178`).
- Все остальные endpoints требуют валидного API-ключа, если сервер запущен
  с `--api-key` / `--api-key-file` / `LLAMA_API_KEY`.

### Регистрация роутов (для сверки)

| Метод | Путь | Обработчик (поле `server_routes`) | server.cpp |
|-------|------|------------------------------------|------------|
| GET   | `/health`, `/v1/health`            | `get_health`                 | 172–173 |
| GET   | `/metrics`                         | `get_metrics`                | 174 |
| GET   | `/props`                           | `get_props`                  | 175 |
| POST  | `/props`                           | `post_props`                 | 176 |
| GET   | `/models`, `/v1/models`            | `get_models`                 | 177–178 |
| POST  | `/completion`, `/completions`      | `post_completions`           | 179–180 |
| POST  | `/v1/completions`                  | `post_completions_oai`       | 181 |
| POST  | `/chat/completions`, `/v1/chat/completions` | `post_chat_completions` | 182–183 |
| POST  | `/v1/responses`, `/responses`      | `post_responses_oai`         | 184–185 |
| POST  | `/v1/audio/transcriptions`, `/audio/transcriptions` | `post_transcriptions_oai` | 186–187 |
| POST  | `/v1/messages`                     | `post_anthropic_messages`    | 188 |
| POST  | `/v1/messages/count_tokens`        | `post_anthropic_count_tokens`| 189 |
| POST  | `/infill`                          | `post_infill`                | 190 |
| POST  | `/embedding`, `/embeddings`        | `post_embeddings`            | 191–192 |
| POST  | `/v1/embeddings`                   | `post_embeddings_oai`        | 193 |
| POST  | `/rerank`, `/reranking`, `/v1/rerank`, `/v1/reranking` | `post_rerank` | 194–197 |
| POST  | `/tokenize`                        | `post_tokenize`              | 198 |
| POST  | `/detokenize`                      | `post_detokenize`            | 199 |
| POST  | `/apply-template`                  | `post_apply_template`        | 200 |
| GET   | `/lora-adapters`                   | `get_lora_adapters`          | 202 |
| POST  | `/lora-adapters`                   | `post_lora_adapters`         | 203 |
| GET   | `/slots`                           | `get_slots`                  | 205 |
| POST  | `/slots/:id_slot`                  | `post_slots`                 | 206 |
| POST  | `/models/load`, `/models/unload`   | router-only                  | 168–169 |
| GET / POST | `/tools`                      | internal, Web UI only        | 223–224 |

---

## Аутентификация

Запуск сервера:

```sh
llama-server --api-key <key>
# или
LLAMA_API_KEY=<key> llama-server
```

Клиент должен добавить один из заголовков:

- `Authorization: Bearer <key>` — основной способ.
- `x-api-key: <key>` — принимается для совместимости с Anthropic API
  (`/v1/messages`).

Query-параметр `api_key` не поддерживается.

Ответ на отсутствующий/неверный ключ:

```json
{
  "error": {
    "code": 401,
    "message": "Invalid API Key",
    "type": "authentication_error"
  }
}
```

---

## Форматы ошибок

Все ошибки приходят в OpenAI-совместимом виде:

```json
{
  "error": {
    "code": <int: HTTP-статус>,
    "message": "<текст>",
    "type": "<string: см. ниже>"
  }
}
```

Ключевые `type`:

| `type`                    | Когда                                            |
|---------------------------|--------------------------------------------------|
| `invalid_request_error`   | Неверный JSON, пустое поле и т.п. (400)          |
| `authentication_error`    | Неверный API-ключ (401)                          |
| `permission_error`        | Доступ запрещён (403)                            |
| `not_found_error`         | Неизвестный endpoint (404)                       |
| `server_error`            | Внутренняя ошибка (500)                          |
| `not_supported_error`     | Endpoint выключен (`--no-slots`, без `--metrics`) (501) |
| `unavailable_error`       | Модель грузится / сервер спит (503)              |

При стриминге ошибка приходит как SSE-событие с именем `error` — см.
[Server-Sent Events](#server-sent-events-sse).

---

## Server-Sent Events (SSE)

Стриминг используют `POST /completion`, `/v1/completions`, `/chat/completions`,
`/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/infill` при
`"stream": true`.

- `Content-Type: text/event-stream`.
- Событие — одна логическая единица, отделённая пустой строкой (`\n\n`).
- Формат события:
  ```
  event: <name>
  data: <json>
  
  ```
  Поле `event:` опционально (по умолчанию `message` / `data`). Сервер
  использует имена `data` и `error` (см. `server.cpp:handle_completions_impl`).
- Специальная последняя строка для OpenAI-совместимых endpoint’ов:
  ```
  data: [DONE]
  ```
  Для `/completion` (native) `[DONE]` не присылается — последний чанк имеет
  `"stop": true` и содержит финальные поля.
- На ошибку:
  ```
  event: error
  data: {"error":{"code":500,"message":"...","type":"server_error"}}
  ```

Парсеры клиента должны уметь обрабатывать частичные чанки (TCP может разрезать
событие в любом байте), `keep-alive` пустые строки (`\n`) и произвольный порядок
полей `event:`/`data:`.

---

## Sampling-параметры

Общие для `/completion`, `/v1/completions`, `/chat/completions`, `/infill`,
`/v1/responses` (если не указано иное).

| Поле                      | Тип          | По умолч.          | Описание |
|---------------------------|--------------|--------------------|----------|
| `temperature`             | float        | `0.8`              | Случайность. |
| `dynatemp_range`          | float        | `0.0`              | Диапазон динамической температуры (±). |
| `dynatemp_exponent`       | float        | `1.0`              | Экспонента динамической температуры. |
| `top_k`                   | int          | `40`               | Top-K. |
| `top_p`                   | float        | `0.95`             | Nucleus. |
| `min_p`                   | float        | `0.05`             | Min-P. |
| `typical_p`               | float        | `1.0`              | Locally typical. |
| `top_n_sigma`             | float        | `-1.0`             | Top-N-Sigma (только `/slots`; опционально). |
| `xtc_probability`         | float        | `0.0`              | Шанс удаления токена XTC. |
| `xtc_threshold`           | float        | `0.1`              | Порог XTC (> 0.5 отключает). |
| `mirostat`                | int (0/1/2)  | `0`                | 0 — off, 1 — v1, 2 — v2. Игнорирует top-K/nucleus/typical. |
| `mirostat_tau`            | float        | `5.0`              | Целевая энтропия. |
| `mirostat_eta`            | float        | `0.1`              | Learning rate. |
| `repeat_penalty`          | float        | `1.0` (CLI `1.1`)  | Штраф за повторы. |
| `repeat_last_n`           | int          | `64`               | Окно штрафа, `-1` — весь контекст, `0` — off. |
| `presence_penalty`        | float        | `0.0`              | Alpha-presence penalty. |
| `frequency_penalty`       | float        | `0.0`              | Alpha-frequency penalty. |
| `dry_multiplier`          | float        | `0.0`              | DRY penalty (off = 0). |
| `dry_base`                | float        | `1.75`             | Основание DRY. |
| `dry_allowed_length`      | int          | `2`                | Порог повторения, дальше — экспонента. |
| `dry_penalty_last_n`      | int          | `-1`               | Окно DRY, `-1` — весь контекст. |
| `dry_sequence_breakers`   | string[]     | `["\n", ":", "\"", "*"]` | Разделители DRY. |
| `grammar`                 | string       | `""`               | BNF-грамматика. |
| `grammar_lazy`            | bool         | `false`            | Применять грамматику лениво. |
| `json_schema`             | object       | `{}`               | JSON Schema для ограничения вывода. |
| `samplers`                | string[]     | `["penalties","dry","top_k","typ_p","top_p","min_p","xtc","temperature"]` | Порядок сэмплеров. |
| `seed`                    | int          | `-1` (rand)        | Random seed. |
| `ignore_eos`              | bool         | `false`            | Игнорировать EOS. |
| `logit_bias`              | см. ниже     | `[]`               | Bias токенов. |
| `n_probs`                 | int          | `0`                | Сколько лучших вариантов возвращать. |
| `min_keep`                | int          | `0`                | Минимум токенов после сэмплера. |
| `t_max_predict_ms`        | int          | `0`                | Лимит времени генерации в мс (0 — off). |
| `n_predict` / `max_tokens`| int          | `-1` (∞)           | Лимит токенов ответа. |
| `n_keep`                  | int          | `0`                | Сколько токенов prompt’а сохранять при сдвиге. |
| `n_indent`                | int          | `0`                | Минимальный отступ (для code completion). |
| `n_cmpl`                  | int          | `1`                | Сколько завершений на запрос. |
| `n_cache_reuse`           | int          | `0`                | Размер чанка для KV-шифтинга из кэша. |
| `stop`                    | string[]     | `[]`               | Стоп-слова. |
| `stream`                  | bool         | `false`            | SSE-стриминг. |
| `timings_per_token`       | bool         | `false`            | Вложить `timings` в каждый чанк. |
| `return_progress`         | bool         | `false`            | Вернуть `prompt_progress` в стриме. |
| `return_tokens`           | bool         | `false`            | Отдать массив `tokens` (id). |
| `post_sampling_probs`     | bool         | `false`            | Вероятности пост-сэмплинга. |
| `cache_prompt`            | bool         | `true`             | Переиспользовать KV. |
| `id_slot`                 | int          | `-1`               | Явный слот, `-1` — idle. |
| `response_fields`         | string[]     | `[]`               | Ограничить набор полей ответа (поддержка `a/b` — «разнесение»). |
| `lora`                    | `[{id,scale}]` | `[]`             | LoRA per-request. |
| `image_data` (legacy)     | `[{data,id}]` | `[]`              | Устарело; для мультимодалки — `multimodal_data`. |

Формат `logit_bias`:

- Массив пар: `[[<token_id|string>, <bias|false>], ...]`, например
  `[[15043, 1.0]]`, `[["Hello", -0.5]]`, `[[15043, false]]`.
- Или OpenAI-объект: `{"<id|string>": <bias>, ...}`.

---

## Endpoints

### GET `/health`

Публичный. `/v1/health` — синоним.

- `200`: `{"status": "ok"}` — модель загружена, сервер готов.
- `503`: `{"error":{"code":503,"message":"Loading model","type":"unavailable_error"}}`.

---

### GET `/v1/models`

OpenAI-совместимый список моделей. У `llama-server` всегда один элемент
(текущая загруженная модель).

```json
{
  "object": "list",
  "data": [{
    "id": "../models/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
    "object": "model",
    "created": 1735142223,
    "owned_by": "llamacpp",
    "meta": {
      "vocab_type": 2,
      "n_vocab": 128256,
      "n_ctx_train": 131072,
      "n_embd": 4096,
      "n_params": 8030261312,
      "size": 4912898304
    }
  }]
}
```

Поле `meta` может быть `null`, пока модель ещё грузится. Alias задаётся
ключом CLI `--alias`.

---

### GET `/models` (router mode)

Только в router mode (запуск `llama-server` без `-m`).

```json
{
  "data": [{
    "id": "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M",
    "in_cache": true,
    "path": "/.../gemma-3-4b-it-Q4_K_M.gguf",
    "status": {
      "value": "loaded",
      "args": ["llama-server", "-ctx", "4096"]
    }
  }]
}
```

`status.value` ∈ `unloaded | loading | loaded | sleeping`. При `failed: true`
дополнительно — `exit_code`.

---

### GET `/props`

Не требует `--props` (только POST требует). Пример (сокращённый):

```json
{
  "default_generation_settings": { /* server_slot */ },
  "total_slots": 1,
  "model_path": "../models/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
  "chat_template": "<jinja ...>",
  "chat_template_caps": { /* bool-флаги возможностей шаблона */ },
  "modalities": { "vision": false },
  "media_marker": "<__media_YoNhud46VdDqbuFmKYEO9PY7A4ARzRfg__>",
  "build_info": "b5678-abcdef0",
  "is_sleeping": false
}
```

В router mode может приниматься `?model=<id>` и `?autoload=true|false`
(`tools/server/README.md:1583–1603`).

Поля `default_generation_settings` полностью соответствуют объекту слота
(см. [Приложение Б](#приложение-б-поля-generation_settings)).

---

### POST `/props`

Требует запуск с `--props`. На момент написания документа обработчик
принимает JSON, но список поддерживаемых ключей не документирован.
Рекомендуется слать пустой объект `{}` и оперативно смотреть `GET /props`.

---

### GET `/slots`

Возвращает массив объектов-слотов. Включён по умолчанию, выключается
`--no-slots`. Поддерживает query-param `?fail_on_no_slot=1` — тогда при
отсутствии свободного слота отвечает 503.

Пример одного элемента (двух-слотного сервера — см. README строки 882–1015):

```json
{
  "id": 0,
  "id_task": 135,
  "n_ctx": 65536,
  "speculative": false,
  "is_processing": true,
  "params": {
    "n_predict": -1,
    "seed": 4294967295,
    "temperature": 0.8,
    "top_k": 40,
    "top_p": 0.95,
    "min_p": 0.05,
    "top_n_sigma": -1.0,
    "xtc_probability": 0.0,
    "xtc_threshold": 0.1,
    "typical_p": 1.0,
    "repeat_last_n": 64,
    "repeat_penalty": 1.0,
    "presence_penalty": 0.0,
    "frequency_penalty": 0.0,
    "dry_multiplier": 0.0,
    "dry_base": 1.75,
    "dry_allowed_length": 2,
    "dry_penalty_last_n": 131072,
    "mirostat": 0,
    "mirostat_tau": 5.0,
    "mirostat_eta": 0.1,
    "max_tokens": -1,
    "n_keep": 0,
    "n_discard": 0,
    "ignore_eos": false,
    "stream": true,
    "n_probs": 0,
    "min_keep": 0,
    "chat_format": "GPT-OSS",
    "reasoning_format": "none",
    "reasoning_in_content": false,
    "generation_prompt": "",
    "samplers": ["penalties","dry","top_k","typ_p","top_p","min_p","xtc","temperature"],
    "speculative.n_max": 16,
    "speculative.n_min": 0,
    "speculative.p_min": 0.75,
    "timings_per_token": false,
    "post_sampling_probs": false,
    "lora": []
  },
  "next_token": {
    "has_next_token": true,
    "has_new_line": false,
    "n_remain": -1,
    "n_decoded": 0
  }
}
```

---

### POST `/slots/{id}?action=save|restore|erase`

Требует `--slot-save-path <dir>`.

**save:**
```json
// запрос
{"filename": "slot_save_file.bin"}
// ответ
{
  "id_slot": 0,
  "filename": "slot_save_file.bin",
  "n_saved": 1745,
  "n_written": 14309796,
  "timings": { "save_ms": 49.865 }
}
```

**restore:**
```json
// запрос
{"filename": "slot_save_file.bin"}
// ответ
{
  "id_slot": 0,
  "filename": "slot_save_file.bin",
  "n_restored": 1745,
  "n_read": 14309796,
  "timings": { "restore_ms": 42.937 }
}
```

**erase:**
```json
// запрос
{}
// ответ
{"id_slot": 0, "n_erased": 1745}
```

---

### GET `/metrics`

Доступен только при `--metrics`. Отдаёт Prometheus text format. Без него —
`501 not_supported_error`.

Экспортируемые метрики:

| Имя                                  | Описание |
|--------------------------------------|----------|
| `llamacpp:prompt_tokens_total`       | Сколько токенов prompt’а обработано. |
| `llamacpp:tokens_predicted_total`    | Сколько токенов сгенерировано. |
| `llamacpp:prompt_tokens_seconds`     | Средняя скорость prompt (tokens/s). |
| `llamacpp:predicted_tokens_seconds`  | Средняя скорость generation (tokens/s). |
| `llamacpp:kv_cache_usage_ratio`      | Использование KV (`1` = 100 %). |
| `llamacpp:kv_cache_tokens`           | Текущий размер KV (tokens). |
| `llamacpp:requests_processing`       | Запросов в работе. |
| `llamacpp:requests_deferred`         | Запросов отложено. |
| `llamacpp:n_tokens_max`              | Максимум контекста, который когда-либо использовали. |

Ответ — обычный text/plain (Prometheus exposition format); клиент должен
обрабатывать его построчно, не как JSON.

---

### POST `/completion`

Родной API llama.cpp. **Не** OAI-совместим — для OAI-клиентов `/v1/completions`.

#### Поле `prompt`

Допустимы формы (см. README строки 395–407):

1. Строка: `"привет"`.
2. Массив токенов: `[12, 34, 56]`.
3. Смешанный массив: `[12, "строка", 56]`.
4. Объект с мультимодалкой:
   ```json
   {"prompt_string": "опиши <__media__>", "multimodal_data": ["<base64>"]}
   ```
5. Массив любой из форм выше — тогда ответ будет массивом.

Количество маркеров `<__media__>` должно равняться длине `multimodal_data`.
Точный маркер сервер отдаёт в `GET /props`.`media_marker`.

#### Дополнительные поля запроса

Помимо всех [sampling-параметров](#sampling-параметры):

| Поле | Тип | Описание |
|------|-----|----------|
| `prompt` | см. выше | См. выше. |
| `stream` | bool | SSE. |
| `response_fields` | string[] | Ограничить поля, опционально «разнести» через `a/b`. |
| `t_max_predict_ms` | int | Таймаут генерации. |
| `cache_prompt` | bool | Переиспользование KV. |
| `id_slot` | int | Конкретный слот. |
| `return_tokens` | bool | Массив id токенов. |

#### Ответ (non-stream)

```json
{
  "content": "<string>",
  "tokens": [123, 456, 789],
  "stop": true,
  "stop_type": "eos",
  "stopping_word": "",
  "generation_settings": { /* см. Приложение Б */ },
  "model": "<alias>",
  "prompt": "<обработанный prompt>",
  "timings": {
    "prompt_n": 1,
    "prompt_ms": 30.958,
    "prompt_per_token_ms": 30.958,
    "prompt_per_second": 32.3,
    "predicted_n": 35,
    "predicted_ms": 661.064,
    "predicted_per_token_ms": 18.89,
    "predicted_per_second": 52.94,
    "cache_n": 236
  },
  "tokens_cached": 236,
  "tokens_evaluated": 1,
  "truncated": false,
  "completion_probabilities": [
    {
      "content": "<token text>",
      "tokens": [123],
      "probs": [
        {
          "id": 123,
          "logprob": -0.123,
          "token": "<string>",
          "bytes": [195, 161],
          "top_logprobs": [
            {"id": 456, "logprob": -0.5, "token": "<t>", "bytes": [...]}
          ]
        }
      ]
    }
  ]
}
```

`stop_type` ∈ `none | eos | limit | word`. Если `post_sampling_probs: true`,
`logprob` заменяется на `prob` (0..1), а `top_logprobs` — на `top_probs`.

#### Ответ (stream)

Между событиями — пустая строка. Каждое событие:

```
data: {"content":"<частичный текст>","tokens":[...],"stop":false}

```

Финальное событие:

```
data: {"content":"","tokens":[],"stop":true,"stop_type":"eos",...}
```

В streaming-режиме в каждом чанке присутствуют `content`, `tokens`
(если `return_tokens`) и `stop`. При `timings_per_token: true` — ещё
`timings`. При `return_progress: true` — объект `prompt_progress` с полями
`total`, `cache`, `processed`, `time_ms`.

`[DONE]` для `/completion` не отправляется.

---

### POST `/v1/completions`

OpenAI-совместимый legacy completions API. Принимает поля OpenAI
(`model`, `prompt`, `max_tokens`, `temperature`, …) + все llama.cpp-специфичные
(`mirostat`, `min_p`, `grammar`, …).

Ответ (non-stream):

```json
{
  "id": "cmpl-xxxx",
  "object": "text_completion",
  "created": 1735142223,
  "model": "<alias>",
  "system_fingerprint": "b5678-abcdef0",
  "choices": [{
    "index": 0,
    "text": "<generated>",
    "finish_reason": "stop",
    "logprobs": null
  }],
  "usage": {"prompt_tokens": 10, "completion_tokens": 35, "total_tokens": 45},
  "timings": { /* см. /completion */ }
}
```

Streaming: события `data: {...}` с `"object": "text_completion"`,
заключительный `data: [DONE]`.

---

### POST `/chat/completions`

OpenAI Chat Completions API. Синонимы: `/v1/chat/completions`.

#### Поля запроса

OpenAI-стандарт:

| Поле | Тип | Описание |
|------|-----|----------|
| `model` | string | Может быть любой строкой — используется alias модели. |
| `messages` | array | `[{role, content, tool_calls?, tool_call_id?, name?}, ...]`. |
| `tools` | array | Определения функций (см. `docs/function-calling.md`). Требует `--jinja`. |
| `tool_choice` | `auto | none | required | {"type":"function","function":{"name":"..."}}` | |
| `response_format` | object | `{"type":"text"}`, `{"type":"json_object","schema":{...}}`, `{"type":"json_schema","schema":{...}}`. |
| `max_tokens` / `max_completion_tokens` | int | Лимит. |
| `temperature`, `top_p`, `frequency_penalty`, `presence_penalty`, `seed`, `stop`, `logit_bias`, `logprobs`, `top_logprobs`, `n` | стандарт OpenAI. |
| `stream` | bool | SSE. |
| `stream_options` | `{"include_usage": bool}` | Вернуть `usage` последним чанком. |

Расширения llama.cpp (все из sampling + следующие):

| Поле | Описание |
|------|----------|
| `mirostat`, `min_p`, `xtc_*`, `dry_*`, `dynatemp_*`, `top_k`, `typical_p`, `samplers` | — |
| `grammar`, `json_schema` | — |
| `cache_prompt`, `id_slot`, `t_max_predict_ms`, `n_probs`, `post_sampling_probs` | — |
| `chat_template_kwargs` | Параметры для jinja-шаблона (напр., `{"enable_thinking": false}`). |
| `reasoning_format` | `none | deepseek | qwen | ...` — парсинг reasoning. |
| `reasoning_in_content` | bool — оставить reasoning в `content`. |
| `generation_prompt` | string — префикс ответа. |
| `parse_tool_calls` | bool — парсить tool-вызовы. |
| `parallel_tool_calls` | bool — разрешить параллельные tool-вызовы. |
| `lora` | `[{id,scale}]` — per-request LoRA. |

Multimodal: в `content` сообщения вкладываются объекты `{"type":"text","text":"..."}`,
`{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,..."}}` —
см. OpenAI spec. `llama-server` также принимает прямой `url`.

#### Ответ (non-stream)

```json
{
  "id": "chatcmpl-xxxx",
  "object": "chat.completion",
  "created": 1735142223,
  "model": "<alias>",
  "system_fingerprint": "b5678-abcdef0",
  "choices": [{
    "index": 0,
    "message": {
      "role": "assistant",
      "content": "<text>",
      "reasoning_content": "<optional>",
      "tool_calls": [{"id":"call_...", "type":"function", "function":{"name":"...","arguments":"..."}}]
    },
    "finish_reason": "stop",
    "logprobs": { "content": [...] }
  }],
  "usage": {"prompt_tokens": 12, "completion_tokens": 35, "total_tokens": 47},
  "timings": { "cache_n": 236, "prompt_n": 1, "prompt_ms": 30.9, "predicted_n": 35, "predicted_ms": 661.06, "..." : "..." }
}
```

`finish_reason` ∈ `stop | length | tool_calls | content_filter`.

#### Ответ (stream)

Каждое событие — `data: { "object":"chat.completion.chunk", ...}`. Дельты:

```json
{
  "id": "chatcmpl-xxxx",
  "object": "chat.completion.chunk",
  "created": 1735142223,
  "model": "<alias>",
  "system_fingerprint": "b5678-abcdef0",
  "choices": [{
    "index": 0,
    "delta": {"role": "assistant", "content": "<part>"},
    "finish_reason": null
  }]
}
```

Финальный чанк: `finish_reason != null`, `delta: {}`. При
`stream_options.include_usage: true` — дополнительно чанк с
`choices: []` и `usage: {...}`. Завершающая строка стрима:
```
data: [DONE]
```

Tool-вызовы в стриминге приходят через `delta.tool_calls[i]` с частичным
`function.arguments`.

---

### POST `/v1/responses`

OpenAI Responses API. Работает как Chat Completions: сервер трансформирует
запрос во внутренний chat-формат. Поля — `instructions`, `input`
(строка или массив блоков), `model`, `stream`, все llama.cpp-расширения.

Пример:

```json
{
  "model": "gpt-4.1",
  "instructions": "You are ChatGPT...",
  "input": "Write a limerick about python exceptions"
}
```

Ответ (non-stream) — объект `"object":"response"` с `output[]`, где
`output[i].type ∈ "message" | "reasoning"`. См. OpenAI spec.

---

### POST `/v1/messages` (Anthropic)

Anthropic-совместимый Messages API. Принимает:

| Поле | Тип | Описание |
|------|-----|----------|
| `model` | string | required |
| `messages` | `[{role, content}]` | required, role ∈ `user|assistant` |
| `max_tokens` | int | default `4096` |
| `system` | string \| `[{type:"text", text:"..."}]` | системный промпт |
| `temperature` | float (0..1) | default `1.0` |
| `top_p`, `top_k` | | |
| `stop_sequences` | string[] | |
| `stream` | bool | SSE |
| `tools` | массив | требует `--jinja` |
| `tool_choice` | `{"type":"auto"}` \| `{"type":"any"}` \| `{"type":"tool","name":"..."}` | |

Заголовок авторизации: `x-api-key: <key>` или `Authorization: Bearer <key>`.

Ответ: формат Anthropic — `{"id":"...","type":"message","role":"assistant","content":[{"type":"text","text":"..."}],"model":"...","stop_reason":"end_turn","usage":{...}}`.

---

### POST `/v1/messages/count_tokens`

Тот же формат запроса, что и `/v1/messages`, но без `max_tokens`. Ответ:

```json
{"input_tokens": 10}
```

---

### POST `/v1/audio/transcriptions`

OpenAI Whisper-совместимый API. Принимает `multipart/form-data` с полем
`file` (аудио), `model`, `prompt`, `language`, `response_format`, `temperature`.
Требует загруженного модельного файла с возможностями transcription. В этом
документе не детализируется — см. OpenAI spec.

---

### POST `/infill`

Code infilling (Fill-In-the-Middle). Принимает все поля `/completion` плюс:

| Поле | Тип | Описание |
|------|-----|----------|
| `input_prefix` | string | Код перед cursor. |
| `input_suffix` | string | Код после cursor. |
| `input_extra` | `[{filename:string, text:string}]` | Репо-контекст, вставляется до FIM-префикса. |
| `prompt` | string | Добавляется после `FIM_MID`. |

Если модель поддерживает `FIM_REP` / `FIM_FILE_SEP`, используется repo-level
шаблон; иначе — обычный FIM. Стриминг — как у `/completion`.

---

### POST `/embedding`

Native (не OAI-совместимый) embedding endpoint.

Запрос:
```json
{"content": "string | [tokens] | [mixed]", "embd_normalize": 2}
```

`embd_normalize`: `-1` — без норм., `0` — max-abs, `1` — L1, `2` — L2 (default),
`>2` — p-norm.

Ответ (per-token, если `--pooling none`):
```json
{"embedding": [[..per-token..], ...], "tokens_evaluated": 42}
```

Иначе (pooled):
```json
{"embedding": [..vector..], "tokens_evaluated": 42}
```

---

### POST `/embeddings`

Расширенный endpoint: принимает массив input’ов, работает в том числе с
`--pooling none`. Поля — как у `/v1/embeddings`. Ответ — массив:

```json
[
  {
    "index": 0,
    "embedding": [[...], [...], ...]
  },
  {"index": 1, "embedding": [...]}
]
```

При `--pooling none` `embedding` — массив токен-векторов, иначе — один вектор.

---

### POST `/v1/embeddings`

OpenAI Embeddings API. Поля:

| Поле | Тип | Описание |
|------|-----|----------|
| `input` | string \| string[] \| int[] \| int[][] | |
| `model` | string | alias |
| `encoding_format` | `float` \| `base64` | default `float` |
| `dimensions` | int | опц. (не всегда поддерживается — зависит от модели) |
| `user` | string | опц. |

Ответ:

```json
{
  "object": "list",
  "data": [
    {"object":"embedding","embedding":[0.1, 0.2, ...],"index":0},
    {"object":"embedding","embedding":[...],"index":1}
  ],
  "model": "<alias>",
  "usage": {"prompt_tokens": 42, "total_tokens": 42}
}
```

Пулинг должен быть `!= none`, иначе вернётся ошибка.

---

### POST `/rerank`

Синонимы: `/reranking`, `/v1/rerank`, `/v1/reranking`. Требует модель-reranker
и запуск с `--embedding --pooling rank`.

Запрос:

```json
{
  "model": "bge-reranker-v2-m3",
  "query": "What is panda?",
  "documents": ["hi", "it is a bear", "giant panda is a bear ..."],
  "top_n": 3
}
```

Ответ:

```json
{
  "model": "bge-reranker-v2-m3",
  "object": "list",
  "usage": {"prompt_tokens": 42, "total_tokens": 42},
  "results": [
    {"index": 2, "relevance_score": 0.98},
    {"index": 1, "relevance_score": 0.42},
    {"index": 0, "relevance_score": 0.01}
  ]
}
```

---

### POST `/tokenize`

Запрос:

```json
{
  "content": "Hello world",
  "add_special": false,
  "parse_special": true,
  "with_pieces": false
}
```

Ответ (`with_pieces: false`):

```json
{"tokens": [15043, 1024]}
```

Ответ (`with_pieces: true`): каждый токен — объект `{id, piece}`. `piece` —
строка (если валидный UTF-8) или массив байт (`number[]`) иначе.

```json
{
  "tokens": [
    {"id": 123, "piece": "Hello"},
    {"id": 456, "piece": " world"},
    {"id": 789, "piece": [195, 161]}
  ]
}
```

---

### POST `/detokenize`

Запрос:

```json
{"tokens": [15043, 1024]}
```

Ответ:

```json
{"content": "Hello world"}
```

---

### POST `/apply-template`

Применить chat template сервера к сообщениям без инференса.

Запрос:

```json
{
  "messages": [
    {"role": "system", "content": "You are helpful."},
    {"role": "user",   "content": "Hi"}
  ]
}
```

Ответ:

```json
{"prompt": "<|system|>You are helpful.<|user|>Hi<|assistant|>"}
```

---

### GET `/lora-adapters`

Возвращает зарегистрированные адаптеры:

```json
[
  {"id": 0, "path": "my_adapter_1.gguf", "scale": 0.0},
  {"id": 1, "path": "my_adapter_2.gguf", "scale": 0.0}
]
```

Адаптеры загружаются при старте сервера через `--lora` / `--lora-scaled`.

---

### POST `/lora-adapters`

Глобальное выставление scale (перезаписывается `lora` в запросе `/completion`).

Запрос:

```json
[
  {"id": 0, "scale": 0.2},
  {"id": 1, "scale": 0.8}
]
```

Ответ: `{"success": true}` (и текущий `GET /lora-adapters`). Чтобы отключить
адаптер — уберите из списка или выставьте `scale: 0`.

---

### POST `/models/load`

Только в router mode.

Запрос:
```json
{"model": "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M"}
```

Ответ:
```json
{"success": true}
```

---

### POST `/models/unload`

Только в router mode.

Запрос:
```json
{"model": "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M"}
```

Ответ:
```json
{"success": true}
```

---

## Приложение А. Матрица совместимости с OpenAI

| OpenAI endpoint               | llama-server | Примечания |
|-------------------------------|:-:|---|
| `POST /v1/chat/completions`   | ✅ | + llama-specific поля. Tool calls — через `--jinja`. |
| `POST /v1/completions`        | ✅ | + llama-specific поля. |
| `POST /v1/embeddings`         | ✅ | Пулинг не `none`. |
| `GET /v1/models`              | ✅ | Один элемент. |
| `POST /v1/responses`          | ✅ | Трансляция в chat completions. |
| `POST /v1/audio/transcriptions` | ⚠️ | Нужна модель с возможностью transcription. |
| Fine-tuning / Assistants / Threads | ❌ | Отсутствует. |
| Function-calling (native)     | ✅ | Через `tools` + `--jinja`. |
| Logprobs                      | ✅ | `logprobs` / `top_logprobs` (иначе используйте `n_probs`). |
| `response_format: json_schema`| ✅ | Через grammar converter. |
| Batch API                     | ❌ | — |

---

## Приложение Б. Поля `generation_settings`

Эти поля приходят в `GET /props` (`default_generation_settings`), `GET /slots`
(`params`) и в ответах `/completion` (`generation_settings`):

```
n_predict, seed, temperature, dynatemp_range, dynatemp_exponent,
top_k, top_p, min_p, top_n_sigma, xtc_probability, xtc_threshold,
typical_p, repeat_last_n, repeat_penalty, presence_penalty, frequency_penalty,
dry_multiplier, dry_base, dry_allowed_length, dry_penalty_last_n,
dry_sequence_breakers, mirostat, mirostat_tau, mirostat_eta,
stop, max_tokens, n_keep, n_discard, ignore_eos, stream,
n_probs, min_keep, grammar, samplers,
chat_format, reasoning_format, reasoning_in_content, generation_prompt,
speculative.n_max, speculative.n_min, speculative.p_min,
timings_per_token, post_sampling_probs, lora
```

Поля `speculative.*` присутствуют только при включённом speculative decoding.
Поле `chat_format` и `reasoning_*` появляются только в ответах chat.

---

## Примечания

- Все цифровые поля — десятичные (JSON number).
- Для избежания неожиданного преобразования float→double лучше слать
  sampling-параметры строго как указано в таблице (int vs float).
- Клиент **должен** не выставлять поля, значение которых совпадает с
  серверным default — сервер уже пишет их в `params` слота.
- При stream’е клиент обязан поддерживать инкрементальное чтение байт
  (частичные SSE-события) и кодировку UTF-8 (поскольку токены могут резать
  символ пополам).
- Endpoint `/tools` (Web UI internal) в этом клиенте не реализуется.
