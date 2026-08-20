//! Body builder + executor для `NodeKind::SortformerDiarizer` — нода диаризации
//! спикеров на базе NVIDIA Streaming Sortformer v2.1.
//!
//! Аналог `asr_gigaam`: Audio in → Text(JSON) out. Live-stream пока не
//! поддерживается (фаза 2 — streaming через KV-cache).

use std::sync::Arc;
use std::thread;

use synaptix::facade::asr::{ComputeDType, Device, StorageDType};
use synaptix::facade::diarization::{DiarizationConfig, Diarizer};
use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::{Column, Reactive, Slider, Toggle};

use super::super::controls::{node_dropdown_field, node_field_row, node_file_picker};
use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    Connection, NodeId, NodeInstance, NodeRuntime, PortValue, SortformerLoadedCfg,
};

// Те же опции что и у AsrGigaam — для единообразия выбора пользователя.
pub const DEVICE_OPTIONS: &[&str] = &["CPU", "GPU (auto)"];
pub const STORAGE_OPTIONS: &[&str] = &["f16", "bf16", "f32", "nvfp4", "mxfp8"];
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
        1 => diarizer_gpu_device(),
        _ => Device::Cpu,
    }
}

fn diarizer_gpu_device() -> Device {
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

pub struct SortformerDiarizerExec;

impl NodeExecutor for SortformerDiarizerExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _in_pv = ctx.read_input("in");
        let track = ctx.track;
        let json = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::SortformerDiarizer { output_json, .. } => {
                    if track {
                        output_json.get()
                    } else {
                        output_json.get_untracked()
                    }
                }
                _ => String::new(),
            },
            Err(_) => String::new(),
        };
        ctx.write_output("out", PortValue::Text(json));
    }
}

