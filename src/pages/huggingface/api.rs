//! HTTP-клиент к HuggingFace Hub API.
//!
//! Все методы async и должны вызываться из `syngui::async_runtime::spawn`.
//! Обновления UI делаются через `run_on_main_thread` в коллбэках. Сам клиент
//! `reqwest::Client` создаётся лениво один раз и переиспользуется (keep-alive
//! пул соединений).
//!
//! Эндпоинты:
//! - `GET https://huggingface.co/api/models?search=&sort=&direction=-1&limit=`
//! - `GET https://huggingface.co/api/models/{repo_id}`
//! - `GET https://huggingface.co/{repo_id}/raw/main/README.md`
//! - `GET https://huggingface.co/{repo_id}/resolve/main/{filename}`

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use futures_util::StreamExt;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::RwSignal;
use syngui::tr;
use reqwest::Client;
use std::collections::HashMap;

use super::state::{DownloadState, HfModel, HfModelDetails, SegmentProgress, SortMode};

/// EMA-обновление поля `speed_bps` в DownloadState. Вызывается из
/// `run_on_main_thread` callbacks после обновления `bytes_done`. Окно
/// измерения ~500мс — короче считаем «измеренная скорость = 0» (пропускаем),
/// длиннее — переменная не успевает отреагировать на realtime-колебания.
fn update_speed(d: &mut DownloadState) {
    let now = std::time::Instant::now();
    let Some(prev) = d.speed_sample_at else {
        d.speed_sample_at = Some(now);
        d.speed_sample_bytes = d.bytes_done;
        return;
    };
    let dt = now.duration_since(prev).as_secs_f64();
    if dt < 0.5 {
        return;
    }
    let delta = d.bytes_done.saturating_sub(d.speed_sample_bytes) as f64;
    let inst = delta / dt;
    // EMA (alpha=0.4) — сглаживает скачки между чанками без заметного лага.
    d.speed_bps = if d.speed_bps <= 0.0 {
        inst
    } else {
        d.speed_bps * 0.6 + inst * 0.4
    };
    d.speed_sample_at = Some(now);
    d.speed_sample_bytes = d.bytes_done;
}

/// Записать прогресс всех сегментов + агрегат в `downloads`. Вызывается из
/// `run_on_main_thread`. `meta` — путь sidecar для throttled-записи (≥1с);
/// `None` — финальный флаш сегмента, meta не трогаем (файл вот-вот завершится).
fn push_segment_progress(
    downloads_sig: RwSignal<HashMap<String, DownloadState>>,
    key: &str,
    dones: &[u64],
    meta: Option<&Path>,
) {
    downloads_sig.update(|m| {
        let Some(d) = m.get_mut(key) else { return };
        for (i, &bd) in dones.iter().enumerate() {
            if let Some(s) = d.segments.get_mut(i) {
                s.bytes_done = bd;
            }
        }
        d.bytes_done = d.segments.iter().map(|s| s.bytes_done).sum();
        update_speed(d);
        let Some(meta_path) = meta else { return };
        let now = std::time::Instant::now();
        let should_flush = d
            .last_meta_flush_at
            .map(|t| now.duration_since(t).as_secs_f64() >= 1.0)
            .unwrap_or(true);
        if should_flush {
            d.last_meta_flush_at = Some(now);
            let total = d.total;
            let segs = d.segments.clone();
            let path = meta_path.to_path_buf();
            syngui::async_runtime::spawn(async move {
                write_meta_async(path, total, segs).await;
            });
        }
    });
}

const API_HOST: &str = "https://huggingface.co";
const USER_AGENT: &str = concat!("synthos-hf/", env!("SYNTHOS_VERSION"));

/// Минимальный интервал между UI-флэшами прогресса (мс). Прогресс копится в
/// задаче и шлётся в main-thread не чаще — на быстрой сети это срезает тысячи
/// `downloads_sig.update` (и перерисовок) в секунду до ~8/сек/сегмент без
/// видимого лага. Финальный флаш после цикла гарантирует точное значение.
const PROGRESS_FLUSH_MS: u128 = 120;

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Чем именно прервали закачку — определяет ветку обработки в `download.rs`
/// (Paused/Stopped сохраняют `.part`, Cancel удаляет).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortKind {
    Pause,
    Stop,
    Cancel,
    GlobalPause,
}

#[derive(Debug)]
pub enum HfError {
    Transport(reqwest::Error),
    Status(u16, String),
    Decode(String),
    Io(std::io::Error),
    /// Закачку прервал пользователь (см. `control.rs`). Не ретраится.
    Aborted(AbortKind),
}

