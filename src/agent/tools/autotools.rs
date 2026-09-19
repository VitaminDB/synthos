//! Tool `autotools` — пул инструментов по запросу, близнец `autoskill`.
//!
//! Инструмент из пула не объявляется модели схемой: в description
//! `autotools` лежит только строка «id — когда звать» ([`chat_tool`]). Модель
//! зовёт `autotools` с id и получает результатом описание, JSON-схему
//! параметров и правила инструмента ([`render`]), а дальше вызывает его по
//! имени — исполнитель принимает любой инструмент каталога, объявлен он в
//! запросе или нет.
//!
//! Почему результатом, а не расширением списка `tools`: схемы рендерятся в
//! голове промпта, и любая правка набора обнуляла бы префикс-KV всей истории.
//! Ответ инструмента дописывается в хвост — история растёт, префикс цел.
//! По той же причине пул и активные инструменты меняет только пользователь.

use serde_json::Value;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use super::catalog::KEY_AUTOSKILL;
use super::descriptor::Tool;
use super::executor::{canonical_tool_name, ToolError};
use crate::agent::schema::{ChatTool, ToolFunctionSchema};
use crate::context::AppCtx;

/// Инструменты пула в порядке, в каком пользователь их туда клал: ключи из
/// `auto`, которые есть в каталоге, выбираются пользователем и не активны
/// (активный уже объявлен схемой — грузить нечего).
pub fn pool(active: &[String], auto: &[String]) -> Vec<&'static Tool> {
    let mut out: Vec<&'static Tool> = Vec::new();
    for key in auto {
        let Some(tool) = Tool::by_key(key) else { continue };
        if tool.is_implicit() || active.iter().any(|k| k == key) {
            continue;
        }
        if !out.iter().any(|t| t.key == tool.key) {
            out.push(tool);
        }
    }
    out
}

/// Переключает `key` в списке `primary` и убирает его из `other`: инструмент
/// либо активен, либо в пуле, но не в обоих сразу.
pub fn toggle_exclusive(primary: &mut Vec<String>, other: &mut Vec<String>, key: &str) {
    if let Some(idx) = primary.iter().position(|k| k == key) {
        primary.remove(idx);
    } else {
        primary.push(key.to_string());
        other.retain(|k| k != key);
    }
}

/// ChatTool `autotools` для непустого пула: статическое описание + каталог
/// «id — когда звать», id — в enum схемы.
pub fn chat_tool(pool: &[&Tool]) -> ChatTool {
    let descriptor = Tool::by_key(super::catalog::KEY_AUTOTOOLS)
        .expect("autotools descriptor должен существовать");
    let mut description = descriptor.description.to_string();
    description.push_str("\n\nTools you can load (id — when to use):\n");
    for t in pool {
        description.push_str(&format!("- {} — {}\n", t.key, t.summary));
    }

    let ids: Vec<Value> = pool.iter().map(|t| Value::String(t.key.to_string())).collect();
    let mut schema = descriptor.schema.clone();
    if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
        if let Some(id) = props.get_mut("id").and_then(|v| v.as_object_mut()) {
            id.insert("enum".to_string(), Value::Array(ids.clone()));
        }
        if let Some(items) = props
            .get_mut("ids")
            .and_then(|v| v.get_mut("items"))
            .and_then(|v| v.as_object_mut())
        {
            items.insert("enum".to_string(), Value::Array(ids));
        }
    }

    ChatTool {
        kind: "function".to_string(),
        function: ToolFunctionSchema {
            name: descriptor.key.to_string(),
            description: Some(description),
            parameters: Some(schema),
            strict: None,
        },
    }
}

/// Запрошенные id: `id` (в том числе через запятую) и `ids`, без повторов,
/// приведённые к ключам каталога (`functions.notes` → `notes`).
fn requested_ids(args_json: &str) -> Result<Vec<String>, ToolError> {
    let v: Value = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let mut raw: Vec<String> = Vec::new();
    if let Some(id) = v.get("id").and_then(Value::as_str) {
        raw.extend(id.split(',').map(str::to_string));
    }
    if let Some(ids) = v.get("ids").and_then(Value::as_array) {
        raw.extend(ids.iter().filter_map(Value::as_str).map(str::to_string));
    }
    let mut out: Vec<String> = Vec::new();
    for id in raw {
        let id = canonical_tool_name(id.trim()).to_string();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err(ToolError::MissingField("id"));
    }
    Ok(out)
}

