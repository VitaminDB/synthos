use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::input::TextField;
use syngui::widgets::{Column, Reactive};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_tts_voxcpm::{GenerateOptions, VoxCpmPipeline, Waveform};

use super::super::controls::{
    node_dropdown_field, node_field_row, node_file_picker, node_int_slider_field, node_slider_field,
};
use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    Connection, NodeId, NodeInstance, NodeRuntime, PortValue, VoxCpm2LoadedCfg,
};

pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f16", "f32"];

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

pub struct VoxCpm2Exec;

impl NodeExecutor for VoxCpm2Exec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _text = ctx.read_input("text");
        let _ref_audio = ctx.read_input("ref_audio");
        let _prompt_audio = ctx.read_input("prompt_audio");
        let _prompt_text = ctx.read_input("prompt_text");

        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::VoxCpm2 { output_buf, output_version, .. } => {
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
            NodeRuntime::VoxCpm2 { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::VoxCpm2 {
                model_path,
                device_idx,
                compute_idx,
                prompt_text_field,
                cfg_value,
                n_timesteps,
                max_len,
                seed,
                pipeline,
                loaded_cfg,
                running,
                error,
                loaded_name,
                output_buf,
                output_version,
            } => Some((
                *model_path,
                *device_idx,
                *compute_idx,
                *prompt_text_field,
                *cfg_value,
                *n_timesteps,
                *max_len,
                *seed,
                pipeline.clone(),
                loaded_cfg.clone(),
                *running,
                *error,
                *loaded_name,
                output_buf.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        model_path,
        device_idx,
        compute_idx,
        prompt_text_field,
        cfg_value,
        n_timesteps,
        max_len,
        seed,
        pipeline,
        loaded_cfg,
        running,
        error_sig,
        loaded_name,
        output_buf,
        output_version,
    )) = snapshot
    else {
        return;
    };

    if running.get_untracked() {
        return;
    }

    // Хэндл Syn Checkpoint (вход `model`) переопределяет собственные поля;
    // оттуда же — резидентность. Без хэндла — legacy-поведение слота.
    let handle = super::current_input_syn_model(ctx, node.id);
    let resident = handle.as_ref().map(|h| h.resident).unwrap_or(true);
    let cfg = match &handle {
        Some(h) => VoxCpm2LoadedCfg {
            bundle_path: h.model_path.clone(),
            device_idx: map_handle_device(h.device_idx),
            compute_idx: map_handle_compute(h.compute_idx),
        },
        None => {
            let Some(bundle_path) = model_path.get_untracked() else {
                error_sig.set(Some(
                    "Выберите .syn bundle VoxCPM2 или подключите Syn Checkpoint".into(),
                ));
                return;
            };
            VoxCpm2LoadedCfg {
                bundle_path,
                device_idx: device_idx.get_untracked(),
                compute_idx: compute_idx.get_untracked(),
            }
        }
    };

    let text = match current_input_text(ctx, node.id, "text") {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            error_sig.set(Some("Подключите Text на вход «text»".into()));
            return;
        }
    };

    let ref_audio_buf = current_input_audio_optional(ctx, node.id, "ref_audio");
    let prompt_audio_buf = current_input_audio_optional(ctx, node.id, "prompt_audio");
    let prompt_text_opt = current_input_text(ctx, node.id, "prompt_text")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let t = prompt_text_field.get_untracked();
            (!t.trim().is_empty()).then_some(t)
        });

    if prompt_audio_buf.is_some() && prompt_text_opt.is_none() {
        error_sig.set(Some("Для prompt-audio нужен prompt-text (транскрипт всего аудио)".into()));
        return;
    }

    let opts = GenerateOptions {
        cfg_value: cfg_value.get_untracked(),
        n_timesteps: n_timesteps.get_untracked() as usize,
        seed: seed.get_untracked(),
        max_len: max_len.get_untracked() as usize,
        ..GenerateOptions::default()
    };

    running.set(true);
    error_sig.set(None);

    let _ = thread::Builder::new()
        .name("synthos-voxcpm2-worker".into())
        .spawn(move || {
            synth_worker(
                pipeline,
                loaded_cfg,
                cfg,
                text,
                ref_audio_buf,
                prompt_audio_buf,
                prompt_text_opt,
                opts,
                running,
                error_sig,
                loaded_name,
                output_buf,
                output_version,
                resident,
            );
        });
}

/// Маппинг предпочтений Syn Checkpoint на индексы опций семейства
/// (Auto → дефолт). Storage-предпочтение семейству не нужно.
fn map_handle_device(pref: usize) -> usize {
    match pref {
        1 => 0, // CUDA
        2 => 1, // CPU
        _ => default_device_idx(),
    }
}

