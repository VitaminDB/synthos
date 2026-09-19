//! Картинки на проводе: Image (файл → `image`) и Image Save (`image` →
//! PNG/JPEG/WebP). Общий тип — [`ImageData`]; его отдают FLUX VAE Decode и
//! Image, принимают FLUX VAE Encode, LTX Image, H3 Keyframe и Image Save.

use std::result::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::visual::{Image, ImageFit};
use syngui::widgets::{Column, Reactive};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::state::NodeEditorCtx;
use super::super::types::{DataBlob, ImageData, NodeInstance, NodeRuntime, PortValue};
use super::acestep::field_row;
use super::{log_worker_done, log_worker_start};
use crate::pages::node_editor::controls::file_picker::node_file_picker;

pub const IMAGE_FILTER: &[(&str, &[&str])] = &[("nodes.filter.images", &["png", "jpg", "jpeg", "webp", "bmp"])];
const SAVE_FILTER: &[(&str, &[&str])] = &[("nodes.filter.images", &["png", "jpg", "jpeg", "webp"])];

/// Превью картинки в теле ноды; без картинки — подсказка.
pub fn image_preview(img: Option<Arc<ImageData>>) -> Box<dyn Widget> {
    match img {
        Some(img) => Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![
                    Box::new(
                        Image::from_rgba(img.key.clone(), img.width, img.height, img.rgba.as_ref().clone())
                            .fit(ImageFit::Contain)
                            .class("image-node-preview"),
                    ) as Box<dyn Widget>,
                    Box::new(Text::new(format!("{}×{}", img.width, img.height)).class("flux-node-info")),
                ]),
        ),
        None => Box::new(Text::new(tr!("node.image.no_image")).class("flux-node-info")),
    }
}

/// Картинка со входа `port` (для LTX Image / H3 Keyframe / Image Save).
pub fn current_input_image(ctx: &NodeEditorCtx, node_id: super::super::types::NodeId, port: &'static str) -> Option<Arc<ImageData>> {
    let conns = ctx.connections.get_untracked();
    let src = conns.iter().find(|c| c.to_node == node_id && c.to_port == port)?.clone();
    ctx.values.get_untracked().get(&(src.from_node, src.from_port)).and_then(|v| v.as_image())
}

// ── Image (загрузка) ──────────────────────────────────────────────────────

pub struct ImageLoadExec;

impl NodeExecutor for ImageLoadExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::ImageLoad { path, error, cache } => {
                    let p = if track { path.get() } else { path.get_untracked() };
                    match p {
                        Some(p) => match load_cached(&p, cache) {
                            Ok(img) => {
                                if error.get_untracked().is_some() {
                                    error.set(None);
                                }
                                PortValue::Data(Arc::new(DataBlob::Image(img)))
                            }
                            Err(e) => {
                                error.set(Some(tr!("node.image_load.open_failed", path = p.display(), error = e)));
                                PortValue::Empty
                            }
                        },
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("image", pv);
    }
}

/// Тот же `Arc`, пока путь не сменился: провод не дёргает потребителей
/// на каждом пересчёте графа.
fn load_cached(path: &PathBuf, cache: &Arc<syngui::core::sync::Mutex<Option<Arc<ImageData>>>>) -> Result<Arc<ImageData>, String> {
    if let Ok(g) = cache.lock() {
        if let Some(img) = g.as_ref().filter(|i| i.source.as_ref() == Some(path)) {
            return Ok(img.clone());
        }
    }
    let img = Arc::new(ImageData::load(path)?);
    if let Ok(mut g) = cache.lock() {
        *g = Some(img.clone());
    }
    Ok(img)
}

pub fn load_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::ImageLoad { path, error, cache } => Some((*path, *error, cache.clone())),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, error, cache)) = snapshot else {
        return Box::new(Column::new());
    };
    let preview = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let p = path.get();
        let _ = error.get();
        let img = cache.lock().ok().and_then(|g| g.clone()).filter(|i| i.source == p);
        vec![image_preview(img)]
    });
    let status = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        match error.get() {
            Some(e) => vec![Box::new(Text::new(tr!("nodes.common.error", error = e)).class("audio-node-error"))],
            None => vec![],
        }
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(preview),
                field_row(
                    &tr!("node.image_load.file"),
                    node_file_picker(tr!("node.image_load.picker"), path, IMAGE_FILTER, |_| {}),
                ),
                Box::new(status),
            ]),
    )
}

