//! Управление процессом `llama-server` + реактивное состояние логов и статуса.
//!
//! Один экземпляр [`LlamaProcess`] живёт в [`crate::context::AppCtx`]; правый
//! сайдбар чата (таб «Ламма контроль») им управляет, страница «Поддержка»
//! читает его логи.

pub mod api;
pub mod process;

pub use api::{
    ApiError, ApiErrorPayload, ChatMessage, ChatRequest, ChatRole, ChatStreamChunk,
    CompletionPrompt, CompletionRequest, EmbeddingRequest, HealthStatus, InfillRequest,
    LlamaClient, LlamaError, LoraAdapter, LoraUpdate, MultimodalPrompt, NativeEmbeddingRequest,
    RerankRequest, SamplingParams, ServerProps, SlotInfo, SseEvent, SseStream,
    TokenizeRequest,
};
pub use process::{LlamaProcess, ProcessStatus};
