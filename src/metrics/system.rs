//! Фоновый семплер системных метрик.
//!
//! Один std::thread крутится пока `MetricsState::sampler_running = true`.
//! Раз в секунду обновляет `sysinfo::System` (CPU per-core + RAM) и
//! опциональный `GpuMonitor` (NVML). Все записи в сигналы — через
//! `run_on_main_thread` (сигналы thread-local).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use super::gpu::GpuMonitor;
use super::MetricsState;

/// Интервал между семплами. 1 Гц — компромисс между «живостью» графиков и
/// накладными расходами sysinfo (CPU-refresh ≈ 1 мс на 16 ядер).
const SAMPLE_INTERVAL: Duration = Duration::from_millis(1000);

/// Стартовать семплер в отдельном потоке. Остановится, когда
/// `state.sampler_running` станет `false` (обычно это происходит на drop
/// `MetricsState` в завершении приложения).
///
/// Вызов идемпотентный: повторный вызов ничего не сломает — просто поднимет
/// ещё один поток. В `lib.rs` вызываем один раз при старте приложения.
pub fn start_sampler(state: Arc<MetricsState>) {
    let running = state.sampler_running.clone();

    thread::Builder::new()
        .name("synthos-metrics-sampler".into())
        .spawn(move || {
            sampler_loop(state, running);
        })
        .expect("не удалось создать поток synthos-metrics-sampler");
}

fn sampler_loop(state: Arc<MetricsState>, running: Arc<std::sync::atomic::AtomicBool>) {
    // sysinfo требует первый `refresh` + короткий sleep + второй `refresh`,
    // чтобы получить осмысленные значения CPU %. Инициализируем раз.
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_cpu_all();
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    let gpu = GpuMonitor::try_new();
    // Проставляем gpu_available один раз на старте — это стабильный факт
    // для текущего процесса.
    {
        let available = gpu.is_some();
        let state_clone = state.clone();
        run_on_main_thread(move || {
            state_clone.gpu_available.set(available);
        });
    }

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        sys.refresh_cpu_all();
        sys.refresh_memory();

        // CPU per-core и суммарный
        let cores: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let cpu_total = if cores.is_empty() {
            0.0
        } else {
            cores.iter().sum::<f32>() / cores.len() as f32
        };

        let ram_used = sys.used_memory();
        let ram_total = sys.total_memory();

        let gpu_sample = gpu.as_ref().and_then(|g| g.sample());

        // Маршалим всё в main thread одним колбэком — один redraw на тик.
        let state_clone = state.clone();
        run_on_main_thread(move || {
            // Per-core: заменяем вектор целиком.
            state_clone.cpu_per_core.set(cores);

            // Total — в историю.
            state_clone
                .cpu_total_history
                .update(|h| h.push(cpu_total as f64));

            state_clone.ram_total.set(ram_total);
            state_clone
                .ram_used_history
                .update(|h| h.push(ram_used as f64));

            if let Some(g) = gpu_sample {
                state_clone
                    .gpu_util_history
                    .update(|h| h.push(g.util_percent as f64));
                state_clone
                    .vram_used_history
                    .update(|h| h.push(g.mem_used as f64));
                state_clone.vram_total.set(g.mem_total);
                state_clone.gpu_temperature_c.set(g.temperature_c);
            }
        });

        thread::sleep(SAMPLE_INTERVAL);
    }
}
