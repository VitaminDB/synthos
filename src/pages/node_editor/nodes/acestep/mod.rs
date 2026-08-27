//! Семейство нод **ACE-Step v1.5** (мульти-модельный text-to-music
//! пайплайн). Категория Нейро → подкатегория «ACE-Step».
//!
//! Разбиение по нодам (8 шт.):
//!
//! - [`text_encoder`] — `tags: Text → text_emb: Hidden` (Qwen 0.6B +
//!   TextProjector).
//! - [`lyric_encoder`] — `lyrics: Text → lyric_emb: Hidden` (Qwen 0.6B
//!   embed-only + LyricEncoder 8L).
//! - [`timbre_encoder`] — `ref_latent: Latent → timbre_emb: Hidden`
//!   (TimbreEncoder 4L).
//! - [`pack`] — `(text_emb, lyric_emb, timbre_emb) → cond: Conditioning`
//!   (без весов, чистый `Tensor::cat(dim=1)`).
//! - [`vae_encode`] — `audio: Audio → latent: Latent` (VAE encoder,
//!   48 kHz, hop=1920, 25 Hz латент).
//! - [`vae_decode`] — `latent: Latent → audio: Audio` (VAE decoder +
//!   опц. post-norm).
//! - [`ar_lm`] — `(tags, lyrics) → src_latent: Latent` (Qwen 4B AR +
//!   FSQ + AudioTokenDetokenizer).
//! - [`sampler`] — `(cond, опц. src_latent) → latent: Latent`
//!   (DiT 32L + FlowMatchEulerDiscreteScheduler, цикл с DCW/APG).
//!
//! Веса грузятся **только из `.syn`-формата** через [`shared::get_or_load`]:
//! ноды с одинаковым `(path, device, storage_dtype, compute_dtype)`
//! разделяют одну `Arc<T>`-инстанцию модели (см. [`shared::ModelKey`]).
//!
//! ## Общие UI-helper'ы
//!
//! [`DEVICE_OPTIONS`], [`QUANT_OPTIONS`], [`COMPUTE_OPTIONS`] — dropdown'ы на
//! каждой ноде; [`device_from_idx`], [`quant_from_idx`], [`compute_from_idx`] —
//! конвертеры индекса в `synaptix_core` `Device`/`DType`. Квант (nvfp4/mxfp8)
//! действует на Sampler (DiT) / ArLm (LM) / TextEncoder; на прочих игнорируется.

pub mod shared;

pub mod checkpoint;
pub mod generate;
pub mod vae_encode;

use synaptix_core::{device::Device, dtype::DType};

/// Каталог моделей приложения (настройка «Каталог моделей», `AppConfig.models_dir`,
/// с раскрытием `~`). Дефолт поля «Каталог моделей» у ACE-Step Checkpoint:
/// шаблоны кладут `models_dir: None`, и раньше нода из шаблона приходила
/// пустой — пользователю приходилось руками выбирать каталог и все 4 бандла.
/// Вне UI-контекста (тесты) — `~/Storage/syn_models`.
pub fn app_models_dir() -> std::path::PathBuf {
    let raw = syngui::context_provider::try_use_context::<crate::context::AppCtx>()
        .map(|c| c.models_dir.get_untracked())
        .unwrap_or_default();
    crate::config::resolve_models_dir(&raw)
}

/// Доступные устройства. 0 = CPU, 1 = GPU (CUDA → Metal auto-detect).
pub const DEVICE_OPTIONS: &[&str] = &["CPU", "GPU (auto)"];

/// Квант весов больших линейных. `none` → dense (compute-dtype). `nvfp4`/`mxfp8`
/// квантуют attn/mlp DiT (Sampler) и attn/mlp LM+text-enc (ArLm/TextEncoder);
/// для LM/text-enc compute форсится в F16 (квант-ядра требуют F16-актив). На
/// прочих нодах (VAE/timbre/fsq/detok) квант игнорируется. Индекс хранится в
/// сигнале `storage_idx` (бэк-совместимо: старые storage-индексы вне диапазона
/// → none = dense).
pub const QUANT_OPTIONS: &[&str] = &["none", "nvfp4", "mxfp8"];

