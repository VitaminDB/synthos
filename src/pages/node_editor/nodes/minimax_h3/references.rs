//! Нода «H3 References» — упорядоченный список референсов партиции Ref2VA.
//!
//! Один список вместо трёх разнотипных: порядок референсов — часть запроса
//! (номера `<Picture i>` / `<Video k>` / `<Audio j>` и RoPE-часы раскладки),
//! а агенту чата достаточно одного `set_state` с путями по порядку.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use syngui::async_runtime::run_on_main_thread;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Column, Row};
use synaptix_video_minimax_h3::refs::{self, RefImageSize, RefKind, RefOptions, RefSource};

use super::super::super::eval::{EvalContext, NodeExecutor};
use super::super::super::state::NodeEditorCtx;
use super::super::super::types::{
    DataBlob, H3Blob, H3Geometry, H3RefEntry, H3Refs, NodeInstance, NodeRuntime, PortValue,
};
use super::super::acestep::{field_row, make_dropdown, status_row};
use super::super::{log_worker_done, log_worker_start};
use super::current_input_av_latent;
use crate::icons::{
    MI_ADD, MI_ARROW_DOWNWARD, MI_ARROW_UPWARD, MI_AUDIOTRACK, MI_CLOSE, MI_IMAGE_ICON, MI_MOVIE,
    MI_VOLUME_OFF, MI_VOLUME_UP,
};

pub const IMAGE_SIZE_OPTIONS: &[&str] = &["match", "max"];

pub fn image_size_of(idx: usize) -> RefImageSize {
    match idx {
        1 => RefImageSize::Max,
        _ => RefImageSize::Match,
    }
}

impl H3RefEntry {
    /// Строка списка по пути: тип — по расширению, наличие дорожки — пробой
    /// файла. `None`, если по расширению не понять, что это.
    pub fn probe(path: impl Into<PathBuf>, use_audio: bool) -> Option<Self> {
        let path = path.into();
        let kind = RefKind::of_path(&path)?;
        let has_audio = kind == RefKind::Video && refs::has_audio_stream(&path);
        Some(Self { path, kind, use_audio, has_audio })
    }

    /// Войдёт ли звук этого референса в запрос своей меткой `<Audio j>`.
    pub fn carries_audio(&self) -> bool {
        match self.kind {
            RefKind::Image => false,
            RefKind::Audio => true,
            RefKind::Video => self.use_audio && self.has_audio,
        }
    }

    pub fn source(&self) -> RefSource {
        match self.kind {
            RefKind::Image => RefSource::Image(self.path.clone()),
            RefKind::Audio => RefSource::Audio(self.path.clone()),
            RefKind::Video => {
                RefSource::Video { path: self.path.clone(), use_audio: self.carries_audio() }
            }
        }
    }
}

/// Метки, под которыми модель увидит референсы, в порядке списка — их и надо
/// писать в промпте.
pub fn labels_of(items: &[H3RefEntry]) -> Vec<String> {
    refs::labels(&items.iter().map(|e| (e.kind, e.carries_audio())).collect::<Vec<_>>())
}

pub struct ReferencesExec;

impl NodeExecutor for ReferencesExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let _ = ctx.read_input("av_latent");
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::H3References { out, output_version, .. } => {
                    if track {
                        let _ = output_version.get();
                    }
                    match out.lock().ok().and_then(|g| g.clone()) {
                        Some(r) => PortValue::Data(Arc::new(DataBlob::H3(H3Blob::Refs(r)))),
                        None => PortValue::Empty,
                    }
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("refs", pv);
    }
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3References {
                items,
                image_size_idx,
                running,
                error,
                loaded_name,
                out,
                output_version,
            } => Some((
                *items,
                *image_size_idx,
                *running,
                *error,
                *loaded_name,
                out.clone(),
                *output_version,
            )),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((items, image_size_idx, running, error, loaded_name, out, output_version)) = snapshot
    else {
        return;
    };
    if running.get_untracked() {
        return;
    }
    let entries = items.get_untracked();
    if entries.is_empty() {
        error.set(Some(tr!("node.minimax_h3_references.empty")));
        return;
    }
    let Some(geometry) = current_input_av_latent(ctx, node.id, "av_latent") else {
        error.set(Some(tr!("node.minimax_h3_references.connect_empty_latent")));
        return;
    };
    let image_size = image_size_of(image_size_idx.get_untracked());

    running.set(true);
    error.set(None);
    loaded_name.set(None);

    let _ = thread::Builder::new().name("synthos-h3-refs".into()).spawn(move || {
        let started = log_worker_start(
            "h3-references",
            &format!("{} шт., под {} кадров", entries.len(), geometry.frame_count),
        );
        let res = worker(&entries, geometry, image_size);
        log_worker_done("h3-references", started, &res);
        match res {
            Ok(decoded) => {
                let summary = decoded.labels.join(" · ");
                if let Ok(mut g) = out.lock() {
                    *g = Some(Arc::new(decoded));
                }
                error.set(None);
                loaded_name.set(Some(summary));
                run_on_main_thread(move || output_version.update(|v| *v = v.wrapping_add(1)));
            }
            Err(e) => error.set(Some(e)),
        }
        running.set(false);
    });
}

