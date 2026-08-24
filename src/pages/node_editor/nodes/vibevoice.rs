use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::input::{MultilineTextEdit, TextField};
use syngui::widgets::{Column, Reactive};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_tts_vibevoice::config::GenerationConfig;
use synaptix_tts_vibevoice::pipeline::{VibeVoicePipeline, VoiceSample};
use synaptix_tts_vibevoice::processor::plain_text_to_script;

use super::super::controls::{
    node_dropdown_field, node_field_row, node_file_picker, node_int_slider_field, node_slider_field,
};
use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    Connection, NodeId, NodeInstance, NodeRuntime, PortValue, VibeVoiceLoadedCfg,
};

pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f16", "f32"];

pub const VOICE_PORTS: [&str; 4] = ["voice1", "voice2", "voice3", "voice4"];

pub fn default_device_idx() -> usize {
    0
}

pub fn default_compute_idx() -> usize {
    0
}

pub fn device_from_idx(i: usize) -> Device {
    match i {
        1 => Device::Cpu,
        _ => Device::Cuda(0),
    }
}

fn compute_from_idx(i: usize) -> DType {
    match COMPUTE_OPTIONS.get(i).copied() {
        Some("f16") => DType::F16,
        Some("f32") => DType::F32,
        _ => DType::BF16,
    }
}

fn ensure_kernels_registered() {
    use std::sync::OnceLock;
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
}

pub struct VibeVoiceExec;

