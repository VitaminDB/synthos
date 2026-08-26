//! Поиск через DuckDuckGo: основной html-endpoint + lite-fallback.
//!
//! Pipeline:
//! 1. `try_html_endpoint` → `https://html.duckduckgo.com/html/?q=…`
//!    парсится через scraper, селекторы `div.result` + `a.result__a`.
//! 2. Если ответ — challenge-страница (anti-bot) или просто пустой
//!    SERP, переходим к `try_lite_endpoint` → `https://lite.duckduckgo.com/lite/?q=…`
//!    с другой вёрсткой (table-based, прямые URL без `/l/?uddg=`).
//! 3. Сетевая ошибка (DNS / timeout / 4xx-5xx) на ЛЮБОМ шаге проброшена
//!    как `SearchError::Network` без попытки следующего endpoint'а —
//!    это означает «нет интернета», fallback не поможет.
//!
//! Контракт ошибок [`SearchError`]:
//! - `Network`     — network/transport error → envelope печатает как «DDG fetch: …»;
//! - `Challenge`   — оба endpoint'а отдали captcha → envelope подсказывает VPN/exit-IP;
//! - `Empty`       — оба endpoint'а отдали 200 без карточек → envelope подсказывает переформулировать.

use scraper::{Html, Selector};

use super::envelope::SerpHit;
use super::http::{self, FetchOpts};

/// Базовый URL DDG html-endpoint'а. Стабилен с 2010-х, работает без JS.
const DDG_BASE: &str = "https://html.duckduckgo.com/html/";

/// Lite-endpoint — резервный SSR-вариант DDG. Вёрстка table-based,
/// URL'ы прямые (без `/l/?uddg=` редиректа). Стабильнее «html»-эндпоинта
/// под anti-bot, но иногда бывает out of sync с актуальным индексом.
const DDG_LITE_BASE: &str = "https://lite.duckduckgo.com/lite/";

/// Referer, который DDG html-endpoint требует для отдачи SERP. Без него
/// возвращает 202 + редирект на главную (пустая страница без результатов).
/// Используем тот же хост (`html.duckduckgo.com`) — как делает searxng,
/// это ведёт себя стабильнее, чем referer от `duckduckgo.com`.
const DDG_REFERER: &str = "https://html.duckduckgo.com/";
const DDG_LITE_REFERER: &str = "https://lite.duckduckgo.com/";

/// Типизированная ошибка поиска. Все варианты конвертируются в
/// envelope с секцией `--- error ---` (см. `mod.rs::run`).
#[derive(Debug)]
pub enum SearchError {
    /// Транспорт: DNS/timeout/connection-reset. Fallback не пробуем —
    /// проблема не на стороне DDG.
    Network(String),
    /// Anti-bot challenge на обоих endpoint'ах. UI-подсказка: подождать
    /// или сменить exit-IP (VPN).
    Challenge,
    /// Оба endpoint'а ответили 200, но без карточек. Скорее всего
    /// слишком узкий запрос или редкая ситуация поломки DDG-индекса.
    Empty,
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "DDG fetch: {e}"),
            Self::Challenge => write!(
                f,
                "DDG is demanding a captcha (anti-bot challenge). Wait or \
                 switch the exit IP (VPN)."
            ),
            Self::Empty => write!(
                f,
                "The DDG html and lite endpoints returned an empty SERP. \
                 The markup may have changed, or the query is too narrow — \
                 try rephrasing it."
            ),
        }
    }
}

/// Внутренний результат одного endpoint'а.
enum EndpointOutcome {
    Hits(Vec<SerpHit>),
    Challenge,
    Empty,
}

