//! `AceStepGenerate` — монолитная text→music нода ACE-Step.
//!
//! Весь пайплайн (LM → text/lyric/timbre-enc → FSQ/detok/cond → DiT →
//! VAE) в одном worker'е через [`generate_music`] (sequential-drop под
//! 24GB). Контент подаётся ВХОДНЫМИ портами (`tags`/`lyrics`/`src_latent`),
//! модель — портом `model` от Checkpoint-ноды; в body только режим-дропдаун
//! и числовые слайдеры. Выходы: `audio` (готовый PCM) + `latent` (для
//! цепочки retake).
//!
//! Режимы (дропдаун `mode`): text2music работает; retake/repaint/extend/edit
//! требуют доработки synaptix-пайплайна (`denoise`/`SamplerOptions` пока без
//! `src_latent`/маски) — реализуются пофазно, до тех пор дают понятную ошибку.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Column, Dropdown, DropdownItem, Reactive};

use synaptix_core::dtype::DType;
use synaptix_core::tensor::Tensor;
use synaptix_music_acestep::ar::CodesGenOptions;
use synaptix_music_acestep::dcw::DcwCorrector;
use synaptix_music_acestep::pipeline::{
    generate_music, EditMode, EditOptions, GenExtras, MusicComponentCache, MusicPaths, MusicStage,
    NormMode, SamplerOptions, StageHook,
};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    AceStepBlob, AceStepModelHandle, DataBlob, DcwModeOption, DcwPresetOption, DcwWaveletOption,
    NodeInstance, NodeRuntime, PortValue, PostNormMode, SamplerPreset,
};
use super::shared::{detect_xl_bundle_kind, XlBundleKind};
use super::{
    compute_from_idx, current_input_latent, current_input_model, current_input_text,
    device_from_idx, field_row, make_int_slider_row, make_seed_slider, make_slider_row,
    make_toggle, quant_from_idx, status_row,
};

/// Слот резидентного кэша компонентов ACE-Step — один на приложение, в
/// формате `models::register_slot` (панель «Модели в памяти» видит его и
/// умеет выгружать). Сам кэш валидирует пути/девайс/кванты по ключу.
fn resident_cache() -> &'static Arc<Mutex<Option<MusicComponentCache>>> {
    static CACHE: std::sync::OnceLock<Arc<Mutex<Option<MusicComponentCache>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// Дефолтные имена 4 бандлов в каталоге моделей (зеркалит CLI `music`).
/// Для LM и DiT — список кандидатов по убыванию приоритета: берётся первый
/// существующий в каталоге (4b → 1.7b; base → turbo), чтобы каталог только с
/// 1.7b-LM или только с turbo-DiT работал без override'ов. Дефолт связки —
/// 4b-LM + xl_base (полное качество); turbo/1.7b выбираются override-полями
/// Checkpoint-ноды (в т. ч. агентом через `pipelines apply set_state`).
pub const LM_NAMES: &[&str] = &["acestep_5hz_lm_4b.syn", "acestep_5hz_lm_1.7b.syn"];
pub const TEXT_ENC_NAMES: &[&str] = &["qwen3-embedding-0.6b.syn"];
pub const DIT_NAMES: &[&str] = &["acestep_v15_xl_base.syn", "acestep_v15_xl_turbo.syn"];
pub const VAE_NAMES: &[&str] = &["acestep_vae.syn"];

pub const MODE_OPTIONS: &[&str] =
    &["text2music", "retake", "repaint", "extend", "edit", "cover", "extract"];

/// Тональность-оверрайд для AR/DiT-метаданных. Индекс `0` = N/A (модель сама
/// предсказывает в Phase-1 CoT); остальные → `Some("<key> <scale>")` как ждёт
/// CLI `--keyscale`. 12 нот × {major, minor}.
pub const KEYSCALE_OPTIONS: &[&str] = &[
    "N/A",
    "C major", "C minor", "C# major", "C# minor", "D major", "D minor",
    "D# major", "D# minor", "E major", "E minor", "F major", "F minor",
    "F# major", "F# minor", "G major", "G minor", "G# major", "G# minor",
    "A major", "A minor", "A# major", "A# minor", "B major", "B minor",
];

/// Размер такта-оверрайд. Индекс `0` = N/A; остальные → `Some("n/d")` как ждёт
/// CLI `--timesig` (знаменатель сохраняется, ср. фикс timesignature→String).
pub const TIMESIG_OPTIONS: &[&str] =
    &["N/A", "4/4", "3/4", "6/8", "2/4", "5/4", "7/8", "9/8", "12/8"];

/// Дорожка режима extract — стемы, которые base-DiT учился выделять (имена
/// `TRACK_NAMES` ACE-Step, порядок свой: вокал первым, он же дефолт `0`).
pub const TRACK_OPTIONS: &[&str] = &[
    "vocals", "backing_vocals", "drums", "bass", "guitar", "keyboard",
    "percussion", "strings", "synth", "fx", "brass", "woodwinds",
];

/// Индекс дропдауна режима → режим synaptix. Cover идёт своим путём: раньше
/// он делил ветку с extract, а extract теперь без LM и со своей инструкцией.
fn edit_mode(mode_idx: usize) -> EditMode {
    match mode_idx {
        1 => EditMode::Retake,
        2 | 3 => EditMode::Repaint,
        4 => EditMode::Edit,
        5 => EditMode::Cover,
        6 => EditMode::Extract,
        _ => EditMode::Text2Music,
    }
}

fn track_name(track_idx: usize) -> &'static str {
    TRACK_OPTIONS.get(track_idx).copied().unwrap_or(TRACK_OPTIONS[0])
}

