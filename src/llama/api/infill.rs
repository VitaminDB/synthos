//! `POST /infill` — code Fill-In-the-Middle.
//!
//! Принимает все поля `/completion` (через `SamplingParams`) + infill-специфичные.

use futures_util::Stream;
use serde::{Deserialize, Serialize};

use super::completion::{CompletionFinal, CompletionStreamChunk};
use super::error::LlamaError;
use super::sampling::SamplingParams;
use super::sse::SseStream;
use super::LlamaClient;

#[derive(Debug, Clone, Serialize)]
pub struct InfillRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_extra: Option<Vec<InfillExtra>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    #[serde(flatten)]
    pub sampling: SamplingParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfillExtra {
    pub filename: String,
    pub text: String,
}

impl InfillRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prefix(mut self, s: impl Into<String>) -> Self {
        self.input_prefix = Some(s.into());
        self
    }
    pub fn with_suffix(mut self, s: impl Into<String>) -> Self {
        self.input_suffix = Some(s.into());
        self
    }
    pub fn with_prompt(mut self, s: impl Into<String>) -> Self {
        self.prompt = Some(s.into());
        self
    }
    pub fn with_extra<I: IntoIterator<Item = InfillExtra>>(mut self, extra: I) -> Self {
        self.input_extra = Some(extra.into_iter().collect());
        self
    }
    pub fn with_sampling(mut self, s: SamplingParams) -> Self {
        self.sampling = s;
        self
    }
    pub fn with_stream(mut self, v: bool) -> Self {
        self.stream = Some(v);
        self
    }
}

impl Default for InfillRequest {
    fn default() -> Self {
        Self {
            input_prefix: None,
            input_suffix: None,
            input_extra: None,
            prompt: None,
            stream: None,
            sampling: SamplingParams::default(),
        }
    }
}

impl LlamaClient {
    /// `POST /infill` (non-stream).
    pub async fn infill(&self, req: &InfillRequest) -> Result<CompletionFinal, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(false);
        self.post_json("/infill", &body).await
    }

    /// `POST /infill` (stream).
    pub async fn infill_stream(
        &self,
        req: &InfillRequest,
    ) -> Result<impl Stream<Item = Result<CompletionStreamChunk, LlamaError>>, LlamaError> {
        let mut body = req.clone();
        body.stream = Some(true);
        let sse = self.post_sse("/infill", &body).await?;
        Ok(decode_infill_stream(sse))
    }
}

fn decode_infill_stream(
    sse: SseStream,
) -> impl Stream<Item = Result<CompletionStreamChunk, LlamaError>> {
    use futures_util::StreamExt;
    sse.filter_map(|ev| async move {
        match ev {
            Ok(e) if e.is_done() => None,
            Ok(e) if e.is_error() => Some(Err(LlamaError::Sse(e.data))),
            Ok(e) => Some(e.json::<CompletionStreamChunk>()),
            Err(e) => Some(Err(e)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_serializes_minimal() {
        let r = InfillRequest::new()
            .with_prefix("fn add(a:i32,b:i32)->i32{")
            .with_suffix("}")
            .with_sampling(SamplingParams::new().with_max_tokens(16));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["input_prefix"], "fn add(a:i32,b:i32)->i32{");
        assert_eq!(v["input_suffix"], "}");
        assert_eq!(v["max_tokens"], 16);
        assert!(v.get("stream").is_none());
        assert!(v.get("input_extra").is_none());
    }

    #[test]
    fn request_with_extra_and_prompt() {
        let r = InfillRequest::new().with_prefix("a").with_suffix("b").with_prompt("c")
            .with_extra([InfillExtra {
                filename: "lib.rs".into(),
                text: "pub fn x(){}".into(),
            }]);
        let v = serde_json::to_value(&r).unwrap();
        let extra = v["input_extra"].as_array().unwrap();
        assert_eq!(extra[0]["filename"], "lib.rs");
        assert_eq!(v["prompt"], "c");
    }

    #[test]
    fn default_request_is_empty() {
        let v = serde_json::to_value(&InfillRequest::default()).unwrap();
        // Без sampling и infill-полей — чистый {}.
        assert_eq!(v, json!({}));
    }
}
