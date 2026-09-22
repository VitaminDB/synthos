//! Переиспользуемые UI-элементы для страниц Settings.
//!
//! Содержит общие helper'ы для построения карточек:
//! - [`row_frame`] — стандартная строка с иконкой, заголовком, описанием и
//!   управляющим элементом справа (по образцу `audio_models::row_frame`).
//! - [`section_card`] — карточка с заголовком и стопкой строк.
//!
//! Для dtype-полей — generic dropdown'ы:
//! - [`storage_dtype_dropdown`] — выбор storage-dtype (F32/BF16/F16/NVFP4/MXFP8).
//! - [`compute_dtype_dropdown`] — выбор compute dtype активаций (F32/BF16/F16/FP8/NVFP4).
//! - [`kv_dtype_dropdown`] — KV-cache dtype: Auto/F16/BF16/F32/FP8E4M3.
//!
//! Они принимают `current: String` и `on_change: F` callback. UI-page строит
//! полную строку через `row_frame(icon, title, desc, control)`.

use syngui::prelude::*;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};

// ─────────────────────────────────────────────────────────────────────────────
// Структурные helper'ы (row_frame / section_card)
// ─────────────────────────────────────────────────────────────────────────────

/// Стандартная строка настроек: иконка + заголовок/описание + контрол справа.
pub fn row_frame(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    control: Box<dyn Widget>,
) -> Box<dyn Widget> {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 18.0).child(inner)),
    )
}

/// Строка настроек без простыни текста: пояснение прячется в подсказку на
/// иконке. Длинные описания в каждой строке превращали страницу в стену
/// текста, из которой не выцепить сам контрол.
pub fn row_tip(
    icon: &'static str,
    title: impl Into<String>,
    tip: impl Into<String>,
    control: Box<dyn Widget>,
) -> Box<dyn Widget> {
    use syngui::widgets::feedback::tooltip::Tooltip;
    let head = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(Text::new(title).class("settings-row-title"));

    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(Tooltip::new(head, tip)))
        .children(vec![control]);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 14.0).child(inner)),
    )
}

/// Карточка с заголовком и стопкой строк-настроек.
pub fn section_card(title: impl Into<String>, rows: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );
    Box::new(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(title).class("settings-section-title"))
            .child(card),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Dtype dropdown'ы
// ─────────────────────────────────────────────────────────────────────────────

/// Storage dtype: формат хранения весов в VRAM. Варианты соответствуют
/// `synaptix_core::dtype::DType`. Callback получает строку (`"f16"`, `"nvfp4"`, ...).
pub fn storage_dtype_dropdown<F>(current: String, on_change: F) -> Box<dyn Widget>
where
    F: Fn(String) + Send + Sync + 'static,
{
    // Подпись зависит от карты: без нативных ядер формат исполняется
    // портируемым путём (деквант в регистрах GEMV / полосами перед GEMM).
    let caps = synaptix::facade::device::cuda_caps(0).ok();
    let native = |ok: Option<bool>| match ok {
        Some(true) => tr!("settings.dtype.native"),
        Some(false) => tr!("settings.dtype.portable"),
        None => String::new(),
    };
    let fp4 = native(caps.as_ref().map(|c| c.fp4_mma()));
    let fp8 = native(caps.as_ref().map(|c| c.mxfp8_mma()));
    let mut items = vec![
        DropdownItem::new("f32", tr!("settings.dtype.storage.f32")),
        DropdownItem::new("bf16", tr!("settings.dtype.storage.bf16")),
        DropdownItem::new("f16", tr!("settings.dtype.storage.f16")),
        DropdownItem::new("nvfp4", format!("NVFP4 — 4.25 bit, FP4 Tensor Cores (Blackwell){fp4}")),
        DropdownItem::new("mxfp8", format!("MXFP8 — 8 bit, FP8 Tensor Cores (Blackwell){fp8}")),
    ];
    for bits in [8u8, 6, 5, 4, 3, 2] {
        items.push(DropdownItem::new(format!("sq{bits}"), format!("SQ{bits} — {:.3} bit, {}", bits as f32 + 0.625, tr!("settings.dtype.sq.desc"))));
    }
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| on_change(s.to_string()))
            .class("models-active-dropdown"),
    )
}

/// Compute dtype: dtype активаций между слоями. F32/BF16/F16 — standard
/// path. FP8/NVFP4 — native cuBLASLt path (требует совместимый storage).
pub fn compute_dtype_dropdown<F>(current: String, on_change: F) -> Box<dyn Widget>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let items = vec![
        DropdownItem::new("f32", tr!("settings.dtype.compute.f32")),
        DropdownItem::new("bf16", tr!("settings.dtype.compute.bf16")),
        DropdownItem::new("f16", tr!("settings.dtype.compute.f16")),
        DropdownItem::new(
            "nvfp4",
            "NVFP4 — native FP4 Tensor Cores (Blackwell sm_120)",
        ),
        DropdownItem::new(
            "mxfp8",
            "MXFP8 — native FP8 Tensor Cores (Hopper+/Ada/Blackwell)",
        ),
    ];
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| on_change(s.to_string()))
            .class("models-active-dropdown"),
    )
}

/// Tied embeddings mode dropdown: Auto / Force On / Force Off.
/// Auto читает `tie_word_embeddings` из модельного config.json — стандартный
/// HF behavior. Force-режимы переопределяют решение (Force On на untied
/// checkpoint изменит числовой выход, но не упадёт).
pub fn tied_embeddings_dropdown<F>(current: String, on_change: F) -> Box<dyn Widget>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let items = vec![
        DropdownItem::new(
            "auto",
            tr!("settings.dtype.tie.auto"),
        ),
        DropdownItem::new(
            "on",
            tr!("settings.dtype.tie.on"),
        ),
        DropdownItem::new("off", tr!("settings.dtype.tie.off")),
    ];
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| on_change(s.to_string()))
            .class("models-active-dropdown"),
    )
}

/// KV-cache dtype для full-attention слоёв (или подобных кэшей: SSM-state,
/// conv-state). Auto = compute. Fp8E4M3 = packed bytes + per-tensor scale,
/// dequant перед matmul — экономия ~50% VRAM на KV.
pub fn kv_dtype_dropdown<F>(current: String, on_change: F) -> Box<dyn Widget>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let items = vec![
        DropdownItem::new("auto", tr!("settings.dtype.kv.auto")),
        DropdownItem::new("f16", "F16"),
        DropdownItem::new("bf16", "BF16"),
        DropdownItem::new("f32", tr!("settings.dtype.kv.f32")),
        DropdownItem::new(
            "fp8e4m3",
            "FP8 E4M3 — packed bytes (−50% VRAM, ~99% accuracy)",
        ),
    ];
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| on_change(s.to_string()))
            .class("models-active-dropdown"),
    )
}
