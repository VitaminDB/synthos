//! Инструмент `generate` — картинка, видео, музыка или речь напрямую через
//! службу нейросетей synaptixd, без графов редактора нод.
//!
//! Один вызов — один запрос к службе (`Image`, `Video`, `Music`, `Speak`):
//! пустая модель — модель по умолчанию этого вида из настроек службы.
//! Видеопамятью между моделью чата и генератором распоряжается служба
//! (модель чата живёт там же и уступает карту), поэтому выгружать LLM
//! перед генерацией agent-loop'у не нужно.
//!
//! Входные файлы (исходная картинка, кадры, референсы, образец голоса) —
//! вложения чата по ссылке `attachment:<имя|sha-префикс|last>` или путь на
//! диске; служба читает их сама.
//!
//! Результат пишется в `~/.local/share/synthos/outputs/generate/` и в
//! основном чате прикладывается CAS-вложением (`syn_chat::attach::ingest`) —
//! картинка, видео или звук показываются в ленте. Цикл чата исполняет
//! инструмент сам ([`run_chat`]): ему нужны живой ход генерации на карточке
//! и отмена. Через [`run`] (`tools::execute`, субагент) — тот же запрос, но
//! в ответе только путь к файлу.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;
use syngui::async_runtime::run_on_main_thread;
use syngui::tr;

use crate::agent::state::{AttachmentKind, MsgAttachment};
use crate::syn_chat::attach::{blobs, ingest};

use super::ToolError;

/// Вид генерации.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Music,
    Speech,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "image" | "picture" | "img" => Kind::Image,
            "video" => Kind::Video,
            "music" | "song" => Kind::Music,
            "speech" | "tts" | "speak" | "voice" => Kind::Speech,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Music => "music",
            Kind::Speech => "speech",
        }
    }

    fn model_kind(self) -> synaptixd::ModelKind {
        match self {
            Kind::Image => synaptixd::ModelKind::Image,
            Kind::Video => synaptixd::ModelKind::Video,
            Kind::Music => synaptixd::ModelKind::Music,
            Kind::Speech => synaptixd::ModelKind::Tts,
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Kind::Image => "png",
            Kind::Video => "mp4",
            Kind::Music | Kind::Speech => "wav",
        }
    }
}

/// Отмена идущей генерации из другого потока: закрывает соединение со
/// службой — служба видит обрыв и прерывает работу.
#[derive(Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    conn: Arc<Mutex<Option<synaptixd::Canceller>>>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        if let Some(c) = self.conn.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            c.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    fn attach(&self, client: &synaptixd::Client) {
        let c = client.canceller();
        let mut slot = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        *slot = c;
        // Отмена пришла раньше соединения — оборвать сразу.
        if self.is_cancelled() {
            if let Some(c) = slot.as_ref() {
                c.cancel();
            }
        }
    }
}

/// Куда писать живой ход генерации (строки с `\r` перерисовывают текущую).
pub type Progress = Arc<dyn Fn(&str) + Send + Sync>;

/// Разобранный вызов.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub kind: Option<Kind>,
    pub list_models: bool,
    pub prompt: String,
    pub model: String,
    pub negative: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub seconds: Option<f64>,
    pub steps: Option<usize>,
    pub guidance: Option<f32>,
    pub seed: Option<u64>,
    pub strength: Option<f32>,
    // Входные файлы — ссылки как их дала модель; пути — после resolve.
    pub init_image: Option<String>,
    pub refs: Vec<String>,
    pub first_frame: Option<String>,
    pub last_frame: Option<String>,
    pub audio: Option<String>,
    pub two_stage: bool,
    pub lyrics: String,
    pub voice: Option<String>,
    pub speaker: Option<String>,
    pub language: Option<String>,
    pub speed: Option<f32>,
    pub voice_sample: Option<String>,
    pub voice_sample_text: Option<String>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}

