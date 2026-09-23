//! Async-исполнители инструментов.
//!
//! Каждый исполнитель принимает сырые аргументы из `ChatToolCall`
//! (строка с JSON) и возвращает [`ToolOutcome`] — структурированный
//! результат, который удобно и показывать в UI, и отправлять обратно
//! в llama как `role=tool` сообщение.
//!
//! Безопасность и защита контекста:
//! - `run_bash` запускает `bash -lc "<cmd>"`. Потенциально опасно — защита
//!   на уровне UI-подтверждения (см. `components::tool_confirm`).
//! - Вывод обрезается до [`MAX_OUTPUT_BYTES`] по char-boundary, чтобы
//!   гигантские листинги не раздували контекст LLM.

use thiserror::Error;
use tokio::process::Command;

use crate::agent::schema::ChatToolCall;

use super::catalog::{
    KEY_AUTOSKILL, KEY_AUTOTOOLS, KEY_BASH, KEY_KB_SEARCH, KEY_NOTES, KEY_PIPELINES, KEY_SUBAGENT,
    KEY_SYSTEM, KEY_WEB,
};

/// Верхняя граница длины вывода одного инструмента (в байтах). Всё, что
/// длиннее, обрезается по char-boundary и помечается `…(truncated)`.
///
/// 64 KB — компромисс под tool `web` (action=read): после Readability+htmd
/// типичная Wikipedia/article-страница даёт 30-80 KB markdown'а. `bash`
/// и `kb_search` укладываются в эти границы с большим запасом.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Предел для `autoskill`: скил — это не выхлоп команды, а инструкция,
/// которую модель обязана выполнить целиком. Обрезка на середине забирает
/// ровно ту часть, ради которой скил и подключали (рукописный скил на
/// 90 KB терял четверть текста). Вчетверо больше общего предела хватает
/// на большой скил и всё ещё страхует от патологического файла.
pub const MAX_SKILL_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES * 4;

/// Предел для `notes`: инструмент сам держит ответ в окне модели — он
/// знает остаток контекста в токенах ([`budget`](super::budget)) и режет
/// пачку на границе страницы. Здесь остаётся предохранитель от патологии:
/// вчетверо выше общего, как у скилов.
pub const MAX_NOTES_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES * 4;

/// Инструменты чтения (`bash`, `web`): сколько поместится в окно, решает
/// бюджет хода ([`budget::fit`](super::budget::fit)) — сюда остаётся
/// предохранитель от `cat` по гигабайтному логу. Вчетверо выше общего: файл
/// или статья на 200 КБ при свободном окне в 100k токенов читается одним
/// вызовом, а не четырьмя.
pub const MAX_READ_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES * 4;

/// Предел вывода конкретного инструмента.
fn output_limit(tool: &str) -> usize {
    match tool {
        KEY_AUTOSKILL | KEY_AUTOTOOLS => MAX_SKILL_OUTPUT_BYTES,
        KEY_NOTES => MAX_NOTES_OUTPUT_BYTES,
        KEY_BASH | KEY_WEB => MAX_READ_OUTPUT_BYTES,
        _ => MAX_OUTPUT_BYTES,
    }
}

/// Укладывает ли цикл выхлоп инструмента в окно ([`budget::fit`]
/// (super::budget::fit)), и по какому потолку в токенах — когда живого окна
/// нет (вызов вне agent-loop). `None` — отдавать модели целиком.
///
/// Мера — токены по токенизатору модели, а не символы: прежний статический
/// клип в 8000 символов резал середину у каждого куска файла и отправлял
/// модель перечитывать вырезанное по кругу. Ужатая копия уходит и в ленту,
/// поэтому история, пересобранная из ленты на следующем сообщении,
/// совпадает с промптом хода и префикс-KV цел.
///
/// Два исключения. `autoskill` отдаёт не выхлоп, а инструкцию, которую
/// модель обязана выполнить целиком: вырезанная середина — ровно то знание,
/// ради которого скил и подключали. `notes` сам меряет ответ живым окном
/// (`notes::ReadBudget`) и обрывает его на границе страницы, называя
/// недочитанное, — укладка поверх этого только вырезала бы у честной пачки
/// середину. Обоим размер ограничивает потолок исполнителя. `autotools` — как
/// скил: схема с вырезанной серединой хуже, чем никакой.
pub fn history_limit(tool: &str) -> Option<usize> {
    match tool {
        KEY_AUTOSKILL | KEY_AUTOTOOLS | KEY_NOTES => None,
        _ => Some(super::budget::FALLBACK_RESULT_TOKENS),
    }
}

