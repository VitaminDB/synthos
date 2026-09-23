//! Body builder + executor для NodeKind::Llm — ПРОСТАЯ LLM-нода (one-shot
//! prompt→answer) на общем фасаде `synaptix::facade::llm`. Без кастома: детект
//! арх, jinja-шаблон чата и стоп-токены — внутри фасада (qwen3/hybrid/llama/
//! gemma3). Для полного чата с agent-loop/инструментами — отдельная нода-чат.
//!
//! Поток данных:
//! - `evaluate`: пишет текущий `output_text` в порт `answer` (Text). Подписка
//!   на сам `output_text` (tracked) перестраивает downstream при стриминге.
//! - Глобальный Run (или per-node hook) вызывает `start`: читает `prompt`
//!   (вопрос) и опциональный `system` со входов, спавнит worker. Worker лениво
//!   грузит модель через `load_llm` (арх детектится в фасаде), строит промпт
//!   через `LlmTokenizer::apply_chat_template_ex_tools` и стримит ответ через
//!   `LlmGeneration::generate_streaming` (callback с текстовой дельтой пишет в
//!   `output_text`). `cancel`-флаг прерывает стрим (callback → false).
//!
//! Модель и её точность приходят только из Syn-чекпойнта на входе `model`:
//! предпочтения storage/compute маппятся на quant (none/nvfp4/mxfp8) +
//! compute (bf16/f16/f32) этого семейства.
//!
//! Layout body — Demo-стилистика (`node_field_row`): [label | spacer | control].

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::input::{TextField, Toggle};
use syngui::widgets::{Column, Reactive};

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_core::precision::PrecisionConfig;
use synaptix::facade::llm::{load_llm, GenerationOptions, Llm, LlmGeneration, LlmTokenizer, Message};

use super::super::controls::{
    node_field_row, node_int_slider_field, node_slider_field,
};
use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    Connection, LlmLoadedCfg, NodeId, NodeInstance, NodeRuntime, PortValue,
};

// ── Опции UI dropdown'ов ──────────────────────────────────────────────────

/// Устройство инференса. 0 = CUDA (основной путь), 1 = CPU (smoke/отладка).
pub const DEVICE_OPTIONS: &[&str] = &["CUDA", "CPU"];

/// Квант весов. `none` → dense (compute-dtype). `nvfp4`/`mxfp8` → квант attn+mlp
/// (+lm_head для nvfp4); compute форсится в F16 (квант-ядра требуют F16-актив).
pub const QUANT_OPTIONS: &[&str] = &["none", "nvfp4", "mxfp8"];

/// Compute dtype активаций для dense-режима (`quant=none`). Для квант-пресетов
/// игнорируется (F16 из пресета).
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f16", "f32"];

