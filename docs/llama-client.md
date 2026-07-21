# `synthos::llama::api` — гид по клиенту llama-server

Модуль `llama::api` — типизированный async HTTP-клиент `llama-server`.
Спецификация протокола — [`llama-server-api.md`](llama-server-api.md).

## Где лежит

```
app/synthos/src/llama/
├── mod.rs              re-export LlamaClient / DTO
├── process.rs          обёртка над дочерним процессом llama-server
└── api/
    ├── mod.rs          LlamaClient, LlamaClientBuilder, приватные хелперы
    ├── error.rs        LlamaError, ApiError, ApiErrorPayload
    ├── sse.rs          SseStream + парсер SSE
    ├── sampling.rs     SamplingParams, LogitBias, SamplerOrder, LoraPatch
    ├── common.rs       ChatMessage, Usage, Timings, TokenProb, …
    ├── health.rs       /health
    ├── props.rs        /props
    ├── slots.rs        /slots, /slots/{id}?action=…
    ├── metrics.rs      /metrics (Prometheus text)
    ├── models.rs       /models, /v1/models, /models/load|unload
    ├── tokenize.rs     /tokenize, /detokenize, /apply-template
    ├── embedding.rs    /embedding, /embeddings, /v1/embeddings
    ├── rerank.rs       /rerank
    ├── infill.rs       /infill
    ├── completion.rs   /completion, /v1/completions
    ├── chat.rs         /v1/chat/completions
    └── lora.rs         /lora-adapters
```

## Конструкторы

```rust
use synthos::llama::api::LlamaClient;

// Самый короткий путь.
let client = LlamaClient::new("127.0.0.1", 8080);

// Или через builder.
use std::time::Duration;
let client = LlamaClient::builder()
    .host_port("127.0.0.1", 8080)
    .api_key("sk-optional")
    .connect_timeout(Duration::from_secs(3))
    .timeout(Duration::from_secs(600)) // стримы чата могут длиться долго
    .pool_max_idle_per_host(4)
    .user_agent("synthos/1.0")
    .build();
```

`LlamaClient` держит `Arc<reqwest::Client>` — можно клонировать и расшаривать.
Если в приложении нужен кастомный `reqwest::Client` (например, с общим
прокси-раутингом), передайте его в `.http_client(c)`.

### Интеграция с конфигом synthos

`GeneralConfig` в `app/synthos/src/config.rs` уже содержит `server_host: String`
и `server_port: u16`. Типовой способ:

```rust
use synthos::llama::api::LlamaClient;

fn make_client_from_ctx(ctx: &crate::context::AppCtx) -> LlamaClient {
    let general = ctx.general.get_untracked();
    LlamaClient::new(&general.server_host, general.server_port)
}
```

## Диагностика

```rust
let status = client.health().await?;
if status.ok { /* готов */ }

let props = client.props().await?;
println!("chat template: {:?}", props.chat_template);
println!("total_slots: {:?}", props.total_slots);

let slots = client.slots(/*fail_on_no_slot*/ false).await?;
let metrics_prom_text = client.metrics().await?;
let models = client.v1_models().await?;
```

## Токенизация

```rust
use synthos::llama::api::{ApplyTemplateRequest, ChatMessage, DetokenizeRequest, TokenizeRequest};

// Быстрые shortcut’ы:
let ids = client.tokenize_text("Hello, world!").await?;
let text = client.detokenize_tokens(&ids).await?;

// Полный контроль:
let resp = client.tokenize(
    &TokenizeRequest::new("Hello").with_add_special(true).with_pieces(true),
).await?;

// Применить шаблон чата, получить готовый prompt.
let t = client.apply_template(&ApplyTemplateRequest::new([
    ChatMessage::system("Будь краток."),
    ChatMessage::user("Привет."),
])).await?;
println!("rendered prompt: {}", t.prompt);
```

## Chat (OpenAI-совместимый) — non-stream и stream