/// Ошибки парсинга/исполнения инструмента. Не панические — маппятся в
/// `ToolOutcome { error: true }` и уходят в LLM как обычный tool-result.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Unknown tool: {0}. Call one of: bash, kb_search, web, autoskill, autotools, subagent, system, pipelines, notes, wizard, view_media")]
    Unknown(String),
    #[error("Invalid arguments JSON: {0}")]
    BadArgs(String),
    /// JSON разобрался, но аргументы не сошлись: нет поля, неизвестный op,
    /// страница/карточка не нашлась. Без слова «JSON» — оно уводило модель
    /// (и пользователя) искать битый синтаксис там, где не хватало поля.
    #[error("Invalid arguments: {0}")]
    Args(String),
    #[error("Missing required field \"{0}\"")]
    MissingField(&'static str),
    #[error("Failed to spawn process: {0}")]
    Spawn(String),
    /// Инструмент запустился, но упал по ходу дела (у субагента — ошибка
    /// генерации). Текст уходит модели как есть: «Invalid arguments JSON»
    /// поверх OOM'а уводил её чинить аргументы вместо задачи.
    #[error("{0}")]
    Runtime(String),
}

/// Структурированный результат исполнения.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub tool_call_id: String,
    pub name: String,
    /// Текстовое содержимое (возможно усечённое). Уходит и в UI-bubble,
    /// и в `role=tool` сообщение серверу.
    pub content: String,
    /// `true` — результат неуспешен (non-zero exit, ошибка парсинга,
    /// неизвестный инструмент). Красится UI иначе.
    pub error: bool,
    /// `true` — до исполнения не дошло: вызов не прошёл разбор аргументов
    /// (битый JSON, нет обязательного поля, неизвестный инструмент).
    /// Agent-loop считает такие ходы подряд: модель, которая трижды не
    /// смогла собрать вызов, уже не соберёт его и на десятый раз.
    pub invalid_args: bool,
}

/// Диспетчер исполнения: читает `call.function.name` и вызывает
/// соответствующий обработчик.
///
/// Никогда не паникует — все ошибки конвертируются в `ToolOutcome` с
/// `error=true`.
fn trim_json_strings(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            let t = s.trim().to_string();
            *s = t;
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(trim_json_strings),
        serde_json::Value::Object(o) => o.values_mut().for_each(trim_json_strings),
        _ => {}
    }
}

pub fn normalize_args(raw: &str) -> String {
    let trimmed = raw.trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(mut v) => {
            trim_json_strings(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| trimmed.to_string())
        }
        Err(_) => trimmed.to_string(),
    }
}

/// Аргументы, приведённые к типам схемы инструмента: `"True"` в поле
/// `boolean` — `true`, `"30"` в `integer` — число. Локальная модель пишет
/// булево строкой (питоновское `True`), а строгий serde-разбор ронял вызов
/// на «invalid type: string». Поля, где схема разрешает строку, не
/// трогаются. Только для исполнения: в ленте и промпте остаются аргументы,
/// как их сгенерировала модель (префикс-KV).
pub fn coerce_args(tool: &str, args: &str) -> String {
    let Some(tool) = super::descriptor::Tool::by_key(tool) else { return args.to_string() };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(args) else { return args.to_string() };
    if !coerce_to_schema(&mut v, &tool.schema) {
        return args.to_string();
    }
    serde_json::to_string(&v).unwrap_or_else(|_| args.to_string())
}

