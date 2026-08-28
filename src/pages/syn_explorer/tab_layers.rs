//! Вкладка «Слои»: из чего состоит открытый пакет.
//!
//! На каждый `tensors:*`-чанк — полоса состава по ролям и таблица групп имён
//! со схлопнутыми индексами слоёв (`model.layers.[0-63].mlp.up_proj.weight`).
//! Так видно, где vision-башня, где `lm_head`, где эмбеддинги и сколько
//! каждая часть весит, — не открывая модель в стороннем инструменте.
//!
//! Данные считаются один раз при открытии пакета
//! ([`super::bundle_io::collect_layers`]): разбор safetensors-заголовка
//! внутри mmap стоит миллисекунды даже на 77 гигабайтах, поэтому здесь
//! только отрисовка готового снимка.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{TableColumn, TableView};

use super::pack_dialogs::{humanize_bytes, role_label, segment_widths};
use super::state::{ComponentLayers, OpenBundle, SynExplorerCtx};

pub fn view(active: OpenBundle) -> impl Widget {
    DecoratedBox::new().class("syn-tab-pane syn-tab-layers").child(Reactive::new(
        move || -> Vec<Box<dyn Widget>> {
            let _ = active.reload_gen.get();
            let layers = active.layers.get();
            let _ = use_context::<SynExplorerCtx>().active_bundle.get();

            if layers.is_empty() {
                return vec![Box::new(empty_placeholder())];
            }
            let mut col = Column::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch);
            for c in layers.iter() {
                let c = c.clone();
                col = col.child(move || component_block(c.clone()));
            }
            vec![Box::new(col)]
        },
    ))
}

fn component_block(layers: ComponentLayers) -> impl Widget {
    let title = tr!(
        "explorer.layers.component_title",
        name = layers.component.clone(),
        count = layers.tensor_count,
        size = humanize_bytes(layers.bytes)
    );
    let bar = role_strip(&layers);
    let table = groups_table(&layers);

    mgui! {
        DecoratedBox::new().class("syn-overview-card") => [
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(title).class("syn-overview-subtitle"),
                    bar,
                    table,
                ]
        ]
    }
}

/// Полоса «из чего состоит» плюс подписи ролей с долями и объёмом.
fn role_strip(layers: &ComponentLayers) -> impl Widget {
    let total = layers.bytes.max(1);
    let widths = segment_widths(&layers.by_role, total);
    let mut bar = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for ((role, _), pct) in layers.by_role.iter().zip(widths) {
        let class = format!("syn-role-seg role-{}", role.key());
        bar = bar.child(move || {
            DecoratedBox::new()
                .class(class.clone())
                .style("width", StyleValue::percent(pct))
        });
    }

    let mut legend = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (role, est) in layers.by_role.iter() {
        let bytes = est.dense;
        let pct = (bytes as f64 / total as f64) * 100.0;
        let text = format!("{} · {} · {:.1}%", role_label(*role), humanize_bytes(bytes), pct);
        let dot_class = format!("syn-role-dot role-{}", role.key());
        legend = legend.child(move || {
            mgui! {
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center) => [
                        DecoratedBox::new().class(dot_class.clone()),
                        Text::new(text.clone()).class("syn-role-legend"),
                    ]
            }
        });
    }

    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("syn-role-bar").child(bar),
                legend,
            ]
    }
}

fn groups_table(layers: &ComponentLayers) -> impl Widget {
    let columns = vec![
        TableColumn::flex(tr!("explorer.layers.column.group"), 3.0),
        TableColumn::fixed(tr!("explorer.layers.column.role"), 130.0),
        TableColumn::fixed(tr!("explorer.layers.column.count"), 70.0),
        TableColumn::fixed(tr!("explorer.layers.column.dtype"), 80.0),
        TableColumn::fixed(tr!("explorer.layers.column.shape"), 150.0),
        TableColumn::fixed(tr!("explorer.layers.column.size"), 110.0),
    ];
    let rows: Vec<Vec<String>> = layers
        .groups
        .iter()
        .map(|g| {
            let shape = if g.shape.is_empty() {
                "—".to_string()
            } else {
                g.shape
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("×")
            };
            vec![
                g.pattern.clone(),
                role_label(g.role),
                format!("{}", g.count),
                g.dtype.clone(),
                shape,
                humanize_bytes(g.bytes),
            ]
        })
        .collect();

    TableView::new(columns, rows)
        .striped(true)
        .row_height(26.0)
        .header_height(30.0)
        .class("syn-layers-table")
}

fn empty_placeholder() -> impl Widget {
    mgui! {
        Center::new().child(
            Text::new(tr!("explorer.layers.empty")).class("syn-empty-hint"),
        )
    }
}
