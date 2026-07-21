//! NVIDIA GPU-метрики через NVML. Модуль всегда собирается, но под feature
//! `nvml` внутри работает реальная обёртка; иначе — стаб-заглушка, которая
//! заявляет «недоступно» и не тянет линкер.
//!
//! Дизайн: `GpuMonitor::try_new()` — Option. Вызывающий код обязан
//! аккуратно обрабатывать None → панель «GPU метрики недоступны».

/// Мгновенный снимок GPU (NVIDIA), читаемый семплером каждую секунду.
#[derive(Debug, Clone, Copy, Default)]
pub struct GpuSample {
    /// Занятость GPU, %, 0..=100.
    pub util_percent: f32,
    /// Использовано VRAM (байт).
    pub mem_used: u64,
    /// Общий объём VRAM (байт).
    pub mem_total: u64,
    /// Температура, °C (опционально — 0 если не удалось прочесть).
    pub temperature_c: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Реализация под `nvml` (NVIDIA-only)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(feature = "nvml")]
mod imp {
    use super::GpuSample;
    use nvml_wrapper::Nvml;

    /// Обёртка-монитор первого NVIDIA-устройства. Держит `Nvml` живым.
    pub struct GpuMonitor {
        nvml: Nvml,
    }

    impl GpuMonitor {
        /// Попытка инициализации. Любая ошибка libnvidia-ml (не установлена,
        /// нет доступа, не NVIDIA) гасится в `None` — вызывающий код
        /// показывает плашку «недоступно».
        pub fn try_new() -> Option<Self> {
            match Nvml::init() {
                Ok(nvml) => Some(Self { nvml }),
                Err(e) => {
                    eprintln!("[metrics] NVML недоступен: {e} — GPU-панель скрыта");
                    None
                }
            }
        }

        /// Один снимок с устройства index 0. Любая ошибка — `None`, вызывающий
        /// код пропускает тик (оставляет предыдущее значение в истории).
        pub fn sample(&self) -> Option<GpuSample> {
            let dev = self.nvml.device_by_index(0).ok()?;

            let util = dev.utilization_rates().ok();
            let mem = dev.memory_info().ok();
            let temp = dev
                .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                .ok()
                .unwrap_or(0);

            Some(GpuSample {
                util_percent: util.map(|u| u.gpu as f32).unwrap_or(0.0),
                mem_used: mem.as_ref().map(|m| m.used).unwrap_or(0),
                mem_total: mem.as_ref().map(|m| m.total).unwrap_or(0),
                temperature_c: temp,
            })
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Заглушка без `nvml`-фичи: монитор никогда не создаётся.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(not(feature = "nvml"))]
mod imp {
    use super::GpuSample;

    pub struct GpuMonitor;

    impl GpuMonitor {
        pub fn try_new() -> Option<Self> {
            None
        }
        pub fn sample(&self) -> Option<GpuSample> {
            None
        }
    }
}

pub use imp::GpuMonitor;
