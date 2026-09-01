//! Панель канваса: PanZoomViewport + слой проводов + карточки + тулбар.
//! Правила из node_editor: overlay без DecoratedBox (passthrough), LMB-pan
//! отключён в зоне карточек.

use syngui::core::Point;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui::containers::PanZoomViewport;

use crate::icons::*;

use super::{card, wires, CanvasHandle};

pub fn view(handle: CanvasHandle) -> impl Widget {
    let filter_handle = handle.clone();
    let click_handle = handle.clone();
    let world = world_layer(handle.clone());
    let viewport = PanZoomViewport::new()
        .pan(handle.pan)
        .zoom(handle.zoom)
        .zoom_range(0.25, 4.0)
        .grid(true)
        .grid_step(40.0)
        .pan_button(MouseButton::Middle)
        .pan_filter(move |world| {
            const INFLATE: f32 = 10.0;
            !filter_handle.hit_node_inflated(world, INFLATE)
        })
        .on_background_click(move |world, _screen| {
            // Клик по ребру выделяет его; по пустому — снимает выделение.
            let zoom = click_handle.zoom.get_untracked().max(0.05);
            if let Some(edge) = wires::edge_hit(&click_handle, world, 8.0 / zoom) {
                click_handle.selected_edge.set(Some(edge));
            } else {
                click_handle.selected_edge.set(None);
            }
            click_handle.selected.set(None);
            click_handle.editing.set(None);
        })
        .child(world)
        .class("notes-canvas-viewport");

    Stack::new()
        .fit(StackFit::Expand)
        .child(viewport)
        .child(toolbar(handle))
}

fn world_layer(handle: CanvasHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        let node_ids: Vec<String> = handle.lock().nodes.iter().map(|n| n.id.clone()).collect();
        let mut layers: Vec<Box<dyn Widget>> = Vec::with_capacity(node_ids.len() + 1);
        layers.push(Box::new(wires::view(handle.clone())));
        for id in node_ids {
            layers.push(Box::new(card::view(handle.clone(), id)));
        }
        vec![Box::new(Stack::new().clip(false).children(layers))]
    })
}

/// Overlay-полоса: passthrough-контейнеры (без DecoratedBox на всю площадь).
fn toolbar(handle: CanvasHandle) -> impl Widget {
    let add_handle = handle.clone();
    let del_handle = handle.clone();
    Padding::only(12.0, 12.0, 12.0, 0.0).child(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(
                DecoratedBox::new().class("notes-canvas-toolbar").child(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            ToolButton::new(MI_ADD)
                                .tooltip(tr!("notes.canvas.add_card"))
                                .on_click(move || {
                                    let pan = add_handle.pan.get_untracked();
                                    let zoom = add_handle.zoom.get_untracked().max(0.05);
                                    let world = Point::new(
                                        (280.0 - pan.x) / zoom,
                                        (200.0 - pan.y) / zoom,
                                    );
                                    add_handle.add_node(world);
                                }),
                        )
                        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                            let sel_node = del_handle.selected.get();
                            let sel_edge = del_handle.selected_edge.get();
                            if sel_node.is_none() && sel_edge.is_none() {
                                return Vec::new();
                            }
                            let h = del_handle.clone();
                            vec![Box::new(
                                ToolButton::new(MI_CLOSE)
                                    .tooltip(tr!("notes.canvas.delete"))
                                    .on_click(move || {
                                        if let Some(id) = h.selected.get_untracked() {
                                            h.delete_node(&id);
                                        } else if let Some(id) =
                                            h.selected_edge.get_untracked()
                                        {
                                            h.delete_edge(&id);
                                        }
                                    }),
                            )]
                        })),
                ),
            ),
    )
}

/// Пустое состояние — подсказка.
pub fn empty_hint(handle: &CanvasHandle) -> Option<impl Widget> {
    let empty = handle.lock().nodes.is_empty();
    empty.then(|| {
        Center::new().child(
            Text::new(tr!("notes.canvas.empty")).class("notes-empty-hint"),
        )
    })
}
