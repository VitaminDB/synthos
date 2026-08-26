//! Постобработка распознанной речи через локальную LLM (нативный synaptix).
//!
//! Триггер — ручной клик пользователя по кнопке «обновить» над полем
//! «Отредактированный текст» в FAB-окне. Запрос — one-shot генерация на той же
//! загруженной Qwen3.6-модели (`SynModelRegistry`), что обслуживает основной
//! Syn-чат: короткое обращение, ответ — отредактированная версия
//! `raw`-стенограммы по системному промпту из Settings → Общие.
//!
//! Модель одна на GPU, поэтому постобработка не запускается, пока идёт
//! генерация основного чата (`SynChatCtx.pending`) — иначе коллизия на девайсе.

use syngui::tr;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use synaptix::facade::llm::{LlmGeneration, Message};

use crate::context::AppCtx;
use crate::syn_chat::model_registry::{LoadedSynModel, SynModelRegistry};
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::state::{SynChatCtx, ThinkParser};

/// Низкая температура для детерминированной корректуры. На выходе нужна
/// аккуратная редакция, а не «креатив».
const REFINE_TEMPERATURE: f32 = 0.2;
/// Верхняя граница длины редактуры — стенограмма короткая, ответ тоже.
const REFINE_MAX_NEW_TOKENS: usize = 2048;

/// Запустить постобработку. Должна вызываться с главного потока.
///
/// Поведение:
/// - `raw.trim().is_empty()` → молча no-op (нечего редактировать).
/// - системный промпт пуст → синхронно `voice.refined = raw` (без модели).
/// - модель не загружена / занята основной генерацией → `refined = raw` +
///   сообщение в `refine_error` (fallback, чтобы Paste не остался пустым).
/// - иначе ставит `voice.refining=true`, спавнит worker-thread с one-shot
///   генерацией, по завершению возвращается на main-поток и кладёт результат
///   либо в `voice.refined`, либо в `voice.refine_error` + fallback `raw`.
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

    // Модель для инференса.
    let Some(model) = use_context::<SynModelRegistry>().current.get_untracked() else {
        app.voice
            .refine_error
            .set(Some(tr!("voice.refine.model_not_loaded")));
        app.voice.refined.set(raw);
        app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
        return;
    };

    // Модель одна на GPU: если идёт генерация основного чата — не лезем.
    if use_context::<SynChatCtx>().pending.get_untracked() {
        app.voice
            .refine_error
            .set(Some(tr!("voice.refine.model_busy")));
        app.voice.refined.set(raw);
        app.voice.refined_gen.update(|n| *n = n.wrapping_add(1));
        return;
    }

    app.voice.refining.set(true);
    app.voice.refine_error.set(None);

    let raw_for_fallback = raw.clone();
    // Worker — обычный std::thread (synaptix-генерация блокирует поток / держит
    // CUDA); tokio-runtime не нужен — постобработка без async tool-вызовов.
    std::thread::spawn(move || {
        let result = run_refine(&model, &prompt, &raw);
        run_on_main_thread(move || {
            let app = use_context::<AppCtx>();
            app.voice.refining.set(false);
            match result {
                Ok(text) if !text.trim().is_empty() => {
                    app.voice.refined.set(text.trim().to_string());
                }
                Ok(_) => {
                    // Пустой ответ — fallback на сырой текст; ошибку не
                    // показываем (это валидное «модель ничего не вернула»).
                    app.voice.refined.set(raw_for_fallback);
                }
                Err(e) => {
                    let msg = tr!("voice.refine.unavailable", error = format!("{e:#}"));
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

/// Синхронная one-shot генерация редактуры. Блокирующая — вызывать только
/// с worker-потока. Промпт: `system = voice_refine_prompt`, `user = raw`,
/// без tools, `enable_thinking=false` (нужна редакция, а не рассуждения).
fn run_refine(model: &LoadedSynModel, system: &str, user: &str) -> anyhow::Result<String> {
    let history = vec![Message::system(system), Message::user(user)];
    let prompt = model
        .tokenizer
        .apply_chat_template_ex_tools(&history, true, false, None)?;
    let prompt_ids = model.tokenizer.encode(&prompt)?;

    // KV-ring cap (как в syn_chat::session::run_agent_loop): rope/embed таблицы
    // размером ровно max_seq_len, позиция model_cap — out of bounds; резервируем
    // минимум 1 позицию и не аллоцируем ring больше, чем реально нужно.
    let model_cap = model.model.config().max_seq_len;
    let usable_cap = model_cap.saturating_sub(1);
    let prompt_capped = prompt_ids.len().min(usable_cap);
    let headroom = usable_cap.saturating_sub(prompt_capped);

    let mut opts = SamplingParams::default().to_options();
    opts.temperature = REFINE_TEMPERATURE;
    let max_new = REFINE_MAX_NEW_TOKENS.min(headroom.max(1));
    opts.max_new_tokens = max_new;
    opts.max_seq_len = (prompt_capped + max_new + 128).min(usable_cap);

    let mut runner = LlmGeneration::new(&model.model, opts);
    crate::syn_chat::session::set_qwen3_stops(&mut runner, &model.tokenizer);

    let mut out = String::new();
    runner.generate_streaming(&prompt_ids, &model.tokenizer, |_id, delta| {
        out.push_str(delta);
        true
    })?;
    drop(runner);

    // enable_thinking=false, но защитно вырезаем `<think>…</think>`, если
    // шаблон всё же их выдал.
    let mut tp = ThinkParser::new();
    let split = tp.feed(&out);
    Ok(split.body.trim().to_string())
}
