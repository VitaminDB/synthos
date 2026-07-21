//! Постобработка распознанной речи через локальную LLM.
//!
//! Триггер — ручной клик пользователя по кнопке «обновить» над полем
//! «Отредактированный текст» в FAB-окне. Запрос — non-streaming
//! `POST /v1/chat/completions` к тому же llama-server, что обслуживает
//! основной чат: обращение короткое, ответ — отредактированная версия
//! `raw`-стенограммы по системному промпту из Settings → Общие.

use syngui::async_runtime::{run_on_main_thread, spawn};

use crate::context::AppCtx;
use crate::llama::api::{ChatMessage, ChatRequest, LlamaClient, SamplingParams};

/// Низкая температура для детерминированной корректуры. На выходе нужна
/// аккуратная редакция, а не «креатив».
const REFINE_TEMPERATURE: f32 = 0.2;

/// Запустить постобработку. Должна вызываться с главного потока.
///
/// Поведение:
/// - `raw.trim().is_empty()` → молча no-op (нечего редактировать).
/// - системный промпт пуст → синхронно `voice.refined = raw` (без сети).
/// - иначе ставит `voice.refining=true`, спавнит async-запрос, по завершению
///   возвращается на main-поток и кладёт результат либо в `voice.refined`,
///   либо в `voice.refine_error` + fallback `voice.refined = raw`.
pub fn refine_voice_text(app: AppCtx, raw: String) {
    if raw.trim().is_empty() {
        return;
    }
    let prompt = app.general.voice_refine_prompt.get_untracked();
    if prompt.trim().is_empty() {
        // Нет промпта — постобработка no-op: показываем сырой текст как есть.
        app.voice.refined.set(raw);
        app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
        app.voice.refine_error.set(None);
        return;
    }
    if app.voice.refining.get_untracked() {
        return;
    }

    let host = app.general.server_host.get_untracked();
    let port = app.general.server_port.get_untracked();
    let base_url = format!("http://{}:{}", host, port);

    app.voice.refining.set(true);
    app.voice.refine_error.set(None);

    let raw_for_fallback = raw.clone();
    spawn(async move {
        let client = LlamaClient::with_base_url(base_url);
        let mut req = ChatRequest::new([
            ChatMessage::system(prompt),
            ChatMessage::user(raw),
        ]);
        req.sampling = SamplingParams::new().with_temperature(REFINE_TEMPERATURE);

        let result = client.chat_completions(&req).await;
        run_on_main_thread(move || {
            let app = use_context::<AppCtx>();
            app.voice.refining.set(false);
            match result {
                Ok(resp) => {
                    let text = resp
                        .first_text()
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if text.is_empty() {
                        // Пустой ответ — fallback на сырой текст; ошибку не
                        // показываем (это валидное «модель ничего не вернула»).
                        app.voice.refined.set(raw_for_fallback);
                    } else {
                        app.voice.refined.set(text);
                    }
                }
                Err(e) => {
                    let msg = format!("Постобработка недоступна: {e}");
                    eprintln!("[synthos/voice_refine] {msg}");
                    app.voice.refine_error.set(Some(msg));
                    // Чтобы Paste не оставался без текста — кладём raw как fallback.
                    app.voice.refined.set(raw_for_fallback);
                }
            }
            // Bump поколения поля после любого исхода, чтобы Reactive-обёртка
            // пересоздала MultilineTextEdit с новым text.
            app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
        });
    });
}

use syngui::context_provider::use_context;
