//! Реестр атомарных флагов прерывания закачек.
//!
//! Tokio-задача `api::download_file` крутится НЕ на main-thread → читать
//! `RwSignal` оттуда нельзя. Управление (пауза/стоп/отмена) идёт через
//! `Arc<AtomicU8>`: UI-обработчики (main thread) выставляют флаг через
//! [`request`], а задача lock-free опрашивает [`DlControl::signal`] на каждом
//! chunk'е и прерывается с соответствующим `HfError::Aborted`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const RUN: u8 = 0;
pub const PAUSE: u8 = 1;
pub const STOP: u8 = 2;
pub const CANCEL: u8 = 3;
/// Глобальная пауза (кнопка «Пауза все»). Семантически как `PAUSE` (сохраняет
/// `.part`), но отдельное значение, чтобы `download.rs` отличал её от per-file
/// паузы и корректно отрабатывал гонку с «Продолжить все».
pub const GLOBAL_PAUSE: u8 = 4;

#[derive(Clone)]
pub struct DlControl(Arc<AtomicU8>);

impl DlControl {
    pub fn signal(&self) -> u8 {
        self.0.load(Ordering::Relaxed)
    }
    fn set(&self, v: u8) {
        self.0.store(v, Ordering::Relaxed);
    }
}

static REG: OnceLock<Mutex<HashMap<String, DlControl>>> = OnceLock::new();

fn reg() -> &'static Mutex<HashMap<String, DlControl>> {
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Зарегистрировать управление для ключа. Сбрасывает в `RUN`, перетирая
/// stale-сигнал, который мог остаться от прошлого прогона (resume). Вызывается
/// в `spawn_one` перед спавном задачи.
pub fn arm(key: &str) -> DlControl {
    let c = DlControl(Arc::new(AtomicU8::new(RUN)));
    reg().lock().unwrap().insert(key.to_string(), c.clone());
    c
}

/// Выставить сигнал прерывания для активной задачи (main thread). No-op, если
/// ключ не зарегистрирован (задача не активна).
pub fn request(key: &str, sig: u8) {
    if let Some(c) = reg().lock().unwrap().get(key) {
        c.set(sig);
    }
}

/// Снять управление из реестра (после завершения/прерывания задачи).
pub fn clear(key: &str) {
    reg().lock().unwrap().remove(key);
}
