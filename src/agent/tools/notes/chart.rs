//! Действие `chart`: график — данные, вид, подписи, ряды, оформление.

use super::*;

fn chart_handle(ctx: NotesCtx, v: &Json) -> Result<(String, ChartHandle), String> {
    match resolve_object(ctx, v, "chart", "chart")? {
        LiveObject::Chart { id, handle } => Ok((id, handle)),
        _ => Err("not a chart".to_string()),
    }
}

/// Число из поля: и `12`, и `"12"` — модели шлют по-разному.
pub(super) fn f64_field(v: &Json, key: &str) -> Option<f64> {
    match v.get(key)? {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Ряд чисел из поля: массив либо строка «12, 24, 18».
pub(super) fn data_field(v: &Json, key: &str) -> Option<Vec<f64>> {
    match v.get(key)? {
        Json::Array(a) => Some(
            a.iter()
                .map(|x| match x {
                    Json::Number(n) => n.as_f64().unwrap_or(0.0),
                    Json::String(s) => chart_model::parse_num(s),
                    _ => 0.0,
                })
                .collect(),
        ),
        Json::String(s) => Some(chart_model::parse_values(s)),
        Json::Number(n) => Some(vec![n.as_f64().unwrap_or(0.0)]),
        _ => None,
    }
}

/// Ряд графика по id или названию.
fn resolve_series(handle: &ChartHandle, s: &str) -> Result<String, String> {
    handle
        .lock()
        .find_series(s)
        .map(|x| x.id.clone())
        .ok_or_else(|| format!("series \"{s}\" not found — ids and names are in chart op=read"))
}

/// Вид графика из поля `kind`.
fn chart_kind_field(v: &Json, key: &str) -> Result<Option<ChartKind>, String> {
    match str_field(v, key) {
        Some(k) => Ok(Some(
            ChartKind::parse(k).ok_or_else(|| format!("bad kind \"{k}\" (line | bar | pie | radar | gauge)"))?,
        )),
        None => Ok(None),
    }
}

/// Текст графика: шапка с видом и настройками, подписи и ряды таблицей.
pub(super) fn chart_text(id: &str, handle: &ChartHandle) -> String {
    let doc = handle.lock();
    let o = &doc.options;
    let mut out = format!(
        "chart:{id} · kind: {} · series: {} · points: {}{}\n",
        doc.kind.key(),
        doc.series.len(),
        doc.categories.len(),
        if doc.title.is_empty() { String::new() } else { format!(" · title: \"{}\"", doc.title) }
    );
    match doc.kind {
        ChartKind::Gauge => {
            out.push_str(&format!(
                "  value {} · range {}…{}{}{}\n",
                chart_model::fmt_num(doc.gauge_value()),
                chart_model::fmt_num(o.gauge_min),
                chart_model::fmt_num(o.gauge_max),
                if o.unit.is_empty() { String::new() } else { format!(" · unit \"{}\"", o.unit) },
                if o.zones.is_empty() { String::new() } else { format!(" · {} zone(s)", o.zones.len()) }
            ));
        }
        _ => {
            out.push_str(&format!("  labels: {}\n", doc.categories.join(", ")));
            for s in &doc.series {
                out.push_str(&format!(
                    "  series {} \"{}\" · {}{}\n",
                    s.id,
                    s.name,
                    chart_model::values_text(&s.data),
                    if s.color.is_empty() { String::new() } else { format!(" · color {}", s.color) }
                ));
            }
        }
    }
    out.push_str(&format!("  legend {} · tooltip {} · animate {}", o.legend.key(), o.tooltip, o.animate));
    match doc.kind {
        ChartKind::Line => out.push_str(&format!(" · smooth {} · points {} · area {}", o.smooth, o.points, o.area)),
        ChartKind::Bar => out.push_str(&format!(
            " · stacked {} · horizontal {} · value_labels {}",
            o.stacked, o.horizontal, o.value_labels
        )),
        ChartKind::Pie => out.push_str(&format!(
            " · donut {} · labels {} · percentage {}",
            o.donut,
            o.pie_labels.key(),
            o.percentage
        )),
        ChartKind::Radar => out.push_str(&format!(
            " · grid {} · levels {} · max {}",
            if o.radar_circle { "circle" } else { "polygon" },
            o.radar_levels,
            o.radar_max.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string())
        )),
        ChartKind::Gauge => out.push_str(&format!(" · needle {} · ticks {} · labels {}", o.needle, o.ticks, o.gauge_labels)),
    }
    if doc.kind.has_axes() {
        out.push_str(&format!(
            " · grid {} · y {}…{}",
            o.grid,
            o.y_min.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string()),
            o.y_max.map(chart_model::fmt_num).unwrap_or_else(|| "auto".to_string())
        ));
    }
    out.push('\n');
    out
}

