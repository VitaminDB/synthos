//! Разбор аргументов инструмента `notes`: поля JSON, которые модели
//! шлют строкой, числом или списком через запятую.

use super::*;

/// Непустая строка (trim). Числа тоже принимаем строкой — модели шлют id
/// и индексы как попало.
pub(super) fn str_field<'a>(v: &'a Json, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Ссылка на блок: строка либо число (индекс).
pub(super) fn ref_field(v: &Json, key: &str) -> Option<String> {
    match v.get(key)? {
        Json::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Строка как есть (возможно пустая — «очистить»); `null` — поля нет.
pub(super) fn raw_string(v: &Json, key: &str) -> Option<String> {
    match v.get(key)? {
        Json::String(s) => Some(s.clone()),
        Json::Null => None,
        other => Some(other.to_string()),
    }
}

pub(super) fn bool_field(v: &Json, key: &str) -> Option<bool> {
    match v.get(key)? {
        Json::Bool(b) => Some(*b),
        Json::String(s) => Some(matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        )),
        Json::Number(n) => Some(n.as_i64().is_some_and(|i| i != 0)),
        _ => None,
    }
}

pub(super) fn usize_field(v: &Json, key: &str) -> Option<usize> {
    match v.get(key)? {
        Json::Number(n) => n.as_u64().map(|x| x as usize),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn f32_field(v: &Json, key: &str) -> Option<f32> {
    match v.get(key)? {
        Json::Number(n) => n.as_f64().map(|x| x as f32),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Список строк: JSON-массив либо строка через запятую / перевод строки.
pub(super) fn list_field(v: &Json, key: &str) -> Option<Vec<String>> {
    match v.get(key)? {
        Json::Array(a) => Some(
            a.iter()
                .filter_map(|x| match x {
                    Json::String(s) => Some(s.trim().to_string()),
                    Json::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        Json::String(s) => Some(
            s.split([',', '\n'])
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
        ),
        Json::Null => None,
        _ => None,
    }
}
