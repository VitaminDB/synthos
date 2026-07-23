//! Форматирование результата tool'а `web` для отправки в LLM.
//!
//! Формат — секционный plain-текст (как у `bash`/`web_read` ранее):
//! - первая строка: `WEB_SEARCH …` или `GET <url>` — каноничный hint
//!   для модели, чтобы она поняла «что это»;
//! - метаданные шапки (`final-url`, `status`, `content-type`);
//! - тело в секции `--- markdown ---` / `--- results ---` / `--- error ---`.
//!
//! Plain-текст вместо JSON: переводы строк остаются реальными `\n` — UI
//! tool-bubble отрисует читабельно, и LLM-ы стабильнее парсят явные
//! заголовки секций, чем escape-нутый JSON.

/// Один результат поиска (для `format_search_envelope`).
#[derive(Debug, Clone)]
pub struct SerpHit {
    pub rank: usize,
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Метаданные прочитанной страницы (для `format_read_envelope`).
#[derive(Debug, Clone)]
pub struct ReadDoc {
    pub source_url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub markdown: String,
    /// `true` — readability не справился, в `markdown` лежит конвертация
    /// сырого `<body>` (или весь HTML). LLM получит подсказку, чтобы не
    /// доверять структуре article-метаданных.
    pub readability_fallback: bool,
    /// `true` — тело было обрезано на лимите чтения (см. `read::MAX_FETCH_BYTES`).
    pub truncated: bool,
}

pub fn format_search_envelope(query: &str, lang: &str, hits: &[SerpHit]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "WEB_SEARCH query=\"{}\" engine=duckduckgo lang={}\n",
        escape_quotes(query),
        lang
    ));
    out.push_str(&format!("results-count: {}\n", hits.len()));

    if hits.is_empty() {
        out.push_str("--- error ---\n");
        out.push_str(
            "Поисковик не вернул результатов. Возможные причины: парсер \
             SERP устарел, запрос слишком узкий, временная ошибка DDG. \
             Попробуй переформулировать запрос.\n",
        );
        return out;
    }

    out.push_str("--- results ---\n");
    for h in hits {
        out.push_str(&format!("{}. {}\n", h.rank, h.title));
        out.push_str(&format!("   {}\n", h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
        out.push('\n');
    }
    out
}

pub fn format_read_envelope(doc: &ReadDoc) -> String {
    let mut out = String::new();
    out.push_str(&format!("GET {}\n", doc.source_url));
    if doc.final_url != doc.source_url {
        out.push_str(&format!("final-url: {}\n", doc.final_url));
    }
    out.push_str(&format!("status: {}\n", doc.status));
    if !doc.content_type.is_empty() {
        out.push_str(&format!("content-type: {}\n", doc.content_type));
    }
    if doc.readability_fallback {
        out.push_str("readability: fallback (raw body)\n");
    }
    out.push_str("--- markdown ---\n");
    out.push_str(doc.markdown.trim_end());
    out.push('\n');
    if doc.truncated {
        out.push_str("(body truncated by read-size limit)\n");
    }
    out
}

/// Envelope для read-ошибок (DNS-fail, connect-timeout, неподдерживаемый
/// content-type и т.п.). Когда HTTP-ответа ещё нет — нет смысла показывать
/// `status:`/`content-type:`, оставляем только `GET <url>` + сообщение.
pub fn format_read_error(url: &str, msg: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("GET {url}\n"));
    out.push_str("--- error ---\n");
    out.push_str(msg.trim_end());
    out.push('\n');
    out
}

pub fn format_search_error(query: &str, lang: &str, msg: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "WEB_SEARCH query=\"{}\" engine=duckduckgo lang={}\n",
        escape_quotes(query),
        lang
    ));
    out.push_str("--- error ---\n");
    out.push_str(msg.trim_end());
    out.push('\n');
    out
}

/// Экранирование внутри `query="..."` шапки — только `\` и `"`.
/// Полноценный JSON-escape тут не нужен: остальные символы (включая
/// переводы строк, которые в query всё равно невалидны) проходят как есть.
fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_envelope_includes_results() {
        let hits = vec![
            SerpHit {
                rank: 1,
                title: "T1".into(),
                url: "https://e.x/1".into(),
                snippet: "snippet 1".into(),
            },
            SerpHit {
                rank: 2,
                title: "T2".into(),
                url: "https://e.x/2".into(),
                snippet: String::new(),
            },
        ];
        let out = format_search_envelope("hi", "ru", &hits);
        assert!(out.starts_with("WEB_SEARCH query=\"hi\" engine=duckduckgo lang=ru\n"));
        assert!(out.contains("results-count: 2"));
        assert!(out.contains("--- results ---"));
        assert!(out.contains("1. T1"));
        assert!(out.contains("https://e.x/1"));
        assert!(out.contains("snippet 1"));
        assert!(out.contains("2. T2"));
        // У второго — нет snippet → нет лишней строки.
        assert!(!out.contains("snippet 2"));
    }

    #[test]
    fn search_envelope_empty_results() {
        let out = format_search_envelope("hi", "ru", &[]);
        assert!(out.contains("results-count: 0"));
        assert!(out.contains("--- error ---"));
        assert!(out.contains("Поисковик не вернул"));
    }

    #[test]
    fn read_envelope_section_markers() {
        let doc = ReadDoc {
            source_url: "https://e.x/p".into(),
            final_url: "https://e.x/p".into(),
            status: 200,
            content_type: "text/html".into(),
            markdown: "# Hi".into(),
            readability_fallback: false,
            truncated: false,
        };
        let out = format_read_envelope(&doc);
        assert!(out.starts_with("GET https://e.x/p\n"));
        assert!(out.contains("status: 200"));
        assert!(out.contains("content-type: text/html"));
        assert!(out.contains("--- markdown ---"));
        assert!(out.contains("# Hi"));
        // final-url == source — строку не печатаем.
        assert!(!out.contains("final-url:"));
        assert!(!out.contains("readability: fallback"));
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn read_envelope_with_redirect_and_fallback_and_truncate() {
        let doc = ReadDoc {
            source_url: "https://e.x/p".into(),
            final_url: "https://e.x/p2".into(),
            status: 200,
            content_type: "text/html".into(),
            markdown: "body".into(),
            readability_fallback: true,
            truncated: true,
        };
        let out = format_read_envelope(&doc);
        assert!(out.contains("final-url: https://e.x/p2"));
        assert!(out.contains("readability: fallback (raw body)"));
        assert!(out.contains("(body truncated by read-size limit)"));
    }

    #[test]
    fn read_error_no_status_field() {
        let out = format_read_error("https://e.x/", "DNS failure");
        assert!(out.starts_with("GET https://e.x/\n"));
        assert!(out.contains("--- error ---"));
        assert!(out.contains("DNS failure"));
        assert!(!out.contains("status:"));
        assert!(!out.contains("content-type:"));
    }

    #[test]
    fn search_error_envelope() {
        let out = format_search_error("rust", "ru", "DNS failure");
        assert!(out.starts_with("WEB_SEARCH query=\"rust\" engine=duckduckgo lang=ru\n"));
        assert!(out.contains("--- error ---"));
        assert!(out.contains("DNS failure"));
    }

    #[test]
    fn escape_quotes_handles_quotes_and_backslash() {
        assert_eq!(escape_quotes(r#"hello"world"#), r#"hello\"world"#);
        assert_eq!(escape_quotes(r"a\b"), r"a\\b");
    }
}