/// Compute dtype forward-проходов (dense-режим). F16 НАМЕРЕННО исключён —
/// диапазон ±65504 даёт inf→NaN в lyric encoder layer 7 на лирике >256 токенов
/// Reference Python — BF16 на
/// Ampere+. На CPU/pre-Ampere — F32. (Квант-пресеты сами форсят F16 только для
/// LM/text-enc.)
pub const COMPUTE_OPTIONS: &[&str] = &["bf16", "f32"];

/// `none` (индекс 0) — без кванта (dense compute-dtype).
pub fn default_storage_idx() -> usize {
    0
}
/// `bf16` (индекс 0) — эталонный compute dtype Python ACE-Step на Ampere+.
pub fn default_compute_idx() -> usize {
    0
}

pub fn device_from_idx(i: usize) -> Device {
    match i {
        1 => Device::Cuda(0),
        _ => Device::Cpu,
    }
}

/// Квант-схема из индекса дропдауна «Quant» (сигнал `storage_idx`):
/// `None` = dense, `Some(NVFP4/MXFP8)` = квант весов attn/mlp.
pub fn quant_from_idx(i: usize) -> Option<DType> {
    match QUANT_OPTIONS.get(i).copied() {
        Some("nvfp4") => Some(DType::NVFP4),
        Some("mxfp8") => Some(DType::MXFP8),
        _ => None,
    }
}

pub fn compute_from_idx(i: usize) -> DType {
    match COMPUTE_OPTIONS.get(i).copied() {
        Some("f32") => DType::F32,
        _ => DType::BF16,
    }
}

pub fn idx_in(arr: &[&str], s: &str) -> Option<usize> {
    arr.iter().position(|x| *x == s)
}

// ── Input port helpers ────────────────────────────────────────────────────

use super::super::state::NodeEditorCtx;
use super::super::types::{
    AceStepBlob, AceStepModelHandle, Connection, DataBlob, NodeId, PortValue,
};

/// Прочитать хэндл ACE-Step чекпойнта (выход Checkpoint-ноды) из
/// подключённого upstream'а. Используется Generate-нодой (порт `model`).
pub fn current_input_model(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<std::sync::Arc<AceStepModelHandle>> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_acestep_model()
}

/// Прочитать текущий PortValue::Text из подключённого upstream'а.
/// Возвращает `None`, если порт не подключён, или upstream выдал другой тип.
pub fn current_input_text(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<String> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    match pv {
        PortValue::Text(s) => Some(s),
        _ => None,
    }
}

/// Прочитать текущий PortValue::Audio из подключённого upstream'а.
pub fn current_input_audio(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<std::sync::Arc<syngui::audio::AudioBuffer>> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_audio()
}

/// Прочитать AceStep-латент `[B, 64, T]` из подключённого upstream'а.
/// Используется TimbreEncoder (`ref_latent`), Sampler (`src_latent`),
/// VaeDecode (`latent`).
pub fn current_input_latent(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_acestep_latent()
}

/// Прочитать AceStep-conditioning `[B, T_cond, 2048]`. Используется Sampler.
pub fn current_input_conditioning(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<synaptix_core::tensor::Tensor> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_acestep_conditioning()
}

/// Прочитать AceStep conditioning + lens (для будущих task_chunk_masks).
/// Сейчас Sampler пробрасывает lens только в tracing::debug.
pub fn current_input_conditioning_with_lens(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<(synaptix_core::tensor::Tensor, usize, usize, usize)> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_acestep_conditioning_with_lens()
}

/// Прочитать Phase 1 CoT metas (`bpm/keyscale/timesignature/duration/language`)
/// из upstream ArLm-ноды. Используется TextEncoder / LyricEncoder.
pub fn current_input_phase1_metas(
    ctx: &NodeEditorCtx,
    node_id: NodeId,
    port: &'static str,
) -> Option<std::sync::Arc<synaptix_music_acestep::tokenizer::Metadata>> {
    let conns: Vec<Connection> = ctx.connections.get_untracked();
    let src = conns
        .iter()
        .find(|c| c.to_node == node_id && c.to_port == port)?;
    let values = ctx.values.get_untracked();
    let pv = values.get(&(src.from_node, src.from_port)).cloned()?;
    pv.as_acestep_phase1_metas()
}

