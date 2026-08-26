//! Body builder + executor для NodeKind::AsrGigaam — ASR-нода на базе
//! модели **GigaAM** (CTC).
//!
//! Поток данных:
//! - `evaluate`: пишет `PortValue::Text(output_text.get())` в порт `out`.
//!   Транскрибат уходит downstream (например, в `TextView`-ноду).
//! - Play-кнопка читает текущий аудио-буфер на входе ноды через
//!   `NodeEditorCtx.values`, спавнит worker-thread. Worker лениво грузит
//!   модель `Transcriber::load(...)` (~секунды), если её ещё нет или
//!   изменился конфиг, и вызывает `transcribe_pcm_text`. Текст
//!   доставляется в UI через `RwSignal::set` (signal.rs сам диспетчерит
//!   `run_on_main_thread`).
//!
//! Layout body — Demo-стилистика (см. `node_view::field_row`):
//! каждая строка это [label слева | spacer | control справа] с
//! теми же MSS-классами `.node-card-field-label`, `.node-card-field-spacer`
//! и `.node-input-*` для контролов. Транскрибат в карточке не показывается —
//! его читает downstream-нода `TextView`.

use std::sync::Arc;
use std::thread;

use synaptix::facade::asr::{AsrConfig, AsrModelKind, ComputeDType, Device, StorageDType, Transcriber};
use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{Column, Reactive};

use super::super::controls::{node_dropdown_field, node_field_row, node_file_picker};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    AsrLoadedCfg, Connection, NodeId, NodeInstance, NodeRuntime, PortValue,
};

// ── Опции для UI dropdown'ов ─────────────────────────────────────────────

/// Доступные устройства. 0 = CPU, 1 = GPU (CUDA/Metal auto-detect).
pub const DEVICE_OPTIONS: &[&str] = &["CPU", "GPU (auto)"];

/// Storage dtype для весов модели в VRAM. См. `synaptix_core::dtype::DType`.
/// На CPU «quantized» варианты внутри `Transcriber::load` фолбэк'нутся в F32.
pub const STORAGE_OPTIONS: &[&str] = &["f16", "bf16", "f32", "nvfp4", "mxfp8"];

/// Compute dtype — путь инференса в backend'е. Native paths (mxfp8/nvfp4)
/// требуют совместимый storage и CUDA.
pub const COMPUTE_OPTIONS: &[&str] = &["f16", "bf16", "f32", "nvfp4", "mxfp8"];

pub fn default_storage_idx() -> usize {
    0
}
pub fn default_compute_idx() -> usize {
    0
}

fn storage_from_idx(i: usize) -> StorageDType {
    match STORAGE_OPTIONS.get(i).copied().unwrap_or("f16") {
        "bf16" => StorageDType::BF16,
        "f32" => StorageDType::F32,
        "nvfp4" => StorageDType::NVFP4,
        "mxfp8" => StorageDType::MXFP8,
        _ => StorageDType::F16,
    }
}

fn compute_from_idx(i: usize) -> ComputeDType {
    match COMPUTE_OPTIONS.get(i).copied().unwrap_or("f16") {
        "bf16" => ComputeDType::BF16,
        "f32" => ComputeDType::F32,
        "mxfp8" => ComputeDType::Fp8E4M3,
        "nvfp4" => ComputeDType::Nvfp4,
        _ => ComputeDType::F16,
    }
}

fn device_from_idx(i: usize) -> Device {
    match i {
        1 => best_gpu_device(),
        _ => Device::Cpu,
    }
}

fn best_gpu_device() -> Device {
    Device::Cuda(0)
}

/// Backend-kernels обязаны быть зарегистрированы до первого тензорного опа
/// (cast весов при загрузке) — иначе `backend not registered for device`.
fn ensure_kernels_registered() {
    use std::sync::OnceLock;
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
}


// ── Executor ──────────────────────────────────────────────────────────────

/// Executor ноды: пишет текущий `output_text` в порт `out`. Реальная
/// транскрибация запускается из click-handler'а Play-кнопки (см. `body`).
pub struct AsrGigaamExec;

impl NodeExecutor for AsrGigaamExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _in_pv = ctx.read_input("in");
        let track = ctx.track;
        let text = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AsrGigaam { output_text, .. } => {
                    if track {
                        output_text.get()
                    } else {
                        output_text.get_untracked()
                    }
                }
                _ => String::new(),
            },
            Err(_) => String::new(),
        };
        ctx.write_output("out", PortValue::Text(text));
    }
}