pub fn default_device_idx() -> usize {
    0
}
pub fn default_quant_idx() -> usize {
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

/// Собирает [`PrecisionConfig`] из dropdown'ов. `none` → dense(compute);
/// квант-пресет берёт собственный compute (F16) — dropdown compute для кванта
/// игнорируется, иначе `validate()` отверг бы bf16+квант.
fn precision_from_idx(quant_idx: usize, compute_idx: usize) -> std::result::Result<PrecisionConfig, String> {
    let p = match QUANT_OPTIONS.get(quant_idx).copied() {
        Some("nvfp4") => PrecisionConfig::nvfp4(),
        Some("mxfp8") => PrecisionConfig::mxfp8(),
        _ => PrecisionConfig::dense(compute_from_idx(compute_idx)),
    };
    p.validate()?;
    Ok(p)
}

// ── Загруженная модель (фасад synaptix) ────────────────────────────────────

/// Загруженная LLM ноды: фасадные `Llm` + `LlmTokenizer`. Детект арх, jinja-
/// шаблон чата и стоп-токены живут в `synaptix::facade::llm` — здесь только
/// хранение. Живёт в `NodeRuntime::Llm.pipeline` (Arc<Mutex<Option<…>>>),
/// пересоздаётся при смене `LlmLoadedCfg`.
/// Малая карта: `load_llm` кладёт блоки на карту, пока они влезают, и места
/// на KV, активации и стримящиеся блоки генерации может не остаться (7 ГБ:
/// `pinned_htod OOM` на первом шаге). Перед генерацией часть блоков уезжает на
/// хост — как `fit_blocks_for_context` у чата. И обратно: если модель при
/// загрузке целиком ушла в оффлоад (попытка «всё на карту» упала OOM), а
/// места хватает на часть блоков, они возвращаются — с запасом в два блока,
/// как у чата. Без этого 27B в FP8 на 24 ГБ стримила все 64 блока, и
/// переписывание промпта в шаблоне FLUX.2 шло 4 минуты.
fn fit_residency(model: &Llm, tokens: usize) {
    let (Some((block_bytes, total)), Some(resident)) = (model.block_offload_shape(), model.resident_blocks()) else {
        return; // архитектура со своим оффлоадом (MoE) — не наше дело
    };
    let Device::Cuda(ord) = *model.device() else { return };
    if block_bytes == 0 || total == 0 {
        return;
    }
    let _ = synaptix_core::device::cuda::synchronize_all(ord);
    let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(ord);
    let free = synaptix_core::device::cuda::mem_info(ord).map(|(f, _)| f).unwrap_or(usize::MAX);
    let need = tokens * model.kv_bytes_per_token() + model.kv_fixed_bytes(tokens) + 2 * block_bytes + (768usize << 20);
    if free >= need {
        let spare = (free - need) / block_bytes;
        if resident < total && spare >= 2 {
            let want = if resident + spare + 1 >= total { total } else { resident + spare };
            let got = model.set_block_residency(want).unwrap_or(resident);
            let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(ord);
            tracing::info!(
                target: super::WORKER_LOG,
                "LLM: блоки вернулись на карту {resident} → {got} из {total} (свободно {} МБ, ходу нужно {} МБ)",
                free >> 20,
                need >> 20
            );
        }
        return;
    }
    let evict = (need - free).div_ceil(block_bytes);
    let want = resident.saturating_sub(evict);
    let got = model.set_block_residency(want).unwrap_or(resident);
    let _ = synaptix_core::memory::cuda_pool::hard_trim_all_pools_device(ord);
    tracing::info!(
        target: super::WORKER_LOG,
        "LLM: блоков на карте {resident} → {got} из {total} (свободно {} МБ, нужно {} МБ на {tokens} токенов)",
        free >> 20,
        need >> 20
    );
}

pub struct LlmPipeline {
    model: Llm,
    tokenizer: LlmTokenizer,
}

impl LlmPipeline {
    fn load(
        model: &Path,
        device: Device,
        precision: PrecisionConfig,
        max_seq: Option<usize>,
    ) -> std::result::Result<Self, String> {
        let (model, tokenizer) =
            load_llm(model, device, precision, max_seq).map_err(|e| e.to_string())?;
        Ok(Self { model, tokenizer })
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

// ── Executor ──────────────────────────────────────────────────────────────

/// Executor ноды: пишет текущий ответ в порт `answer`. Реальная генерация
/// запускается из `start()` (Per-node Play / глобальный Run).
pub struct LlmExec;

impl NodeExecutor for LlmExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _prompt = ctx.read_input("prompt");
        let _system = ctx.read_input("system");
        let track = ctx.track;

        let text = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Llm { output_text, .. } => {
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
        ctx.write_output("answer", PortValue::Text(text));
    }
}

// ── Hooks для run_controls ────────────────────────────────────────────────

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Llm { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

// ── Запуск генерации ───────────────────────────────────────────────────────

/// Запустить генерацию ноды. Идемпотентно: если worker уже работает — no-op.
/// При ошибках (нет модели / нет вопроса) выставляет `error` и возвращается.
pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Llm {
                system_prompt,
                max_tokens,
                temperature,
                seed,
                pipeline,
                loaded_cfg,
                running,
                error,
                loaded_name,
                output_text,
                text_version,
                cancel,
                ..
            } => Some((
                *system_prompt,
                *max_tokens,
                *temperature,
                *seed,
                pipeline.clone(),
                loaded_cfg.clone(),
                *running,
                *error,
                *loaded_name,
                *output_text,
                *text_version,
                cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        system_prompt,
        max_tokens,
        temperature,
        seed,
        pipeline,
        loaded_cfg,
        running,
        error_sig,
        loaded_name,
        output_text,
        text_version,
        cancel,
    )) = snapshot
    else {
        return;
    };

    if running.get_untracked() {
        return;
    }

    // Доп. параметры (context/think/sampling) — отдельным локом, чтобы не
    // раздувать кортеж снимка.
    let (context, think, top_k, top_p, min_p, repetition_penalty) =
        match node.runtime.lock() {
            Ok(g) => match &*g {
                NodeRuntime::Llm {
                    context,
                    think,
                    top_k,
                    top_p,
                    min_p,
                    repetition_penalty,
                    ..
                } => (
                    context.get_untracked(),
                    think.get_untracked(),
                    top_k.get_untracked(),
                    top_p.get_untracked(),
                    min_p.get_untracked(),
                    repetition_penalty.get_untracked(),
                ),
                _ => return,
            },
            Err(_) => return,
        };

    // Модель — только из Syn-чекпойнта на входе `model`; оттуда же
    // предпочтения device/storage/compute и резидентность.
    let Some(h) = super::current_input_syn_model(ctx, node.id) else {
        error_sig.set(Some(tr!("nodes.common.err.connect_checkpoint")));
        return;
    };
    let resident = h.resident;
    let cfg = LlmLoadedCfg {
        model_path: h.model_path.clone(),
        device_idx: map_handle_device(h.device_idx),
        quant_idx: map_handle_quant(h.storage_idx),
        compute_idx: map_handle_compute(h.compute_idx),
    };

    let question = match current_input_text(ctx, node.id, "prompt") {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            error_sig.set(Some(tr!("node.llm.err.connect_prompt")));
            return;
        }
    };

    // system: приоритет порта, иначе fallback-поле.
    let system = current_input_text(ctx, node.id, "system")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let t = system_prompt.get_untracked();
            (!t.trim().is_empty()).then_some(t)
        });
    let gen = GenParams {
        context,
        think,
        max_new_tokens: max_tokens.get_untracked() as usize,
        temperature: temperature.get_untracked(),
        top_k,
        top_p,
        min_p,
        repetition_penalty,
        seed: seed.get_untracked(),
    };

    cancel.store(false, Ordering::Relaxed);
    running.set(true);
    error_sig.set(None);
    output_text.set(String::new());

    let _ = thread::Builder::new()
        .name("synthos-llm-worker".into())
        .spawn(move || {
            gen_worker(
                pipeline,
                loaded_cfg,
                cfg,
                system,
                question,
                gen,
                cancel,
                running,
                error_sig,
                loaded_name,
                output_text,
                text_version,
                resident,
            );
        });
}

