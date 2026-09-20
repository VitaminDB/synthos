//! `Yue2Generate` — песня целиком: партитура → музыка → латенты → звук.
//!
//! Контент приходит портами (`style`, `lyrics`, опц. `abc`), модель — портом
//! `model` от Checkpoint-ноды. Выходы: `audio` (48 кГц стерео), `score`
//! (партитура в ABC — её можно посмотреть, поправить и подать назад в `abc`)
//! и `latent` (акустические латенты для повторного декода).
//!
//! Обе AR-фазы идут по одной сессии движка, поэтому текст и партитура
//! префиллятся один раз. Веса берутся из общего кэша ([`super::shared`]) —
//! повторный прогон не платит загрузку.

use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::Column;

use synaptix_core::tensor::Tensor;
use synaptix_music_yue2::ar::Phase;
use synaptix_music_yue2::pipeline::{Callbacks, Yue2Options, Yue2Pipeline};
use synaptix_music_yue2::protocol::{seconds_to_frames, GenerationConfig, SongRequest, CONTEXT};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, NodeInstance, NodeRuntime, PortValue, Yue2Blob, Yue2ModelHandle,
};
use super::shared::{
    device_and_dtypes, keep_resident, load_ar, load_nar, load_tokenizer, load_vae,
    release_resident, resolve_paths,
};
use super::{
    cot_from_idx, current_input_latent, current_input_model, current_input_text, field_row,
    make_dropdown, make_int_slider_row, make_seed_slider, make_slider_row, status_row, COT_OPTIONS,
};

/// Сколько секунд музыки просить по умолчанию. Потолок — окно модели: на
/// кадр уходит и семантический токен, и латентная позиция.
pub const DEFAULT_SECONDS: f32 = 60.0;
/// Максимум, который вообще влезает (25 кадров в секунду).
pub const MAX_SECONDS: f32 = 300.0;

pub struct GenerateExec;