```rust
use synthos::llama::api::{ChatMessage, ChatRequest, SamplingParams};
use futures_util::StreamExt;

let req = ChatRequest::new([
    ChatMessage::system("Ты помощник."),
    ChatMessage::user("Что такое llama.cpp?"),
])
.with_sampling(
    SamplingParams::new()
        .with_temperature(0.6)
        .with_top_p(0.92)
        .with_max_tokens(512),
);

// Non-stream:
let resp = client.chat_completions(&req).await?;
println!("{}", resp.first_text().unwrap_or_default());

// Stream:
let mut s = client.chat_completions_stream(&req).await?;
while let Some(chunk) = s.next().await {
    let chunk = chunk?;
    if let Some(delta) = chunk.first_delta_content() {
        print!("{}", delta);
    }
    if chunk.first_finish_reason().is_some() {
        break;
    }
}
```

### Tool calling

```rust
use synthos::llama::api::{ChatTool, ChatToolChoice, ToolFunctionSchema};
use serde_json::json;

let req = ChatRequest::new([ChatMessage::user("какая погода в Париже?")])
    .with_tools([ChatTool {
        kind: "function".into(),
        function: ToolFunctionSchema {
            name: "get_weather".into(),
            description: Some("Fetch current weather".into()),
            parameters: Some(json!({
                "type": "object",
                "properties": { "city": {"type": "string"} },
                "required": ["city"]
            })),
            strict: None,
        },
    }])
    .with_tool_choice(ChatToolChoice::Simple("auto".into()));
```

После генерации ассистент положит tool-вызовы в
`response.choices[0].message.tool_calls`; ответ инструмента отправляется
обратно как `ChatMessage::tool(text, tool_call_id)`.

### JSON-schema / response_format

```rust
use synthos::llama::api::ResponseFormat;
use serde_json::json;

let req = ChatRequest::new([ChatMessage::user("Верни JSON {name:string}.")])
    .with_response_format(ResponseFormat::JsonSchema {
        schema: Some(json!({
            "type":"object",
            "properties":{"name":{"type":"string"}},
            "required":["name"]
        })),
    });
```

### Multimodal

```rust
use synthos::llama::api::{ChatContentPart, ChatImageUrl, ChatRole, ChatMessage};

let msg = ChatMessage::multipart(ChatRole::User, vec![
    ChatContentPart::Text { text: "опиши картинку".into() },
    ChatContentPart::ImageUrl {
        image_url: ChatImageUrl {
            url: "data:image/png;base64,iVBORw0KGgo...".into(),
            detail: None,
        }
    },
]);
```

## Completion (legacy) и streaming

```rust
use synthos::llama::api::{CompletionPrompt, CompletionRequest, MultimodalPrompt, SamplingParams};
use futures_util::StreamExt;

// Обычный текст.
let req = CompletionRequest::new("The capital of France is")
    .with_sampling(SamplingParams::new().with_n_predict(64));

let final_ = client.completion(&req).await?;
println!("{}", final_.content);

// Массив токенов.
let req = CompletionRequest::new(CompletionPrompt::Tokens(vec![1, 100, 200]));

// Multimodal.
let req = CompletionRequest::new(CompletionPrompt::Multimodal(
    MultimodalPrompt::new("опиши <__media__>", vec!["AAAA".into()]),
));

// Stream.
let mut s = client.completion_stream(&req).await?;
while let Some(chunk) = s.next().await {
    let c = chunk?;
    print!("{}", c.content);
    if c.stop {
        break;
    }
}
```

## Embeddings

```rust
use synthos::llama::api::{EmbeddingRequest, NativeEmbeddingRequest};

// OpenAI-совместимо.
let r = client.embeddings(
    &EmbeddingRequest::from_texts(["hello", "world"]).with_model("bge"),
).await?;
for item in r.data {
    // item.embedding — EmbeddingValue::Floats(Vec<f32>)
}

// Native (/embedding) — в том числе per-token при --pooling none.
let r = client.embedding_native(
    &NativeEmbeddingRequest::from_text("hello").with_normalize(2),
).await?;
```

## Rerank

```rust
use synthos::llama::api::RerankRequest;

let r = client.rerank(
    &RerankRequest::new("panda", vec![
        "hi".into(),
        "it is a bear".into(),
        "giant panda is a bear species".into(),
    ]).with_top_n(3),
).await?;
for res in r.results { println!("{}: {}", res.index, res.relevance_score); }
```

