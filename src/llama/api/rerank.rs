//! `POST /rerank`, `POST /reranking`, `POST /v1/rerank`, `POST /v1/reranking`.

use serde::{Deserialize, Serialize};

use super::common::Usage;
use super::error::LlamaError;
use super::LlamaClient;

#[derive(Debug, Clone, Serialize)]
pub struct RerankRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub query: String,
    pub documents: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_n: Option<i32>,
}

impl RerankRequest {
    pub fn new(query: impl Into<String>, documents: Vec<String>) -> Self {
        Self {
            model: None,
            query: query.into(),
            documents,
            top_n: None,
        }
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = Some(m.into());
        self
    }

    pub fn with_top_n(mut self, n: i32) -> Self {
        self.top_n = Some(n);
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RerankResponse {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,

    pub results: Vec<RerankResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RerankResult {
    pub index: i32,
    pub relevance_score: f32,
}

impl LlamaClient {
    /// `POST /v1/rerank` (рекомендуемый endpoint — максимально совместим).
    pub async fn rerank(&self, req: &RerankRequest) -> Result<RerankResponse, LlamaError> {
        self.post_json("/v1/rerank", req).await
    }

    /// `POST /rerank` (native путь, синоним).
    pub async fn rerank_native(&self, req: &RerankRequest) -> Result<RerankResponse, LlamaError> {
        self.post_json("/rerank", req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_serializes() {
        let r = RerankRequest::new(
            "panda",
            vec!["hi".into(), "bear".into(), "giant panda".into()],
        )
        .with_model("bge-reranker")
        .with_top_n(2);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["query"], "panda");
        assert_eq!(v["documents"].as_array().unwrap().len(), 3);
        assert_eq!(v["top_n"], 2);
        assert_eq!(v["model"], "bge-reranker");
    }

    #[test]
    fn response_parses() {
        let raw = json!({
            "model": "bge",
            "object": "list",
            "usage": {"prompt_tokens": 10, "total_tokens": 10},
            "results": [
                {"index": 2, "relevance_score": 0.9},
                {"index": 1, "relevance_score": 0.4}
            ]
        });
        let r: RerankResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.results.len(), 2);
        assert!((r.results[0].relevance_score - 0.9).abs() < 1e-6);
    }
}