impl fmt::Display for HfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HfError::Transport(e) => write!(f, "{}: {e}", tr!("hf.error.network")),
            HfError::Status(code, body) => {
                let hint = match code {
                    401 => tr!("hf.error.status_hint.401"),
                    403 => tr!("hf.error.status_hint.403"),
                    429 => tr!("hf.error.status_hint.429"),
                    _ => String::new(),
                };
                let snippet: String = body.chars().take(160).collect();
                write!(f, "HTTP {code}{hint}: {snippet}")
            }
            HfError::Decode(msg) => write!(f, "{}: {msg}", tr!("hf.error.decode")),
            HfError::Io(e) => write!(f, "{}: {e}", tr!("hf.error.io")),
            HfError::Aborted(kind) => match kind {
                AbortKind::Pause => write!(f, "{}", tr!("hf.error.aborted.paused")),
                AbortKind::Stop => write!(f, "{}", tr!("hf.error.aborted.stopped")),
                AbortKind::Cancel => write!(f, "{}", tr!("hf.error.aborted.cancelled")),
                AbortKind::GlobalPause => write!(f, "{}", tr!("hf.error.aborted.global_pause")),
            },
        }
    }
}

impl From<reqwest::Error> for HfError {
    fn from(e: reqwest::Error) -> Self { HfError::Transport(e) }
}
impl From<std::io::Error> for HfError {
    fn from(e: std::io::Error) -> Self { HfError::Io(e) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared client
// ─────────────────────────────────────────────────────────────────────────────

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest client build")
    })
}

/// Глобальный HF access-token. Пишется из `HuggingFaceCtx` (стартовый load +
/// эффект на смену поля в Settings); читается каждым запросом ниже. Пусто =
/// анонимный доступ. `RwLock` достаточно — пишется редко, читается per-request.
static HF_TOKEN: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// Установить/сменить токен (вызывается из UI-слоя при загрузке конфига и при
/// правке поля). Trim — пользователь часто копирует с пробелом/переводом строки.
pub fn set_token(token: &str) {
    if let Ok(mut g) = HF_TOKEN.write() {
        *g = token.trim().to_string();
    }
}

fn current_token() -> Option<String> {
    let g = HF_TOKEN.read().ok()?;
    if g.is_empty() { None } else { Some(g.clone()) }
}

/// `client().get` с авто-инъекцией `Authorization: Bearer <token>`, когда токен
/// задан. Все HF-запросы (list/details/readme/probe/download) идут через это —
/// gated/private модели и снятие anon rate-limit.
fn authed_get(url: &str) -> reqwest::RequestBuilder {
    let rb = client().get(url);
    match current_token() {
        Some(t) => rb.bearer_auth(t),
        None => rb,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// API methods
// ─────────────────────────────────────────────────────────────────────────────

/// Список моделей: trending / search / sort. `limit` ≤ 100.
pub async fn list_models(
    query: &str,
    sort: SortMode,
    limit: u32,
    gguf_only: bool,
) -> Result<Vec<HfModel>, HfError> {
    let mut url = format!(
        "{API_HOST}/api/models?sort={}&direction=-1&limit={}",
        sort.api_field(),
        limit
    );
    let q = query.trim();
    if !q.is_empty() {
        url.push_str("&search=");
        url.push_str(&urlencoding::encode(q));
    }
    if gguf_only {
        url.push_str("&filter=gguf");
    }
    let resp = authed_get(&url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(HfError::Status(status.as_u16(), body));
    }
    let bytes = resp.bytes().await?;
    serde_json::from_slice::<Vec<HfModel>>(&bytes).map_err(|e| HfError::Decode(e.to_string()))
}

pub async fn get_model_details(repo_id: &str) -> Result<HfModelDetails, HfError> {
    // `?blobs=true` заставляет HF Hub отдать `size` для каждого sibling'а —
    // без этого поля файлы в Files-tab показывают «—». Запрос становится
    // тяжелее (HEAD по каждому файлу на стороне HF), но он делается один раз
    // на выбор карточки, не на каждый rerender.
    let url = format!("{API_HOST}/api/models/{}?blobs=true", repo_id);
    let resp = authed_get(&url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(HfError::Status(status.as_u16(), body));
    }
    let bytes = resp.bytes().await?;
    serde_json::from_slice::<HfModelDetails>(&bytes).map_err(|e| HfError::Decode(e.to_string()))
}

/// Возвращает README репозитория или пустую строку, если README не существует
/// (404 трактуется как «нет README», а не ошибка сети).
pub async fn fetch_readme(repo_id: &str) -> Result<String, HfError> {
    let url = format!("{API_HOST}/{}/raw/main/README.md", repo_id);
    let resp = authed_get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(String::new());
    }
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(HfError::Status(status.as_u16(), body));
    }
    let body = resp.text().await?;
    Ok(rewrite_readme_image_urls(&body, repo_id))
}

