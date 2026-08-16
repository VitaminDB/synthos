//! CRUD над списком Syn-чатов.
//!
//! Все функции — для вызова с main thread (`use_context::<SynChatCtx>()`).

use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;

use syngui::prelude::*;

use crate::agent::time::{unix_nanos, unix_secs};

use super::state::{ChatMeta, ChatMsg, SynChatCtx};
use super::storage::{self, StoredChat};

/// Контент-зависимый отпечаток чата для пропуска идемпотентного автосейва.
pub fn fingerprint(title: &str, messages: &[ChatMsg]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut h);
    messages.hash(&mut h);
    h.finish()
}

/// Перезаливает список чатов с диска и активирует самый свежий.
pub fn load_all() {
    let ctx = use_context::<SynChatCtx>();
    ctx.loading.set(true);
    let metas = storage::list_meta();
    if let Some(top) = metas.first().cloned() {
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
        title: "Новый чат".to_string(),
        created_at: now,
        updated_at: now,
        model_name: None,
        messages: Vec::new(),
        syn_params: None,
    };
    storage::save(&stored);
    ctx.chats.update(|list| list.insert(0, stored.to_meta()));
    ctx.loading.set(true);
    ctx.active_chat_id.set(Some(id.clone()));
    ctx.last_saved_fp.set(fingerprint(&stored.title, &[]));
    ctx.messages.set(Vec::new());
    ctx.input.set(String::new());
    ctx.error.set(None);
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
    let fp = fingerprint(&stored.title, &stored.messages);
    ctx.last_saved_fp.set(fp);
    ctx.messages.set(stored.messages);
    // Per-chat sampling params: либо то, что сохранено в чате, либо дефолты
    // из AppConfig (для свежих чатов и старых файлов без поля).
    let params = stored
        .syn_params
        .unwrap_or_else(|| crate::config::AppConfig::load().syn_chat_defaults);
    ctx.params.set_always(params);
    ctx.input.set(String::new());
    ctx.input_tokens.set_always(0);
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
    let was_active = ctx.active_chat_id.get_untracked().as_deref() == Some(id);
    ctx.chats.update(|list| list.retain(|m| m.id != id));
    if was_active {
        let next_id = ctx.chats.get_untracked().first().map(|m| m.id.clone());
        match next_id {
            Some(id) => select_internal(&id, &ctx),
            None => {
                ctx.loading.set(true);
                ctx.active_chat_id.set(None);
                ctx.messages.set(Vec::new());
                ctx.input.set(String::new());
                ctx.loading.set(false);
            }
        }
    }
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

pub fn snapshot_current() -> Option<StoredChat> {
    let ctx = use_context::<SynChatCtx>();
    let id = ctx.active_chat_id.get_untracked()?;
    let meta = ctx
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)?;
    let messages: Vec<ChatMsg> = ctx.messages.get_untracked();
    let created_at = storage::load(&id).map(|c| c.created_at).unwrap_or_else(unix_secs);
    Some(StoredChat {
        id: meta.id.clone(),
        title: meta.title.clone(),
        created_at,
        updated_at: unix_secs(),
        model_name: None,
        messages,
        syn_params: Some(ctx.params.get_untracked()),
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
