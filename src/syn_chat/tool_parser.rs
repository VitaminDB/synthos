//! Streaming-парсер блоков `<tool_call>...</tool_call>` в выводе модели.
//!
//! Qwen3 ChatML формат tool-calling:
//! ```text
//! <tool_call>
//! {"name": "bash", "arguments": {"command": "ls"}}
//! </tool_call>
//! ```
//!
//! Парсер получает дельты текста (как в `generate_streaming` callback),
//! выделяет «чистый» текст для UI (без tool-маркеров) и накапливает
//! завершённые tool-вызовы для последующего исполнения.
//!
//! Безопасность частичных тегов: если delta заканчивается префиксом
//! `<`, `<t`, `<too`, `</tool_cal` и т. д. — парсер придерживает буфер,
//! чтобы UI не показал «битый» тег с последующим переписыванием.

const OPEN_TAG: &str = "<tool_call>";
const CLOSE_TAG: &str = "</tool_call>";

/// Один сырой tool-вызов: имя функции + JSON-аргументы как строка.
#[derive(Debug, Clone)]
pub struct RawToolCall {
    pub name: String,
    pub arguments_json: String,
}

/// Результат одного `feed`: чистый текст, готовый к показу пользователю,
/// плюс live-дельта содержимого tool_call-блока для превью команды в UI.
#[derive(Debug, Default)]
pub struct ToolParseFeed {
    pub clean_delta: String,
    /// Текст, «съеденный» внутри `<tool_call>`-блока этим feed'ом. Сами теги
    /// и их частичные префиксы сюда не попадают — UI может показывать дельту
    /// как есть, пока модель дописывает вызов.
    pub tool_delta: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Снаружи tool_call-блока — текст идёт в clean_delta.
    Outside,
    /// Внутри tool_call-блока — байты копятся в `inner_buf`.
    Inside,
}

/// Накопительный парсер. Создаётся один на сессию генерации, переиспользуется
/// между callback-вызовами.
pub struct ToolCallParser {
    state: State,
    /// Незавершённый «хвост» из предыдущих feed-ов. В режиме Outside может
    /// содержать частичный префикс OPEN_TAG; в режиме Inside — частичный
    /// префикс CLOSE_TAG (само тело сразу уходит в `inner` и `tool_delta`).
    buf: String,
    /// Тело текущего tool_call-блока, накопленное для разбора на закрытии.
    /// Дублирует то, что уже отдано наружу через `tool_delta`.
    inner: String,
    /// Завершённые tool-вызовы.
    calls: Vec<RawToolCall>,
}

impl ToolCallParser {
    pub fn new() -> Self {
        Self {
            state: State::Outside,
            buf: String::new(),
            inner: String::new(),
            calls: Vec::new(),
        }
    }

    /// Количество уже распознанных и распарсенных tool-вызовов.
    /// Используется agent-loop'ом для раннего выхода из stream'а после
    /// закрытия `</tool_call>`.
    pub fn calls_count(&self) -> usize {
        self.calls.len()
    }

    /// Возвращает `true`, если парсер в данный момент НЕ внутри открытого
    /// `<tool_call>`-блока. Сочетается с `calls_count() > 0` для надёжного
    /// определения «модель закончила tool-call».
    pub fn is_outside(&self) -> bool {
        matches!(self.state, State::Outside)
    }

