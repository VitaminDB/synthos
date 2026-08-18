use std::sync::Arc;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::input::Slider;
use syngui::widgets::{Column, Dropdown, DropdownItem, Reactive};
use synaptix_video_minimax_h3 as h3;

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::types::{
    DataBlob, H3Blob, H3Geometry, H3Keyframe, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, idx_in, make_slider_row};
use crate::pages::node_editor::controls::keyframe_slot::KeyframeSlot;

// ── Пропорции кадра ───────────────────────────────────────────────────────
//
// «Свободно» — ширина и высота независимы. Фиксированная пропорция
// связывает слайдеры: двигаешь ширину — высота пересчитывается (и
// наоборот), диапазон каждого слайдера сужается так, чтобы парная
// величина не вылетала за свои пределы. Портретные варианты (9:16,
// 10:16, 3:4) — вертикальное видео.

pub const ASPECT_OPTIONS: &[&str] =
    &["Свободно", "16:9", "9:16", "16:10", "10:16", "4:3", "3:4", "1:1", "21:9"];

const W_MIN: u32 = 256;
const W_MAX: u32 = 1920;
const H_MIN: u32 = 256;
const H_MAX: u32 = 1088;
const DIM_STEP: u32 = 32;

/// `(w, h)` пропорции по индексу дропдауна; `None` — «Свободно».
pub fn aspect_of(idx: usize) -> Option<(u32, u32)> {
    match ASPECT_OPTIONS.get(idx).copied() {
        Some("16:9") => Some((16, 9)),
        Some("9:16") => Some((9, 16)),
        Some("16:10") => Some((16, 10)),
        Some("10:16") => Some((10, 16)),
        Some("4:3") => Some((4, 3)),
        Some("3:4") => Some((3, 4)),
        Some("1:1") => Some((1, 1)),
        Some("21:9") => Some((21, 9)),
        _ => None,
    }
}

/// Ближайшее кратное 32 (шаг латентной сетки H3).
fn snap_dim(v: f32) -> u32 {
    ((v / DIM_STEP as f32).round().max(0.0) as u32) * DIM_STEP
}

pub fn height_from_width(w: u32, (rw, rh): (u32, u32)) -> u32 {
    snap_dim(w as f32 * rh as f32 / rw as f32).clamp(H_MIN, H_MAX)
}

pub fn width_from_height(h: u32, (rw, rh): (u32, u32)) -> u32 {
    snap_dim(h as f32 * rw as f32 / rh as f32).clamp(W_MIN, W_MAX)
}

/// Диапазон ширины при активной пропорции: сужен так, чтобы вычисленная
/// из ширины высота оставалась в [H_MIN, H_MAX].
pub fn width_range(aspect: Option<(u32, u32)>) -> (u32, u32) {
    let Some((rw, rh)) = aspect else { return (W_MIN, W_MAX) };
    let lo = ((H_MIN as f32 * rw as f32 / rh as f32) / DIM_STEP as f32).ceil() as u32 * DIM_STEP;
    let hi = ((H_MAX as f32 * rw as f32 / rh as f32) / DIM_STEP as f32).floor() as u32 * DIM_STEP;
    (lo.clamp(W_MIN, W_MAX), hi.clamp(W_MIN, W_MAX))
}

/// Симметрично [`width_range`] — диапазон высоты.
pub fn height_range(aspect: Option<(u32, u32)>) -> (u32, u32) {
    let Some((rw, rh)) = aspect else { return (H_MIN, H_MAX) };
    let lo = ((W_MIN as f32 * rh as f32 / rw as f32) / DIM_STEP as f32).ceil() as u32 * DIM_STEP;
    let hi = ((W_MAX as f32 * rh as f32 / rw as f32) / DIM_STEP as f32).floor() as u32 * DIM_STEP;
    (lo.clamp(H_MIN, H_MAX), hi.clamp(H_MIN, H_MAX))
}

