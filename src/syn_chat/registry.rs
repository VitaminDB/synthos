//! CRUD над списком Syn-чатов.
//!
//! Все функции — для вызова с main thread (`use_context::<SynChatCtx>()`).

use std::hash::{Hash, Hasher};

use syngui::prelude::*;

use crate::agent::time::{unix_nanos, unix_secs};

use super::chat_settings::ChatSettings;
use super::params::SamplingParams;
use super::state::{ChatMeta, ChatMsg, SynChatCtx};
use super::storage::{self, StoredChat};
use super::{autosave, session, telemetry};

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
///
/// Настройки чата (инструменты, скилы, промпт) входят в отпечаток так же,
/// как params: их правка — повод записать файл.
pub fn state_fingerprint(
    title: &str,
    messages: &[ChatMsg],
    params: &SamplingParams,
    settings: &ChatSettings,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fingerprint(title, messages).hash(&mut h);
    // serde_json для f32/u32 — стабильный hash без NaN-issues.
    if let Ok(bytes) = serde_json::to_vec(params) {
        bytes.hash(&mut h);
    }
    settings.hash(&mut h);
    h.finish()
}

/// Перезаливает список чатов с диска и активирует самый свежий.
pub fn load_all() {
    let ctx = use_context::<SynChatCtx>();
    ctx.loading.set(true);
    // Сигналы панелей сейчас — стартовые, из конфига и библиотеки промптов:
    // ровно те общие настройки, с которыми работали чаты, записанные до
    // того, как настройки переехали в файл чата. Такие файлы получают их.
    let metas = storage::list_meta_filling(&ChatSettings::capture(&ctx));
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
    // Покидаемый чат дописывается до переключения: отложенная запись
    // досталась бы уже новому чату, и правки прежнего пропали бы.
    autosave::flush();
    let now = unix_secs();
    let id = format!("{:016x}", unix_nanos());
    // Новый чат начинает с настроек открытого (сэмплинг — так же: `params`
    // при создании не сбрасываются), дальше они у каждого свои.
    let settings = ChatSettings::capture(&ctx);
    let stored = StoredChat {
        id: id.clone(),
        title: tr!("chat.registry.new_chat_title"),
        created_at: now,
        updated_at: now,
        model_name: None,
        messages: Vec::new(),
        syn_params: None,
        settings: Some(settings.clone()),
        archived: false,
    };
    storage::save_async(stored.clone());
    ctx.chats.update(|list| list.insert(0, stored.to_meta()));
    ctx.loading.set(true);
    leave_current_chat();
    ctx.active_chat_id.set(Some(id.clone()));
    ctx.messages.set(Vec::new());
    ctx.input.set(String::new());
    ctx.clear_draft_attachments();
    ctx.error.set(None);
    // Отпечаток ставим последним — params к этому моменту уже те, с
    // которыми чат уйдёт в автосейв.
    ctx.last_saved_fp.set(state_fingerprint(
        &stored.title,
        &[],
        &ctx.params.get_untracked(),
        &settings,
    ));
    ctx.loading.set(false);
    id
}

/// Общее для `select` и `create_new`: отпустить всё, что принадлежало
/// покидаемому чату.
///
/// Кэш префикс-KV живёт в единственном глобальном слоте и ключуется id
/// чата — уходя, чат его всё равно теряет. Освобождаем сразу, а не ждём
/// первой генерации в новом чате: иначе гигабайты VRAM висят под контекст,
/// к которому уже никто не обратится (а если вернуться назад — кэш всё
/// равно пересоберётся полным префиллом, лениво он не восстанавливается).
fn leave_current_chat() {
    let ctx = use_context::<SynChatCtx>();
    // Кэш префикс-KV принадлежит чату, который сейчас считает: с 04.09.2026
    // ход переживает переключение, и ронять его контекст на полпути нельзя —
    // это стоило бы полного префилла на следующем же ходу.
    if ctx.generating_chat.get_untracked().is_none() {
        session::drop_kv_session();
    }
    telemetry::reset();
    // Начатая правка названия принадлежит покидаемому чату — поле не
    // должно пережить переключение и открыться на новом.
    ctx.renaming_chat.set(false);
}

pub fn select(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    select_internal(id, &ctx);
}

