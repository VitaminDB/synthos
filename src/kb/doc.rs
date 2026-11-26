//! Разбор документов, нарезка и обход папок для базы знаний. `parse`:
//! md/html/pdf/text → plain_text(+title); `chunk`: окно по токенам с
//! перекрытием, границы токенов → байтовые срезы (границы даёт токенизатор
//! эмбеддера в службе, [`TokenSpans`]); `walk`: обход с учётом `.gitignore`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Markdown,
    Html,
    Pdf,
    Text,
}

impl SourceKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())?;
        match ext.as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "html" | "htm" => Some(Self::Html),
            "pdf" => Some(Self::Pdf),
            "txt" | "text" => Some(Self::Text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParsedDoc {
    pub plain_text: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub token_count: usize,
}

#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub target_tokens: usize,
    pub overlap_tokens: usize,
    pub min_tokens: usize,
}

pub fn parse(bytes: &[u8], kind: SourceKind) -> Result<ParsedDoc, String> {
    match kind {
        SourceKind::Text => Ok(ParsedDoc {
            plain_text: String::from_utf8_lossy(bytes).into_owned(),
            title: None,
        }),
        SourceKind::Markdown => parse_markdown(bytes),
        SourceKind::Html => parse_html(bytes),
        SourceKind::Pdf => parse_pdf(bytes),
    }
}

fn parse_markdown(bytes: &[u8]) -> Result<ParsedDoc, String> {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    let src = String::from_utf8_lossy(bytes);
    let mut plain = String::new();
    let mut title: Option<String> = None;
    let mut in_h1 = false;
    let mut h1_buf = String::new();
    let mut pending_break = false;

    for ev in Parser::new(&src) {
        match ev {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                if title.is_none() {
                    in_h1 = true;
                    h1_buf.clear();
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if in_h1 {
                    in_h1 = false;
                    if title.is_none() && !h1_buf.trim().is_empty() {
                        title = Some(h1_buf.trim().to_string());
                    }
                }
                pending_break = true;
            }
            Event::Start(_) => {}
            Event::End(_) => {
                pending_break = true;
            }
            Event::Text(t) | Event::Code(t) => {
                if in_h1 {
                    h1_buf.push_str(&t);
                }
                if pending_break && !plain.is_empty() {
                    plain.push('\n');
                    pending_break = false;
                }
                plain.push_str(&t);
            }
            Event::SoftBreak | Event::HardBreak => {
                plain.push('\n');
            }
            Event::Rule => {
                plain.push('\n');
            }
            _ => {}
        }
    }

    Ok(ParsedDoc { plain_text: plain, title })
}

fn parse_html(bytes: &[u8]) -> Result<ParsedDoc, String> {
    let src = String::from_utf8_lossy(bytes);
    let title = extract_html_title(&src);
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec![
            "script", "style", "noscript", "nav", "header", "footer", "iframe",
        ])
        .build();
    let md = converter.convert(&src).map_err(|e| format!("htmd convert: {e}"))?;
    let title = title.or_else(|| first_markdown_h1(&md));
    Ok(ParsedDoc { plain_text: md, title })
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find("</title>")? + gt;
    let raw = html[gt..close].trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn first_markdown_h1(md: &str) -> Option<String> {
    for line in md.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("# ") {
            let h = rest.trim();
            if !h.is_empty() {
                return Some(h.to_string());
            }
        }
    }
    None
}

fn parse_pdf(bytes: &[u8]) -> Result<ParsedDoc, String> {
    // pdf-extract обычно возвращает Result, но на повреждённых PDF может
    // паниковать в зависимостях — оборачиваем в catch_unwind.
    let bytes_owned = bytes.to_vec();
    let res = std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&bytes_owned));
    match res {
        Ok(Ok(text)) => Ok(ParsedDoc { plain_text: text, title: None }),
        Ok(Err(e)) => Err(format!("pdf-extract: {e}")),
        Err(_) => Err("pdf-extract: паника при разборе PDF".into()),
    }
}

/// Байтовые границы токенов текста `[начало, конец)`.
pub trait TokenSpans {
    fn spans(&self, text: &str) -> Result<Vec<(usize, usize)>, String>;
}

pub fn chunk(doc: &ParsedDoc, tokenizer: &dyn TokenSpans, cfg: &ChunkConfig) -> Result<Vec<Chunk>, String> {
    let text = &doc.plain_text;
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let offsets = tokenizer.spans(text.as_str()).map_err(|e| format!("токенизатор: {e}"))?;
    let offsets = offsets.as_slice();
    let n_tokens = offsets.len();
    if n_tokens == 0 {
        return Ok(Vec::new());
    }

    let target = cfg.target_tokens.max(1);
    let step = if cfg.overlap_tokens >= target {
        1
    } else {
        target - cfg.overlap_tokens
    };

    let text_len = text.len();
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut start = 0usize;

    while start < n_tokens {
        let end = (start + target).min(n_tokens);
        let token_count = end - start;

        let raw_start = offsets[start].0;
        let raw_end = offsets[end - 1].1;
        let (s, e) = clamp_char_boundaries(text, raw_start, raw_end, text_len);

        if e > s {
            chunks.push(Chunk {
                text: text[s..e].to_string(),
                start_byte: s,
                end_byte: e,
                token_count,
            });
        }

        if end >= n_tokens {
            break;
        }
        start += step;
    }

    let filtered: Vec<Chunk> = chunks
        .iter()
        .filter(|c| c.token_count >= cfg.min_tokens)
        .cloned()
        .collect();

    if filtered.is_empty() {
        if let Some(first) = chunks.into_iter().find(|c| !c.text.is_empty()) {
            return Ok(vec![first]);
        }
        return Ok(Vec::new());
    }

    Ok(filtered)
}

fn clamp_char_boundaries(
    text: &str,
    mut start: usize,
    mut end: usize,
    text_len: usize,
) -> (usize, usize) {
    if start > text_len {
        start = text_len;
    }
    if end > text_len {
        end = text_len;
    }
    while start < text_len && !text.is_char_boundary(start) {
        start += 1;
    }
    while end < text_len && !text.is_char_boundary(end) {
        end += 1;
    }
    if start > end {
        start = end;
    }
    (start, end)
}

pub mod ignore {
    use super::{Path, PathBuf, SourceKind};

    #[derive(Debug, Clone)]
    pub struct WalkedFile {
        pub path: PathBuf,
        pub kind: SourceKind,
    }

    #[derive(Debug, Clone, Default)]
    pub struct WalkConfig {
        pub root: PathBuf,
        pub extra_excludes: Vec<String>,
    }

    impl WalkConfig {
        pub fn new(root: impl AsRef<Path>) -> Self {
            Self { root: root.as_ref().to_path_buf(), extra_excludes: Vec::new() }
        }
    }

    pub fn walk(cfg: &WalkConfig) -> Vec<WalkedFile> {
        let mut out = Vec::new();
        for entry in ::ignore::WalkBuilder::new(&cfg.root).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let is_file = entry.file_type().map(|ft| ft.is_file()).unwrap_or(false);
            if !is_file {
                continue;
            }
            let path = entry.path();
            let kind = match SourceKind::from_path(path) {
                Some(k) => k,
                None => continue,
            };
            let path_str = path.to_string_lossy();
            if cfg
                .extra_excludes
                .iter()
                .any(|ex| !ex.is_empty() && path_str.contains(ex.as_str()))
            {
                continue;
            }
            out.push(WalkedFile { path: path.to_path_buf(), kind });
        }
        out
    }
}