fn worker(
    entries: &[H3RefEntry],
    geometry: H3Geometry,
    image_size: RefImageSize,
) -> std::result::Result<H3Refs, String> {
    let sources: Vec<RefSource> = entries.iter().map(H3RefEntry::source).collect();
    let media = refs::decode(
        &sources,
        &RefOptions {
            image_size,
            target_width: geometry.width,
            target_height: geometry.height,
            frame_count: geometry.frame_count,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(H3Refs { media, labels: labels_of(entries), frame_count: geometry.frame_count })
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3References { running, .. } => Some(*running),
            _ => None,
        },
        Err(_) => None,
    }
}

fn kind_icon(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Image => MI_IMAGE_ICON,
        RefKind::Video => MI_MOVIE,
        RefKind::Audio => MI_AUDIOTRACK,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn item_row(
    items: RwSignal<Vec<H3RefEntry>>,
    index: usize,
    count: usize,
    entry: &H3RefEntry,
    label: &str,
) -> Box<dyn Widget> {
    let mut cells: Vec<Box<dyn Widget>> = vec![
        Box::new(Icon::new(kind_icon(entry.kind)).class("h3-ref-kind-icon")),
        Box::new(
            Column::new()
                .gap(1.0)
                .children(vec![
                    Box::new(Text::new(label).class("h3-keyframe-badge")) as Box<dyn Widget>,
                    Box::new(Text::new(file_name(&entry.path)).class("h3-ref-name")),
                ])
                .class("h3-ref-text"),
        ),
    ];
    if entry.kind == RefKind::Video && entry.has_audio {
        let on = entry.use_audio;
        cells.push(Box::new(
            ToolButton::new(if on { MI_VOLUME_UP } else { MI_VOLUME_OFF })
                .tooltip(if on {
                    tr!("node.minimax_h3_references.audio_on")
                } else {
                    tr!("node.minimax_h3_references.audio_off")
                })
                .on_click(move || {
                    items.update(|v| {
                        if let Some(e) = v.get_mut(index) {
                            e.use_audio = !e.use_audio;
                        }
                    })
                })
                .class("node-file-picker-btn"),
        ));
    }
    if index > 0 {
        cells.push(Box::new(
            ToolButton::new(MI_ARROW_UPWARD)
                .tooltip(tr!("node.minimax_h3_references.move_up"))
                .on_click(move || items.update(|v| v.swap(index, index - 1)))
                .class("node-file-picker-btn"),
        ));
    }
    if index + 1 < count {
        cells.push(Box::new(
            ToolButton::new(MI_ARROW_DOWNWARD)
                .tooltip(tr!("node.minimax_h3_references.move_down"))
                .on_click(move || items.update(|v| v.swap(index, index + 1)))
                .class("node-file-picker-btn"),
        ));
    }
    cells.push(Box::new(
        ToolButton::new(MI_CLOSE)
            .tooltip(tr!("node.minimax_h3_references.remove"))
            .on_click(move || {
                items.update(|v| {
                    if index < v.len() {
                        v.remove(index);
                    }
                })
            })
            .class("node-file-picker-btn"),
    ));
    Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(cells)
            .class("h3-ref-row"),
    )
}