impl NodeExecutor for GenerateExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _style = ctx.read_input("style");
        let _lyrics = ctx.read_input("lyrics");
        let _abc = ctx.read_input("abc");
        let track = ctx.track;
        let (audio_pv, score_pv, latent_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::Yue2Generate {
                    output_buf_audio,
                    output_buf_latent,
                    output_buf_score,
                    output_version,
                    ..
                } => {
                    if track {
                        let _ = output_version.get();
                    }
                    let a = match output_buf_audio.lock() {
                        Ok(b) => b.clone().map(PortValue::Audio).unwrap_or(PortValue::Empty),
                        Err(_) => PortValue::Empty,
                    };
                    let s = match output_buf_score.lock() {
                        Ok(b) => b.clone().map(PortValue::Text).unwrap_or(PortValue::Empty),
                        Err(_) => PortValue::Empty,
                    };
                    let l = match output_buf_latent.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(t) => PortValue::Data(Arc::new(DataBlob::Yue2(Yue2Blob::Latent(
                                t.clone(),
                            )))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    };
                    (a, s, l)
                }
                _ => (PortValue::Empty, PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("audio", audio_pv);
        ctx.write_output("score", score_pv);
        ctx.write_output("latent", latent_pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2Generate { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

struct Snapshot {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    cot_idx: RwSignal<usize>,
    seconds: RwSignal<f32>,
    ode_steps: RwSignal<u32>,
    cfg_scale: RwSignal<f32>,
    seed: RwSignal<u64>,
    temperature: RwSignal<f32>,
    top_p: RwSignal<f32>,
    top_k: RwSignal<u32>,
    repetition_penalty: RwSignal<f32>,
    vae_core_frames: RwSignal<u32>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    progress_pct: RwSignal<f32>,
}

fn snapshot(node: &NodeInstance) -> Option<Snapshot> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::Yue2Generate {
                cot_idx,
                seconds,
                ode_steps,
                cfg_scale,
                seed,
                temperature,
                top_p,
                top_k,
                repetition_penalty,
                vae_core_frames,
                running,
                error,
                loaded_name,
                progress_pct,
                cancel,
                ..
            } => Some(Snapshot {
                cancel: cancel.clone(),
                cot_idx: *cot_idx,
                seconds: *seconds,
                ode_steps: *ode_steps,
                cfg_scale: *cfg_scale,
                seed: *seed,
                temperature: *temperature,
                top_p: *top_p,
                top_k: *top_k,
                repetition_penalty: *repetition_penalty,
                vae_core_frames: *vae_core_frames,
                running: *running,
                error: *error,
                loaded_name: *loaded_name,
                progress_pct: *progress_pct,
            }),
            _ => None,
        },
        Err(_) => None,
    }
}

struct RunParams {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    handle: Arc<Yue2ModelHandle>,
    request: SongRequest,
    generation: GenerationConfig,
    vae_core_frames: usize,
}

/// `0` в поле сида — «каждый прогон новый»; иначе повторяемый прогон.
fn resolve_seed(v: u64) -> u64 {
    if v != 0 {
        return v;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64 & 0x7FFF_FFFF)
        .unwrap_or(1)
        .max(1)
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(s) = snapshot(node) else { return };
    if s.running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        s.error.set(Some(tr!("node.yue2_generate.error.no_checkpoint")));
        return;
    };
    let style = current_input_text(ctx, node.id, "style").unwrap_or_default();
    let lyrics = current_input_text(ctx, node.id, "lyrics").unwrap_or_default();
    let abc = current_input_text(ctx, node.id, "abc").filter(|t| !t.trim().is_empty());
    if style.trim().is_empty() {
        s.error.set(Some(tr!("node.yue2_generate.error.no_style")));
        return;
    }

    let cot = cot_from_idx(s.cot_idx.get_untracked());
    let cfg = s.cfg_scale.get_untracked();
    let request = SongRequest {
        style,
        lyrics,
        cot,
        seed: resolve_seed(s.seed.get_untracked()),
        // Партитура из порта сильнее сгенерированной; в режиме `off` её не бывает.
        abc: if cot == synaptix_music_yue2::protocol::Cot::Off { None } else { abc },
        // 0 на слайдере — дефолт режима.
        cfg_scale: if cfg <= 0.0 { None } else { Some(cfg) },
    };
    if let Err(e) = request.validate() {
        s.error.set(Some(e.to_string()));
        return;
    }

    let mut generation = GenerationConfig::default();
    generation.ode_steps = s.ode_steps.get_untracked().max(1) as usize;
    let want = s.seconds.get_untracked();
    if want > 0.0 {
        // Кадров на песню: они же считаются и как семантические токены.
        let frames = seconds_to_frames(want).clamp(1, CONTEXT - 2);
        generation.semantic.max_tokens = frames;
        generation.semantic.min_tokens = generation.semantic.min_tokens.min(frames);
    }
    generation.semantic.temperature = s.temperature.get_untracked();
    generation.semantic.top_p = s.top_p.get_untracked();
    generation.semantic.top_k = s.top_k.get_untracked().max(1) as usize;
    generation.semantic.repetition_penalty = s.repetition_penalty.get_untracked();
    if let Err(e) = generation.semantic.validate() {
        s.error.set(Some(e.to_string()));
        return;
    }

    let params = RunParams {
        cancel: s.cancel.clone(),
        handle,
        request,
        generation,
        vae_core_frames: s.vae_core_frames.get_untracked().max(16) as usize,
    };

    // Флаг прошлого прогона не должен мгновенно останавливать новый.
    s.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    s.running.set(true);
    s.error.set(None);
    s.progress_pct.set(0.0);

    let (output_buf_audio, output_buf_latent, output_buf_score, output_version) =
        match node.runtime.lock() {
            Ok(g) => match &*g {
                NodeRuntime::Yue2Generate {
                    output_buf_audio,
                    output_buf_latent,
                    output_buf_score,
                    output_version,
                    ..
                } => (
                    output_buf_audio.clone(),
                    output_buf_latent.clone(),
                    output_buf_score.clone(),
                    *output_version,
                ),
                _ => return,
            },
            Err(_) => return,
        };
    let (running, error, loaded_name, progress) =
        (s.running, s.error, s.loaded_name, s.progress_pct);

    let _ = thread::Builder::new()
        .name("synthos-yue2-generate".into())
        .spawn(move || {
            let outputs = Outputs {
                audio: output_buf_audio,
                latent: output_buf_latent,
                score: output_buf_score,
                version: output_version,
            };
            worker(params, running, error, loaded_name, progress, outputs);
        });
}