/// Данные графика из аргументов: таблица целиком либо подписи и ряд.
fn apply_chart_data(handle: &ChartHandle, v: &Json) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if let Some(table) = raw_string(v, "table").filter(|t| !t.trim().is_empty()) {
        let kind = handle.kind();
        let fresh = ChartDoc::from_table(&table, kind);
        handle.edit(|d| {
            d.categories = fresh.categories.clone();
            d.series = fresh.series.clone();
        });
        changes.push(format!("{} labels, {} series from the table", fresh.categories.len(), fresh.series.len()));
    }
    if let Some(labels) = list_field(v, "categories") {
        handle.set_categories(labels.clone());
        changes.push(format!("{} labels", labels.len()));
    }
    if let Some(data) = data_field(v, "data") {
        // Ряд по имени/id, а без него — первый: у круговой и шкалы он
        // единственный, и указывать его каждый раз бессмысленно.
        let sid = match str_field(v, "series") {
            Some(s) => resolve_series(handle, s)?,
            None => handle.lock().series.first().map(|s| s.id.clone()).unwrap_or_default(),
        };
        match f64_field(v, "value").zip(usize_field(v, "index")) {
            Some((value, i)) => {
                handle.set_value(&sid, i, value);
                changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
            }
            None => {
                handle.set_series_data(&sid, data.clone());
                changes.push(format!("{} values", data.len()));
            }
        }
    } else if let Some(value) = f64_field(v, "value") {
        let sid = match str_field(v, "series") {
            Some(s) => resolve_series(handle, s)?,
            None => handle.lock().series.first().map(|s| s.id.clone()).unwrap_or_default(),
        };
        let i = usize_field(v, "index").unwrap_or(0);
        handle.set_value(&sid, i, value);
        changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
    }
    Ok(changes)
}

