//! Починка JSON, приехавшего от модели.
//!
//! Локальные модели устойчиво ошибаются в одном месте: закрывающие скобки в
//! конце длинного значения. Реальный вызов из чата — `…"}}]}` вместо
//! `…"}}}]`: элемент массива не закрыт, зато лишняя `}` после `]`. Такой
//! `<tool_call>` разбирался в ничто, ход уходил впустую, а модель повторяла
//! ровно тот же текст (при том же префикс-KV — байт в байт).
//!
//! Чинится только скобочный хвост и ничего не додумывается: незакрытая
//! строка или скобки, не сходящиеся по типу, — отказ. Результат всегда
//! проверяется парсером у вызывающего, поэтому неверная починка не пройдёт
//! дальше молча.

/// Пересобрать скобочный хвост: срезаем закрывающие скобки с конца и
/// дописываем заново по стеку, посчитанному на срезанном префиксе.
/// `None` — чинить нечего либо нечем.
pub fn repair_bracket_tail(text: &str) -> Option<String> {
    let head = text.trim_end_matches(|c: char| c == '}' || c == ']' || c.is_whitespace());
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in head.chars() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop()? != c {
                    return None;
                }
            }
            _ => {}
        }
    }
    if in_string || stack.is_empty() {
        return None;
    }
    let mut out = head.to_string();
    while let Some(c) = stack.pop() {
        out.push(c);
    }
    // Хвост сошёлся сам — чинить было нечего, и предупреждать не о чем.
    (out != text.trim_end()).then_some(out)
}

/// Разобрать JSON, при неудаче — починив скобочный хвост. Второй элемент
/// пары говорит, применялась ли починка: вызывающему это нужно, чтобы
/// сказать модели, что её вызов исправили за неё.
pub fn parse_with_repair(text: &str) -> Option<(serde_json::Value, bool)> {
    if let Ok(v) = serde_json::from_str(text) {
        return Some((v, false));
    }
    let fixed = repair_bracket_tail(text)?;
    serde_json::from_str(&fixed).ok().map(|v| (v, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Реальное тело `<tool_call>` из чата: `data` закрыт, элемент массива —
    /// нет, а лишняя `}` уехала за `]`.
    #[test]
    fn repairs_out_of_order_tail_from_real_tool_call() {
        let broken = r#"{"name":"pipelines","arguments":{"action":"apply","set_state":[{"node":9,"state":{"kind":"TextView","data":{"output_text":"[verse]"}}]}}"#;
        let (v, repaired) = parse_with_repair(broken).expect("чинится");
        assert!(repaired);
        assert_eq!(v["name"], "pipelines");
        assert_eq!(v["arguments"]["set_state"][0]["node"], 9);
        assert_eq!(
            v["arguments"]["set_state"][0]["state"]["data"]["output_text"],
            "[verse]"
        );
    }

    /// Недостача закрывающих скобок (оборванная генерация) тоже чинится.
    #[test]
    fn closes_missing_brackets() {
        assert_eq!(repair_bracket_tail(r#"[{"a": 1}"#).as_deref(), Some(r#"[{"a": 1}]"#));
    }

    /// Целый JSON не трогаем, а по оборванной строке не гадаем.
    #[test]
    fn leaves_intact_and_gives_up_on_truncated_string() {
        assert!(repair_bracket_tail(r#"[{"a": 1}]"#).is_none());
        assert!(repair_bracket_tail(r#"[{"a": "unterminated"#).is_none());
        // Несоответствие типа внутри префикса — не хвостовая ошибка, гадать
        // о ней нельзя: `[` закрыт `}` в середине текста.
        assert!(repair_bracket_tail(r#"{"a": [1}, "b": 2}"#).is_none());
        // А хвост, закрытый не тем символом, — ровно наш случай.
        assert_eq!(repair_bracket_tail(r#"[{"a": 1]"#).as_deref(), Some(r#"[{"a": 1}]"#));
        let (_, repaired) = parse_with_repair(r#"{"a": 1}"#).expect("целый JSON");
        assert!(!repaired);
    }
}
