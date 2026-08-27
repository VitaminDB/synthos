//! CRUD над списком Syn-чатов.
//!
//! Все функции — для вызова с main thread (`use_context::<SynChatCtx>()`).

use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;

use syngui::prelude::*;

use crate::agent::time::{unix_nanos, unix_secs};

use super::params::SamplingParams;
use super::state::{ChatMeta, ChatMsg, SynChatCtx};
use super::storage::{self, StoredChat};

/// Контент-зависимый отпечаток чата для пропуска идемпотентного автосейва.
pub fn fingerprint(title: &str, messages: &[ChatMsg]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut h);
    messages.hash(&mut h);
    h.finish()
}

/// Полный отпечаток состояния чата — то, что автосейв в
/// `install_syn_chat_autosave` сравнивает с `last_saved_fp`.
///
/// Обязан считаться ровно одинаково и здесь, и в автосейве: раньше
/// `select_internal` клал в `last_saved_fp` «голый» [`fingerprint`] без
/// params, а автосейв сравнивал с хэшем `(fingerprint, params)`. Значения
/// не совпадали никогда, поэтому сразу после выбора чата автосейв считал
/// его изменённым, звал `refresh_active_preview` — и чат прыгал наверх
/// списка с новым `updated_at`. Одна функция на оба места убирает этот
/// класс расхождений.
pub fn state_fingerprint(title: &str, messages: &[ChatMsg], params: &SamplingParams) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fingerprint(title, messages).hash(&mut h);
    // serde_json для f32/u32 — стабильный hash без NaN-issues.
    if let Ok(bytes) = serde_json::to_vec(params) {
        bytes.hash(&mut h);
    }
    h.finish()
}

/// Перезаливает список чатов с диска и активирует самый свежий.
pub fn load_all() {
    let ctx = use_context::<SynChatCtx>();
    ctx.loading.set(true);
    let metas = storage::list_meta();
    // Активным становится самый свежий из НЕархивных: архивные чаты в
    // рейле не видны, и открывать их молча нельзя.
    if let Some(top) = metas.iter().find(|m| !m.archived).cloned() {
        ctx.chats.set(metas);
        select_internal(&top.id, &ctx);
    } else {
        ctx.chats.set(Vec::new());
        ctx.active_chat_id.set(None);
        ctx.messages.set(Vec::new());
    }
    ctx.loading.set(false);
}

/// Создаёт новый пустой чат и делает его активным. Возвращает id.
pub fn create_new() -> String {
    let ctx = use_context::<SynChatCtx>();
    let now = unix_secs();
    let id = format!("{:016x}", unix_nanos());
    let stored = StoredChat {
        id: id.clone(),
        title: tr!("chat.registry.new_chat_title"),
        created_at: now,
        updated_at: now,
        model_name: None,
        messages: Vec::new(),
        syn_params: None,
        archived: false,
    };
    storage::save(&stored);
    ctx.chats.update(|list| list.insert(0, stored.to_meta()));
    ctx.loading.set(true);
    ctx.active_chat_id.set(Some(id.clone()));
    ctx.messages.set(Vec::new());
    ctx.input.set(String::new());
    ctx.pending_attachments.set(Vec::new());
    ctx.error.set(None);
    // Отпечаток ставим последним — params к этому моменту уже те, с
    // которыми чат уйдёт в автосейв.
    ctx.last_saved_fp
        .set(state_fingerprint(&stored.title, &[], &ctx.params.get_untracked()));
    ctx.loading.set(false);
    id
}

pub fn select(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    select_internal(id, &ctx);
}

fn select_internal(id: &str, ctx: &SynChatCtx) {
    let Some(stored) = storage::load(id) else {
        eprintln!("[syn_chat] чат {id} не найден на диске");
        ctx.chats.update(|list| list.retain(|m| m.id != id));
        if ctx.active_chat_id.get_untracked().as_deref() == Some(id) {
            ctx.active_chat_id.set(None);
            ctx.messages.set(Vec::new());
        }
        return;
    };
    ctx.loading.set(true);
    ctx.active_chat_id.set(Some(stored.id.clone()));
    let title = stored.title.clone();
    let messages = stored.messages;
    ctx.messages.set(messages.clone());
    // Per-chat sampling params: либо то, что сохранено в чате, либо дефолты
    // из AppConfig (для свежих чатов и старых файлов без поля).
    let params = stored
        .syn_params
        .unwrap_or_else(|| crate::config::AppConfig::load().syn_chat_defaults);
    ctx.params.set_always(params.clone());
    // Отпечаток — ровно тот, что посчитает автосейв. Иначе выбор чата
    // выглядит для него как правка и двигает чат наверх списка.
    ctx.last_saved_fp
        .set(state_fingerprint(&title, &messages, &params));
    ctx.input.set(String::new());
    ctx.input_tokens.set_always(0);
    // Черновик вложений принадлежал прошлому чату — сами blob'ы остаются
    // в CAS, но к новому чату они не прикрепляются.
    ctx.pending_attachments.set(Vec::new());
    ctx.viewer.set(None);
    // Подсветка принадлежала прошлому переходу из поиска, правка — прошлой ленте.
    ctx.highlight_msg.set(None);
    ctx.editing_msg.set(None);
    ctx.error.set(None);
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    // Сбрасываем любую идущую генерацию — она принадлежала прошлому чату.
    ctx.abort.fetch_add(1, Ordering::Relaxed);
    ctx.pending.set(false);
    ctx.loading.set(false);
}

