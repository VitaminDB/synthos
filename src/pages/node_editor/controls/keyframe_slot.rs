use std::path::PathBuf;

use syngui::prelude::*;
use syngui::widgets::visual::{Image, ImageFit};
use syngui::widgets::{Column, Row};

use super::dropdown_field::node_dropdown_field;
use super::field_row::node_field_row;
use super::file_picker::node_file_picker;

pub const FRAME_SLOTS: &[&str] = &["первый кадр", "последний кадр"];
pub const RESIZE_MODES: &[&str] = &["растянуть", "кроп по центру"];

pub struct KeyframeSlot {
    path: RwSignal<Option<PathBuf>>,
    slot_idx: RwSignal<usize>,
    resize_idx: RwSignal<usize>,
    error: Option<RwSignal<Option<String>>>,
    preview_class: &'static str,
    show_resize: bool,
}

impl KeyframeSlot {
    pub fn new(
        path: RwSignal<Option<PathBuf>>,
        slot_idx: RwSignal<usize>,
        resize_idx: RwSignal<usize>,
    ) -> Self {
        Self {
            path,
            slot_idx,
            resize_idx,
            error: None,
            preview_class: "h3-keyframe-thumb",
            show_resize: true,
        }
    }

    pub fn error(mut self, sig: RwSignal<Option<String>>) -> Self {
        self.error = Some(sig);
        self
    }

    pub fn preview_class(mut self, c: &'static str) -> Self {
        self.preview_class = c;
        self
    }

    pub fn show_resize(mut self, on: bool) -> Self {
        self.show_resize = on;
        self
    }

    pub fn build(self) -> Box<dyn Widget> {
        let path = self.path;
        let slot_idx = self.slot_idx;
        let resize_idx = self.resize_idx;
        let error = self.error;
        let preview_class = self.preview_class;

        let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
            match path.get() {
                Some(p) => vec![Box::new(
                    Image::new(p.to_string_lossy().to_string())
                        .fit(ImageFit::Cover)
                        .class(preview_class),
                )],
                None => vec![Box::new(
                    Text::new("кадр не выбран").class("h3-node-info"),
                )],
            }
        });

        let badge = Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let i = slot_idx.get().min(FRAME_SLOTS.len() - 1);
            vec![Box::new(
                Text::new(format!("<Picture {}> · {}", i + 1, FRAME_SLOTS[i]))
                    .class("h3-keyframe-badge"),
            )]
        });

        let mut rows: Vec<Box<dyn Widget>> = vec![
            Box::new(thumb),
            Box::new(badge),
            node_file_picker(
                "Кадр-якорь (PNG/JPEG)",
                path,
                &[("Изображения", &["png", "jpg", "jpeg", "webp"])],
                |_| {},
            ),
            node_field_row("Слот", node_dropdown_field(FRAME_SLOTS, slot_idx)),
        ];
        if self.show_resize {
            rows.push(node_field_row("Ресайз", node_dropdown_field(RESIZE_MODES, resize_idx)));
        }
        if let Some(e) = error {
            rows.push(Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                match e.get() {
                    Some(msg) => {
                        vec![Box::new(Text::new(format!("Ошибка: {msg}")).class("audio-node-error"))]
                    }
                    None => vec![],
                }
            })));
        }
        Box::new(Column::new().gap(3.0).children(rows))
    }
}

pub struct ReferenceTray {
    entries: Vec<ReferenceEntry>,
    thumb_class: &'static str,
}

#[derive(Clone)]
pub struct ReferenceEntry {
    pub path: RwSignal<Option<PathBuf>>,
    pub kind: ReferenceKind,
    pub ordinal: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    Image,
    Video,
    Audio,
}

impl ReferenceKind {
    pub fn tag(self, ordinal: usize) -> String {
        match self {
            ReferenceKind::Image => format!("<Picture {ordinal}>"),
            ReferenceKind::Video => format!("<Video {ordinal}>"),
            ReferenceKind::Audio => format!("<Audio {ordinal}>"),
        }
    }

    pub fn filters(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            ReferenceKind::Image => &[("Изображения", &["png", "jpg", "jpeg", "webp"])],
            ReferenceKind::Video => &[("Видео", &["mp4", "mov", "mkv", "webm"])],
            ReferenceKind::Audio => &[("Аудио", &["wav", "mp3", "flac", "ogg"])],
        }
    }
}

impl Default for ReferenceTray {
    fn default() -> Self {
        Self::new()
    }
}

impl ReferenceTray {
    pub fn new() -> Self {
        Self { entries: Vec::new(), thumb_class: "h3-ref-thumb" }
    }

    pub fn entry(mut self, e: ReferenceEntry) -> Self {
        self.entries.push(e);
        self
    }

    pub fn thumb_class(mut self, c: &'static str) -> Self {
        self.thumb_class = c;
        self
    }

    pub fn build(self) -> Box<dyn Widget> {
        let thumb_class = self.thumb_class;
        let cells: Vec<Box<dyn Widget>> = self
            .entries
            .into_iter()
            .map(|e| {
                let path = e.path;
                let kind = e.kind;
                let ordinal = e.ordinal;
                let visual = Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    match (kind, path.get()) {
                        (ReferenceKind::Image, Some(p)) => vec![Box::new(
                            Image::new(p.to_string_lossy().to_string())
                                .fit(ImageFit::Cover)
                                .class(thumb_class),
                        )],
                        (_, Some(p)) => vec![Box::new(
                            Text::new(
                                p.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default(),
                            )
                            .class("h3-node-info"),
                        )],
                        (_, None) => {
                            vec![Box::new(Text::new("пусто").class("h3-node-info"))]
                        }
                    }
                });
                let visual_w: Box<dyn Widget> = Box::new(visual);
                let badge_w: Box<dyn Widget> =
                    Box::new(Text::new(kind.tag(ordinal)).class("h3-keyframe-badge"));
                let picker_w = node_file_picker("Референс", path, kind.filters(), |_| {});
                let cell: Box<dyn Widget> =
                    Box::new(Column::new().gap(2.0).children(vec![visual_w, badge_w, picker_w]));
                cell
            })
            .collect();
        Box::new(Row::new().gap(6.0).children(cells))
    }
}