pub fn geometry_of(width: u32, height: u32, seconds: f32) -> H3Geometry {
    let g = h3::pipeline::Geometry::from_duration(width as usize, height as usize, seconds as f64);
    H3Geometry {
        width: g.width,
        height: g.height,
        frame_count: g.frame_count,
        latent_t: g.latent_t,
        latent_h: g.latent_h,
        latent_w: g.latent_w,
        audio_t: g.audio_t,
    }
}

pub struct EmptyLatentExec;

impl NodeExecutor for EmptyLatentExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, .. } => {
                    let (w, h, d) = if track {
                        (width.get(), height.get(), duration_seconds.get())
                    } else {
                        (
                            width.get_untracked(),
                            height.get_untracked(),
                            duration_seconds.get_untracked(),
                        )
                    };
                    PortValue::Data(Arc::new(DataBlob::H3(H3Blob::AvLatent(geometry_of(w, h, d)))))
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("av_latent", pv);
    }
}

/// Слайдер ширины/высоты, связанный пропорцией: движение (или текстовый
/// ввод по клику на число) пересчитывает парную величину; при активной
/// пропорции диапазон слайдера сужен до валидного.
fn dim_slider(
    sig: RwSignal<u32>,
    other: RwSignal<u32>,
    aspect_idx: RwSignal<usize>,
    is_width: bool,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let aspect = aspect_of(aspect_idx.get());
        let (min, max) = if is_width { width_range(aspect) } else { height_range(aspect) };
        vec![Box::new(
            Slider::new()
                .value(sig.get().clamp(min, max) as f32)
                .range(min as f32, max as f32)
                .step(DIM_STEP as f32)
                .show_value(0)
                .on_change(move |v| {
                    let v = v.round().max(0.0) as u32;
                    sig.set(v);
                    if let Some(r) = aspect_of(aspect_idx.get_untracked()) {
                        other.set(if is_width {
                            height_from_width(v, r)
                        } else {
                            width_from_height(v, r)
                        });
                    }
                })
                .class("node-input-slider node-slider-stretch"),
        ) as Box<dyn Widget>]
    }))
}

/// Дропдаун пропорций. Выбор фиксированной пропорции сразу вгоняет ширину
/// в её суженный диапазон и пересчитывает высоту (ширина — якорь).
fn aspect_dropdown(
    aspect_idx: RwSignal<usize>,
    width: RwSignal<u32>,
    height: RwSignal<u32>,
) -> Box<dyn Widget> {
    let items: Vec<DropdownItem> =
        ASPECT_OPTIONS.iter().map(|s| DropdownItem::simple(*s)).collect();
    let current = ASPECT_OPTIONS
        .get(aspect_idx.get_untracked())
        .copied()
        .unwrap_or(ASPECT_OPTIONS[0])
        .to_string();
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let Some(i) = idx_in(ASPECT_OPTIONS, s) else { return };
                aspect_idx.set(i);
                if let Some(r) = aspect_of(i) {
                    let (w_min, w_max) = width_range(Some(r));
                    let w = width.get_untracked().clamp(w_min, w_max);
                    width.set(w);
                    height.set(height_from_width(w, r));
                }
            })
            .class("node-input-dropdown"),
    )
}

pub fn empty_latent_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds, aspect_idx } => {
                Some((*width, *height, *duration_seconds, *aspect_idx))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((width, height, duration, aspect_idx)) = snapshot else {
        return Box::new(Column::new());
    };
    let info = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let g = geometry_of(width.get(), height.get(), duration.get());
        vec![Box::new(
            Text::new(format!(
                "{} кадров @24fps, латент {}×{}×{}, аудио {}",
                g.frame_count, g.latent_t, g.latent_h, g.latent_w, g.audio_t
            ))
            .class("h3-node-info"),
        )]
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                field_row("Пропорции", aspect_dropdown(aspect_idx, width, height)),
                field_row("Ширина", dim_slider(width, height, aspect_idx, true)),
                field_row("Высота", dim_slider(height, width, aspect_idx, false)),
                field_row("Длительность, с", make_slider_row(duration, 1.0, 15.0, 0.5, 1)),
                Box::new(info),
            ]),
    )
}

