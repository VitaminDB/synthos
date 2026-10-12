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
    /// Вызов не уложился в `timeout_sec` (или в предел по умолчанию):
    /// исполнение брошено, процессы `bash` убиты вместе с группой.
    #[error("Timed out after {0} s: the call was stopped (bash: the process group was killed). If it legitimately needs longer, call again with a larger \"timeout_sec\"; for long jobs run them in the background (`cmd > /tmp/log 2>&1 &`) and poll the log.")]
    Timeout(u64),
}

/// Инструменты с необязательным аргументом `timeout_sec` — пределом
/// времени вызова, который выставляет модель. У `wizard` поле с тем же
/// именем значит другое (когда свернуть вопрос), `autoskill`/`autotools`
/// мгновенны, `view_media` исполняет цикл чата.
pub(crate) const TIMEOUT_TOOLS: [&str; 7] =
    [KEY_BASH, KEY_WEB, KEY_KB_SEARCH, KEY_PIPELINES, KEY_SUBAGENT, KEY_SYSTEM, KEY_NOTES];

/// Предел по умолчанию, когда модель `timeout_sec` не задала. Только у
/// `bash`: команда, ждущая ввода или держащая pipe фоновым потомком,
/// висела бесконечно — ход стоял на «Генерация ответа» при пустой GPU.
const BASH_DEFAULT_TIMEOUT_SECS: u64 = 600;
/// Потолок, который может попросить модель: сутки.
const MAX_TIMEOUT_SECS: u64 = 24 * 3600;

/// Предел времени вызова: `timeout_sec` из аргументов, иначе умолчание
/// инструмента. `None` — без предела.
pub fn timeout_for(tool: &str, args: &str) -> Option<u64> {
    let tool = canonical_tool_name(tool);
    if !TIMEOUT_TOOLS.contains(&tool) {
        return None;
    }
    let asked = serde_json::from_str::<serde_json::Value>(args).ok().and_then(|v| {
        let t = v.get("timeout_sec")?;
        t.as_f64().or_else(|| t.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
    });
    match asked {
        Some(t) if t > 0.0 => Some((t.ceil() as u64).clamp(1, MAX_TIMEOUT_SECS)),
        _ if tool == KEY_BASH => Some(BASH_DEFAULT_TIMEOUT_SECS),
        _ => None,
    }
}

/// Добавить `timeout_sec` в схемы [`TIMEOUT_TOOLS`]. Зовётся один раз при
/// сборке каталога (`Tool::all`).
pub(crate) fn add_timeout_params(tools: &mut [super::descriptor::Tool]) {
    for t in tools.iter_mut().filter(|t| TIMEOUT_TOOLS.contains(&t.key)) {
        let default = if t.key == KEY_BASH {
            format!(" Default {BASH_DEFAULT_TIMEOUT_SECS}.")
        } else {
            " Default: no limit.".to_string()
        };
        if let Some(props) = t.schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
            props.insert(
                "timeout_sec".to_string(),
                serde_json::json!({
                    "type": "integer",
                    "description": format!(
                        "Optional time limit for this call in seconds; when it runs out the call is \
                         stopped and you get a timeout error. Set it when you expect the call may hang \
                         or take long.{default}"
                    )
                }),
            );
        }
    }
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

pub fn normalize_args(raw: &str) -> String {
    synassist::tools::args::normalize(raw)
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

pub use synassist::tools::args::coerce_to_schema;

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

pub(crate) fn canonical_tool_name(name: &str) -> &str {
    synassist::tools::args::canonical_name(name, &TOOL_KEYS)
}

pub async fn execute(call: &ChatToolCall) -> ToolOutcome {
    let name = call.function.name.clone().unwrap_or_default();
    let name = canonical_tool_name(&name).to_string();
    let raw_args = call.function.arguments.as_deref().unwrap_or("");
    let normalized = coerce_args(&name, &normalize_args(raw_args));
    let args = normalized.as_str();

    let limit = timeout_for(&name, args);
    let run = run_tool(&name, args);
    let result = match limit {
        Some(secs) => match tokio::time::timeout(std::time::Duration::from_secs(secs), run).await {
            Ok(r) => r,
            Err(_) => Err(ToolError::Timeout(secs)),
        },
        None => run.await,
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

/// Исполнить инструмент по ключу (без предела времени — его ставит
/// [`execute`]).
async fn run_tool(name: &str, args: &str) -> Result<String, ToolError> {
    match name {
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
    }
}

pub async fn run_bash(args_json: &str) -> Result<String, ToolError> {
    let v: serde_json::Value = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let command = v.get("command").and_then(|x| x.as_str()).ok_or(ToolError::MissingField("command"))?;
    synassist::tools::bash::run(command, None, MAX_READ_OUTPUT_BYTES).await.map_err(|e| match e {
        synassist::tools::bash::BashError::Spawn(s) => ToolError::Spawn(s),
        other => ToolError::Runtime(other.to_string()),
    })
}

fn truncate_output(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…(truncated {} bytes)", &s[..end], s.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Предел времени: свой у модели (и строкой), умолчание у bash, у
    /// wizard `timeout_sec` — не предел вызова.
    #[test]
    fn timeout_for_reads_model_value_and_defaults() {
        assert_eq!(timeout_for("bash", r#"{"command":"ls"}"#), Some(BASH_DEFAULT_TIMEOUT_SECS));
        assert_eq!(timeout_for("bash", r#"{"command":"ls","timeout_sec":5}"#), Some(5));
        assert_eq!(timeout_for("bash.run", r#"{"command":"ls","timeout_sec":"7"}"#), Some(7));
        assert_eq!(timeout_for("web", r#"{"action":"read"}"#), None);
        assert_eq!(timeout_for("web", r#"{"timeout_sec":30}"#), Some(30));
        assert_eq!(timeout_for("wizard", r#"{"timeout_sec":30}"#), None);
        let bash = super::super::descriptor::Tool::by_key("bash").unwrap();
        assert!(bash.schema["properties"]["timeout_sec"].is_object());
        let notes = super::super::descriptor::Tool::by_key("notes").unwrap();
        assert!(notes.schema["properties"]["timeout_sec"].is_object());
    }

    /// Зависшая команда обрывается по `timeout_sec`, ошибка говорит модели,
    /// что делать дальше.
    #[tokio::test]
    async fn bash_call_stops_at_timeout() {
        let call = ChatToolCall {
            id: "t1".to_string(),
            kind: "function".to_string(),
            function: crate::agent::schema::ChatToolCallFunction {
                name: Some("bash".to_string()),
                arguments: Some(r#"{"command":"sleep 30","timeout_sec":1}"#.to_string()),
            },
        };
        let t0 = std::time::Instant::now();
        let out = execute(&call).await;
        assert!(t0.elapsed() < std::time::Duration::from_secs(5));
        assert!(out.error);
        assert!(out.content.contains("Timed out after 1 s"), "{}", out.content);
    }

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