fn map_handle_compute(pref: usize) -> usize {
    // COMPUTE_OPTIONS: ["bf16", "f16", "f32"].
    match pref {
        1 => 1,
        2 => 0,
        3 => 2,
        _ => default_compute_idx(),
    }
}

#[allow(clippy::too_many_arguments)]
fn synth_worker(
    pipeline: Arc<Mutex<Option<VoxCpmPipeline>>>,
    loaded_cfg: Arc<Mutex<Option<VoxCpm2LoadedCfg>>>,
    cfg: VoxCpm2LoadedCfg,
    text: String,
    ref_audio_buf: Option<Arc<AudioBuffer>>,
    prompt_audio_buf: Option<Arc<AudioBuffer>>,
    prompt_text_opt: Option<String>,
    opts: GenerateOptions,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    output_version: RwSignal<u32>,
    resident: bool,
) {
    ensure_kernels_registered();

    let needs_load = match loaded_cfg.lock() {
        Ok(g) => match &*g {
            Some(prev) => prev != &cfg,
            None => true,
        },
        Err(_) => true,
    };
    if needs_load {
        if let Ok(mut g) = pipeline.lock() {
            *g = None;
        }
        loaded_name.set(None);
        let device = device_from_idx(cfg.device_idx);
        let dtype = compute_from_idx(cfg.compute_idx);
        let vram_before = crate::models::cuda_allocated();
        match VoxCpmPipeline::from_bundle(&cfg.bundle_path, device, dtype) {
            Ok(p) => {
                let bytes = crate::models::cuda_allocated().saturating_sub(vram_before);
                let name = cfg
                    .bundle_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cfg.bundle_path.display().to_string());
                if let Ok(mut g) = pipeline.lock() {
                    *g = Some(p);
                }
                if let Ok(mut g) = loaded_cfg.lock() {
                    *g = Some(cfg.clone());
                }
                loaded_name.set(Some(name.clone()));
                register_in_panel(
                    "TTS",
                    "VoxCPM",
                    name,
                    device,
                    bytes,
                    pipeline.clone(),
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
    }

    let tmp_ref = match ref_audio_buf {
        Some(buf) => match save_audio_to_tmp_wav(&buf, "ref") {
            Ok(p) => Some(p),
            Err(e) => {
                error_sig.set(Some(format!("Не удалось сохранить ref-аудио: {e}")));
                running.set(false);
                return;
            }
        },
        None => None,
    };
    let tmp_prompt = match prompt_audio_buf {
        Some(buf) => match save_audio_to_tmp_wav(&buf, "prompt") {
            Ok(p) => Some(p),
            Err(e) => {
                if let Some(p) = &tmp_ref {
                    let _ = std::fs::remove_file(p);
                }
                error_sig.set(Some(format!("Не удалось сохранить prompt-аудио: {e}")));
                running.set(false);
                return;
            }
        },
        None => None,
    };

    let synth_result = match pipeline.lock() {
        Ok(g) => match &*g {
            Some(pl) => synthesize_by_mode(pl, &text, &tmp_ref, &tmp_prompt, &prompt_text_opt, &opts),
            None => Err("Pipeline не загружен".into()),
        },
        Err(_) => Err("Lock error pipeline".into()),
    };

    if let Some(p) = tmp_ref {
        let _ = std::fs::remove_file(p);
    }
    if let Some(p) = tmp_prompt {
        let _ = std::fs::remove_file(p);
    }

    match synth_result {
        Ok(wav) => {
            let sr = wav.sample_rate as u32;
            let buf = Arc::new(AudioBuffer::new(Arc::from(wav.pcm.into_boxed_slice()), sr, 1));
            if let Ok(mut g) = output_buf.lock() {
                *g = Some(buf);
            }
            syngui::prelude::run_on_main_thread(move || {
                output_version.update(|v| *v = v.wrapping_add(1));
            });
            error_sig.set(None);
        }
        Err(e) => {
            error_sig.set(Some(format!("Ошибка синтеза: {e}")));
        }
    }
    // Хэндл без резидентности («Держать в памяти» выключен у Syn
    // Checkpoint): слот очищается сразу после прогона, VRAM возвращается.
    if !resident {
        if let Ok(mut g) = pipeline.lock() {
            *g = None;
        }
        if let Ok(mut g) = loaded_cfg.lock() {
            *g = None;
        }
        loaded_name.set(None);
        crate::models::trim_device(device_from_idx(cfg.device_idx));
    }
    running.set(false);
}

fn synthesize_by_mode(
    pl: &VoxCpmPipeline,
    text: &str,
    tmp_ref: &Option<PathBuf>,
    tmp_prompt: &Option<PathBuf>,
    prompt_text_opt: &Option<String>,
    opts: &GenerateOptions,
) -> std::result::Result<Waveform, String> {
    let map_err = |e: synaptix_tts_voxcpm::VoxError| e.to_string();
    match (tmp_ref, tmp_prompt, prompt_text_opt) {
        (Some(r), Some(pw), Some(pt)) => {
            let (rs, pws) = (path_str(r)?, path_str(pw)?);
            pl.synthesize_combined(text, pt, pws, rs, opts).map_err(map_err)
        }
        (Some(r), None, _) => {
            let rs = path_str(r)?;
            pl.synthesize_with_reference(text, rs, opts).map_err(map_err)
        }
        (None, Some(pw), Some(pt)) => {
            let pws = path_str(pw)?;
            pl.synthesize_continuation(text, pt, pws, opts).map_err(map_err)
        }
        _ => pl.synthesize(text, opts).map_err(map_err),
    }
}

fn path_str(p: &PathBuf) -> std::result::Result<&str, String> {
    p.to_str().ok_or_else(|| format!("bad path: {}", p.display()))
}

fn save_audio_to_tmp_wav(buf: &AudioBuffer, tag: &str) -> std::result::Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("synthos-voxcpm2-{tag}-{pid}-{nanos}.wav"));

    let spec = hound::WavSpec {
        channels: buf.channels.max(1),
        sample_rate: buf.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec)
        .map_err(|e| format!("WavWriter::create({}): {e}", path.display()))?;
    for s in buf.pcm.iter() {
        writer.write_sample(*s).map_err(|e| format!("write_sample: {e}"))?;
    }
    writer.finalize().map_err(|e| format!("WavWriter::finalize: {e}"))?;
    Ok(path)
}