/// Имя бандла, которое возьмётся из каталога при пустом override'е: первое
/// существующее из `names`, иначе первое в списке (чтобы ошибка «не найден»
/// называла ожидаемый файл). Общая точка для резолва и для подсказок в UI
/// Checkpoint-ноды.
pub fn default_bundle_name<'a>(dir: Option<&std::path::Path>, names: &'a [&'a str]) -> &'a str {
    dir.and_then(|d| names.iter().find(|n| d.join(n).exists()).copied())
        .unwrap_or(names[0])
}

/// Путь одного бандла из хэндла: override → каталог/первое существующее из
/// дефолтных имён (иначе первое имя — для понятной ошибки «не найден»).
/// Зеркалит CLI `pick`. Существование не проверяет.
fn pick_bundle(
    h: &AceStepModelHandle,
    o: &Option<PathBuf>,
    names: &[&str],
) -> std::result::Result<PathBuf, String> {
    if let Some(p) = o {
        // Голое имя бандла (агент пишет `acestep_5hz_lm_4b.syn` — так их
        // печатает схема ноды) — от каталога моделей, а не от cwd.
        if p.is_relative() {
            if let Some(d) = &h.models_dir {
                return Ok(d.join(p));
            }
        }
        return Ok(p.clone());
    }
    match &h.models_dir {
        Some(d) => Ok(d.join(default_bundle_name(Some(d), names))),
        None => Err(tr!("node.acestep_generate.error.missing_dir_or_override", name = names[0])),
    }
}

fn ensure_exists(label: &str, p: &std::path::Path) -> std::result::Result<(), String> {
    if p.exists() {
        Ok(())
    } else {
        Err(tr!(
            "node.acestep_generate.error.bundle_not_found",
            label = label,
            path = p.display()
        ))
    }
}

/// Резолв 4 путей-бандлов из хэндла; проверяет существование файлов.
pub fn resolve_paths(
    h: &AceStepModelHandle,
) -> std::result::Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let lm = pick_bundle(h, &h.lm_path, LM_NAMES)?;
    let te = pick_bundle(h, &h.text_encoder_path, TEXT_ENC_NAMES)?;
    let dit = pick_bundle(h, &h.dit_path, DIT_NAMES)?;
    let vae = pick_bundle(h, &h.vae_path, VAE_NAMES)?;
    for (label, p) in [("lm", &lm), ("text-encoder", &te), ("dit", &dit), ("vae", &vae)] {
        ensure_exists(label, p)?;
    }
    Ok((lm, te, dit, vae))
}

/// Только VAE-бандл — VAE Encode остальные подмодели не нужны.
pub fn resolve_vae(h: &AceStepModelHandle) -> std::result::Result<PathBuf, String> {
    let vae = pick_bundle(h, &h.vae_path, VAE_NAMES)?;
    ensure_exists("vae", &vae)?;
    Ok(vae)
}

pub struct GenerateExec;

impl NodeExecutor for GenerateExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _model = ctx.read_input("model");
        let _tags = ctx.read_input("tags");
        let _lyrics = ctx.read_input("lyrics");
        let _src_latent = ctx.read_input("src_latent");
        let track = ctx.track;
        let (audio_pv, latent_pv) = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AceStepGenerate {
                    output_buf_audio,
                    output_buf_latent,
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
                    let l = match output_buf_latent.lock() {
                        Ok(b) => match b.as_ref() {
                            Some(t) => PortValue::Data(Arc::new(DataBlob::AceStep(
                                AceStepBlob::Latent(t.clone()),
                            ))),
                            None => PortValue::Empty,
                        },
                        Err(_) => PortValue::Empty,
                    };
                    (a, l)
                }
                _ => (PortValue::Empty, PortValue::Empty),
            },
            Err(_) => (PortValue::Empty, PortValue::Empty),
        };
        ctx.write_output("audio", audio_pv);
        ctx.write_output("latent", latent_pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    start(node, ctx);
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepGenerate { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

// ── UI ──────────────────────────────────────────────────────────────────

fn make_mode_dropdown(mode_idx: RwSignal<usize>) -> Box<dyn Widget> {
    super::make_dropdown(MODE_OPTIONS, mode_idx)
}

fn make_preset_dropdown(
    preset: RwSignal<SamplerPreset>,
    infer_steps: RwSignal<u32>,
    cfg_scale: RwSignal<f32>,
    flow_match_shift: RwSignal<f32>,
) -> Box<dyn Widget> {
    const OPTIONS: &[(&str, SamplerPreset)] = &[
        ("Auto", SamplerPreset::Auto),
        ("Turbo", SamplerPreset::Turbo),
        ("Base", SamplerPreset::Base),
        ("SFT", SamplerPreset::Sft),
    ];
    let items: Vec<DropdownItem> = OPTIONS.iter().map(|(l, _)| DropdownItem::simple(*l)).collect();
    let current = OPTIONS
        .iter()
        .find(|(_, p)| *p == preset.get_untracked())
        .map(|(l, _)| (*l).to_string())
        .unwrap_or_else(|| "Auto".into());
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let Some((_, new_p)) = OPTIONS.iter().find(|(l, _)| *l == s) else {
                    return;
                };
                preset.set(*new_p);
                let kind = match new_p {
                    SamplerPreset::Turbo => Some(XlBundleKind::Turbo),
                    SamplerPreset::Base => Some(XlBundleKind::Base),
                    SamplerPreset::Sft => Some(XlBundleKind::Sft),
                    SamplerPreset::Auto => None,
                };
                if let Some(k) = kind {
                    infer_steps.set(k.default_infer_steps());
                    cfg_scale.set(k.default_cfg_scale());
                    flow_match_shift.set(k.default_flow_match_shift());
                }
            })
            .class("node-input-dropdown"),
    )
}

