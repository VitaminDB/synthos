//! Сколько контекста осталось под результат инструмента — общий модуль
//! библиотеки заметок (`syn_notes::tool::budget`): инструмент `notes` режет
//! чтение пачкой по этому же бюджету, так что слот один на программу.
//! Здесь — только счётчик токенов по загруженной модели.

use std::sync::Arc;

pub use syn_notes::tool::budget::*;

use crate::syn_chat::model_registry::LoadedSynModel;

/// Счётчик по загруженной модели. Ошибка кодирования не должна ронять ход:
/// где токенизатор не смог, сойдёт оценка по символам.
///
/// Держит только токенизатор, не модель: счётчик живёт в бюджете хода и в
/// `MediaCaps` весь agent-loop, и клон `Arc<LoadedSynModel>` не давал
/// `pipelines run` с `free_vram` освободить веса — ACE-Step и LTX грузились
/// рядом с чат-LLM и падали в OOM.
pub fn model_counter(model: &Arc<LoadedSynModel>) -> Counter {
    let tokenizer = Arc::clone(&model.tokenizer);
    Arc::new(move |s: &str| tokenizer.encode(s).map(|ids| ids.len()).unwrap_or_else(|_| estimate(s)))
}