fn num(v: &Value, k: &str) -> Option<f64> {
    let x = v.get(k)?;
    x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
}

fn flag(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => false,
    }
}

/// Список строк: массив, одна строка или JSON-массив строкой.
fn list(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        Some(Value::String(s)) if s.trim_start().starts_with('[') => {
            serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
        }
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

pub fn parse(args_json: &str) -> Result<Request, ToolError> {
    let v: Value = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let kind = match s(&v, "kind") {
        Some(k) => Some(Kind::parse(&k).ok_or_else(|| {
            ToolError::BadArgs(format!("unknown kind \"{k}\" (image | video | music | speech)"))
        })?),
        None => None,
    };
    let list_models = flag(&v, "list_models");
    let prompt = s(&v, "prompt").unwrap_or_default();
    if kind.is_none() && !list_models {
        return Err(ToolError::MissingField("kind"));
    }
    if prompt.is_empty() && !list_models {
        return Err(ToolError::MissingField("prompt"));
    }
    let usize_of = |k: &str| num(&v, k).filter(|x| *x > 0.0).map(|x| x.round() as usize);
    Ok(Request {
        kind,
        list_models,
        prompt,
        model: s(&v, "model").unwrap_or_default(),
        negative: s(&v, "negative"),
        width: usize_of("width"),
        height: usize_of("height"),
        seconds: num(&v, "seconds").filter(|x| *x > 0.0),
        steps: usize_of("steps"),
        guidance: num(&v, "guidance").map(|x| x as f32),
        seed: num(&v, "seed").filter(|x| *x >= 0.0).map(|x| x as u64),
        strength: num(&v, "strength").map(|x| x.clamp(0.0, 1.0) as f32),
        init_image: s(&v, "init_image"),
        refs: list(&v, "refs"),
        first_frame: s(&v, "first_frame"),
        last_frame: s(&v, "last_frame"),
        audio: s(&v, "audio"),
        two_stage: flag(&v, "two_stage"),
        lyrics: s(&v, "lyrics").unwrap_or_default(),
        voice: s(&v, "voice"),
        speaker: s(&v, "speaker"),
        language: s(&v, "language"),
        speed: num(&v, "speed").filter(|x| *x > 0.0).map(|x| x as f32),
        voice_sample: s(&v, "voice_sample"),
        voice_sample_text: s(&v, "voice_sample_text"),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Вложения чата
// ─────────────────────────────────────────────────────────────────────────────

/// Вложение чата по ссылке: имя файла (без учёта регистра), префикс sha256
/// (от 6 hex) или `last` — самое свежее.
pub fn find_attachment_in(atts: &[MsgAttachment], r: &str) -> Option<MsgAttachment> {
    let r = r.trim().to_lowercase();
    if r == "last" || r == "latest" {
        return atts.last().cloned();
    }
    if let Some(a) = atts.iter().rev().find(|a| a.original_name.to_lowercase() == r) {
        return Some(a.clone());
    }
    if r.len() >= 6 && r.chars().all(|c| c.is_ascii_hexdigit()) {
        return atts.iter().rev().find(|a| a.sha256.starts_with(&r)).cloned();
    }
    None
}

/// Вложения чата текущего хода (main thread). Без контекста чата — пусто.
pub fn chat_attachments() -> Vec<MsgAttachment> {
    syngui::context_provider::try_use_context::<crate::syn_chat::SynChatCtx>()
        .map(|c| c.turn_chat_attachments())
        .unwrap_or_default()
}

/// [`find_attachment_in`] по вложениям чата текущего хода (main thread).
pub fn find_attachment(r: &str) -> Option<MsgAttachment> {
    find_attachment_in(&chat_attachments(), r)
}

async fn chat_attachments_async() -> Vec<MsgAttachment> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let _ = tx.send(chat_attachments());
    });
    rx.await.unwrap_or_default()
}

/// Путь к файлу, который читает служба: копия «под модель» у картинок и
/// звука (PNG, WAV), оригинал у видео.
fn service_path(a: &MsgAttachment) -> PathBuf {
    match a.kind {
        AttachmentKind::Video => blobs::source_path(a),
        _ => blobs::model_path(a),
    }
}

/// Ссылка модели → путь: `attachment:<ref>`, голая ссылка на вложение или
/// существующий файл на диске.
fn resolve_ref(r: &str, atts: &[MsgAttachment]) -> Result<String, String> {
    let bare = r.strip_prefix("attachment:").unwrap_or(r).trim();
    if !r.starts_with("attachment:") {
        let p = crate::paths::expand_home(bare);
        if p.is_absolute() && p.is_file() {
            return Ok(p.display().to_string());
        }
    }
    match find_attachment_in(atts, bare) {
        Some(a) => Ok(service_path(&a).display().to_string()),
        None => {
            let names = if atts.is_empty() {
                tr!("chat.generate.attachments.none")
            } else {
                atts.iter()
                    .map(|a| format!("{} (sha:{})", a.original_name, &a.sha256[..12.min(a.sha256.len())]))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Err(tr!("chat.generate.error.attachment", reference = r, list = names))
        }
    }
}

/// Заменить ссылки запроса путями. Вложения читаются, только если ссылки есть.
async fn resolve_inputs(req: &mut Request) -> Result<(), String> {
    let any = req.init_image.is_some()
        || !req.refs.is_empty()
        || req.first_frame.is_some()
        || req.last_frame.is_some()
        || req.audio.is_some()
        || req.voice_sample.is_some();
    if !any {
        return Ok(());
    }
    let atts = chat_attachments_async().await;
    for slot in [
        &mut req.init_image,
        &mut req.first_frame,
        &mut req.last_frame,
        &mut req.audio,
        &mut req.voice_sample,
    ] {
        if let Some(r) = slot.as_mut() {
            *r = resolve_ref(r, &atts)?;
        }
    }
    for r in req.refs.iter_mut() {
        *r = resolve_ref(r, &atts)?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Запрос к службе
// ─────────────────────────────────────────────────────────────────────────────

/// Каталог результатов: `~/.local/share/synthos/outputs/generate/`.
pub fn outputs_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/synthos/outputs/generate")
}

fn out_path(kind: Kind) -> PathBuf {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    outputs_dir().join(format!("{}-{ms}.{}", kind.id(), kind.ext()))
}

/// Готовый результат генерации (файл на диске и строка-описание).
pub struct Generated {
    pub path: PathBuf,
    pub summary: String,
}

fn secs(t: Instant) -> String {
    format!("{:.1}", t.elapsed().as_secs_f32())
}

/// Модели вида в службе: строка для ответа модели.
fn models_text(c: &mut synaptixd::Client, kind: Kind) -> String {
    let mk = kind.model_kind();
    let default = c
        .defaults()
        .ok()
        .and_then(|d| d.into_iter().find(|x| x.kind == mk))
        .and_then(|d| d.resolved.or(d.model))
        .unwrap_or_else(|| "—".into());
    let ids: Vec<String> = c
        .list()
        .map(|l| l.into_iter().filter(|m| m.kind == mk).map(|m| m.id).collect())
        .unwrap_or_default();
    let list = if ids.is_empty() { tr!("chat.generate.attachments.none") } else { ids.join(", ") };
    tr!("chat.generate.models", kind = kind.id(), default = default, list = list)
}

fn write_wav(path: &Path, pcm: &[f32], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &x in pcm {
        w.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Синхронная генерация (рабочий поток): запрос к службе, запись файла.
fn generate_blocking(req: &Request, progress: Option<Progress>, cancel: &Cancel) -> Result<Generated, String> {
    let kind = req.kind.ok_or("kind")?;
    let mut c = synaptixd::Client::connect()
        .map_err(|e| tr!("chat.generate.error.service", error = e.to_string()))?;
    let _ = c.label("generate");
    cancel.attach(&c);
    if cancel.is_cancelled() {
        return Err(tr!("chat.generate.cancelled"));
    }
    // Пустая модель — по умолчанию этого вида: имя узнаём заранее, для хода
    // и для ответа (у речи служба своей подписи не возвращает).
    let model_label = if req.model.is_empty() {
        c.resolve(kind.model_kind())
            .map(|e| e.id)
            .unwrap_or_else(|_| tr!("chat.generate.default_model"))
    } else {
        req.model.clone()
    };
    if let Some(p) = &progress {
        p(&format!("{}\n", tr!("chat.generate.progress.start", kind = kind.id(), model = model_label)));
    }
    let t0 = Instant::now();
    let mut on_step = |step: usize, total: usize| {
        if let Some(p) = &progress {
            p(&format!("\r{}", tr!("chat.generate.progress.step", step = step, total = total, sec = secs(t0))));
        }
    };
    let path = out_path(kind);
    std::fs::create_dir_all(outputs_dir()).map_err(|e| tr!("chat.generate.error.write", path = path.display(), error = e))?;
    let write = |bytes: &[u8]| {
        std::fs::write(&path, bytes).map_err(|e| tr!("chat.generate.error.write", path = path.display(), error = e))
    };

    let result: Result<String, synaptixd::Error> = match kind {
        Kind::Image => {
            let p = synaptixd::ImageParams {
                negative: req.negative.clone(),
                width: req.width,
                height: req.height,
                steps: req.steps,
                guidance: req.guidance,
                seed: req.seed,
                images: req.refs.clone(),
                init: req.init_image.clone(),
                strength: req.strength,
            };
            c.image(&req.model, &req.prompt, &p, &mut on_step).and_then(|(r, png)| {
                write(&png).map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Failed, e))?;
                Ok(tr!(
                    "chat.generate.done.image",
                    width = r.width,
                    height = r.height,
                    label = r.label,
                    seed = r.seed,
                    sec = secs(t0)
                ))
            })
        }
        Kind::Video => {
            let p = synaptixd::VideoParams {
                negative: req.negative.clone(),
                width: req.width,
                height: req.height,
                seconds: req.seconds,
                steps: req.steps,
                guidance: req.guidance,
                seed: req.seed,
                first_frame: req.first_frame.clone(),
                last_frame: req.last_frame.clone(),
                refs: req.refs.clone(),
                audio: req.audio.clone(),
                two_stage: req.two_stage,
                strength: req.strength,
                ..Default::default()
            };
            c.video(&req.model, &req.prompt, &p, &mut on_step).and_then(|(r, mp4)| {
                write(&mp4).map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Failed, e))?;
                let dur = if r.fps > 0.0 { r.frames as f64 / r.fps } else { 0.0 };
                Ok(tr!(
                    "chat.generate.done.video",
                    width = r.width,
                    height = r.height,
                    seconds = format!("{dur:.1}"),
                    frames = r.frames,
                    fps = format!("{:.0}", r.fps),
                    label = r.label,
                    seed = r.seed,
                    sec = secs(t0)
                ))
            })
        }
        Kind::Music => {
            // Свой запрос, а не `Client::music`: тот не передаёт steps.
            let r = synaptixd::Request::Music {
                model: req.model.clone(),
                prompt: req.prompt.clone(),
                lyrics: req.lyrics.clone(),
                seconds: req.seconds.map(|s| s.round().max(1.0) as u32),
                seed: req.seed,
                steps: req.steps,
            };
            c.call_events(r, &[], |ev| {
                if let synaptixd::Event::Progress { step, total } = ev {
                    on_step(step, total);
                }
            })
            .and_then(|(v, fds)| {
                let reply: synaptixd::MusicReply = serde_json::from_value(v)
                    .map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Protocol, e.to_string()))?;
                let fd = fds.into_iter().next().ok_or_else(|| {
                    synaptixd::Error::new(synaptixd::ErrorCode::Protocol, "no fd with music")
                })?;
                let mut wav = Vec::with_capacity(reply.bytes);
                use std::io::Read;
                std::fs::File::from(fd)
                    .read_to_end(&mut wav)
                    .map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Failed, e.to_string()))?;
                write(&wav).map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Failed, e))?;
                Ok(tr!(
                    "chat.generate.done.music",
                    seconds = format!("{:.1}", reply.seconds),
                    label = reply.label,
                    seed = reply.seed,
                    sec = secs(t0)
                ))
            })
        }
        Kind::Speech => {
            let mut params = synaptixd::Params::new();
            if let Some(v) = &req.voice {
                params = params.with("voice", v.as_str());
            }
            if let Some(v) = &req.speaker {
                params = params.with("speaker", v.as_str());
            }
            if let Some(v) = &req.language {
                params = params.with("language", v.as_str());
            }
            if let Some(v) = req.speed {
                params = params.with("speed", v);
            }
            if let Some(v) = req.seed {
                params = params.with("seed", v as i64);
            }
            if let Some(v) = req.steps {
                params = params.with("steps", v as i64);
            }
            if let Some(v) = req.guidance {
                params = params.with("guidance", v);
            }
            if let Some(v) = &req.voice_sample {
                params = params.with("ref_audio", v.as_str());
                params = params.with("ref_text", req.voice_sample_text.as_deref().unwrap_or(""));
            }
            c.speak_with(&req.model, &req.prompt, &params).and_then(|sp| {
                let pcm = sp.audio.f32s();
                write_wav(&path, pcm, sp.sample_rate)
                    .map_err(|e| synaptixd::Error::new(synaptixd::ErrorCode::Failed, tr!("chat.generate.error.write", path = path.display(), error = e)))?;
                let seconds = pcm.len() as f32 / sp.sample_rate.max(1) as f32;
                let seed = sp.seed.map(|s| s.to_string()).unwrap_or_else(|| "—".into());
                Ok(tr!(
                    "chat.generate.done.speech",
                    seconds = format!("{seconds:.1}"),
                    label = model_label,
                    seed = seed,
                    sec = secs(t0)
                ))
            })
        }
    };

    if let Some(p) = &progress {
        p("\n");
    }
    match result {
        Ok(summary) => Ok(Generated { path, summary }),
        Err(_) if cancel.is_cancelled() => Err(tr!("chat.generate.cancelled")),
        Err(e) => {
            // Ошибка о модели — модели вида в ответ, чтобы агент выбрал
            // существующую, а не гадал. Соединение могло оборваться —
            // список берём новым.
            let mut msg = tr!("chat.generate.error.failed", error = e.to_string());
            if let Ok(mut c2) = synaptixd::Client::connect() {
                msg.push('\n');
                msg.push_str(&models_text(&mut c2, kind));
            }
            Err(msg)
        }
    }
}