fn make_dcw_mode_dropdown(mode: RwSignal<DcwModeOption>) -> Box<dyn Widget> {
    const OPTIONS: &[(&str, DcwModeOption)] = &[
        ("double", DcwModeOption::Double),
        ("low", DcwModeOption::Low),
        ("high", DcwModeOption::High),
        ("pix", DcwModeOption::Pix),
    ];
    let items: Vec<DropdownItem> = OPTIONS.iter().map(|(l, _)| DropdownItem::simple(*l)).collect();
    let current = OPTIONS
        .iter()
        .find(|(_, m)| *m == mode.get_untracked())
        .map(|(l, _)| (*l).to_string())
        .unwrap_or_else(|| "double".into());
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                if let Some((_, m)) = OPTIONS.iter().find(|(l, _)| *l == s) {
                    mode.set(*m);
                }
            })
            .class("node-input-dropdown"),
    )
}

fn make_dcw_wavelet_dropdown(wavelet: RwSignal<DcwWaveletOption>) -> Box<dyn Widget> {
    const OPTIONS: &[(&str, DcwWaveletOption)] = &[
        ("haar", DcwWaveletOption::Haar),
        ("db4 (UI only)", DcwWaveletOption::Db4),
        ("sym8 (UI only)", DcwWaveletOption::Sym8),
    ];
    let items: Vec<DropdownItem> = OPTIONS.iter().map(|(l, _)| DropdownItem::simple(*l)).collect();
    let current = OPTIONS
        .iter()
        .find(|(_, w)| *w == wavelet.get_untracked())
        .map(|(l, _)| (*l).to_string())
        .unwrap_or_else(|| "haar".into());
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                if let Some((_, w)) = OPTIONS.iter().find(|(l, _)| *l == s) {
                    wavelet.set(*w);
                }
            })
            .class("node-input-dropdown"),
    )
}

fn make_dcw_preset_dropdown(
    preset: RwSignal<DcwPresetOption>,
    dcw_scaler: RwSignal<f32>,
    dcw_high_scaler: RwSignal<f32>,
) -> Box<dyn Widget> {
    const OPTIONS: &[(&str, DcwPresetOption)] = &[
        ("Think", DcwPresetOption::Think),
        ("No Think", DcwPresetOption::NoThink),
        ("Custom", DcwPresetOption::Custom),
    ];
    let items: Vec<DropdownItem> = OPTIONS.iter().map(|(l, _)| DropdownItem::simple(*l)).collect();
    let current = OPTIONS
        .iter()
        .find(|(_, p)| *p == preset.get_untracked())
        .map(|(l, _)| (*l).to_string())
        .unwrap_or_else(|| "Custom".into());
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let Some((_, new_p)) = OPTIONS.iter().find(|(l, _)| *l == s) else {
                    return;
                };
                preset.set(*new_p);
                if let Some((lo, hi)) = new_p.scalers() {
                    dcw_scaler.set(lo);
                    dcw_high_scaler.set(hi);
                }
            })
            .class("node-input-dropdown"),
    )
}

/// Dropdown нормализации выходного PCM — общий synthos `PostNormMode`
/// (None/RMS/Peak), как в output-ноде.
fn make_norm_dropdown(norm: RwSignal<PostNormMode>) -> Box<dyn Widget> {
    let items: Vec<DropdownItem> =
        PostNormMode::ALL.iter().map(|m| DropdownItem::simple(m.label())).collect();
    let current = norm.get_untracked().label().to_string();
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                if let Some(m) = PostNormMode::from_label(s) {
                    norm.set(m);
                }
            })
            .class("node-input-dropdown"),
    )
}

#[allow(clippy::type_complexity)]
struct BodySnapshot {
    mode_idx: RwSignal<usize>,
    preset: RwSignal<SamplerPreset>,
    duration_seconds: RwSignal<f32>,
    infer_steps: RwSignal<u32>,
    cfg_scale: RwSignal<f32>,
    flow_match_shift: RwSignal<f32>,
    seed: RwSignal<u64>,
    temperature: RwSignal<f32>,
    top_p: RwSignal<f32>,
    top_k: RwSignal<u32>,
    min_p: RwSignal<f32>,
    lm_cfg_scale: RwSignal<f32>,
    use_cot: RwSignal<bool>,
    use_ar: RwSignal<bool>,
    bpm: RwSignal<u32>,
    keyscale_idx: RwSignal<usize>,
    timesig_idx: RwSignal<usize>,
    norm_mode: RwSignal<PostNormMode>,
    enable_dcw: RwSignal<bool>,
    dcw_mode: RwSignal<DcwModeOption>,
    dcw_scaler: RwSignal<f32>,
    dcw_high_scaler: RwSignal<f32>,
    dcw_wavelet: RwSignal<DcwWaveletOption>,
    dcw_preset: RwSignal<DcwPresetOption>,
    retake_variance: RwSignal<f32>,
    retake_seed: RwSignal<u64>,
    repaint_start_sec: RwSignal<f32>,
    repaint_end_sec: RwSignal<f32>,
    repaint_strength: RwSignal<f32>,
    edit_n_min: RwSignal<f32>,
    edit_n_max: RwSignal<f32>,
    track_idx: RwSignal<usize>,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
}

