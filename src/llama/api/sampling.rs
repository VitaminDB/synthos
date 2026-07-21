//! Sampling-параметры — общий набор для `/completion`, `/v1/completions`,
//! `/chat/completions`, `/infill`, `/v1/responses`.
//!
//! Все поля `Option<T>` со `skip_serializing_if = "Option::is_none"`: клиент
//! не передаёт то, что пользователь не выставил явно, и сервер применит свои
//! defaults (зафиксированы в `app/synthos/docs/llama-server-api.md`).
//!
//! Структура встраивается в запросы через `#[serde(flatten)]`:
//!
//! ```ignore
//! #[derive(Serialize)]
//! pub struct CompletionRequest {
//!     pub prompt: ...,
//!     #[serde(flatten)]
//!     pub sampling: SamplingParams,
//! }
//! ```
//!
//! Так клиентские запросы получают все sampling-поля одним набором, а
//! билдер остаётся читаемым.

use serde::{Deserialize, Serialize};

/// Полный набор sampling-параметров. Конвенция: `None` — «не переопределять».
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingParams {
    // ── Температура ──────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynatemp_range: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynatemp_exponent: Option<f32>,

    // ── Top-K / Top-P / Min-P / Typical / XTC / Top-N-Sigma ─────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typical_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_n_sigma: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xtc_probability: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xtc_threshold: Option<f32>,

    // ── Mirostat ─────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirostat: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirostat_tau: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirostat_eta: Option<f32>,

    // ── Penalties ────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_last_n: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,

    // ── DRY ──────────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_base: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_allowed_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_penalty_last_n: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_sequence_breakers: Option<Vec<String>>,

    // ── Grammar / JSON schema ────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grammar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grammar_lazy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<serde_json::Value>,

    // ── Общее ────────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samplers: Option<SamplerOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_eos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<LogitBias>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_probs: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_keep: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_max_predict_ms: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_keep: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_indent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_discard: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_cmpl: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_cache_reuse: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings_per_token: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_progress: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_tokens: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_sampling_probs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_prompt: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_slot: Option<i32>,

    /// `response_fields`: список полей ответа. Поддерживает разделитель
    /// `/` для «разнесения» вложенных полей в корень (например
    /// `"generation_settings/n_predict"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_fields: Option<Vec<String>>,

    /// Per-request LoRA-патчи: `[{id, scale}, ...]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lora: Option<Vec<LoraPatch>>,

    /// Устаревшее: multimodal через `image_data`. Для новых клиентов —
    /// `CompletionPrompt::Multimodal` (поле `multimodal_data` у объекта
    /// prompt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data: Option<serde_json::Value>,
}

impl SamplingParams {
    /// Пустой builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Удобные chain-setters для популярных полей.
    pub fn with_temperature(mut self, v: f32) -> Self {
        self.temperature = Some(v);
        self
    }
    pub fn with_top_k(mut self, v: i32) -> Self {
        self.top_k = Some(v);
        self
    }
    pub fn with_top_p(mut self, v: f32) -> Self {
        self.top_p = Some(v);
        self
    }
    pub fn with_min_p(mut self, v: f32) -> Self {
        self.min_p = Some(v);
        self
    }
    pub fn with_max_tokens(mut self, v: i32) -> Self {
        self.max_tokens = Some(v);
        self
    }
    pub fn with_n_predict(mut self, v: i32) -> Self {
        self.n_predict = Some(v);
        self
    }
    pub fn with_seed(mut self, v: i64) -> Self {
        self.seed = Some(v);
        self
    }
    pub fn with_stop<I, S>(mut self, stops: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.stop = Some(stops.into_iter().map(Into::into).collect());
        self
    }
    pub fn with_grammar(mut self, v: impl Into<String>) -> Self {
        self.grammar = Some(v.into());
        self
    }
    pub fn with_json_schema(mut self, v: serde_json::Value) -> Self {
        self.json_schema = Some(v);
        self
    }
    pub fn with_cache_prompt(mut self, v: bool) -> Self {
        self.cache_prompt = Some(v);
        self
    }
    pub fn with_id_slot(mut self, v: i32) -> Self {
        self.id_slot = Some(v);
        self
    }
    pub fn with_samplers(mut self, order: SamplerOrder) -> Self {
        self.samplers = Some(order);
        self
    }
    pub fn with_logit_bias(mut self, bias: LogitBias) -> Self {
        self.logit_bias = Some(bias);
        self
    }
    pub fn with_lora<I: IntoIterator<Item = LoraPatch>>(mut self, patches: I) -> Self {
        self.lora = Some(patches.into_iter().collect());
        self
    }
}

/// Порядок сэмплеров. Сервер принимает массив строк; клиент хранит
/// нормализованную строковую форму, но готов есть и сырой массив.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SamplerOrder(pub Vec<String>);

impl SamplerOrder {
    pub fn new<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(iter.into_iter().map(Into::into).collect())
    }

    /// Сэмплеры по умолчанию llama-server (на момент master).
    pub fn default_order() -> Self {
        Self::new([
            "penalties",
            "dry",
            "top_k",
            "typ_p",
            "top_p",
            "min_p",
            "xtc",
            "temperature",
        ])
    }
}

