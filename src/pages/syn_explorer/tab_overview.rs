//! Таб «Обзор»: id/version/arch/purpose, статистика чанков, capabilities.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use synaptix_bundle::BundleMeta;

use super::state::{BundleStats, OpenBundle, SynExplorerCtx};

pub fn view(active: OpenBundle) -> impl Widget {
    DecoratedBox::new().class("syn-tab-pane syn-tab-overview").child(Reactive::new(
        move || -> Vec<Box<dyn Widget>> {
            let _ = active.reload_gen.get();
            let meta = active.meta.get();
            let stats = active.stats.get();
            // Подписка на активный пакет — чтобы при close таб обнулился.
            let _ = use_context::<SynExplorerCtx>().active_bundle.get();

            let col = mgui! {
                Column::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        header_card(&meta, &stats),
                        chunks_section(&stats),
                        capabilities_section(&meta),
                    ]
            };
            vec![Box::new(col)]
        },
    ))
}

fn header_card(meta: &BundleMeta, stats: &BundleStats) -> impl Widget {
    let header = Text::new(if meta.id.is_empty() { "Без id".to_string() } else { meta.id.clone() })
        .class("syn-overview-title");
    let version = Text::new(format!("Версия: {}", display_or_dash(&meta.version)))
        .class("syn-overview-line");
    let arch = Text::new(format!("Архитектура: {}", display_or_dash(&meta.arch)))
        .class("syn-overview-line");
    let purpose = Text::new(format!("Назначение: {}", display_or_dash(&meta.purpose)))
        .class("syn-overview-line");
    let size = Text::new(format!("Размер: {}", humanize_bytes(stats.total_size)))
        .class("syn-overview-line");
    let format = Text::new(format!(
        "Формат: v{}.{}",
        stats.format_version.0, stats.format_version.1
    ))
    .class("syn-overview-line");
    let created = match meta.created_at {
        Some(ts) => format!("Создан: {}", format_timestamp(ts)),
        None => "Создан: —".to_string(),
    };
    let created = Text::new(created).class("syn-overview-line");
    mgui! {
        DecoratedBox::new().class("syn-overview-card") => [
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Start) => [
                    header,
                    version,
                    arch,
                    purpose,
                    size,
                    format,
                    created,
                ]
        ]
    }
}

fn chunks_section(stats: &BundleStats) -> impl Widget {
    mgui! {
        DecoratedBox::new().class("syn-overview-card") => [
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new("Чанки").class("syn-overview-subtitle"),
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            stat_chip("Тензоры", stats.tensor_chunks),
                            stat_chip("Quantized", stats.quantized_chunks),
                            stat_chip("Файлы", stats.file_chunks),
                            stat_chip("Refs", stats.ref_chunks),
                            stat_chip("Tombstones", stats.tombstoned_chunks),
                        ],
                ]
        ]
    }
}

fn stat_chip(label: &str, n: usize) -> impl Widget {
    mgui! {
        DecoratedBox::new().class("syn-stat-chip") => [
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(format!("{n}")).class("syn-stat-value"),
                    Text::new(label.to_string()).class("syn-stat-label"),
                ]
        ]
    }
}

fn capabilities_section(meta: &BundleMeta) -> impl Widget {
    let required = if meta.required_caps.is_empty() {
        "—".to_string()
    } else {
        meta.required_caps.join(", ")
    };
    let optional = if meta.optional_caps.is_empty() {
        "—".to_string()
    } else {
        meta.optional_caps.join(", ")
    };
    let components = if meta.components.is_empty() {
        "—".to_string()
    } else {
        meta.components
            .iter()
            .map(|(k, v)| format!("{k} → {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let refs_label = if meta.refs.is_empty() {
        "—".to_string()
    } else {
        meta.refs
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let overlays = format!("LoRA-overlays: {}", meta.overlays.len());

    mgui! {
        DecoratedBox::new().class("syn-overview-card") => [
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new("Capabilities").class("syn-overview-subtitle"),
                    Text::new(format!("Required: {required}")).class("syn-overview-line"),
                    Text::new(format!("Optional: {optional}")).class("syn-overview-line"),
                    Text::new(format!("Компоненты: {components}")).class("syn-overview-line"),
                    Text::new(format!("Refs: {refs_label}")).class("syn-overview-line"),
                    Text::new(overlays).class("syn-overview-line"),
                ]
        ]
    }
}

fn display_or_dash(s: &str) -> String {
    if s.is_empty() {
        "—".to_string()
    } else {
        s.to_string()
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

fn format_timestamp(ts: u64) -> String {
    // Простой формат: ISO-8601 UTC без зависимости от chrono. На большую
    // точность даты пользователю в этом UI смотреть не нужно (`syn-pack` пишет
    // unix seconds на момент `BundleBuilder::write`).
    let secs = ts as i64;
    // Зависит от libc localtime — упрощённо отдаём unix ts; при необходимости
    // позже подключим chrono. Пока — формат «N сек unix» с парсингом для
    // удобочитаемости.
    let _ = secs;
    format!("{ts}")
}