/// Снимок с main-потока: наборы активных и пула хода и живое описание
/// `autoskill` (список скилов в нём собирается из сигнала).
struct Snapshot {
    active: Vec<String>,
    auto: Vec<String>,
    autoskill: ChatTool,
}

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let ids = requested_ids(args_json)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_main_thread(move || {
        let app = use_context::<AppCtx>();
        // Настройки своего хода: чат мог уйти в фон, и панели показывают
        // инструменты другого.
        let turn = crate::syn_chat::chat_settings::for_turn(&use_context::<crate::syn_chat::SynChatCtx>());
        let _ = tx.send(Snapshot {
            autoskill: crate::agent::tool_flow::build_autoskill_chat_tool(&app, &turn.chat.skills_active),
            active: turn.chat.tools_active,
            auto: turn.chat.tools_auto,
        });
    });
    let snap = rx.await.map_err(|e| ToolError::Runtime(format!("autotools: {e}")))?;
    render(&ids, &snap.active, &snap.auto, &snap.autoskill)
}

/// Ответ `autotools` на уже разобранные id. Инструмент из пула — описание,
/// схема и правила; активный — пометка «уже объявлен»; прочее — ошибка со
/// списком того, что можно загрузить.
pub fn render(
    ids: &[String],
    active: &[String],
    auto: &[String],
    autoskill: &ChatTool,
) -> Result<String, ToolError> {
    let pool = pool(active, auto);
    let mut loaded: Vec<&'static Tool> = Vec::new();
    let mut declared: Vec<&str> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for id in ids {
        if let Some(t) = pool.iter().find(|t| t.key == id.as_str()).copied() {
            loaded.push(t);
        } else if active.iter().any(|k| k == id) && Tool::by_key(id).is_some_and(|t| !t.is_implicit()) {
            declared.push(id);
        } else {
            missing.push(id);
        }
    }

    let loadable = if pool.is_empty() {
        "none".to_string()
    } else {
        pool.iter().map(|t| t.key).collect::<Vec<_>>().join(", ")
    };
    if loaded.is_empty() && declared.is_empty() {
        return Err(ToolError::Args(format!(
            "no such tool to load: {}. Tools you can load: {loadable}",
            missing.join(", ")
        )));
    }

    let mut out = String::new();
    if !loaded.is_empty() {
        let names: Vec<&str> = loaded.iter().map(|t| t.key).collect();
        out.push_str(&format!(
            "Loaded: {}. Call {} directly by name, with arguments matching the \
             parameters below.\n",
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" },
        ));
    }
    if !declared.is_empty() {
        out.push_str(&format!(
            "Already declared, call directly: {}.\n",
            declared.join(", ")
        ));
    }
    if !missing.is_empty() {
        out.push_str(&format!(
            "Not available: {}. Tools you can load: {loadable}.\n",
            missing.join(", ")
        ));
    }

    for t in loaded {
        let (description, schema) = if t.key == KEY_AUTOSKILL {
            (
                autoskill.function.description.clone().unwrap_or_default(),
                autoskill.function.parameters.clone().unwrap_or_else(|| t.schema.clone()),
            )
        } else {
            (t.description.to_string(), t.schema.clone())
        };
        out.push_str(&format!("\n## {}\n\n{}\n\nParameters (JSON Schema):\n", t.key, description));
        out.push_str(&serde_json::to_string(&schema).unwrap_or_default());
        out.push('\n');
        if let Some(rules) = crate::syn_chat::system_prompt::tool_rules(t.key) {
            out.push_str("\nRules:\n");
            out.push_str(rules.trim_start());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn autoskill_stub() -> ChatTool {
        Tool::by_key(KEY_AUTOSKILL).unwrap().to_chat_tool()
    }

    #[test]
    fn pool_skips_active_unknown_and_itself() {
        let active = keys(&["bash", "web"]);
        let auto = keys(&["notes", "web", "nope", "autotools", "notes", "pipelines"]);
        let got: Vec<&str> = pool(&active, &auto).iter().map(|t| t.key).collect();
        assert_eq!(got, ["notes", "pipelines"]);
    }

    #[test]
    fn toggle_moves_between_active_and_pool() {
        let mut active = keys(&["bash", "notes"]);
        let mut auto = keys(&["pipelines"]);
        // В пул — уходит из активных.
        toggle_exclusive(&mut auto, &mut active, "notes");
        assert_eq!((active.clone(), auto.clone()), (keys(&["bash"]), keys(&["pipelines", "notes"])));
        // Повторный клик в пуле — просто выключает.
        toggle_exclusive(&mut auto, &mut active, "notes");
        assert_eq!((active.clone(), auto.clone()), (keys(&["bash"]), keys(&["pipelines"])));
        // В активные — уходит из пула.
        toggle_exclusive(&mut active, &mut auto, "pipelines");
        assert_eq!((active, auto), (keys(&["bash", "pipelines"]), keys(&[])));
    }

    #[test]
    fn chat_tool_lists_pool_with_summaries_and_enum() {
        let p = pool(&[], &keys(&["notes", "pipelines"]));
        let t = chat_tool(&p);
        assert_eq!(t.function.name, "autotools");
        let desc = t.function.description.unwrap();
        assert!(desc.contains(&format!("- notes — {}", Tool::by_key("notes").unwrap().summary)), "{desc}");
        assert!(desc.contains("- pipelines — "), "{desc}");
        // Самих схем в описании нет — ради этого пул и нужен.
        assert!(!desc.contains("\"properties\""), "{desc}");
        let params = t.function.parameters.unwrap();
        assert_eq!(params["properties"]["id"]["enum"], serde_json::json!(["notes", "pipelines"]));
        assert_eq!(params["properties"]["ids"]["items"]["enum"], serde_json::json!(["notes", "pipelines"]));
    }

    #[test]
    fn render_gives_description_schema_and_rules() {
        let out = render(&keys(&["notes"]), &keys(&["bash"]), &keys(&["notes"]), &autoskill_stub()).unwrap();
        let notes = Tool::by_key("notes").unwrap();
        assert!(out.starts_with("Loaded: notes."), "{out}");
        assert!(out.contains(notes.description), "описание целиком");
        assert!(out.contains(&serde_json::to_string(&notes.schema).unwrap()), "схема целиком");
        // Правила, которые активному notes дал бы системный промпт.
        let rules = crate::syn_chat::system_prompt::tool_rules("notes").unwrap();
        assert!(out.contains(rules.trim_start()), "правила целиком");
    }

    #[test]
    fn render_marks_active_and_rejects_unavailable() {
        let active = keys(&["bash"]);
        let auto = keys(&["web"]);
        let out = render(&keys(&["web", "bash", "notes"]), &active, &auto, &autoskill_stub()).unwrap();
        assert!(out.contains("Loaded: web."), "{out}");
        assert!(out.contains("Already declared, call directly: bash."), "{out}");
        assert!(out.contains("Not available: notes. Tools you can load: web."), "{out}");

        let err = render(&keys(&["notes"]), &active, &auto, &autoskill_stub()).unwrap_err();
        assert!(matches!(err, ToolError::Args(_)));
        assert_eq!(err.to_string(), "Invalid arguments: no such tool to load: notes. Tools you can load: web");
    }

    #[test]
    fn render_uses_live_autoskill_description() {
        let mut live = autoskill_stub();
        live.function.description = Some("skills: greet — tone".to_string());
        let out = render(&keys(&["autoskill"]), &[], &keys(&["autoskill"]), &live).unwrap();
        assert!(out.contains("skills: greet — tone"), "{out}");
    }

    #[test]
    fn requested_ids_accepts_id_list_and_qualified_names() {
        assert_eq!(requested_ids(r#"{"id":"notes"}"#).unwrap(), ["notes"]);
        assert_eq!(requested_ids(r#"{"id":"notes, web"}"#).unwrap(), ["notes", "web"]);
        assert_eq!(
            requested_ids(r#"{"id":"functions.notes","ids":["notes","pipelines"]}"#).unwrap(),
            ["notes", "pipelines"]
        );
        assert!(matches!(requested_ids("{}"), Err(ToolError::MissingField("id"))));
    }
}
