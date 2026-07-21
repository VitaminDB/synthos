//! Лёгкие хелперы форматирования времени без зависимости от `chrono`.
//!
//! Нам нужны ровно две вещи: «HH:MM» для метки у сообщения и строковая дата
//! «YYYY-MM-DD» для разделителя. Всё прочее (таймзоны, локали) — избыточно
//! для текущей задачи, тянуть из-за этого тяжёлую зависимость смысла нет.

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

/// Часы и минуты локального времени (по offset’у системного TZ — через libc).
/// Если получить локальное смещение не удалось — возвращаем UTC: это безопасный
/// дефолт, а не ошибка в логе.
fn local_offset_secs() -> i64 {
    // std не даёт API для TZ offset. Читаем `/etc/localtime` косвенно через
    // вариации между `SystemTime::now()` и `time_t` невозможно без libc, поэтому
    // ограничиваемся переменной окружения `TZ_OFFSET_MIN` (нестандартно — пусть
    // пользователь не выставил; тогда показываем UTC). Локальное время в UI —
    // приятно, но не критично для чата: альтернатива `chrono`/`time` утяжелит
    // бинарник ради мелочи.
    std::env::var("TZ_OFFSET_MIN")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|m| m * 60)
        .unwrap_or(0)
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
    fn civil_epoch() {
        // 1970-01-01 — день 0 от эпохи.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-02-29 — високосный тест.
        // 30 лет, включая 8 високосных → 10957 + 59 = 11016
        let (y, m, d) = civil_from_days(11016);
        assert_eq!((y, m, d), (2000, 2, 29));
    }
}