/// Снять с хвоста ленты пустые assistant-плейсхолдеры: без текста,
/// размышлений, вызовов и вложений.
fn trim_dead_placeholders(messages: &mut Vec<crate::agent::state::ChatMsg>) {
    use crate::agent::state::{ChatMsgKind, ChatMsgRole};
    while messages.last().is_some_and(|m| {
        m.role == ChatMsgRole::Assistant
            && matches!(m.kind, ChatMsgKind::Text)
            && m.body.trim().is_empty()
            && m.thinking.trim().is_empty()
            && m.tool_calls.as_ref().is_none_or(|c| c.is_empty())
            && m.attachments.is_empty()
    }) {
        messages.pop();
    }
}

fn select_internal(id: &str, ctx: &SynChatCtx) {
    // Открытый чат дописывается до того, как лента сменится: отложенная
    // запись сняла бы снимок уже с нового. Повторный выбор того же чата
    // перечитывает его с диска — там тоже должны быть последние правки.
    autosave::flush();
    // Играющий звук инлайн-карточек принадлежал прежней ленте. Обычная
    // уборка его бережёт (строка могла просто уйти из окна), поэтому здесь
    // останавливаем явно.
    crate::pages::syn_chat::media_audio::stop_inline_all();
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
    // Повторный выбор уже открытого чата (клик по активной плитке) кэш не
    // роняет — пересобирать его полным префиллом было бы за что.
    if ctx.active_chat_id.get_untracked().as_deref() != Some(stored.id.as_str()) {
        leave_current_chat();
    }
    let stored_id = stored.id.clone();
    ctx.active_chat_id.set(Some(stored.id.clone()));
    let title = stored.title.clone();
    let mut messages = stored.messages;
    // Ход, оборванный закрытием приложения, оставляет в файле пустой
    // assistant-плейсхолдер: пустой пузырь в конце ленты выглядит как
    // зависшая генерация и уходит пустой репликой в историю следующего хода.
    // Если чат сейчас не генерирует — это мусор.
    if ctx.generating_chat.get_untracked().as_deref() != Some(stored_id.as_str()) {
        trim_dead_placeholders(&mut messages);
    }
    ctx.messages.set(messages.clone());
    // Per-chat sampling params: либо то, что сохранено в чате, либо дефолты
    // из AppConfig (для свежих чатов и старых файлов без поля).
    let params = stored
        .syn_params
        .unwrap_or_else(|| crate::config::AppConfig::load().syn_chat_defaults);
    ctx.params.set_always(params.clone());
    // Инструменты, скилы и промпт чата — в панели. Файла без них после
    // заполнения на старте (`load_all`) не бывает; если всё же попался —
    // чат остаётся с тем, что открыто сейчас, и запишет это с первой правкой.
    let settings = stored
        .settings
        .unwrap_or_else(|| ChatSettings::capture(ctx));
    settings.apply(ctx);
    // Отпечаток — ровно тот, что посчитает автосейв. Иначе выбор чата
    // выглядит для него как правка и двигает чат наверх списка.
    ctx.last_saved_fp
        .set(state_fingerprint(&title, &messages, &params, &settings));
    ctx.input.set(String::new());
    ctx.input_tokens.set_always(0);
    // Черновик вложений принадлежал прошлому чату — сами blob'ы остаются
    // в CAS, но к новому чату они не прикрепляются.
    ctx.clear_draft_attachments();
    ctx.viewer.set(None);
    // Подсветка принадлежала прошлому переходу из поиска, правка — прошлой ленте.
    ctx.highlight_msg.set(None);
    ctx.editing_msg.set(None);
    ctx.queue_editing.set(None);
    ctx.wizard_drafts.update(|m| m.clear());
    ctx.error.set(None);
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    // Генерация прошлого чата продолжается в фоне: её сообщения пишутся
    // прямо в файл своего чата (`session::ledger_update`), а сюда вернутся
    // при следующем открытии. `pending` — про открытый чат: он «занят»
    // только если считает именно он.
    ctx.pending
        .set(ctx.generating_chat.get_untracked().as_deref() == Some(stored_id.as_str()));
    ctx.loading.set(false);
    // В этом чате могли ждать сообщения очереди, а ход другого чата уже
    // закончился: отправляем отдельным тиком, когда переключение завершено.
    syngui::async_runtime::run_on_main_thread(crate::syn_chat::session::flush_queue);
}

