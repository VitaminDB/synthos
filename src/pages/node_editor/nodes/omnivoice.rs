//! Body builder + executor для NodeKind::OmniVoice — TTS-нода на базе
//! движка **OmniVoice** (Qwen3-1B LM + Higgs-Audio codec + discrete
//! mask-diffusion).
//!
//! Поток данных:
//! - `evaluate`: пишет текущий `output_buf` (Arc<AudioBuffer>) в порт
//!   `audio`. Если буфера ещё нет — `PortValue::Empty`. Подписка на
//!   `output_version` обеспечивает re-evaluate downstream при появлении
//!   нового результата.
//! - Глобальный Run (или per-node hook) запускает `start`, который читает
//!   `text`/`ref_audio`/`ref_text` со входов, авто-выбирает `GenerationMode`
//!   и спавнит worker-thread. Worker лениво загружает `OmniVoicePipeline`
//!   (~30 с на CPU при первом запуске), вызывает `synthesize`, заворачивает
//!   PCM в `AudioBuffer` (24 kHz mono), кладёт в `output_buf` и бампает
//!   `output_version` через `run_on_main_thread`.
//!
//! Mode resolution: ref_audio есть → `Clone(VoiceClonePrompt)`; иначе
//! instruct непуст → `Design{instruct}`; иначе → `Auto`. Никаких явных
//! переключателей режима в UI — пользователь подключает порт / заполняет
//! textfield, mode подбирается автоматически.
//!
//! Layout body — Demo-стилистика (`field_like_row`): [label | spacer |
//! control]. MSS-классы — общие `node-card-field-*` + `node-input-*`,
//! плюс собственный `omnivoice-node-running` для пульса при синтезе.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_core::dtype::DType as StorageDType;
use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::input::TextField;
use syngui::widgets::{Column, Reactive};

use super::super::controls::{
    node_dropdown_field, node_field_row, node_file_picker, node_int_slider_field, node_slider_field,
};

use synaptix::facade::tts::core::{GenerationConfig, GenerationMode, VoiceClonePrompt};
use synaptix::facade::tts::TtsPipeline as OmniVoicePipeline;

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    Connection, NodeId, NodeInstance, NodeRuntime, OmniLoadedCfg, PortValue,
};

// ── Опции UI dropdown'ов ──────────────────────────────────────────────────

/// Доступные устройства. 0 = CPU, 1 = GPU (CUDA → Metal auto-detect).
pub const DEVICE_OPTIONS: &[&str] = &["CPU", "GPU (auto)"];

/// Storage dtype для LM весов в `.syn` bundle. Пробрасывается в
/// `OmniVoicePipeline::from_syn` через `synaptix_core::dtype::DType` — для
/// MXFP8/NVFP4 включает PTQ on-load квантизацию Qwen3 linear-слоёв.
pub const STORAGE_OPTIONS: &[&str] = &["f16", "bf16", "f32", "nvfp4", "mxfp8"];

/// Compute dtype для forward LM/codec и dequant в quantized GEMM.
pub const COMPUTE_OPTIONS: &[&str] = &["f16", "bf16", "f32", "nvfp4", "mxfp8"];

pub fn default_storage_idx() -> usize {
    // f16 — баланс скорости/качества, как у GigaAM-ноды.
    0
}
pub fn default_compute_idx() -> usize {
    0
}

fn device_from_idx(i: usize) -> Device {
    match i {
        1 => OmniVoicePipeline::best_device(),
        _ => Device::Cpu,
    }
}

fn storage_from_idx(i: usize) -> StorageDType {
    match STORAGE_OPTIONS.get(i).copied() {
        Some("bf16") => StorageDType::BF16,
        Some("f32") => StorageDType::F32,
        Some("nvfp4") => StorageDType::NVFP4,
        Some("mxfp8") => StorageDType::MXFP8,
        _ => StorageDType::F16,
    }
}