    /// Подать очередной кусок текста. Возвращает clean_delta — текст,
    /// который безопасно немедленно отрендерить в UI.
    pub fn feed(&mut self, delta: &str) -> ToolParseFeed {
        let mut out = String::new();
        let mut tool_out = String::new();
        // Объединяем хвост с новой дельтой, чтобы корректно ловить теги
        // на стыке chunks.
        let mut work = std::mem::take(&mut self.buf);
        work.push_str(delta);

        loop {
            match self.state {
                State::Outside => {
                    if let Some(idx) = work.find(OPEN_TAG) {
                        // Всё до тега — чистый текст.
                        out.push_str(&work[..idx]);
                        work.drain(..idx + OPEN_TAG.len());
                        self.state = State::Inside;
                        continue;
                    }
                    // Тег не найден. Эмитим всё кроме потенциально-частичного
                    // суффикса.
                    let hold = potential_tag_suffix_len(&work, OPEN_TAG);
                    let emit_end = work.len() - hold;
                    if emit_end > 0 {
                        out.push_str(&work[..emit_end]);
                        work.drain(..emit_end);
                    }
                    break;
                }
                State::Inside => {
                    if let Some(idx) = work.find(CLOSE_TAG) {
                        tool_out.push_str(&work[..idx]);
                        self.inner.push_str(&work[..idx]);
                        match parse_tool_call_body(&self.inner) {
                            Some((call, _format)) => {
                                self.calls.push(call);
                            }
                            None => {
                                tracing::warn!(
                                    tool_call_body = %self.inner.trim(),
                                    "failed to parse <tool_call> JSON — пропускаем"
                                );
                            }
                        }
                        self.inner.clear();
                        work.drain(..idx + CLOSE_TAG.len());
                        self.state = State::Outside;
                        continue;
                    }
                    // Закрывающий тег не найден: придерживаем возможный
                    // префикс тега, остальное — в тело и live-дельту.
                    let hold = potential_tag_suffix_len(&work, CLOSE_TAG);
                    let emit_end = work.len() - hold;
                    if emit_end > 0 {
                        tool_out.push_str(&work[..emit_end]);
                        self.inner.push_str(&work[..emit_end]);
                        work.drain(..emit_end);
                    }
                    break;
                }
            }
        }

        self.buf = work;
        ToolParseFeed { clean_delta: out, tool_delta: tool_out }
    }

    /// Финализирует парсер: возвращает накопленные tool-вызовы и хвост
    /// чистого текста (если что-то осталось вне tool_call-блоков).
    pub fn finish(mut self) -> (Vec<RawToolCall>, String) {
        match self.state {
            State::Outside => {
                // Всё в buf — это «чистый» текст. Частичный префикс тега
                // в самом конце потока выводим как есть (модель закончила
                // обычным текстом, а не tool-call'ом).
                let tail = std::mem::take(&mut self.buf);
                (self.calls, tail)
            }
            State::Inside => {
                // Открытый, но не закрытый tool_call. Модель нередко
                // заканчивает генерацию сразу после JSON, не написав
                // `</tool_call>` — тело при этом целое. Отбрасывать такой
                // вызов значит потерять ход: цикл завершается без действия,
                // и пользователю остаётся жать «Продолжить». Поэтому сначала
                // пробуем разобрать накопленное, и только если не вышло —
                // выбрасываем.
                self.inner.push_str(&self.buf);
                match parse_tool_call_body(&self.inner) {
                    Some((call, _format)) => {
                        tracing::debug!(
                            name = %call.name,
                            "tool_call без </tool_call> — тело целое, принимаем"
                        );
                        self.calls.push(call);
                    }
                    None => tracing::warn!(
                        truncated_body = %self.inner.trim(),
                        "stream закончился внутри <tool_call> без </tool_call>, \
                         тело не разбирается — отбрасываем"
                    ),
                }
                (self.calls, String::new())
            }
        }
    }
}

impl Default for ToolCallParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Возвращает длину суффикса `text` в байтах, который мог бы быть началом
/// `tag`. Например, если `text = "abc<too"` и `tag = "<tool_call>"`, вернёт 4.
/// Нужно, чтобы не эмитить «битый» префикс в UI.
///
/// Учитывает UTF-8 char-boundary: если `text.len() - hold` попадает внутрь
/// многобайтового символа, такой `hold` пропускается. Поскольку `tag` целиком
/// ASCII, валидный суффикс гарантированно начинается с char-boundary.
fn potential_tag_suffix_len(text: &str, tag: &str) -> usize {
    let bytes = text.as_bytes();
    let tag_bytes = tag.as_bytes();
    let max_check = tag_bytes.len().saturating_sub(1).min(bytes.len());
    for hold in (1..=max_check).rev() {
        let suffix_start = bytes.len() - hold;
        if !text.is_char_boundary(suffix_start) {
            continue;
        }
        let suffix = &bytes[suffix_start..];
        if tag_bytes.starts_with(suffix) {
            return hold;
        }
    }
    0
}

