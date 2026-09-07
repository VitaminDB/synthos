//! Параметры sampling для Syn-чата.
//!
//! Сериализуется в [`AppConfig::syn_chat_defaults`] и в `StoredChat.syn_params`
//! (per-chat override). На каждом turn конвертируется в [`GenerationOptions`]
//! фасада `synaptix::facade::llm`.

use std::sync::atomic::{AtomicU64, Ordering};

use synaptix::facade::llm::GenerationOptions;
use serde::{Deserialize, Serialize};

/// Дефолты — рекомендованные Qwen для семейства Qwen3 в режиме размышлений:
/// temperature 0.6, top_p 0.95, top_k 20, min_p 0, без штрафов за повторы.
/// Greedy-декод (temperature 0) Qwen прямо не советует: он даёт повторы, а
/// повтор в агентном цикле включает guard. Штрафы за повторы (repeat/
/// frequency) в агентном режиме вредны: они бьют по самым частым токенам
/// вызова инструмента — кавычкам, переводам строк, скобкам — и модель
/// подменяет их редкими вариантами, ломая синтаксис вызова (см. разбор
/// 03.09.2026 в `docs/chat_tool_call_robustness_2026.md`).
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
    /// `-1` = энтропия из ОС (каждый ход — свой seed), иначе deterministic
    /// seed для PRNG.
    pub seed: i64,
    pub max_new_tokens: u32,
    pub max_seq_len: u32,
    pub enable_thinking: bool,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: 0.6,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,
            repeat_penalty: 1.0,
            repeat_last_n: 64,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
            seed: -1,
            // 16384, а не «сколько влезет»: бюджет ответа входит в план KV
            // каждого хода (см. `session::RING_ANSWER_TOKENS`), и потолок в
            // 131072 лишь занимал память под ринг. 16k токенов хватает и на
            // большой файл — 4096 модели бывало мало (пользователь, 07.09.2026).
            max_new_tokens: 16384,
            max_seq_len: 131072,
            enable_thinking: true,
        }
    }
}

/// Счётчик вызовов — подмешивается к времени, чтобы два `to_options` в одну
/// наносекунду не получили одинаковый seed.
static SEED_TICK: AtomicU64 = AtomicU64::new(0);

/// Seed «из энтропии ОС» для `seed = -1`. Раньше эта ветка отдавала
/// константу 0: sampler получал один и тот же PRNG на каждом ходу, и при
/// почти не менявшемся промпте agent-loop воспроизводил ровно тот же ответ —
/// прямой вклад в зацикливание на одном и том же tool-вызове.
fn entropy_seed() -> u64 {
    let nanos = crate::agent::time::unix_nanos() as u64;
    let tick = SEED_TICK.fetch_add(1, Ordering::Relaxed);
    // splitmix64-финализатор: соседние наносекунды дают несоседние seed'ы.
    let mut x = nanos ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

impl SamplingParams {
    /// Потолок ответа по умолчанию до 07.09.2026 — см.
    /// [`crate::config::AppConfig::migrate_sampling_defaults`].
    pub const LEGACY_MAX_NEW_TOKENS: u32 = 131072;

    /// Дефолты до 03.09.2026 (0.7 / 0.9 / 40 / repeat 1.05). Нужны только
    /// [`crate::config::AppConfig::migrate_sampling_defaults`]: конфиг с ровно
    /// этим набором переводится на новые дефолты, изменённый пользователем —
    /// не трогается.
    pub fn legacy_v1() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.05,
            ..Self::default()
        }
    }

    pub fn to_options(&self) -> GenerationOptions {
        let seed = if self.seed < 0 {
            entropy_seed()
        } else {
            self.seed as u64
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_seed_varies_between_turns() {
        let p = SamplingParams { seed: -1, ..Default::default() };
        let a = p.to_options().seed;
        let b = p.to_options().seed;
        assert_ne!(a, b, "seed=-1 обязан быть разным на каждом ходу");
    }

    #[test]
    fn explicit_seed_is_reproducible() {
        let p = SamplingParams { seed: 42, ..Default::default() };
        assert_eq!(p.to_options().seed, 42);
        assert_eq!(p.to_options().seed, 42);
    }

    #[test]
    fn penalties_reach_generation_options() {
        let p = SamplingParams {
            repeat_last_n: 64,
            presence_penalty: 0.5,
            frequency_penalty: 0.25,
            ..Default::default()
        };
        let o = p.to_options();
        assert_eq!(o.repeat_last_n, 64);
        assert_eq!(o.presence_penalty, 0.5);
        assert_eq!(o.frequency_penalty, 0.25);
    }
}