/// Переписывает относительные ссылки на изображения в README (markdown
/// `![](path)` и HTML `<img src="path">`) в абсолютные HF-raw-URL'ы
/// `{API_HOST}/{repo_id}/resolve/main/{path}`. Без этого MarkdownView
/// трактует `comparison.png` как URL и падает на «unsupported url scheme»,
/// а картинки model-card'а не отображаются. Абсолютные ссылки (http/https/
/// data/protocol-relative/anchor/angle-bracket) не трогаем.
pub fn rewrite_readme_image_urls(md: &str, repo_id: &str) -> String {
    let md = rewrite_markdown_images(md, repo_id);
    rewrite_html_img_src(&md, repo_id)
}

fn is_absolute_ref(url: &str) -> bool {
    let t = url.trim();
    t.is_empty()
        || t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("data:")
        || t.starts_with("//")
        || t.starts_with('#')
        || t.starts_with("mailto:")
        || t.starts_with('<')
}

fn resolve_hf_url(url: &str, repo_id: &str) -> String {
    let path = url.trim().trim_start_matches("./").trim_start_matches('/');
    format!("{API_HOST}/{}/resolve/main/{}", repo_id, path)
}

fn find_byte(bytes: &[u8], start: usize, b: u8) -> Option<usize> {
    bytes[start..].iter().position(|&x| x == b).map(|p| start + p)
}

/// Из содержимого скобок `(...)` markdown-картинки выделяет URL и «хвост»
/// (опциональный ` "title"`). Ведущие пробелы URL'а отбрасываем.
fn split_url_title(inner: &str) -> (&str, &str) {
    let lead = inner.len() - inner.trim_start().len();
    let s = &inner[lead..];
    match s.find(char::is_whitespace) {
        Some(p) => (&s[..p], &s[p..]),
        None => (s, ""),
    }
}