/// Генерация целиком: ссылки → пути, запрос в рабочем потоке.
pub async fn generate(mut req: Request, progress: Option<Progress>, cancel: Cancel) -> Result<Generated, String> {
    resolve_inputs(&mut req).await?;
    tokio::task::spawn_blocking(move || generate_blocking(&req, progress, &cancel))
        .await
        .map_err(|e| e.to_string())?
}

async fn list_models(kind: Option<Kind>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut c = synaptixd::Client::connect()
            .map_err(|e| tr!("chat.generate.error.service", error = e.to_string()))?;
        let kinds = match kind {
            Some(k) => vec![k],
            None => vec![Kind::Image, Kind::Video, Kind::Music, Kind::Speech],
        };
        Ok(kinds.into_iter().map(|k| models_text(&mut c, k)).collect::<Vec<_>>().join("\n"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Итог вызова в основном чате.
pub struct ChatOutcome {
    pub content: String,
    pub error: bool,
    pub attachments: Vec<MsgAttachment>,
}

/// Исполнение в цикле основного чата: результат — вложение ленты.
pub async fn run_chat(args_json: &str, progress: Option<Progress>, cancel: Cancel) -> ChatOutcome {
    let fail = |content: String| ChatOutcome { content, error: true, attachments: Vec::new() };
    let req = match parse(args_json) {
        Ok(r) => r,
        Err(e) => return fail(e.to_string()),
    };
    if req.list_models {
        return match list_models(req.kind).await {
            Ok(t) => ChatOutcome { content: t, error: false, attachments: Vec::new() },
            Err(e) => fail(e),
        };
    }
    let g = match generate(req, progress, cancel).await {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let path = g.path.clone();
    let ingested = tokio::task::spawn_blocking(move || ingest::ingest(&path))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let pretty = crate::paths::pretty(&g.path);
    match ingested {
        Ok(a) => {
            let line = tr!(
                "chat.generate.file.attached",
                path = pretty,
                size = crate::models::human_bytes(a.size_bytes)
            );
            ChatOutcome { content: format!("{}\n{line}", g.summary), error: false, attachments: vec![a] }
        }
        Err(e) => ChatOutcome {
            content: format!("{}\n{}", g.summary, tr!("chat.generate.file.attach_failed", path = pretty, error = e)),
            error: false,
            attachments: Vec::new(),
        },
    }
}

/// Исполнение через `tools::execute` (субагент): файл на диске, путь в ответе.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let req = parse(args_json)?;
    if req.list_models {
        return list_models(req.kind).await.map_err(ToolError::Runtime);
    }
    let g = generate(req, None, Cancel::default()).await.map_err(ToolError::Runtime)?;
    Ok(format!("{}\n{}", g.summary, tr!("chat.generate.file.saved", path = g.path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_kinds_and_coerces_values() {
        let r = parse(r#"{"kind":"Image","prompt":"кот","width":"1024","seed":7,"refs":"attachment:last"}"#).unwrap();
        assert_eq!(r.kind, Some(Kind::Image));
        assert_eq!(r.width, Some(1024));
        assert_eq!(r.seed, Some(7));
        assert_eq!(r.refs, ["attachment:last"]);
        let r = parse(r#"{"kind":"tts","prompt":"Привет","speed":1.2}"#).unwrap();
        assert_eq!(r.kind, Some(Kind::Speech));
        assert_eq!(r.speed, Some(1.2));
        assert!(matches!(parse(r#"{"prompt":"x"}"#), Err(ToolError::MissingField("kind"))));
        assert!(matches!(parse(r#"{"kind":"image"}"#), Err(ToolError::MissingField("prompt"))));
        assert!(parse(r#"{"kind":"gif","prompt":"x"}"#).is_err());
        // Список моделей — без prompt и даже без kind.
        assert!(parse(r#"{"list_models":true}"#).unwrap().list_models);
    }

    fn att(name: &str, sha: &str) -> MsgAttachment {
        serde_json::from_value(serde_json::json!({
            "sha256": sha, "mime": "image/png", "original_name": name
        }))
        .unwrap()
    }

    #[test]
    fn attachments_found_by_name_sha_and_last() {
        let atts = vec![att("cat.png", "abcdef0123"), att("dog.jpg", "fedcba9876")];
        assert_eq!(find_attachment_in(&atts, "CAT.png").unwrap().sha256, "abcdef0123");
        assert_eq!(find_attachment_in(&atts, "fedcba").unwrap().original_name, "dog.jpg");
        assert_eq!(find_attachment_in(&atts, "last").unwrap().original_name, "dog.jpg");
        assert!(find_attachment_in(&atts, "abc").is_none());
        assert!(resolve_ref("attachment:nope.png", &atts).is_err());
    }
}
