//! Низкоуровневый HTTP-fetcher для tool'а `web` (search + read).
//!
//! Единственная зависимость — `reqwest` (уже подключён в проекте для
//! llama-API). Намеренно НЕ переиспользуем `chat::tools::web_fetch` —
//! тот заточен под kb-ingest pipeline (FetchedDoc с готовым markdown,
//! строгая фильтрация по mime, лимит 64 KB). Здесь нам нужен сырой body
//! с увеличенным лимитом и обязательной декодировкой content-type.
//!
//! API:
//! - [`fetch_raw_with`] / [`fetch_html_text_with`] — общий entrypoint
//!   с настраиваемыми headers через [`FetchOpts`]. Используется в search
//!   (нужен Accept-Language под язык SERP'а).
//! - [`fetch_raw`] / [`fetch_html_text`] — тонкие обёртки с дефолтными
//!   опциями (передаётся только Referer). Используется в read.rs.

use std::time::Duration;

use futures_util::StreamExt;
use thiserror::Error;

/// Лимит сырого тела HTTP-ответа. 1 MB — компромисс: Wikipedia/GitHub
/// README укладываются с запасом, после Readability+htmd итоговый
/// markdown — 30-100 KB. Меньшие лимиты (64 KB) рвали статьи на середине.
pub const MAX_FETCH_BYTES: usize = 1024 * 1024;

/// Таймаут одного запроса: connect + чтение тела.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// Честный desktop-Chrome UA. Дефолтный `reqwest/x.y` ловит 403 на части
/// сайтов; собственный маркер `syngui-synthos/<ver>` в прошлой итерации
/// триггерил капчу у поисковиков. Этот UA общий для search и read.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Сырой результат fetch'а. Без классификации mime / без html→markdown —
/// эти решения принимает caller (search использует только body как HTML;
/// read смотрит на content_type и применяет Readability/htmd).
#[derive(Debug, Clone)]
pub struct RawDoc {
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP-клиент: {0}")]
    Client(String),
    #[error("сетевая ошибка: {0}")]
    Network(String),
    #[error("чтение тела: {0}")]
    Read(String),
}

/// Настройки headers для одного fetch'а. Все поля опциональные — если
/// `None`, соответствующий header просто не ставится.
///
/// Зачем отдельная структура: SERP-запрос (DDG) требует кроме `Referer`
/// ещё `Accept` + `Accept-Language` под язык, иначе DDG чаще роняет в
/// challenge / отдаёт пустую страницу. Read-сайтам это не нужно — у
/// них дефолтный `User-Agent` достаточен.
#[derive(Debug, Default, Clone, Copy)]
pub struct FetchOpts<'a> {
    pub referer: Option<&'a str>,
    pub accept_language: Option<&'a str>,
}

/// Тонкая обёртка для read.rs — Referer и больше ничего.
pub async fn fetch_raw(url: &str, referer: Option<&str>) -> Result<RawDoc, FetchError> {
    fetch_raw_with(
        url,
        FetchOpts {
            referer,
            accept_language: None,
        },
    )
    .await
}

pub async fn fetch_raw_with(url: &str, opts: FetchOpts<'_>) -> Result<RawDoc, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| FetchError::Client(e.to_string()))?;

    let mut req = client.get(url);
    if let Some(r) = opts.referer {
        // DDG html-endpoint без Referer'а отдаёт 202 и редиректит на
        // главную (нет SERP). Для DDG используется Referer того же
        // хоста ("https://html.duckduckgo.com/") — как делает searxng.
        // Для read-сайтов Referer не вреден.
        req = req.header(reqwest::header::REFERER, r);
    }
    if let Some(al) = opts.accept_language {
        req = req.header(reqwest::header::ACCEPT_LANGUAGE, al);
        // Accept ставим только вместе с Accept-Language — это маркер,
        // что мы хотим максимально «браузерный» запрос. Read.rs
        // (без accept_language) оставляем дефолтный Accept reqwest'а.
        req = req.header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        );
    }
    let response = req
        .send()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let final_url = response.url().to_string();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

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

    Ok(RawDoc {
        final_url,
        status,
        content_type,
        body: buf,
        truncated,
    })
}

/// Удобная обёртка: возвращает body как UTF-8-строку (lossy). DDG-SERP
/// и большинство сайтов отдаются в UTF-8, но lossy-декодинг защищает
/// от мусора в edge-кейсах.
pub async fn fetch_html_text(url: &str, referer: Option<&str>) -> Result<String, FetchError> {
    fetch_html_text_with(
        url,
        FetchOpts {
            referer,
            accept_language: None,
        },
    )
    .await
}

pub async fn fetch_html_text_with(
    url: &str,
    opts: FetchOpts<'_>,
) -> Result<String, FetchError> {
    let doc = fetch_raw_with(url, opts).await?;
    Ok(String::from_utf8_lossy(&doc.body).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_is_desktop_chrome() {
        // Маркер `synthos` в UA триггерил капчу — проверяем, что его нет.
        assert!(!USER_AGENT.to_lowercase().contains("synthos"));
        assert!(USER_AGENT.contains("Mozilla/5.0"));
        assert!(USER_AGENT.contains("Chrome"));
    }

    #[test]
    fn max_fetch_bytes_is_1mb() {
        // Защита от случайного снижения лимита: 1 MB — минимум для
        // адекватного покрытия article-сайтов.
        assert_eq!(MAX_FETCH_BYTES, 1024 * 1024);
    }

    #[test]
    fn fetch_opts_default_is_all_none() {
        let opts = FetchOpts::default();
        assert!(opts.referer.is_none());
        assert!(opts.accept_language.is_none());
    }
}
