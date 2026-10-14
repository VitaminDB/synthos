//! Генерация языковой модели чата в службе synaptixd: промпт по шаблону
//! модели, KV-кэш по ключу сессии (чат, субагент), разбор размышлений и
//! вызовов инструментов — на её стороне. Здесь — перевод истории в сообщения
//! протокола и прерывание хода (кнопка «Стоп», зацикливание).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use synaptix::facade::llm::Message;

use crate::syn_chat::params::SamplingParams;

/// Вложения сообщений истории: номер сообщения → файлы картинок и видео. В
/// тексте сообщения на их местах стоят [`synaptixd::MEDIA_MARK`].
pub type MediaAt = HashMap<usize, Vec<synaptixd::Media>>;

/// История для службы: роли, размышления и вызовы — как в шаблоне модели.
pub fn to_service(history: &[Message], media: &MediaAt) -> Vec<synaptixd::Message> {
    history
        .iter()
        .enumerate()
        .map(|(i, m)| synaptixd::Message {
            role: m.role.clone(),
            content: m.content.clone(),
            name: m.name.clone(),
            reasoning: m.reasoning.clone().filter(|r| !r.is_empty()),
            tool_calls: m.tool_calls.iter().map(|(name, arguments)| synaptixd::ToolCall { name: name.clone(), arguments: arguments.clone() }).collect(),
            media: media.get(&i).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Параметры хода: сэмплинг карточки Sampling, размышления, ключ KV-сессии.
pub fn params(p: &SamplingParams, session: Option<&str>) -> synaptixd::Params {
    let mut out = synaptixd::Params::new()
        .with("thinking", p.enable_thinking)
        .with("max_tokens", p.max_new_tokens as i64)
        .with("temperature", p.temperature as f64)
        .with("top_p", p.top_p as f64)
        .with("top_k", p.top_k as i64)
        .with("min_p", p.min_p as f64)
        .with("repeat_penalty", p.repeat_penalty as f64)
        .with("repeat_last_n", p.repeat_last_n as i64)
        .with("presence_penalty", p.presence_penalty as f64)
        .with("frequency_penalty", p.frequency_penalty as f64);
    if p.seed >= 0 {
        out = out.with("seed", p.seed);
    }
    if p.max_seq_len > 0 {
        out = out.with("context", p.max_seq_len as i64);
    }
    if let Some(e) = p.effort() {
        out = out.with("reasoning", e.to_string());
    }
    if let Some(s) = session {
        out = out.with("session", s.to_string());
    }
    out
}

/// Ход прерван: «Стоп» пользователя или `on_event` вернул `false`.
#[derive(Debug)]
pub struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ход прерван")
    }
}

impl std::error::Error for Interrupted {}

/// Ход модели в службе. `stop` опрашивается и во время префилла (событий
/// ещё нет); `on_event` возвращает `false`, чтобы оборвать ход.
pub fn generate(
    model: &str,
    messages: Vec<synaptixd::Message>,
    tools: Vec<serde_json::Value>,
    params: &synaptixd::Params,
    stop: Arc<dyn Fn() -> bool + Send + Sync>,
    on_event: &mut dyn FnMut(synaptixd::Event) -> bool,
) -> anyhow::Result<synaptixd::Generated> {
    let mut c = synaptixd::Client::connect().map_err(|e| anyhow::anyhow!("служба нейросетей: {e}"))?;
    let cancel = c.canceller();
    let cut = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    // Сторож «Стопа»: клиентский разрыв обрывает ход в службе и на префилле.
    let watch = {
        let (cut, done) = (cut.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                if cut.load(Ordering::SeqCst) || stop() {
                    cut.store(true, Ordering::SeqCst);
                    if let Some(k) = &cancel {
                        k.cancel();
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        })
    };
    let r = c.chat(model, messages, tools, params, |ev| {
        if !cut.load(Ordering::SeqCst) && !on_event(ev) {
            cut.store(true, Ordering::SeqCst);
        }
    });
    done.store(true, Ordering::SeqCst);
    let _ = watch.join();
    match r {
        Err(_) if cut.load(Ordering::SeqCst) => Err(Interrupted.into()),
        Err(e) => Err(anyhow::anyhow!("служба нейросетей: {e}")),
        Ok(g) => Ok(g),
    }
}

/// Простой ход без инструментов (сводка, правка текста): ответ целиком.
pub fn complete(model: &str, history: &[Message], params: &synaptixd::Params, stop: Arc<dyn Fn() -> bool + Send + Sync>) -> anyhow::Result<synaptixd::Generated> {
    generate(model, to_service(history, &MediaAt::new()), Vec::new(), params, stop, &mut |_| true)
}