// ── Запуск ноды (общий путь Per-node Play / глобальный Run) ─────────────

/// Hook для `NodeKindMeta.on_run` — вызывается из `run_controls` для каждой
/// AsrGigaam-ноды в графе. Делает то же самое, что и Per-node Play в body.
pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

/// Hook для `NodeKindMeta.busy_signal` — возвращает RwSignal, который true
/// пока идёт загрузка модели или транскрибация. `run_controls` подписывается
/// на эти сигналы у всех нод и сбрасывает Run-state в Stopped когда все
/// false.
pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AsrGigaam { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Запустить транскрибацию ноды. Идемпотентно: если worker уже работает —
/// no-op. Если нет audio на входе или не выбрана модель — выставляет
/// `error` и тоже no-op (без spawn-а worker'а).
pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let (
        model_path,
        device_idx,
        storage_idx,
        compute_idx,
        running,
        error_sig,
        loaded_name,
        output_text,
        text_version,
        transcriber,
        loaded_cfg,
    ) = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AsrGigaam {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                running,
                error,
                loaded_name,
                output_text,
                text_version,
                transcriber,
                loaded_cfg,
                ..
            } => (
                *model_path,
                *device_idx,
                *storage_idx,
                *compute_idx,
                *running,
                *error,
                *loaded_name,
                *output_text,
                *text_version,
                transcriber.clone(),
                loaded_cfg.clone(),
            ),
            _ => return,
        },
        Err(_) => return,
    };

    if running.get_untracked() {
        return;
    }

    // Хэндл Syn Checkpoint (вход `model`) переопределяет собственные поля;
    // оттуда же — резидентность. Без хэндла — legacy-поведение слота.
    let handle = super::current_input_syn_model(ctx, node.id);
    let resident = handle.as_ref().map(|h| h.resident).unwrap_or(true);
    let cfg = match &handle {
        Some(h) => AsrLoadedCfg {
            model_path: h.model_path.clone(),
            device_idx: map_handle_device(h.device_idx),
            storage_idx: map_handle_storage(h.storage_idx),
            compute_idx: map_handle_compute(h.compute_idx),
        },
        None => {
            let Some(mp) = model_path.get_untracked() else {
                error_sig.set(Some(tr!("node.asr_gigaam.err.select_model")));
                return;
            };
            AsrLoadedCfg {
                model_path: mp,
                device_idx: device_idx.get_untracked(),
                storage_idx: storage_idx.get_untracked(),
                compute_idx: compute_idx.get_untracked(),
            }
        }
    };
    let buf = match current_input_audio(ctx, node.id) {
        Ok(b) => b,
        Err(msg) => {
            error_sig.set(Some(msg));
            return;
        }
    };

    running.set(true);
    error_sig.set(None);

    let _ = thread::Builder::new()
        .name("synthos-asr-gigaam-worker".into())
        .spawn(move || {
            play_worker(
                transcriber,
                loaded_cfg,
                cfg,
                buf,
                running,
                error_sig,
                loaded_name,
                output_text,
                text_version,
                resident,
            );
        });
}

/// Маппинг предпочтений Syn Checkpoint на индексы опций семейства.
/// DEVICE: ["CPU","GPU (auto)"] — Auto → GPU; STORAGE/COMPUTE:
/// ["f16","bf16","f32","nvfp4","mxfp8"] — Auto → дефолт семейства.
fn map_handle_device(pref: usize) -> usize {
    match pref {
        2 => 0, // CPU
        _ => 1, // Auto/CUDA → GPU (auto)
    }
}

fn map_handle_storage(pref: usize) -> usize {
    match pref {
        1 => 0, // F16
        2 => 1, // BF16
        3 => 4, // FP8 → mxfp8
        4 => 3, // NVFP4
        _ => default_storage_idx(),
    }
}

fn map_handle_compute(pref: usize) -> usize {
    match pref {
        1 => 0, // F16
        2 => 1, // BF16
        3 => 2, // F32
        _ => default_compute_idx(),
    }
}

