//! Сколько контекста осталось под результат инструмента — общий модуль
//! библиотеки заметок (`syn_notes::tool::budget`): инструмент `notes` режет
//! чтение пачкой по этому же бюджету, так что слот один на программу.
//! Здесь — только счётчик токенов по загруженной модели.

use std::sync::Arc;

pub use syn_notes::tool::budget::*;

use crate::syn_chat::model_registry::LoadedSynModel;

/// Счётчик по загруженной модели: токенизатор её в службе. Ошибка не должна
/// ронять ход: где служба не ответила, сойдёт оценка по символам.
///
/// Держит только своё соединение со службой, не модель: счётчик живёт в
/// бюджете хода и в `MediaCaps` весь agent-loop.
pub fn model_counter(model: &Arc<LoadedSynModel>) -> Counter {
    let counter = model.token_counter();
    Arc::new(move |s: &str| counter.count(s).unwrap_or_else(|| estimate(s)))
}
