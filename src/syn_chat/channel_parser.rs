//! Разбор канального протокола Muse Glimmer (ATEM) в потоке генерации.
//!
//! В отличие от ChatML, где ход ассистента — один непрерывный текст, здесь
//! модель за один ход пишет несколько сообщений, каждое со своим адресатом:
//!
//! ```text
//! <|start|>assistant to=self<|message|>размышления<|eom|>
//! <|start|>assistant to=web.search<|message|><atem:function_calls>…<|eom|>
//! <|start|>assistant to=user<|message|>ответ пользователю<|eot|>
//! ```
//!
//! Prompt заканчивается на `<|start|>assistant` (см. `add_generation_prompt`
//! в chat-шаблоне), поэтому генерация стартует прямо в заголовке: первые
//! токены — ` to=self`, и только после `<|message|>` начинается тело.
//!
//! Ловушка, ради которой этот модуль и появился: `LlmTokenizer::decode`
//! декодирует со `skip_special_tokens`, так что `<|start|>`, `<|message|>` и
//! `<|eom|>` в дельтах НЕ видны — а `to=self` виден, потому что это обычный
//! текст. Без разбора по id токенов заголовок утекал прямо в реплику
//! (`to=userПривет!`), а оттуда — в историю и в следующий prompt.
//!
//! Поэтому парсер работает по id спецтокенов (их даёт callback
//! `generate_streaming`), а текст дельт раскладывает по каналам:
//! `to=self` → reasoning, `to=user` → тело ответа, всё остальное —
//! tool-call (адресат и есть имя функции).

use synaptix::facade::llm::LlmTokenizer;

use crate::syn_chat::tool_parser::RawToolCall;

/// Начало заголовка сообщения — `<|start|>`.
const TOK_START: &str = "<|start|>";
/// Конец заголовка, начало тела — `<|message|>`.
const TOK_MESSAGE: &str = "<|message|>";
/// Конец сообщения внутри хода — за ним модель пишет следующий заголовок.
const TOK_EOM: &str = "<|eom|>";
/// Конец хода целиком (он же в `eos_ids`, так что генерация тут и встанет).
const TOK_EOT: &str = "<|eot|>";

/// Открытие ATEM-блока tool-вызовов.
const ATEM_OPEN: &str = "<atem:function_calls>";
/// Закрытие ATEM-блока — оно же stop-sequence для генерации.
pub const ATEM_CLOSE: &str = "</atem:function_calls>";

/// Id спецтокенов канального протокола. `None` из [`Self::detect`] означает
/// «модель не канальная» — работает обычный ChatML-путь с `<think>`.
#[derive(Debug, Clone, Copy)]
pub struct ChannelIds {
    start: u32,
    message: u32,
    eom: u32,
    eot: u32,
}

impl ChannelIds {
    /// Пробует резолвить спецтокены протокола в словаре модели.
    ///
    /// Признак канальной модели — что `<|start|>` и `<|message|>` кодируются
    /// ровно одним токеном: у ChatML-моделей этих строк в словаре нет и
    /// токенайзер разбирает их на куски (`<`, `|`, `start`, …).
    pub fn detect(tokenizer: &LlmTokenizer) -> Option<Self> {
        let single = |s: &str| -> Option<u32> {
            match tokenizer.encode(s) {
                Ok(ids) if ids.len() == 1 => Some(ids[0]),
                _ => None,
            }
        };
        Some(Self {
            start: single(TOK_START)?,
            message: single(TOK_MESSAGE)?,
            eom: single(TOK_EOM)?,
            eot: single(TOK_EOT)?,
        })
    }
}

/// Куда уходит текст текущего сообщения.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Channel {
    /// `to=self` — reasoning, показывается в блоке «Размышления».
    SelfChannel,
    /// `to=user` (или адресат не указан) — обычный ответ.
    User,
    /// `to=<имя функции>` — tool-call в ATEM-разметке.
    Tool(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Между `<|start|>` и `<|message|>`: копим заголовок, в UI ничего не идёт.
    Header,
    /// После `<|message|>`: тело сообщения течёт в канал.
    Body(Channel),
}