/// Per-request LoRA-патч: `[{"id": int, "scale": float}]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraPatch {
    pub id: i32,
    pub scale: f32,
}

impl LoraPatch {
    pub fn new(id: i32, scale: f32) -> Self {
        Self { id, scale }
    }
}

/// Bias токенов: массив пар или OpenAI-объект.
///
/// Сервер принимает:
///
/// * `[[15043, 1.0], [15043, -0.5], [15043, false], ["Hello", -0.5]]` — массив пар;
/// * `{"15043": 1.0, "Hello": -0.5}` — OpenAI-объект.
///
/// Клиент хранит обе формы: используйте `LogitBias::pairs` или
/// `LogitBias::openai`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogitBias {
    Pairs(Vec<LogitBiasEntry>),
    OpenAi(std::collections::HashMap<String, f32>),
}

/// Один элемент массива `logit_bias`.
///
/// `token` — либо id, либо строка. `bias` — либо float, либо `false`
/// (полный бан). Мы храним это как варианты, но для сериализации
/// используем плоский массив из двух элементов (совместимо с сервером).
#[derive(Debug, Clone)]
pub struct LogitBiasEntry {
    pub token: LogitBiasToken,
    pub bias: LogitBiasValue,
}

#[derive(Debug, Clone)]
pub enum LogitBiasToken {
    Id(i32),
    Text(String),
}

#[derive(Debug, Clone, Copy)]
pub enum LogitBiasValue {
    Float(f32),
    Ban,
}

impl Serialize for LogitBiasEntry {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeTuple;
        let mut t = s.serialize_tuple(2)?;
        match &self.token {
            LogitBiasToken::Id(i) => t.serialize_element(i)?,
            LogitBiasToken::Text(s) => t.serialize_element(s)?,
        }
        match &self.bias {
            LogitBiasValue::Float(f) => t.serialize_element(f)?,
            LogitBiasValue::Ban => t.serialize_element(&false)?,
        }
        t.end()
    }
}

impl<'de> Deserialize<'de> for LogitBiasEntry {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let arr: Vec<serde_json::Value> = Vec::deserialize(d)?;
        if arr.len() != 2 {
            return Err(serde::de::Error::custom(
                "logit_bias entry must be a 2-element array [token, bias]",
            ));
        }
        let token = match &arr[0] {
            serde_json::Value::Number(n) => LogitBiasToken::Id(n.as_i64().unwrap_or(0) as i32),
            serde_json::Value::String(s) => LogitBiasToken::Text(s.clone()),
            other => {
                return Err(serde::de::Error::custom(format!(
                    "logit_bias token must be int or string, got {}",
                    other
                )))
            }
        };
        let bias = match &arr[1] {
            serde_json::Value::Number(n) => LogitBiasValue::Float(n.as_f64().unwrap_or(0.0) as f32),
            serde_json::Value::Bool(false) => LogitBiasValue::Ban,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "logit_bias value must be float or `false`, got {}",
                    other
                )))
            }
        };
        Ok(LogitBiasEntry { token, bias })
    }
}

impl LogitBias {
    pub fn pairs<I: IntoIterator<Item = LogitBiasEntry>>(iter: I) -> Self {
        Self::Pairs(iter.into_iter().collect())
    }

    pub fn openai<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = (S, f32)>,
        S: Into<String>,
    {
        Self::OpenAi(iter.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Тесты сериализации
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_params_produce_empty_object() {
        let p = SamplingParams::new();
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "{}");
    }

    #[test]
    fn only_set_fields_are_serialized() {
        let p = SamplingParams::new()
            .with_temperature(0.7)
            .with_top_k(50)
            .with_stop(["<|end|>", "###"]);
        let s = serde_json::to_value(&p).unwrap();
        // f32 → JSON number — сравниваем с допуском, как и положено
        // (0.7 не представим точно в float).
        let temp = s["temperature"].as_f64().unwrap();
        assert!((temp - 0.7_f64).abs() < 1e-6);
        assert_eq!(s["top_k"], 50);
        assert_eq!(s["stop"], serde_json::json!(["<|end|>", "###"]));
        // Прочие поля не появились.
        assert!(s.get("seed").is_none());
        assert!(s.get("grammar").is_none());
    }

    #[test]
    fn logit_bias_pairs_roundtrip() {
        let b = LogitBias::pairs([
            LogitBiasEntry {
                token: LogitBiasToken::Id(15043),
                bias: LogitBiasValue::Float(1.0),
            },
            LogitBiasEntry {
                token: LogitBiasToken::Text("Hello".into()),
                bias: LogitBiasValue::Ban,
            },
        ]);
        let s = serde_json::to_value(&b).unwrap();
        assert_eq!(s, serde_json::json!([[15043, 1.0], ["Hello", false]]));

        let parsed: LogitBias = serde_json::from_value(s).unwrap();
        match parsed {
            LogitBias::Pairs(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected Pairs"),
        }
    }

    #[test]
    fn sampler_order_default_matches_docs() {
        let s = SamplerOrder::default_order();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                "penalties", "dry", "top_k", "typ_p", "top_p", "min_p", "xtc", "temperature"
            ])
        );
    }
}
