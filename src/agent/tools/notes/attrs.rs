//! Атрибуты блока: цвета, размеры, выравнивание — разбор значений,
//! которые агент передаёт в `blocks op=set_attrs` и при вставке.

use super::*;

/// `#rrggbb` / `#rgb` / имя палитры / `none`; с `alpha` — и `#rrggbbaa`.
pub(super) fn parse_hex_color(s: &str, alpha: bool) -> Result<String, String> {
    if let Ok(c) = parse_color(s) {
        return Ok(c);
    }
    let t = s.trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    if alpha && hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(format!("#{}", hex.to_uppercase()));
    }
    Err(format!(
        "bad color \"{s}\" — use #rrggbb{}, gray | orange | green | blue | purple | red | teal or none",
        if alpha { " / #rrggbbaa" } else { "" }
    ))
}

/// Проверить и нормализовать значение атрибута блока; `Ok(None)` — снять.
pub(super) fn validate_attr(key: &str, value: &str) -> Result<Option<String>, String> {
    let v = value.trim();
    let cleared = v.is_empty() || matches!(v.to_ascii_lowercase().as_str(), "none" | "null" | "default");
    let range = |lo: f32, hi: f32| -> Result<Option<String>, String> {
        if cleared {
            return Ok(None);
        }
        let n: f32 = v.parse().map_err(|_| format!("bad \"{key}\" \"{v}\" — a number {lo}..{hi}"))?;
        if !(lo..=hi).contains(&n) {
            return Err(format!("\"{key}\" must be within {lo}..{hi}"));
        }
        Ok(Some(fnum(n)))
    };
    match key {
        "color" | "bg" | "fill" => {
            if cleared {
                return Ok(None);
            }
            parse_hex_color(v, true).map(Some)
        }
        "stroke" => {
            if v.is_empty() || matches!(v.to_ascii_lowercase().as_str(), "default" | "null") {
                return Ok(None);
            }
            if v.eq_ignore_ascii_case("none") {
                return Ok(Some("none".to_string()));
            }
            parse_hex_color(v, true).map(Some)
        }
        "size" => range(6.0, 160.0),
        "weight" => match v.to_ascii_lowercase().as_str() {
            "" | "none" | "null" | "default" => Ok(None),
            "bold" | "normal" => Ok(Some(v.to_ascii_lowercase())),
            _ => Err(format!("bad \"weight\" \"{v}\" (bold | normal)")),
        },
        "align" => match v.to_ascii_lowercase().as_str() {
            "" | "none" | "null" | "default" => Ok(None),
            "left" | "center" | "right" => Ok(Some(v.to_ascii_lowercase())),
            _ => Err(format!("bad \"align\" \"{v}\" (left | center | right)")),
        },
        "x" | "y" | "x1" | "y1" | "x2" | "y2" | "cx1" | "cy1" | "cx2" | "cy2" => range(-1.0e6, 1.0e6),
        "w" => range(40.0, 1.0e5),
        "h" => range(20.0, 1.0e5),
        "sw" => range(0.0, 40.0),
        "dash" => range(0.0, 60.0),
        "radius" => range(0.0, 200.0),
        "opacity" => range(0.0, 100.0),
        other => Err(format!(
            "unknown attribute \"{other}\" — allowed: color bg size weight align (text), x y w h (geometry), \
             fill stroke sw dash radius opacity (shape), x1 y1 x2 y2 cx1 cy1 cx2 cy2 (line ends, relative)"
        )),
    }
}

/// Ключи атрибутов, которые `set_attrs` берёт и прямо из аргументов, как
/// `pin`: 14.09.2026 модель прислала `{"op":"set_attrs","block":14,"h":2100}`
/// и получила «missing attrs».
const PLAIN_ATTR_ARGS: [&str; 23] = [
    "x", "y", "w", "h", "color", "bg", "size", "weight", "align", "fill", "stroke", "sw", "dash", "radius", "opacity", "x1",
    "y1", "x2", "y2", "cx1", "cy1", "cx2", "cy2",
];

/// Атрибуты из аргумента `attrs` (JSON-объект либо строка `{k=v …}` /
/// `k=v k=v` / JSON-текст) и из тех же ключей прямо в аргументах — `attrs`
/// важнее. `null` прямо в аргументах ничего не снимает: модели шлют пустые
/// поля схемы, и `"x": null` рядом со стилем открепил бы блок.
pub(super) fn attrs_arg(v: &Json) -> Result<Vec<(String, String)>, String> {
    let mut out = match v.get("attrs") {
        None | Some(Json::Null) => Vec::new(),
        Some(raw) => attrs_value(raw)?,
    };
    for key in PLAIN_ATTR_ARGS {
        let Some(val) = v.get(key).filter(|val| !val.is_null()) else { continue };
        if !out.iter().any(|(k, _)| k.trim().eq_ignore_ascii_case(key)) {
            out.push((key.to_string(), attr_text(val)));
        }
    }
    if out.is_empty() {
        return Err(if v.get("attrs").is_some_and(|a| !a.is_null()) {
            "\"attrs\" is empty".to_string()
        } else {
            "missing \"attrs\" (object {key: value}; empty or null value clears)".to_string()
        });
    }
    Ok(out)
}

