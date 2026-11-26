//! Вызов инструмента из ответа модели. Разбирает вызовы служба
//! (`synaptixd::Generated::tool_calls`), здесь — их вид для истории и ленты.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawToolCall {
    pub name: String,
    pub arguments_json: String,
}