// Помечаем что использование AceStepBlob/DataBlob в imports не для warning'а.
#[allow(dead_code)]
fn _phantom_data_blob(_: DataBlob, _: AceStepBlob) {}

// ── UI helpers (общие для всех ACE-Step нод) ──────────────────────────────

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::input::{Slider, TextField, Toggle};
use syngui::widgets::{Column, DecoratedBox, Dropdown, DropdownItem, Reactive, Row, ToolButton};

use crate::icons::MI_FOLDER_OPEN;

/// Стандартный 10-px горизонтальный padding для field-row'ов ноды.
pub const NODE_PADDING_H: f32 = 10.0;
pub const ROW_PADDING_V: f32 = 2.0;

/// File picker для `.syn` bundle'а. При выборе вызывает `on_pick(path)`,
/// затем сбрасывает loaded_name/error. Слева — иконка-кнопка, справа —
/// текущее имя файла (reactive).
pub fn syn_picker_row(
    model_path: RwSignal<Option<std::path::PathBuf>>,
    loaded_name: RwSignal<Option<String>>,
    error_sig: RwSignal<Option<String>>,
    title: impl Into<String>,
) -> Box<dyn Widget> {
    let title = title.into();
    let title_for_dialog = title.clone();
    let pick_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(title)
        .on_click(move || {
            let dlg = rfd::FileDialog::new()
                .add_filter("Syn bundle", &["syn"])
                .set_title(&title_for_dialog);
            if let Some(p) = dlg.pick_file() {
                model_path.set(Some(p));
                loaded_name.set(None);
                error_sig.set(None);
            }
        })
        .class("audio-node-open-btn");
    let filename_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match model_path.get() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Box::new(Text::new(name).class("audio-node-filename"))
            }
            None => Box::new(Text::new(tr!("nodes.common.no_file_selected")).class("audio-node-empty")),
        };
        vec![widget]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![
                Box::new(pick_btn) as Box<dyn Widget>,
                Box::new(filename_text),
            ]),
    )
}

/// Picker каталога моделей (как CLI `--models <dir>`). При выборе пишет путь
/// в сигнал; справа — имя каталога (reactive). Дефолтные имена 4 бандлов
/// резолвятся относительно него у Generate-ноды.
pub fn dir_picker_row(
    tooltip: impl Into<String>,
    sig: RwSignal<Option<std::path::PathBuf>>,
) -> Box<dyn Widget> {
    let tooltip = tooltip.into();
    let tooltip_for_dialog = tooltip.clone();
    let pick_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tooltip)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().set_title(&tooltip_for_dialog);
            if let Some(p) = dlg.pick_folder() {
                sig.set(Some(p));
            }
        })
        .class("audio-node-open-btn");
    let dirname_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match sig.get() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Box::new(Text::new(name).class("audio-node-filename"))
            }
            None => Box::new(Text::new(tr!("nodes.common.no_folder_selected")).class("audio-node-empty")),
        };
        vec![widget]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![
                Box::new(pick_btn) as Box<dyn Widget>,
                Box::new(dirname_text),
            ]),
    )
}

/// Dropdown с привязкой к `RwSignal<usize>` (индекс выбранной опции).
pub fn make_dropdown(options: &'static [&'static str], idx: RwSignal<usize>) -> Box<dyn Widget> {
    let items: Vec<DropdownItem> = options.iter().map(|s| DropdownItem::simple(*s)).collect();
    let current = options
        .get(idx.get_untracked())
        .copied()
        .unwrap_or(options[0])
        .to_string();
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                if let Some(i) = idx_in(options, s) {
                    idx.set(i);
                }
            })
            .class("node-input-dropdown"),
    )
}

