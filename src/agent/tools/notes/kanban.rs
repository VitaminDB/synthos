//! Действие `kanban`: доски, колонки, карточки, метки и стиль доски.

use super::*;

/// Цвет метки: `#rrggbb`, имя из палитры либо пусто/`none`.
pub(super) fn parse_color(s: &str) -> Result<String, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(String::new());
    }
    let named = match lower.as_str() {
        "gray" | "grey" => Some(PALETTE[0]),
        "orange" | "yellow" => Some(PALETTE[1]),
        "green" => Some(PALETTE[2]),
        "blue" => Some(PALETTE[3]),
        "purple" | "violet" => Some(PALETTE[4]),
        "red" => Some(PALETTE[5]),
        "teal" | "cyan" => Some(PALETTE[6]),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c.to_string());
    }
    let hex = t.strip_prefix('#').unwrap_or(t);
    if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let full: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_string()
        };
        return Ok(format!("#{}", full.to_uppercase()));
    }
    Err(format!(
        "bad color \"{s}\" — use #rrggbb or gray | orange | green | blue | purple | red | teal | none"
    ))
}

pub(super) fn parse_priority(s: &str) -> Result<Option<Priority>, String> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    Priority::parse(&lower)
        .map(Some)
        .ok_or_else(|| format!("bad priority \"{s}\" (low | medium | high | urgent | none)"))
}

pub(super) fn parse_due(s: &str) -> Result<Option<String>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    if lower == "today" {
        return Ok(Some(days_to_iso(today_days())));
    }
    if lower == "tomorrow" {
        return Ok(Some(days_to_iso(today_days() + 1)));
    }
    let days = parse_days(t).ok_or_else(|| format!("bad date \"{s}\" (yyyy-mm-dd)"))?;
    Ok(Some(days_to_iso(days)))
}

/// Момент плана карточки: `yyyy-mm-dd`, `yyyy-mm-ddThh:mm`, `today`,
/// `tomorrow` (можно со временем: `today 10:00`); пусто/`none` — снять.
fn parse_plan_moment(s: &str) -> Result<Option<String>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" {
        return Ok(None);
    }
    for (word, delta) in [("today", 0i64), ("tomorrow", 1)] {
        let Some(rest) = lower.strip_prefix(word) else { continue };
        let rest = rest.trim_start_matches(['t', ' ']).trim();
        let min = match rest.is_empty() {
            true => None,
            false => Some(parse_hm(rest).ok_or_else(|| format!("bad time \"{rest}\" (hh:mm)"))?),
        };
        return Ok(Some(Moment { day: today_days() + delta, min }.iso()));
    }
    Moment::parse(t)
        .map(|m| Some(m.iso()))
        .ok_or_else(|| format!("bad date \"{s}\" (yyyy-mm-dd | yyyy-mm-ddThh:mm | today | tomorrow | none)"))
}

/// Оценка длительности: `1d`, `2h`, `90m`, `1d 4h`, голое число — часы;
/// пусто/`none` — снять.
fn parse_duration_arg(s: &str) -> Result<Option<u32>, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "null" || lower == "0" {
        return Ok(None);
    }
    parse_duration(t)
        .map(Some)
        .ok_or_else(|| format!("bad duration \"{s}\" (1d | 2h | 90m | \"1d 4h\" | a bare number = hours | none)"))
}

/// Повтор карточки: `none | daily | weekly | monthly | yearly`.
fn parse_repeat(s: &str) -> Result<Repeat, String> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() || t == "null" || t == "off" {
        return Ok(Repeat::None);
    }
    Repeat::parse(&t).ok_or_else(|| format!("bad repeat \"{s}\" (none | daily | weekly | monthly | yearly)"))
}

/// Колонка по id, названию (без регистра) или номеру с единицы.
fn resolve_column(handle: &KanbanHandle, s: &str) -> Result<KanbanColumn, String> {
    let doc = handle.lock();
    let key = s.trim().to_lowercase();
    if let Some(c) = doc.columns.iter().find(|c| c.id == s.trim()) {
        return Ok(c.clone());
    }
    let by_name: Vec<&KanbanColumn> = doc.columns.iter().filter(|c| c.name.trim().to_lowercase() == key).collect();
    if by_name.len() == 1 {
        return Ok(by_name[0].clone());
    }
    if let Ok(n) = key.parse::<usize>() {
        if (1..=doc.columns.len()).contains(&n) {
            return Ok(doc.columns[n - 1].clone());
        }
    }
    Err(format!(
        "column \"{s}\" not found — columns: {}",
        doc.columns.iter().map(|c| format!("\"{}\" ({})", c.name, c.id)).collect::<Vec<_>>().join(", ")
    ))
}