// ── Body builder ──────────────────────────────────────────────────────────

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    // Сигналы, использующиеся UI. `output_text` / `text_version` / реальные
    // handle'ы транскрайбера body не использует напрямую — ими управляет
    // `start()` (по нажатию Per-node Play или глобального Run).
    let (
        model_path,
        device_idx,
        storage_idx,
        compute_idx,
        running,
        error_sig,
        loaded_name,
        transcriber_handle,
        loaded_cfg_handle,
    ) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AsrGigaam {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                running,
                error,
                loaded_name,
                transcriber,
                loaded_cfg,
                ..
            } => (
                *model_path,
                *device_idx,
                *storage_idx,
                *compute_idx,
                *running,
                *error,
                *loaded_name,
                transcriber.clone(),
                loaded_cfg.clone(),
            ),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "AsrGigaam")),
        },
        Err(_) => return error_widget("AsrGigaam: lock error"),
    };

    let transcriber_h = transcriber_handle.clone();
    let loaded_h = loaded_cfg_handle.clone();
    let on_pick_error = error_sig;
    let on_pick_loaded_name = loaded_name;
    let model_control: Box<dyn Widget> = node_file_picker(
        tr!("node.asr_gigaam.pick_model_tooltip"),
        model_path,
        &[("Syn bundle", &["syn"]), ("nodes.filter.all_files", &["*"])],
        move |_p| {
            if let Ok(mut g) = transcriber_h.lock() {
                *g = None;
            }
            if let Ok(mut g) = loaded_h.lock() {
                *g = None;
            }
            on_pick_loaded_name.set(None);
            on_pick_error.set(None);
        },
    );

    let device_dd = node_dropdown_field(DEVICE_OPTIONS, device_idx);
    let storage_dd = node_dropdown_field(STORAGE_OPTIONS, storage_idx);
    let compute_dd = node_dropdown_field(COMPUTE_OPTIONS, compute_idx);

    // ── Row 5: Статус ─────────────────────────────────────────────────────
    //
    // Per-node Play кнопка убрана — запуск идёт через глобальный «Run»
    // в верхней панели страницы (`run_controls.rs` → `on_run` хук). Здесь
    // показываем только статус-строку (Распознавание / Ошибка / имя
    // загруженной модели).

    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(tr!("nodes.common.error", error = msg)).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            return vec![
                Box::new(
                    Text::new(tr!("node.asr_gigaam.status.recognizing"))
                        .class("audio-node-meta asr-node-running"),
                ) as Box<dyn Widget>,
            ];
        }
        if let Some(name) = loaded_name.get() {
            return vec![
                Box::new(Text::new(name).class("audio-node-meta")) as Box<dyn Widget>,
            ];
        }
        vec![Box::new(Text::new("—").class("audio-node-meta")) as Box<dyn Widget>]
    });

    let status_control: Box<dyn Widget> = Box::new(status_text);

    // ── Сборка ────────────────────────────────────────────────────────────

    let col: Column = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![
            node_field_row(&tr!("nodes.common.model"), model_control),
            node_field_row("Device", device_dd),
            node_field_row("Storage", storage_dd),
            node_field_row("Compute", compute_dd),
            node_field_row(&tr!("nodes.common.status"), status_control),
        ]);

    Box::new(col)
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
}

// ── Audio helpers ─────────────────────────────────────────────────────────

fn current_input_audio(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
) -> std::result::Result<Arc<AudioBuffer>, String> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == "in")
        .ok_or_else(|| tr!("nodes.common.connect_audio_input"))?;
    let values = ctx.values.get_untracked();
    let pv = values
        .get(&(src.from_node, src.from_port))
        .cloned()
        .ok_or_else(|| tr!("nodes.common.source_no_value"))?;
    match pv {
        PortValue::Audio(b) => Ok(b),
        PortValue::AudioStream(_) => {
            Err(tr!("nodes.common.live_stream_unsupported"))
        }
        _ => Err(tr!("nodes.common.no_audio_input_data")),
    }
}

fn downmix_to_mono(pcm: &[f32], channels: usize) -> Vec<f32> {
    let ch = channels.max(1);
    if ch == 1 {
        return pcm.to_vec();
    }
    let frames = pcm.len() / ch;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut s = 0.0_f32;
        for c in 0..ch {
            s += pcm[i * ch + c];
        }
        out.push(s / ch as f32);
    }
    out
}

