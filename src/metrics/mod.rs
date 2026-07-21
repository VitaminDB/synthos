//! Реактивное состояние метрик для вкладки «Детали» правого сайдбара.
//!
//! Два источника:
//!   * **LLM-метрики** — `chat::session` вызывает [`llama::apply_final_chunk`]
//!     с финальным `timings/usage` от llama-server и стартует poller `/slots`.
//!   * **Системные метрики** — отдельный std::thread ([`system::start_sampler`])
//!     читает `sysinfo` (CPU per-core, RAM) и опционально NVML (VRAM/GPU util).
//!
//! Все сигналы — thread-local (у syngui они живут только в UI-потоке), поэтому
//! из фоновых потоков пишем через `async_runtime::run_on_main_thread`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use syngui::signal::{use_signal, RwSignal};

use crate::llama::api::{SlotInfo, Timings, Usage};

pub mod gpu;
pub mod history;
pub mod llama;
pub mod system;

pub use history::{RingBuffer, DEFAULT_CAPACITY};

/// Глобальное состояние всех метрик. Живёт в `AppCtx.metrics: Arc<MetricsState>`.
///
/// `Drop` останавливает фоновый семплер (через `sampler_running = false`).
pub struct MetricsState {
    // ── LLM ────────────────────────────────────────────────────────────────
    /// Последний authoritative `timings` из финального chunk'а стрима.
    pub llama_timings: RwSignal<Option<Timings>>,
    /// Последний `usage` (prompt/completion/total tokens).
    pub llama_usage: RwSignal<Option<Usage>>,
    /// Текущий активный слот (для n_decoded, n_remain, n_ctx).
    pub llama_slot: RwSignal<Option<SlotInfo>>,
    /// История `predicted_per_second`, строим LineChart.
    pub predicted_tps_history: RwSignal<RingBuffer>,
    /// История `prompt_per_second`.
    pub prompt_tps_history: RwSignal<RingBuffer>,

    // ── System CPU/RAM ─────────────────────────────────────────────────────
    /// Мгновенные значения каждого ядра (заменяется целиком каждый тик).
    pub cpu_per_core: RwSignal<Vec<f32>>,
    /// История суммарного CPU %.
    pub cpu_total_history: RwSignal<RingBuffer>,
    /// История `used_memory` (байт).
    pub ram_used_history: RwSignal<RingBuffer>,
    /// Общий объём RAM (байт).
    pub ram_total: RwSignal<u64>,

    // ── GPU / VRAM (опционально, NVML) ─────────────────────────────────────
    /// `true`, если NVML удалось инициализировать на старте.
    pub gpu_available: RwSignal<bool>,
    /// История utilisation %.
    pub gpu_util_history: RwSignal<RingBuffer>,
    /// История used VRAM (байт).
    pub vram_used_history: RwSignal<RingBuffer>,
    /// Общий объём VRAM (байт).
    pub vram_total: RwSignal<u64>,
    /// Температура GPU, °C.
    pub gpu_temperature_c: RwSignal<u32>,

    // ── Control ────────────────────────────────────────────────────────────
    /// Флаг, по которому фоновый поток понимает, что пора завершаться.
    pub sampler_running: Arc<AtomicBool>,
}

impl MetricsState {
    pub fn new() -> Self {
        Self {
            llama_timings: use_signal(None),
            llama_usage: use_signal(None),
            llama_slot: use_signal(None),
            predicted_tps_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),
            prompt_tps_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),

            cpu_per_core: use_signal(Vec::new()),
            cpu_total_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),
            ram_used_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),
            ram_total: use_signal(0),

            gpu_available: use_signal(false),
            gpu_util_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),
            vram_used_history: use_signal(RingBuffer::new(DEFAULT_CAPACITY)),
            vram_total: use_signal(0),
            gpu_temperature_c: use_signal(0),

            sampler_running: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MetricsState {
    fn drop(&mut self) {
        self.sampler_running
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