fn current_input_text(ctx: &NodeEditorCtx, node_id: NodeId, port: &'static str) -> Option<String> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    match pv {
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
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    match pv {
        PortValue::Audio(b) => Some(b),
        _ => None,
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::VoxCpm2 {
                model_path,
                device_idx,
                compute_idx,
                prompt_text_field,
                cfg_value,
                n_timesteps,
                max_len,
                seed,
                pipeline,
                loaded_cfg,
                running,
                error,
                loaded_name,
                ..
            } => Some((
                *model_path,
                *device_idx,
                *compute_idx,
                *prompt_text_field,
                *cfg_value,
                *n_timesteps,
                *max_len,
                *seed,
                pipeline.clone(),
                loaded_cfg.clone(),
                *running,
                *error,
                *loaded_name,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        model_path,
        device_idx,
        compute_idx,
        prompt_text_field,
        cfg_value,
        n_timesteps,
        max_len,
        seed,
        pipeline_handle,
        loaded_cfg_handle,
        running,
        error_sig,
        loaded_name,
    )) = snapshot
    else {
        return error_widget("VoxCpm2: некорректный runtime");
    };

    let pipeline_h = pipeline_handle.clone();
    let loaded_h = loaded_cfg_handle.clone();
    let on_pick_error = error_sig;
    let on_pick_loaded_name = loaded_name;
    let model_control: Box<dyn Widget> = node_file_picker(
        "Выбрать .syn bundle VoxCPM2",
        model_path,
        &[("Syn bundle", &["syn"])],
        move |_p| {
            if let Ok(mut g) = pipeline_h.lock() {
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
    let compute_dd = node_dropdown_field(COMPUTE_OPTIONS, compute_idx);

    let prompt_text_widget = Box::new(
        TextField::new()
            .text(prompt_text_field.get_untracked())
            .placeholder("Транскрипт prompt-аудио (весь текст)")
            .on_change(move |s| prompt_text_field.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    let cfg_control = node_slider_field(cfg_value, 0.0, 5.0, 0.05, 2);
    let steps_control = node_int_slider_field(n_timesteps, 4, 50, 1);
    let max_len_control = node_int_slider_field(max_len, 200, 4000, 100);

    let seed_widget = Box::new(
        TextField::new()
            .text(seed.get_untracked().to_string())
            .placeholder("1988")
            .on_change(move |s| {
                if let Ok(v) = s.parse::<u64>() {
                    seed.set(v);
                }
            })
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(format!("Ошибка: {msg}")).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            return vec![Box::new(
                Text::new("Синтезирование…").class("audio-node-meta voxcpm-node-running"),
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
            node_field_row("Device", device_dd),
            node_field_row("Compute", compute_dd),
            node_field_row("Prompt text", prompt_text_widget),
            node_field_row("CFG", cfg_control),
            node_field_row("Steps", steps_control),
            node_field_row("Max len", max_len_control),
            node_field_row("Seed", seed_widget),
            node_field_row("Статус", Box::new(status_text) as Box<dyn Widget>),
        ]);

    Box::new(col)
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
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