struct Outputs {
    audio: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    latent: Arc<Mutex<Option<Tensor>>>,
    score: Arc<Mutex<Option<String>>>,
    version: RwSignal<u32>,
}

fn worker(
    p: RunParams,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    progress: RwSignal<f32>,
    out: Outputs,
) {
    let finish_err = |msg: String| {
        error.set(Some(msg));
        running.set(false);
    };
    let (model_path, vae_path) = match resolve_paths(&p.handle) {
        Ok(t) => t,
        Err(e) => return finish_err(e),
    };
    let (device, compute, quant, vae_dtype) = device_and_dtypes(&p.handle);
    let h = &p.handle;
    let ar = match load_ar(&model_path, h.device_idx, h.quant_idx, h.compute_idx) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    let nar = match load_nar(&model_path, h.device_idx, h.quant_idx, h.compute_idx) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    let tokenizer = match load_tokenizer(&model_path) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    let name = model_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.display().to_string());
    loaded_name.set(Some(tr!("node.yue2_generate.status.planning")));

    let options = Yue2Options {
        device,
        compute,
        quant,
        vae_dtype,
        vae_core_frames: p.vae_core_frames,
        vae_halo_frames: 16,
        generation: p.generation.clone(),
    };
    let pipe = Yue2Pipeline::from_parts(ar, nar, tokenizer, None, options);

    // Прогресс: партитура и музыка считаются по токенам от своих лимитов,
    // акустика — по шагам решателя, декод — по тайлам. Внутри одного прогона
    // доли фаз фиксированы, чтобы полоса не прыгала назад.
    let abc_budget = p.generation.abc.max_tokens.max(1) as f32;
    let sem_budget = p.generation.semantic.max_tokens.max(1) as f32;
    let on_token = {
        let progress = progress;
        move |phase: Phase, _token: u32, step: usize| match phase {
            Phase::Abc => progress.set(10.0 * (step as f32 / abc_budget).min(1.0)),
            Phase::Semantic => progress.set(10.0 + 45.0 * (step as f32 / sem_budget).min(1.0)),
        }
    };
    let on_ode = {
        let progress = progress;
        move |done: usize, total: usize| {
            let part = if total == 0 { 0.0 } else { done as f32 / total as f32 };
            progress.set(55.0 + 35.0 * part.min(1.0));
        }
    };
    let on_vae = {
        let progress = progress;
        move |done: usize, total: usize| {
            let part = if total == 0 { 0.0 } else { done as f32 / total as f32 };
            progress.set(90.0 + 10.0 * part.min(1.0));
        }
    };
    let cancel_flag = p.cancel.clone();
    let cancelled = move || cancel_flag.load(std::sync::atomic::Ordering::Relaxed);
    let cb = Callbacks {
        cancel: Some(&cancelled),
        on_token: Some(&on_token),
        on_ode: Some(&on_ode),
        on_vae: Some(&on_vae),
    };

    let started = std::time::Instant::now();
    let semantic = match pipe.generate(&p.request, cb) {
        Ok(v) => v,
        Err(e) => return finish_err(format!("{e}")),
    };
    if let (Ok(mut g), Some(abc)) = (out.score.lock(), semantic.plan.abc.clone()) {
        *g = Some(abc);
    }
    loaded_name.set(Some(tr!(
        "node.yue2_generate.status.synthesizing",
        seconds = format!("{:.0}", semantic.seconds())
    )));
    let latents = match pipe.synthesize(&semantic, cb) {
        Ok(v) => v,
        Err(e) => return finish_err(format!("{e}")),
    };
    // Декодер грузится последним: во время генерации его вес в памяти не нужен.
    let vae = match load_vae(&vae_path, h.device_idx, h.vae_dtype_idx) {
        Ok(v) => v,
        Err(e) => return finish_err(e),
    };
    let audio = match pipe.decode_with(&vae, &latents, cb) {
        Ok(v) => v,
        Err(e) => return finish_err(format!("{e}")),
    };

    if p.handle.resident {
        // Кэш держит модели через `Weak`: без сильных ссылок они выгрузились бы
        // сразу после воркера, и чекбокс ничего бы не значил.
        keep_resident(
            &model_path,
            &vae_path,
            &p.handle,
            pipe.ar.clone(),
            pipe.nar.clone(),
            Some(vae.clone()),
        );
    } else {
        if release_resident() {
            tracing::info!("[yue2] резидентные веса освобождены (чекбокс выключен)");
        }
        // Модели уже отпущены, но пул CUDA держит их блоки — без trim процесс
        // так и сидит на своих гигабайтах.
        drop(vae);
        drop(pipe);
        crate::models::trim_all();
    }
    crate::models::changed();

    let seconds = audio.len() as f32 / (2.0 * 48000.0);
    tracing::info!(
        "[yue2] Generate ✓ {seconds:.1} с музыки за {:.1} с (партитура {} ток., музыка {} ток., шагов {}, seed {})",
        started.elapsed().as_secs_f32(),
        semantic.plan.abc_ids.len(),
        semantic.tokens.len(),
        p.generation.ode_steps,
        p.request.seed
    );
    loaded_name.set(Some(tr!(
        "node.yue2_generate.status.done",
        seconds = format!("{seconds:.0}"),
        name = name,
        seed = p.request.seed.to_string()
    )));

    if let Ok(mut g) = out.audio.lock() {
        *g = Some(Arc::new(AudioBuffer::new(
            Arc::from(audio.into_boxed_slice()),
            48_000,
            2,
        )));
    }
    if let Ok(mut g) = out.latent.lock() {
        *g = Some(latents);
    }
    let version = out.version;
    syngui::prelude::run_on_main_thread(move || {
        version.update(|v| *v = v.wrapping_add(1));
    });
    progress.set(100.0);
    error.set(None);
    running.set(false);
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some(s) = snapshot(node) else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "Yue2Generate"))
                .class("node-card-field-error"),
        );
    };
    let rows: Vec<Box<dyn Widget>> = vec![
        field_row(&tr!("node.yue2_generate.field.cot"), make_dropdown(COT_OPTIONS, s.cot_idx)),
        field_row(
            &tr!("node.yue2_generate.field.seconds"),
            make_slider_row(s.seconds, 0.0, MAX_SECONDS, 5.0, 0),
        ),
        field_row(
            &tr!("node.yue2_generate.field.ode_steps"),
            make_int_slider_row(s.ode_steps, 1, 64, 1),
        ),
        field_row(&tr!("node.yue2_generate.field.cfg"), make_slider_row(s.cfg_scale, 0.0, 5.0, 0.01, 2)),
        field_row("Seed (0 = random)", make_seed_slider(s.seed)),
        field_row("Temperature", make_slider_row(s.temperature, 0.0, 2.0, 0.01, 2)),
        field_row("Top-p", make_slider_row(s.top_p, 0.05, 1.0, 0.01, 2)),
        field_row("Top-k", make_int_slider_row(s.top_k, 1, 500, 1)),
        field_row(
            &tr!("node.yue2_generate.field.repetition_penalty"),
            make_slider_row(s.repetition_penalty, 0.5, 2.0, 0.005, 3),
        ),
        field_row(
            &tr!("node.yue2_generate.field.vae_core_frames"),
            make_int_slider_row(s.vae_core_frames, 64, 4096, 64),
        ),
        field_row(
            &tr!("nodes.common.status"),
            status_row(
                s.running,
                s.error,
                s.loaded_name,
                tr!("node.yue2_generate.status.busy"),
                "acestep-node-running",
            ),
        ),
        // Кнопка появляется только во время прогона.
        field_row(&tr!("app.cancel"), super::super::ltx::cancel_button(s.running, s.cancel.clone())),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

/// Латенты с входа `latent` — ими Generate не пользуется, но порт читают
/// downstream-ноды; функция нужна для симметрии с `vae_decode`.
#[allow(dead_code)]
fn _unused_latent(ctx: &NodeEditorCtx, id: super::super::super::types::NodeId) -> Option<Tensor> {
    current_input_latent(ctx, id, "latent")
}