/// Привести значение к JSON-схеме на месте (объекты — по `properties`,
/// массивы — по `items`); `true` — что-то поменялось.
pub fn coerce_to_schema(v: &mut serde_json::Value, schema: &serde_json::Value) -> bool {
    use serde_json::Value as Json;
    if let Json::Object(map) = v {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else { return false };
        let mut changed = false;
        for (k, val) in map.iter_mut() {
            if let Some(s) = props.get(k) {
                changed |= coerce_to_schema(val, s);
            }
        }
        return changed;
    }
    if let Json::Array(items) = v {
        let Some(s) = schema.get("items") else { return false };
        return items.iter_mut().fold(false, |changed, it| coerce_to_schema(it, s) | changed);
    }
    let types: Vec<&str> = match schema.get("type") {
        Some(Json::String(t)) => vec![t.as_str()],
        Some(Json::Array(a)) => a.iter().filter_map(|t| t.as_str()).collect(),
        _ => Vec::new(),
    };
    let allows = |t: &str| types.contains(&t);
    let next = match &*v {
        Json::String(s) if !types.is_empty() && !allows("string") => {
            let t = s.trim();
            let as_bool = || match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Some(Json::Bool(true)),
                "false" | "no" | "off" | "0" => Some(Json::Bool(false)),
                _ => None,
            };
            (if allows("boolean") { as_bool() } else { None })
                .or_else(|| if allows("integer") { t.parse::<i64>().ok().map(Json::from) } else { None })
                .or_else(|| if allows("number") { t.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Json::Number) } else { None })
        }
        Json::Number(n) if allows("boolean") && !allows("integer") && !allows("number") => match n.as_i64() {
            Some(0) => Some(Json::Bool(false)),
            Some(1) => Some(Json::Bool(true)),
            _ => None,
        },
        _ => None,
    };
    match next {
        Some(n) => {
            *v = n;
            true
        }
        None => false,
    }
}

/// Все ключи каталога — для канонизации имени вызова и для подсказки в
/// тексте ошибки о неизвестном инструменте.
pub(crate) const TOOL_KEYS: [&str; 11] = [
    KEY_BASH,
    KEY_KB_SEARCH,
    KEY_WEB,
    KEY_AUTOSKILL,
    KEY_AUTOTOOLS,
    KEY_SUBAGENT,
    KEY_SYSTEM,
    KEY_PIPELINES,
    KEY_NOTES,
    super::catalog::KEY_WIZARD,
    super::catalog::KEY_VIEW_MEDIA,
];

/// Приводит имя вызова к ключу каталога.
///
/// Канальные шаблоны (Muse Glimmer) объявляют инструменты пространствами
/// имён — chat-шаблон рендерит `# Valid recipients: "self", "notes.*",
/// "bash.*", "user".`, то есть сам разрешает модели любой суффикс внутри
/// пространства. Она этим пользуется: `to=notes.action`, `to=bash.run`,
/// а иногда пишет и `bash.bash` или harmony-стиль `functions.web`. Ключи
/// каталога плоские, поэтому ключом считаем первый сегмент имени, который
/// в каталоге есть, — 04.09.2026 `notes.action` уходил в «Unknown tool», и
/// ход умирал на трёх одинаковых вызовах подряд.
pub(crate) fn canonical_tool_name(name: &str) -> &str {
    if TOOL_KEYS.contains(&name) {
        return name;
    }
    name.split('.')
        .find(|seg| TOOL_KEYS.contains(seg))
        .unwrap_or(name)
}

pub async fn execute(call: &ChatToolCall) -> ToolOutcome {
    let name = call.function.name.clone().unwrap_or_default();
    let name = canonical_tool_name(&name).to_string();
    let raw_args = call.function.arguments.as_deref().unwrap_or("");
    let normalized = coerce_args(&name, &normalize_args(raw_args));
    let args = normalized.as_str();

    let result = match name.as_str() {
        KEY_BASH => run_bash(args).await,
        KEY_KB_SEARCH => super::kb_search::run(args).await,
        KEY_WEB => super::web::run(args).await,
        KEY_AUTOSKILL => super::autoskill::run(args).await,
        KEY_AUTOTOOLS => super::autotools::run(args).await,
        KEY_SUBAGENT => super::subagent::run(args).await,
        KEY_SYSTEM => super::system::run(args).await,
        KEY_PIPELINES => super::pipelines::run(args).await,
        KEY_NOTES => super::notes::run(args).await,
        super::catalog::KEY_WIZARD => super::wizard::run(args).await,
        // Основной чат исполняет его сам (`syn_chat::session`): нужна модель.
        super::catalog::KEY_VIEW_MEDIA => super::view_media::run(args).await,
        other => Err(ToolError::Unknown(other.to_string())),
    };

    match result {
        Ok(content) => ToolOutcome {
            tool_call_id: call.id.clone(),
            content: truncate_output(&content, output_limit(&name)),
            name,
            error: false,
            invalid_args: false,
        },
        Err(e) => ToolOutcome {
            tool_call_id: call.id.clone(),
            name,
            invalid_args: matches!(
                e,
                ToolError::BadArgs(_) | ToolError::Args(_) | ToolError::MissingField(_) | ToolError::Unknown(_)
            ),
            content: e.to_string(),
            error: true,
        },
    }
}

