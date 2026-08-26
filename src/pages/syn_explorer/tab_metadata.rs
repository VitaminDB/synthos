//! Таб «Метаданные»: редактируемые поля BundleMeta.
//!
//! TextField'ы обернуты в Reactive, чтобы повторное открытие пакета (reload)
//! пересоздавало виджеты с актуальными значениями. on_change пишет в
//! `OpenBundle.meta` — dirty-tracker effect видит расхождение с
//! `original_meta` и зажигает dirty.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::TextField;

use super::state::{OpenBundle, SynExplorerCtx};

pub fn view(active: OpenBundle) -> impl Widget {
    DecoratedBox::new().class("syn-tab-pane syn-tab-metadata").child(Reactive::new(
        move || -> Vec<Box<dyn Widget>> {
            let _ = active.reload_gen.get();
            let meta = active.meta.get();
            let _ = use_context::<SynExplorerCtx>().active_bundle.get();

            // Снимаем текущие значения в локалы — каждый TextField получает
            // initial-text из них, дальше пишет в active.meta через update.
            let id_v = meta.id.clone();
            let version_v = meta.version.clone();
            let arch_v = meta.arch.clone();
            let purpose_v = meta.purpose.clone();

            let active_id = active;
            let active_version = active;
            let active_arch = active;
            let active_purpose = active;

            let col = mgui! {
                Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        field_row("id", tr!("explorer.metadata.id.hint"), TextField::with_text(id_v)
                            .on_change(move |s| {
                                let v = s.to_string();
                                active_id.meta.update(|m| m.id = v);
                            })
                            .class("syn-meta-input")),
                        field_row("version", tr!("explorer.metadata.version.hint"), TextField::with_text(version_v)
                            .on_change(move |s| {
                                let v = s.to_string();
                                active_version.meta.update(|m| m.version = v);
                            })
                            .class("syn-meta-input")),
                        field_row("arch", tr!("explorer.metadata.arch.hint"), TextField::with_text(arch_v)
                            .on_change(move |s| {
                                let v = s.to_string();
                                active_arch.meta.update(|m| m.arch = v);
                            })
                            .class("syn-meta-input")),
                        field_row("purpose", tr!("explorer.metadata.purpose.hint"), TextField::with_text(purpose_v)
                            .on_change(move |s| {
                                let v = s.to_string();
                                active_purpose.meta.update(|m| m.purpose = v);
                            })
                            .class("syn-meta-input")),
                        components_section(&meta),
                        refs_section(&meta),
                        Text::new(tr!("explorer.metadata.readonly_hint"))
                            .class("syn-meta-hint"),
                    ]
            };
            vec![Box::new(col)]
        },
    ))
}

fn field_row<W: Widget + 'static>(label: &str, hint: impl Into<String>, field: W) -> impl Widget {
    let label_owned = label.to_string();
    let hint_owned = hint.into();
    mgui! {
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(label_owned).class("syn-property-label"),
                Text::new(hint_owned).class("syn-property-hint"),
                field,
            ]
    }
}

fn components_section(meta: &synaptix_bundle::BundleMeta) -> impl Widget {
    let items: Vec<String> = meta
        .components
        .iter()
        .map(|(k, v)| format!("{k} → {v}"))
        .collect();
    let body = if items.is_empty() {
        "—".to_string()
    } else {
        items.join("\n")
    };
    mgui! {
        DecoratedBox::new().class("syn-meta-readonly-block") => [
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.metadata.components_label")).class("syn-property-label"),
                    Text::new(body).class("syn-meta-readonly-text"),
                ]
        ]
    }
}

fn refs_section(meta: &synaptix_bundle::BundleMeta) -> impl Widget {
    let items: Vec<String> = meta
        .refs
        .iter()
        .map(|r| format!("{} ({})", r.id, r.tensor_prefix))
        .collect();
    let body = if items.is_empty() {
        "—".to_string()
    } else {
        items.join("\n")
    };
    mgui! {
        DecoratedBox::new().class("syn-meta-readonly-block") => [
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new("Refs").class("syn-property-label"),
                    Text::new(body).class("syn-meta-readonly-text"),
                ]
        ]
    }
}