/// Карточка по id либо заголовку (без регистра, единственная).
pub(super) fn resolve_card(handle: &KanbanHandle, s: &str) -> Result<KanbanCard, String> {
    let doc = handle.lock();
    if let Some(c) = doc.card(s.trim()) {
        return Ok(c.clone());
    }
    let key = s.trim().to_lowercase();
    let hits: Vec<&KanbanCard> = doc.cards.iter().filter(|c| c.title.trim().to_lowercase() == key).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!("card \"{s}\" not found — ids and titles are in kanban op=read")),
        n => Err(format!(
            "{n} cards are titled \"{s}\" — use the id: {}",
            hits.iter().map(|c| c.id.clone()).collect::<Vec<_>>().join(", ")
        )),
    }
}

pub(super) fn card_line(c: &KanbanCard) -> String {
    let mut s = format!("{} \"{}\"", c.id, c.title);
    if let Some(p) = c.priority {
        s.push_str(&format!(" · priority: {}", p.key()));
    }
    if !c.tags.is_empty() {
        s.push_str(&format!(" · tags: {}", c.tags.join(", ")));
    }
    if let Some(d) = &c.due {
        s.push_str(&format!(" · due: {d}"));
    }
    if let Some(d) = c.duration {
        s.push_str(&format!(" · duration: {}", fmt_duration(d)));
    }
    let span = c.span_text();
    if !span.is_empty() {
        s.push_str(&format!(" · planned: {span}"));
    }
    if let Some((done, total)) = c.checklist() {
        s.push_str(&format!(" · checklist {done}/{total}"));
    }
    if c.repeat != Repeat::None {
        s.push_str(&format!(" · repeat: {}", c.repeat.key()));
    }
    if let Some(d) = &c.created {
        s.push_str(&format!(" · created: {d}"));
    }
    if let Some(d) = &c.done {
        s.push_str(&format!(" · done: {d}"));
    }
    if !c.files.is_empty() {
        s.push_str(&format!(" · files: {}", c.files.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")));
    }
    s
}

/// Текст доски: шапка, колонки, карточки по колонкам; `full` — с
/// markdown-содержимым карточек.
pub(super) fn board_text(id: &str, handle: &KanbanHandle, full: bool) -> String {
    let doc = handle.lock();
    let mut out = String::new();
    out.push_str(&format!(
        "kanban:{id} · columns: {} · cards: {}{}{}{}\n",
        doc.columns.len(),
        doc.cards.len(),
        if doc.archive.is_empty() { String::new() } else { format!(" · archived: {}", doc.archive.len()) },
        doc.archive_after.map(|d| format!(" · archive_after: {d} days")).unwrap_or_default(),
        if full {
            format!(
                " · style: column_width={} lane_bg={} card_bg={} counts={}",
                doc.style.column_width,
                if doc.style.lane_bg.is_empty() { "theme" } else { doc.style.lane_bg.as_str() },
                if doc.style.card_bg.is_empty() { "theme" } else { doc.style.card_bg.as_str() },
                if doc.style.show_counts { "on" } else { "off" }
            )
        } else {
            String::new()
        }
    ));
    if doc.done_column().is_none() {
        out.push_str("  !! no done column: cards never get a done stamp — kanban op=update_column column=<name> done=true\n");
    }
    for col in &doc.columns {
        let cards = doc.cards_of(&col.id);
        out.push_str(&format!(
            "  column {} \"{}\"{}{}{} · {} card{}\n",
            col.id,
            col.name,
            if col.done { " · DONE column" } else { "" },
            if col.color.is_empty() { String::new() } else { format!(" · color {}", col.color) },
            col.width.map(|w| format!(" · width {w}")).unwrap_or_default(),
            cards.len(),
            if cards.len() == 1 { "" } else { "s" }
        ));
        for c in cards {
            out.push_str(&format!("    {}\n", card_line(c)));
            if full && !c.md.trim().is_empty() {
                for l in c.md.trim_end().lines() {
                    out.push_str(&format!("      | {l}\n"));
                }
            }
        }
    }
    out
}

/// Шпаргалка операций доски с их полями — уходит модели вместо голого
/// списка имён, когда `op` пропущен или неизвестен. Живой чат (MyLife,
/// 08.09.2026): модель дважды подряд собирала `add_card` без `op` и с текстом
/// карточки в `card`; список имён ей не помог, сигнатуры — помогают.
const KANBAN_OPS_HELP: &str = "kanban ops (every op but create addresses the board by board=<id> \
or page=<page with one board>): \
create {page, columns, done_column, title, x y w h} · read {archived} · \
set_style {column_width, lane_bg, card_bg, show_counts, archive_after} · \
add_column {name, color, width, done} · update_column {column, name, color, width, done} · \
delete_column {column} · \
add_card {title = the new card's text, column, md, priority, tags, due, duration, start, end, repeat, before} · \
update_card {card, title, md, priority, tags, due, duration, start, end, repeat, column, before} · \
schedule {card, date = today | tomorrow | yyyy-mm-dd, time, duration} · unschedule {card} · \
move_card {card, column | to_board, before} · delete_card {card} · archive {card} · \
unarchive {card} · attach {card, path | attachment, name} · detach {card, file} · delete. \
\"card\" = an existing card (id or exact title); a new card's text goes in \"title\".";

pub(super) fn kanban_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op").ok_or_else(|| format!("missing \"op\". {KANBAN_OPS_HELP}"))?;
    match op {
        "create" => {
            let pid = page_arg(ctx, v, "page")?;
            let id = ctx.create_object("kanban").ok_or("failed to create the board")?;
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &id) else {
                return Err("board vanished".to_string());
            };
            if let Some(names) = list_field(v, "columns").filter(|c| !c.is_empty()) {
                handle.edit(|doc| {
                    doc.columns = names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| KanbanColumn {
                            id: item_id("c"),
                            name: name.clone(),
                            color: PALETTE[i % PALETTE.len()].to_string(),
                            width: None,
                            done: false,
                        })
                        .collect();
                    doc.detect_done_column();
                });
            }
            if let Some(dc) = str_field(v, "done_column") {
                let col = resolve_column(&handle, dc)?;
                handle.edit(|doc| {
                    for c in &mut doc.columns {
                        c.done = c.id == col.id;
                    }
                });
            }
            let (pos, idx) = embed_object(ctx, &pid, "kanban", &id, v)?;
            Ok(format!(
                "created board {} ({})\n{}{}\n",
                pos_text(pos),
                indices_text(&idx),
                board_text(&id, &handle, false),
                page_line(ctx, &pid)
            ))
        }
        "read" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let mut out = board_text(&id, &handle, true);
            if bool_field(v, "archived").unwrap_or(false) {
                let doc = handle.lock();
                out.push_str(&format!("--- Archive ({}) ---\n", doc.archive.len()));
                for c in &doc.archive {
                    out.push_str(&format!("    {}\n", card_line(c)));
                }
            }
            Ok(format!("{out}{}\n", object_page_line(ctx, "kanban", &id)))
        }
        "set_style" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let mut changes = Vec::new();
            if let Some(raw) = raw_string(v, "archive_after") {
                let t = raw.trim().to_ascii_lowercase();
                let days = if t.is_empty() || t == "none" || t == "off" || t == "0" {
                    None
                } else {
                    Some(t.parse::<u32>().map_err(|_| format!("bad \"archive_after\" \"{raw}\" (days, 0/none = off)"))?)
                };
                handle.set_archive_after(days);
                changes.push(days.map(|d| format!("archive_after {d} days")).unwrap_or_else(|| "archive_after off".to_string()));
            }
            let column_width = f32_field(v, "column_width");
            let lane_bg = match raw_string(v, "lane_bg") {
                Some(c) => Some(parse_hex_color(&c, true).or_else(|e| if c.trim().is_empty() { Ok(String::new()) } else { Err(e) })?),
                None => None,
            };
            let card_bg = match raw_string(v, "card_bg") {
                Some(c) => Some(parse_hex_color(&c, true).or_else(|e| if c.trim().is_empty() { Ok(String::new()) } else { Err(e) })?),
                None => None,
            };
            let show_counts = bool_field(v, "show_counts");
            if let Some(w) = column_width {
                if !(crate::pages::notes::kanban::model::MIN_COLUMN_WIDTH..=crate::pages::notes::kanban::model::MAX_COLUMN_WIDTH).contains(&w) {
                    return Err(format!(
                        "\"column_width\" must be within {}..{} px",
                        crate::pages::notes::kanban::model::MIN_COLUMN_WIDTH,
                        crate::pages::notes::kanban::model::MAX_COLUMN_WIDTH
                    ));
                }
                changes.push(format!("column_width {}", fnum(w)));
            }
            if let Some(c) = &lane_bg {
                changes.push(if c.is_empty() { "lane_bg cleared".to_string() } else { format!("lane_bg {c}") });
            }
            if let Some(c) = &card_bg {
                changes.push(if c.is_empty() { "card_bg cleared".to_string() } else { format!("card_bg {c}") });
            }
            if let Some(s) = show_counts {
                changes.push(format!("counts {}", if s { "on" } else { "off" }));
            }
            if changes.is_empty() {
                return Err("nothing to change: pass column_width, lane_bg, card_bg, show_counts or archive_after".to_string());
            }
            handle.set_style(|s| {
                if let Some(w) = column_width {
                    s.column_width = w;
                }
                if let Some(c) = lane_bg {
                    s.lane_bg = c;
                }
                if let Some(c) = card_bg {
                    s.card_bg = c;
                }
                if let Some(v) = show_counts {
                    s.show_counts = v;
                }
            });
            Ok(format!("style: {}\n{}", changes.join(", "), board_text(&id, &handle, true)))
        }
        "add_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let name = str_field(v, "name").ok_or("missing \"name\"")?;
            let cid = handle.add_column(name);
            if let Some(c) = str_field(v, "color") {
                let color = parse_color(c)?;
                handle.edit(|doc| {
                    if let Some(col) = doc.columns.iter_mut().find(|c| c.id == cid) {
                        col.color = color;
                    }
                });
            }
            if let Some(w) = f32_field(v, "width") {
                handle.set_column_width(&cid, (w > 0.0).then_some(w));
            }
            if let Some(d) = bool_field(v, "done") {
                handle.set_column_done(&cid, d);
            }
            Ok(format!("added column {cid} \"{name}\"\n{}", board_text(&id, &handle, false)))
        }
        "update_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let col = resolve_column(&handle, str_field(v, "column").ok_or("missing \"column\"")?)?;
            let mut changes = Vec::new();
            if let Some(name) = str_field(v, "name") {
                handle.rename_column(&col.id, name);
                changes.push(format!("renamed to \"{name}\""));
            }
            if let Some(c) = str_field(v, "color") {
                let color = parse_color(c)?;
                let cid = col.id.clone();
                handle.edit(|doc| {
                    if let Some(col) = doc.columns.iter_mut().find(|c| c.id == cid) {
                        col.color = color;
                    }
                });
                changes.push(format!("color {c}"));
            }
            if let Some(w) = f32_field(v, "width") {
                handle.set_column_width(&col.id, (w > 0.0).then_some(w));
                changes.push(format!("width {w}"));
            }
            if let Some(d) = bool_field(v, "done") {
                handle.set_column_done(&col.id, d);
                changes.push(if d { "done column".to_string() } else { "not a done column".to_string() });
            }
            if changes.is_empty() {
                return Err("nothing to update: pass name, color, width or done".to_string());
            }
            Ok(format!("column {}: {}\n{}", col.id, changes.join(", "), board_text(&id, &handle, false)))
        }
        "delete_column" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let col = resolve_column(&handle, str_field(v, "column").ok_or("missing \"column\"")?)?;
            handle.delete_column(&col.id);
            Ok(format!(
                "deleted column \"{}\" (its cards moved to the neighbouring column)\n{}",
                col.name,
                board_text(&id, &handle, false)
            ))
        }
        "add_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let column = match str_field(v, "column") {
                Some(c) => resolve_column(&handle, c)?,
                None => handle.lock().columns.first().cloned().ok_or("the board has no columns")?,
            };
            // Текст новой карточки модель кладёт в "card" — в остальных op так
            // адресуют существующую, а у add_card другого смысла у поля нет.
            let title = str_field(v, "title")
                .or_else(|| str_field(v, "card"))
                .ok_or("missing \"title\" — the new card's text (\"card\" names an existing card in update_card/move_card/delete_card)")?;
            let mut card = KanbanCard::new(item_id("k"), column.id.clone());
            card.title = title.to_string();
            card.md = raw_string(v, "md").unwrap_or_default().trim().to_string();
            if let Some(p) = str_field(v, "priority") {
                card.priority = parse_priority(p)?;
            }
            if let Some(tags) = list_field(v, "tags") {
                card.tags = parse_tags(&tags.join(","));
            }
            if let Some(d) = str_field(v, "due") {
                card.due = parse_due(d)?;
            }
            if let Some(r) = str_field(v, "repeat") {
                card.repeat = parse_repeat(r)?;
            }
            if let Some(d) = str_field(v, "duration") {
                card.duration = parse_duration_arg(d)?;
            }
            if let Some(d) = str_field(v, "start") {
                card.start = parse_plan_moment(d)?;
            }
            if let Some(d) = str_field(v, "end") {
                card.end = parse_plan_moment(d)?;
            }
            let before = match str_field(v, "before") {
                Some(b) => Some(resolve_card(&handle, b)?),
                None => None,
            };
            let cid = card.id.clone();
            let spot = match before {
                Some(b) if b.column == column.id => DropSpot::before(&column.id, &b.id),
                _ => DropSpot::end(&column.id),
            };
            handle.insert_card(card, &spot);
            let line = handle.card(&cid).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("added card to \"{}\": {line}\n{}", column.name, board_text(&id, &handle, false)))
        }
        "update_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let mut changes = Vec::new();
            if let Some(t) = str_field(v, "title") {
                handle.set_card_title(&card.id, t);
                changes.push("title".to_string());
            }
            if let Some(md) = raw_string(v, "md") {
                let md = md.trim().to_string();
                let cid = card.id.clone();
                handle.edit(|doc| {
                    if let Some(c) = doc.cards.iter_mut().find(|c| c.id == cid) {
                        c.md = md;
                    }
                });
                changes.push("md".to_string());
            }
            if let Some(p) = raw_string(v, "priority") {
                handle.set_priority(&card.id, parse_priority(&p)?);
                changes.push("priority".to_string());
            }
            if let Some(tags) = list_field(v, "tags") {
                handle.set_tags(&card.id, parse_tags(&tags.join(",")));
                changes.push("tags".to_string());
            }
            if let Some(d) = raw_string(v, "due") {
                handle.set_due(&card.id, parse_due(&d)?);
                changes.push("due".to_string());
            }
            if let Some(r) = raw_string(v, "repeat") {
                handle.set_repeat(&card.id, parse_repeat(&r)?);
                changes.push("repeat".to_string());
            }
            if let Some(d) = raw_string(v, "duration") {
                handle.set_duration(&card.id, parse_duration_arg(&d)?);
                changes.push("duration".to_string());
            }
            if raw_string(v, "start").is_some() || raw_string(v, "end").is_some() {
                let now = handle.card(&card.id).unwrap_or_else(|| card.clone());
                let start = match raw_string(v, "start") {
                    Some(d) => parse_plan_moment(&d)?,
                    None => now.start.clone(),
                };
                let end = match raw_string(v, "end") {
                    Some(d) => parse_plan_moment(&d)?,
                    None => now.end.clone(),
                };
                handle.set_schedule(&card.id, start, end);
                changes.push("plan".to_string());
            }
            if str_field(v, "column").is_some() || str_field(v, "before").is_some() {
                let col = match str_field(v, "column") {
                    Some(c) => resolve_column(&handle, c)?,
                    None => resolve_column(&handle, &card.column)?,
                };
                let spot = match str_field(v, "before") {
                    Some(b) => {
                        let b = resolve_card(&handle, b)?;
                        if b.column != col.id {
                            return Err(format!("card \"{}\" is not in column \"{}\"", b.title, col.name));
                        }
                        DropSpot::before(&col.id, &b.id)
                    }
                    None => DropSpot::end(&col.id),
                };
                handle.move_card(&card.id, &spot);
                changes.push(format!("moved to \"{}\"", col.name));
            }
            if changes.is_empty() {
                return Err(
                    "nothing to update: pass title, md, priority, tags, due, duration, start, end, repeat, column or before"
                        .to_string(),
                );
            }
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("updated {}: {line}\n{}", changes.join(", "), board_text(&id, &handle, false)))
        }
        "move_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            // Другая доска: карточка изымается и кладётся туда.
            if let Some(to) = str_field(v, "to_board") {
                let to_v = serde_json::json!({ "board": to });
                let (to_id, to_handle) = kanban_handle(ctx, &to_v)?;
                if to_id != id {
                    let column = match str_field(v, "column") {
                        Some(c) => resolve_column(&to_handle, c)?,
                        None => to_handle.lock().columns.first().cloned().ok_or("the target board has no columns")?,
                    };
                    let taken = handle.take_card(&card.id).ok_or("card vanished")?;
                    to_handle.insert_card(taken, &DropSpot::end(&column.id));
                    return Ok(format!(
                        "moved card \"{}\" to board kanban:{to_id}, column \"{}\"\n{}",
                        card.title,
                        column.name,
                        board_text(&to_id, &to_handle, false)
                    ));
                }
            }
            let column = match str_field(v, "column") {
                Some(c) => resolve_column(&handle, c)?,
                None => resolve_column(&handle, &card.column)?,
            };
            let spot = match str_field(v, "before") {
                Some(b) => {
                    let b = resolve_card(&handle, b)?;
                    if b.column != column.id {
                        return Err(format!("card \"{}\" is not in column \"{}\"", b.title, column.name));
                    }
                    DropSpot::before(&column.id, &b.id)
                }
                None => DropSpot::end(&column.id),
            };
            handle.move_card(&card.id, &spot);
            Ok(format!("moved card \"{}\" to \"{}\"\n{}", card.title, column.name, board_text(&id, &handle, false)))
        }
        "delete_card" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            handle.delete_card(&card.id);
            Ok(format!("deleted card \"{}\"\n{}", card.title, board_text(&id, &handle, false)))
        }
        "archive" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            handle.archive_card(&card.id);
            Ok(format!("archived card \"{}\" (kanban op=unarchive brings it back)\n{}", card.title, board_text(&id, &handle, false)))
        }
        "unarchive" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let key = str_field(v, "card").ok_or("missing \"card\"")?;
            let found = {
                let doc = handle.lock();
                let lower = key.trim().to_lowercase();
                let hits: Vec<&KanbanCard> =
                    doc.archive.iter().filter(|c| c.id == key.trim() || c.title.trim().to_lowercase() == lower).collect();
                match hits.len() {
                    1 => hits[0].id.clone(),
                    0 => return Err(format!("card \"{key}\" is not in the archive — kanban op=read archived=true lists it")),
                    n => return Err(format!("{n} archived cards are titled \"{key}\" — pass the id")),
                }
            };
            handle.unarchive_card(&found);
            let line = handle.card(&found).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("restored {line}\n{}", board_text(&id, &handle, false)))
        }
        "attach" => {
            let (_id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let (bytes, ext, stem, filename) = attachment_bytes(v)?;
            let size = bytes.len();
            let url = media::ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
            let name = str_field(v, "name").map(str::to_string).unwrap_or(if filename.is_empty() { stem } else { filename });
            let file = crate::pages::notes::kanban::model::CardFile::new(url.clone(), name.clone());
            let kind = if file.is_image() { "image (thumbnail on the card)" } else { "file (paperclip on the card)" };
            handle.add_file(&card.id, file);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("attached {url} ({size} bytes) as {kind} \"{name}\" to {line}\n"))
        }
        "detach" => {
            let (_, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            let key = str_field(v, "file").ok_or("missing \"file\" (attachment name or asset url)")?;
            let lower = key.trim().to_lowercase();
            let hits: Vec<&crate::pages::notes::kanban::model::CardFile> =
                card.files.iter().filter(|f| f.url == key.trim() || f.label().to_lowercase() == lower).collect();
            let url = match hits.len() {
                1 => hits[0].url.clone(),
                0 => return Err(format!("card \"{}\" has no attachment \"{key}\"", card.title)),
                n => return Err(format!("{n} attachments are named \"{key}\" — pass the asset url")),
            };
            handle.remove_file(&card.id, &url);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("removed attachment {url} from {line}\n"))
        }
        "delete" => {
            let (id, _) = kanban_handle(ctx, v)?;
            delete_object(ctx, "kanban", &id)
        }
        "schedule" | "unschedule" => {
            let (id, handle) = kanban_handle(ctx, v)?;
            let card = resolve_card(&handle, str_field(v, "card").ok_or("missing \"card\"")?)?;
            if op == "unschedule" {
                handle.unschedule_card(&card.id);
                let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
                return Ok(format!("unscheduled: {line}\n{}", board_text(&id, &handle, false)));
            }
            if let Some(d) = raw_string(v, "duration") {
                handle.set_duration(&card.id, parse_duration_arg(&d)?);
            }
            // День и время плана: `date` (или `start`) + `time`; без них —
            // сегодня. Конец досчитывается по оценке.
            let iso = match str_field(v, "date").or_else(|| str_field(v, "start")) {
                Some(d) => parse_plan_moment(d)?.ok_or("\"date\" is empty — use op=unschedule to clear the plan")?,
                None => Moment { day: today_days(), min: None }.iso(),
            };
            let mut moment = Moment::parse(&iso).ok_or("bad date")?;
            if let Some(t) = str_field(v, "time") {
                moment.min = Some(parse_hm(t).ok_or_else(|| format!("bad time \"{t}\" (hh:mm)"))?);
            }
            handle.schedule_card(&card.id, moment.day, moment.min);
            let line = handle.card(&card.id).map(|c| card_line(&c)).unwrap_or_default();
            Ok(format!("scheduled: {line}\n{}", board_text(&id, &handle, false)))
        }
        "set_boards" => Err(
            // Доски выбирает тот виджет, который их показывает; подсказка
            // здесь — чтобы модель не искала операцию у самой доски.
            "boards are picked on the widget that shows them: calendar op=set_view {calendar, boards} \
             or gantt op=set_boards {gantt, boards}"
                .to_string(),
        ),
        other => Err(format!("unknown kanban op \"{other}\". {KANBAN_OPS_HELP}")),
    }
}

