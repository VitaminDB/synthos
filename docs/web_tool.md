# Tool `web` — единый интерфейс к интернету (search + read)

## Назначение

Единственный tool synthos для работы с интернетом. Заменил предыдущие
два инструмента (`web_search` + `web_read`) и тяжёлый Servo-движок.

## Pipeline

```
LLM tool-call:
  { "action": "search", "query": "..." }
  { "action": "read",   "url":   "..." }
        │
        ▼
  chat::tools::web::run(args_json)         ← src/chat/tools/web/mod.rs
        │
        ▼
  ┌─────────────┐         ┌────────────────────┐
  │ action=     │         │ action=            │
  │   search    │         │   read             │
  └─────┬───────┘         └─────────┬──────────┘
        │                           │
   reqwest GET                 reqwest GET
   html.duckduckgo.com/html/   <user URL>
   ?q=...&kl=...&ia=web        +ничего особенного
   + Referer:                  + лимит 1 MB
   https://duckduckgo.com/                │
        │                                 ▼
        ▼                       classify Content-Type:
   scraper-парсинг                ├ text/html → Readability
   div.result h2 a.result__a      │   (dom_smoothie::parse)
   + a.result__snippet            │   → article.content (HTML)
   + unwrap /l/?uddg=             │   → htmd::convert → markdown
   + dedup по host+path           ├ text/plain | text/markdown
   + clip до max_results          │   → как есть
        │                         └ прочее → envelope error
        ▼                                 │
   format_search_envelope                 ▼
        │                       format_read_envelope
        │                                 │
        └────────────┬────────────────────┘
                     ▼
            envelope plain-text
            ушёл в LLM как `role=tool`
```

## Файлы

| Путь                                      | Назначение                                      |
|-------------------------------------------|-------------------------------------------------|
| `src/chat/tools/web/mod.rs`               | публичный `run(args_json)`, диспатч action      |
| `src/chat/tools/web/action.rs`            | `enum WebAction`, `parse_args` с валидацией     |
| `src/chat/tools/web/http.rs`              | reqwest fetch (UA, лимит 1 MB, optional Referer) |
| `src/chat/tools/web/search.rs`            | DDG endpoint, scraper SERP, unwrap `/l/?uddg=`  |
| `src/chat/tools/web/read.rs`              | Readability + htmd, content-type classify       |
| `src/chat/tools/web/envelope.rs`          | форматирование `WEB_SEARCH …` / `GET …` секций  |

## JSON-schema

```json
{
  "type": "object",
  "properties": {
    "action":      { "type": "string", "enum": ["search", "read"] },
    "query":       { "type": "string", "description": "для action=search" },
    "url":         { "type": "string", "description": "для action=read" },
    "max_results": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 },
    "lang":        { "type": "string", "default": "ru" }
  },
  "required": ["action"],
  "additionalProperties": false
}
```

## Формат envelope

### action=search (успех)

```
WEB_SEARCH query="rust async" engine=duckduckgo lang=ru
results-count: 10
--- results ---
1. Fundamentals of Asynchronous Programming
   https://doc.rust-lang.org/book/ch17-00-async-await.html
   Async, Await, Futures, and Streams.

2. Introduction - Asynchronous Programming in Rust
   https://rust-lang.github.io/async-book/
   ...
```

### action=read (успех)

```
GET https://example.com/
status: 200
content-type: text/html
--- markdown ---
# Example Domain

This domain is for use in documentation examples ...
```

С redirect / fallback:

```
GET https://example.com/old
final-url: https://example.com/new
status: 200
content-type: text/html
readability: fallback (raw body)
--- markdown ---
...
```

### Ошибки (любой action)

```
GET https://invalid.invalid/
--- error ---
сетевая ошибка: dns error: failed to lookup address information
```

## Зависимости

| Crate          | Где используется                              |
|----------------|------------------------------------------------|
| `reqwest`      | HTTP-fetch (search и read)                    |
| `scraper`      | парсинг DDG SERP (CSS-селекторы)              |
| `dom_smoothie` | Mozilla Readability port (action=read)        |
| `htmd`         | HTML → Markdown                               |
| `urlencoding`  | encode query, unwrap /l/?uddg=                |
| `url`          | парсинг redirect-обёрток DDG                  |
| `serde_json`   | парсинг tool-аргументов                       |

## DDG selectors

Стабильны с 2010-х:
- `div.result, div.web-result` — карточка
- `h2.result__title a.result__a` — заголовок-ссылка (href всегда uddg-обёрнут)
- `a.result__snippet, div.result__snippet` — превью

Запасные в `parse_ddg`: `h2 a` для title, `[data-result="snippet"]` для snippet.

## Почему Servo удалён

В прошлой архитектуре (`web_render/` + `chromiumoxide`/Servo) фактически:
- Google всегда давал капчу → автоматический fallback на DDG;
- DDG отлично работает через простой `reqwest` с честным desktop UA + `Referer: https://duckduckgo.com/`;
- большинство article-сайтов отдают SSR-HTML или JSON-LD/og:meta — JS-рендер не нужен;
- Servo тащил **~800 транзитивных пакетов** и сотни МБ release-бинарника.

JS-рендер для SPA-сайтов (X/Twitter, новый Reddit, JIRA/Notion) пока
backlog. Когда понадобится — реализуем через `chromiumoxide` (CDP к
локальному Chrome) за отдельной фичей `web-cdp`. ~10 транзитивных
пакетов вместо 800.

## Тесты

```bash
# Unit-тесты (без сети):
cargo test -p synthos --lib chat::tools::web

# Smoke-тесты (требуют сети):
cargo test -p synthos --lib chat::tools::web::search::tests::smoke_search_real_ddg \
    -- --ignored --nocapture
cargo test -p synthos --lib chat::tools::web::read::tests::smoke_read_real_example_com \
    -- --ignored --nocapture
```

## Лимиты

- `MAX_FETCH_BYTES = 1 MB` (`web/http.rs`) — сырой body одного fetch'а.
- `MAX_OUTPUT_BYTES = 64 KB` (`tools/executor.rs`) — итоговый envelope в LLM.
- `TIMEOUT = 15s` (`web/http.rs`) — connect + чтение тела.
- `max_results` в action=search: 1..=20, default 10.

## Связь с другими инструментами

- `web_fetch` (`src/chat/tools/web_fetch.rs`) — отдельный low-level helper
  для kb-ingest pipeline'а. Не использует Readability (kb-ingest сам
  делает chunking над raw HTML→markdown). НЕ переиспользуется в `web` —
  у `web::http` свой собственный pipeline с raw-bytes для Readability и
  опциональным Referer.

## История

- 2026-04-30: первая редакция, Servo-based двойной tool web_search + web_read.
- 2026-05-01: переписано на единый tool `web` с DDG html-endpoint'ом
  (Servo удалён, размер бинарника −сотни МБ, ~800 транзитивных пакетов −).