fn compute_from_idx(i: usize) -> DType {
    match COMPUTE_OPTIONS.get(i).copied() {
        Some("bf16") => DType::BF16,
        Some("f32") => DType::F32,
        Some("nvfp4") => DType::NVFP4,
        Some("mxfp8") => DType::MXFP8,
        _ => DType::F16,
    }
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

/// Executor ноды: пишет последний синтезированный буфер в порт `audio`.
/// Реальный синтез запускается из `start()` (Per-node Play / глобальный Run).
pub struct OmniVoiceExec;

impl NodeExecutor for OmniVoiceExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        // Прочитаем входы — нужны исключительно для пометки зависимости в
        // графе (downstream-evaluate каскадно перестроится при изменении).
        let _text_pv = ctx.read_input("text");
        let _ref_audio_pv = ctx.read_input("ref_audio");
        let _ref_text_pv = ctx.read_input("ref_text");
        let track = ctx.track;

        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::OmniVoice {
                    output_buf,
                    output_version,
                    ..
                } => {
                    if track {
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

// ── Hooks для run_controls ────────────────────────────────────────────────

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::OmniVoice { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

// ── Запуск синтеза ────────────────────────────────────────────────────────

/// Запустить синтез ноды. Идемпотентно: если worker уже работает — no-op.
/// При ошибках (нет модели / нет текста) выставляет `error` и возвращается.
pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    // Снимок всех сигналов + Arc-clone handle'ов, чтобы не держать lock
    // на runtime пока работает worker (lock держит pipeline, ref_audio
    // декодируется через symphonia — параллельные ноды должны иметь
    // доступ к runtime для UI-отрисовки).
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::OmniVoice {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                instruct,
                ref_text_field,
                language,
                num_step,
                guidance_scale,
                t_shift,
                speed,
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
                *storage_idx,
                *compute_idx,
                *instruct,
                *ref_text_field,
                *language,
                *num_step,
                *guidance_scale,
                *t_shift,
                *speed,
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
        storage_idx,
        compute_idx,
        instruct,
        ref_text_field,
        language,
        num_step,
        guidance_scale,
        t_shift,
        speed,
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

    // 1. Bundle path — обязателен, должен указывать на `.syn`.
    let Some(bundle_path) = model_path.get_untracked() else {
        error_sig.set(Some("Выберите .syn bundle OmniVoice".into()));
        return;
    };

    // 2. Target-text — обязателен.
    let text = match current_input_text(ctx, node.id, "text") {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            error_sig.set(Some("Подключите Text на вход «text»".into()));
            return;
        }
    };

    // 3. Опциональный ref_audio.
    let ref_audio_buf = current_input_audio_optional(ctx, node.id, "ref_audio");

    // 4. ref_text — приоритет порта, иначе textfield.
    let ref_text_opt = current_input_text(ctx, node.id, "ref_text")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let t = ref_text_field.get_untracked();
            (!t.trim().is_empty()).then_some(t)
        });

    let instruct_text = instruct.get_untracked();
    let language_text = language.get_untracked();

    let cfg = OmniLoadedCfg {
        bundle_path,
        device_idx: device_idx.get_untracked(),
        storage_idx: storage_idx.get_untracked(),
        compute_idx: compute_idx.get_untracked(),
    };
    let gen_cfg = GenerationConfig {
        num_step: num_step.get_untracked(),
        guidance_scale: guidance_scale.get_untracked(),
        t_shift: t_shift.get_untracked(),
        speed: speed.get_untracked(),
        seed: seed.get_untracked(),
        ..Default::default()
    };

    running.set(true);
    error_sig.set(None);

    let _ = thread::Builder::new()
        .name("synthos-omnivoice-worker".into())
        .spawn(move || {
            synth_worker(
                pipeline,
                loaded_cfg,
                cfg,
                text,
                ref_audio_buf,
                ref_text_opt,
                instruct_text,
                language_text,
                gen_cfg,
                running,
                error_sig,
                loaded_name,
                output_buf,
                output_version,
            );
        });
}

// ── Worker ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn synth_worker(
    pipeline: Arc<Mutex<Option<OmniVoicePipeline>>>,
    loaded_cfg: Arc<Mutex<Option<OmniLoadedCfg>>>,
    cfg: OmniLoadedCfg,
    text: String,
    ref_audio_buf: Option<Arc<AudioBuffer>>,
    ref_text_opt: Option<String>,
    instruct_text: String,
    language_text: String,
    gen_cfg: GenerationConfig,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_buf: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    output_version: RwSignal<u32>,
) {
    // 1. Lazy-load pipeline'а при несовпадении snapshot'а.
    let needs_load = match loaded_cfg.lock() {
        Ok(g) => match &*g {
            Some(prev) => prev != &cfg,
            None => true,
        },
        Err(_) => true,
    };
    if needs_load {
        ensure_kernels_registered();
        if let Ok(mut g) = pipeline.lock() {
            *g = None;
        }
        loaded_name.set(None);
        let device = device_from_idx(cfg.device_idx);
        let storage = storage_from_idx(cfg.storage_idx);
        let compute = compute_from_idx(cfg.compute_idx);
        match OmniVoicePipeline::from_syn(&cfg.bundle_path, &device, storage, compute) {
            Ok(p) => {
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
                loaded_name.set(Some(name));
            }
            Err(e) => {
                error_sig.set(Some(format!("Не удалось загрузить модель: {e}")));
                running.set(false);
                return;
            }
        }
    }

    // 2. Если есть ref_audio — сохраняем во временный WAV (symphonia в
    //    pipeline'е сам декодирует и ресэмплит в 24 kHz). Удалим по
    //    завершении.
    let tmp_ref_wav = match ref_audio_buf {
        Some(buf) => match save_ref_audio_to_tmp_wav(&buf) {
            Ok(p) => Some(p),
            Err(e) => {
                error_sig.set(Some(format!("Не удалось сохранить ref-аудио: {e}")));
                running.set(false);
                return;
            }
        },
        None => None,
    };

    // 3. Сборка GenerationMode по приоритету: Clone > Design > Auto.
    let mode = if let Some(path) = tmp_ref_wav.clone() {
        let mut prompt = VoiceClonePrompt::new(path);
        if let Some(rt) = ref_text_opt.clone() {
            prompt = prompt.with_text(rt);
        }
        if !language_text.trim().is_empty() {
            prompt = prompt.with_lang(language_text.clone());
        }
        GenerationMode::Clone(prompt)
    } else if !instruct_text.trim().is_empty() {
        GenerationMode::Design { instruct: instruct_text }
    } else {
        GenerationMode::Auto
    };

    // 4. Synthesize (sync). Держим lock pipeline'а на всё время инференса —
    //    в одном экземпляре ноды нет смысла запускать параллельные синтезы
    //    (running уже отсекает повторный Play).
    let synth_result = match pipeline.lock() {
        Ok(g) => match &*g {
            Some(pl) => pl
                .synthesize(&text, &mode, &gen_cfg)
                .map(|pcm| (pcm, pl.sample_rate())),
            None => Err(synaptix::facade::tts::core::OmniVoiceError::Inference(
                "Pipeline не загружен".into(),
            )),
        },
        Err(_) => Err(synaptix::facade::tts::core::OmniVoiceError::Inference(
            "Lock error pipeline".into(),
        )),
    };

    // 5. Cleanup tmp ref WAV (best-effort).
    if let Some(p) = tmp_ref_wav {
        let _ = std::fs::remove_file(p);
    }

    // 6. Публикация результата + UI.
    match synth_result {
        Ok((pcm, sr)) => {
            let buf = Arc::new(AudioBuffer::new(Arc::from(pcm.into_boxed_slice()), sr, 1));
            if let Ok(mut g) = output_buf.lock() {
                *g = Some(buf);
            }
            // `update` валится с не-main треда (signal-RUNTIME thread_local
            // пуст в worker'е). Маршалим bump на main thread целиком.
            syngui::prelude::run_on_main_thread(move || {
                output_version.update(|v| *v = v.wrapping_add(1));
            });
            error_sig.set(None);
        }
        Err(e) => {
            error_sig.set(Some(format!("Ошибка синтеза: {e}")));
        }
    }
    running.set(false);
}