fn snapshot(node: &NodeInstance) -> Option<BodySnapshot> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepGenerate {
                mode_idx,
                preset,
                duration_seconds,
                infer_steps,
                cfg_scale,
                flow_match_shift,
                seed,
                temperature,
                top_p,
                top_k,
                min_p,
                lm_cfg_scale,
                use_cot,
                use_ar,
                bpm,
                keyscale_idx,
                timesig_idx,
                norm_mode,
                enable_dcw,
                dcw_mode,
                dcw_scaler,
                dcw_high_scaler,
                dcw_wavelet,
                dcw_preset,
                retake_variance,
                retake_seed,
                repaint_start_sec,
                repaint_end_sec,
                repaint_strength,
                edit_n_min,
                edit_n_max,
                track_idx,
                running,
                error,
                loaded_name,
                ..
            } => Some(BodySnapshot {
                mode_idx: *mode_idx,
                preset: *preset,
                duration_seconds: *duration_seconds,
                infer_steps: *infer_steps,
                cfg_scale: *cfg_scale,
                flow_match_shift: *flow_match_shift,
                seed: *seed,
                temperature: *temperature,
                top_p: *top_p,
                top_k: *top_k,
                min_p: *min_p,
                lm_cfg_scale: *lm_cfg_scale,
                use_cot: *use_cot,
                use_ar: *use_ar,
                bpm: *bpm,
                keyscale_idx: *keyscale_idx,
                timesig_idx: *timesig_idx,
                norm_mode: *norm_mode,
                enable_dcw: *enable_dcw,
                dcw_mode: *dcw_mode,
                dcw_scaler: *dcw_scaler,
                dcw_high_scaler: *dcw_high_scaler,
                dcw_wavelet: *dcw_wavelet,
                dcw_preset: *dcw_preset,
                retake_variance: *retake_variance,
                retake_seed: *retake_seed,
                repaint_start_sec: *repaint_start_sec,
                repaint_end_sec: *repaint_end_sec,
                repaint_strength: *repaint_strength,
                edit_n_min: *edit_n_min,
                edit_n_max: *edit_n_max,
                track_idx: *track_idx,
                running: *running,
                error: *error,
                loaded_name: *loaded_name,
            }),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Заголовок-разделитель секции body'а (AR / DiT / Выход).
fn section_header(label: &str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(super::NODE_PADDING_H, super::ROW_PADDING_V).child(
            Text::new(label).class("node-card-hint acestep-section-header"),
        ),
    )
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some(s) = snapshot(node) else {
        return Box::new(
            Text::new(tr!("nodes.common.invalid_runtime", name = "AceStepGenerate"))
                .class("node-card-field-error"),
        );
    };

    // Режим-зависимые строки: показываются/прячутся по `mode_idx`.
    let mode_idx = s.mode_idx;
    let retake_variance = s.retake_variance;
    let retake_seed = s.retake_seed;
    let repaint_start_sec = s.repaint_start_sec;
    let repaint_end_sec = s.repaint_end_sec;
    let repaint_strength = s.repaint_strength;
    let edit_n_min = s.edit_n_min;
    let edit_n_max = s.edit_n_max;
    let track_idx = s.track_idx;
    let mode_rows = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        match mode_idx.get() {
            1 => vec![
                field_row("Retake variance", make_slider_row(retake_variance, 0.0, 1.0, 0.01, 2)),
                field_row("Retake seed (0 = random)", make_seed_slider(retake_seed)),
            ],
            2 | 3 => vec![
                field_row(
                    &tr!("node.acestep_generate.field.region_start"),
                    make_slider_row(repaint_start_sec, 0.0, 600.0, 0.1, 1),
                ),
                field_row(
                    &tr!("node.acestep_generate.field.region_end"),
                    make_slider_row(repaint_end_sec, -1.0, 600.0, 0.1, 1),
                ),
                field_row("Strength", make_slider_row(repaint_strength, 0.0, 1.0, 0.01, 2)),
            ],
            4 => vec![
                field_row("Edit n_min", make_slider_row(edit_n_min, 0.0, 1.0, 0.01, 2)),
                field_row("Edit n_max", make_slider_row(edit_n_max, 0.0, 1.0, 0.01, 2)),
            ],
            6 => vec![field_row(
                &tr!("node.acestep_generate.field.track"),
                super::make_dropdown(TRACK_OPTIONS, track_idx),
            )],
            _ => vec![],
        }
    });

    let rows: Vec<Box<dyn Widget>> = vec![
        Box::new(
            Padding::symmetric(super::NODE_PADDING_H, super::ROW_PADDING_V).child(
                Text::new(tr!("node.acestep_generate.hint"))
                    .class("node-card-hint acestep-model-hint"),
            ),
        ),
        field_row(&tr!("node.acestep_generate.field.mode"), make_mode_dropdown(s.mode_idx)),
        Box::new(mode_rows),
        field_row(
            "Preset",
            make_preset_dropdown(s.preset, s.infer_steps, s.cfg_scale, s.flow_match_shift),
        ),
        field_row(
            &tr!("node.acestep_generate.field.duration"),
            make_slider_row(s.duration_seconds, -1.0, 600.0, 0.1, 1),
        ),
        field_row("Seed (0 = random)", make_seed_slider(s.seed)),
        // ── AR (5Hz LM): генерация audio-кодов + метадата-оверрайды ──
        section_header("AR · 5Hz LM"),
        field_row(&tr!("node.acestep_generate.field.ar_enabled"), make_toggle(s.use_ar)),
        field_row("BPM (0 = N/A)", make_int_slider_row(s.bpm, 0, 300, 1)),
        field_row(
            &tr!("node.acestep_generate.field.keyscale"),
            super::make_dropdown(KEYSCALE_OPTIONS, s.keyscale_idx),
        ),
        field_row(
            &tr!("node.acestep_generate.field.timesig"),
            super::make_dropdown(TIMESIG_OPTIONS, s.timesig_idx),
        ),
        field_row("CoT (Phase-1)", make_toggle(s.use_cot)),
        field_row("Temperature", make_slider_row(s.temperature, 0.0, 2.0, 0.01, 2)),
        field_row("Top-p", make_slider_row(s.top_p, 0.0, 1.0, 0.01, 2)),
        field_row("Top-k", make_int_slider_row(s.top_k, 0, 200, 1)),
        field_row("Min-p", make_slider_row(s.min_p, 0.0, 1.0, 0.01, 2)),
        field_row("LM CFG", make_slider_row(s.lm_cfg_scale, 1.0, 3.0, 0.1, 1)),
        // ── DiT (диффузия): шаги + CFG + DCW-коррекция ──
        section_header(&tr!("node.acestep_generate.section.dit")),
        field_row("Steps", make_int_slider_row(s.infer_steps, 4, 200, 1)),
        field_row("CFG scale", make_slider_row(s.cfg_scale, 1.0, 15.0, 0.1, 1)),
        field_row("Shift", make_slider_row(s.flow_match_shift, 1.0, 5.0, 0.1, 2)),
        field_row("DCW on/off", make_toggle(s.enable_dcw)),
        field_row("DCW preset", make_dcw_preset_dropdown(s.dcw_preset, s.dcw_scaler, s.dcw_high_scaler)),
        field_row("DCW mode", make_dcw_mode_dropdown(s.dcw_mode)),
        field_row("DCW wavelet", make_dcw_wavelet_dropdown(s.dcw_wavelet)),
        field_row("DCW scaler", make_slider_row(s.dcw_scaler, 0.0, 0.2, 0.001, 3)),
        field_row("DCW high", make_slider_row(s.dcw_high_scaler, 0.0, 0.2, 0.001, 3)),
        // ── Выход: нормализация PCM + статус ──
        section_header(&tr!("node.acestep_generate.section.output")),
        field_row(&tr!("node.acestep_generate.field.normalization"), make_norm_dropdown(s.norm_mode)),
        field_row(
            &tr!("nodes.common.status"),
            status_row(
                s.running,
                s.error,
                s.loaded_name,
                tr!("nodes.common.generating"),
                "acestep-node-running",
            ),
        ),
    ];
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