// ── Run hooks ─────────────────────────────────────────────────────────────

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SortformerDiarizer { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let (
        model_path,
        device_idx,
        storage_idx,
        compute_idx,
        threshold,
        allow_overlap,
        running,
        error_sig,
        loaded_name,
        output_pretty,
        output_json,
        text_version,
        diarizer,
        loaded_cfg,
    ) = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SortformerDiarizer {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                threshold,
                allow_overlap,
                running,
                error,
                loaded_name,
                output_pretty,
                output_json,
                text_version,
                diarizer,
                loaded_cfg,
                ..
            } => (
                *model_path,
                *device_idx,
                *storage_idx,
                *compute_idx,
                *threshold,
                *allow_overlap,
                *running,
                *error,
                *loaded_name,
                *output_pretty,
                *output_json,
                *text_version,
                diarizer.clone(),
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
        Some(h) => SortformerLoadedCfg {
            model_path: h.model_path.clone(),
            device_idx: map_handle_device(h.device_idx),
            storage_idx: map_handle_storage(h.storage_idx),
            compute_idx: map_handle_compute(h.compute_idx),
        },
        None => {
            let Some(mp) = model_path.get_untracked() else {
                error_sig.set(Some(
                    "Выберите .syn модель Sortformer или подключите Syn Checkpoint".into(),
                ));
                return;
            };
            SortformerLoadedCfg {
                model_path: mp,
                device_idx: device_idx.get_untracked(),
                storage_idx: storage_idx.get_untracked(),
                compute_idx: compute_idx.get_untracked(),
            }
        }
    };
    let threshold_v = threshold.get_untracked();
    let allow_overlap_v = allow_overlap.get_untracked();
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
        .name("synthos-sortformer-worker".into())
        .spawn(move || {
            play_worker(
                diarizer,
                loaded_cfg,
                cfg,
                threshold_v,
                allow_overlap_v,
                buf,
                running,
                error_sig,
                loaded_name,
                output_pretty,
                output_json,
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

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();
    let (
        model_path,
        device_idx,
        storage_idx,
        compute_idx,
        threshold,
        allow_overlap,
        running,
        error_sig,
        loaded_name,
        diarizer_h,
        loaded_cfg_h,
    ) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::SortformerDiarizer {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                threshold,
                allow_overlap,
                running,
                error,
                loaded_name,
                diarizer,
                loaded_cfg,
                ..
            } => (
                *model_path,
                *device_idx,
                *storage_idx,
                *compute_idx,
                *threshold,
                *allow_overlap,
                *running,
                *error,
                *loaded_name,
                diarizer.clone(),
                loaded_cfg.clone(),
            ),
            _ => return error_widget("SortformerDiarizer: некорректный runtime"),
        },
        Err(_) => return error_widget("SortformerDiarizer: lock error"),
    };

    let diarizer_h_pick = diarizer_h.clone();
    let loaded_h_pick = loaded_cfg_h.clone();
    let on_pick_error = error_sig;
    let on_pick_loaded_name = loaded_name;
    let model_control: Box<dyn Widget> = node_file_picker(
        "Выбрать модель Sortformer (.syn)",
        model_path,
        &[("Syn bundle", &["syn"]), ("Все файлы", &["*"])],
        move |_p| {
            if let Ok(mut g) = diarizer_h_pick.lock() {
                *g = None;
            }
            if let Ok(mut g) = loaded_h_pick.lock() {
                *g = None;
            }
            on_pick_loaded_name.set(None);
            on_pick_error.set(None);
        },
    );

    let device_dd = node_dropdown_field(DEVICE_OPTIONS, device_idx);
    let storage_dd = node_dropdown_field(STORAGE_OPTIONS, storage_idx);
    let compute_dd = node_dropdown_field(COMPUTE_OPTIONS, compute_idx);

    // Threshold slider 0.1..0.9.
    let threshold_slider: Box<dyn Widget> = Box::new(
        Slider::new()
            .value(threshold.get_untracked())
            .range(0.1, 0.9)
            .step(0.01)
            .on_change(move |v| threshold.set(v))
            .class("node-input-slider"),
    );

    // Allow-overlap toggle.
    let allow_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(allow_overlap.get_untracked())
            .on_change(move |v| allow_overlap.set(v))
            .class("node-input-toggle"),
    );

    // Status строка.
    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(format!("Ошибка: {msg}")).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            return vec![
                Box::new(
                    Text::new("Диаризация…")
                        .class("audio-node-meta sortformer-node-running"),
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

    let col: Column = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![
            node_field_row("Модель", model_control),
            node_field_row("Device", device_dd),
            node_field_row("Storage", storage_dd),
            node_field_row("Compute", compute_dd),
            node_field_row("Threshold", threshold_slider),
            node_field_row("Overlap", allow_toggle),
            node_field_row("Статус", status_control),
        ]);

    Box::new(col)
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
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
        .ok_or_else(|| "Подключите audio на вход".to_string())?;
    let values = ctx.values.get_untracked();
    let pv = values
        .get(&(src.from_node, src.from_port))
        .cloned()
        .ok_or_else(|| "Источник не имеет значения".to_string())?;
    match pv {
        PortValue::Audio(b) => Ok(b),
        PortValue::AudioStream(_) => {
            Err("Live-stream пока не поддерживается. Остановите запись.".into())
        }
        _ => Err("На входе нет audio-данных".into()),
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
    diarizer: Arc<Mutex<Option<Diarizer>>>,
    loaded_cfg: Arc<Mutex<Option<SortformerLoadedCfg>>>,
    cfg: SortformerLoadedCfg,
    threshold: f32,
    allow_overlap: bool,
    buf: Arc<AudioBuffer>,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_pretty: RwSignal<String>,
    output_json: RwSignal<String>,
    text_version: RwSignal<u32>,
    resident: bool,
) {
    let needs_load = match loaded_cfg.lock() {
        Ok(g) => match &*g {
            Some(prev) => prev != &cfg,
            None => true,
        },
        Err(_) => true,
    };
    if needs_load {
        ensure_kernels_registered();
        if let Ok(mut g) = diarizer.lock() {
            *g = None;
        }
        loaded_name.set(None);

        let diar_cfg = DiarizationConfig {
            model_path: cfg.model_path.clone(),
            device: device_from_idx(cfg.device_idx),
            storage_dtype: storage_from_idx(cfg.storage_idx),
            compute_dtype: compute_from_idx(cfg.compute_idx),
            threshold,
            allow_overlap,
        };
        let vram_before = crate::models::cuda_allocated();
        match Diarizer::load(diar_cfg) {
            Ok(d) => {
                let bytes = crate::models::cuda_allocated().saturating_sub(vram_before);
                let name = d.model_name().to_string();
                if let Ok(mut g) = diarizer.lock() {
                    *g = Some(d);
                }
                if let Ok(mut g) = loaded_cfg.lock() {
                    *g = Some(cfg.clone());
                }
                loaded_name.set(Some(name.clone()));
                register_in_panel(
                    "Диаризация",
                    "Sortformer",
                    name,
                    device_from_idx(cfg.device_idx),
                    bytes,
                    diarizer.clone(),
                    loaded_cfg.clone(),
                    loaded_name,
                );
            }
            Err(e) => {
                error_sig.set(Some(format!("Не удалось загрузить модель: {e}")));
                running.set(false);
                return;
            }
        }
    } else if let Ok(mut g) = diarizer.lock() {
        if let Some(d) = g.as_mut() {
            d.set_threshold(threshold);
            d.set_allow_overlap(allow_overlap);
        }
    }

    let pcm_mono = downmix_to_mono(&buf.pcm, buf.channels as usize);
    let sr = buf.sample_rate;
    let result = match diarizer.lock() {
        Ok(mut g) => {
            let Some(d) = g.as_mut() else {
                error_sig.set(Some("Модель не загружена".into()));
                running.set(false);
                return;
            };
            d.diarize_pcm(&pcm_mono, sr)
        }
        Err(_) => {
            error_sig.set(Some("Lock error diarizer".into()));
            running.set(false);
            return;
        }
    };

    match result {
        Ok(r) => {
            output_pretty.set(r.to_pretty());
            output_json.set(r.to_json());
            syngui::prelude::run_on_main_thread(move || {
                text_version.update(|v| *v = v.wrapping_add(1));
            });
            error_sig.set(None);
        }
        Err(e) => {
            error_sig.set(Some(format!("Ошибка диаризации: {e}")));
        }
    }
    // Хэндл без резидентности («Держать в памяти» выключен у Syn
    // Checkpoint): слот очищается сразу после прогона, VRAM возвращается.
    if !resident {
        if let Ok(mut g) = diarizer.lock() {
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
