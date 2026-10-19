//! Корневые страницы приложения (top-level маршруты).

pub mod code_editor;
pub mod huggingface;
pub mod node_editor;
/// Режим «Заметки» — библиотека заметок synshell (тот же код, что в syn-notes).
pub use syn_notes::notes;
pub mod settings;
pub mod syn_chat;
pub mod syn_explorer;