pub struct KeyframeExec;

impl NodeExecutor for KeyframeExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3Keyframe { image, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match image.lock().ok().and_then(|g| g.clone()) {
                        Some(kf) => PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Keyframe(kf)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("keyframe", pv);
    }
}

pub fn keyframe_on_run(node: &NodeInstance, _ctx: &super::super::super::state::NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Keyframe {
                path,
                frame_slot_idx,
                image,
                error,
                output_version,
                ..
            } => Some((*path, *frame_slot_idx, image.clone(), *error, *output_version)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, slot, image, error, output_version)) = snapshot else {
        return;
    };
    let Some(p) = path.get_untracked() else {
        error.set(Some("выберите изображение".into()));
        return;
    };
    match synaptix_io::image::png::load_image(&p, synaptix_core::device::Device::Cpu) {
        Ok(img) => {
            let frame_index = if slot.get_untracked() == 1 { usize::MAX } else { 0 };
            if let Ok(mut g) = image.lock() {
                *g = Some(Arc::new(H3Keyframe { image: img, frame_index }));
            }
            error.set(None);
            output_version.update(|v| *v = v.wrapping_add(1));
        }
        Err(e) => error.set(Some(format!("не удалось открыть {}: {e}", p.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_16_9_from_default_width() {
        // Дефолт ноды 1344×768 — ровно 16:9 после снапа к 32.
        assert_eq!(height_from_width(1344, (16, 9)), 768);
        assert_eq!(width_from_height(768, (16, 9)), 1376); // 768*16/9=1365 → 1376
    }

    #[test]
    fn portrait_ranges_stay_within_bounds() {
        // 9:16 (вертикальное видео): ширина сужается так, чтобы высота
        // не превысила 1088.
        let (lo, hi) = width_range(Some((9, 16)));
        assert_eq!((lo, hi), (256, 608));
        assert!(height_from_width(hi, (9, 16)) <= 1088);
        assert!(height_from_width(lo, (9, 16)) >= 256);
    }

    #[test]
    fn square_range() {
        assert_eq!(width_range(Some((1, 1))), (256, 1088));
        assert_eq!(height_range(Some((1, 1))), (256, 1088));
    }

    #[test]
    fn free_mode_full_ranges() {
        assert_eq!(width_range(None), (256, 1920));
        assert_eq!(height_range(None), (256, 1088));
        assert!(aspect_of(0).is_none());
    }

    #[test]
    fn all_aspect_options_resolve() {
        for (i, name) in ASPECT_OPTIONS.iter().enumerate().skip(1) {
            let r = aspect_of(i).unwrap_or_else(|| panic!("нет пропорции для {name}"));
            let (lo, hi) = width_range(Some(r));
            assert!(lo <= hi, "{name}: пустой диапазон ширины {lo}..{hi}");
            let (hlo, hhi) = height_range(Some(r));
            assert!(hlo <= hhi, "{name}: пустой диапазон высоты {hlo}..{hhi}");
            // Связка из середины диапазона держит обе величины в пределах.
            let w = ((lo + hi) / 2 / DIM_STEP) * DIM_STEP;
            let h = height_from_width(w, r);
            assert!((H_MIN..=H_MAX).contains(&h), "{name}: {w} → {h}");
        }
    }
}

pub fn keyframe_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3Keyframe { path, frame_slot_idx, resize_idx, error, .. } => {
                Some((*path, *frame_slot_idx, *resize_idx, *error))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, frame_slot_idx, resize_idx, error)) = snapshot else {
        return Box::new(Column::new());
    };
    KeyframeSlot::new(path, frame_slot_idx, resize_idx)
        .error(error)
        .build()
}