/// Какой синтаксис был у успешно распарсенного tool_call'а. Возвращается
/// из `parse_tool_call_body` чтобы verhalten мог отметить Anthropic XML
/// как «неродной» для Qwen3 формат и предупредить пользователя в UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolCallFormat {
    Qwen3Json,
    AnthropicXml,
}

/// Распарсить тело `<tool_call>...</tool_call>`.
///
/// Поддерживает два формата:
/// 1. **Qwen3 native JSON** — `{"name": "...", "arguments": {...}}` (предпочтительный).
/// 2. **Anthropic-style XML** — `<function=NAME><parameter=KEY>VAL</parameter>...</function>`,
///    который некоторые fine-tune'ы Qwen генерируют вместо JSON (видели даже
///    глюк токенайзера `<functionfunction=NAME>` с удвоенным `function`).
///
/// Возвращает None, если не подошёл ни один формат.
fn parse_tool_call_body(body: &str) -> Option<(RawToolCall, ToolCallFormat)> {
    if let Some(call) = parse_tool_call_json_qwen(body) {
        return Some((call, ToolCallFormat::Qwen3Json));
    }
    parse_tool_call_xml_anthropic(body).map(|c| (c, ToolCallFormat::AnthropicXml))
}

fn parse_tool_call_json_qwen(body: &str) -> Option<RawToolCall> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let arguments_json = match v.get("arguments") {
        Some(args) => serde_json::to_string(args).ok()?,
        None => "{}".to_string(),
    };
    Some(RawToolCall { name, arguments_json })
}

/// Парсер для Anthropic-стиля. Принимает варианты:
/// - `<function=NAME>...</function>`
/// - `<functionfunction=NAME>...` (модель удвоила `function`-токен)
/// - тот же tag без закрывающего `</function>` (обрезанный поток).
///
/// Внутри ожидает `<parameter=KEY>VAL</parameter>` пары. VAL пробуем сначала
/// распарсить как JSON (для чисел/массивов/объектов), иначе кладём как строку.
fn parse_tool_call_xml_anthropic(body: &str) -> Option<RawToolCall> {
    let body = body.trim();
    let mut rest = body.strip_prefix('<')?;
    // Snap any number of leading `function` tokens — нужно для случая
    // `<functionfunction=NAME>` (см. doc-comment).
    let mut stripped = false;
    while let Some(r) = rest.strip_prefix("function") {
        rest = r;
        stripped = true;
    }
    if !stripped {
        return None;
    }
    let after_eq = rest.strip_prefix('=')?;
    let name_end = after_eq.find('>')?;
    let name = after_eq[..name_end].trim().to_string();
    if name.is_empty() {
        return None;
    }

    let mut cursor = &after_eq[name_end + 1..];
    let mut args = serde_json::Map::new();
    loop {
        let Some(p_start) = cursor.find("<parameter=") else { break };
        let after_p = &cursor[p_start + "<parameter=".len()..];
        let Some(key_end) = after_p.find('>') else { break };
        let key = after_p[..key_end].trim().to_string();
        let value_start = key_end + 1;
        let Some(value_end_rel) = after_p[value_start..].find("</parameter>") else { break };
        let raw_val = &after_p[value_start..value_start + value_end_rel];
        let v = serde_json::from_str::<serde_json::Value>(raw_val.trim())
            .unwrap_or_else(|_| serde_json::Value::String(raw_val.to_string()));
        if !key.is_empty() {
            args.insert(key, v);
        }
        cursor = &after_p[value_start + value_end_rel + "</parameter>".len()..];
    }

    let arguments_json = serde_json::to_string(&serde_json::Value::Object(args)).ok()?;
    Some(RawToolCall { name, arguments_json })
}