/// Результат разбора одной дельты: что дописать в тело ответа, что — в
/// блок размышлений, что — в live-превью tool-вызова. Все части могут быть
/// пустыми.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChannelSplit {
    pub body: String,
    pub thinking: String,
    /// Текст, который модель пишет как вызов инструмента (тело ATEM-блока
    /// и «сырьё» служебного канала) — для live-превью команды в UI.
    pub tool: String,
}

/// Инкрементальный парсер канального протокола.
///
/// Создаётся один на turn генерации: prompt заканчивается заголовком
/// `<|start|>assistant`, поэтому стартовое состояние — [`State::Header`].
pub struct ChannelParser {
    ids: ChannelIds,
    state: State,
    /// Текст заголовка после `<|start|>` (` to=self`, `assistant to=user`, …).
    header: String,
    /// Разбор ATEM-блоков в теле сообщения.
    atem: AtemParser,
    /// Ход завершён `<|eot|>` — дальше писать некуда.
    done: bool,
}

impl ChannelParser {
    pub fn new(ids: ChannelIds) -> Self {
        Self {
            ids,
            state: State::Header,
            header: String::new(),
            atem: AtemParser::new(),
            done: false,
        }
    }

    /// Подать очередной токен: `id` — чтобы поймать невидимые в тексте
    /// спецтокены, `delta` — декодированный текст (для спецтокенов пустой).
    pub fn feed(&mut self, id: u32, delta: &str) -> ChannelSplit {
        if id == self.ids.message {
            let channel = parse_recipient(&self.header);
            self.header.clear();
            self.state = State::Body(channel);
            return ChannelSplit::default();
        }
        if id == self.ids.eom || id == self.ids.start || id == self.ids.eot {
            self.atem.close_block();
            self.state = State::Header;
            self.header.clear();
            if id == self.ids.eot {
                self.done = true;
            }
            return ChannelSplit::default();
        }
        if delta.is_empty() || self.done {
            return ChannelSplit::default();
        }

        match &self.state {
            State::Header => {
                self.header.push_str(delta);
                ChannelSplit::default()
            }
            // Модель иногда открывает ATEM-блок прямо в канале user или self
            // (адресат при этом остаётся прежним), поэтому tool-разметку
            // выцеживаем из любого канала, а не только из `to=<функция>`.
            State::Body(Channel::User) => {
                let (clean, tool) = self.atem.feed(delta, None);
                ChannelSplit { body: clean, thinking: String::new(), tool }
            }
            State::Body(Channel::SelfChannel) => {
                let (clean, tool) = self.atem.feed(delta, None);
                ChannelSplit { body: String::new(), thinking: clean, tool }
            }
            State::Body(Channel::Tool(name)) => {
                let name = name.clone();
                // Служебный канал: текст вокруг блока — «сырьё» вызова.
                // Пользователю в ответ он не идёт, но в live-превью — да.
                let (clean, tool) = self.atem.feed(delta, Some(&name));
                let mut merged = clean;
                merged.push_str(&tool);
                ChannelSplit { body: String::new(), thinking: String::new(), tool: merged }
            }
        }
    }

    /// Есть ли уже собранный tool-вызов и закрыт ли его блок — сигнал
    /// agent-loop'у, что стрим можно прерывать.
    pub fn has_closed_call(&self) -> bool {
        self.atem.has_closed_call()
    }

    /// Собранные tool-вызовы. Незакрытый блок отбрасывается внутри
    /// [`AtemParser`], как и в ChatML-пути.
    pub fn finish(self) -> Vec<RawToolCall> {
        self.atem.finish()
    }
}

