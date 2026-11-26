//! Секундомер идущего вызова инструмента — для обратного отсчёта в шапке
//! карточки вызова (`pages::syn_chat::message_bubble::tool_countdown`).
//!
//! Тройка сигналов вокруг монотонной точки старта: `started` — пока идёт
//! отсчёт, `live_ms` — «живое» показание, `last_ms` — итог последнего
//! измерения. Живое показание двигает фоновый поток: раз в
//! [`TICK_INTERVAL`] он просит главный поток подтянуть `live_ms` и
//! заканчивается вместе с отсчётом (или при новом старте — по поколению).

use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;

/// Шаг обновления «живого» показания: обратный отсчёт показывает секунды,
/// чаще пересобирать текст незачем.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
pub struct Stopwatch {
    started: RwSignal<Option<Instant>>,
    live_ms: RwSignal<u64>,
    last_ms: RwSignal<Option<u64>>,
    /// Номер текущего измерения: поток прошлого старта видит, что он
    /// устарел, и выходит.
    gen: RwSignal<u64>,
}

impl Stopwatch {
    pub fn new() -> Self {
        Self {
            started: use_signal(None::<Instant>),
            live_ms: use_signal(0_u64),
            last_ms: use_signal(None::<u64>),
            gen: use_signal(0_u64),
        }
    }

    /// Начать отсчёт с нуля. Только с главного потока.
    pub fn start(&self) {
        self.live_ms.set(0);
        self.started.set(Some(Instant::now()));
        let gen = self.gen.get_untracked() + 1;
        self.gen.set(gen);
        let sw = *self;
        std::thread::spawn(move || loop {
            std::thread::sleep(TICK_INTERVAL);
            let (tx, rx) = std::sync::mpsc::channel();
            run_on_main_thread(move || {
                let _ = tx.send(sw.tick(gen));
            });
            if !rx.recv().unwrap_or(false) {
                break;
            }
        });
    }

    /// Остановить отсчёт и зафиксировать длительность. No-op, если
    /// секундомер уже стоит.
    pub fn finish(&self) {
        let Some(t0) = self.started.get_untracked() else {
            return;
        };
        let ms = t0.elapsed().as_millis() as u64;
        self.started.set(None);
        self.live_ms.set(ms);
        self.last_ms.set(Some(ms));
    }

    /// Подтянуть живое показание. `false` — измерение `gen` закончилось,
    /// потоку пора выйти.
    fn tick(&self, gen: u64) -> bool {
        if self.gen.get_untracked() != gen {
            return false;
        }
        match self.started.get_untracked() {
            Some(t0) => {
                self.live_ms.set(t0.elapsed().as_millis() as u64);
                true
            }
            None => false,
        }
    }

    /// Что показывать: идущее показание, иначе итог последнего измерения,
    /// иначе `None`. Читает сигналы с подпиской — для `Reactive`.
    pub fn display_ms(&self) -> Option<u64> {
        if self.started.get().is_some() {
            Some(self.live_ms.get())
        } else {
            self.last_ms.get()
        }
    }
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_tick_finish_cycle() {
        let sw = Stopwatch::new();
        assert_eq!(sw.display_ms(), None);
        sw.start();
        let gen = sw.gen.get_untracked();
        assert_eq!(sw.display_ms(), Some(0));
        std::thread::sleep(Duration::from_millis(20));
        assert!(sw.tick(gen));
        let live = sw.display_ms().expect("идёт отсчёт");
        assert!(live >= 15, "живое показание не двигается: {live}");
        // Устаревшее поколение потока не трогает показание.
        assert!(!sw.tick(gen + 1));
        sw.finish();
        assert!(!sw.tick(gen), "после финиша поток должен выйти");
        let last = sw.display_ms().expect("итог остаётся после финиша");
        assert!(last >= live);
        sw.finish();
        assert_eq!(sw.display_ms(), Some(last));
    }
}