// ── Run ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
struct RunParams {
    handle: Arc<AceStepModelHandle>,
    mode_idx: usize,
    tags: String,
    lyrics: String,
    src_latent: Option<Tensor>,
    duration_sec: u32,
    steps: usize,
    cfg: f32,
    shift: f32,
    seed: u64,
    temperature: f32,
    top_p: f32,
    top_k: usize,
    min_p: f32,
    lm_cfg: f32,
    use_cot: bool,
    use_ar: bool,
    bpm: Option<u32>,
    keyscale: Option<String>,
    timesig: Option<String>,
    norm_mode: NormMode,
    dcw: DcwCorrector,
    retake_variance: f32,
    retake_seed: u64,
    repaint_start_sec: f32,
    repaint_end_sec: f32,
    repaint_strength: f32,
    edit_n_min: f32,
    edit_n_max: f32,
    track_idx: usize,
}

/// Seed `0` — «случайный на каждый прогон» (как `use_random_seed` у ACE-Step:
/// там это `-1`, но SpinBox отрицательных не даёт). Берётся из 1..=u32::MAX,
/// чтобы выпавшее значение можно было вписать обратно в поле и повторить трек.
fn resolve_seed(v: u64) -> u64 {
    if v == 0 {
        u64::from(rand::random::<u32>().max(1))
    } else {
        v
    }
}