#[cfg(test)]
mod tests {
    /// Стрим кончился сразу после JSON, без `</tool_call>` — вызов должен
    /// доехать: иначе ход теряется впустую и пользователь жмёт «Продолжить».
    #[test]
    fn unclosed_tool_call_with_valid_body_is_accepted() {
        let mut p = ToolCallParser::new();
        p.feed(
            "<tool_call>{\"name\":\"pipelines\",\"arguments\":{\"action\":\"run\",\"free_vram\":true}}",
        );
        let (calls, tail) = p.finish();
        assert_eq!(calls.len(), 1, "вызов принят");
        assert_eq!(calls[0].name, "pipelines");
        assert!(calls[0].arguments_json.contains("\"action\":\"run\""));
        assert!(tail.is_empty());
    }

    /// Оборвался на полуслове — разбирать нечего, отбрасываем.
    #[test]
    fn unclosed_tool_call_with_broken_body_is_dropped() {
        let mut p = ToolCallParser::new();
        p.feed("<tool_call>{\"name\":\"pipel");
        let (calls, _) = p.finish();
        assert!(calls.is_empty());
    }

    use super::*;

    fn feed_all(parser: &mut ToolCallParser, chunks: &[&str]) -> String {
        let mut out = String::new();
        for c in chunks {
            out.push_str(&parser.feed(c).clean_delta);
        }
        out
    }

    #[test]
    fn feed_no_tool() {
        let mut p = ToolCallParser::new();
        let out = feed_all(&mut p, &["Hello", " world!"]);
        let (calls, tail) = p.finish();
        assert_eq!(out, "Hello world!");
        assert!(calls.is_empty());
        assert_eq!(tail, "");
    }

    #[test]
    fn feed_full_tool_call_one_chunk() {
        let mut p = ToolCallParser::new();
        let chunk = r#"<tool_call>{"name":"bash","arguments":{"command":"ls"}}</tool_call>"#;
        let out = feed_all(&mut p, &[chunk]);
        let (calls, tail) = p.finish();
        assert_eq!(out, "");
        assert_eq!(tail, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert!(calls[0].arguments_json.contains("\"command\":\"ls\""));
    }

    #[test]
    fn feed_split_across_chunks() {
        let mut p = ToolCallParser::new();
        let out = feed_all(&mut p, &[
            "Сейчас вызову bash. ",
            "<to",
            "ol_call>",
            r#"{"name":"bash","#,
            r#""arguments":{"command":"pwd"}}"#,
            "</tool_call>",
            " готово.",
        ]);
        let (calls, _tail) = p.finish();
        assert_eq!(out, "Сейчас вызову bash.  готово.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn feed_partial_tag_held() {
        let mut p = ToolCallParser::new();
        let part1 = p.feed("hello <to");
        // <to — потенциальный префикс <tool_call>, придерживается.
        assert_eq!(part1.clean_delta, "hello ");
        let part2 = p.feed("o");
        // Теперь "<too" — всё ещё префикс, ничего не эмитим (кроме старого).
        assert_eq!(part2.clean_delta, "");
        // А вот не tool_call:
        let part3 = p.feed("ot");
        // "<tooot" — больше не префикс, эмитим всё.
        assert_eq!(part3.clean_delta, "<tooot");
    }

    #[test]
    fn multiple_calls() {
        let mut p = ToolCallParser::new();
        let chunk = format!(
            "{}{}",
            r#"<tool_call>{"name":"a","arguments":{}}</tool_call>"#,
            r#"<tool_call>{"name":"b","arguments":{"x":1}}</tool_call>"#,
        );
        let out = feed_all(&mut p, &[&chunk]);
        let (calls, _tail) = p.finish();
        assert_eq!(out, "");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "a");
        assert_eq!(calls[1].name, "b");
        assert!(calls[1].arguments_json.contains("\"x\":1"));
    }

    #[test]
    fn mixed_prose_and_call() {
        let mut p = ToolCallParser::new();
        let chunk = r#"Сейчас<tool_call>{"name":"x","arguments":{}}</tool_call>готово"#;
        let out = feed_all(&mut p, &[chunk]);
        let (calls, _tail) = p.finish();
        assert_eq!(out, "Сейчасготово");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn invalid_json_logged_not_panicking() {
        let mut p = ToolCallParser::new();
        let out = feed_all(&mut p, &["<tool_call>not a json</tool_call>"]);
        let (calls, _tail) = p.finish();
        assert!(out.is_empty());
        assert!(calls.is_empty(), "невалидный JSON — call пропущен");
    }

    #[test]
    fn arguments_missing_treated_as_empty_object() {
        let mut p = ToolCallParser::new();
        let _ = p.feed(r#"<tool_call>{"name":"foo"}</tool_call>"#);
        let (calls, _tail) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments_json, "{}");
    }

    #[test]
    fn anthropic_xml_inside_tool_call() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call>\n<function=subagent>\n\
            <parameter=system_prompt>You are helper</parameter>\n\
            <parameter=task>Do X</parameter>\n\
            </function>\n</tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "subagent");
        assert!(calls[0].arguments_json.contains("\"system_prompt\":\"You are helper\""));
        assert!(calls[0].arguments_json.contains("\"task\":\"Do X\""));
    }