pub fn delete(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    storage::delete(id);
    // Blob'ы удалённого чата больше никому не нужны — но только если на
    // них не ссылается другой чат (CAS дедуплицирует по содержимому).
    crate::syn_chat::attach::gc_after_delete();
    let was_active = ctx.active_chat_id.get_untracked().as_deref() == Some(id);
    ctx.chats.update(|list| list.retain(|m| m.id != id));
    if was_active {
        select_next_visible(&ctx);
    }
}

/// После ухода активного чата (архив/удаление) — открыть самый свежий из
/// оставшихся видимых или очистить ленту, если таких нет.
fn select_next_visible(ctx: &SynChatCtx) {
    let next_id = ctx
        .chats
        .get_untracked()
        .iter()
        .find(|m| !m.archived)
        .map(|m| m.id.clone());
    match next_id {
        Some(id) => select_internal(&id, ctx),
        None => {
            ctx.loading.set(true);
            ctx.active_chat_id.set(None);
            ctx.messages.set(Vec::new());
            ctx.input.set(String::new());
            ctx.pending_attachments.set(Vec::new());
            ctx.loading.set(false);
        }
    }
}

/// Убрать чат в архив: файл остаётся, в рейле плитка исчезает. Активный
/// чат перед этим сохраняется — иначе автосейв, не найдя его в списке,
/// потерял бы последние сообщения.
pub fn archive(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    let was_active = ctx.active_chat_id.get_untracked().as_deref() == Some(id);
    if was_active {
        if let Some(mut snap) = snapshot_current() {
            snap.archived = true;
            storage::save(&snap);
        }
    } else if let Some(mut stored) = storage::load(id) {
        stored.archived = true;
        storage::save(&stored);
    }
    ctx.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.archived = true;
        }
    });
    if was_active {
        select_next_visible(&ctx);
    }
}

/// Вернуть чат из архива в рейл и сделать его активным.
pub fn unarchive(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    if let Some(mut stored) = storage::load(id) {
        stored.archived = false;
        storage::save(&stored);
    }
    ctx.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.archived = false;
        }
    });
    select_internal(id, &ctx);
}

/// Удалить насовсем все чаты из архива.
pub fn clear_archive() {
    let ctx = use_context::<SynChatCtx>();
    let ids: Vec<String> = ctx
        .chats
        .get_untracked()
        .iter()
        .filter(|m| m.archived)
        .map(|m| m.id.clone())
        .collect();
    for id in ids {
        storage::delete(&id);
    }
    crate::syn_chat::attach::gc_after_delete();
    ctx.chats.update(|list| list.retain(|m| !m.archived));
}

pub fn rename_active(title: String) {
    let ctx = use_context::<SynChatCtx>();
    let Some(id) = ctx.active_chat_id.get_untracked() else {
        return;
    };
    ctx.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.title = title;
        }
    });
}

/// Имя загруженного бандла (`qwen3.8-27b.syn` → `qwen3.8-27b`) для записи
/// в файл чата.
fn current_model_name() -> Option<String> {
    use_context::<crate::syn_chat::SynModelRegistry>()
        .current
        .get_untracked()
        .and_then(|m| {
            m.path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
}

pub fn snapshot_current() -> Option<StoredChat> {
    let ctx = use_context::<SynChatCtx>();
    let id = ctx.active_chat_id.get_untracked()?;
    let meta = ctx
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)?;
    let messages: Vec<ChatMsg> = ctx.messages.get_untracked();
    let on_disk = storage::load(&id);
    let created_at = on_disk.as_ref().map(|c| c.created_at).unwrap_or_else(unix_secs);
    // Имя модели раньше не сохранялось вообще: по файлу чата нельзя было
    // понять, какой бандл отвечал, — а разбор поведения агента без этого
    // сводится к угадыванию. Пишем имя текущего бандла; если модель ещё не
    // загружена — оставляем то, что было записано раньше.
    let model_name = current_model_name()
        .or_else(|| on_disk.as_ref().and_then(|c| c.model_name.clone()));
    Some(StoredChat {
        id: meta.id.clone(),
        title: meta.title.clone(),
        created_at,
        updated_at: unix_secs(),
        model_name,
        messages,
        syn_params: Some(ctx.params.get_untracked()),
        archived: meta.archived,
    })
}

/// Обновляет preview активного meta — для мгновенного отображения «свежие сверху».
pub fn refresh_active_preview(messages: &[ChatMsg]) {
    let ctx = use_context::<SynChatCtx>();
    let Some(id) = ctx.active_chat_id.get_untracked() else {
        return;
    };
    let preview = storage::preview_from_messages(messages);
    let updated_at = unix_secs();
    ctx.chats.update(|list| {
        if let Some(m) = list.iter_mut().find(|m| m.id == id) {
            m.preview = preview;
            m.updated_at = updated_at;
        }
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    });
}

pub fn active_meta() -> Option<ChatMeta> {
    let ctx = use_context::<SynChatCtx>();
    let id = ctx.active_chat_id.get_untracked()?;
    ctx.chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)
}