fn attr_text(val: &Json) -> String {
    match val {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn attrs_value(raw: &Json) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    match raw {
        Json::Object(map) => {
            for (k, val) in map {
                out.push((k.clone(), attr_text(val)));
            }
        }
        Json::String(s) => {
            let t = s.trim();
            if let Ok(parsed @ Json::Object(_)) = serde_json::from_str::<Json>(t) {
                return attrs_value(&parsed);
            }
            let braced = if t.starts_with('{') { t.to_string() } else { format!("{{{t}}}") };
            let parsed = parse_attr_block(&braced).ok_or_else(|| format!("can't parse attrs \"{t}\" — pass an object"))?;
            for (k, val) in parsed.0 {
                out.push((k, val));
            }
        }
        _ => return Err("\"attrs\" must be an object {key: value}".to_string()),
    }
    Ok(out)
}

/// Применить проверенные атрибуты к блоку; возвращает описание изменений.
pub(super) fn apply_attrs(block: &mut DocBlock, pairs: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for (k, v) in pairs {
        let key = k.trim().to_ascii_lowercase();
        match validate_attr(&key, v)? {
            Some(val) => {
                block.attrs.set(key.clone(), val.clone());
                changes.push(format!("{key}={val}"));
            }
            None => {
                block.attrs.remove(&key);
                changes.push(format!("{key} cleared"));
            }
        }
    }
    // Координаты — только парой; ширина/высота у линии пересчитываются по концам.
    let has_x = block.attrs.get("x").is_some();
    let has_y = block.attrs.get("y").is_some();
    if has_x != has_y {
        block.attrs.remove("x");
        block.attrs.remove("y");
        return Err("pass both x and y (or clear both) to place a block on the canvas".to_string());
    }
    if let BlockKind::Shape { shape } = block.kind {
        if shape.is_line() {
            canonicalize_line(block, shape);
        }
    }
    Ok(changes)
}

/// Привести линейную фигуру к канону: минимум точек — в нуле, рамка сдвинута
/// под них, ширина/высота — по bbox с полем.
pub(super) fn canonicalize_line(block: &mut DocBlock, kind: ShapeKind) {
    let pts = shape::line_handles(&block.attrs, kind);
    let (min_x, min_y) = pts.iter().fold((f32::MAX, f32::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
    if min_x.abs() > 0.05 || min_y.abs() > 0.05 {
        let (p1, p2) = shape::endpoints_of(&block.attrs);
        shape::set_endpoints(&mut block.attrs, (p1.0 - min_x, p1.1 - min_y), (p2.0 - min_x, p2.1 - min_y));
        // Направляющие сдвигаются только те, что заданы явно: остальные
        // выводятся из концов заново.
        for (key, d) in [("cx1", min_x), ("cy1", min_y), ("cx2", min_x), ("cy2", min_y)] {
            if let Some(v) = block.attrs.get(key).and_then(|v| v.parse::<f32>().ok()) {
                block.attrs.set(key, fnum(v - d));
            }
        }
        if let Some((x, y)) = free::pos_of(&block.attrs) {
            free::set_pos(&mut block.attrs, x + min_x, y + min_y);
        }
    }
    let size = shape::line_box(&block.attrs, kind);
    free::set_width(&mut block.attrs, size.width.max(40.0));
    free::set_height(&mut block.attrs, size.height.max(20.0));
}

/// Убрать врезки `![[kind:id]]` из markdown (и из вложенных блоков).
pub(super) fn remove_embed(md: &str, target: &str) -> Option<String> {
    fn walk(blocks: &mut Vec<DocBlock>, target: &str) -> bool {
        let before = blocks.len();
        blocks.retain(|b| !matches!(&b.kind, BlockKind::Embed { target: t } if t.trim() == target));
        let mut removed = blocks.len() != before;
        for b in blocks.iter_mut() {
            if let Some(children) = b.kind.children_mut() {
                removed |= walk(children, target);
            }
        }
        removed
    }
    let mut model = parse_document(md);
    walk(&mut model.blocks, target).then(|| serialize_document(&model))
}

pub(super) fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}
