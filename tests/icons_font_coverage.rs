//! Каждая константа `MI_*` из `src/icons.rs` обязана иметь глиф в
//! `MaterialIcons-Regular.ttf`, который syngui грузит как семейство
//! "Material Icons".
//!
//! Зачем тест: отсутствующий codepoint не даёт ни ошибки компиляции, ни
//! предупреждения в рантайме — иконка просто не рисуется. Так в настройках
//! пункт «AI модели» месяцами стоял без иконки (`smart_toy` U+EF8A нет в
//! этой версии шрифта), и заметить это можно было только глазами на
//! скриншоте. Ещё два битых codepoint'а (`monitor_heart` U+F154,
//! `deployed_code` U+F510) нашлись только когда стали проверять все разом.
//!
//! Тест парсит cmap формата 4 напрямую — тянуть ttf-parser в dev-deps ради
//! одной таблицы незачем.

use std::path::PathBuf;

/// Путь к шрифту иконок внутри syngui (path-зависимость воркспейса).
fn font_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../syngui/syngui/assets/fonts/MaterialIcons-Regular.ttf")
}

fn be16(d: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([d[off], d[off + 1]])
}

fn be32(d: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

/// Множество codepoint'ов из cmap-подтаблицы формата 4.
fn font_codepoints(data: &[u8]) -> Vec<std::ops::RangeInclusive<u32>> {
    let num_tables = be16(data, 4) as usize;
    let mut cmap_off = None;
    for i in 0..num_tables {
        let rec = 12 + 16 * i;
        if &data[rec..rec + 4] == b"cmap" {
            cmap_off = Some(be32(data, rec + 8) as usize);
        }
    }
    let cmap = cmap_off.expect("в шрифте нет таблицы cmap");

    let n_sub = be16(data, cmap + 2) as usize;
    let mut fmt4 = None;
    for i in 0..n_sub {
        let rec = cmap + 4 + 8 * i;
        let sub = cmap + be32(data, rec + 4) as usize;
        if be16(data, sub) == 4 {
            fmt4 = Some(sub);
        }
    }
    let sub = fmt4.expect("в cmap нет подтаблицы формата 4");

    let seg_x2 = be16(data, sub + 6) as usize;
    let end_off = sub + 14;
    let start_off = end_off + seg_x2 + 2;

    let mut ranges = Vec::with_capacity(seg_x2 / 2);
    for s in 0..seg_x2 / 2 {
        let end = be16(data, end_off + 2 * s) as u32;
        let start = be16(data, start_off + 2 * s) as u32;
        // Последний сегмент — терминатор 0xFFFF..=0xFFFF, глифов не несёт.
        if start == 0xFFFF {
            continue;
        }
        ranges.push(start..=end.min(0xFFFE));
    }
    ranges
}

/// Все `pub const MI_*: &str = "\u{XXXX}";` из `src/icons.rs`.
fn declared_icons(src: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const MI_") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(':') else {
            continue;
        };
        let Some(open) = tail.find("\\u{") else {
            continue;
        };
        let tail = &tail[open + 3..];
        let Some(close) = tail.find('}') else {
            continue;
        };
        let Ok(cp) = u32::from_str_radix(&tail[..close], 16) else {
            continue;
        };
        out.push((format!("MI_{}", name.trim()), cp));
    }
    out
}

#[test]
fn every_icon_constant_has_a_glyph() {
    let font = std::fs::read(font_path()).expect("MaterialIcons-Regular.ttf не найден");
    let ranges = font_codepoints(&font);
    let has = |cp: u32| ranges.iter().any(|r| r.contains(&cp));

    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/icons.rs"),
    )
    .expect("src/icons.rs не читается");

    let icons = declared_icons(&src);
    assert!(
        icons.len() > 100,
        "разобрано всего {} констант — парсер icons.rs сломался",
        icons.len()
    );

    let missing: Vec<String> = icons
        .iter()
        .filter(|(_, cp)| !has(*cp))
        .map(|(name, cp)| format!("{name} (U+{cp:04X})"))
        .collect();

    assert!(
        missing.is_empty(),
        "в MaterialIcons-Regular.ttf нет глифов для {} констант — они будут \
         рисоваться пустым местом:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}
