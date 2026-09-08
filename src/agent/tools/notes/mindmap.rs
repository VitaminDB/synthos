//! Действие `mindmap`: узлы карты, связи, свёртка и оформление.

use super::*;

/// Узел карты по id либо тексту (единственное совпадение); `root` — корень.
fn resolve_node(handle: &MindmapHandle, s: &str) -> Result<String, String> {
    let doc = handle.lock();
    let t = s.trim();
    if t.eq_ignore_ascii_case("root") {
        return Ok(doc.root_id());
    }
    if doc.node(t).is_some() {
        return Ok(t.to_string());
    }
    let key = t.to_lowercase();
    let hits: Vec<&str> = doc.nodes.iter().filter(|n| n.text.trim().to_lowercase() == key).map(|n| n.id.as_str()).collect();
    match hits.len() {
        1 => Ok(hits[0].to_string()),
        0 => Err(format!("node \"{s}\" not found — ids and texts are in mindmap op=read")),
        n => Err(format!("{n} nodes are named \"{s}\" — use the id: {}", hits.join(", "))),
    }
}

/// Стиль карты из аргумента `style` (объект); возвращает описание изменений.
fn apply_map_style(handle: &MindmapHandle, v: &Json) -> Result<Vec<String>, String> {
    let Some(raw) = v.get("style") else { return Ok(Vec::new()) };
    let pairs = style_pairs(raw)?;
    let mut changes = Vec::new();
    let mut err = None;
    handle.set_style(|s| {
        for (k, val) in &pairs {
            let key = k.trim().to_ascii_lowercase();
            let num = || val.trim().parse::<f32>().ok();
            match key.as_str() {
                "palette" => {
                    let colors: Vec<String> = val
                        .split([',', ' '])
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(|c| parse_hex_color(c, false).unwrap_or_else(|_| c.to_string()))
                        .collect();
                    let colors = if colors.len() == 1 {
                        match crate::pages::notes::mindmap::model::palette_by_key(&colors[0].to_lowercase()) {
                            Some(p) => p.iter().map(|x| x.to_string()).collect(),
                            None => colors,
                        }
                    } else {
                        colors
                    };
                    if !colors.is_empty() {
                        s.palette = colors;
                        changes.push("palette".to_string());
                    }
                }
                "node_fill" | "node_stroke" | "text_color" | "line_color" | "bg" => {
                    let color = if val.trim().is_empty() || val.eq_ignore_ascii_case("none") {
                        String::new()
                    } else {
                        match parse_hex_color(val, true) {
                            Ok(c) => c,
                            Err(e) => {
                                err = Some(e);
                                return;
                            }
                        }
                    };
                    match key.as_str() {
                        "node_fill" => s.node_fill = color,
                        "node_stroke" => s.node_stroke = color,
                        "text_color" => s.text_color = color,
                        "line_color" => s.line_color = color,
                        _ => s.bg = color,
                    }
                    changes.push(key.clone());
                }
                "font_size" | "radius" | "padding" | "line_width" | "line_dash" | "max_node_w" => {
                    let Some(n) = num() else {
                        err = Some(format!("bad \"{key}\" \"{val}\" — a number"));
                        return;
                    };
                    match key.as_str() {
                        "font_size" => s.font_size = n,
                        "radius" => s.radius = n,
                        "padding" => s.padding = n,
                        "line_width" => s.line_width = n,
                        "line_dash" => s.line_dash = n,
                        _ => s.max_node_w = n,
                    }
                    changes.push(format!("{key}={}", fnum(n)));
                }
                "weight" => {
                    s.weight = if val.eq_ignore_ascii_case("bold") { "bold".to_string() } else { String::new() };
                    changes.push("weight".to_string());
                }
                "show_icons" => {
                    s.show_icons = matches!(val.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
                    changes.push("show_icons".to_string());
                }
                other => {
                    err = Some(format!(
                        "unknown style key \"{other}\" — palette, node_fill, node_stroke, text_color, line_color, bg, \
                         font_size, weight, radius, padding, line_width, line_dash, show_icons, max_node_w"
                    ));
                }
            }
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(changes),
    }
}

/// Пары ключ-значение из объекта либо строки `k=v …`.
pub(super) fn style_pairs(raw: &Json) -> Result<Vec<(String, String)>, String> {
    match raw {
        Json::Object(map) => Ok(map
            .iter()
            .map(|(k, v)| {
                let s = match v {
                    Json::Null => String::new(),
                    Json::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect()),
        Json::String(s) => {
            let t = s.trim();
            if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(t) {
                return style_pairs(&Json::Object(map));
            }
            let braced = if t.starts_with('{') { t.to_string() } else { format!("{{{t}}}") };
            let parsed = parse_attr_block(&braced).ok_or_else(|| format!("can't parse style \"{t}\" — pass an object"))?;
            Ok(parsed.0.into_iter().collect())
        }
        _ => Err("\"style\" must be an object {key: value}".to_string()),
    }
}

/// Поля узла из аргументов; возвращает описание изменений.
fn apply_node_fields(ctx: NotesCtx, handle: &MindmapHandle, id: &str, v: &Json) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if let Some(t) = raw_string(v, "text") {
        handle.set_text(id, &t);
        changes.push("text".to_string());
    }
    if let Some(n) = raw_string(v, "note") {
        handle.set_note(id, &n);
        changes.push("note".to_string());
    }
    if let Some(l) = raw_string(v, "link") {
        let page = if l.trim().is_empty() || l.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(resolve_page_ref(ctx, &l)?)
        };
        handle.set_link(id, page);
        changes.push("link".to_string());
    }
    if let Some(c) = raw_string(v, "color") {
        let color = if c.trim().is_empty() || c.eq_ignore_ascii_case("none") { None } else { Some(parse_hex_color(&c, false)?) };
        handle.set_color(id, color);
        changes.push("color".to_string());
    }
    if let Some(sh) = str_field(v, "shape") {
        let shape = NodeShape::parse(sh)
            .ok_or_else(|| format!("bad shape \"{sh}\" (auto | rect | rounded | pill | ellipse | text)"))?;
        handle.set_shape(id, shape);
        changes.push(format!("shape {}", shape.key()));
    }
    if let Some(i) = raw_string(v, "icon") {
        handle.set_icon(id, &i);
        changes.push("icon".to_string());
    }
    if let Some(c) = bool_field(v, "collapsed") {
        handle.set_collapsed(id, c);
        changes.push(format!("collapsed {c}"));
    }
    let (dx, dy) = (f32_field(v, "dx"), f32_field(v, "dy"));
    if dx.is_some() || dy.is_some() {
        let (cur_dx, cur_dy) = handle.lock().node(id).map(|n| (n.dx, n.dy)).unwrap_or((0.0, 0.0));
        handle.offset(id, dx.unwrap_or(cur_dx) - cur_dx, dy.unwrap_or(cur_dy) - cur_dy);
        changes.push("offset".to_string());
    }
    Ok(changes)
}

pub(super) fn mindmap_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | add_node | update_node | move_node | delete_node | \
         add_link | delete_link | set_layout | set_style | from_list | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let root = str_field(v, "title").unwrap_or(&tr!("notes.mindmap.root")).to_string();
            let doc = match raw_string(v, "outline").filter(|o| !o.trim().is_empty()) {
                Some(outline) => MindmapDoc::from_outline(&outline, &root),
                None => MindmapDoc::template(&root),
            };
            let mut doc = doc;
            if let Some(d) = str_field(v, "direction") {
                doc.layout.direction =
                    Direction::parse(d).ok_or_else(|| format!("bad direction \"{d}\" (right | left | both | down | radial)"))?;
            }
            let id = ctx.create_mindmap(doc);
            let (pos, idx) = embed_object_with(ctx, &pid, "mindmap", &id, v, None)?;
            let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &id) else {
                return Err("map vanished".to_string());
            };
            Ok(format!(
                "created mind map {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                map_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            Ok(format!("{}{}\n", map_text(&id, &handle), object_page_line(ctx, "mindmap", &id)))
        }
        "add_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let parent = match str_field(v, "parent") {
                Some(p) => resolve_node(&handle, p)?,
                None => handle.lock().root_id(),
            };
            let text = str_field(v, "text").unwrap_or("").to_string();
            let index = usize_field(v, "index");
            let mut node = None;
            handle.edit(|d| node = d.add_node(&parent, &text, index));
            let node = node.ok_or("parent vanished")?;
            handle.editing.set(None);
            apply_node_fields(ctx, &handle, &node, v)?;
            Ok(format!("added node {node} \"{text}\"\n{}", map_text(&id, &handle)))
        }
        "update_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let changes = apply_node_fields(ctx, &handle, &node, v)?;
            if changes.is_empty() {
                return Err("nothing to update: pass text, note, link, color, shape, icon, collapsed, dx or dy".to_string());
            }
            Ok(format!("updated node {node}: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "move_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let parent = resolve_node(&handle, str_field(v, "parent").ok_or("missing \"parent\"")?)?;
            if !handle.reparent(&node, &parent, usize_field(v, "index")) {
                return Err("move failed: the root can't be moved and a node can't go into its own subtree".to_string());
            }
            Ok(format!("moved node {node} under {parent}\n{}", map_text(&id, &handle)))
        }
        "delete_node" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let node = resolve_node(&handle, str_field(v, "node").ok_or("missing \"node\"")?)?;
            let n = handle.delete(&node);
            if n == 0 {
                return Err("the root node can't be deleted — delete the whole map with op=delete".to_string());
            }
            Ok(format!("deleted {n} node{}\n{}", if n == 1 { "" } else { "s" }, map_text(&id, &handle)))
        }
        "add_link" | "delete_link" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let from = resolve_node(&handle, str_field(v, "from").ok_or("missing \"from\"")?)?;
            let to = resolve_node(&handle, str_field(v, "to").ok_or("missing \"to\"")?)?;
            let ok = if op == "add_link" {
                handle.add_link(&from, &to, str_field(v, "label").unwrap_or(""))
            } else {
                handle.delete_link(&from, &to)
            };
            if !ok {
                return Err(if op == "add_link" {
                    "link not added: the nodes are the same or the link already exists".to_string()
                } else {
                    "no such link".to_string()
                });
            }
            Ok(format!("{op} {from} → {to}\n{}", map_text(&id, &handle)))
        }
        "set_layout" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let mut changes = Vec::new();
            let direction = match str_field(v, "direction") {
                Some(d) => Some(Direction::parse(d).ok_or_else(|| format!("bad direction \"{d}\" (right | left | both | down | radial)"))?),
                None => None,
            };
            let curve = match str_field(v, "curve") {
                Some(c) => Some(Curve::parse(c).ok_or_else(|| format!("bad curve \"{c}\" (bezier | straight | elbow)"))?),
                None => None,
            };
            let h_gap = f32_field(v, "h_gap");
            let v_gap = f32_field(v, "v_gap");
            if let Some(d) = direction {
                changes.push(format!("direction {}", d.key()));
            }
            if let Some(c) = curve {
                changes.push(format!("curve {}", c.key()));
            }
            if let Some(g) = h_gap {
                changes.push(format!("h_gap {}", fnum(g)));
            }
            if let Some(g) = v_gap {
                changes.push(format!("v_gap {}", fnum(g)));
            }
            if !changes.is_empty() {
                handle.set_layout(|l| {
                    if let Some(d) = direction {
                        l.direction = d;
                    }
                    if let Some(c) = curve {
                        l.curve = c;
                    }
                    if let Some(g) = h_gap {
                        l.h_gap = g;
                    }
                    if let Some(g) = v_gap {
                        l.v_gap = g;
                    }
                });
            }
            if bool_field(v, "reset").unwrap_or(false) {
                handle.reset_offsets();
                changes.push("offsets reset".to_string());
            }
            if changes.is_empty() {
                return Err("nothing to change: pass direction, curve, h_gap, v_gap or reset".to_string());
            }
            Ok(format!("layout: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "set_style" => {
            let (id, handle) = mindmap_handle(ctx, v)?;
            let changes = apply_map_style(&handle, v)?;
            if changes.is_empty() {
                return Err("nothing to change: pass style with palette, colors, font_size, radius, padding, lines or show_icons".to_string());
            }
            Ok(format!("style: {}\n{}", changes.join(", "), map_text(&id, &handle)))
        }
        "from_list" => {
            let pid = page_arg(ctx, v, "page")?;
            let mut model = load_model(ctx, &pid);
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\" (list or heading to convert)")?)?;
            let md = block_markdown(&model.blocks[i]);
            let root = str_field(v, "title").unwrap_or(&tr!("notes.mindmap.root")).to_string();
            let doc = MindmapDoc::from_outline(&md, &root);
            let nodes = doc.nodes.len();
            let id = ctx.create_mindmap(doc);
            let geom = model.blocks[i].attrs.clone();
            let embed = format!("![[mindmap:{id}]]{{h={}}}", fnum(embeds::default_object_h("mindmap")));
            let idx = replace_block(&mut model, i, &embed)?;
            // Врезка встаёт на место блока и наследует его координаты.
            for (k, val) in geom.0.iter() {
                if free::is_geom_key(k) && model.blocks[idx[0]].attrs.get(k).is_none() {
                    model.blocks[idx[0]].attrs.set(k.clone(), val.clone());
                }
            }
            store_model(ctx, &pid, &model)?;
            let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &id) else {
                return Err("map vanished".to_string());
            };
            Ok(format!(
                "block #{i} → mind map with {nodes} nodes\n{}{}\n",
                map_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "delete" => {
            let (id, _) = mindmap_handle(ctx, v)?;
            delete_object(ctx, "mindmap", &id)
        }
        other => Err(format!(
            "unknown mindmap op \"{other}\" (create | read | add_node | update_node | move_node | \
             delete_node | add_link | delete_link | set_layout | set_style | from_list | delete)"
        )),
    }
}