// ── Tmp WAV для ref_audio ─────────────────────────────────────────────────

fn save_ref_audio_to_tmp_wav(buf: &AudioBuffer) -> std::result::Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("synthos-omnivoice-ref-{pid}-{nanos}.wav"));

    let spec = hound::WavSpec {
        channels: buf.channels.max(1),
        sample_rate: buf.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec)
        .map_err(|e| format!("WavWriter::create({}): {e}", path.display()))?;
    for s in buf.pcm.iter() {
        writer
            .write_sample(*s)
            .map_err(|e| format!("write_sample: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("WavWriter::finalize: {e}"))?;
    Ok(path)
}

// ── Helpers: чтение портов ────────────────────────────────────────────────

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

// ── Body builder ──────────────────────────────────────────────────────────

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let snapshot = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::OmniVoice {
                model_path,
                device_idx,
                storage_idx,
                compute_idx,
                instruct,
                ref_text_field,
                language,
                num_step,
                guidance_scale,
                t_shift,
                speed,
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
                *storage_idx,
                *compute_idx,
                *instruct,
                *ref_text_field,
                *language,
                *num_step,
                *guidance_scale,
                *t_shift,
                *speed,
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
        storage_idx,
        compute_idx,
        instruct,
        ref_text_field,
        language,
        num_step,
        guidance_scale,
        t_shift,
        speed,
        seed,
        pipeline_handle,
        loaded_cfg_handle,
        running,
        error_sig,
        loaded_name,
    )) = snapshot
    else {
        return error_widget("OmniVoice: некорректный runtime");
    };

    let pipeline_h = pipeline_handle.clone();
    let loaded_h = loaded_cfg_handle.clone();
    let on_pick_error = error_sig;
    let on_pick_loaded_name = loaded_name;
    let model_control: Box<dyn Widget> = node_file_picker(
        "Выбрать .syn bundle OmniVoice",
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
    let storage_dd = node_dropdown_field(STORAGE_OPTIONS, storage_idx);
    let compute_dd = node_dropdown_field(COMPUTE_OPTIONS, compute_idx);

    // ── Row 5: Instruct (textfield) ─────────────────────────────────────
    //
    // Используется только когда нет ref_audio. Single-line `TextField` —
    // визуально согласован с остальными textfield'ами ноды
    // (`.node-input-text` уже стилизован под единый паттерн).
    let instruct_field = Box::new(
        TextField::new()
            .text(instruct.get_untracked())
            .placeholder("женский низкий тембр")
            .on_change(move |s| instruct.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    // ── Row 6: Ref text (fallback-textfield, single-line) ──────────────
    let ref_text_widget = Box::new(
        TextField::new()
            .text(ref_text_field.get_untracked())
            .placeholder("Транскрипт ref-аудио (опционально)")
            .on_change(move |s| ref_text_field.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    // ── Row 7: Language ─────────────────────────────────────────────────
    let language_widget = Box::new(
        TextField::new()
            .text(language.get_untracked())
            .placeholder("ru")
            .on_change(move |s| language.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    let cfg_control = node_slider_field(guidance_scale, 0.0, 5.0, 0.05, 2);
    let steps_control = node_int_slider_field(num_step, 8, 64, 1);
    let tshift_control = node_slider_field(t_shift, 0.0, 1.0, 0.01, 2);
    let speed_control = node_slider_field(speed, 0.5, 2.0, 0.01, 2);

    // Seed — TextField (u64 не помещается в Slider). Парсим строку,
    // некорректные ввод игнорируется.
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

    // ── Row 13: Статус ──────────────────────────────────────────────────
    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(format!("Ошибка: {msg}")).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            return vec![Box::new(
                Text::new("Синтезирование…").class("audio-node-meta omnivoice-node-running"),
            ) as Box<dyn Widget>];
        }
        if let Some(name) = loaded_name.get() {
            return vec![
                Box::new(Text::new(name).class("audio-node-meta")) as Box<dyn Widget>
            ];
        }
        vec![Box::new(Text::new("—").class("audio-node-meta")) as Box<dyn Widget>]
    });
    let status_control: Box<dyn Widget> = Box::new(status_text);

    // ── Сборка ───────────────────────────────────────────────────────────
    //
    // gap=3.0 — небольшой воздух между field-row'ами (без него подряд идущие
    // dropdown'ы / textfield'ы визуально слипаются, как было на ранней
    // итерации). У GigaAM ноды строк меньше — там gap=0 ещё читается.
    let col: Column = Column::new()
        .gap(3.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![
            node_field_row("Модель", model_control),
            node_field_row("Device", device_dd),
            node_field_row("Storage", storage_dd),
            node_field_row("Compute", compute_dd),
            node_field_row("Instruct", instruct_field),
            node_field_row("Ref text", ref_text_widget),
            node_field_row("Language", language_widget),
            node_field_row("CFG", cfg_control),
            node_field_row("Steps", steps_control),
            node_field_row("T-shift", tshift_control),
            node_field_row("Speed", speed_control),
            node_field_row("Seed", seed_widget),
            node_field_row("Статус", status_control),
        ]);

    Box::new(col)
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
}