## Infill

```rust
use synthos::llama::api::{InfillExtra, InfillRequest, SamplingParams};
use futures_util::StreamExt;

let req = InfillRequest::new()
    .with_prefix("fn add(a: i32, b: i32) -> i32 {\n    ")
    .with_suffix("\n}\n")
    .with_extra([InfillExtra {
        filename: "lib.rs".into(),
        text: "pub fn main() {}".into(),
    }])
    .with_sampling(SamplingParams::new().with_n_predict(32));

let mut s = client.infill_stream(&req).await?;
while let Some(chunk) = s.next().await {
    print!("{}", chunk?.content);
}
```

## LoRA

```rust
use synthos::llama::api::LoraUpdate;

let adapters = client.lora_adapters().await?;
client.set_lora_adapters([LoraUpdate::new(0, 0.25), LoraUpdate::new(1, 0.0)]).await?;
```

## Slots: save/restore/erase

```rust
client.slot_save(0, "my-prompt.bin").await?;
client.slot_restore(0, "my-prompt.bin").await?;
client.slot_erase(0).await?;
```

## Ошибки

```rust
use synthos::llama::api::LlamaError;

match client.health().await {
    Ok(h) if h.ok => { /* готов */ }
    Ok(_) => { /* модель грузится — HTTP 503 */ }
    Err(LlamaError::Transport(e)) if e.is_timeout() => { /* таймаут */ }
    Err(LlamaError::Http { status, payload }) => {
        eprintln!("HTTP {status}: {:?}", payload);
    }
    Err(e) => eprintln!("{e}"),
}
```

`LlamaError` — `thiserror`-enum. Полезные предикаты:
- `LlamaError::status() -> Option<StatusCode>`
- `LlamaError::api_error() -> Option<&ApiError>` (структурированная ошибка
  OpenAI-формата, если сервер её вернул)
- `LlamaError::is_timeout()`, `is_connect()`

## Интеграция с UI (future work)

`syngui` включает feature `tokio`, поэтому можно `spawn` задачу прямо из
UI-потока:

```rust
use syngui::async_runtime::{run_on_main_thread, spawn};
use futures_util::StreamExt;

fn submit_chat(ctx: &crate::context::AppCtx, user_text: String) {
    let client = LlamaClient::new(
        &ctx.general.get_untracked().server_host,
        ctx.general.get_untracked().server_port,
    );
    let bubbles = ctx.messages; // RwSignal<Vec<...>>

    spawn(async move {
        let req = ChatRequest::new([ChatMessage::user(user_text)]);
        let mut stream = match client.chat_completions_stream(&req).await {
            Ok(s) => s,
            Err(e) => {
                run_on_main_thread(move || tracing::error!(?e, "chat error"));
                return;
            }
        };
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else { continue };
            if let Some(delta) = chunk.first_delta_content() {
                let d = delta.to_string();
                run_on_main_thread(move || {
                    bubbles.update(|v| {
                        if let Some(last) = v.last_mut() { last.push_str(&d); }
                    });
                });
            }
        }
    });
}
```

Это паттерн — реальная интеграция с `input_panel` / `message_area` — это
отдельная задача.

## Тестирование

- `cargo test -p synthos llama::api` — все unit-тесты.
- SSE-парсер проверяется сборкой событий из произвольно нарезанных
  байтов, включая разбивку по 1 байту.
- Serde-раунтрипы покрыты для `SamplingParams`, `ChatRequest`,
  `ChatCompletionResponse`, `ChatStreamChunk`, `CompletionFinal`,
  `CompletionStreamChunk`, `TokenizeResponse` (id-only и with-pieces),
  `SlotInfo`, `ModelList`, `EmbeddingResponse`, `RerankResponse`,
  `LogitBias`, `LoraAdapter`.

Для ручного smoke-теста на живом сервере:
1. Запустите `llama-server --model <path> --port 8080`.
2. В отдельном тесте (помеченном `#[ignore]`) используйте `LlamaClient`.
3. Запускайте как `cargo test -p synthos -- --ignored llama_smoke`.