/// Оформление графика из объекта `style`.
fn apply_chart_style(handle: &ChartHandle, v: &Json) -> Result<Vec<String>, String> {
    let Some(raw) = v.get("style") else { return Ok(Vec::new()) };
    let pairs = style_pairs(raw)?;
    let mut changes = Vec::new();
    let mut err = None;
    handle.set_options(|o| {
        for (k, val) in &pairs {
            let key = k.trim().to_ascii_lowercase();
            let val = val.trim();
            let flag = |err: &mut Option<String>| -> Option<bool> {
                match val.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "1" | "on" => Some(true),
                    "false" | "no" | "0" | "off" => Some(false),
                    _ => {
                        *err = Some(format!("bad \"{key}\" \"{val}\" — true or false"));
                        None
                    }
                }
            };
            let num = |err: &mut Option<String>| -> Option<f64> {
                match val.parse::<f64>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        *err = Some(format!("bad \"{key}\" \"{val}\" — a number"));
                        None
                    }
                }
            };
            // Пустое значение и «auto» снимают границу оси.
            let auto = val.is_empty() || val.eq_ignore_ascii_case("auto") || val.eq_ignore_ascii_case("none");
            match key.as_str() {
                "legend" => match LegendPos::parse(val) {
                    Some(p) => o.legend = p,
                    None => {
                        err = Some(format!("bad legend \"{val}\" (top | bottom | left | right | none)"));
                        return;
                    }
                },
                "pie_labels" | "labels" => match PieLabels::parse(val) {
                    Some(p) => o.pie_labels = p,
                    None => {
                        err = Some(format!("bad pie_labels \"{val}\" (outside | inside | none)"));
                        return;
                    }
                },
                "tooltip" | "animate" | "grid" | "smooth" | "points" | "stacked" | "horizontal"
                | "value_labels" | "percentage" | "radar_circle" | "needle" | "ticks" | "gauge_labels" => {
                    let Some(b) = flag(&mut err) else { return };
                    match key.as_str() {
                        "tooltip" => o.tooltip = b,
                        "animate" => o.animate = b,
                        "grid" => o.grid = b,
                        "smooth" => o.smooth = b,
                        "points" => o.points = b,
                        "stacked" => o.stacked = b,
                        "horizontal" => o.horizontal = b,
                        "value_labels" => o.value_labels = b,
                        "percentage" => o.percentage = b,
                        "radar_circle" => o.radar_circle = b,
                        "needle" => o.needle = b,
                        "ticks" => o.ticks = b,
                        _ => o.gauge_labels = b,
                    }
                }
                "area" | "donut" | "bar_radius" | "radar_levels" | "gauge_min" | "gauge_max" => {
                    let Some(n) = num(&mut err) else { return };
                    match key.as_str() {
                        "area" => o.area = n as f32,
                        "donut" => o.donut = n as f32,
                        "bar_radius" => o.bar_radius = n as f32,
                        "radar_levels" => o.radar_levels = n.max(1.0) as usize,
                        "gauge_min" => o.gauge_min = n,
                        _ => o.gauge_max = n,
                    }
                }
                "y_min" | "y_max" | "radar_max" => {
                    let value = if auto {
                        None
                    } else {
                        let Some(n) = num(&mut err) else { return };
                        Some(n)
                    };
                    match key.as_str() {
                        "y_min" => o.y_min = value,
                        "y_max" => o.y_max = value,
                        _ => o.radar_max = value,
                    }
                }
                "x_title" | "y_title" | "unit" => {
                    let text = if auto { String::new() } else { val.to_string() };
                    match key.as_str() {
                        "x_title" => o.x_title = text,
                        "y_title" => o.y_title = text,
                        _ => o.unit = text,
                    }
                }
                "zones" => {
                    match parse_zones(val) {
                        Ok(z) => o.zones = z,
                        Err(e) => {
                            err = Some(e);
                            return;
                        }
                    };
                }
                other => {
                    err = Some(format!(
                        "unknown style key \"{other}\" (legend, tooltip, animate, grid, x_title, y_title, \
                         y_min, y_max, smooth, points, area, stacked, horizontal, value_labels, bar_radius, \
                         donut, pie_labels, percentage, radar_circle, radar_levels, radar_max, gauge_min, \
                         gauge_max, needle, ticks, gauge_labels, unit, zones)"
                    ));
                    return;
                }
            }
            changes.push(key);
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(changes),
    }
}

/// Зоны шкалы: «0-50 green, 50-80 #E8A33D» либо JSON-массив
/// `[{"from":0,"to":50,"color":"green"}]`.
fn parse_zones(raw: &str) -> Result<Vec<GaugeZone>, String> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    if let Ok(Json::Array(items)) = serde_json::from_str::<Json>(t) {
        let mut out = Vec::new();
        for it in &items {
            let from = f64_field(it, "from").ok_or("zone needs \"from\"")?;
            let to = f64_field(it, "to").ok_or("zone needs \"to\"")?;
            let color = str_field(it, "color").unwrap_or("#4FBF7A");
            out.push(GaugeZone { from, to, color: parse_hex_color(color, false)? });
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    for part in t.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (range, color) = part.split_once(char::is_whitespace).unwrap_or((part, "#4FBF7A"));
        let (from, to) = range
            .split_once(['-', '…', ':'])
            .ok_or_else(|| format!("bad zone \"{part}\" — «from-to color», e.g. «0-50 green»"))?;
        out.push(GaugeZone {
            from: chart_model::parse_num(from),
            to: chart_model::parse_num(to),
            color: parse_hex_color(color.trim(), false)?,
        });
    }
    Ok(out)
}

