//! CRUD-операции над списком чатов — точка входа для UI.
//!
//! Все публичные функции ходят в [`super::storage`] за файлами и правят
//! сигналы внутри [`crate::context::AppCtx`]. Вызывать их можно только с
//! главного потока (в обработчиках кликов, эффектах, билдерах) — они
//! используют `use_context::<AppCtx>()`, который thread-local.
//!
//! Контракт на согласованность:
//! - Список `chats` отсортирован по `updated_at` убывающе (свежие сверху).
//! - При `select` выставляется `loading=true` до конца массовых
//!   `signal.set` вызовов, чтобы автосейв не перезаписал только что
//!   прочитанный файл.
//! - `snapshot_current` собирает полный `StoredChat` из сигналов — именно
//!   его пишет автосейв в `lib.rs::install_chat_autosave`.

use std::hash::{Hash, Hasher};

use syngui::prelude::*;

use crate::context::AppCtx;

use super::state::{ChatMeta, ChatMsg};
use super::storage::{self, StoredChat};
use super::time::{unix_nanos, unix_secs};
use super::tools::ToolDecision;

/// Считает контент-зависимый отпечаток чата. Используется автосейвом,
/// чтобы пропускать запись при загрузке или rebuild без реальных правок.
pub fn fingerprint(title: &str, model_name: &Option<String>, messages: &[ChatMsg]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut h);
    match model_name {
        Some(m) => {
            1u8.hash(&mut h);
            m.hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    messages.hash(&mut h);
    h.finish()
}

// ─────────────────────────────────────────────────────────────────────────────
// Инициализация
// ─────────────────────────────────────────────────────────────────────────────

/// Перезаливает `chats` из диска и выбирает самый свежий как активный.
/// Если на диске ничего нет — оставляет всё в исходном (пустом) состоянии.
/// Автосейв в это время подавляется через `loading=true`.
pub fn load_all() {
    let ctx = use_context::<AppCtx>();
    let chat = ctx.chat.clone();

    chat.loading.set(true);
    let metas = storage::list_meta();

    if let Some(top) = metas.first().cloned() {
        chat.chats.set(metas);
        // Активируем самый свежий — внутри select мы ещё раз выставим
        // loading; это нормально.
        select_internal(&top.id, &ctx);
    } else {
        chat.chats.set(Vec::new());
        chat.active_chat_id.set(None);
        chat.active_model.set(None);
        chat.messages.set(Vec::new());
    }

    chat.loading.set(false);
}

// ─────────────────────────────────────────────────────────────────────────────
// Создание
// ─────────────────────────────────────────────────────────────────────────────

/// Создаёт новый пустой чат, сохраняет на диск, делает активным.
/// Возвращает id — удобно тесту, и вызывающая сторона может сразу
/// подкинуть сообщение (см. `input_panel` при автосоздании).
pub fn create_new() -> String {
    let ctx = use_context::<AppCtx>();
    let chat = ctx.chat.clone();

    reset_tools_session(&ctx);

    let now = unix_secs();
    let id = format!("{:016x}", unix_nanos());
    // Берём модель из общего `selected_model` — это «последняя выбранная»
    // глобально; новому чату естественно унаследовать её.
    let model_name = ctx.selected_model.get_untracked();

    let stored = StoredChat {
        id: id.clone(),
        title: "Новый чат".to_string(),
        created_at: now,
        updated_at: now,
        model_name: model_name.clone(),
        messages: Vec::new(),
        syn_params: None,
    };
    storage::save(&stored);

    // Вставляем в список (сверху — по соглашению «свежие сверху»).
    chat.chats.update(|list| {
        list.insert(0, stored.to_meta());
    });

    // Переключаемся на новый чат — поскольку сообщений нет, messages=[].
    chat.loading.set(true);
    chat.active_chat_id.set(Some(id.clone()));
    chat.active_model.set(model_name.clone());
    chat.last_saved_fp
        .set(fingerprint(&stored.title, &model_name, &[]));
    chat.messages.set(Vec::new());
    chat.input.set(String::new());
    chat.error.set(None);
    chat.loading.set(false);

    id
}

// ─────────────────────────────────────────────────────────────────────────────
// Выбор активного
// ─────────────────────────────────────────────────────────────────────────────

/// Активирует чат по id — грузит с диска, раскладывает по сигналам.
/// Если чат не найден на диске — убирает его из списка (данные
/// рассинхронизировались, восстановить нечего).
pub fn select(id: &str) {
    let ctx = use_context::<AppCtx>();
    select_internal(id, &ctx);
}

fn select_internal(id: &str, ctx: &AppCtx) {
    let chat = ctx.chat.clone();

    // Сбрасываем per-chat состояние инструментов: пропадает «Разрешить все»,
    // снимается висящий диалог подтверждения (если он был — активный
    // агент-цикл прерывается `abort`-инкрементом ниже).
    reset_tools_session(ctx);

    let Some(stored) = storage::load(id) else {
        eprintln!("[synthos] Чат {id} не найден на диске, убираю из списка");
        chat.chats.update(|list| list.retain(|m| m.id != id));
        if chat.active_chat_id.get_untracked().as_deref() == Some(id) {
            chat.active_chat_id.set(None);
            chat.messages.set(Vec::new());
            chat.active_model.set(None);
            chat.draft_attachments.set(Vec::new());
        }
        return;
    };

    chat.loading.set(true);
    chat.active_chat_id.set(Some(stored.id.clone()));
    chat.active_model.set(stored.model_name.clone());
    // Синхронизируем общий `selected_model` — чтобы Dropdown справа сразу
    // отразил модель активного чата. Эффект в lib.rs (selected_model →
    // active_model) во время loading=true не перезапишет active_model
    // обратно.
    ctx.selected_model.set(stored.model_name.clone());
    // Фиксируем fingerprint ДО set(messages): автосейв, сработав из-за
    // подписки на messages, сравнит свежий снимок с этим значением и
    // пропустит запись — загрузка чата не должна ломать `updated_at`
    // и переупорядочивать список.
    let fp = fingerprint(&stored.title, &stored.model_name, &stored.messages);
    chat.last_saved_fp.set(fp);
    chat.messages.set(stored.messages);
    chat.input.set(String::new());
    // Черновик прикреплённых файлов — per-chat-эфемерное состояние,
    // не persist'ится. При переключении чата сбрасываем, иначе
    // картинки от чата A «утекут» в чат B.
    chat.draft_attachments.set(Vec::new());
    chat.error.set(None);
    // Streaming-хвост — тоже per-chat-эфемерное состояние; иначе
    // partial body чата A «приклеется» к последнему bubble чата B.
    chat.streaming_body.set(String::new());
    chat.streaming_thinking.set(String::new());
    // RAG-блок прошлого turn'а тоже эфемерный per-chat — иначе augment
    // чата A попадёт в system prompt первого ответа чата B.
    chat.augment_text.set(String::new());
    // Сбрасываем любую текущую генерацию — она принадлежала прошлому чату.
    chat.abort
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    chat.pending.set(false);
    chat.loading.set(false);
}

// ─────────────────────────────────────────────────────────────────────────────
// Удаление
// ─────────────────────────────────────────────────────────────────────────────

/// Удаляет чат с диска и из списка. Если это был активный — переключается
/// на следующий в списке (свежайший) или в пустое состояние, если список
/// опустел.
pub fn delete(id: &str) {
    let ctx = use_context::<AppCtx>();
    let chat = ctx.chat.clone();

    storage::delete(id);

    let was_active = chat.active_chat_id.get_untracked().as_deref() == Some(id);
    chat.chats.update(|list| list.retain(|m| m.id != id));

    if was_active {
        let next_id = chat.chats.get_untracked().first().map(|m| m.id.clone());
        match next_id {
            Some(id) => select_internal(&id, &ctx),
            None => {
                chat.loading.set(true);
                chat.active_chat_id.set(None);
                chat.active_model.set(None);
                ctx.selected_model.set(None);
                chat.messages.set(Vec::new());
                chat.input.set(String::new());
                chat.draft_attachments.set(Vec::new());
                chat.loading.set(false);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Переименование (авто)
// ─────────────────────────────────────────────────────────────────────────────

/// Обновляет title активного чата и мета-запись в `chats`. Вызывается
/// автоматически из `session::send_message` после первого user-сообщения.
pub fn rename_active(title: String) {
    let ctx = use_context::<AppCtx>();
    let chat = ctx.chat.clone();

    let Some(id) = chat.active_chat_id.get_untracked() else {
        return;
    };
    chat.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.title = title;
        }
    });
    // Полный StoredChat запишется автосейвом в lib.rs — он увидит изменение
    // chats (через get) и соберёт snapshot_current, где title берётся из
    // ChatMeta. Явная запись не нужна.
}

// ─────────────────────────────────────────────────────────────────────────────
// Снимок для автосейва
// ─────────────────────────────────────────────────────────────────────────────

/// Собирает полный [`StoredChat`] из текущих сигналов для записи на диск.
/// Возвращает `None`, если активного чата нет (писать нечего) или если
/// активный id не присутствует в `chats` (рассинхрон).
pub fn snapshot_current() -> Option<StoredChat> {
    let ctx = use_context::<AppCtx>();
    let chat = ctx.chat.clone();

    let id = chat.active_chat_id.get_untracked()?;
    let meta = chat
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)?;
    let model_name = chat.active_model.get_untracked();
    let messages: Vec<ChatMsg> = chat.messages.get_untracked();
    let created_at = storage::load(&id).map(|c| c.created_at).unwrap_or_else(unix_secs);

    Some(StoredChat {
        id: meta.id.clone(),
        title: meta.title.clone(),
        created_at,
        updated_at: unix_secs(),
        model_name,
        messages,
        // llama-chat не использует поле; сохраняем существующее значение с диска.
        syn_params: storage::load(&id).and_then(|c| c.syn_params),
    })
}

/// Обновляет preview у активной ChatMeta в списке — вызывается из автосейва
/// после смены `messages`, чтобы карточка слева мгновенно показала
/// последнее сообщение, не дожидаясь `load_all`.
pub fn refresh_active_preview(messages: &[ChatMsg]) {
    let ctx = use_context::<AppCtx>();
    let Some(id) = ctx.chat.active_chat_id.get_untracked() else {
        return;
    };
    let preview = storage::preview_from_messages(messages);
    let updated_at = unix_secs();
    ctx.chat.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.preview = preview;
            m.updated_at = updated_at;
        }
        // Пересортируем, чтобы активный поднялся наверх.
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    });
}