// ── Worker ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn play_worker(
    transcriber: Arc<Mutex<Option<Transcriber>>>,
    loaded_cfg: Arc<Mutex<Option<AsrLoadedCfg>>>,
    cfg: AsrLoadedCfg,
    buf: Arc<AudioBuffer>,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_text: RwSignal<String>,
    text_version: RwSignal<u32>,
    resident: bool,
) {
    // 1. Загрузка (если нужно).
    let needs_load = match loaded_cfg.lock() {
        Ok(g) => match &*g {
            Some(prev) => prev != &cfg,
            None => true,
        },
        Err(_) => true,
    };
    if needs_load {
        ensure_kernels_registered();
        if let Ok(mut g) = transcriber.lock() {
            *g = None;
        }
        loaded_name.set(None);

        let asr_cfg = AsrConfig {
            kind: AsrModelKind::GigaAm,
            model_path: cfg.model_path.clone(),
            language: Some("ru".into()),
            device: device_from_idx(cfg.device_idx),
            storage_dtype: storage_from_idx(cfg.storage_idx),
            compute_dtype: compute_from_idx(cfg.compute_idx),
        };
        let vram_before = crate::models::cuda_allocated();
        match Transcriber::load(asr_cfg) {
            Ok(t) => {
                let bytes = crate::models::cuda_allocated().saturating_sub(vram_before);
                let name = t.model_name().to_string();
                if let Ok(mut g) = transcriber.lock() {
                    *g = Some(t);
                }
                if let Ok(mut g) = loaded_cfg.lock() {
                    *g = Some(cfg.clone());
                }
                loaded_name.set(Some(name.clone()));
                register_in_panel(
                    "ASR",
                    "GigaAM",
                    name,
                    device_from_idx(cfg.device_idx),
                    bytes,
                    transcriber.clone(),
                    loaded_cfg.clone(),
                    loaded_name,
                );
            }
            Err(e) => {
                error_sig.set(Some(tr!("nodes.common.model_load_failed", error = e)));
                running.set(false);
                return;
            }
        }
    }

    // 2. Транскрибация.
    let pcm_mono = downmix_to_mono(&buf.pcm, buf.channels as usize);
    let sr = buf.sample_rate;
    let result = match transcriber.lock() {
        Ok(mut g) => {
            let Some(t) = g.as_mut() else {
                error_sig.set(Some(tr!("nodes.common.model_not_loaded")));
                running.set(false);
                return;
            };
            t.transcribe_pcm_text(&pcm_mono, sr, None)
        }
        Err(_) => {
            error_sig.set(Some("Lock error transcriber".into()));
            running.set(false);
            return;
        }
    };

    // 3. UI update.
    match result {
        Ok(text) => {
            output_text.set(text);
            // `update` и `get_untracked` валятся с не-main треда (signal-
            // RUNTIME thread_local пуст в worker'е). Маршалим всю операцию
            // инкремента на main thread целиком — там signals доступны.
            syngui::prelude::run_on_main_thread(move || {
                text_version.update(|v| *v = v.wrapping_add(1));
            });
            error_sig.set(None);
        }
        Err(e) => {
            error_sig.set(Some(tr!("node.asr_gigaam.err.transcription_failed", error = e)));
        }
    }
    // Хэндл без резидентности («Держать в памяти» выключен у Syn
    // Checkpoint): слот очищается сразу после прогона, VRAM возвращается.
    if !resident {
        if let Ok(mut g) = transcriber.lock() {
            *g = None;
        }
        if let Ok(mut g) = loaded_cfg.lock() {
            *g = None;
        }
        loaded_name.set(None);
        crate::models::trim_all();
    }
    running.set(false);
}

/// Показать модель в панели загруженных моделей. Ключ — адрес слота
/// `NodeRuntime`: id ноды до воркера не доезжает, а слот у каждой ноды
/// свой и живёт ровно столько же. Выгрузка снимает и `loaded_cfg`, иначе
/// следующий прогон решил бы, что грузить нечего.
#[allow(clippy::too_many_arguments)]
fn register_in_panel<T: Send + 'static, C: Send + 'static>(
    family: &'static str,
    component: &'static str,
    label: String,
    device: synaptix_core::device::Device,
    bytes: u64,
    slot: Arc<Mutex<Option<T>>>,
    cfg_slot: Arc<Mutex<Option<C>>>,
    loaded_name: RwSignal<Option<String>>,
) {
    let key = format!("{family}/{component}/{:p}", Arc::as_ptr(&slot));
    crate::models::register_slot(key, family, component, label, device, bytes, slot, move || {
        if let Ok(mut g) = cfg_slot.lock() {
            *g = None;
        }
        loaded_name.set(None);
    });
}
