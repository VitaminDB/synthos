//! Лёгкие хелперы форматирования времени без зависимости от `chrono`.
//!
//! Нам нужны ровно две вещи: «HH:MM» для метки у сообщения и строковая дата
//! «YYYY-MM-DD» для разделителя. Локальное смещение берём у libc
//! (`localtime_r`) — до 03.09.2026 метки в ленте были в UTC, потому что
//! смещение читалось только из нестандартной переменной окружения.

use std::time::{SystemTime, UNIX_EPOCH};

/// Количество наносекунд с UNIX-эпохи. Используется как монотонный
/// источник уникального id для нового чата — в пределах процесса двух
/// одинаковых значений получить почти невозможно даже на скоростном SSD.
pub fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Количество секунд с UNIX-эпохи. При невалидных часах — `0`.
pub fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Переопределение смещения для тестов: `i64::MIN` — не задано.
static OFFSET_OVERRIDE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);

/// Подменить локальное смещение (секунды) — только для тестов; `None`
/// возвращает системное.
pub fn override_offset_secs(secs: Option<i64>) {
    OFFSET_OVERRIDE.store(secs.unwrap_or(i64::MIN), std::sync::atomic::Ordering::Relaxed);
}

/// Смещение локального времени от UTC в секундах: переопределение из
/// тестов, переменная `TZ_OFFSET_MIN` (минуты), иначе системная зона через
/// `localtime_r`; если и её нет — UTC.
pub fn local_offset_secs() -> i64 {
    let o = OFFSET_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if o != i64::MIN {
        return o;
    }
    if let Some(m) = std::env::var("TZ_OFFSET_MIN").ok().and_then(|s| s.parse::<i64>().ok()) {
        return m * 60;
    }
    system_offset_secs().unwrap_or(0)
}

#[cfg(unix)]
fn system_offset_secs() -> Option<i64> {
    // SAFETY: localtime_r пишет в переданную структуру и не хранит указатели.
    unsafe {
        let t: libc::time_t = unix_secs() as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        Some(tm.tm_gmtoff as i64)
    }
}

#[cfg(not(unix))]
fn system_offset_secs() -> Option<i64> {
    None
}

/// Локальное «сейчас»: дни от эпохи и минуты с полуночи.
pub fn local_now() -> (i64, u32) {
    let secs = unix_secs() as i64 + local_offset_secs();
    (secs.div_euclid(86_400), (secs.rem_euclid(86_400) / 60) as u32)
}

/// Локальный сегодняшний день в днях от эпохи.
pub fn local_today_days() -> i64 {
    local_now().0
}

/// Формат «HH:MM» текущего времени.
pub fn format_hm_now() -> String {
    format_hm(unix_secs() as i64 + local_offset_secs())
}

/// Формат «HH:MM» произвольного `unix_secs` в той же TZ-политике.
pub fn format_hm(unix_secs: i64) -> String {
    let secs = unix_secs.rem_euclid(86_400);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    format!("{:02}:{:02}", h, m)
}

/// Локальные «сейчас» одной строкой: «YYYY-MM-DD HH:MM». Дата и время
/// берутся из одного отсчёта — на границе суток пары «вчерашняя дата плюс
/// сегодняшняя полночь» не получится.
pub fn format_now() -> String {
    let secs = unix_secs() as i64 + local_offset_secs();
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{:04}-{:02}-{:02} {}", y, m, d, format_hm(secs))
}

/// Возвращает дату сегодняшнего дня в формате «YYYY-MM-DD» для date-divider.
/// Календарные вычисления — через алгоритм Howard Hinnant (days_from_civil).
pub fn format_date_today() -> String {
    let days = (unix_secs() as i64 + local_offset_secs()).div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Обратная проекция `days_from_civil` Хинанта: число дней с 1970-01-01
/// → (year, month, day). Корректно для всего диапазона i64.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hm_boundaries() {
        assert_eq!(format_hm(0), "00:00");
        assert_eq!(format_hm(60), "00:01");
        assert_eq!(format_hm(3600 + 60), "01:01");
        assert_eq!(format_hm(23 * 3600 + 59 * 60), "23:59");
    }

    #[test]
    fn local_offset_override_and_now() {
        override_offset_secs(Some(5 * 3600));
        assert_eq!(local_offset_secs(), 5 * 3600);
        let (days, minutes) = local_now();
        assert!(minutes < 24 * 60);
        assert_eq!(days, (unix_secs() as i64 + 5 * 3600).div_euclid(86_400));
        override_offset_secs(None);
        // Системное смещение — в пределах суток.
        assert!(local_offset_secs().abs() <= 14 * 3600);
    }

    #[test]
    fn civil_epoch() {
        // 1970-01-01 — день 0 от эпохи.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-02-29 — високосный тест.
        // 30 лет, включая 8 високосных → 10957 + 59 = 11016
        let (y, m, d) = civil_from_days(11016);
        assert_eq!((y, m, d), (2000, 2, 29));
    }
}