pub fn delete(id: &str) {
    let ctx = use_context::<SynChatCtx>();
    // Сборка мусора ниже берёт ссылки на вложения с диска: открытый чат
    // должен лежать там со всеми правками, иначе его свежие blob'ы уйдут.
    autosave::flush();
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
            ctx.clear_draft_attachments();
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
        // Снимок — полное состояние чата: отложенная запись не нужна.
        autosave::cancel();
        if let Some(mut snap) = snapshot_current() {
            snap.archived = true;
            // Иначе переход на следующий чат записал бы тот же снимок ещё раз.
            ctx.last_saved_fp.set(state_fingerprint(
                &snap.title,
                &snap.messages,
                &ctx.params.get_untracked(),
                snap.settings.as_ref().expect("снимок открытого чата с настройками"),
            ));
            storage::save_async(snap);
        }
    } else {
        storage::update_async(id, |stored| stored.archived = true);
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
    // `select_internal` ниже читает файл после этой правки: `load` ждёт
    // очередь записи.
    storage::update_async(id, |stored| stored.archived = false);
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
    // Как в `delete`: ссылки открытого чата на вложения — на диск до GC.
    autosave::flush();
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
/// в файл чата. Реестра моделей нет только в тестах.
pub(crate) fn current_model_name() -> Option<String> {
    try_use_context::<crate::syn_chat::SynModelRegistry>()?
        .current
        .get_untracked()
        .and_then(|m| {
            m.path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
}

/// Файл чата из его `ChatMeta` и ленты. `created_at` и прежнее имя модели
/// берутся из меты, а не с диска: раньше каждый автосейв ради них читал и
/// разбирал весь файл чата.
pub(crate) fn stored_from_meta(
    meta: &ChatMeta,
    messages: Vec<ChatMsg>,
    params: SamplingParams,
    settings: ChatSettings,
    model_name: Option<String>,
) -> StoredChat {
    StoredChat {
        id: meta.id.clone(),
        title: meta.title.clone(),
        created_at: meta.created_at,
        updated_at: unix_secs(),
        model_name,
        messages,
        syn_params: Some(params),
        settings: Some(settings),
        archived: meta.archived,
    }
}

pub fn snapshot_current() -> Option<StoredChat> {
    let ctx = use_context::<SynChatCtx>();
    let id = ctx.active_chat_id.get_untracked()?;
    let meta = ctx
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == id)?;
    // Имя модели раньше не сохранялось вообще: по файлу чата нельзя было
    // понять, какой бандл отвечал, — а разбор поведения агента без этого
    // сводится к угадыванию. Пишем имя текущего бандла; если модель ещё не
    // загружена — оставляем то, что было записано раньше (оно в мете).
    let model_name = current_model_name().or_else(|| meta.model_name.clone());
    Some(stored_from_meta(
        &meta,
        ctx.messages.get_untracked(),
        ctx.params.get_untracked(),
        ChatSettings::capture(&ctx),
        model_name,
    ))
}

/// Обновляет preview активного meta — для отображения «свежие сверху».
/// Имя модели, с которым уходит запись, кладётся туда же: следующий снимок
/// возьмёт его из меты, если модель к тому времени выгрузят.
pub fn refresh_active_preview(messages: &[ChatMsg], model_name: Option<String>) {
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
            if model_name.is_some() {
                m.model_name = model_name;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::ChatMsg;

    fn msg(role: &str, body: &str, kind: serde_json::Value) -> ChatMsg {
        serde_json::from_value(serde_json::json!({
            "role": role, "author": "", "initials": "", "tone_class": "",
            "time": "20:23", "body": body, "error": false, "kind": kind,
        }))
        .expect("ChatMsg из JSON файла чата")
    }

    /// Хвост чата «Vocal» после закрытия приложения посреди хода: пустой
    /// assistant-плейсхолдер снимается, всё содержательное остаётся.
    #[test]
    fn dead_placeholders_are_trimmed_from_tail_only() {
        let text = serde_json::json!({"variant": "text"});
        let result = serde_json::json!({
            "variant": "tool_result", "tool_call_id": "c1", "tool_name": "pipelines", "error": true
        });
        let mut messages = vec![
            msg("User", "извлеки вокал", text.clone()),
            msg("Assistant", "", text.clone()),
            msg("System", "--- Прогон остановлен ---", result),
            msg("Assistant", "  \n", text.clone()),
            msg("Assistant", "", text),
        ];
        trim_dead_placeholders(&mut messages);
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[2].body, "--- Прогон остановлен ---");
        // Пустой ответ в середине ленты — не хвост, его не трогаем.
        assert!(messages[1].body.is_empty());
    }
}
