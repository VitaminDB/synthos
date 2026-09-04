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
                        self.accept_body(false);
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
                let buf = std::mem::take(&mut self.buf);
                self.inner.push_str(&buf);
                self.accept_body(true);
                (self.calls, String::new())
            }
        }
    }
}

impl ToolCallParser {
    /// Разобрать накопленное тело блока и, если вышло, добавить вызов.
    ///
    /// Заодно оставляет в журнале то, без чего поломки вызовов не разобрать
    /// (сырой вывод модели больше нигде не сохраняется):
    /// - тело целиком, когда парсер узнал имя, но не нашёл ни одного
    ///   параметра — так выглядят ходы «bash {}», после которых агент ходит
    ///   по кругу, а в ленте остаётся только `{}`;
    /// - факт XML-стиля вместо родного JSON — верный признак того, что
    ///   сэмплинг или контекст увели модель с обученного формата.
    ///
    /// `unclosed` — поток кончился без `</tool_call>`: модель нередко
    /// заканчивает генерацию сразу после JSON, тело при этом целое.
    /// Отбрасывать такой вызов значит потерять ход, поэтому разбираем.
    fn accept_body(&mut self, unclosed: bool) {
        match parse_tool_call_body(&self.inner) {
            Some((call, format)) => {
                if unclosed {
                    tracing::debug!(
                        name = %call.name,
                        "tool_call без </tool_call> — тело целое, принимаем"
                    );
                }
                if call.arguments_json == "{}" {
                    tracing::warn!(
                        name = %call.name,
                        tool_call_body = %self.inner.trim(),
                        "tool_call без аргументов — модель не дописала параметры"
                    );
                } else if format == ToolCallFormat::AnthropicXml {
                    tracing::info!(
                        name = %call.name,
                        "tool_call в XML-стиле вместо родного JSON"
                    );
                    tracing::debug!(
                        name = %call.name,
                        tool_call_body = %self.inner.trim(),
                        "тело XML-вызова"
                    );
                }
                self.calls.push(call);
            }
            None if unclosed => tracing::warn!(
                truncated_body = %self.inner.trim(),
                "stream закончился внутри <tool_call> без </tool_call>, \
                 тело не разбирается — отбрасываем"
            ),
            None => tracing::warn!(
                tool_call_body = %self.inner.trim(),
                "failed to parse <tool_call> JSON — пропускаем"
            ),
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
    // Скобочный хвост чиним до разбора: модели устойчиво промахиваются на
    // закрытии длинного значения (`…"}}]}` вместо `…"}}}]`), и такой блок
    // раньше выбрасывался целиком — ход впустую, а модель при том же
    // префикс-KV повторяла его байт в байт.
    let (v, repaired) = crate::agent::json_repair::parse_with_repair(body.trim())?;
    if repaired {
        tracing::warn!(
            tool_call_body = %body.trim(),
            "скобки в <tool_call> закрыты не в том порядке — пересобрали хвост"
        );
    }
    let name = v.get("name")?.as_str()?.to_string();
    let arguments = match v.get("arguments") {
        Some(args) => normalize_arguments(args.clone()),
        // Обёртки `arguments` нет: модель положила параметры рядом с `name`
        // (`{"name":"bash","command":"ls"}`). Берём всё, кроме имени.
        None => {
            let mut rest = v.as_object().cloned().unwrap_or_default();
            rest.remove("name");
            normalize_arguments(serde_json::Value::Object(rest))
        }
    };
    let arguments_json = serde_json::to_string(&arguments).ok()?;
    Some(RawToolCall { name, arguments_json })
}

/// Приводит `arguments` к тому, что ждёт executor: JSON-объект с полями
/// инструмента. Модели устойчиво промахиваются тремя способами, и каждый
/// раньше заканчивался «Missing required field» с зацикливанием на месте:
///
/// - `arguments` — строка с экранированным JSON внутри
///   (`"arguments": "{\"command\": \"ls\"}"`), ровно то, что правило 7
///   системного промпта запрещает, но flash-next так пишет и без штрафов;
/// - лишняя вложенность `{"arguments": {"command": …}}` — параметр назван
///   именем обёртки (в XML-стиле — `<parameter name="arguments">`);
/// - пустой параметр-заглушка `"arguments": "\n\n"` рядом с настоящими
///   полями.
///
/// Всё это разворачивается без потерь: у инструментов нет собственного
/// параметра с именем `arguments`, так что путаницы быть не может.
fn normalize_arguments(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) => match lenient_json(&s) {
            Some(inner @ Value::Object(_)) => normalize_arguments(inner),
            _ => Value::String(s),
        },
        Value::Object(mut map) => {
            let blank = map
                .get("arguments")
                .and_then(Value::as_str)
                .is_some_and(|s| s.trim().is_empty());
            if blank {
                map.remove("arguments");
                return Value::Object(map);
            }
            if map.len() == 1 {
                if let Some(inner) = map.remove("arguments") {
                    let inner = normalize_arguments(inner);
                    if inner.is_object() {
                        return inner;
                    }
                    map.insert("arguments".to_string(), inner);
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

/// Парсер для Anthropic-стиля. Синтаксис у него два поколения, и модели
/// пишут оба:
///
/// 1. **Атрибутный** — `<invoke name="NAME"><parameter name="KEY">VAL</parameter></invoke>`,
///    опционально завёрнутый в `<function_calls>`. Именно его выдаёт
///    `qwen3.8-flash-next`: до поддержки здесь такой вызов целиком уходил в
///    мусор («тело не разбирается — отбрасываем»), ход заканчивался без
///    действия, и агент только объявлял намерение.
/// 2. **Tag-in-name** — `<function=NAME><parameter=KEY>VAL</parameter></function>`,
///    включая глюк токенайзера `<functionfunction=NAME>` с удвоенным
///    `function`.
///
/// В обоих закрывающий тег может отсутствовать: поток обрывается на EOS.
/// VAL пробуем сначала распарсить как JSON (для чисел/массивов/объектов),
/// иначе кладём как строку.
fn parse_tool_call_xml_anthropic(body: &str) -> Option<RawToolCall> {
    let body = body.trim();
    parse_xml_invoke_style(body).or_else(|| parse_xml_function_eq_style(body))
}

/// `<invoke name="NAME">` + `<parameter name="KEY">VAL</parameter>`.
fn parse_xml_invoke_style(body: &str) -> Option<RawToolCall> {
    let start = body.find("<invoke")?;
    let after_tag = &body[start + "<invoke".len()..];
    let tag_end = after_tag.find('>')?;
    let name = xml_attr(&after_tag[..tag_end], "name")?;
    if name.is_empty() {
        return None;
    }
    let args = parse_xml_params_attr(&after_tag[tag_end + 1..]);
    let arguments_json =
        serde_json::to_string(&normalize_arguments(serde_json::Value::Object(args))).ok()?;
    Some(RawToolCall { name, arguments_json })
}

/// Значение атрибута из содержимого открывающего тега. Принимает `key="val"`,
/// `key='val'` и `key=val` без кавычек.
fn xml_attr(attrs: &str, key: &str) -> Option<String> {
    let pos = attrs.find(key)?;
    let rest = attrs[pos + key.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'');
    match quote {
        Some(q) => {
            let rest = &rest[q.len_utf8()..];
            let end = rest.find(q)?;
            Some(rest[..end].trim().to_string())
        }
        None => {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            Some(rest[..end].trim().to_string())
        }
    }
}

/// Пары `<parameter name="KEY">VAL</parameter>` подряд.
fn parse_xml_params_attr(mut cursor: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut args = serde_json::Map::new();
    while let Some(p_start) = cursor.find("<parameter") {
        let after_p = &cursor[p_start + "<parameter".len()..];
        let Some(tag_end) = after_p.find('>') else { break };
        let Some(key) = xml_attr(&after_p[..tag_end], "name") else { break };
        let value_start = tag_end + 1;
        let (raw_val, next) = match after_p[value_start..].find("</parameter>") {
            Some(rel) => (
                &after_p[value_start..value_start + rel],
                &after_p[value_start + rel + "</parameter>".len()..],
            ),
            // Поток оборвался внутри значения: берём хвост до закрытия
            // вызова — лучше вызвать с целым аргументом, чем потерять ход.
            None => (cut_at_invoke_close(&after_p[value_start..]), ""),
        };
        if !key.is_empty() {
            args.insert(key, xml_param_value(raw_val));
        }
        if next.is_empty() {
            break;
        }
        cursor = next;
    }
    args
}

/// То же для tag-in-name стиля: хвост до `</function>` / `</tool_call>`.
fn cut_at_function_close(v: &str) -> &str {
    let end = v
        .find("</function")
        .or_else(|| v.find("</tool_call"))
        .unwrap_or(v.len());
    &v[..end]
}

/// Хвост значения до `</invoke>` / `</function_calls>` — на случай, когда
/// `</parameter>` модель не дописала.
fn cut_at_invoke_close(v: &str) -> &str {
    let end = v
        .find("</invoke")
        .or_else(|| v.find("</function_calls"))
        .unwrap_or(v.len());
    &v[..end]
}

/// Значение параметра: сначала как JSON (числа, массивы, объекты), иначе
/// строкой. Перевод строки сразу за `>` и перед закрывающим тегом — это
/// вёрстка XML, а не часть аргумента; внутренние отступы (важные, например,
/// для содержимого файла) не трогаем.
fn xml_param_value(raw: &str) -> serde_json::Value {
    let v = raw.strip_prefix('\n').unwrap_or(raw);
    let v = v.strip_suffix('\n').unwrap_or(v);
    lenient_json(v).unwrap_or_else(|| serde_json::Value::String(v.to_string()))
}

/// JSON из текста, который модель могла слегка испортить: строгий разбор,
/// затем первое целое значение с отброшенным хвостом (лишняя `}` в конце —
/// `{"command":"…"}}` из живого прогона 03.09.2026), затем починка
/// скобочного хвоста ([`crate::agent::json_repair`]). `None` — это не JSON.
fn lenient_json(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        return Some(v);
    }
    if t.starts_with('{') || t.starts_with('[') {
        let mut stream = serde_json::Deserializer::from_str(t).into_iter::<serde_json::Value>();
        if let Some(Ok(v)) = stream.next() {
            return Some(v);
        }
    }
    crate::agent::json_repair::parse_with_repair(t).map(|(v, _)| v)
}

/// `<function=NAME>` + `<parameter=KEY>VAL</parameter>`.
fn parse_xml_function_eq_style(body: &str) -> Option<RawToolCall> {
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
        // Без `</parameter>` (модель оборвала блок сразу после значения —
        // flash-next 04.09.2026: `<parameter=arguments>{…}}` и конец) берём
        // хвост до закрытия функции: вызов с целым аргументом лучше пустого.
        let (raw_val, next) = match after_p[value_start..].find("</parameter>") {
            Some(rel) => (
                &after_p[value_start..value_start + rel],
                &after_p[value_start + rel + "</parameter>".len()..],
            ),
            None => (cut_at_function_close(&after_p[value_start..]), ""),
        };
        if !key.is_empty() {
            args.insert(key, xml_param_value(raw_val));
        }
        if next.is_empty() {
            break;
        }
        cursor = next;
    }

    let arguments_json =
        serde_json::to_string(&normalize_arguments(serde_json::Value::Object(args))).ok()?;
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

    /// `arguments` строкой с JSON внутри (так пишет flash-next даже на
    /// temp 0) — разворачивается в объект.
    #[test]
    fn stringified_arguments_are_unwrapped() {
        let mut p = ToolCallParser::new();
        p.feed(r#"<tool_call>{"name":"bash","arguments":"{\"command\": \"ls -la\"}"}</tool_call>"#);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments_json, r#"{"command":"ls -la"}"#);
    }

    /// Лишняя вложенность `{"arguments": {...}}` (чат 28.08) снимается.
    #[test]
    fn nested_arguments_wrapper_is_unwrapped() {
        let mut p = ToolCallParser::new();
        p.feed(r#"<tool_call>{"name":"bash","arguments":{"arguments":{"command":"pwd"}}}</tool_call>"#);
        let (calls, _) = p.finish();
        assert_eq!(calls[0].arguments_json, r#"{"command":"pwd"}"#);
    }

    /// Пустая заглушка `"arguments": "\n\n"` рядом с настоящим полем
    /// выбрасывается, а одна — даёт пустой объект.
    #[test]
    fn blank_arguments_stub_is_dropped() {
        let mut p = ToolCallParser::new();
        p.feed("<tool_call>{\"name\":\"bash\",\"arguments\":{\"arguments\":\"\\n\\n\",\"command\":\"id\"}}</tool_call>");
        let (calls, _) = p.finish();
        assert_eq!(calls[0].arguments_json, r#"{"command":"id"}"#);

        let mut p = ToolCallParser::new();
        p.feed("<tool_call>{\"name\":\"bash\",\"arguments\":{\"arguments\":\"\\n\"}}</tool_call>");
        let (calls, _) = p.finish();
        assert_eq!(calls[0].arguments_json, "{}");
    }

    /// Живой случай 03.09.2026 (qwen3.8-27b, temp 0.6): `arguments`
    /// строкой, внутри которой лишняя закрывающая скобка.
    #[test]
    fn stringified_arguments_with_extra_brace_are_unwrapped() {
        let raw = r#"{"arguments":"{\"command\":\"cd /tmp && echo \\\"---\\\" && wc -l a b\"}}"}"#;
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        let out = normalize_arguments(v);
        assert_eq!(
            out,
            serde_json::json!({"command": "cd /tmp && echo \"---\" && wc -l a b"})
        );

        let mut p = ToolCallParser::new();
        p.feed("<tool_call><function=bash><parameter=arguments>{\"command\":\"ls\"}}</parameter></function></tool_call>");
        let (calls, _) = p.finish();
        assert_eq!(calls[0].arguments_json, r#"{"command":"ls"}"#);
    }

    /// flash-next 04.09.2026: `<function=web><parameter=arguments>{…}}` и
    /// конец потока — ни `</parameter>`, ни `</function>`.
    #[test]
    fn function_style_without_closing_parameter_keeps_value() {
        let mut p = ToolCallParser::new();
        p.feed("<tool_call><function=web>\n<parameter=arguments>\n{\"action\":\"search\",\"query\":\"redmi\",\"max_results\":10}}\n");
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web");
        // Порядок ключей — как их написала модель (serde_json собран с
        // `preserve_order`), это же уходит в шаблон.
        assert_eq!(
            calls[0].arguments_json,
            r#"{"action":"search","query":"redmi","max_results":10}"#
        );
    }

    /// Параметры рядом с `name`, без обёртки `arguments`.
    #[test]
    fn parameters_next_to_name_become_arguments() {
        let mut p = ToolCallParser::new();
        p.feed(r#"<tool_call>{"name":"bash","command":"uname -a"}</tool_call>"#);
        let (calls, _) = p.finish();
        assert_eq!(calls[0].arguments_json, r#"{"command":"uname -a"}"#);
    }

    /// XML-стиль с параметром `arguments` (модель назвала параметр именем
    /// обёртки) — тоже разворачивается.
    #[test]
    fn xml_arguments_parameter_is_unwrapped() {
        let mut p = ToolCallParser::new();
        p.feed(
            "<tool_call><invoke name=\"bash\"><parameter name=\"arguments\">\
             {\"command\": \"ls\"}</parameter></invoke></tool_call>",
        );
        let (calls, _) = p.finish();
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].arguments_json, r#"{"command":"ls"}"#);
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

    /// Реальный вызов из чата: `data` закрыт, элемент массива — нет, лишняя
    /// `}` уехала за `]`. Раньше парсер выбрасывал такой блок целиком, и ход
    /// уходил впустую.
    #[test]
    fn out_of_order_brackets_are_repaired() {
        let mut p = ToolCallParser::new();
        p.feed(
            "<tool_call>\n{\"name\":\"pipelines\",\"arguments\":{\"action\":\"apply\",\
             \"set_state\":[{\"node\":9,\"state\":{\"kind\":\"TextView\",\
             \"data\":{\"output_text\":\"[verse]\"}}]}}\n</tool_call>",
        );
        let (calls, _tail) = p.finish();
        assert_eq!(calls.len(), 1, "вызов принят после починки скобок");
        assert_eq!(calls[0].name, "pipelines");
        assert!(calls[0].arguments_json.contains("\"node\":9"), "{}", calls[0].arguments_json);
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

    /// Ровно то тело, на котором 01.09 ход уходил впустую: модель пишет
    /// атрибутный Anthropic-синтаксис и обрывает поток, не дописав
    /// `</tool_call>`.
    #[test]
    fn anthropic_invoke_style_unclosed_stream() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call>\n<invoke name=\"bash\">\n\
            <parameter name=\"command\">cd /home/master/Projects/2027/quitsmoke \
            && find src styles -type f | sort</parameter>\n\
            </invoke>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1, "вызов не должен теряться");
        assert_eq!(calls[0].name, "bash");
        assert!(calls[0].arguments_json.contains("find src styles -type f"));
    }

    #[test]
    fn anthropic_invoke_style_with_function_calls_wrapper() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><function_calls>\
            <invoke name=\"subagent\">\
            <parameter name=\"task\">Do X</parameter>\
            <parameter name=\"depth\">2</parameter>\
            </invoke></function_calls></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "subagent");
        assert!(calls[0].arguments_json.contains("\"task\":\"Do X\""));
        assert!(calls[0].arguments_json.contains("\"depth\":2"));
    }

    /// Значение на своей строке: перевод строки от вёрстки в аргумент не течёт.
    #[test]
    fn anthropic_invoke_style_multiline_value_keeps_inner_layout() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><invoke name=\"bash\">\
            <parameter name=\"command\">\nls -la\n  nested\n</parameter>\
            </invoke></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].arguments_json.contains("\"command\":\"ls -la\\n  nested\""),
            "было: {}",
            calls[0].arguments_json
        );
    }

    /// Оборвался прямо в значении, без `</parameter>` и `</invoke>`.
    #[test]
    fn anthropic_invoke_style_truncated_inside_value() {
        let mut p = ToolCallParser::new();
        let _ = p.feed("<tool_call><invoke name=\"bash\">\
            <parameter name=\"command\">ls -la");
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert!(calls[0].arguments_json.contains("\"command\":\"ls -la\""));
    }

    #[test]
    fn anthropic_invoke_style_single_quoted_name() {
        let mut p = ToolCallParser::new();
        let chunk = "<tool_call><invoke name='web'>\
            <parameter name='url'>https://example.com</parameter>\
            </invoke></tool_call>";
        let _ = p.feed(chunk);
        let (calls, _) = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web");
        assert!(calls[0].arguments_json.contains("https://example.com"));
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
