//! Календарная арифметика диаграммы: дни от эпохи ↔ civil-даты
//! (алгоритмы `days_from_civil` / `civil_from_days`), ISO-строки, «сегодня».

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = понедельник.
pub fn weekday_of(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// Локальный день (смещение зоны — `agent::time::local_offset_secs`).
pub fn today_days() -> i64 {
    crate::agent::time::local_today_days()
}

#[allow(dead_code)]
fn today_days_utc() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs.div_euclid(86400)
}

/// `YYYY-MM-DD` → дни от эпохи.
pub fn parse_days(iso: &str) -> Option<i64> {
    let mut parts = iso.trim().splitn(3, '-');
    let y = parts.next()?.parse::<i64>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

pub fn days_to_iso(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `ДД.ММ` — подпись на баре.
pub fn short_date(days: i64) -> String {
    let (_, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for iso in ["2026-09-01", "2000-02-29", "1999-12-31", "2026-01-01"] {
            let days = parse_days(iso).unwrap();
            assert_eq!(days_to_iso(days), iso);
        }
        // 2026-09-01 — вторник.
        assert_eq!(weekday_of(parse_days("2026-09-01").unwrap()), 1);
        assert_eq!(short_date(parse_days("2026-09-01").unwrap()), "01.09");
        assert!(parse_days("мусор").is_none());
    }
}
