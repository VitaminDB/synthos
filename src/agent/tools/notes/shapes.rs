//! Действие `shape`: фигуры и линии на холсте — создание, правка,
//! стрелка между двумя закреплёнными блоками.

use super::*;

const SHAPE_STYLE_KEYS: [&str; 6] = ["fill", "stroke", "sw", "dash", "radius", "opacity"];
const LINE_POINT_KEYS: [&str; 8] = ["x1", "y1", "x2", "y2", "cx1", "cy1", "cx2", "cy2"];

fn parse_shape_kind(s: &str) -> Result<ShapeKind, String> {
    ShapeKind::from_name(&s.trim().to_ascii_lowercase()).ok_or_else(|| {
        format!(
            "unknown shape \"{s}\" — rect | ellipse | triangle | diamond | line | arrow | arrow2 | curve | \
             curve-arrow | curve-arrow2"
        )
    })
}

/// Стиль фигуры из аргументов (`fill stroke sw dash radius opacity`).
fn shape_style_pairs(v: &Json) -> Vec<(String, String)> {
    SHAPE_STYLE_KEYS
        .iter()
        .filter_map(|k| raw_string(v, k).map(|val| (k.to_string(), val)))
        .collect()
}

/// Абсолютные точки линии из аргументов `x1 y1 x2 y2 [cx1 cy1 cx2 cy2]`.
fn line_points_arg(v: &Json) -> Result<Option<Vec<(String, f32)>>, String> {
    let mut out = Vec::new();
    for k in LINE_POINT_KEYS {
        if v.get(k).is_some_and(|x| !x.is_null()) {
            let n = f32_field(v, k).filter(|f| f.is_finite()).ok_or_else(|| format!("bad \"{k}\" — a number in px"))?;
            out.push((k.to_string(), n));
        }
    }
    if out.is_empty() {
        return Ok(None);
    }
    let has = |k: &str| out.iter().any(|(key, _)| key == k);
    if has("x1") != has("y1") || has("x2") != has("y2") || has("cx1") != has("cy1") || has("cx2") != has("cy2") {
        return Err("line points come in pairs: x1 y1, x2 y2, cx1 cy1, cx2 cy2".to_string());
    }
    Ok(Some(out))
}

/// Записать абсолютные точки линии в блок: пересчитать рамку и относительные
/// координаты (канон: минимум точек в нуле). У незакреплённого блока точки
/// считаются относительными.
fn set_line_points(block: &mut DocBlock, kind: ShapeKind, points: &[(String, f32)]) {
    let pinned = free::pos_of(&block.attrs);
    let (ox, oy) = pinned.map(|(x, y)| (x + shape::LINE_PAD, y + shape::LINE_PAD)).unwrap_or((0.0, 0.0));
    // Текущие абсолютные значения, затем перекрываем заданными.
    let (p1, p2) = shape::endpoints_of(&block.attrs);
    let mut abs: Vec<(String, f32)> = vec![
        ("x1".into(), p1.0 + ox),
        ("y1".into(), p1.1 + oy),
        ("x2".into(), p2.0 + ox),
        ("y2".into(), p2.1 + oy),
    ];
    if kind.is_curve() {
        for k in ["cx1", "cy1", "cx2", "cy2"] {
            if let Some(val) = block.attrs.get(k).and_then(|s| s.parse::<f32>().ok()) {
                abs.push((k.to_string(), val + if k.starts_with("cx") { ox } else { oy }));
            }
        }
    }
    for (k, val) in points {
        if let Some(e) = abs.iter_mut().find(|(key, _)| key == k) {
            e.1 = *val;
        } else if kind.is_curve() {
            abs.push((k.clone(), *val));
        }
    }
    let xs: Vec<f32> = abs.iter().filter(|(k, _)| k.ends_with("x1") || k.ends_with("x2")).map(|(_, v)| *v).collect();
    let ys: Vec<f32> = abs.iter().filter(|(k, _)| k.ends_with("y1") || k.ends_with("y2")).map(|(_, v)| *v).collect();
    let min_x = xs.iter().copied().fold(f32::MAX, f32::min);
    let min_y = ys.iter().copied().fold(f32::MAX, f32::min);
    for k in LINE_POINT_KEYS {
        block.attrs.remove(k);
    }
    for (k, val) in &abs {
        let d = if k.ends_with("x1") || k.ends_with("x2") { min_x } else { min_y };
        block.attrs.set(k.clone(), fnum(val - d));
    }
    if pinned.is_some() || points.iter().any(|(k, _)| k == "x1" || k == "x2") {
        free::set_pos(&mut block.attrs, min_x - shape::LINE_PAD, min_y - shape::LINE_PAD);
    }
    let size = shape::line_box(&block.attrs, kind);
    free::set_width(&mut block.attrs, size.width.max(40.0));
    free::set_height(&mut block.attrs, size.height.max(20.0));
}

