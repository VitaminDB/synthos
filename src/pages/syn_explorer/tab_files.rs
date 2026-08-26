//! Таб «Файлы»: TableView со всеми чанками пакета.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{TableColumn, TableView};
use synaptix_bundle::{ChunkType, FileTag};

use super::state::{FileEntryView, OpenBundle, SynExplorerCtx};

pub fn view(active: OpenBundle) -> impl Widget {
    DecoratedBox::new().class("syn-tab-pane syn-tab-files").child(Reactive::new(
        move || -> Vec<Box<dyn Widget>> {
            let _ = active.reload_gen.get();
            let files = active.files.get();
            let _ = use_context::<SynExplorerCtx>().active_bundle.get();

            if files.is_empty() {
                return vec![Box::new(empty_placeholder())];
            }

            let columns = vec![
                TableColumn::flex(tr!("explorer.files.column.name"), 3.0),
                TableColumn::fixed(tr!("explorer.files.column.type"), 120.0),
                TableColumn::fixed(tr!("explorer.files.column.size"), 110.0),
                TableColumn::fixed("Tag", 90.0),
                TableColumn::fixed("CRC32C", 100.0),
                TableColumn::fixed("SHA-256", 110.0),
                TableColumn::fixed(tr!("explorer.files.column.status"), 100.0),
            ];

            let rows: Vec<Vec<String>> = files
                .iter()
                .map(|f| row_for_entry(f))
                .collect();

            // Сохраним alive-карту + имена для on_row_click → selected_path.
            let names: Vec<(String, bool)> = files
                .iter()
                .map(|f| (f.name.clone(), f.alive))
                .collect();
            let active_for_cb = active;

            let table = TableView::new(columns, rows)
                .striped(true)
                .row_height(28.0)
                .header_height(32.0)
                .on_row_click(move |idx| {
                    if let Some((name, alive)) = names.get(idx) {
                        if *alive {
                            active_for_cb.selected_path.set(Some(name.clone()));
                        }
                    }
                })
                .class("syn-files-table");

            vec![Box::new(table)]
        },
    ))
}

fn row_for_entry(f: &FileEntryView) -> Vec<String> {
    vec![
        f.name.clone(),
        chunk_type_label(f.kind),
        humanize_bytes(f.size),
        tag_label(f.tag),
        format!("{:08x}", f.crc32c),
        f.sha256_short.clone().unwrap_or_else(|| "—".to_string()),
        if f.alive { "alive".to_string() } else { "tombstone".to_string() },
    ]
}

fn chunk_type_label(t: ChunkType) -> String {
    match t {
        ChunkType::Tensors => "Tensors".to_string(),
        ChunkType::QuantizedTensors => "Quantized".to_string(),
        ChunkType::File => "File".to_string(),
        ChunkType::Ref => "Ref".to_string(),
        ChunkType::Meta => "Meta".to_string(),
        ChunkType::TensorDelta => "TensorDelta".to_string(),
        ChunkType::Unknown(v) => format!("Unknown({v})"),
    }
}

fn tag_label(t: Option<FileTag>) -> String {
    match t {
        Some(FileTag::Inference) => "inference".to_string(),
        Some(FileTag::Doc) => "doc".to_string(),
        Some(FileTag::Example) => "example".to_string(),
        Some(FileTag::Asset) => "asset".to_string(),
        None => "—".to_string(),
    }
}

fn humanize_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.0} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

fn empty_placeholder() -> impl Widget {
    mgui! {
        Center::new().child(
            Text::new(tr!("explorer.files.empty")).class("syn-empty-hint"),
        )
    }
}
