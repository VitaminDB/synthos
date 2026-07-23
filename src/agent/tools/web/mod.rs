//! Tool `web` — единый интерфейс для поиска и чтения веб-страниц.
//!
//! Архитектура:
//! - [`action`]   — типы аргументов tool-call'а и их парсинг
//!   (`{action: "search"|"read", query?, url?, max_results?, lang?}`).
//! - [`http`]     — общий low-level fetch (reqwest, единый UA, лимит 1 MB).
//! - [`search`]   — DuckDuckGo html-endpoint + scraper-парсер SERP.
//! - [`read`]     — fetch HTML → Readability (`dom_smoothie`) → htmd → markdown.
//! - [`envelope`] — форматирование результата для LLM (plain-секции
//!   `WEB_SEARCH … --- results ---`, `GET … --- markdown ---`,
//!   `--- error ---`).
//!
//! Контракт ошибок: `run` возвращает [`ToolError`] **только** на невалидные
//! аргументы (BadArgs / MissingField). Все runtime-ошибки (DNS-fail, 4xx/5xx,
//! пустые results) уходят в LLM как `Ok(envelope с --- error ---)` —
//! модели полезнее видеть структурную ошибку как данные.
//!
//! JS-fallback (chromiumoxide / WebDriver) намеренно НЕ реализован. По
//! результатам тестов прошлого Servo-движка, 90%+ запросов LLM покрываются
//! чистым reqwest+Readability (новости, доки, Wikipedia, GitHub, блоги).
//! Когда понадобится — подключим за отдельной фичей `web-cdp`.

pub mod action;
pub mod envelope;
pub mod http;
pub mod read;
pub mod search;

use crate::agent::tools::executor::ToolError;

use self::action::{parse_args, WebArgs};

/// Async-entrypoint, который дёргает `executor::execute` для `KEY_WEB`.
///
/// Возвращает `Ok(envelope_text)` для успешных и осмысленно-неуспешных
/// сценариев (LLM получает результат как plain-текст). Возвращает
/// `Err(ToolError::*)` только на невалидные аргументы.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let parsed = parse_args(args_json)?;
    match parsed {
        WebArgs::Search {
            query,
            max_results,
            lang,
        } => match search::search(&query, &lang, max_results).await {
            Ok(hits) => Ok(envelope::format_search_envelope(&query, &lang, &hits)),
            // SearchError::{Network,Challenge,Empty} → форматируется через
            // Display импл; envelope печатает осмысленное «почему».
            Err(e) => Ok(envelope::format_search_error(&query, &lang, &e.to_string())),
        },
        WebArgs::Read { url } => match read::read(&url).await {
            Ok(doc) => Ok(envelope::format_read_envelope(&doc)),
            Err(e) => Ok(envelope::format_read_error(&url, &e.to_string())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn run_bad_json_returns_err() {
        let err = run("not json").await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_missing_action_returns_err() {
        let err = run(r#"{"query": "x"}"#).await.unwrap_err();
        assert!(matches!(err, ToolError::MissingField("action")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_search_missing_query() {
        let err = run(r#"{"action": "search"}"#).await.unwrap_err();
        assert!(matches!(err, ToolError::MissingField("query")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_read_missing_url() {
        let err = run(r#"{"action": "read"}"#).await.unwrap_err();
        assert!(matches!(err, ToolError::MissingField("url")));
    }

    /// Невалидный URL → должен вернуть Ok(envelope с --- error ---),
    /// а не ToolError. Для `https://invalid.invalid` reqwest ругнётся
    /// на DNS — это «runtime»-ошибка, попадает в envelope.
    #[tokio::test(flavor = "current_thread")]
    async fn run_read_dns_fail_is_envelope_error() {
        let out = run(r#"{"action": "read", "url": "https://invalid.invalid/"}"#)
            .await
            .expect("должен вернуть Ok с envelope, не ToolError");
        assert!(out.contains("GET https://invalid.invalid/"), "{out}");
        assert!(out.contains("--- error ---"), "{out}");
    }
}