fn rewrite_markdown_images(md: &str, repo_id: &str) -> String {
    let bytes = md.as_bytes();
    let mut out = String::with_capacity(md.len() + 64);
    let mut last = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'!' && bytes[i + 1] == b'[' {
            // alt-текст до ']' , затем обязательная '(' и URL до ')'.
            if let Some(close_alt) = find_byte(bytes, i + 2, b']') {
                if bytes.get(close_alt + 1) == Some(&b'(') {
                    if let Some(close_paren) = find_byte(bytes, close_alt + 2, b')') {
                        let inner = &md[close_alt + 2..close_paren];
                        let (url, title) = split_url_title(inner);
                        if !url.is_empty() && !is_absolute_ref(url) {
                            out.push_str(&md[last..close_alt + 2]);
                            out.push_str(&resolve_hf_url(url, repo_id));
                            out.push_str(title);
                            out.push(')');
                            last = close_paren + 1;
                        }
                        i = close_paren + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out.push_str(&md[last..]);
    out
}

fn rewrite_html_img_src(md: &str, repo_id: &str) -> String {
    // ASCII-lowercase копия только для поиска тегов/атрибутов — позиции байт
    // совпадают с оригиналом, поэтому URL вырезаем из `md` (регистр пути цел).
    let lower = md.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out = String::with_capacity(md.len() + 64);
    let mut last = 0usize;
    let mut search = 0usize;
    while let Some(rel) = lower[search..].find("<img") {
        let tag_start = search + rel;
        let Some(gt) = lower[tag_start..].find('>') else { break };
        let tag_end = tag_start + gt;
        let tag = &lower[tag_start..tag_end];
        // Первый `src=`, перед которым пробел — отсекаем `data-src=`/`srcset=`.
        let mut from = 0usize;
        let mut src_at = None;
        while let Some(p) = tag[from..].find("src=") {
            let abs = p + from;
            if abs > 0 && tag.as_bytes()[abs - 1].is_ascii_whitespace() {
                src_at = Some(tag_start + abs);
                break;
            }
            from = abs + 4;
        }
        if let Some(src_pos) = src_at {
            let val_start = src_pos + 4;
            let q = lb.get(val_start).copied();
            if q == Some(b'"') || q == Some(b'\'') {
                let q = q.unwrap();
                if let Some(qe) = lower[val_start + 1..tag_end].find(q as char) {
                    let url_start = val_start + 1;
                    let url_end = url_start + qe;
                    let url = &md[url_start..url_end];
                    if !url.is_empty() && !is_absolute_ref(url) {
                        out.push_str(&md[last..url_start]);
                        out.push_str(&resolve_hf_url(url, repo_id));
                        last = url_end;
                    }
                }
            }
        }
        search = tag_end + 1;
    }
    out.push_str(&md[last..]);
    out
}

/// Минимальный размер файла для сегментированной загрузки. Мельче — одно
/// соединение даёт меньше overhead'а на TLS-handshake'и.
const SEG_THRESHOLD: u64 = 8 * 1024 * 1024;

/// Зондирует размер и поддержку Range-запросов. Возвращает `(total, supports_range)`.
/// `total = 0` означает, что сервер не отдал `Content-Length`.
///
/// HEAD-зонду нельзя доверять для проверки Range: HF Hub отвечает 302 на
/// CDN, не все CDN'ы возвращают `Accept-Ranges: bytes` в HEAD-ответах
/// (но при этом нормально обслуживают Range на GET). Поэтому делаем
/// `GET Range: bytes=0-0` — если возвращает 206 Partial Content и
/// `Content-Range: bytes 0-0/N` — Range гарантированно поддерживается,
/// и из `Content-Range` достаём настоящий `total` (а не размер 1-байтного куска).
pub async fn probe_file(repo_id: &str, filename: &str) -> Result<(u64, bool), HfError> {
    let url = format!("{API_HOST}/{}/resolve/main/{}", repo_id, filename);
    let resp = authed_get(&url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(HfError::Status(status.as_u16(), body));
    }
    let ranges_supported = status.as_u16() == 206;
    // 206 → Content-Range: bytes 0-0/TOTAL.  200 → Content-Length=TOTAL.
    let total = if ranges_supported {
        resp.headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        resp.content_length().unwrap_or(0)
    };
    Ok((total, ranges_supported))
}

fn part_path_for(dest: &Path) -> PathBuf {
    dest.with_extension(match dest.extension() {
        Some(e) => format!("{}.part", e.to_string_lossy()),
        None => "part".into(),
    })
}

fn meta_path_for(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_os_string();
    s.push(".meta");
    PathBuf::from(s)
}

/// Sidecar формат `.part.meta`. Versioned — при breaking change'е bump'аем
/// `version` и при mismatch ignore + truncate.
#[derive(serde::Serialize, serde::Deserialize)]
struct PartMeta {
    version: u8,
    total: u64,
    segments: Vec<PartMetaSegment>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct PartMetaSegment {
    from: u64,
    to: u64,
    bytes_done: u64,
}

const META_VERSION: u8 = 1;

fn try_load_meta(path: &Path, expected_total: u64, expected_segments: usize) -> Option<Vec<PartMetaSegment>> {
    let content = std::fs::read_to_string(path).ok()?;
    let meta: PartMeta = serde_json::from_str(&content).ok()?;
    if meta.version != META_VERSION
        || meta.total != expected_total
        || meta.segments.len() != expected_segments
    {
        return None;
    }
    // Каждый bytes_done должен укладываться в свой диапазон [from..=to].
    for s in &meta.segments {
        let span = s.to.saturating_sub(s.from).saturating_add(1);
        if s.bytes_done > span {
            return None;
        }
    }
    Some(meta.segments)
}

fn meta_json(total: u64, segments: &[SegmentProgress]) -> Option<String> {
    let meta = PartMeta {
        version: META_VERSION,
        total,
        segments: segments
            .iter()
            .map(|s| PartMetaSegment { from: s.from, to: s.to, bytes_done: s.bytes_done })
            .collect(),
    };
    serde_json::to_string(&meta).ok()
}

/// Sync-write meta — используется на initial-старте сегментированной загрузки,
/// когда нам нужен sidecar до того, как первый chunk прилетит.
fn write_meta_blocking(path: &Path, total: u64, segments: &[SegmentProgress]) {
    let Some(json) = meta_json(total, segments) else { return };
    let _ = std::fs::write(path, json);
}

/// Async-write meta — используется в throttled-flush после `update_speed`.
/// Запись идёт через `tokio::fs::write` (под капотом — `spawn_blocking`
/// тот же, но на tokio thread-pool, без создания нового OS-thread'а на
/// каждое обновление).
async fn write_meta_async(path: PathBuf, total: u64, segments: Vec<SegmentProgress>) {
    let Some(json) = meta_json(total, &segments) else { return };
    let _ = tokio::fs::write(&path, json).await;
}

/// Главная точка входа. Выбирает single-stream или segmented в зависимости
/// от `probe_file` + `n_segments`. Файл пишется в `dest.part` и атомарно
/// переименовывается по успешному завершению — частичный `.part` остаётся
/// при крэше для отладки и не путается с готовыми файлами.
pub async fn download_file(
    repo_id: &str,
    filename: &str,
    dest: &Path,
    key: String,
    downloads_sig: RwSignal<HashMap<String, DownloadState>>,
    n_segments: u32,
    ctl: super::control::DlControl,
) -> Result<(), HfError> {
    let n_segments = n_segments.clamp(1, 16);
    let (total, ranges) = match probe_file(repo_id, filename).await {
        Ok(p) => p,
        // Если HEAD не пускают (некоторые CDN'ы) — fallback на streaming GET без сегментов.
        Err(_) => (0u64, false),
    };

    if n_segments > 1 && ranges && total >= SEG_THRESHOLD {
        download_segmented(repo_id, filename, dest, key, downloads_sig, total, n_segments, ctl).await
    } else {
        download_single(repo_id, filename, dest, key, downloads_sig, total, ctl).await
    }
}

/// Прочитать сигнал управления и превратить непустой в `HfError::Aborted`.
/// `RUN` → `Ok(())`. Вызывается в начале каждой итерации chunk-loop'а.
fn check_abort(ctl: &super::control::DlControl) -> Result<(), HfError> {
    match ctl.signal() {
        super::control::PAUSE => Err(HfError::Aborted(AbortKind::Pause)),
        super::control::STOP => Err(HfError::Aborted(AbortKind::Stop)),
        super::control::CANCEL => Err(HfError::Aborted(AbortKind::Cancel)),
        super::control::GLOBAL_PAUSE => Err(HfError::Aborted(AbortKind::GlobalPause)),
        _ => Ok(()),
    }
}

async fn download_single(
    repo_id: &str,
    filename: &str,
    dest: &Path,
    key: String,
    downloads_sig: RwSignal<HashMap<String, DownloadState>>,
    known_total: u64,
    ctl: super::control::DlControl,
) -> Result<(), HfError> {
    let url = format!("{API_HOST}/{}/resolve/main/{}", repo_id, filename);
    let part_path = part_path_for(dest);

    // Если на диске лежит частично скачанный `.part` — пробуем докачать
    // через `Range: bytes=N-`. Если сервер не поддерживает Range и вернёт
    // 200 OK с полным телом — стартуем заново (truncate). Если ответил 416
    // (Range Not Satisfiable) — у нас файл уже целиком, переименовываем.
    let existing: u64 = std::fs::metadata(&part_path)
        .map(|m| if m.is_file() { m.len() } else { 0 })
        .unwrap_or(0);

    let mut req = authed_get(&url);
    if existing > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-", existing));
    }
    let resp = req.send().await?;
    let status = resp.status();
    // 416 → файл уже полный; rename и выходим.
    if status.as_u16() == 416 && existing > 0 {
        std::fs::rename(&part_path, dest)?;
        return Ok(());
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(HfError::Status(status.as_u16(), body));
    }
    let resumed = status.as_u16() == 206;
    // content-length у 206 — это длина оставшегося куска, не полная.
    let total = if known_total > 0 {
        known_total
    } else if resumed {
        existing.saturating_add(resp.content_length().unwrap_or(0))
    } else {
        resp.content_length().unwrap_or(0)
    };
    if total > 0 {
        let key_t = key.clone();
        run_on_main_thread(move || {
            downloads_sig.update(|m| {
                if let Some(d) = m.get_mut(&key_t) {
                    d.total = total;
                }
            });
        });
    }

    // Откроем файл: если сервер согласился отдать диапазон — append,
    // иначе создаём заново (могли натолкнуться на 200 OK без поддержки Range).
    let (mut file, start_bytes) = if resumed {
        let f = OpenOptions::new().append(true).open(&part_path)?;
        (f, existing)
    } else {
        let f = File::create(&part_path)?;
        (f, 0)
    };
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = start_bytes;
    // Сразу прокинем стартовый прогресс, чтобы UI показал «уже X% есть».
    if start_bytes > 0 {
        let key_s = key.clone();
        let done = start_bytes;
        run_on_main_thread(move || {
            downloads_sig.update(|m| {
                if let Some(d) = m.get_mut(&key_s) {
                    d.bytes_done = done;
                }
            });
        });
    }
    let mut last_flush = std::time::Instant::now();
    while let Some(chunk_res) = stream.next().await {
        check_abort(&ctl)?;
        let chunk = chunk_res?;
        file.write_all(&chunk)?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        super::rate::throttle(chunk.len() as u64).await;
        let now = std::time::Instant::now();
        if now.duration_since(last_flush).as_millis() < PROGRESS_FLUSH_MS {
            continue;
        }
        last_flush = now;
        let key_c = key.clone();
        let done = downloaded;
        run_on_main_thread(move || {
            downloads_sig.update(|m| {
                if let Some(d) = m.get_mut(&key_c) {
                    d.bytes_done = done;
                    update_speed(d);
                }
            });
        });
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&part_path, dest)?;
    let key_c = key.clone();
    let done = downloaded;
    run_on_main_thread(move || {
        downloads_sig.update(|m| {
            if let Some(d) = m.get_mut(&key_c) {
                d.bytes_done = done;
                update_speed(d);
            }
        });
    });
    Ok(())
}

/// Качаем файл несколькими параллельными HTTP-Range-запросами. Целевая
/// `.part`-копия предварительно расширяется до `total` через `set_len`,
/// каждая задача пишет в свой offset через `pwrite` (`write_at`) — порядок
/// записей не важен, mutex'а нет. По завершению всех задач — rename.
async fn download_segmented(
    repo_id: &str,
    filename: &str,
    dest: &Path,
    key: String,
    downloads_sig: RwSignal<HashMap<String, DownloadState>>,
    total: u64,
    n_segments: u32,
    ctl: super::control::DlControl,
) -> Result<(), HfError> {
    let url = format!("{API_HOST}/{}/resolve/main/{}", repo_id, filename);
    let part_path = part_path_for(dest);
    let meta_path = meta_path_for(&part_path);

    // Подготовим список диапазонов [from..=to]. Последний диапазон забирает
    // остаток (total % n_segments).
    let seg_size = total / n_segments as u64;
    let mut ranges: Vec<(u64, u64)> = Vec::with_capacity(n_segments as usize);
    for i in 0..n_segments as u64 {
        let from = i * seg_size;
        let to = if i + 1 == n_segments as u64 {
            total - 1
        } else {
            (i + 1) * seg_size - 1
        };
        ranges.push((from, to));
    }

    // Resume через sidecar `.part.meta`: если файл существует, не совпадает
    // total/n_segments или повреждён — игнорируем и стартуем заново.
    let resume = if part_path.exists() {
        try_load_meta(&meta_path, total, n_segments as usize)
    } else {
        None
    };

    let file = if resume.is_some() {
        // .part уже preallocated с прошлого запуска; просто открываем на запись.
        OpenOptions::new().write(true).open(&part_path)?
    } else {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part_path)?;
        f.set_len(total)?;
        f
    };
    let file = Arc::new(file);

    // Сегменты со стартовым `bytes_done` (0 для fresh, ранее накопленные
    // байты — для resume).
    let segments_init: Vec<SegmentProgress> = ranges
        .iter()
        .enumerate()
        .map(|(idx, (from, to))| {
            let done = resume
                .as_ref()
                .and_then(|segs| segs.get(idx))
                .map(|s| s.bytes_done)
                .unwrap_or(0);
            SegmentProgress { from: *from, to: *to, bytes_done: done }
        })
        .collect();
    {
        let key_t = key.clone();
        let segs = segments_init.clone();
        run_on_main_thread(move || {
            downloads_sig.update(|m| {
                if let Some(d) = m.get_mut(&key_t) {
                    d.total = total;
                    d.bytes_done = segs.iter().map(|s| s.bytes_done).sum();
                    d.segments = segs;
                }
            });
        });
    }

    // Если это новый старт — записать meta initial. Иначе оставить как есть
    // (он уже соответствует resume-данным).
    if resume.is_none() {
        write_meta_blocking(&meta_path, total, &segments_init);
    }

    // Общий по файлу гейт флэшей: `(время последнего флэша, done по сегментам)`.
    // Каждый сегмент пишет свой done в общий Vec, но в UI флэшит лишь если с
    // прошлого флэша ЛЮБОГО сегмента прошло ≥ PROGRESS_FLUSH_MS. Так суммарная
    // частота downloads_sig.update держится ~8/с НА ФАЙЛ (а не ×N сегментов) и
    // остаётся ниже кадровой — большинство кадров не пересобирают список файлов.
    let flush = Arc::new(std::sync::Mutex::new((
        std::time::Instant::now(),
        segments_init.iter().map(|s| s.bytes_done).collect::<Vec<u64>>(),
    )));

    let tasks = segments_init
        .iter()
        .enumerate()
        .map(|(idx, seg)| {
            let url = url.clone();
            let key = key.clone();
            let file = file.clone();
            let from = seg.from;
            let to = seg.to;
            let start_done = seg.bytes_done;
            let meta_path = meta_path.clone();
            let ctl = ctl.clone();
            let flush = flush.clone();
            async move {
                // Если сегмент уже скачан полностью (resume завершённого) —
                // ничего не делаем.
                let span = to.saturating_sub(from).saturating_add(1);
                if start_done >= span {
                    return Ok::<(), HfError>(());
                }
                let start_offset = from.saturating_add(start_done);
                let mut req = authed_get(&url);
                req = req.header(
                    reqwest::header::RANGE,
                    format!("bytes={}-{}", start_offset, to),
                );
                let resp = req.send().await?;
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(HfError::Status(status.as_u16(), body));
                }
                // Range НЕ поддержан, сервер отдал 200 + полное тело.
                // Только сегмент с idx=0 имеет право качаться целиком;
                // остальные обязаны прерваться, иначе мы запишем 4×total
                // на диск с гонкой за write_at. Возвращаем ошибку — retry
                // или верхний слой решат, что делать.
                if status.as_u16() == 200 && (start_offset != 0 || to + 1 != total) {
                    return Err(HfError::Status(
                        200,
                        tr!("hf.error.range_not_supported", start = start_offset, end = to),
                    ));
                }
                let mut stream = resp.bytes_stream();
                let mut offset = start_offset;
                let mut done_in_seg: u64 = start_done;
                while let Some(chunk_res) = stream.next().await {
                    check_abort(&ctl)?;
                    let chunk = chunk_res?;
                    let bytes: &[u8] = &chunk;
                    file.write_at(bytes, offset).map_err(HfError::Io)?;
                    let n = bytes.len() as u64;
                    offset = offset.saturating_add(n);
                    done_in_seg = done_in_seg.saturating_add(n);
                    super::rate::throttle(n).await;
                    // Записываем свой done в общий Vec и снимаем снимок для флэша,
                    // только если гейт открыт. Лок держим микросекунды, НИКОГДА
                    // через await (run_on_main_thread лишь ставит задачу в очередь).
                    let snapshot = {
                        let mut g = flush.lock().unwrap();
                        if let Some(slot) = g.1.get_mut(idx) {
                            *slot = done_in_seg;
                        }
                        let now = std::time::Instant::now();
                        if now.duration_since(g.0).as_millis() >= PROGRESS_FLUSH_MS {
                            g.0 = now;
                            Some(g.1.clone())
                        } else {
                            None
                        }
                    };
                    if let Some(dones) = snapshot {
                        let key_c = key.clone();
                        let meta_path_c = meta_path.clone();
                        run_on_main_thread(move || {
                            push_segment_progress(downloads_sig, &key_c, &dones, Some(&meta_path_c));
                        });
                    }
                }
                // Финальный флаш сегмента: гарантирует точный done даже если
                // последний чанк не попал в окно гейта.
                let dones = {
                    let mut g = flush.lock().unwrap();
                    if let Some(slot) = g.1.get_mut(idx) {
                        *slot = done_in_seg;
                    }
                    g.1.clone()
                };
                let key_f = key.clone();
                run_on_main_thread(move || {
                    push_segment_progress(downloads_sig, &key_f, &dones, None);
                });
                Ok::<(), HfError>(())
            }
        })
        .collect::<Vec<_>>();

    // Запустить все параллельно через try_join_all — при первой ошибке
    // (Err) join вернёт её и остальные продолжат до отмены. Sidecar
    // остаётся на диске, чтобы следующая попытка резюмировала с этого места.
    futures_util::future::try_join_all(tasks).await?;

    // Все сегменты завершены — финальный sync meta, drop файла, rename и
    // удаление meta-sidecar'а. Порядок важен: meta удаляем после rename,
    // чтобы при сбое строго между ними мы могли восстановиться.
    drop(file);
    std::fs::rename(&part_path, dest)?;
    let _ = std::fs::remove_file(&meta_path);
    Ok(())
}

/// Удалить частичные артефакты закачки (`.part` + `.part.meta`). Используется
/// при Cancel — Paused/Stopped, наоборот, сохраняют их для резюма.
pub fn remove_partial(dest: &Path) {
    let part = part_path_for(dest);
    let _ = std::fs::remove_file(&part);
    let _ = std::fs::remove_file(meta_path_for(&part));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Пробелы перед URL не сдвигают хвост: раньше отступ вычитался дважды,
    /// и `![x](   a b)` паниковал срезом за концом строки.
    #[test]
    fn split_url_title_with_leading_spaces() {
        assert_eq!(split_url_title("   a.png \"t\""), ("a.png", " \"t\""));
        assert_eq!(split_url_title("   a b"), ("a", " b"));
        assert_eq!(split_url_title("a.png"), ("a.png", ""));
    }

    const RID: &str = "org/model";
    const BASE: &str = "https://huggingface.co/org/model/resolve/main";

    #[test]
    fn markdown_relative_image_rewritten() {
        let md = "![cmp](comparison.png)";
        assert_eq!(
            rewrite_readme_image_urls(md, RID),
            format!("![cmp]({BASE}/comparison.png)")
        );
    }

    #[test]
    fn markdown_dotslash_and_subdir() {
        assert_eq!(
            rewrite_readme_image_urls("![a](./assets/x.png)", RID),
            format!("![a]({BASE}/assets/x.png)")
        );
        assert_eq!(
            rewrite_readme_image_urls("![a](images/y.jpg)", RID),
            format!("![a]({BASE}/images/y.jpg)")
        );
    }

    #[test]
    fn markdown_title_preserved() {
        assert_eq!(
            rewrite_readme_image_urls("![a](fig.png \"caption\")", RID),
            format!("![a]({BASE}/fig.png \"caption\")")
        );
    }

    #[test]
    fn absolute_and_data_left_alone() {
        for md in [
            "![a](https://x.dev/y.png)",
            "![a](http://x.dev/y.png)",
            "![a](data:image/png;base64,AAAA)",
            "![a](//cdn.dev/y.png)",
        ] {
            assert_eq!(rewrite_readme_image_urls(md, RID), md);
        }
    }

    #[test]
    fn non_image_link_untouched() {
        let md = "[text](docs.html)";
        assert_eq!(rewrite_readme_image_urls(md, RID), md);
    }

    #[test]
    fn linked_image_rewrites_only_image() {
        let md = "[![logo](logo.png)](https://site.dev)";
        assert_eq!(
            rewrite_readme_image_urls(md, RID),
            format!("[![logo]({BASE}/logo.png)](https://site.dev)")
        );
    }

    #[test]
    fn html_img_relative_rewritten() {
        assert_eq!(
            rewrite_readme_image_urls("<img src=\"banner.png\">", RID),
            format!("<img src=\"{BASE}/banner.png\">")
        );
        assert_eq!(
            rewrite_readme_image_urls("<img alt='x' src='pics/a.png' width=\"40\">", RID),
            format!("<img alt='x' src='{BASE}/pics/a.png' width=\"40\">")
        );
    }

    #[test]
    fn html_img_absolute_left_alone() {
        let md = "<img src=\"https://cdn.dev/a.png\">";
        assert_eq!(rewrite_readme_image_urls(md, RID), md);
    }

    #[test]
    fn html_data_src_not_mistaken() {
        // `data-src=` не должен перехватываться вместо настоящего `src=`.
        let md = "<img data-src=\"lazy.png\" src=\"real.png\">";
        assert_eq!(
            rewrite_readme_image_urls(md, RID),
            format!("<img data-src=\"lazy.png\" src=\"{BASE}/real.png\">")
        );
    }

    #[test]
    fn unicode_alt_text_safe() {
        let md = "![Описание модели](граф.png)";
        assert_eq!(
            rewrite_readme_image_urls(md, RID),
            format!("![Описание модели]({BASE}/граф.png)")
        );
    }

    #[test]
    fn multiple_images() {
        let md = "![a](one.png) text ![b](two.png)";
        assert_eq!(
            rewrite_readme_image_urls(md, RID),
            format!("![a]({BASE}/one.png) text ![b]({BASE}/two.png)")
        );
    }
}