// ── Image Save ────────────────────────────────────────────────────────────

pub struct ImageSaveExec;

impl NodeExecutor for ImageSaveExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("image");
        ctx.write_output("path", PortValue::Empty);
    }
}

pub fn save_on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::ImageSave { path, running, error, saved, preview, preview_version } => {
                Some((*path, *running, *error, *saved, preview.clone(), *preview_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, saved, preview, preview_version)) = snapshot else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let Some(img) = current_input_image(ctx, node.id, "image") else {
        error.set(Some(tr!("node.image_save.connect_image")));
        return;
    };
    let out = path.get_untracked().unwrap_or_else(|| PathBuf::from("image.png"));
    if let Ok(mut g) = preview.lock() {
        *g = Some(img.clone());
    }
    preview_version.update(|v| *v = v.wrapping_add(1));

    running.set(true);
    error.set(None);
    saved.set(None);
    let _ = thread::Builder::new().name("synthos-image-save".into()).spawn(move || {
        let started = log_worker_start("image-save", &format!("{}x{} → {}", img.width, img.height, out.display()));
        let res = write_image(&img, &out);
        log_worker_done("image-save", started, &res);
        match res {
            Ok(()) => saved.set(Some(out.display().to_string())),
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

/// Формат — по расширению; без расширения — PNG.
pub fn write_image(img: &ImageData, out: &std::path::Path) -> Result<(), String> {
    let out = if out.extension().is_none() { out.with_extension("png") } else { out.to_path_buf() };
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    synaptix_io::image::save_image(&img.tensor, &out).map_err(|e| e.to_string())
}

pub fn save_busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::ImageSave { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

pub fn save_body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::ImageSave { path, running, error, saved, preview, preview_version } => {
                Some((*path, *running, *error, *saved, preview.clone(), *preview_version))
            }
            _ => None,
        },
        Err(_) => None,
    };
    let Some((path, running, error, saved, preview, preview_version)) = snapshot else {
        return Box::new(Column::new());
    };
    let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = preview_version.get();
        match preview.lock().ok().and_then(|g| g.clone()) {
            Some(img) => vec![image_preview(Some(img))],
            None => vec![],
        }
    });
    let status = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if let Some(msg) = error.get() {
            return vec![Box::new(Text::new(tr!("nodes.common.error", error = msg)).class("audio-node-error"))];
        }
        if running.get() {
            return vec![Box::new(Text::new(tr!("node.minimax_h3_save.saving")).class("flux-node-running"))];
        }
        match saved.get() {
            Some(p) => vec![Box::new(Text::new(tr!("node.minimax_h3_save.saved", path = p)).class("flux-node-info"))],
            None => vec![],
        }
    });
    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(thumb),
                field_row(
                    &tr!("node.minimax_h3_save.file_label"),
                    node_file_picker(tr!("node.image_save.picker"), path, SAVE_FILTER, |_| {}),
                ),
                Box::new(status),
            ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_round_trips_through_png() {
        synaptix_kernels_cpu::ensure_registered();
        let t = synaptix_core::tensor::Tensor::from_vec(
            (0..3 * 4 * 6).map(|i| (i % 7) as f32 / 6.0).collect::<Vec<f32>>(),
            (3, 4, 6),
            synaptix_core::device::Device::Cpu,
        )
        .unwrap();
        let img = ImageData::from_tensor(t, None).unwrap();
        assert_eq!((img.width, img.height), (6, 4));
        assert_eq!(img.rgba.len(), 6 * 4 * 4);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/out");
        write_image(&img, &p).unwrap();
        let back = ImageData::load(&dir.path().join("sub/out.png")).unwrap();
        assert_eq!((back.width, back.height), (6, 4));
        assert_eq!(back.rgba, img.rgba, "PNG 8 бит — те же байты");
        assert_ne!(back.key, img.key, "ключ текстуры уникален");
    }
}
