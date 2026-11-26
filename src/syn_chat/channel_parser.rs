//! Канальный формат реплик (Muse Glimmer: `<|start|>…<|message|>…`, вызовы —
//! блоком `<atem:function_calls>`). Разбор ответа делает служба; здесь —
//! признак канальной модели и пересборка реплики ассистента для истории.

use super::tool_parser::RawToolCall;

/// Служебные токены канального формата: модель канальная, если каждый из
/// них — один токен её словаря (проверяет служба, `count_tokens`).
pub const CHANNEL_TOKENS: [&str; 4] = ["<|start|>", "<|message|>", "<|eom|>", "<|eot|>"];

const ATEM_OPEN: &str = "<atem:function_calls>";
pub const ATEM_CLOSE: &str = "</atem:function_calls>";

/// Реплика ассистента канальной модели: текст и ATEM-блок вызовов — в том
/// виде, в каком модель их печатает (заголовки каналов шаблон припишет сам).
pub fn rebuild_turn_text(body: &str, calls: &[RawToolCall]) -> String {
    let mut out = body.trim().to_string();
    if calls.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(ATEM_OPEN);
    for call in calls {
        out.push_str(&format!("\n<atem:invoke name=\"{}\">\n", call.name));
        if let Ok(serde_json::Value::Object(args)) = serde_json::from_str::<serde_json::Value>(&call.arguments_json) {
            for (k, v) in args {
                let text = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                out.push_str(&format!("<atem:parameter name=\"{k}\">{text}</atem:parameter>\n"));
            }
        }
        out.push_str("</atem:invoke>");
    }
    out.push('\n');
    out.push_str(ATEM_CLOSE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_adds_atem_block() {
        let calls = vec![RawToolCall { name: "bash".into(), arguments_json: r#"{"command":"ls"}"#.into() }];
        let t = rebuild_turn_text(" Смотрю. ", &calls);
        assert_eq!(t, "Смотрю.\n<atem:function_calls>\n<atem:invoke name=\"bash\">\n<atem:parameter name=\"command\">ls</atem:parameter>\n</atem:invoke>\n</atem:function_calls>");
        assert_eq!(rebuild_turn_text("текст", &[]), "текст");
    }
}