pub(super) fn chart_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or(
        "missing \"op\" (create | read | update | set_data | add_series | update_series | \
         delete_series | set_style | from_table | delete)",
    )?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let kind = chart_kind_field(v, "kind")?.unwrap_or(ChartKind::Line);
            let mut doc = match raw_string(v, "table").filter(|t| !t.trim().is_empty()) {
                Some(table) => ChartDoc::from_table(&table, kind),
                None => ChartDoc::template(kind, &tr!("notes.chart.series")),
            };
            if let Some(t) = str_field(v, "title") {
                doc.title = t.trim().to_string();
            }
            if let Some(labels) = list_field(v, "categories") {
                doc.categories = labels;
            }
            if let Some(data) = data_field(v, "data") {
                let name = str_field(v, "name").unwrap_or(&tr!("notes.chart.series")).to_string();
                doc.series = vec![chart_model::ChartSeries::new(&name, data)];
            }
            doc.sanitize();
            let id = ctx.create_chart(doc);
            let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &id) else {
                return Err("chart vanished".to_string());
            };
            apply_chart_style(&handle, v)?;
            let (pos, idx) = embed_object_with(ctx, &pid, "chart", &id, v, None)?;
            Ok(format!(
                "created chart {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                chart_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = chart_handle(ctx, v)?;
            Ok(format!(
                "{}{}{}\n",
                chart_text(&id, &handle),
                handle.lock().to_table(),
                object_page_line(ctx, "chart", &id)
            ))
        }
        "update" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let mut changes = Vec::new();
            if let Some(kind) = chart_kind_field(v, "kind")? {
                handle.set_kind(kind);
                changes.push(format!("kind {}", kind.key()));
            }
            if let Some(t) = str_field(v, "title") {
                handle.set_title(t);
                changes.push("title".to_string());
            }
            changes.extend(apply_chart_data(&handle, v)?);
            changes.extend(apply_chart_style(&handle, v)?);
            if changes.is_empty() {
                return Err("nothing to update: pass kind, title, categories, data, table or style".to_string());
            }
            Ok(format!("updated chart: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "set_data" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let changes = apply_chart_data(&handle, v)?;
            if changes.is_empty() {
                return Err("nothing to set: pass table, categories, data or value with index".to_string());
            }
            Ok(format!("data: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "add_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            if handle.kind().single_series() {
                return Err(format!(
                    "a {} chart shows one series — change its data instead (op=set_data) or switch the kind",
                    handle.kind().key()
                ));
            }
            let name = str_field(v, "name")
                .map(str::to_string)
                .unwrap_or_else(|| format!("{} {}", tr!("notes.chart.series"), handle.lock().series.len() + 1));
            let sid = handle.add_series(&name, data_field(v, "data").unwrap_or_default());
            if let Some(c) = str_field(v, "color") {
                handle.set_series_color(&sid, chart_color(c)?);
            }
            Ok(format!("added series {sid} \"{name}\"\n{}", chart_text(&id, &handle)))
        }
        "update_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let sid = match str_field(v, "series") {
                Some(s) => resolve_series(&handle, s)?,
                None => handle.lock().series.first().map(|s| s.id.clone()).ok_or("the chart has no series")?,
            };
            let mut changes = Vec::new();
            if let Some(name) = str_field(v, "name") {
                handle.rename_series(&sid, name);
                changes.push("name".to_string());
            }
            if let Some(c) = str_field(v, "color") {
                handle.set_series_color(&sid, chart_color(c)?);
                changes.push("color".to_string());
            }
            if let Some(data) = data_field(v, "data") {
                handle.set_series_data(&sid, data.clone());
                changes.push(format!("{} values", data.len()));
            }
            if let (Some(value), Some(i)) = (f64_field(v, "value"), usize_field(v, "index")) {
                handle.set_value(&sid, i, value);
                changes.push(format!("point {i} = {}", chart_model::fmt_num(value)));
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, color, data, or value with index".to_string());
            }
            Ok(format!("updated series {sid}: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "delete_series" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let sid = resolve_series(&handle, str_field(v, "series").ok_or("missing \"series\"")?)?;
            if !handle.delete_series(&sid) {
                return Err("the last series can't be deleted — delete the whole chart with op=delete".to_string());
            }
            Ok(format!("deleted series {sid}\n{}", chart_text(&id, &handle)))
        }
        "set_style" => {
            let (id, handle) = chart_handle(ctx, v)?;
            let changes = apply_chart_style(&handle, v)?;
            if changes.is_empty() {
                return Err(
                    "nothing to change: pass style with legend, tooltip, animate, grid, axis titles, \
                     y_min/y_max, smooth, points, area, stacked, horizontal, value_labels, bar_radius, \
                     donut, pie_labels, percentage, radar_circle, radar_levels, radar_max, gauge_min, \
                     gauge_max, needle, ticks, gauge_labels, unit or zones"
                        .to_string(),
                );
            }
            Ok(format!("style: {}\n{}", changes.join(", "), chart_text(&id, &handle)))
        }
        "from_table" => {
            let pid = page_arg(ctx, v, "page")?;
            let mut model = load_model(ctx, &pid);
            let i = resolve_block(&model, &ref_field(v, "block").ok_or("missing \"block\" (table to convert)")?)?;
            let md = block_markdown(&model.blocks[i]);
            if !md.trim_start().starts_with('|') {
                return Err(format!("block #{i} is not a table — chart op=from_table converts markdown tables"));
            }
            let kind = chart_kind_field(v, "kind")?.unwrap_or(ChartKind::Bar);
            let mut doc = ChartDoc::from_table(&md, kind);
            if let Some(t) = str_field(v, "title") {
                doc.title = t.trim().to_string();
            }
            let points = doc.categories.len();
            let rows = doc.series.len();
            let id = ctx.create_chart(doc);
            let geom = model.blocks[i].attrs.clone();
            let embed = format!("![[chart:{id}]]{{h={}}}", fnum(embeds::default_object_h("chart")));
            let idx = replace_block(&mut model, i, &embed)?;
            // Врезка встаёт на место таблицы и наследует её координаты.
            for (k, val) in geom.0.iter() {
                if free::is_geom_key(k) && model.blocks[idx[0]].attrs.get(k).is_none() {
                    model.blocks[idx[0]].attrs.set(k.clone(), val.clone());
                }
            }
            store_model(ctx, &pid, &model)?;
            let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &id) else {
                return Err("chart vanished".to_string());
            };
            apply_chart_style(&handle, v)?;
            Ok(format!(
                "block #{i} → chart with {rows} series over {points} points\n{}{}\n",
                chart_text(&id, &handle),
                page_line(ctx, &pid)
            ))
        }
        "delete" => {
            let (id, _) = chart_handle(ctx, v)?;
            delete_object(ctx, "chart", &id)
        }
        other => Err(format!(
            "unknown chart op \"{other}\" (create | read | update | set_data | add_series | \
             update_series | delete_series | set_style | from_table | delete)"
        )),
    }
}

/// Цвет ряда или доли: `none` снимает его (тогда цвет берётся из палитры).
fn chart_color(s: &str) -> Result<Option<String>, String> {
    if s.trim().is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    parse_hex_color(s, false).map(Some)
}