    #[test]
    fn anthropic_xml_double_function_glitch() {
        // Видели в реальных логах: <functionfunction=NAME> — модель удвоила токен.
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><functionfunction=bash>\
            <parameter=command>ls -la</parameter></function></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert!(calls[0].arguments_json.contains("\"command\":\"ls -la\""));
    }

    #[test]
    fn anthropic_xml_numeric_value_parsed_as_number() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><function=foo>\
            <parameter=n>42</parameter></function></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].arguments_json.contains("\"n\":42"));
    }

    #[test]
    fn anthropic_xml_without_closing_function_tag() {
        // Поток может оборваться или модель не закрывает </function>.
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><function=foo>\
            <parameter=x>y</parameter></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "foo");
        assert!(calls[0].arguments_json.contains("\"x\":\"y\""));
    }

    #[test]
    fn qwen_json_still_works_after_fallback_added() {
        let mut p = ToolCallParser::new();
        let chunk = r#"<tool_call>{"name":"bash","arguments":{"command":"ls"}}</tool_call>"#;
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert!(calls[0].arguments_json.contains("\"command\":\"ls\""));
    }

    #[test]
    fn tool_delta_streams_incrementally() {
        let mut p = ToolCallParser::new();
        assert_eq!(p.feed("<tool_call>").tool_delta, "");
        assert_eq!(p.feed(r#"{"name":"bash","#).tool_delta, r#"{"name":"bash","#);
        assert_eq!(
            p.feed(r#""arguments":{"command":"ls"}}"#).tool_delta,
            r#""arguments":{"command":"ls"}}"#
        );
        assert_eq!(p.feed("</tool_call>").tool_delta, "");
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn tool_delta_holds_partial_close_tag() {
        let mut p = ToolCallParser::new();
        let _ = p.feed("<tool_call>");
        // Частичный `</tool_c` придерживается, JSON — отдаётся.
        assert_eq!(p.feed(r#"{"name":"x"}</tool_c"#).tool_delta, r#"{"name":"x"}"#);
        // Хвост тега пришёл — дельта пустая, вызов зафиксирован.
        assert_eq!(p.feed("all>").tool_delta, "");
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "x");
    }

    #[test]
    fn tool_delta_empty_outside_block() {
        let mut p = ToolCallParser::new();
        let feed = p.feed("обычный текст");
        assert_eq!(feed.clean_delta, "обычный текст");
        assert_eq!(feed.tool_delta, "");
    }

    #[test]
    fn potential_tag_suffix_basic() {
        assert_eq!(potential_tag_suffix_len("abc<too", "<tool_call>"), 4);
        assert_eq!(potential_tag_suffix_len("abc<", "<tool_call>"), 1);
        assert_eq!(potential_tag_suffix_len("plain text", "<tool_call>"), 0);
        assert_eq!(potential_tag_suffix_len("", "<tool_call>"), 0);
    }
}
