//! Модальный Portal-диалог «Цвет ноды» для редактора нод.
//!
//! Открывается через `NodeEditorCtx.tint_dialog: RwSignal<Option<NodeId>>`
//! из ContextMenu карточки (пункт «Цвет ноды…»). На «Применить» —
//! записывает `NodeStyle.tint` через `node.style.update(...)`. На
//! «Сбросить цвет» — `tint = None`. На «Отмена» / закрытие — ничего
//! не меняет.
//!
//! Архитектура зеркалит [`crate::pages::settings::knowledge_base::url_dialog`]:
//! один Portal с Reactive внутри, реагирует на сигнал-флаг в context'е.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{ColorPicker, ColorValue};

use crate::icons::{MI_CHECK, MI_CLOSE, MI_PALETTE, MI_REMOVE_CIRCLE_OUTLINE};

use super::state::NodeEditorCtx;
use super::types::NodeId;

pub fn view(ctx: NodeEditorCtx) -> impl Widget {
    let is_open = use_signal(false);
    let ctx_for_effect = ctx;
    create_effect(move || {
        let has = ctx_for_effect.tint_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let Some(node_id) = ctx.tint_dialog.get() else {
                return vec![Box::new(DecoratedBox::new().class("node-tint-dialog-empty"))];
            };
            vec![Box::new(card(ctx, node_id))]
        }))
}

fn card(ctx: NodeEditorCtx, node_id: NodeId) -> impl Widget {
    // Считываем текущий tint для инициализации ColorPicker и для
    // последующего commit'а. Если у ноды нет tint — стартуем с
    // приятного primary-accent'а (как у дефолтного ColorPicker'а).
    let nodes = ctx.nodes.get_untracked();
    let current = nodes
        .iter()
        .find(|n| n.id == node_id)
        .map(|n| n.style.get_untracked().tint);

    let initial = current
        .flatten()
        .map(ColorValue::from_color)
        .unwrap_or_else(|| ColorValue::new(238, 94, 72)); // var(--primary)

    let picked = use_signal(initial);

    let apply = move || {
        let cv = picked.get_untracked();
        let nodes = ctx.nodes.get_untracked();
        if let Some(node) = nodes.iter().find(|n| n.id == node_id) {
            node.style.update(|s| s.tint = Some(cv.to_color()));
        }
        ctx.tint_dialog.set(None);
    };

    let reset = move || {
        let nodes = ctx.nodes.get_untracked();
        if let Some(node) = nodes.iter().find(|n| n.id == node_id) {
            node.style.update(|s| s.tint = None);
        }
        ctx.tint_dialog.set(None);
    };

    let cancel = move || ctx.tint_dialog.set(None);

    mgui! {
        DecoratedBox::new().class("node-tint-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(MI_PALETTE).class("node-tint-dialog-icon"),
                    Text::new("Цвет ноды").class("node-tint-dialog-title"),
                ],
                Text::new("Оттенок смешивается с базовым тёмным фоном карточки — нода остаётся читаемой, но получает узнаваемый акцент.")
                    .class("node-tint-dialog-hint"),
                ColorPicker::new()
                    .color(initial)
                    .width(280.0)
                    .on_change(move |c| picked.set(c)),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Button::new("Сбросить")
                        .leading_icon(MI_REMOVE_CIRCLE_OUTLINE)
                        .on_click(reset)
                        .class("node-tint-dialog-btn-secondary"),
                    Row::new().gap(10.0) => [
                        Button::new("Отмена")
                            .leading_icon(MI_CLOSE)
                            .on_click(cancel)
                            .class("node-tint-dialog-btn-secondary"),
                        Button::new("Применить")
                            .leading_icon(MI_CHECK)
                            .on_click(apply)
                            .class("node-tint-dialog-btn-primary"),
                    ],
                ],
            ]
        ]
    }
}