/// Маппинг предпочтений Syn Checkpoint на индексы опций этого семейства.
/// Prefs: device 0=Auto,1=CUDA,2=CPU; storage 0=Auto,1=F16,2=BF16,3=FP8,
/// 4=NVFP4; compute 0=Auto,1=F16,2=BF16,3=F32. Auto → дефолт семейства.
fn map_handle_device(pref: usize) -> usize {
    match pref {
        1 => 0, // CUDA
        2 => 1, // CPU
        _ => default_device_idx(),
    }
}

fn map_handle_quant(pref: usize) -> usize {
    // QUANT_OPTIONS: ["none", "nvfp4", "mxfp8"].
    match pref {
        1 | 2 => 0, // F16/BF16 → без квантизации
        3 => 2,     // FP8 → mxfp8
        4 => 1,     // NVFP4
        _ => default_quant_idx(),
    }
}

fn map_handle_compute(pref: usize) -> usize {
    // COMPUTE_OPTIONS: ["bf16", "f16", "f32"].
    match pref {
        1 => 1, // F16
        2 => 0, // BF16
        3 => 2, // F32
        _ => default_compute_idx(),
    }
}

struct GenParams {
    context: u32,
    think: bool,
    max_new_tokens: usize,
    temperature: f32,
    top_k: u32,
    top_p: f32,
    min_p: f32,
    repetition_penalty: f32,
    seed: u64,
}