fn pick_and_add(items: RwSignal<Vec<H3RefEntry>>, error: RwSignal<Option<String>>) {
    let picked = rfd::FileDialog::new()
        .set_title(tr!("node.minimax_h3_references.add"))
        .add_filter(
            tr!("node.minimax_h3_references.filter"),
            &[
                "png", "jpg", "jpeg", "webp", "bmp", "heic", "mp4", "mov", "mkv", "webm", "m4v",
                "wav", "mp3", "flac", "ogg", "m4a",
            ],
        )
        .pick_files();
    let Some(paths) = picked else { return };
    let mut next = items.get_untracked();
    for p in paths {
        match H3RefEntry::probe(&p, true) {
            Some(e) => next.push(e),
            None => {
                error.set(Some(tr!("node.minimax_h3_references.unknown_type", path = p.display())));
                return;
            }
        }
    }
    let sources: Vec<RefSource> = next.iter().map(H3RefEntry::source).collect();
    match refs::validate(&sources) {
        Ok(()) => {
            error.set(None);
            items.set(next);
        }
        Err(e) => error.set(Some(e.to_string())),
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let snapshot = match node.runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::H3References {
                items, image_size_idx, running, error, loaded_name, ..
            } => Some((*items, *image_size_idx, *running, *error, *loaded_name)),
            _ => None,
        },
        Err(_) => None,
    };
    let Some((items, image_size_idx, running, error, loaded_name)) = snapshot else {
        return Box::new(Column::new());
    };

    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let entries = items.get();
        if entries.is_empty() {
            return vec![Box::new(
                Text::new(tr!("node.minimax_h3_references.hint")).class("h3-node-info"),
            )];
        }
        let labels = labels_of(&entries);
        entries
            .iter()
            .zip(&labels)
            .enumerate()
            .map(|(i, (e, label))| item_row(items, i, entries.len(), e, label))
            .collect()
    });

    let footer = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let n = items.get().len();
        let mut cells: Vec<Box<dyn Widget>> = Vec::new();
        if n < refs::MAX_REFS {
            cells.push(Box::new(
                ToolButton::new(MI_ADD)
                    .tooltip(tr!("node.minimax_h3_references.add"))
                    .on_click(move || pick_and_add(items, error))
                    .class("node-file-picker-btn"),
            ));
        }
        cells.push(Box::new(
            Text::new(format!("{n}/{}", refs::MAX_REFS)).class("h3-node-info"),
        ));
        vec![Box::new(
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).children(cells),
        )]
    });

    Box::new(
        Column::new()
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(list) as Box<dyn Widget>,
                Box::new(footer),
                field_row(
                    &tr!("node.minimax_h3_references.image_size"),
                    make_dropdown(IMAGE_SIZE_OPTIONS, image_size_idx),
                ),
                field_row(
                    &tr!("nodes.common.status"),
                    status_row(
                        running,
                        error,
                        loaded_name,
                        tr!("node.minimax_h3_references.busy"),
                        "h3-node-running",
                    ),
                ),
            ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: RefKind, use_audio: bool, has_audio: bool) -> H3RefEntry {
        H3RefEntry { path: path.into(), kind, use_audio, has_audio }
    }

    /// Дорожка видео получает свой `<Audio j>` только если её просили И она в
    /// файле есть — иначе следующие аудио нумеруются без неё.
    #[test]
    fn labels_follow_effective_soundtracks() {
        let items = [
            entry("a.mp4", RefKind::Video, true, true),
            entry("b.mp4", RefKind::Video, true, false),
            entry("c.mp4", RefKind::Video, false, true),
            entry("v.wav", RefKind::Audio, true, false),
            entry("p.png", RefKind::Image, true, false),
        ];
        assert_eq!(
            labels_of(&items),
            ["<Video 1> + <Audio 1>", "<Video 2>", "<Video 3>", "<Audio 2>", "<Picture 1>"]
        );
        assert_eq!(
            items[1].source(),
            RefSource::Video { path: "b.mp4".into(), use_audio: false }
        );
    }

    #[test]
    fn unknown_extension_is_not_a_reference() {
        assert!(H3RefEntry::probe("notes.txt", true).is_none());
        let img = H3RefEntry::probe("/nonexistent/hero.png", true).unwrap();
        assert_eq!(img.kind, RefKind::Image);
        assert!(!img.carries_audio());
    }
}