/// Обновляет model_name у активной ChatMeta — чтобы subtitle в заголовке
/// сразу отразил переключение модели.
pub fn refresh_active_model(model_name: Option<String>) {
    let ctx = use_context::<AppCtx>();
    let Some(id) = ctx.chat.active_chat_id.get_untracked() else {
        return;
    };
    ctx.chat.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.model_name = model_name.clone();
        }
    });
}

/// Сбрасывает per-chat состояние инструментов (флаг «разрешить все» и
/// висящий диалог подтверждения). Вызывается при создании/выборе чата,
/// чтобы переключение между чатами не уносило trust-affordance туда,
/// где пользователь его не давал.
///
/// Висящий диалог сперва получает `Cancel` в одноразовый канал — иначе
/// оркестратор агент-цикла прошлого чата зависнет навечно в `await`.
fn reset_tools_session(ctx: &AppCtx) {
    if let Some(pending) = ctx.tools.pending_approval.get_untracked() {
        pending.send(ToolDecision::Cancel);
    }
    ctx.tools.pending_approval.set(None);
    ctx.tools.allow_all.set(false);
}

/// Возвращает `ChatMeta` активного чата, если есть.
pub fn active_meta() -> Option<ChatMeta> {
    let ctx = use_context::<AppCtx>();
    let id = ctx.chat.active_chat_id.get_untracked()?;
    ctx.chat
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)
}