impl NodeExecutor for VibeVoiceExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _script = ctx.read_input("script");
        for port in VOICE_PORTS {
            let _ = ctx.read_input(port);
        }
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::VibeVoice { output_buf, output_version, .. } => {
                    if ctx.track {
                        let _ = output_version.get();
                    }
                    match output_buf.lock() {
                        Ok(b) => b.clone().map(PortValue::Audio).unwrap_or(PortValue::Empty),
                        Err(_) => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("audio", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::VibeVoice { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

struct Snapshot {
    model_path: RwSignal<Option<std::path::PathBuf>>,
    device_idx: RwSignal<usize>,
    compute_idx: RwSignal<usize>,
    script_field: RwSignal<String>,
    cfg_value: RwSignal<f32>,
    ddpm_steps: RwSignal<u32>,
    max_length_times: RwSignal<f32>,
    seed: RwSignal<u64>,
    pipeline: Arc<Mutex<Option<VibeVoicePipeline>>>,
    loaded_cfg: Arc<Mutex<Option<VibeVoiceLoadedCfg>>>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    progress: RwSignal<u32>,
    output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    output_version: RwSignal<u32>,
}

fn snapshot(node: &NodeInstance) -> Option<Snapshot> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::VibeVoice {
                model_path,
                device_idx,
                compute_idx,
                script_field,
                cfg_value,
                ddpm_steps,
                max_length_times,
                seed,
                pipeline,
                loaded_cfg,
                running,
                error,
                loaded_name,
                progress,
                output_buf,
                output_version,
            } => Some(Snapshot {
                model_path: *model_path,
                device_idx: *device_idx,
                compute_idx: *compute_idx,
                script_field: *script_field,
                cfg_value: *cfg_value,
                ddpm_steps: *ddpm_steps,
                max_length_times: *max_length_times,
                seed: *seed,
                pipeline: pipeline.clone(),
                loaded_cfg: loaded_cfg.clone(),
                running: *running,
                error: *error,
                loaded_name: *loaded_name,
                progress: *progress,
                output_buf: output_buf.clone(),
                output_version: *output_version,
            }),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(snap) = snapshot(node) else { return };
    if snap.running.get_untracked() {
        return;
    }

    let handle = super::current_input_syn_model(ctx, node.id);
    let resident = handle.as_ref().map(|h| h.resident).unwrap_or(true);
    let cfg = match &handle {
        Some(h) => VibeVoiceLoadedCfg {
            bundle_path: h.model_path.clone(),
            device_idx: map_handle_device(h.device_idx),
            compute_idx: map_handle_compute(h.compute_idx),
        },
        None => {
            let Some(bundle_path) = snap.model_path.get_untracked() else {
                snap.error.set(Some(
                    "Выберите .syn bundle VibeVoice или подключите Syn Checkpoint".into(),
                ));
                return;
            };
            VibeVoiceLoadedCfg {
                bundle_path,
                device_idx: snap.device_idx.get_untracked(),
                compute_idx: snap.compute_idx.get_untracked(),
            }
        }
    };

    let raw_script = current_input_text(ctx, node.id, "script")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| snap.script_field.get_untracked());
    let script = plain_text_to_script(&raw_script);
    if script.is_empty() {
        snap.error
            .set(Some("Подключите Text на вход «script» или впишите сценарий".into()));
        return;
    }

    let mut voices: Vec<Arc<AudioBuffer>> = Vec::new();
    for port in VOICE_PORTS {
        match current_input_audio_optional(ctx, node.id, port) {
            Some(b) => voices.push(b),
            None => break,
        }
    }

    let gen = GenerationConfig {
        cfg_scale: snap.cfg_value.get_untracked(),
        ddpm_inference_steps: snap.ddpm_steps.get_untracked() as usize,
        max_length_times: snap.max_length_times.get_untracked(),
        seed: snap.seed.get_untracked(),
        ..GenerationConfig::default()
    };

    snap.running.set(true);
    snap.progress.set(0);
    snap.error.set(None);

    let _ = thread::Builder::new()
        .name("synthos-vibevoice-worker".into())
        .spawn(move || {
            synth_worker(snap, cfg, script, voices, gen, resident);
        });
}

fn map_handle_device(pref: usize) -> usize {
    match pref {
        1 => 0,
        2 => 1,
        _ => default_device_idx(),
    }
}

fn map_handle_compute(pref: usize) -> usize {
    match pref {
        1 => 1,
        2 => 0,
        3 => 2,
        _ => default_compute_idx(),
    }
}

fn synth_worker(
    snap: Snapshot,
    cfg: VibeVoiceLoadedCfg,
    script: String,
    voices: Vec<Arc<AudioBuffer>>,
    gen: GenerationConfig,
    resident: bool,
) {
    ensure_kernels_registered();
    let started = super::log_worker_start(
        "vibevoice",
        &format!(
            "голосов {}, cfg {:.2}, шагов {}",
            voices.len(),
            gen.cfg_scale,
            gen.ddpm_inference_steps
        ),
    );

    let needs_load = match snap.loaded_cfg.lock() {
        Ok(g) => match &*g {
            Some(prev) => prev != &cfg,
            None => true,
        },
        Err(_) => true,
    };
    if needs_load {
        if let Ok(mut g) = snap.pipeline.lock() {
            *g = None;
        }
        snap.loaded_name.set(None);
        let device = device_from_idx(cfg.device_idx);
        let dtype = compute_from_idx(cfg.compute_idx);
        let vram_before = crate::models::cuda_allocated();
        match VibeVoicePipeline::from_syn(&cfg.bundle_path, device, dtype) {
            Ok(p) => {
                let bytes = crate::models::cuda_allocated().saturating_sub(vram_before);
                let name = cfg
                    .bundle_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cfg.bundle_path.display().to_string());
                if let Ok(mut g) = snap.pipeline.lock() {
                    *g = Some(p);
                }
                if let Ok(mut g) = snap.loaded_cfg.lock() {
                    *g = Some(cfg.clone());
                }
                snap.loaded_name.set(Some(name.clone()));
                register_in_panel(
                    name,
                    device,
                    bytes,
                    snap.pipeline.clone(),
                    snap.loaded_cfg.clone(),
                    snap.loaded_name,
                );
            }
            Err(e) => {
                let res: std::result::Result<(), String> = Err(format!("загрузка: {e}"));
                super::log_worker_done("vibevoice", started, &res);
                snap.error.set(Some(format!("Не удалось загрузить модель: {e}")));
                snap.running.set(false);
                return;
            }
        }
    }

    let samples: Vec<VoiceSample> = voices
        .iter()
        .map(|b| VoiceSample::new(downmix(b), b.sample_rate))
        .collect();

    let progress = snap.progress;
    let mut last_pct = 0u32;
    let mut on_step = move |step: usize, total: usize| {
        let pct = (step * 100 / total.max(1)) as u32;
        if pct != last_pct {
            last_pct = pct;
            progress.set(pct);
        }
    };

    let result: std::result::Result<(Vec<f32>, u32), String> = match snap.pipeline.lock() {
        Ok(g) => match &*g {
            Some(pl) => pl
                .synthesize_with(&script, &samples, &gen, None, Some(&mut on_step))
                .map(|out| (out.audio, pl.sample_rate()))
                .map_err(|e| e.to_string()),
            None => Err("Pipeline не загружен".to_string()),
        },
        Err(_) => Err("Lock error pipeline".to_string()),
    };

    match &result {
        Ok((pcm, rate)) if !pcm.is_empty() => {
            let buf = Arc::new(AudioBuffer::new(
                Arc::from(pcm.clone().into_boxed_slice()),
                *rate,
                1,
            ));
            if let Ok(mut g) = snap.output_buf.lock() {
                *g = Some(buf);
            }
            let version = snap.output_version;
            syngui::prelude::run_on_main_thread(move || {
                version.update(|v| *v = v.wrapping_add(1));
            });
            snap.error.set(None);
        }
        Ok(_) => snap.error.set(Some("Модель не сгенерировала аудио".into())),
        Err(e) => snap.error.set(Some(format!("Ошибка синтеза: {e}"))),
    }
    super::log_worker_done("vibevoice", started, &result);

    if !resident {
        if let Ok(mut g) = snap.pipeline.lock() {
            *g = None;
        }
        if let Ok(mut g) = snap.loaded_cfg.lock() {
            *g = None;
        }
        snap.loaded_name.set(None);
        crate::models::trim_device(device_from_idx(cfg.device_idx));
    }
    snap.progress.set(0);
    snap.running.set(false);
}

fn downmix(buf: &AudioBuffer) -> Vec<f32> {
    let ch = buf.channels.max(1) as usize;
    if ch == 1 {
        return buf.pcm.to_vec();
    }
    buf.pcm
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

fn current_input_text(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<String> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    match values.get(&(src.from_node, src.from_port)).cloned()? {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

fn current_input_audio_optional(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<Arc<AudioBuffer>> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    match values.get(&(src.from_node, src.from_port)).cloned()? {
        PortValue::Audio(b) => Some(b),
        _ => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some(snap) = snapshot(node) else {
        return error_widget("VibeVoice: некорректный runtime");
    };

    let pipeline_h = snap.pipeline.clone();
    let loaded_h = snap.loaded_cfg.clone();
    let pick_error = snap.error;
    let pick_name = snap.loaded_name;
    let model_control: Box<dyn Widget> = node_file_picker(
        "Выбрать .syn bundle VibeVoice",
        snap.model_path,
        &[("Syn bundle", &["syn"])],
        move |_p| {
            if let Ok(mut g) = pipeline_h.lock() {
                *g = None;
            }
            if let Ok(mut g) = loaded_h.lock() {
                *g = None;
            }
            pick_name.set(None);
            pick_error.set(None);
        },
    );

    let script_field = snap.script_field;
    let script_widget = Box::new(
        MultilineTextEdit::new()
            .text(script_field.get_untracked())
            .placeholder("Speaker 1: Привет!\nSpeaker 2: И тебе привет.")
            .on_change(move |s| script_field.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    let seed = snap.seed;
    let seed_widget = Box::new(
        TextField::new()
            .text(seed.get_untracked().to_string())
            .placeholder("0")
            .on_change(move |s| {
                if let Ok(v) = s.parse::<u64>() {
                    seed.set(v);
                }
            })
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    let (running, error_sig, loaded_name, progress) =
        (snap.running, snap.error, snap.loaded_name, snap.progress);
    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(format!("Ошибка: {msg}")).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            let pct = progress.get();
            let label = if pct > 0 {
                format!("Синтезирование… {pct}%")
            } else {
                "Загрузка модели…".to_string()
            };
            return vec![Box::new(
                Text::new(label).class("audio-node-meta vibevoice-node-running"),
            ) as Box<dyn Widget>];
        }
        if let Some(name) = loaded_name.get() {
            return vec![Box::new(Text::new(name).class("audio-node-meta")) as Box<dyn Widget>];
        }
        vec![Box::new(Text::new("—").class("audio-node-meta")) as Box<dyn Widget>]
    });

    let col: Column = Column::new()
        .gap(3.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![
            node_field_row("Модель", model_control),
            node_field_row("Device", node_dropdown_field(DEVICE_OPTIONS, snap.device_idx)),
            node_field_row("Compute", node_dropdown_field(COMPUTE_OPTIONS, snap.compute_idx)),
            node_field_row("Сценарий", script_widget),
            node_field_row("CFG", node_slider_field(snap.cfg_value, 0.5, 3.0, 0.05, 2)),
            node_field_row("Steps", node_int_slider_field(snap.ddpm_steps, 5, 50, 1)),
            node_field_row(
                "Длина ×",
                node_slider_field(snap.max_length_times, 1.0, 6.0, 0.1, 1),
            ),
            node_field_row("Seed", seed_widget),
            node_field_row("Статус", Box::new(status_text) as Box<dyn Widget>),
        ]);

    Box::new(col)
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
}

fn register_in_panel<T: Send + 'static, C: Send + 'static>(
    label: String,
    device: Device,
    bytes: u64,
    slot: Arc<Mutex<Option<T>>>,
    cfg_slot: Arc<Mutex<Option<C>>>,
    loaded_name: RwSignal<Option<String>>,
) {
    let key = format!("TTS/VibeVoice/{:p}", Arc::as_ptr(&slot));
    crate::models::register_slot(
        key,
        "TTS",
        "VibeVoice",
        label,
        device,
        bytes,
        slot,
        move || {
            if let Ok(mut g) = cfg_slot.lock() {
                *g = None;
            }
            loaded_name.set(None);
        },
    );
}