/// Врезать объект на страницу: `### title` (если задан) + `![[kind:id]]`
/// с высотой по умолчанию; позиция и геометрия — из аргументов (геометрия
/// достаётся самой врезке).
pub(super) fn embed_object(ctx: NotesCtx, pid: &str, kind: &str, id: &str, v: &Json) -> Result<(InsertPos, Vec<usize>), String> {
    embed_object_with(ctx, pid, kind, id, v, str_field(v, "title"))
}

/// То же, но заголовок над врезкой задаётся явно: у карты `title` — текст
/// корневого узла, а не подпись блока.
pub(super) fn embed_object_with(
    ctx: NotesCtx,
    pid: &str,
    kind: &str,
    id: &str,
    v: &Json,
    heading: Option<&str>,
) -> Result<(InsertPos, Vec<usize>), String> {
    let geom = parse_geom(v)?;
    let h = geom.h.unwrap_or_else(|| embeds::default_object_h(kind));
    let embed = format!("![[{kind}:{id}]]{{h={}}}", fnum(h));
    let md = match heading {
        Some(t) => format!("### {t}\n\n{embed}"),
        None => embed,
    };
    let mut model = load_model(ctx, pid);
    let pos = parse_pos(&model, v, InsertPos::End)?;
    let blocks = fragment_blocks(&md, &Geom { h: None, ..geom }, true)?;
    let idx = insert_blocks(&mut model, blocks, pos);
    store_model(ctx, pid, &model)?;
    Ok((pos, idx))
}

/// Удалить объект: врезки — со страниц, файл — из бандла.
pub(super) fn delete_object(ctx: NotesCtx, kind: &str, id: &str) -> Result<String, String> {
    let target = format!("{kind}:{id}");
    let pages = ctx.pages_referencing(kind, id);
    for pid in &pages {
        if let Some(md) = remove_embed(&ctx.page_markdown(pid), &target) {
            ctx.set_page_markdown(pid, &md);
        }
    }
    ctx.delete_object(kind, id);
    Ok(format!(
        "deleted {target} (embeds removed from {} page{})\n{}",
        pages.len(),
        if pages.len() == 1 { "" } else { "s" },
        pages.first().map(|p| page_line(ctx, p) + "\n").unwrap_or_default()
    ))
}
