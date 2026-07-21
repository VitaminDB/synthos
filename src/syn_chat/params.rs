//! Параметры sampling для Syn-чата.
//!
//! Сериализуется в [`AppConfig::syn_chat_defaults`] и в `StoredChat.syn_params`
//! (per-chat override). На каждом turn конвертируется в [`GenerationOptions`]
//! фасада `synaptix::facade::llm`.

use synaptix::facade::llm::GenerationOptions;
use serde::{Deserialize, Serialize};

/// Дефолты — баланс «разнообразие vs стабильность» из текущих констант
/// `syn_chat::session` плюс умеренный `repeat_penalty`, чтобы Qwen3 не
/// вырождался в loop'ы на простых prompt'ах.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SamplingParams {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub repeat_penalty: f32,
    pub repeat_last_n: u32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
    /// `-1` = энтропия из ОС, иначе deterministic seed для PRNG.
    pub seed: i64,
    pub max_new_tokens: u32,
    pub max_seq_len: u32,
    pub enable_thinking: bool,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            min_p: 0.0,
            repeat_penalty: 1.05,
            repeat_last_n: 64,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
            seed: -1,
            max_new_tokens: 131072,
            max_seq_len: 131072,
            enable_thinking: true,
        }
    }
}

impl SamplingParams {
    pub fn to_options(&self) -> GenerationOptions {
        let seed = if self.seed < 0 { 0 } else { self.seed as u64 };
        GenerationOptions {
            max_new_tokens: self.max_new_tokens as usize,
            max_seq_len: self.max_seq_len as usize,
            temperature: self.temperature,
            top_k: self.top_k as usize,
            top_p: self.top_p,
            min_p: self.min_p,
            seed,
            repeat_penalty: self.repeat_penalty,
            repeat_last_n: self.repeat_last_n as usize,
            presence_penalty: self.presence_penalty,
            frequency_penalty: self.frequency_penalty,
        }
    }
}
