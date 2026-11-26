//! Размышления в готовом тексте. Служба отдаёт их отдельно
//! (`Generated::thinking`); здесь — защитная чистка ответа, если шаблон всё
//! же оставил блок `<think>…</think>` в тексте.

const TAGS: [(&str, &str); 2] = [("<think>", "</think>"), ("<think:opensource>", "</think:opensource>")];

/// Текст без блоков размышлений. Незакрытый блок отрезает хвост; закрытие
/// без открытия (блок открыл шаблон) отрезает всё до него.
pub fn strip_thinking(raw: &str) -> String {
    let mut s = raw.to_string();
    for (open, close) in TAGS {
        if let Some(end) = s.find(close) {
            if s.find(open).is_none_or(|o| o > end) {
                s = s[end + close.len()..].to_string();
            }
        }
        while let Some(start) = s.find(open) {
            match s[start..].find(close) {
                Some(rel) => s.replace_range(start..start + rel + close.len(), ""),
                None => s.truncate(start),
            }
        }
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::strip_thinking;

    #[test]
    fn strips_blocks() {
        assert_eq!(strip_thinking("<think>думаю</think>\n\nОтвет"), "Ответ");
        assert_eq!(strip_thinking("думаю</think>Ответ"), "Ответ");
        assert_eq!(strip_thinking("Ответ<think>хвост"), "Ответ");
        assert_eq!(strip_thinking("Просто текст"), "Просто текст");
    }
}