/// Собирает реплику ассистента для истории: текст, адресованный
/// пользователю, плюс ATEM-блок сделанных вызовов.
///
/// Сырой текст хода для этого не годится — в нём заголовки каналов
/// (` to=user`), которые chat-шаблон при следующем рендере припишет заново,
/// и модель начинает повторять их в ответах. Reasoning в историю не идёт:
/// шаблон отдаёт его отдельным полем, а внутри `content` он бы выглядел
/// как обычный ответ пользователю.
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
        if let Ok(serde_json::Value::Object(args)) =
            serde_json::from_str::<serde_json::Value>(&call.arguments_json)
        {
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

/// Разбирает заголовок сообщения в адресата.
///
/// На входе то, что модель написала между `<|start|>` и `<|message|>`:
/// ` to=self`, `assistant to=web.search`, иногда просто `assistant`.
/// Отсутствие `to=` — это канал пользователя (так же трактует chat-шаблон).
fn parse_recipient(header: &str) -> Channel {
    let Some(pos) = header.find("to=") else {
        return Channel::User;
    };
    let rest = &header[pos + 3..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '<')
        .unwrap_or(rest.len());
    match rest[..end].trim() {
        "self" => Channel::SelfChannel,
        "user" | "" => Channel::User,
        name => Channel::Tool(name.to_string()),
    }
}

/// Стейт-машина ATEM-разметки tool-вызовов:
///
/// ```text
/// <atem:function_calls>
/// <atem:invoke name="web.search">
/// <atem:parameter name="query">погода</atem:parameter>
/// </atem:invoke>
/// </atem:function_calls>
/// ```
///
/// Наружу отдаёт «чистый» текст (без блока), внутрь — накапливает вызовы.
/// Частичный префикс тега на стыке дельт придерживается, чтобы в UI не
/// мигало `<atem:fun`.
struct AtemParser {
    inside: bool,
    /// Снаружи блока — потенциальный префикс `<atem:function_calls>`;
    /// внутри — потенциальный префикс закрывающего тега (тело блока сразу
    /// уходит в `inner` и tool-дельту).
    buf: String,
    /// Тело текущего блока, накопленное для разбора на закрытии. Дублирует
    /// то, что уже отдано наружу как tool-дельта.
    inner: String,
    calls: Vec<RawToolCall>,
    /// Хотя бы один блок закрыт корректно.
    closed_block: bool,
}

impl AtemParser {
    fn new() -> Self {
        Self {
            inside: false,
            buf: String::new(),
            inner: String::new(),
            calls: Vec::new(),
            closed_block: false,
        }
    }

    /// `fallback_name` — адресат канала: им подписывается вызов, если в
    /// `<atem:invoke>` имя не указано. Возвращает `(clean, tool)`: чистый
    /// текст вне блока и live-дельту его содержимого (без самих тегов).
    fn feed(&mut self, delta: &str, fallback_name: Option<&str>) -> (String, String) {
        let mut out = String::new();
        let mut tool_out = String::new();
        let mut work = std::mem::take(&mut self.buf);
        work.push_str(delta);

        loop {
            if self.inside {
                if let Some(idx) = work.find(ATEM_CLOSE) {
                    tool_out.push_str(&work[..idx]);
                    self.inner.push_str(&work[..idx]);
                    self.calls.extend(parse_atem_invokes(&self.inner, fallback_name));
                    self.inner.clear();
                    self.closed_block = true;
                    work.drain(..idx + ATEM_CLOSE.len());
                    self.inside = false;
                    continue;
                }
                // Закрывающий тег не найден: придерживаем его возможный
                // префикс, остальное — в тело и live-дельту.
                let hold = potential_prefix_len(&work, ATEM_CLOSE);
                let emit_end = work.len() - hold;
                if emit_end > 0 {
                    tool_out.push_str(&work[..emit_end]);
                    self.inner.push_str(&work[..emit_end]);
                    work.drain(..emit_end);
                }
                break;
            }
            if let Some(idx) = work.find(ATEM_OPEN) {
                out.push_str(&work[..idx]);
                work.drain(..idx + ATEM_OPEN.len());
                self.inside = true;
                continue;
            }
            let hold = potential_prefix_len(&work, ATEM_OPEN);
            let emit_end = work.len() - hold;
            if emit_end > 0 {
                out.push_str(&work[..emit_end]);
                work.drain(..emit_end);
            }
            break;
        }

        self.buf = work;
        (out, tool_out)
    }

    /// Конец сообщения: незакрытый блок отбрасываем (модель оборвалась),
    /// придержанный хвост чистого текста тоже — он уже вне канала.
    fn close_block(&mut self) {
        if self.inside && !(self.inner.trim().is_empty() && self.buf.trim().is_empty()) {
            log::warn!("[syn_chat] ATEM-блок оборван без закрывающего тега — пропускаем");
        }
        self.inside = false;
        self.buf.clear();
        self.inner.clear();
    }

    fn has_closed_call(&self) -> bool {
        self.closed_block && !self.calls.is_empty()
    }

    fn finish(mut self) -> Vec<RawToolCall> {
        self.close_block();
        self.calls
    }
}

/// Разбирает тело `<atem:function_calls>` на отдельные `<atem:invoke>`.
fn parse_atem_invokes(body: &str, fallback_name: Option<&str>) -> Vec<RawToolCall> {
    let mut out = Vec::new();
    let mut cursor = body;
    while let Some(start) = cursor.find("<atem:invoke") {
        let after = &cursor[start + "<atem:invoke".len()..];
        let Some(tag_end) = after.find('>') else { break };
        let name = attr_value(&after[..tag_end], "name")
            .or_else(|| fallback_name.map(str::to_string))
            .unwrap_or_default();
        let inner_start = tag_end + 1;
        let (inner, next) = match after[inner_start..].find("</atem:invoke>") {
            Some(rel) => (
                &after[inner_start..inner_start + rel],
                inner_start + rel + "</atem:invoke>".len(),
            ),
            // Закрывающего тега нет — берём остаток как тело последнего вызова.
            None => (&after[inner_start..], after.len()),
        };
        if !name.is_empty() {
            out.push(RawToolCall { name, arguments_json: parse_atem_params(inner) });
        } else {
            log::warn!("[syn_chat] <atem:invoke> без имени функции — пропускаем");
        }
        cursor = &after[next..];
    }
    out
}

/// Пары `<atem:parameter name="k">v</atem:parameter>` → JSON-объект.
/// Значение сначала пробуем как JSON (числа, списки, объекты), иначе —
/// строкой как есть: шаблон пишет скаляры без кавычек.
fn parse_atem_params(inner: &str) -> String {
    let mut args = serde_json::Map::new();
    let mut cursor = inner;
    while let Some(start) = cursor.find("<atem:parameter") {
        let after = &cursor[start + "<atem:parameter".len()..];
        let Some(tag_end) = after.find('>') else { break };
        let key = attr_value(&after[..tag_end], "name").unwrap_or_default();
        let value_start = tag_end + 1;
        let Some(rel) = after[value_start..].find("</atem:parameter>") else { break };
        let raw = &after[value_start..value_start + rel];
        if !key.is_empty() {
            let v = serde_json::from_str::<serde_json::Value>(raw.trim())
                .unwrap_or_else(|_| serde_json::Value::String(raw.to_string()));
            args.insert(key, v);
        }
        cursor = &after[value_start + rel + "</atem:parameter>".len()..];
    }
    serde_json::Value::Object(args).to_string()
}

/// Значение атрибута `key="..."` из текста открывающего тега.
fn attr_value(tag: &str, key: &str) -> Option<String> {
    let pat = format!("{key}=\"");
    let start = tag.find(&pat)? + pat.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

/// Длина суффикса `text`, который может быть началом `tag` — его придерживаем
/// до следующей дельты, чтобы не показать пользователю обрывок тега.
fn potential_prefix_len(text: &str, tag: &str) -> usize {
    let bytes = text.as_bytes();
    let tag_bytes = tag.as_bytes();
    let max_check = tag_bytes.len().saturating_sub(1).min(bytes.len());
    for hold in (1..=max_check).rev() {
        let start = bytes.len() - hold;
        if !text.is_char_boundary(start) {
            continue;
        }
        if tag_bytes.starts_with(&bytes[start..]) {
            return hold;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: u32 = 1;
    const MESSAGE: u32 = 2;
    const EOM: u32 = 3;
    const EOT: u32 = 4;
    /// Любой id обычного текстового токена.
    const TEXT: u32 = 100;

    fn ids() -> ChannelIds {
        ChannelIds { start: START, message: MESSAGE, eom: EOM, eot: EOT }
    }

    /// Сценарий из стрима: (id, delta). Спецтокены приходят с пустой дельтой.
    fn run(script: &[(u32, &str)]) -> (ChannelSplit, Vec<RawToolCall>) {
        let mut p = ChannelParser::new(ids());
        let mut acc = ChannelSplit::default();
        for (id, delta) in script {
            let s = p.feed(*id, delta);
            acc.body.push_str(&s.body);
            acc.thinking.push_str(&s.thinking);
            acc.tool.push_str(&s.tool);
        }
        let calls = p.finish();
        (acc, calls)
    }

    #[test]
    fn header_does_not_leak_into_body() {
        let (out, _) = run(&[
            (TEXT, " to"),
            (TEXT, "=user"),
            (MESSAGE, ""),
            (TEXT, "Привет!"),
            (EOT, ""),
        ]);
        assert_eq!(out.body, "Привет!");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn self_channel_goes_to_thinking() {
        let (out, _) = run(&[
            (TEXT, " to=self"),
            (MESSAGE, ""),
            (TEXT, "надо подумать"),
            (EOM, ""),
            (START, ""),
            (TEXT, "assistant to=user"),
            (MESSAGE, ""),
            (TEXT, "ответ"),
            (EOT, ""),
        ]);
        assert_eq!(out.thinking, "надо подумать");
        assert_eq!(out.body, "ответ");
    }

    #[test]
    fn missing_recipient_is_user_channel() {
        let (out, _) = run(&[(TEXT, "assistant"), (MESSAGE, ""), (TEXT, "текст"), (EOT, "")]);
        assert_eq!(out.body, "текст");
    }

    #[test]
    fn tool_channel_collects_call_and_hides_text() {
        let (out, calls) = run(&[
            (TEXT, " to=web.search"),
            (MESSAGE, ""),
            (TEXT, "<atem:function_calls>\n<atem:invoke name=\"web.search\">\n"),
            (TEXT, "<atem:parameter name=\"query\">погода в Томске</atem:parameter>\n"),
            (TEXT, "<atem:parameter name=\"limit\">5</atem:parameter>\n"),
            (TEXT, "</atem:invoke>\n</atem:function_calls>"),
            (EOM, ""),
        ]);
        assert_eq!(out.body, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web.search");
        let v: serde_json::Value = serde_json::from_str(&calls[0].arguments_json).unwrap();
        assert_eq!(v["query"], "погода в Томске");
        // Скаляр без кавычек разбирается как число, а не как строка.
        assert_eq!(v["limit"], 5);
    }

    #[test]
    fn atem_block_inside_user_channel_is_extracted() {
        let (out, calls) = run(&[
            (TEXT, " to=user"),
            (MESSAGE, ""),
            (TEXT, "сейчас гляну "),
            (TEXT, "<atem:function_calls><atem:invoke name=\"bash\">"),
            (TEXT, "<atem:parameter name=\"command\">ls</atem:parameter>"),
            (TEXT, "</atem:invoke></atem:function_calls>"),
            (EOT, ""),
        ]);
        assert_eq!(out.body, "сейчас гляну ");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn partial_open_tag_is_held_back() {
        let mut p = ChannelParser::new(ids());
        p.feed(MESSAGE, "");
        assert_eq!(p.feed(TEXT, "готово <atem:fun").body, "готово ");
        assert_eq!(p.feed(TEXT, "ction_calls><atem:invoke name=\"x\"></atem:invoke>").body, "");
    }

    #[test]
    fn unterminated_block_is_dropped() {
        let (out, calls) = run(&[
            (TEXT, " to=bash"),
            (MESSAGE, ""),
            (TEXT, "<atem:function_calls><atem:invoke name=\"bash\">"),
        ]);
        assert_eq!(out.body, "");
        assert!(calls.is_empty());
    }

    #[test]
    fn invoke_name_falls_back_to_recipient() {
        let (_, calls) = run(&[
            (TEXT, " to=kb_search"),
            (MESSAGE, ""),
            (TEXT, "<atem:function_calls><atem:invoke>"),
            (TEXT, "<atem:parameter name=\"q\">rust</atem:parameter>"),
            (TEXT, "</atem:invoke></atem:function_calls>"),
            (EOM, ""),
        ]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "kb_search");
    }

    #[test]
    fn history_text_drops_channel_headers() {
        let (out, calls) = run(&[
            (TEXT, " to=self"),
            (MESSAGE, ""),
            (TEXT, "надо посмотреть файлы"),
            (EOM, ""),
            (START, ""),
            (TEXT, "assistant to=bash"),
            (MESSAGE, ""),
            (TEXT, "<atem:function_calls><atem:invoke name=\"bash\">"),
            (TEXT, "<atem:parameter name=\"command\">ls -la</atem:parameter>"),
            (TEXT, "</atem:invoke></atem:function_calls>"),
            (EOM, ""),
        ]);
        let text = rebuild_turn_text(&out.body, &calls);
        assert!(!text.contains("to=self"), "{text}");
        assert!(!text.contains("надо посмотреть файлы"), "{text}");
        assert!(text.contains("<atem:invoke name=\"bash\">"), "{text}");
        assert!(text.contains("<atem:parameter name=\"command\">ls -la</atem:parameter>"), "{text}");
        assert!(text.ends_with(ATEM_CLOSE), "{text}");
    }

    #[test]
    fn history_text_without_calls_is_plain_answer() {
        assert_eq!(rebuild_turn_text(" Привет!\n", &[]), "Привет!");
    }

    #[test]
    fn tool_delta_streams_from_tool_channel() {
        let (out, calls) = run(&[
            (TEXT, " to=bash"),
            (MESSAGE, ""),
            (TEXT, "<atem:function_calls><atem:invoke name=\"bash\">"),
            (TEXT, "<atem:parameter name=\"command\">ls -la</atem:parameter>"),
            (TEXT, "</atem:invoke></atem:function_calls>"),
            (EOM, ""),
        ]);
        // Live-дельта — содержимое блока без внешних тегов.
        assert!(out.tool.contains("<atem:invoke name=\"bash\">"), "{}", out.tool);
        assert!(out.tool.contains("ls -la"), "{}", out.tool);
        assert!(!out.tool.contains("<atem:function_calls>"), "{}", out.tool);
        assert_eq!(out.body, "");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn tool_delta_streams_from_user_channel_block() {
        let (out, calls) = run(&[
            (TEXT, " to=user"),
            (MESSAGE, ""),
            (TEXT, "сейчас гляну "),
            (TEXT, "<atem:function_calls><atem:invoke name=\"bash\">"),
            (TEXT, "<atem:parameter name=\"command\">pwd</atem:parameter>"),
            (TEXT, "</atem:invoke></atem:function_calls>"),
            (EOT, ""),
        ]);
        assert_eq!(out.body, "сейчас гляну ");
        assert!(out.tool.contains("pwd"), "{}", out.tool);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn recipient_parsing() {
        assert_eq!(parse_recipient(" to=self"), Channel::SelfChannel);
        assert_eq!(parse_recipient("assistant to=user"), Channel::User);
        assert_eq!(parse_recipient("assistant"), Channel::User);
        assert_eq!(parse_recipient(" to=web.search "), Channel::Tool("web.search".into()));
    }
}
