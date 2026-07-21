//! Параллельный чат-режим с native in-process Qwen3.6 inference
//! (через крейт `llm-qwen36`), без HTTP к llama-server.
//!
//! Существующий [`crate::chat`] на `llama-server` остаётся как есть —
//! это второй, независимый чат с собственным storage'ом
//! (`~/.config/synthos/syn_chats/`).

pub mod model_registry;
pub mod params;
pub mod registry;
pub mod session;
pub mod state;
pub mod storage;
pub mod tool_parser;

pub use model_registry::{LoadedSynModel, SynModelRegistry};
pub use params::SamplingParams;
pub use state::SynChatCtx;