/// Запускает `bash -lc <cmd>`. Формат результата — человекочитаемый текст
/// с реальными `\n`, а не JSON-строка. Причина:
/// - JSON экранирует переводы строк (`\n` → `"\\n"`), и многокилобайтный
///   `stdout` складывается в одну длинную линию, которая рвёт layout bubble.
/// - LLM-ы стабильно понимают plain-текстовую «секцию» `--- stdout ---`.
pub async fn run_bash(args_json: &str) -> Result<String, ToolError> {
    let v: serde_json::Value = serde_json::from_str(args_json)
        .map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let command = v
        .get("command")
        .and_then(|x| x.as_str())
        .ok_or(ToolError::MissingField("command"))?;

    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Stop бросает этот future — процесс должен умереть вместе с ним,
        // а не остаться сиротой (`yes`, `tail -f`, сервер).
        .kill_on_drop(true);
    // Своя группа процессов: при отмене гасим и то, что bash запустил
    // (иначе потомок держит pipe, и чтение не кончается).
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| ToolError::Spawn(e.to_string()))?;
    let mut group = ProcessGroupGuard(child.id());
    let (stdout, stdout_cut) = read_capped(child.stdout.take(), MAX_READ_OUTPUT_BYTES);
    let (stderr, stderr_cut) = read_capped(child.stderr.take(), MAX_READ_OUTPUT_BYTES);
    // Оба потока — одновременно: иначе процесс, заполнивший pipe stderr,
    // встанет, пока мы ждём конца stdout.
    let (stdout, stderr) = tokio::join!(stdout, stderr);
    let status = child.wait().await.map_err(|e| ToolError::Spawn(e.to_string()))?;
    // Команда завершилась сама — её фоновые потомки (`cmd &`) живут дальше,
    // как раньше; группу гасим только при отмене.
    group.0 = None;
    let stdout = with_cut_note(String::from_utf8_lossy(&stdout).into_owned(), &stdout_cut);
    let stderr = with_cut_note(String::from_utf8_lossy(&stderr).into_owned(), &stderr_cut);
    let exit_code = status.code().unwrap_or(-1);

    let mut out = String::new();
    out.push_str(&format!("$ {}\n", command));
    out.push_str(&format!("exit: {}\n", exit_code));
    if !stdout.is_empty() {
        out.push_str("--- stdout ---\n");
        out.push_str(&stdout);
        if !stdout.ends_with('\n') {
            out.push('\n');
        }
    }
    if !stderr.is_empty() {
        out.push_str("--- stderr ---\n");
        out.push_str(&stderr);
        if !stderr.ends_with('\n') {
            out.push('\n');
        }
    }
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("(no output)\n");
    }
    Ok(out)
}

/// При drop (отмена хода посреди команды) убивает группу процессов команды
/// `bash` вместе с потомками. После нормального завершения разоружается.
struct ProcessGroupGuard(Option<u32>);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: killpg — обычный syscall; pgid = pid лидера группы,
            // созданной `process_group(0)`.
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// Читает поток до конца, храня не больше `cap` байт: остальное вычитывается
/// и выбрасывается (иначе процесс встанет на полном pipe), а счётчик
/// выброшенного лежит во втором значении. Без потолка `yes` или бесконечный
/// лог копились в памяти целиком до конца процесса.
fn read_capped<R>(
    stream: Option<R>,
    cap: usize,
) -> (impl std::future::Future<Output = Vec<u8>>, std::sync::Arc<std::sync::atomic::AtomicU64>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = dropped.clone();
    let fut = async move {
        let mut kept = Vec::new();
        let Some(mut stream) = stream else { return kept };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len()).min(n);
                    kept.extend_from_slice(&buf[..room]);
                    counter.fetch_add((n - room) as u64, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        kept
    };
    (fut, dropped)
}