/// Slider со встроенным readout'ом (`Slider::show_value`): клик по числу —
/// точный текстовый ввод (снап к step + кламп). Привязка к `RwSignal<f32>`;
/// внешние изменения сигнала двигают ползунок (Reactive-обёртка).
pub fn make_slider_row(
    sig: RwSignal<f32>,
    min: f32,
    max: f32,
    step: f32,
    decimals: usize,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![Box::new(
            Slider::new()
                .value(sig.get())
                .range(min, max)
                .step(step)
                .show_value(decimals as u8)
                .on_change(move |v| sig.set(v))
                .class("node-input-slider acestep-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}

/// Int-slider (`RwSignal<u32>`). `Slider<f32>` под капотом, на изменение
/// округляется до ближайшего u32. Readout встроенный, с текстовым вводом.
pub fn make_int_slider_row(
    sig: RwSignal<u32>,
    min: u32,
    max: u32,
    step: u32,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        vec![Box::new(
            Slider::new()
                .value(sig.get() as f32)
                .range(min as f32, max as f32)
                .step(step as f32)
                .show_value(0)
                .on_change(move |v| sig.set(v.round().max(0.0) as u32))
                .class("node-input-slider acestep-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}

/// Bool-переключатель (Switch) с привязкой к `RwSignal<bool>`. Без подписи —
/// label рисуется снаружи через `field_row`.
pub fn make_toggle(sig: RwSignal<bool>) -> Box<dyn Widget> {
    Box::new(
        Toggle::new()
            .on(sig.get_untracked())
            .on_change(move |b| sig.set(b))
            .class("node-input-toggle"),
    )
}

/// Step counter (SpinBox) для u64 (seed). Точный keyboard input + up/down
/// arrows. `0` = «random» (wall-clock fallback в worker'е), > 0 —
/// детерминированный seed.
pub fn make_seed_slider(sig: RwSignal<u64>) -> Box<dyn Widget> {
    use syngui::widgets::input::SpinBox;
    Box::new(
        SpinBox::new()
            .value(sig.get_untracked() as f64)
            .min(0.0)
            .max(u32::MAX as f64) // u64::MAX не помещается в f64 без потери; 4G seed'ов хватит
            .step(1.0)
            .decimal_places(0)
            .on_change(move |v| sig.set(v.round().max(0.0) as u64))
            .class("node-input-spinbox"),
    )
}

/// Текстовое поле (single-line) с привязкой к `RwSignal<String>`.
pub fn make_text_field(sig: RwSignal<String>, placeholder: impl Into<String>) -> Box<dyn Widget> {
    Box::new(
        TextField::new()
            .text(sig.get_untracked())
            .placeholder(placeholder.into())
            .on_change(move |s| sig.set(s.to_string()))
            .class("node-input-text"),
    )
}

/// Строка с лейблом слева и control'ом справа. Layout идентичен OmniVoice:
/// `[label_cell flex-grow:3 | control_cell flex-grow:7]` через MSS-классы
/// `.acestep-field-label-cell` / `.acestep-field-control-cell`. Без этого
/// control (особенно Slider) не получает остатка ширины и слипается в
/// ~120px фиксированной геометрии `.node-input-slider`. Padding задаётся
/// MSS-свойством на cells, чтобы не тащить отдельный widget-wrapper.
pub fn field_row(label: &str, control: Box<dyn Widget>) -> Box<dyn Widget> {
    let label_cell: Box<dyn Widget> = Box::new(
        DecoratedBox::new()
            .child(Text::new(label).class("node-card-field-label acestep-field-label"))
            .class("acestep-field-label-cell"),
    );
    // DecoratedBox::child<M: IntoWidget> не принимает Box<dyn Widget>;
    // поле `child` публичное — выставляем напрямую и далее class() для MSS.
    let mut control_cell_inner = DecoratedBox::new();
    control_cell_inner.child = Some(control);
    let control_cell: Box<dyn Widget> =
        Box::new(control_cell_inner.class("acestep-field-control-cell"));
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![label_cell, control_cell])
            .class("acestep-field-row"),
    )
}

/// Status-row: показывает ошибку, прогресс или имя загруженной модели.
/// Используется как последняя строка body'а каждой ACE-Step ноды.
pub fn status_row(
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    busy_label: impl Into<String>,
    pulse_class: &'static str,
) -> Box<dyn Widget> {
    let busy_label = busy_label.into();
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error_sig.get() {
            return vec![
                Box::new(Text::new(tr!("nodes.common.error", error = msg)).class("audio-node-error"))
                    as Box<dyn Widget>,
            ];
        }
        if running.get() {
            let cls = format!("audio-node-meta {pulse_class}");
            return vec![
                Box::new(Text::new(busy_label.clone()).class(cls.as_str())) as Box<dyn Widget>,
            ];
        }
        if let Some(name) = loaded_name.get() {
            return vec![Box::new(Text::new(name).class("audio-node-meta")) as Box<dyn Widget>];
        }
        vec![Box::new(Text::new("—").class("audio-node-meta")) as Box<dyn Widget>]
    }))
}

/// Стандартная сборка body для модельной ACE-Step ноды: подсказка по
/// модели (что именно нужно загрузить) + file picker + device/storage/
/// compute dropdowns + кастомные extra-rows + status.
///
/// `model_hint` — короткая строка вида `«Qwen 0.6B + TextProjector —
/// acestep_v15_xl_turbo.syn»`, которая всегда видна над picker-row.
/// Пустая строка отключает подсказку (используется для Pack-ноды без
/// модели).
pub fn standard_body(
    model_path: RwSignal<Option<std::path::PathBuf>>,
    device_idx: RwSignal<usize>,
    storage_idx: RwSignal<usize>,
    compute_idx: RwSignal<usize>,
    show_quant: bool,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    picker_title: impl Into<String>,
    busy_label: impl Into<String>,
    model_hint: impl Into<String>,
    extra_rows: Vec<Box<dyn Widget>>,
) -> Box<dyn Widget> {
    let model_hint = model_hint.into();
    let mut rows: Vec<Box<dyn Widget>> = Vec::with_capacity(6 + extra_rows.len());
    if !model_hint.is_empty() {
        rows.push(Box::new(
            Padding::symmetric(NODE_PADDING_H, ROW_PADDING_V).child(
                Text::new(model_hint).class("node-card-hint acestep-model-hint"),
            ),
        ));
    }
    rows.push(field_row(&tr!("nodes.common.model"), syn_picker_row(model_path, loaded_name, error_sig, picker_title)));
    rows.push(field_row("Device", make_dropdown(DEVICE_OPTIONS, device_idx)));
    // Quant — только на квантуемых нодах (Sampler/ArLm/TextEncoder); на прочих
    // дропдаун инертен (DiT/LM/text-enc — единственные с dtype-входом), скрываем.
    if show_quant {
        rows.push(field_row("Quant", make_dropdown(QUANT_OPTIONS, storage_idx)));
    }
    rows.push(field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)));
    rows.extend(extra_rows);
    rows.push(field_row(
        &tr!("nodes.common.status"),
        status_row(running, error_sig, loaded_name, busy_label, "acestep-node-running"),
    ));
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

/// Body для ACE-Step ноды БЕЗ собственного path-picker'а: только подсказка
/// (откуда тянутся веса) + device/storage/compute dropdowns + extra rows +
/// status. Используется TimbreEncoder/VaeEncode/VaeDecode/Sampler — путь к
/// xl-/vae-bundle берётся из глобального Settings → AI Models → ACE-Step
/// preset'а.
pub fn standard_body_no_path(
    device_idx: RwSignal<usize>,
    storage_idx: RwSignal<usize>,
    compute_idx: RwSignal<usize>,
    show_quant: bool,
    running: RwSignal<bool>,
    error_sig: RwSignal<Option<String>>,
    loaded_name: RwSignal<Option<String>>,
    busy_label: impl Into<String>,
    model_hint: impl Into<String>,
    extra_rows: Vec<Box<dyn Widget>>,
) -> Box<dyn Widget> {
    let model_hint = model_hint.into();
    let mut rows: Vec<Box<dyn Widget>> = Vec::with_capacity(5 + extra_rows.len());
    if !model_hint.is_empty() {
        rows.push(Box::new(
            Padding::symmetric(NODE_PADDING_H, ROW_PADDING_V).child(
                Text::new(model_hint).class("node-card-hint acestep-model-hint"),
            ),
        ));
    }
    rows.push(field_row("Device", make_dropdown(DEVICE_OPTIONS, device_idx)));
    // Quant — только на квантуемой ноде Sampler (DiT); на Timbre/VAE инертен → скрыт.
    if show_quant {
        rows.push(field_row("Quant", make_dropdown(QUANT_OPTIONS, storage_idx)));
    }
    rows.push(field_row("Compute", make_dropdown(COMPUTE_OPTIONS, compute_idx)));
    rows.extend(extra_rows);
    rows.push(field_row(
        &tr!("nodes.common.status"),
        status_row(running, error_sig, loaded_name, busy_label, "acestep-node-running"),
    ));
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}