/// Entrypoint для tool'а. Каскадно пробует html → lite endpoint.
pub async fn search(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<Vec<SerpHit>, SearchError> {
    let primary = try_html_endpoint(query, lang, max_results).await?;
    match primary {
        EndpointOutcome::Hits(hits) if !hits.is_empty() => return Ok(hits),
        EndpointOutcome::Hits(_) | EndpointOutcome::Empty | EndpointOutcome::Challenge => {
            // Падающий вниз сценарий: оба исхода без хитов → пробуем lite.
        }
    }

    // Запоминаем, был ли challenge на основном — если на lite тоже challenge
    // или empty, выбираем «более информативное» сообщение.
    let primary_was_challenge = matches!(primary, EndpointOutcome::Challenge);
    let secondary = try_lite_endpoint(query, lang, max_results).await?;
    match secondary {
        EndpointOutcome::Hits(hits) if !hits.is_empty() => Ok(hits),
        EndpointOutcome::Challenge => Err(SearchError::Challenge),
        EndpointOutcome::Hits(_) | EndpointOutcome::Empty => {
            if primary_was_challenge {
                Err(SearchError::Challenge)
            } else {
                Err(SearchError::Empty)
            }
        }
    }
}

async fn try_html_endpoint(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<EndpointOutcome, SearchError> {
    let url = build_search_url(query, lang);
    let html = http::fetch_html_text_with(
        &url,
        FetchOpts {
            referer: Some(DDG_REFERER),
            accept_language: Some(accept_language_for(lang)),
        },
    )
    .await
    .map_err(|e| SearchError::Network(e.to_string()))?;

    if detect_challenge(&html) {
        return Ok(EndpointOutcome::Challenge);
    }

    let mut hits = parse_ddg(&html, max_results);
    hits = dedup_by_origin(hits);
    let hits = renumber(hits, max_results);
    if hits.is_empty() {
        Ok(EndpointOutcome::Empty)
    } else {
        Ok(EndpointOutcome::Hits(hits))
    }
}

async fn try_lite_endpoint(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<EndpointOutcome, SearchError> {
    let url = build_lite_url(query, lang);
    let html = http::fetch_html_text_with(
        &url,
        FetchOpts {
            referer: Some(DDG_LITE_REFERER),
            accept_language: Some(accept_language_for(lang)),
        },
    )
    .await
    .map_err(|e| SearchError::Network(e.to_string()))?;

    if detect_challenge(&html) {
        return Ok(EndpointOutcome::Challenge);
    }

    let mut hits = parse_ddg_lite(&html, max_results);
    hits = dedup_by_origin(hits);
    let hits = renumber(hits, max_results);
    if hits.is_empty() {
        Ok(EndpointOutcome::Empty)
    } else {
        Ok(EndpointOutcome::Hits(hits))
    }
}

fn build_search_url(query: &str, lang: &str) -> String {
    let q_enc = urlencoding::encode(query);
    let kl = kl_for(lang);
    format!("{DDG_BASE}?q={q_enc}&kl={kl}&ia=web")
}

fn build_lite_url(query: &str, lang: &str) -> String {
    let q_enc = urlencoding::encode(query);
    let kl = kl_for(lang);
    format!("{DDG_LITE_BASE}?q={q_enc}&kl={kl}")
}

/// `kl` = country/locale-locale. Для ru — `ru-ru`, для en — `us-en`,
/// прочее — пробрасываем как есть в формате `<lang>-<lang>`.
fn kl_for(lang: &str) -> String {
    match lang {
        "en" => "us-en".to_string(),
        other => format!("{other}-{other}"),
    }
}

/// Header `Accept-Language` под язык запроса. Если язык неизвестный —
/// пробрасываем сам lang + английский fallback.
fn accept_language_for(lang: &str) -> &'static str {
    match lang {
        "ru" => "ru-RU,ru;q=0.9,en-US;q=0.5,en;q=0.3",
        "en" => "en-US,en;q=0.9",
        "de" => "de-DE,de;q=0.9,en;q=0.5",
        "fr" => "fr-FR,fr;q=0.9,en;q=0.5",
        "es" => "es-ES,es;q=0.9,en;q=0.5",
        "it" => "it-IT,it;q=0.9,en;q=0.5",
        "pt" => "pt-PT,pt;q=0.9,en;q=0.5",
        "uk" => "uk-UA,uk;q=0.9,ru;q=0.6,en;q=0.3",
        // Остальные языки: en fallback. Можно расширить под необходимость.
        _ => "en-US,en;q=0.9",
    }
}

/// Детект anti-bot challenge'а DDG. Срабатывает если в HTML есть форма
/// с известными признаками челленджа или явная фраза в тексте.
///
/// На обоих endpoint'ах (html и lite) DDG вместо SERP отдаёт страницу
/// «Unfortunately, bots use DuckDuckGo too» с формой `id="challenge-form"`
/// или `action`-URL содержащим `challenge`.
pub(super) fn detect_challenge(html: &str) -> bool {
    // Быстрая проверка по тексту — без парсинга всего DOM.
    if html.contains("Unfortunately, bots use DuckDuckGo too")
        || html.contains("anomaly_modal")
    {
        return true;
    }
    let doc = Html::parse_document(html);
    let Ok(sel) = Selector::parse(
        r#"form#challenge-form, form[action*="challenge"], div#anomaly-modal__title"#,
    ) else {
        return false;
    };
    doc.select(&sel).next().is_some()
}

fn parse_ddg(html: &str, max: usize) -> Vec<SerpHit> {
    let doc = Html::parse_document(html);

    // Селекторы DDG html-endpoint'а. Стабильны давно: `div.result` —
    // карточка одного результата; `a.result__a` — заголовок-ссылка
    // (href всегда uddg-обёрнут); `a.result__snippet` — превью текста.
    // Запасные селекторы добавлены на случай, если DDG чуть переструктурирует
    // вёрстку (исторически такое случается раз в год-два).
    let Ok(card_sel) = Selector::parse("div.result, div.web-result") else {
        return Vec::new();
    };
    let Ok(title_sel) = Selector::parse("h2.result__title a.result__a, a.result__a, h2 a") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = Selector::parse(
        "a.result__snippet, div.result__snippet, [data-result=\"snippet\"]",
    ) else {
        return Vec::new();
    };

    let mut out: Vec<SerpHit> = Vec::new();
    for card in doc.select(&card_sel) {
        if out.len() >= max {
            break;
        }
        let Some(a) = card.select(&title_sel).next() else {
            continue;
        };
        let title = clean_text(&a.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let raw_href = a.value().attr("href").unwrap_or("");
        let url = unwrap_uddg(raw_href);
        if !is_http_url(&url) {
            continue;
        }
        let snippet = card
            .select(&snippet_sel)
            .next()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SerpHit {
            rank: out.len() + 1,
            title,
            url,
            snippet,
        });
    }
    out
}

/// Парсер lite-вёрстки DDG. Структура — таблица: для каждого результата
/// идут три `<tr>`:
///   1. ссылка-результат: `<a class="result-link" href="<прямой URL>">title</a>`
///   2. сниппет: `<td class="result-snippet">…</td>`
///   3. display-URL: `<span class="link-text">domain.com</span>`
///
/// Title и snippet склеиваются по индексу — проще и надёжнее, чем
/// пытаться найти их через DOM-навигацию (lite-html шумный).
fn parse_ddg_lite(html: &str, max: usize) -> Vec<SerpHit> {
    let doc = Html::parse_document(html);

    let Ok(link_sel) = Selector::parse("a.result-link") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = Selector::parse("td.result-snippet") else {
        return Vec::new();
    };

    let links: Vec<_> = doc.select(&link_sel).collect();
    let snippets: Vec<String> = doc
        .select(&snippet_sel)
        .map(|td| clean_text(&td.text().collect::<String>()))
        .collect();

    let mut out: Vec<SerpHit> = Vec::new();
    for (i, a) in links.iter().enumerate() {
        if out.len() >= max {
            break;
        }
        let title = clean_text(&a.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let raw_href = a.value().attr("href").unwrap_or("");
        // Lite иногда тоже использует /l/?uddg= — на всякий случай прогоняем.
        let url = unwrap_uddg(raw_href);
        if !is_http_url(&url) {
            continue;
        }
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        out.push(SerpHit {
            rank: out.len() + 1,
            title,
            url,
            snippet,
        });
    }
    out
}

/// Распаковывает обёртку DDG `/l/?uddg=<encoded url>`. Если href —
/// обычный http(s):// URL, возвращает его как есть.
///
/// Принимаемые формы:
/// - `//duckduckgo.com/l/?uddg=…` (protocol-relative);
/// - `/l/?uddg=…` (path-only);
/// - `https://duckduckgo.com/l/?uddg=…`.
fn unwrap_uddg(href: &str) -> String {
    if href.is_empty() {
        return String::new();
    }
    if !href.contains("/l/?") {
        return href.to_string();
    }
    let candidate = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    if let Ok(u) = url::Url::parse(&candidate) {
        if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "uddg") {
            return v.into_owned();
        }
    }
    href.to_string()
}

/// Дедуп по `host + path` (без query/fragment). Сохраняет порядок.
fn dedup_by_origin(hits: Vec<SerpHit>) -> Vec<SerpHit> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let key = url::Url::parse(&h.url)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|host| format!("{}{}", host.to_ascii_lowercase(), u.path()))
            })
            .unwrap_or_else(|| h.url.clone());
        if seen.insert(key) {
            out.push(h);
        }
    }
    out
}