fn with_cut_note(mut text: String, dropped: &std::sync::atomic::AtomicU64) -> String {
    let n = dropped.load(std::sync::atomic::Ordering::Relaxed);
    if n > 0 {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("…(truncated {n} bytes)\n"));
    }
    text
}

/// Обрезает строку до `limit` по char-boundary. Если укладывается —
/// возвращает как есть. Если нет — усечение + `…(truncated N bytes)`.
fn truncate_output(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = s.len() - end;
    let mut out = String::with_capacity(end + 32);
    out.push_str(&s[..end]);
    out.push_str(&format!("\n…(truncated {} bytes)", omitted));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Вывод сверх потолка вычитывается и отбрасывается, а не копится.
    #[tokio::test]
    async fn bash_output_is_capped() {
        let args = serde_json::json!({"command": "head -c 3000000 /dev/zero | tr '\\0' a"}).to_string();
        let out = run_bash(&args).await.unwrap();
        assert!(out.len() < MAX_READ_OUTPUT_BYTES + 1024, "len={}", out.len());
        assert!(out.contains("…(truncated"));
        assert!(out.contains("exit: 0"));
    }

    /// Отмена (drop future) убивает команду и её потомков.
    #[tokio::test]
    async fn bash_is_killed_on_drop() {
        let marker = std::env::temp_dir().join(format!("synthos-bash-kill-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("sleep 1.5 && touch {}", marker.display());
        let args = serde_json::json!({"command": cmd}).to_string();
        let r = tokio::time::timeout(std::time::Duration::from_millis(300), run_bash(&args)).await;
        assert!(r.is_err(), "команда должна была прерваться по таймауту");
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        assert!(!marker.exists(), "потомок пережил отмену");
    }

    #[test]
    fn qualified_tool_name_resolves_to_catalog_key() {
        assert_eq!(canonical_tool_name("bash"), "bash");
        assert_eq!(canonical_tool_name("bash.bash"), "bash");
        // Шаблон Muse объявляет пространство `notes.*` — модель дописывает
        // в имя действие; ключ каталога сидит в голове, а не в хвосте.
        assert_eq!(canonical_tool_name("notes.action"), "notes");
        assert_eq!(canonical_tool_name("bash.run"), "bash");
        assert_eq!(canonical_tool_name("functions.notes.create"), "notes");
        assert_eq!(canonical_tool_name("tools.web"), "web");
        // Незнакомое имя остаётся как есть — ошибку про него отдаст execute.
        assert_eq!(canonical_tool_name("weather.today"), "weather.today");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_echo_ok() {
        let out = run_bash(r#"{"command":"echo hi"}"#).await.unwrap();
        assert!(out.contains("exit: 0"), "{out}");
        assert!(out.contains("--- stdout ---"), "{out}");
        assert!(out.contains("hi"), "{out}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_nonzero_exit() {
        // `false` всегда возвращает exit=1 — проверяем, что мы его снимаем.
        let out = run_bash(r#"{"command":"false"}"#).await.unwrap();
        assert!(out.contains("exit: 1"), "{out}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_missing_command() {
        let err = run_bash(r#"{}"#).await.unwrap_err();
        matches!(err, ToolError::MissingField("command"));
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // Многобайтный символ "я" занимает 2 байта; повторяем до полной
        // длины MAX_OUTPUT_BYTES + ровно один символ сверху, чтобы
        // активировать обрезку.
        let long = "я".repeat(MAX_OUTPUT_BYTES);
        let t = truncate_output(&long, MAX_OUTPUT_BYTES);
        assert!(t.ends_with(" bytes)"));
        // Убедимся, что не сломали UTF-8.
        let _ = t.chars().count();
    }

    #[test]
    fn truncate_noop_when_short() {
        assert_eq!(truncate_output("ok", MAX_OUTPUT_BYTES), "ok");
    }

    /// Ошибка аргументов после успешного разбора JSON не должна называться
    /// «Invalid arguments JSON» — модель шла чинить синтаксис, а не поле.
    #[tokio::test(flavor = "current_thread")]
    async fn notes_argument_error_has_no_json_word() {
        let e = ToolError::Args("missing \"op\"".to_string());
        assert_eq!(e.to_string(), "Invalid arguments: missing \"op\"");
        let call = ChatToolCall {
            id: "c1".to_string(),
            kind: "function".to_string(),
            function: crate::agent::schema::ChatToolCallFunction {
                name: Some("notes".to_string()),
                arguments: Some("{\"action\":\"kanban\"".to_string()),
            },
        };
        let out = execute(&call).await;
        assert!(out.error && out.invalid_args, "{out:?}");
        assert!(out.content.starts_with("Invalid arguments JSON: "), "{}", out.content);
    }

    /// `"True"` вместо `true` (дважды подряд у wizard, 14.09.2026) — не повод
    /// ронять вызов: строка приводится к типу схемы.
    #[test]
    fn string_booleans_and_numbers_follow_the_schema() {
        use crate::agent::tools::catalog::KEY_WIZARD;
        let out = coerce_args(
            KEY_WIZARD,
            r#"{"question":"q","allow_free_text":"True","required":"False","timeout_sec":"30","options":[{"label":"true","allow_free_text":"yes"}]}"#,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["allow_free_text"], true);
        assert_eq!(v["required"], false);
        assert_eq!(v["timeout_sec"], 30);
        assert_eq!(v["options"][0]["allow_free_text"], true, "вложенные поля по items");
        assert_eq!(v["options"][0]["label"], "true", "строковое поле не трогается");
        // Непонятное слово остаётся — ошибку честно отдаст разбор.
        let odd = coerce_args(KEY_WIZARD, r#"{"question":"q","required":"maybe"}"#);
        assert!(odd.contains("\"maybe\""), "{odd}");
        // Схема разрешает строку (`done` у notes: boolean | string) — как есть.
        let notes = coerce_args(KEY_NOTES, r#"{"action":"calendar","done":"True"}"#);
        assert!(notes.contains("\"True\""), "{notes}");
        assert_eq!(coerce_args("weather", r#"{"x":"True"}"#), r#"{"x":"True"}"#);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wizard_accepts_python_style_booleans() {
        let call = ChatToolCall {
            id: "w1".to_string(),
            kind: "function".to_string(),
            function: crate::agent::schema::ChatToolCallFunction {
                name: Some("wizard".to_string()),
                arguments: Some(r#"{"question":"Роль?","options":[{"label":"A"},{"label":"B"}],"allow_free_text":"True","required":"True"}"#.to_string()),
            },
        };
        let out = execute(&call).await;
        assert!(!out.error, "{}", out.content);
    }

    #[test]
    fn skill_body_survives_common_limit() {
        // Рукописный скил на ~90 KB не должен обрезаться: у `autoskill`
        // свой предел, иначе модель получает инструкцию без хвоста.
        let skill = "я".repeat(48 * 1024); // 96 KB в байтах
        assert!(skill.len() > MAX_OUTPUT_BYTES);
        assert_eq!(output_limit(KEY_AUTOSKILL), MAX_SKILL_OUTPUT_BYTES);
        assert_eq!(truncate_output(&skill, output_limit(KEY_AUTOSKILL)), skill);
        assert!(truncate_output(&skill, MAX_OUTPUT_BYTES).ends_with(" bytes)"));
        // `bash` и `web` — тот же предохранитель, что у скилов: файл или
        // статья на 200 КБ при свободном окне читается одним вызовом, сколько
        // влезет в окно — решает `budget::fit`, а не байты.
        assert_eq!(output_limit(KEY_BASH), MAX_READ_OUTPUT_BYTES);
        assert_eq!(output_limit(KEY_WEB), MAX_READ_OUTPUT_BYTES);
        assert_eq!(truncate_output(&skill, output_limit(KEY_BASH)), skill);
        // Схема из пула `autotools` — тоже инструкция: целиком и без укладки.
        assert_eq!(output_limit(KEY_AUTOTOOLS), MAX_SKILL_OUTPUT_BYTES);
        assert_eq!(history_limit(KEY_AUTOTOOLS), None);
        assert_eq!(canonical_tool_name("functions.autotools"), KEY_AUTOTOOLS);
    }
}