// ── Worker ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn gen_worker(
    pipeline: Arc<Mutex<Option<LlmPipeline>>>,
    loaded_cfg: Arc<Mutex<Option<LlmLoadedCfg>>>,
    cfg: LlmLoadedCfg,
    system: Option<String>,
    question: String,
    gen: GenParams,
    cancel: Arc<AtomicBool>,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_text: RwSignal<String>,
    text_version: RwSignal<u32>,
    resident: bool,
) {
    ensure_kernels_registered();

    // 1. Lazy-load pipeline'а при несовпадении snapshot'а.
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
        let precision = match precision_from_idx(cfg.quant_idx, cfg.compute_idx) {
            Ok(p) => p,
            Err(e) => {
                error_sig.set(Some(tr!("node.llm.err.invalid_precision", error = e)));
                running.set(false);
                return;
            }
        };
        let device = device_from_idx(cfg.device_idx);
        let max_seq = Some(gen.context.max(1) as usize);
        let vram_before = crate::models::cuda_allocated();
        match LlmPipeline::load(&cfg.model_path, device, precision, max_seq) {
            Ok(p) => {
                let bytes = crate::models::cuda_allocated().saturating_sub(vram_before);
                let name = cfg
                    .model_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cfg.model_path.display().to_string());
                if let Ok(mut g) = pipeline.lock() {
                    *g = Some(p);
                }
                if let Ok(mut g) = loaded_cfg.lock() {
                    *g = Some(cfg.clone());
                }
                loaded_name.set(Some(name.clone()));
                // Ключ по адресу слота: id ноды сюда не доезжает, а слот
                // у каждой ноды свой и живёт столько же, сколько нода.
                let cfg_slot = loaded_cfg.clone();
                crate::models::register_slot(
                    format!("llm/{:p}", Arc::as_ptr(&pipeline)),
                    "LLM",
                    "Pipeline",
                    name,
                    device,
                    bytes,
                    pipeline.clone(),
                    move || {
                        if let Ok(mut g) = cfg_slot.lock() {
                            *g = None;
                        }
                        loaded_name.set(None);
                    },
                );
            }
            Err(e) => {
                error_sig.set(Some(tr!("nodes.common.model_load_failed", error = e)));
                running.set(false);
                return;
            }
        }
    }

    // 2. Генерация (sync, lock держим на всё время инференса). Sink декодит
    //    накопленные токены и пишет в output_text по мере стрима; cancel →
    //    false прерывает генерацию.
    let result: std::result::Result<(), String> = match pipeline.lock() {
        Ok(g) => match &*g {
            Some(pl) => {
                // Промпт через фасадный токенайзер: jinja-шаблон чата (+ ChatML-
                // fallback и enable_thinking) живут в synaptix::facade::llm.
                let mut msgs: Vec<Message> = Vec::new();
                if let Some(s) = system.as_deref() {
                    if !s.trim().is_empty() {
                        msgs.push(Message::system(s));
                    }
                }
                msgs.push(Message::user(question.as_str()));
                let prompt = match pl
                    .tokenizer
                    .apply_chat_template_ex_tools(&msgs, true, gen.think, None)
                {
                    Ok(p) => p,
                    Err(e) => {
                        drop(g);
                        error_sig.set(Some(tr!("node.llm.err.chat_template", error = e)));
                        running.set(false);
                        return;
                    }
                };
                let prompt_ids = match pl.tokenizer.encode(&prompt) {
                    Ok(ids) => ids,
                    Err(e) => {
                        drop(g);
                        error_sig.set(Some(tr!("node.llm.err.tokenization", error = e)));
                        running.set(false);
                        return;
                    }
                };
                let opts = GenerationOptions {
                    max_new_tokens: gen.max_new_tokens,
                    max_seq_len: gen.context.max(1) as usize,
                    temperature: gen.temperature,
                    top_k: gen.top_k as usize,
                    top_p: gen.top_p,
                    min_p: gen.min_p,
                    seed: gen.seed,
                    repeat_penalty: gen.repetition_penalty,
                    repeat_last_n: 64,
                    presence_penalty: 0.0,
                    frequency_penalty: 0.0,
                };
                fit_residency(&pl.model, prompt_ids.len() + gen.max_new_tokens as usize);
                let mut runner = LlmGeneration::new(&pl.model, opts);
                runner.set_stop_tokens(pl.tokenizer.eos_ids().to_vec());

                let mut acc = String::new();
                let r = runner.generate_streaming(&prompt_ids, &pl.tokenizer, |_id, delta| {
                    if cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    acc.push_str(delta);
                    output_text.set(acc.clone());
                    true
                });
                r.map(|_| ()).map_err(|e| e.to_string())
            }
            None => Err(tr!("nodes.common.pipeline_not_loaded")),
        },
        Err(_) => Err("Lock error pipeline".into()),
    };

    // 3. Финал: фиксируем результат + бамп version для Reactive-статуса.
    match result {
        Ok(()) => {
            error_sig.set(None);
            syngui::prelude::run_on_main_thread(move || {
                text_version.update(|v| *v = v.wrapping_add(1));
            });
        }
        Err(e) => {
            if cancel.load(Ordering::Relaxed) {
                error_sig.set(None);
            } else {
                tracing::warn!(target: super::WORKER_LOG, "LLM: генерация упала: {e}");
                error_sig.set(Some(tr!("node.llm.err.generation_failed", error = e)));
            }
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

// ── Body builder ──────────────────────────────────────────────────────────

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let snapshot = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Llm {
                system_prompt,
                max_tokens,
                temperature,
                seed,
                running,
                error,
                loaded_name,
                cancel,
                ..
            } => Some((
                *system_prompt,
                *max_tokens,
                *temperature,
                *seed,
                *running,
                *error,
                *loaded_name,
                cancel.clone(),
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((
        system_prompt,
        max_tokens,
        temperature,
        seed,
        running,
        error_sig,
        loaded_name,
        cancel,
    )) = snapshot
    else {
        return error_widget(tr!("nodes.common.invalid_runtime", name = "Llm"));
    };

    // Доп. параметры (context/think/sampling) — отдельным локом.
    let (context, think, top_k, top_p, min_p, repetition_penalty) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Llm {
                context,
                think,
                top_k,
                top_p,
                min_p,
                repetition_penalty,
                ..
            } => (*context, *think, *top_k, *top_p, *min_p, *repetition_penalty),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "Llm")),
        },
        Err(_) => return error_widget("Llm: lock error"),
    };

    let system_field = Box::new(
        TextField::new()
            .text(system_prompt.get_untracked())
            .placeholder(tr!("node.llm.system_placeholder"))
            .on_change(move |s| system_prompt.set(s.to_string()))
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    // Контекст (max_seq) — textfield: значения крупные (4096/8192/32768),
    // парсим u32, некорректный ввод игнорируется.
    let context_widget = Box::new(
        TextField::new()
            .text(context.get_untracked().to_string())
            .placeholder("4096")
            .on_change(move |s| {
                if let Ok(v) = s.parse::<u32>() {
                    context.set(v);
                }
            })
            .class("node-input-text"),
    ) as Box<dyn Widget>;

    // Think-режим (Qwen3 <think>): on → enable_thinking в чат-шаблоне.
    let think_toggle = Box::new(
        Toggle::with_state(think.get_untracked())
            .on_change(move |v| think.set(v))
            .class("node-input-toggle"),
    ) as Box<dyn Widget>;

    let max_tokens_control = node_int_slider_field(max_tokens, 16, 8192, 16);
    let temperature_control = node_slider_field(temperature, 0.0, 2.0, 0.05, 2);
    let top_k_control = node_int_slider_field(top_k, 0, 200, 1);
    let top_p_control = node_slider_field(top_p, 0.0, 1.0, 0.01, 2);
    let min_p_control = node_slider_field(min_p, 0.0, 1.0, 0.01, 2);
    let rep_penalty_control = node_slider_field(repetition_penalty, 1.0, 2.0, 0.01, 2);

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

    // Кнопка отмены — видима только во время генерации; выставляет cancel-флаг
    // (sink вернёт false → generate_streaming прервётся без паники).
    let cancel_for_btn = cancel.clone();
    let cancel_control = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if running.get() {
            let c = cancel_for_btn.clone();
            vec![Box::new(
                Button::new(tr!("app.cancel"))
                    .on_click(move || c.store(true, Ordering::Relaxed))
                    .class("node-input-button llm-node-cancel"),
            ) as Box<dyn Widget>]
        } else {
            vec![Box::new(Text::new("—").class("audio-node-meta")) as Box<dyn Widget>]
        }
    });

    let status_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(tr!("nodes.common.error", error = msg)).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            return vec![Box::new(
                Text::new(tr!("nodes.common.generating")).class("audio-node-meta llm-node-running"),
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
            node_field_row("System", system_field),
            node_field_row("Context", context_widget),
            node_field_row("Think", think_toggle),
            node_field_row("Max tokens", max_tokens_control),
            node_field_row("Temp", temperature_control),
            node_field_row("Top-k", top_k_control),
            node_field_row("Top-p", top_p_control),
            node_field_row("Min-p", min_p_control),
            node_field_row("Rep. penalty", rep_penalty_control),
            node_field_row("Seed", seed_widget),
            node_field_row("", Box::new(cancel_control) as Box<dyn Widget>),
            node_field_row(&tr!("nodes.common.status"), Box::new(status_text) as Box<dyn Widget>),
        ]);

    Box::new(col)
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
}
