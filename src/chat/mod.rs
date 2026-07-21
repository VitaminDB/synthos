//! Логика мультиконтекстного чата с `llama-server`.
//!
//! Модуль целиком инкапсулирует «что такое чат», «что такое сообщение»,
//! «как их хранить и отправлять». Виджеты (`components::message_area`,
//! `components::message_bubble`, `components::input_panel`,
//! `components::chats_column`) работают только с типами отсюда и с
//! публичными функциями [`registry`] / [`session`].
//!
//! Организация:
//! - [`state`]    — типы (`ChatCtx`, `ChatMsg`, `ChatMsgRole`, `ChatMeta`);
//! - [`storage`]  — хранение чатов на диске (`~/.config/synthos/chats/`);
//! - [`registry`] — CRUD над списком чатов (create/select/delete), снимок
//!                  для автосейва;
//! - [`session`]  — отправка сообщения + async-драйвер SSE-стрима;
//! - [`think_parser`] — стейт-машина выделения `<think>...</think>` блоков
//!                  из стрим-content reasoning-моделей;
//! - [`time`]     — минимальные хелперы времени без `chrono`;
//! - [`tools`]    — агентские инструменты (bash, web_read) + executor.

pub mod attach;
pub mod audio;
pub mod blobs;
pub mod streaming_asr;
pub mod compact;
pub mod registry;
pub mod session;
pub mod state;
pub mod storage;
pub mod think_parser;
pub mod time;
pub mod tools;
pub mod voice_refine;

pub use state::{ChatCtx, ChatMeta, ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment};
