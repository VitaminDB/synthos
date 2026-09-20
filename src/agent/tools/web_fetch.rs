//! Общий HTTP-fetcher: bytes → text/html/markdown.
//!
//! Используется и tool'ом `web_read` (форматирует результат как envelope
//! для LLM), и kb-ingest pipeline'ом (нужен plain markdown для индексации).
//! Вынесено отсюда, чтобы не дублировать reqwest-конфиг, парсинг
//! content-type и стрим тела с потолком.
//!
//! Дизайн:
//! - `fetch_url(...)` — низкоуровневый: возвращает [`FetchedDoc`] с
//!   плоским телом (md/text), статусом, content-type, флагом truncated.
//! - Никаких envelope-форматов, никакой LLM-специфики — это уровень выше.

use std::time::Duration;

use futures_util::StreamExt;

/// Лимит сырого тела HTTP-ответа. HTML обычно сжимается в Markdown в 3–5×.
/// Совпадает с `executor::MAX_FETCH_BYTES` — общий потолок на одну загрузку.
pub const MAX_FETCH_BYTES: usize = 64 * 1024;

/// Таймаут одного fetch'а: connect + чтение тела.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// User-Agent для исходящего HTTP-запроса. Многие сайты 403'ят дефолтный
/// `reqwest/x.y` — даём осмысленный self-identifier.
pub const USER_AGENT: &str = concat!("syngui-synthos/", env!("SYNTHOS_VERSION"));

/// Категории content-type, которые мы умеем обрабатывать.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Html,
    PlainText,
    Markdown,
    Unsupported,
}

pub fn classify_mime(mime: &str) -> BodyKind {
    match mime {
        "text/html" | "application/xhtml+xml" => BodyKind::Html,
        "text/plain" => BodyKind::PlainText,
        "text/markdown" | "text/x-markdown" => BodyKind::Markdown,
        _ => BodyKind::Unsupported,
    }
}

/// Структура результата fetch'а. body всегда trimmed до MAX_FETCH_BYTES;
/// `truncated=true` означает, что хвост обрезан.
#[derive(Debug, Clone)]
pub struct FetchedDoc {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub kind: BodyKind,
    /// Plain text. Для HTML — уже сконвертированный Markdown через `htmd`;
    /// для PlainText/Markdown — как есть. Для Unsupported — пустая строка.
    pub body: String,
    pub truncated: bool,
}

/// Ошибки fetch'а. В отличие от tool-варианта (где сетевые ошибки
/// возвращаются как `Ok(envelope с error-секцией)`), для kb-ingest
/// удобнее иметь `Result<…, FetchError>` — пропустим источник на уровне
/// pipeline.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("ошибка инициализации HTTP-клиента: {0}")]
    Client(String),
    #[error("сетевая ошибка: {0}")]
    Network(String),
    #[error("ошибка чтения тела: {0}")]
    Read(String),
    #[error("неподдерживаемый content-type: {0}")]
    Unsupported(String),
    #[error("html→markdown failed: {0}")]
    HtmlConvert(String),
}

/// Загружает URL и возвращает [`FetchedDoc`] с телом-Markdown.
pub async fn fetch_url(url: &str) -> Result<FetchedDoc, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FetchError::Client(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let final_url = response.url().to_string();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("(unknown)")
        .to_string();
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let kind = classify_mime(&mime);
    if matches!(kind, BodyKind::Unsupported) {
        return Err(FetchError::Unsupported(content_type));
    }

    // Стрим body с потолком.
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| FetchError::Read(e.to_string()))?;
        if buf.len() + bytes.len() > MAX_FETCH_BYTES {
            let take = MAX_FETCH_BYTES.saturating_sub(buf.len());
            buf.extend_from_slice(&bytes[..take]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&bytes);
    }
    let raw = String::from_utf8_lossy(&buf).into_owned();

    let body = match kind {
        BodyKind::Html => htmd::convert(&raw).map_err(|e| FetchError::HtmlConvert(e.to_string()))?,
        BodyKind::PlainText | BodyKind::Markdown => raw,
        BodyKind::Unsupported => unreachable!("отфильтровано выше"),
    };

    Ok(FetchedDoc {
        url: url.to_string(),
        final_url,
        status,
        content_type,
        kind,
        body,
        truncated,
    })
}

/// Извлекает первый h1 из markdown как title, либо пустую строку.
pub fn extract_title(markdown: &str) -> Option<String> {
    for line in markdown.lines().take(50) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_dispatches() {
        assert_eq!(classify_mime("text/html"), BodyKind::Html);
        assert_eq!(classify_mime("application/xhtml+xml"), BodyKind::Html);
        assert_eq!(classify_mime("text/plain"), BodyKind::PlainText);
        assert_eq!(classify_mime("text/markdown"), BodyKind::Markdown);
        assert_eq!(classify_mime("application/pdf"), BodyKind::Unsupported);
    }

    #[test]
    fn extract_h1_title() {
        assert_eq!(extract_title("# Title\nbody"), Some("Title".to_string()));
        assert_eq!(extract_title("body\n# Sub later"), Some("Sub later".to_string()));
        assert_eq!(extract_title("no title here"), None);
        assert_eq!(extract_title(""), None);
    }
}
