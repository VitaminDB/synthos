//! `POST /tokenize`, `POST /detokenize`, `POST /apply-template`.

use serde::{Deserialize, Serialize};

use super::common::ChatMessage;
use super::error::LlamaError;
use super::LlamaClient;

#[derive(Debug, Clone, Serialize)]
pub struct TokenizeRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_special: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_special: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_pieces: Option<bool>,
}

impl TokenizeRequest {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            add_special: None,
            parse_special: None,
            with_pieces: None,
        }
    }
    pub fn with_add_special(mut self, v: bool) -> Self {
        self.add_special = Some(v);
        self
    }
    pub fn with_parse_special(mut self, v: bool) -> Self {
        self.parse_special = Some(v);
        self
    }
    pub fn with_pieces(mut self, v: bool) -> Self {
        self.with_pieces = Some(v);
        self
    }
}

/// Ответ: либо массив id, либо массив объектов `{id, piece}`.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenizeResponse {
    pub tokens: Vec<TokenEntry>,
}

/// Один элемент `tokens`. Десериализация сама определяет форму.
#[derive(Debug, Clone)]
pub enum TokenEntry {
    Id(i32),
    WithPiece { id: i32, piece: TokenPiece },
}

impl TokenEntry {
    pub fn id(&self) -> i32 {
        match self {
            TokenEntry::Id(i) => *i,
            TokenEntry::WithPiece { id, .. } => *id,
        }
    }
}

/// `piece`: либо валидный UTF-8 (`String`), либо массив байт.
#[derive(Debug, Clone)]
pub enum TokenPiece {
    Text(String),
    Bytes(Vec<u8>),
}

impl<'de> Deserialize<'de> for TokenEntry {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::Number(n) => Ok(TokenEntry::Id(n.as_i64().unwrap_or(0) as i32)),
            serde_json::Value::Object(map) => {
                let id = map
                    .get("id")
                    .and_then(|x| x.as_i64())
                    .ok_or_else(|| serde::de::Error::custom("missing token id"))?
                    as i32;
                let piece = map
                    .get("piece")
                    .cloned()
                    .ok_or_else(|| serde::de::Error::custom("missing token piece"))?;
                let piece = match piece {
                    serde_json::Value::String(s) => TokenPiece::Text(s),
                    serde_json::Value::Array(arr) => {
                        let bytes: Vec<u8> = arr
                            .into_iter()
                            .filter_map(|n| n.as_u64().map(|x| x as u8))
                            .collect();
                        TokenPiece::Bytes(bytes)
                    }
                    _ => {
                        return Err(serde::de::Error::custom(
                            "piece must be string or byte array",
                        ))
                    }
                };
                Ok(TokenEntry::WithPiece { id, piece })
            }
            other => Err(serde::de::Error::custom(format!(
                "unexpected token shape: {}",
                other
            ))),
        }
    }
}

impl Serialize for TokenEntry {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        match self {
            TokenEntry::Id(i) => s.serialize_i32(*i),
            TokenEntry::WithPiece { id, piece } => {
                let mut st = s.serialize_struct("TokenWithPiece", 2)?;
                st.serialize_field("id", id)?;
                match piece {
                    TokenPiece::Text(t) => st.serialize_field("piece", t)?,
                    TokenPiece::Bytes(b) => st.serialize_field("piece", b)?,
                }
                st.end()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DetokenizeRequest {
    pub tokens: Vec<i32>,
}

impl DetokenizeRequest {
    pub fn new<I: IntoIterator<Item = i32>>(tokens: I) -> Self {
        Self {
            tokens: tokens.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetokenizeResponse {
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyTemplateRequest {
    pub messages: Vec<ChatMessage>,
}

impl ApplyTemplateRequest {
    pub fn new<I: IntoIterator<Item = ChatMessage>>(messages: I) -> Self {
        Self {
            messages: messages.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplyTemplateResponse {
    pub prompt: String,
}

impl LlamaClient {
    /// `POST /tokenize`.
    pub async fn tokenize(&self, req: &TokenizeRequest) -> Result<TokenizeResponse, LlamaError> {
        self.post_json("/tokenize", req).await
    }

    /// Удобный shortcut: токенизировать строку без дополнительных опций.
    pub async fn tokenize_text(&self, text: impl Into<String>) -> Result<Vec<i32>, LlamaError> {
        let resp = self.tokenize(&TokenizeRequest::new(text)).await?;
        Ok(resp.tokens.iter().map(TokenEntry::id).collect())
    }

    /// `POST /detokenize`.
    pub async fn detokenize(
        &self,
        req: &DetokenizeRequest,
    ) -> Result<DetokenizeResponse, LlamaError> {
        self.post_json("/detokenize", req).await
    }

    /// Удобный shortcut: токены → строка.
    pub async fn detokenize_tokens(&self, tokens: &[i32]) -> Result<String, LlamaError> {
        let resp = self.detokenize(&DetokenizeRequest::new(tokens.iter().copied())).await?;
        Ok(resp.content)
    }

    /// `POST /apply-template`.
    pub async fn apply_template(
        &self,
        req: &ApplyTemplateRequest,
    ) -> Result<ApplyTemplateResponse, LlamaError> {
        self.post_json("/apply-template", req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tokenize_request_serialization_omits_nulls() {
        let r = TokenizeRequest::new("hi").with_add_special(true);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["content"], "hi");
        assert_eq!(v["add_special"], true);
        assert!(v.get("with_pieces").is_none());
        assert!(v.get("parse_special").is_none());
    }

    #[test]
    fn tokenize_response_with_id_only() {
        let r: TokenizeResponse = serde_json::from_value(json!({"tokens": [123, 456, 789]})).unwrap();
        let ids: Vec<i32> = r.tokens.iter().map(TokenEntry::id).collect();
        assert_eq!(ids, vec![123, 456, 789]);
    }

    #[test]
    fn tokenize_response_with_pieces_text() {
        let r: TokenizeResponse = serde_json::from_value(json!({
            "tokens": [
                {"id": 123, "piece": "Hello"},
                {"id": 789, "piece": "!"}
            ]
        })).unwrap();
        assert_eq!(r.tokens.len(), 2);
        assert!(matches!(r.tokens[0], TokenEntry::WithPiece { id: 123, piece: TokenPiece::Text(ref s) } if s == "Hello"));
    }

    #[test]
    fn tokenize_response_with_bytes_piece() {
        let r: TokenizeResponse = serde_json::from_value(json!({
            "tokens": [{"id": 198, "piece": [195]}, {"id": 164, "piece": [161]}]
        })).unwrap();
        match &r.tokens[0] {
            TokenEntry::WithPiece { piece: TokenPiece::Bytes(b), .. } => assert_eq!(b, &vec![195u8]),
            _ => panic!("expected byte piece"),
        }
    }

    #[test]
    fn detokenize_roundtrip() {
        let req = DetokenizeRequest::new([1, 2, 3]);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["tokens"], json!([1, 2, 3]));
    }

    #[test]
    fn apply_template_request_serialization() {
        let r = ApplyTemplateRequest::new([
            ChatMessage::system("sys"),
            ChatMessage::user("hi"),
        ]);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["content"], "hi");
    }
}