fn renumber(mut hits: Vec<SerpHit>, max: usize) -> Vec<SerpHit> {
    hits.truncate(max);
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i + 1;
    }
    hits
}

/// Стандартизация whitespace: trim + collapse runs of whitespace в один пробел.
fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_http_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_encodes_cyrillic_and_locale() {
        let u = build_search_url("руст асинхронный", "ru");
        assert!(u.starts_with("https://html.duckduckgo.com/html/?q="), "{u}");
        assert!(u.contains("&kl=ru-ru"), "{u}");
        assert!(u.contains("&ia=web"), "{u}");
        // Кириллица должна быть процент-кодирована.
        assert!(u.contains("%D1%80%D1%83%D1%81%D1%82"), "{u}");
    }

    #[test]
    fn build_url_english_uses_us_en() {
        let u = build_search_url("rust async", "en");
        assert!(u.contains("&kl=us-en"), "{u}");
    }

    #[test]
    fn build_lite_url_uses_lite_host() {
        let u = build_lite_url("rust async", "ru");
        assert!(u.starts_with("https://lite.duckduckgo.com/lite/?q="), "{u}");
        assert!(u.contains("&kl=ru-ru"), "{u}");
    }

    #[test]
    fn accept_language_known_langs() {
        assert!(accept_language_for("ru").starts_with("ru-RU,"));
        assert!(accept_language_for("en").starts_with("en-US,"));
        assert!(accept_language_for("de").starts_with("de-DE,"));
        // Неизвестный язык → en fallback.
        assert!(accept_language_for("xx").starts_with("en-US,"));
    }

    #[test]
    fn parse_minimal_card() {
        let html = r#"<!doctype html><html><body>
            <div class="result">
              <h2 class="result__title">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fpage&amp;rut=abc">Example Page</a>
              </h2>
              <a class="result__snippet">A page about examples.</a>
            </div>
        </body></html>"#;
        let hits = parse_ddg(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Example Page");
        assert_eq!(hits[0].url, "https://example.org/page");
        assert!(hits[0].snippet.contains("examples"));
    }

    #[test]
    fn parse_clamps_to_max() {
        let mut html = String::from("<html><body>");
        for i in 0..5 {
            html.push_str(&format!(
                r#"<div class="result">
                    <h2 class="result__title"><a class="result__a" href="https://e{}.x/p">Title {}</a></h2>
                    <a class="result__snippet">Snippet {}</a>
                </div>"#,
                i, i, i
            ));
        }
        html.push_str("</body></html>");
        let hits = parse_ddg(&html, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Title 0");
        assert_eq!(hits[2].title, "Title 2");
    }

    #[test]
    fn parse_skips_card_without_title() {
        let html = r#"<html><body>
            <div class="result"><a class="result__a" href="https://e.x/p"></a></div>
            <div class="result">
                <h2><a class="result__a" href="https://e.x/p2">Real Title</a></h2>
            </div>
        </body></html>"#;
        let hits = parse_ddg(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Real Title");
    }

    #[test]
    fn parse_empty_html_returns_empty() {
        assert!(parse_ddg("", 10).is_empty());
        assert!(parse_ddg("<html></html>", 10).is_empty());
    }

    #[test]
    fn parse_lite_minimal_card() {
        // Lite-вёрстка: title в a.result-link, snippet в td.result-snippet
        // (отдельный <tr>). Связка по индексу — чем длиннее списки, тем
        // важнее порядок DOM-обхода (`scraper` сохраняет document order).
        let html = r#"<!doctype html><html><body><table>
            <tr><td><a class="result-link" rel="nofollow" href="https://example.org/page">Example Page</a></td></tr>
            <tr><td class="result-snippet">A page about examples.</td></tr>
            <tr><td><span class="link-text">example.org</span></td></tr>
        </table></body></html>"#;
        let hits = parse_ddg_lite(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Example Page");
        assert_eq!(hits[0].url, "https://example.org/page");
        assert!(hits[0].snippet.contains("examples"));
    }

    #[test]
    fn parse_lite_unwraps_uddg() {
        // На всякий случай — если lite вдруг отдаст /l/?uddg=…, мы это
        // тоже обрабатываем (через общий unwrap_uddg).
        let html = r#"<html><body><table>
            <tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.b%2Fc">Title</a></td></tr>
            <tr><td class="result-snippet">Snip</td></tr>
        </table></body></html>"#;
        let hits = parse_ddg_lite(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://a.b/c");
    }

    #[test]
    fn parse_lite_clamps_to_max() {
        let mut html = String::from("<html><body><table>");
        for i in 0..5 {
            html.push_str(&format!(
                r#"<tr><td><a class="result-link" href="https://e{i}.x/p">Title {i}</a></td></tr>
                   <tr><td class="result-snippet">Snippet {i}</td></tr>"#
            ));
        }
        html.push_str("</table></body></html>");
        let hits = parse_ddg_lite(&html, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Title 0");
        assert_eq!(hits[2].title, "Title 2");
        assert_eq!(hits[2].snippet, "Snippet 2");
    }

    #[test]
    fn parse_lite_empty_html_returns_empty() {
        assert!(parse_ddg_lite("", 10).is_empty());
        assert!(parse_ddg_lite("<html></html>", 10).is_empty());
    }

    #[test]
    fn detect_challenge_form_id() {
        let html = r#"<html><body>
            <form id="challenge-form" action="/challenge" method="post"></form>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_action_substring() {
        let html = r#"<html><body>
            <form action="/anti-bot/challenge?id=1"></form>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_text_phrase() {
        let html = r#"<html><body>
            <p>Unfortunately, bots use DuckDuckGo too — please prove you are human.</p>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_normal_serp_is_negative() {
        let html = r#"<html><body>
            <div class="result">
                <h2><a class="result__a" href="https://e.x/p">Title</a></h2>
            </div>
        </body></html>"#;
        assert!(!detect_challenge(html));
    }

    #[test]
    fn search_error_display_messages() {
        // Сообщения должны быть осмысленными — этим текстом envelope
        // подсвечивает причину пользователю.
        assert!(SearchError::Network("conn".into()).to_string().contains("DDG fetch"));
        assert!(SearchError::Challenge.to_string().contains("captcha"));
        assert!(SearchError::Empty.to_string().contains("empty SERP"));
    }

    #[test]
    fn unwrap_uddg_protocol_relative() {
        assert_eq!(
            unwrap_uddg("//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.b%2F&rut=x"),
            "https://a.b/"
        );
    }

    #[test]
    fn unwrap_uddg_path_only() {
        assert_eq!(
            unwrap_uddg("/l/?uddg=https%3A%2F%2Fc.d%2Fe&rut=y"),
            "https://c.d/e"
        );
    }

    #[test]
    fn unwrap_uddg_passthrough() {
        // Прямой http(s) URL — не трогаем.
        assert_eq!(
            unwrap_uddg("https://target.example/path"),
            "https://target.example/path"
        );
    }

    #[test]
    fn dedup_collapses_same_path_with_query_diff() {
        let hits = vec![
            SerpHit {
                rank: 1,
                title: "a".into(),
                url: "https://example.com/p?x=1".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 2,
                title: "b".into(),
                url: "https://example.com/p?x=2".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 3,
                title: "c".into(),
                url: "https://OTHER.com/p".into(),
                snippet: String::new(),
            },
        ];
        let out = dedup_by_origin(hits);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "a");
        assert_eq!(out[1].title, "c");
    }

    #[test]
    fn dedup_handles_invalid_url_as_unique_key() {
        let hits = vec![
            SerpHit {
                rank: 1,
                title: "a".into(),
                url: "not a url".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 2,
                title: "b".into(),
                url: "not a url".into(),
                snippet: String::new(),
            },
        ];
        let out = dedup_by_origin(hits);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn renumber_truncates_and_resets_ranks() {
        let hits = vec![
            SerpHit { rank: 5, title: "a".into(), url: "u1".into(), snippet: String::new() },
            SerpHit { rank: 7, title: "b".into(), url: "u2".into(), snippet: String::new() },
            SerpHit { rank: 9, title: "c".into(), url: "u3".into(), snippet: String::new() },
        ];
        let out = renumber(hits, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].rank, 1);
        assert_eq!(out[1].rank, 2);
    }

    #[test]
    fn clean_text_collapses_whitespace() {
        assert_eq!(clean_text("  hello   world\n\t!  "), "hello world !");
    }

    /// Реальный smoke: запрос к DDG html-endpoint'у. Запуск:
    /// `cargo test -p synthos --lib chat::tools::web::search::tests::smoke_search_real_ddg \
    ///     -- --ignored --nocapture`
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg() {
        let hits = super::search("rust async", "en", 10)
            .await
            .expect("DDG должен ответить");
        eprintln!("hits: {}", hits.len());
        for h in &hits {
            eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
        }
        assert!(
            hits.len() >= 3,
            "ожидаемо минимум 3 результата на популярный запрос, получили: {}",
            hits.len()
        );
    }

    /// Smoke на русский запрос (тот самый кейс, на котором изначально
    /// поймали поломку). Локаль `kl=ru-ru` + `Accept-Language: ru-RU,…`.
    /// Запуск:
    /// `cargo test -p synthos --lib chat::tools::web::search::tests::smoke_search_real_ddg_ru \
    ///     -- --ignored --nocapture`
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg_ru() {
        let hits = super::search("новости сегодня главные события", "ru", 10)
            .await
            .expect("DDG должен ответить на русский запрос");
        eprintln!("ru hits: {}", hits.len());
        for h in &hits {
            eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
        }
        assert!(
            hits.len() >= 3,
            "ожидаемо минимум 3 результата на новости, получили: {}",
            hits.len()
        );
    }

    /// Smoke к lite-endpoint'у. Запуск:
    /// `cargo test -p synthos --lib chat::tools::web::search::tests::smoke_search_real_ddg_lite \
    ///     -- --ignored --nocapture`
    /// Ожидаем хиты: lite — менее агрессивно фильтруется anti-bot'ом.
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg_lite() {
        let outcome = super::try_lite_endpoint("rust async", "en", 10)
            .await
            .expect("lite DDG должен ответить");
        match outcome {
            super::EndpointOutcome::Hits(hits) => {
                eprintln!("lite hits: {}", hits.len());
                for h in &hits {
                    eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
                }
                assert!(hits.len() >= 3, "минимум 3 хита, получили: {}", hits.len());
            }
            super::EndpointOutcome::Empty => panic!("lite вернул пустой SERP"),
            super::EndpointOutcome::Challenge => panic!("lite ловит captcha — IP-block"),
        }
    }
}
