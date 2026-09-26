//! Глобальный лимитер скорости скачивания (token-bucket).
//!
//! Tokio-задачи `api::download_*` крутятся НЕ на main-thread → читать `RwSignal`
//! оттуда нельзя (как в [`super::control`]). Живой лимит хранится в атомике
//! [`LIMIT_BPS`] (байт/сек, `0` = без лимита), который выставляет UI-слой через
//! [`set_limit_bps`], а задачи после записи каждого чанка зовут [`throttle`].
//!
//! Бакет один на ВСЕ активные загрузки и их HTTP-Range-сегменты: суммарная
//! скорость не превышает лимит. Параллельные сегменты сериализуются на коротком
//! Mutex (рефилл — микросекунды); лок никогда не удерживается через `await`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static LIMIT_BPS: AtomicU64 = AtomicU64::new(0);

struct Bucket {
    tokens: f64,
    last: Instant,
}

static BUCKET: OnceLock<Mutex<Bucket>> = OnceLock::new();

fn bucket() -> &'static Mutex<Bucket> {
    BUCKET.get_or_init(|| {
        Mutex::new(Bucket {
            tokens: 0.0,
            last: Instant::now(),
        })
    })
}

/// Выставить глобальный лимит в байт/сек. `0` — без лимита. Вызывается из
/// main-thread эффекта при изменении `speed_limit_mbps`.
pub fn set_limit_bps(bps: u64) {
    LIMIT_BPS.store(bps, Ordering::Relaxed);
}

/// Затормозить после записи `n` байт, чтобы суммарная скорость всех задач не
/// превысила лимит. При `0` (без лимита) выходит мгновенно, не трогая Mutex.
pub async fn throttle(n: u64) {
    let limit = LIMIT_BPS.load(Ordering::Relaxed);
    if limit == 0 {
        return;
    }
    let limit_f = limit as f64;
    let sleep_secs = {
        let mut b = bucket().lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.last = now;
        // Рефилл с ограничением burst ≤ 1с накопления.
        b.tokens = (b.tokens + elapsed * limit_f).min(limit_f);
        b.tokens -= n as f64;
        if b.tokens < 0.0 {
            let debt = -b.tokens;
            b.tokens = 0.0;
            debt / limit_f
        } else {
            0.0
        }
    };
    if sleep_secs > 0.0 {
        tokio::time::sleep(Duration::from_secs_f64(sleep_secs)).await;
    }
}
