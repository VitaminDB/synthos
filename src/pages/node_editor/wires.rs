//! Канвас связей. Для каждого `Connection` рисует cubic bezier между
//! портами, и при наличии `pending_wire` — текущий drag-провод.
//!
//! Координаты портов восстанавливаются геометрически по `NodeInstance.pos`
//! и метадате нод (см. `node_view::port_world_pos`). Поэтому Canvas
//! зависит только от ctx.nodes / ctx.connections / ctx.pending — а
//! значит реактивно перерисовывается при их изменении.

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::visual::Canvas;

use super::node_view;
use super::state::NodeEditorCtx;
use super::types::{Connection, NodeInstance, PendingWire, PortKind, PortSide};

/// Цвет провода по типу порта. Возвращает RGBA для `CanvasContext::set_color`.
pub fn wire_color(kind: PortKind, active: bool) -> Color {
    if active {
        Color::from_hex("#F59E0B") // amber-500 — pending wire
    } else {
        match kind {
            PortKind::Data => Color::from_hex("#3B82F6"),
            PortKind::Audio => Color::from_hex("#10B981"),
            PortKind::Control => Color::from_hex("#A855F7"),
            PortKind::Text => Color::from_hex("#EAB308"),
            PortKind::Video => Color::from_hex("#EC4899"),
            PortKind::Image => Color::from_hex("#F97316"),
        }
    }
}

/// Создать Canvas-виджет с провод-рендером. Размер = большой `world`-rect
/// (5000×5000 хватит для MVP), Canvas лежит внутри Stack под нодами,
/// внутри `PanZoomViewport` (transform применится внешним viewport'ом).
pub fn view() -> impl Widget {
    let ctx = use_context::<NodeEditorCtx>();
    Canvas::new(move |c, _t| {
        // Реактивно подписываемся.
        let nodes = ctx.nodes.get();
        let conns = ctx.connections.get();
        let pending = ctx.pending.get();

        c.set_anti_alias(1.0);
        c.set_stroke_width(2.0);

        for conn in &conns {
            draw_connection(c, &nodes, conn);
        }
        if let Some(p) = pending {
            draw_pending(c, &nodes, &p);
        }
    })
    .size(5000.0, 5000.0)
    .animated(false)
}

fn draw_connection(c: &mut syngui::core::canvas::CanvasContext, nodes: &[NodeInstance], conn: &Connection) {
    let Some(from) = nodes.iter().find(|n| n.id == conn.from_node) else { return; };
    let Some(to) = nodes.iter().find(|n| n.id == conn.to_node) else { return; };
    let Some((p1, kind)) = node_view::port_world_pos(from, PortSide::Output, conn.from_port) else { return; };
    let Some((p2, _)) = node_view::port_world_pos(to, PortSide::Input, conn.to_port) else { return; };
    c.set_color(wire_color(kind, false));
    draw_bezier(c, p1, p2);
}

fn draw_pending(c: &mut syngui::core::canvas::CanvasContext, nodes: &[NodeInstance], pending: &PendingWire) {
    let Some(from) = nodes.iter().find(|n| n.id == pending.from_node) else { return; };
    let Some((p1, _)) = node_view::port_world_pos(from, PortSide::Output, pending.from_port) else { return; };
    c.set_color(wire_color(pending.from_kind, true));
    draw_bezier(c, p1, pending.current);
}

/// Рисование «классической» cubic bezier-линии между портами:
/// control points смещены по горизонтали на |dx|*0.4. Минимум — 60px,
/// чтобы провод не вырождался в прямую при близких точках.
fn draw_bezier(c: &mut syngui::core::canvas::CanvasContext, a: syngui::core::Point, b: syngui::core::Point) {
    let dx = (b.x - a.x).abs();
    let cp_off = (dx * 0.4).max(60.0);
    let c1 = (a.x + cp_off, a.y);
    let c2 = (b.x - cp_off, b.y);
    c.draw_cubic_bezier(a.x, a.y, c1.0, c1.1, c2.0, c2.1, b.x, b.y);
}