/// Середина стороны прямоугольника; `auto` — сторона, обращённая к `other`.
fn side_point(rect: (f32, f32, f32, f32), side: &str, other: (f32, f32)) -> Result<(f32, f32), String> {
    let (x, y, w, h) = rect;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let side = match side.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => {
            let (dx, dy) = (other.0 - cx, other.1 - cy);
            if dx.abs() >= dy.abs() {
                if dx >= 0.0 { "right" } else { "left" }
            } else if dy >= 0.0 {
                "bottom"
            } else {
                "top"
            }
        }
        "left" | "l" => "left",
        "right" | "r" => "right",
        "top" | "t" => "top",
        "bottom" | "b" => "bottom",
        "center" | "c" => "center",
        other => return Err(format!("bad side \"{other}\" (auto | left | right | top | bottom | center)")),
    };
    Ok(match side {
        "left" => (x, cy),
        "right" => (x + w, cy),
        "top" => (cx, y),
        "bottom" => (cx, y + h),
        _ => (cx, cy),
    })
}

pub(super) fn shape_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or("missing \"op\" (create | update | delete | connect)")?;
    let id = page_arg(ctx, v, "page")?;
    let mut model = load_model(ctx, &id);
    match op {
        "create" => {
            let kind = parse_shape_kind(str_field(v, "kind").unwrap_or("rect"))?;
            let mut block = DocBlock { id: model.alloc_id(), kind: BlockKind::Shape { shape: kind }, attrs: Attrs::default() };
            apply_attrs(&mut block, &shape_style_pairs(v))?;
            let geom = parse_geom(v)?;
            if kind.is_line() {
                let points = line_points_arg(v)?.unwrap_or_else(|| {
                    let (x, y) = (geom.x.unwrap_or(40.0), geom.y.unwrap_or(40.0));
                    vec![("x1".into(), x), ("y1".into(), y), ("x2".into(), x + shape::DEFAULT_W), ("y2".into(), y)]
                });
                if let (Some(x), Some(y)) = (geom.x, geom.y) {
                    free::set_pos(&mut block.attrs, x, y);
                }
                set_line_points(&mut block, kind, &points);
            } else {
                geom.apply(&mut block.attrs);
                if geom.w.is_none() {
                    free::set_width(&mut block.attrs, shape::DEFAULT_W);
                }
                if geom.h.is_none() {
                    free::set_height(&mut block.attrs, shape::DEFAULT_H);
                }
            }
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let idx = insert_blocks(&mut model, vec![block], pos);
            store_model(ctx, &id, &model)?;
            let i = idx[0];
            Ok(format!("created shape {}\n{}\n{}\n", pos_text(pos), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "update" => {
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\"")?)?;
            let BlockKind::Shape { shape: mut kind } = model.blocks[i].kind else {
                return Err(format!("block #{i} is not a shape (blocks op=set_attrs edits other blocks)"));
            };
            let mut changes = Vec::new();
            if let Some(k) = str_field(v, "kind") {
                let new_kind = parse_shape_kind(k)?;
                if new_kind != kind {
                    model.blocks[i].kind = BlockKind::Shape { shape: new_kind };
                    if new_kind.is_line() != kind.is_line() {
                        for key in LINE_POINT_KEYS {
                            model.blocks[i].attrs.remove(key);
                        }
                    }
                    kind = new_kind;
                    changes.push(format!("kind {}", new_kind.name()));
                }
            }
            let style = shape_style_pairs(v);
            if !style.is_empty() {
                changes.extend(apply_attrs(&mut model.blocks[i], &style)?);
            }
            let geom = parse_geom(v)?;
            if !geom.is_empty() {
                geom.apply(&mut model.blocks[i].attrs);
                changes.push("geometry".to_string());
            }
            if let Some(points) = line_points_arg(v)? {
                if !kind.is_line() {
                    return Err("x1/y1/x2/y2 apply to lines and arrows; frame shapes take x y w h".to_string());
                }
                set_line_points(&mut model.blocks[i], kind, &points);
                changes.push("points".to_string());
            } else if kind.is_line() {
                canonicalize_line(&mut model.blocks[i], kind);
            }
            if changes.is_empty() {
                return Err("nothing to change: pass kind, fill/stroke/sw/dash/radius/opacity, x y w h or line points".to_string());
            }
            store_model(ctx, &id, &model)?;
            Ok(format!("updated {}\n{}\n{}\n", changes.join(", "), block_line_checked(&model, i), page_line(ctx, &id)))
        }
        "delete" => {
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\"")?)?;
            if !matches!(model.blocks[i].kind, BlockKind::Shape { .. }) {
                return Err(format!("block #{i} is not a shape — blocks op=delete removes any block"));
            }
            let line = block_line(i, &model.blocks[i]);
            model.blocks.remove(i);
            store_model(ctx, &id, &model)?;
            Ok(format!("deleted {line}\n{}\n", page_line(ctx, &id)))
        }
        "connect" => {
            let from = resolve_block(&model, &ref_field(v, "from").ok_or("missing \"from\" (block)")?)?;
            let to = resolve_block(&model, &ref_field(v, "to").ok_or("missing \"to\" (block)")?)?;
            if from == to {
                return Err("\"from\" and \"to\" are the same block".to_string());
            }
            let need = |i: usize| block_rect(&model.blocks[i]).ok_or_else(|| format!("block #{i} has no coordinates — blocks op=pin it first"));
            let (ra, rb) = (need(from)?, need(to)?);
            let center = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
            let p1 = side_point(ra, str_field(v, "from_side").unwrap_or("auto"), center(rb))?;
            let p2 = side_point(rb, str_field(v, "to_side").unwrap_or("auto"), center(ra))?;
            let kind = parse_shape_kind(str_field(v, "kind").unwrap_or("arrow"))?;
            if !kind.is_line() {
                return Err("connect draws a line: kind must be line | arrow | arrow2 | curve | curve-arrow | curve-arrow2".to_string());
            }
            let mut block = DocBlock { id: model.alloc_id(), kind: BlockKind::Shape { shape: kind }, attrs: Attrs::default() };
            apply_attrs(&mut block, &shape_style_pairs(v))?;
            let points = vec![("x1".into(), p1.0), ("y1".into(), p1.1), ("x2".into(), p2.0), ("y2".into(), p2.1)];
            set_line_points(&mut block, kind, &points);
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let idx = insert_blocks(&mut model, vec![block], pos);
            store_model(ctx, &id, &model)?;
            let i = idx[0];
            Ok(format!(
                "connected #{from} → #{to} with {}\n{}\n{}\n",
                kind.name(),
                block_line(i, &model.blocks[i]),
                page_line(ctx, &id)
            ))
        }
        other => Err(format!("unknown shape op \"{other}\" (create | update | delete | connect)")),
    }
}
