//! Рёбра канваса: кубические безье со стрелками между ближайшими краями
//! карточек + pending-провод при протяжке. Рендер — Canvas-замыкание под
//! слоем карточек (паттерн node_editor::wires).

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Size};
use syngui::prelude::*;
use syngui::widgets::visual::Canvas;

use super::CanvasHandle;

pub fn view(handle: CanvasHandle) -> impl Widget {
    Canvas::new(move |c, _t| {
        // Подписки: структура + позиции карточек (через явные сигналы).
        let _ = handle.structure_rev.get();
        let edges: Vec<(String, String, String)> = handle
            .lock()
            .edges
            .iter()
            .map(|e| (e.id.clone(), e.from.clone(), e.to.clone()))
            .collect();
        // Явная подписка на позиции/размеры участвующих карточек.
        for (_, from, to) in &edges {
            let _ = handle.node_pos(from).get();
            let _ = handle.node_pos(to).get();
            let _ = handle.node_size(from).get();
            let _ = handle.node_size(to).get();
        }
        let pending = handle.pending_wire.get();
        let selected_edge = handle.selected_edge.get();

        c.set_anti_alias(1.0);
        for (id, from, to) in &edges {
            let (Some(a), Some(b)) = (handle.node_rect(from), handle.node_rect(to)) else {
                continue;
            };
            let (p1, p2) = anchor_points(a, b);
            let selected = selected_edge.as_deref() == Some(id.as_str());
            let color = if selected {
                Color::from_hex("#EE5E48")
            } else {
                Color::from_hex("#8b95a6")
            };
            c.set_color(color);
            c.set_stroke_width(if selected { 2.5 } else { 1.8 });
            draw_edge(c, p1, p2);
        }
        if let Some((from, cursor)) = pending {
            if let Some(a) = handle.node_rect(&from) {
                let p1 = Point::new(a.0.x + a.1.width / 2.0, a.0.y + a.1.height / 2.0);
                c.set_color(Color::from_hex("#EE5E48").with_alpha(0.8));
                c.set_stroke_width(2.0);
                draw_edge(c, p1, cursor);
            }
        }
    })
    .size(6000.0, 6000.0)
    .animated(false)
}

/// Точки крепления: центры ближайших сторон двух прямоугольников.
fn anchor_points(a: (Point, Size), b: (Point, Size)) -> (Point, Point) {
    let ac = Point::new(a.0.x + a.1.width / 2.0, a.0.y + a.1.height / 2.0);
    let bc = Point::new(b.0.x + b.1.width / 2.0, b.0.y + b.1.height / 2.0);
    let dx = bc.x - ac.x;
    let dy = bc.y - ac.y;
    let horizontal = dx.abs() >= dy.abs();
    let p1 = if horizontal {
        Point::new(if dx >= 0.0 { a.0.x + a.1.width } else { a.0.x }, ac.y)
    } else {
        Point::new(ac.x, if dy >= 0.0 { a.0.y + a.1.height } else { a.0.y })
    };
    let p2 = if horizontal {
        Point::new(if dx >= 0.0 { b.0.x } else { b.0.x + b.1.width }, bc.y)
    } else {
        Point::new(bc.x, if dy >= 0.0 { b.0.y } else { b.0.y + b.1.height })
    };
    (p1, p2)
}

fn draw_edge(c: &mut CanvasContext, p1: Point, p2: Point) {
    let dx = (p2.x - p1.x).abs().max(40.0) * 0.45;
    let horizontal = (p2.x - p1.x).abs() >= (p2.y - p1.y).abs();
    let (c1, c2) = if horizontal {
        let dir = if p2.x >= p1.x { 1.0 } else { -1.0 };
        (
            Point::new(p1.x + dir * dx, p1.y),
            Point::new(p2.x - dir * dx, p2.y),
        )
    } else {
        let dy = (p2.y - p1.y).abs().max(40.0) * 0.45;
        let dir = if p2.y >= p1.y { 1.0 } else { -1.0 };
        (
            Point::new(p1.x, p1.y + dir * dy),
            Point::new(p2.x, p2.y - dir * dy),
        )
    };
    c.draw_cubic_bezier(p1.x, p1.y, c1.x, c1.y, c2.x, c2.y, p2.x, p2.y);
    // Стрелка по направлению касательной в конце.
    let tx = p2.x - c2.x;
    let ty = p2.y - c2.y;
    let len = (tx * tx + ty * ty).sqrt().max(0.001);
    let (ux, uy) = (tx / len, ty / len);
    let (nx, ny) = (-uy, ux);
    c.fill_polygon(&[
        (p2.x, p2.y),
        (p2.x - ux * 9.0 + nx * 4.5, p2.y - uy * 9.0 + ny * 4.5),
        (p2.x - ux * 9.0 - nx * 4.5, p2.y - uy * 9.0 - ny * 4.5),
    ]);
}

/// Расстояние точки до ребра (по сэмплам безье) — hit-test клика.
pub fn edge_hit(handle: &CanvasHandle, world: Point, tolerance: f32) -> Option<String> {
    let edges: Vec<(String, String, String)> = handle
        .lock()
        .edges
        .iter()
        .map(|e| (e.id.clone(), e.from.clone(), e.to.clone()))
        .collect();
    for (id, from, to) in edges {
        let (Some(a), Some(b)) = (handle.node_rect(&from), handle.node_rect(&to)) else {
            continue;
        };
        let (p1, p2) = anchor_points(a, b);
        // Сэмплируем прямую p1..p2 c лёгким прогибом достаточно грубо.
        for i in 0..=24 {
            let t = i as f32 / 24.0;
            let x = p1.x + (p2.x - p1.x) * t;
            let y = p1.y + (p2.y - p1.y) * t;
            let d = ((world.x - x).powi(2) + (world.y - y).powi(2)).sqrt();
            if d <= tolerance {
                return Some(id);
            }
        }
    }
    None
}
