//! Таб «Просмотр»: содержимое выбранного файла внутри пакета.
//!
//! Routing по расширению:
//! - текстовые → `CodeEditor.read_only(true).auto_detect_language(path)`
//! - изображения → `Image::from_bytes`
//! - бинарные → hex-dump первых 4 KB через `Text`

use std::path::Path;

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::input::code_editor::CodeEditor;
use syngui::widgets::visual::image::{Image, ImageFit};

use super::bundle_io;
use super::state::{OpenBundle, SynExplorerCtx};

pub fn view(active: OpenBundle) -> impl Widget {
    DecoratedBox::new().class("syn-tab-pane syn-tab-preview").child(Reactive::new(
        move || -> Vec<Box<dyn Widget>> {
            let _ = active.reload_gen.get();
            let _ = use_context::<SynExplorerCtx>().active_bundle.get();
            let Some(name) = active.selected_path.get() else {
                return vec![Box::new(empty_placeholder())];
            };
            // Папки и tombstoned-файлы — нечего показать.
            let is_alive_file = active
                .files
                .get()
                .iter()
                .any(|f| f.alive && f.name == name);
            if !is_alive_file {
                return vec![Box::new(empty_placeholder())];
            }

            // Lazy-load payload в preview_cache. Первый клик → блокирующий
            // read через mmap (быстро); повторные — из кеша.
            let bytes = {
                let cache = active.preview_cache.get();
                cache.get(&name).cloned()
            };
            let bytes = match bytes {
                Some(b) => b,
                None => {
                    let path = active.path.get_untracked();
                    let loaded = bundle_io::read_file_owned(&path, &name);
                    match loaded {
                        Some(b) => {
                            let name_for_insert = name.clone();
                            let b_clone = b.clone();
                            active.preview_cache.update(move |m| {
                                m.insert(name_for_insert, b_clone);
                            });
                            b
                        }
                        None => {
                            return vec![Box::new(error_placeholder(&tr!(
                                "explorer.preview.read_failed",
                                name = name
                            )))];
                        }
                    }
                }
            };

            vec![render_preview(&name, &bytes)]
        },
    ))
}

fn render_preview(name: &str, bytes: &[u8]) -> Box<dyn Widget> {
    let path = Path::new(name);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    // Изображения через syngui::Image::from_bytes — uses встроенный image-crate.
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
    ) {
        return Box::new(
            DecoratedBox::new().class("syn-preview-frame syn-preview-image").child(
                Image::from_bytes(name.to_string(), bytes.to_vec())
                    .fit(ImageFit::Contain),
            ),
        );
    }

    // Текстовый файл: пробуем как UTF-8. Если ок — CodeEditor read-only с
    // подсветкой по расширению.
    if let Ok(text) = std::str::from_utf8(bytes) {
        // Cap размера preview: 2 MB (на больших JSON/токенайзерах CodeEditor
        // подвисает на инициализации, а размер пользователю всё равно
        // видится в табе Files).
        const PREVIEW_CAP: usize = 2 * 1024 * 1024;
        let text_str = if text.len() > PREVIEW_CAP {
            format!(
                "{}\n\n{}",
                &text[..PREVIEW_CAP],
                tr!("explorer.preview.truncated", shown = PREVIEW_CAP, total = text.len())
            )
        } else {
            text.to_string()
        };
        return Box::new(
            DecoratedBox::new().class("syn-preview-frame syn-preview-text").child(
                CodeEditor::new()
                    .text(text_str)
                    .auto_detect_language(path)
                    .read_only(true)
                    .show_line_numbers(true)
                    .soft_wrap(true)
                    .class("syn-preview-codeeditor"),
            ),
        );
    }

    // Иначе — hex-dump первых 4 KB.
    let dump = hex_dump(bytes, 4096);
    Box::new(
        DecoratedBox::new().class("syn-preview-frame syn-preview-hex").child(mgui! {
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(trn!("explorer.preview.binary_header", bytes.len()))
                        .class("syn-preview-hex-header"),
                    Text::new(dump).class("syn-preview-hex-body"),
                ]
        }),
    )
}

fn empty_placeholder() -> impl Widget {
    mgui! {
        Center::new().child(
            Text::new(tr!("explorer.preview.empty_hint"))
                .class("syn-empty-hint"),
        )
    }
}

fn error_placeholder(msg: &str) -> impl Widget {
    let msg = msg.to_string();
    mgui! {
        Center::new().child(
            Text::new(msg).class("syn-empty-hint"),
        )
    }
}

fn hex_dump(bytes: &[u8], cap: usize) -> String {
    let take = cap.min(bytes.len());
    let mut out = String::with_capacity(take * 4);
    for (i, chunk) in bytes[..take].chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", i * 16));
        for b in chunk {
            out.push_str(&format!("{:02x} ", b));
        }
        // Padding до 16 столбцов
        for _ in chunk.len()..16 {
            out.push_str("   ");
        }
        out.push_str(" |");
        for b in chunk {
            let ch = *b;
            if (0x20..0x7f).contains(&ch) {
                out.push(ch as char);
            } else {
                out.push('.');
            }
        }
        out.push('|');
        out.push('\n');
    }
    if bytes.len() > cap {
        out.push('\n');
        out.push_str(&tr!("explorer.preview.hex_truncated", shown = cap, total = bytes.len()));
    }
    out
}