pub fn start(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(s) = snapshot(node) else {
        return;
    };
    if s.running.get_untracked() {
        return;
    }
    let Some(handle) = current_input_model(ctx, node.id, "model") else {
        s.error.set(Some(tr!("node.acestep_generate.error.no_checkpoint")));
        return;
    };
    let mode = s.mode_idx.get_untracked();
    // Все 5 режимов (text2music/retake/repaint/extend/edit) реализованы в
    // generate_music. Для repaint/extend/edit нужен вход src_latent — если он
    // не подключён, generate_music вернёт понятную ошибку.
    if matches!(mode, 2..=6) && current_input_latent(ctx, node.id, "src_latent").is_none() {
        let name = MODE_OPTIONS.get(mode).copied().unwrap_or("?");
        s.error.set(Some(tr!(
            "node.acestep_generate.error.mode_needs_src_latent",
            name = name
        )));
        return;
    }
    // Резолв путей и preset-auto на main-thread (можем писать сигналы/ошибку).
    let (lm, te, dit, vae) = match resolve_paths(&handle) {
        Ok(p) => p,
        Err(e) => {
            s.error.set(Some(e));
            return;
        }
    };
    // AR off (no-codes) штатно ТОЛЬКО для turbo: base/sft не дистиллированы под
    // пустые коды и падают illegal-instruction. Даём понятную ошибку заранее.
    // Extract LM не зовёт вовсе: DiT берёт контекст из исходника.
    if mode != 6
        && !s.use_ar.get_untracked()
        && !matches!(detect_xl_bundle_kind(&dit), XlBundleKind::Turbo)
    {
        s.error.set(Some(tr!("node.acestep_generate.error.no_ar_requires_turbo")));
        return;
    }
    let (steps, cfg, shift) = if matches!(s.preset.get_untracked(), SamplerPreset::Auto) {
        let kind = detect_xl_bundle_kind(&dit);
        let st = kind.default_infer_steps();
        let cf = kind.default_cfg_scale();
        let sh = kind.default_flow_match_shift();
        s.infer_steps.set(st);
        s.cfg_scale.set(cf);
        s.flow_match_shift.set(sh);
        (st as usize, cf, sh)
    } else {
        (
            s.infer_steps.get_untracked().max(1) as usize,
            s.cfg_scale.get_untracked(),
            s.flow_match_shift.get_untracked(),
        )
    };
    let _ = (lm, te, vae); // пути перерезолвятся в worker'е через resolve_paths(handle)

    let tags = current_input_text(ctx, node.id, "tags").unwrap_or_default();
    let lyrics = current_input_text(ctx, node.id, "lyrics").unwrap_or_default();
    let src_latent = current_input_latent(ctx, node.id, "src_latent");

    let duration_raw = s.duration_seconds.get_untracked();
    let duration_sec: u32 = if duration_raw <= 0.0 { 0 } else { duration_raw.round() as u32 };

    let dcw = DcwCorrector {
        enabled: s.enable_dcw.get_untracked(),
        mode: s.dcw_mode.get_untracked().to_corrector_mode(),
        scaler: s.dcw_scaler.get_untracked(),
        high_scaler: s.dcw_high_scaler.get_untracked(),
    };

    // Метадата-оверрайды: индекс 0 = N/A → None (Python пишет "N/A"); BPM 0 = N/A.
    let ks_idx = s.keyscale_idx.get_untracked();
    let ts_idx = s.timesig_idx.get_untracked();
    let bpm_raw = s.bpm.get_untracked();
    let keyscale =
        if ks_idx == 0 { None } else { KEYSCALE_OPTIONS.get(ks_idx).map(|s| (*s).to_string()) };
    let timesig =
        if ts_idx == 0 { None } else { TIMESIG_OPTIONS.get(ts_idx).map(|s| (*s).to_string()) };
    let bpm = if bpm_raw == 0 { None } else { Some(bpm_raw) };
    let norm_mode = match s.norm_mode.get_untracked() {
        PostNormMode::None => NormMode::Off,
        PostNormMode::Rms => NormMode::Rms,
        PostNormMode::Peak => NormMode::Peak,
    };

    let params = RunParams {
        handle,
        mode_idx: mode,
        tags,
        lyrics,
        src_latent,
        duration_sec,
        steps,
        cfg,
        shift,
        seed: resolve_seed(s.seed.get_untracked()),
        temperature: s.temperature.get_untracked(),
        top_p: s.top_p.get_untracked(),
        top_k: s.top_k.get_untracked() as usize,
        min_p: s.min_p.get_untracked(),
        lm_cfg: s.lm_cfg_scale.get_untracked(),
        use_cot: s.use_cot.get_untracked(),
        use_ar: s.use_ar.get_untracked(),
        bpm,
        keyscale,
        timesig,
        norm_mode,
        dcw,
        retake_variance: s.retake_variance.get_untracked(),
        retake_seed: resolve_seed(s.retake_seed.get_untracked()),
        repaint_start_sec: s.repaint_start_sec.get_untracked(),
        repaint_end_sec: s.repaint_end_sec.get_untracked(),
        repaint_strength: s.repaint_strength.get_untracked(),
        edit_n_min: s.edit_n_min.get_untracked(),
        edit_n_max: s.edit_n_max.get_untracked(),
        track_idx: s.track_idx.get_untracked(),
    };

    s.running.set(true);
    s.error.set(None);

    let (output_buf_audio, output_buf_latent, output_version) = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AceStepGenerate {
                output_buf_audio,
                output_buf_latent,
                output_version,
                ..
            } => (output_buf_audio.clone(), output_buf_latent.clone(), *output_version),
            _ => return,
        },
        Err(_) => return,
    };
    let running = s.running;
    let error = s.error;
    let loaded_name = s.loaded_name;

    let _ = thread::Builder::new()
        .name("synthos-acestep-generate".into())
        .spawn(move || {
            worker(
                params,
                running,
                error,
                loaded_name,
                output_buf_audio,
                output_buf_latent,
                output_version,
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn worker(
    p: RunParams,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    output_buf_audio: Arc<Mutex<Option<Arc<AudioBuffer>>>>,
    output_buf_latent: Arc<Mutex<Option<Tensor>>>,
    output_version: RwSignal<u32>,
) {
    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();

    let (lm, te, dit, vae) = match resolve_paths(&p.handle) {
        Ok(t) => t,
        Err(e) => {
            error.set(Some(e));
            running.set(false);
            return;
        }
    };
    let device = device_from_idx(p.handle.device_idx);
    let compute = compute_from_idx(p.handle.compute_idx);
    let dit_quant = quant_from_idx(p.handle.quant_dit_idx).unwrap_or(compute);
    let enc_quant = quant_from_idx(p.handle.quant_enc_idx).unwrap_or(compute);

    let name = dit
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| dit.display().to_string());
    loaded_name.set(Some(name.clone()));

    // «Держать в памяти» на ACE-Step Checkpoint: резидентный кэш компонентов
    // (LM/TE/DiT/VAE) переживает прогоны — повторная генерация не платит
    // загрузку и квантизацию. Lock держится на весь прогон: второй
    // параллельный Generate подождёт (одновременно им VRAM всё равно не
    // хватит). Выключенная резидентность освобождает прежний кэш.
    let resident = p.handle.resident;
    let cache_slot = resident_cache().clone();
    let mut cache_guard = if resident {
        cache_slot.lock().ok()
    } else {
        if let Ok(mut g) = cache_slot.lock() {
            if g.take().is_some() {
                crate::models::trim_all();
                tracing::info!("[acestep] резидентный кэш освобождён (чекбокс выключен)");
            }
        }
        None
    };
    let cache_was_empty = cache_guard
        .as_ref()
        .map(|g| g.is_none())
        .unwrap_or(false);
    let vram_before_cache = crate::models::cuda_allocated();
    let cache_ref: Option<&mut MusicComponentCache> = cache_guard
        .as_mut()
        .map(|g| g.get_or_insert_with(MusicComponentCache::default));

    let paths = MusicPaths { lm: &lm, text_encoder: &te, dit: &dit, vae: &vae };
    let opts = SamplerOptions {
        steps: p.steps,
        shift: p.shift,
        guidance_scale: p.cfg,
        dcw: p.dcw.clone(),
    };
    let copts = CodesGenOptions {
        temperature: p.temperature,
        top_p: p.top_p,
        top_k: p.top_k,
        min_p: p.min_p,
        cfg_scale: p.lm_cfg,
        seed: p.seed,
        ..CodesGenOptions::default()
    };

    // src_latent → [1, T, 64] channels-last F32 (VaeEncode даёт [1,64,T]).
    let src_cl = match p.src_latent.as_ref() {
        Some(t) => match t.to_device(device).and_then(|t| t.to_dtype(DType::F32)) {
            Ok(t) => {
                let d = t.dims().to_vec();
                if d.len() == 3 && d[2] == 64 {
                    Some(t)
                } else if d.len() == 3 && d[1] == 64 {
                    t.transpose(1, 2).and_then(|t| t.contiguous()).ok()
                } else {
                    None
                }
            }
            Err(_) => None,
        },
        None => None,
    };
    // Автодетект длительности из src (латент 25 Гц → сек = T/25) для аудио-
    // режимов repaint/edit/cover/extract — выход = длина исходника, ручной ввод
    // не нужен. Extend (mode 3) использует заданную длительность (она больше src).
    // text2music/retake — без src, берут duration со слайдера.
    let eff_duration = match (p.mode_idx, src_cl.as_ref()) {
        (2 | 4 | 5 | 6, Some(s)) => {
            let t = s.dims().get(1).copied().unwrap_or(0);
            ((t as f32 / 25.0).round() as u32).max(1)
        }
        _ => p.duration_sec,
    };
    let edit = EditOptions {
        mode: edit_mode(p.mode_idx),
        track_name: track_name(p.track_idx).to_string(),
        retake_variance: p.retake_variance,
        retake_seed: p.retake_seed,
        src_latent: src_cl,
        repaint_start_sec: p.repaint_start_sec,
        repaint_end_sec: p.repaint_end_sec,
        repaint_strength: p.repaint_strength,
        edit_n_min: p.edit_n_min,
        edit_n_max: p.edit_n_max,
        edit_n_avg: 1,
        edit_source_caption: String::new(),
        edit_source_lyric: String::new(),
    };

    let started = std::time::Instant::now();
    // Пока идёт прогон, панель «Модели в памяти» видит текущий компонент:
    // LM → TE → DiT → VAE грузятся по очереди внутри generate_music. Запись
    // одна (общий ключ, стадия заменяет прежнюю), жива, пока жив `run_token`.
    let run_token = Arc::new(());
    let run_weak = Arc::downgrade(&run_token);
    let vram_base = crate::models::cuda_allocated();
    let on_stage = StageHook(Arc::new(move |stage: MusicStage, path: &std::path::Path| {
        let component = match stage {
            MusicStage::Lm => "5Hz LM",
            MusicStage::TextEncoder => "Text Encoder",
            MusicStage::Dit => "DiT",
            MusicStage::Vae => "VAE",
        };
        let label = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        crate::models::register_weak(
            "acestep/run",
            "ACE-Step",
            component,
            label,
            device,
            crate::models::cuda_allocated().saturating_sub(vram_base),
            run_weak.clone(),
            || {},
        );
    }));
    let extras = GenExtras {
        use_ar: p.use_ar,
        bpm: p.bpm,
        keyscale: p.keyscale.clone(),
        timesig: p.timesig.clone(),
        norm_mode: p.norm_mode,
        on_stage: Some(on_stage),
    };
    let (samples, sr, latent) = match generate_music(
        &paths,
        &p.tags,
        &p.lyrics,
        eff_duration,
        device,
        compute,
        dit_quant,
        enc_quant,
        &opts,
        &copts,
        p.use_cot,
        &edit,
        &extras,
        cache_ref,
    ) {
        Ok(r) => r,
        Err(e) => {
            error.set(Some(format!("generate_music: {e}")));
            drop(run_token);
            if !resident {
                crate::models::trim_all();
            }
            crate::models::changed();
            running.set(false);
            return;
        }
    };
    drop(run_token);
    crate::models::changed();

    // Резидентный кэш заполнился в этом прогоне — показать в панели
    // «Модели в памяти» (unload оттуда очистит слот).
    if resident {
        let filled = cache_guard
            .as_ref()
            .map(|g| g.as_ref().map(|c| c.is_loaded()).unwrap_or(false))
            .unwrap_or(false);
        drop(cache_guard.take());
        if filled && cache_was_empty {
            let bytes = crate::models::cuda_allocated().saturating_sub(vram_before_cache);
            let label = dit
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| dit.display().to_string());
            crate::models::register_slot(
                "acestep/resident-cache".to_string(),
                "ACE-Step",
                "Resident (LM+TE+DiT+VAE)",
                label,
                device,
                bytes,
                cache_slot.clone(),
                || {},
            );
        }
    }

    // Без резидентности компоненты уже отпущены generate_music, но пул CUDA
    // держит их блоки: без trim процесс так и сидит на ~9 ГБ после прогона.
    if !resident {
        crate::models::trim_all();
    }

    let dur = samples.len() as f32 / sr.max(1) as f32;
    tracing::info!(
        "[acestep] Generate ✓ {dur:.1}s аудио за {:.1}s (steps={}, cfg={:.1}, seed={})",
        started.elapsed().as_secs_f32(),
        p.steps,
        p.cfg,
        p.seed
    );
    // Выпавший seed виден в статусе — удачный трек повторяется вписыванием.
    loaded_name.set(Some(format!("{name} · seed {}", p.seed)));

    let buf = Arc::new(AudioBuffer::new(
        Arc::from(samples.into_boxed_slice()),
        sr,
        1,
    ));
    if let Ok(mut g) = output_buf_audio.lock() {
        *g = Some(buf);
    }
    if let Ok(mut g) = output_buf_latent.lock() {
        // Финальный латент [1,64,T] — для выхода `latent` (цепочка retake/repaint).
        *g = Some(latent);
    }
    syngui::prelude::run_on_main_thread(move || {
        output_version.update(|v| *v = v.wrapping_add(1));
    });
    error.set(None);
    running.set(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(dir: &std::path::Path) -> AceStepModelHandle {
        AceStepModelHandle {
            models_dir: Some(dir.to_path_buf()),
            lm_path: None,
            text_encoder_path: None,
            dit_path: None,
            vae_path: None,
            device_idx: 1,
            quant_dit_idx: 0,
            quant_enc_idx: 0,
            compute_idx: 0,
            resident: false,
        }
    }

    fn touch(dir: &std::path::Path, names: &[&str]) {
        for n in names {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
    }

    /// Каталог с 1.7b-LM и turbo-DiT (без 4b/base) резолвится без override'ов —
    /// раньше жёсткие имена давали «bundle не найден».
    #[test]
    fn resolve_paths_falls_back_to_alternate_names() {
        let dir = tempfile::tempdir().unwrap();
        touch(
            dir.path(),
            &[
                "acestep_5hz_lm_1.7b.syn",
                "qwen3-embedding-0.6b.syn",
                "acestep_v15_xl_turbo.syn",
                "acestep_vae.syn",
            ],
        );
        let (lm, te, dit, vae) = resolve_paths(&handle(dir.path())).expect("resolve");
        assert_eq!(lm.file_name().unwrap(), "acestep_5hz_lm_1.7b.syn");
        assert_eq!(te.file_name().unwrap(), "qwen3-embedding-0.6b.syn");
        assert_eq!(dit.file_name().unwrap(), "acestep_v15_xl_turbo.syn");
        assert_eq!(vae.file_name().unwrap(), "acestep_vae.syn");
    }

    /// Оба варианта в каталоге — берётся первый по приоритету (4b / base).
    #[test]
    fn resolve_paths_prefers_first_candidate() {
        let dir = tempfile::tempdir().unwrap();
        touch(
            dir.path(),
            &[
                "acestep_5hz_lm_1.7b.syn",
                "acestep_5hz_lm_4b.syn",
                "qwen3-embedding-0.6b.syn",
                "acestep_v15_xl_base.syn",
                "acestep_v15_xl_turbo.syn",
                "acestep_vae.syn",
            ],
        );
        let (lm, _, dit, _) = resolve_paths(&handle(dir.path())).expect("resolve");
        assert_eq!(lm.file_name().unwrap(), "acestep_5hz_lm_4b.syn");
        assert_eq!(dit.file_name().unwrap(), "acestep_v15_xl_base.syn");
    }

    /// Override сильнее каталога.
    #[test]
    fn resolve_paths_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        touch(
            dir.path(),
            &[
                "acestep_5hz_lm_1.7b.syn",
                "qwen3-embedding-0.6b.syn",
                "acestep_v15_xl_base.syn",
                "acestep_vae.syn",
                "my_lm.syn",
            ],
        );
        let mut h = handle(dir.path());
        h.lm_path = Some(dir.path().join("my_lm.syn"));
        let (lm, _, _, _) = resolve_paths(&h).expect("resolve");
        assert_eq!(lm.file_name().unwrap(), "my_lm.syn");
    }

    /// Голое имя в override (так пишет агент — схема ноды печатает имена
    /// бандлов) резолвится от каталога моделей, а не от cwd процесса.
    #[test]
    fn resolve_paths_relative_override_joins_models_dir() {
        let dir = tempfile::tempdir().unwrap();
        touch(
            dir.path(),
            &[
                "acestep_5hz_lm_1.7b.syn",
                "acestep_5hz_lm_4b.syn",
                "qwen3-embedding-0.6b.syn",
                "acestep_v15_xl_base.syn",
                "acestep_vae.syn",
            ],
        );
        let mut h = handle(dir.path());
        h.lm_path = Some(PathBuf::from("acestep_5hz_lm_1.7b.syn"));
        h.dit_path = Some(PathBuf::from("acestep_v15_xl_base.syn"));
        let (lm, _, dit, _) = resolve_paths(&h).expect("resolve");
        assert_eq!(lm, dir.path().join("acestep_5hz_lm_1.7b.syn"));
        assert_eq!(dit, dir.path().join("acestep_v15_xl_base.syn"));
    }

    /// Нет ни каталога, ни override'ов — понятная ошибка, а не паника.
    #[test]
    fn resolve_paths_without_dir_errors() {
        let mut h = handle(std::path::Path::new("/nonexistent"));
        h.models_dir = None;
        assert!(resolve_paths(&h).is_err());
    }

    /// Cover больше не делит ветку с extract: extract в synaptix идёт без LM
    /// и со своей инструкцией, cover — прежним путём.
    #[test]
    fn extract_and_cover_map_to_their_own_modes() {
        assert_eq!(MODE_OPTIONS[5], "cover");
        assert_eq!(MODE_OPTIONS[6], "extract");
        assert_eq!(edit_mode(5), EditMode::Cover);
        assert_eq!(edit_mode(6), EditMode::Extract);
        assert_eq!(edit_mode(3), EditMode::Repaint);
        assert_eq!(edit_mode(0), EditMode::Text2Music);
    }

    /// Дропдаун «Дорожка» — ровно стемы ACE-Step, дефолт и выход за границы
    /// дают вокал.
    #[test]
    fn track_options_are_acestep_stems() {
        use synaptix_music_acestep::text_encoder::TRACK_NAMES;
        assert_eq!(TRACK_OPTIONS.len(), TRACK_NAMES.len());
        for t in TRACK_OPTIONS {
            assert!(TRACK_NAMES.contains(t), "{t} не из TRACK_NAMES");
        }
        assert_eq!(track_name(0), "vocals");
        assert_eq!(track_name(99), "vocals");
        assert_eq!(
            TRACK_OPTIONS[crate::templates::model::AceStepGenerateStateData::default().track_idx],
            "vocals"
        );
    }
}
