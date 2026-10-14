//! Квантование весов при упаковке — у службы нейросетей: квантующий поток
//! живёт в `synaptix::facade::pack`, служба исполняет его операцией `pack`
//! на своей карте (Synthos своего CUDA не держит).

pub use synaptix::facade::pack::QuantDecision;

/// Есть ли карта NVIDIA, на которой служба посчитает квант (по NVML, без
/// CUDA-контекста в Synthos).
pub fn cuda_available() -> bool {
    crate::metrics::gpu::GpuMonitor::try_new().is_some()
}
