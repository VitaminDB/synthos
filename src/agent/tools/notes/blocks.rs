//! Действие `blocks`: блоки страницы как структура — список, вставка,
//! замена, перенос, удаление, атрибуты, закрепление на холсте.
//!
//! Операции адресуют блоки верхнего уровня. Вложенные (содержимое
//! toggle, выноски, цитаты, пункта списка) видны в `op=list` строками
//! `#3.0`, а двигают их `op=nest` (блок уходит внутрь соседа сверху или
//! указанного `into`) и `op=unnest` (обратно наружу): в markdown такое
//! вложение — строки цитаты `> `, и написать его подряд, забыв префикс,
//! легче лёгкого — тогда таблица лежит рядом с toggle, а не внутри.

use super::*;

/// Блок целиком: строка списка, markdown и атрибуты — тело `blocks op=read`.
fn block_read_text(i: usize, b: &DocBlock) -> String {
    let mut out = format!("{}\n--- Markdown ---\n{}", block_line(i, b), block_markdown(b));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !b.attrs.is_empty() {
        out.push_str(&format!("--- Attributes ---\n{}\n", serialize_attrs(&b.attrs)));
    }
    out
}

/// Ссылки сразу на несколько блоков для `op=read`: `all`, список индексов
/// и диапазонов («0,2,5-7»), массив ссылок либо одна ссылка (индекс или
/// `find:<текст>`) — как в остальных операциях.
fn resolve_block_list(model: &DocModel, v: &Json) -> Result<Vec<usize>, String> {
    let n = model.blocks.len();
    let check = |i: usize| -> Result<usize, String> {
        (i < n).then_some(i).ok_or_else(|| {
            format!("block #{i} does not exist — the page has {n} blocks (blocks op=list)")
        })
    };
    let mut out: Vec<usize> = Vec::new();
    match v.get("block").or_else(|| v.get("blocks")) {
        Some(Json::Array(a)) => {
            for e in a {
                let s = match e {
                    Json::String(s) => s.trim().to_string(),
                    Json::Number(num) => num.to_string(),
                    other => return Err(format!("\"block\" list takes indices or find:<text>, got {other}")),
                };
                out.push(resolve_block(model, &s)?);
            }
        }
        Some(Json::Number(num)) => out.push(resolve_block(model, &num.to_string())?),
        Some(Json::String(raw)) => {
            let t = raw.trim();
            let t = t.strip_prefix("block:").unwrap_or(t).trim();
            let numeric = !t.is_empty()
                && t.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '-' | '#' | ' '))
                && (t.contains(',') || t.contains('-'));
            if matches!(t.to_ascii_lowercase().as_str(), "all" | "*" | "every") {
                out.extend(0..n);
            } else if numeric {
                for part in t.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let num = |s: &str| -> Result<usize, String> {
                        s.trim()
                            .trim_start_matches('#')
                            .parse::<usize>()
                            .map_err(|_| format!("bad block index \"{part}\" (\"0,2,5-7\" or \"all\")"))
                    };
                    match part.split_once('-') {
                        Some((a, b)) => {
                            let (a, b) = (num(a)?, num(b)?);
                            if a > b {
                                return Err(format!("bad block range \"{part}\" — it starts after it ends"));
                            }
                            for i in a..=b {
                                out.push(check(i)?);
                            }
                        }
                        None => out.push(check(num(part)?)?),
                    }
                }
            } else {
                out.push(resolve_block(model, t)?);
            }
        }
        _ => {
            return Err("missing \"block\" (index from blocks op=list, find:<text>, a list like \
                        \"0,2,5-7\" or \"all\" for the whole page)"
                .to_string())
        }
    }
    let mut uniq: Vec<usize> = Vec::with_capacity(out.len());
    for i in out {
        if !uniq.contains(&i) {
            uniq.push(i);
        }
    }
    Ok(uniq)
}

/// Пояснения к ответу строками (пустая строка, если их нет).
fn notes_text(notes: &[String]) -> String {
    notes.iter().map(|n| format!("{n}\n")).collect()
}

pub(super) fn blocks_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let op = str_field(v, "op")
        .ok_or("missing \"op\" (list | read | insert | set_markdown | delete | move | nest | unnest | set_attrs | pin | unpin)")?;
    let id = page_arg(ctx, v, "page")?;
    let mut model = load_model(ctx, &id);
    let block_arg = |model: &DocModel| -> Result<usize, String> {
        resolve_block(model, &ref_field(v, "block").ok_or("missing \"block\" (index from blocks op=list or find:<text>)")?)
    };
    // Заголовок на холсте переезжает вместе со своей секцией.
    let carry_sections = bool_field(v, "section") != Some(false);
    let section = |model: &DocModel, i: usize| if carry_sections { section_of(model, i) } else { Vec::new() };
    match op {
        "list" => Ok(format!("{}{}\n", blocks_text(&model), page_line(ctx, &id))),
        "read" => {
            let idx = resolve_block_list(&model, v)?;
            if idx.is_empty() {
                return Ok(format!("(no blocks)\n{}\n", page_line(ctx, &id)));
            }
            if idx.len() == 1 {
                return Ok(block_read_text(idx[0], &model.blocks[idx[0]]));
            }
            let budget = ReadBudget::take();
            let mut body = String::new();
            let mut done = 0usize;
            let mut spent = (0usize, 0usize);
            let mut by_window = false;
            for &i in &idx {
                let text = block_read_text(i, &model.blocks[i]);
                let cost = ReadBudget::cost(&text);
                if !body.is_empty() && !budget.fits(spent, cost) {
                    by_window = budget.by_window(spent, cost);
                    break;
                }
                spent = (spent.0 + cost.0, spent.1 + cost.1);
                body.push_str(&text);
                body.push('\n');
                done += 1;
            }
            let mut out = if done == idx.len() {
                format!("--- {} blocks ---\n", idx.len())
            } else {
                format!(
                    "--- {done} of {} blocks{} · the rest did not fit in one reply ---\n",
                    idx.len(),
                    budget.note(spent.0)
                )
            };
            out.push_str(&body);
            if done < idx.len() {
                out.push_str(&format!(
                    "--- Not read: {} ---\n{}",
                    indices_text(&idx[done..]),
                    budget.advice(by_window)
                ));
            }
            out.push_str(&format!("{}\n", page_line(ctx, &id)));
            Ok(out)
        }
        "insert" => {
            let md = raw_string(v, "md").or_else(|| raw_string(v, "content")).ok_or("missing \"md\" (markdown to insert)")?;
            let pos = parse_pos(&model, v, InsertPos::End)?;
            let blocks = fragment_blocks(&md, &parse_geom(v)?, false)?;
            let idx = insert_blocks(&mut model, blocks, pos);
            store_model(ctx, &id, &model)?;
            let lines: Vec<String> = idx.iter().map(|&i| block_line_checked(&model, i)).collect();
            Ok(format!("inserted {} {}\n{}\n{}\n", indices_text(&idx), pos_text(pos), lines.join("\n"), page_line(ctx, &id)))
        }
        "set_markdown" => {
            let i = block_arg(&model)?;
            let md = raw_string(v, "md").or_else(|| raw_string(v, "content")).ok_or("missing \"md\" (new markdown of the block)")?;
            let idx = replace_block(&mut model, i, &md)?;
            store_model(ctx, &id, &model)?;
            if idx.is_empty() {
                return Ok(format!("block #{i} removed (empty markdown)\n{}\n", page_line(ctx, &id)));
            }
            let lines: Vec<String> = idx.iter().map(|&i| block_line(i, &model.blocks[i])).collect();
            Ok(format!("replaced block #{i} with {}\n{}\n{}\n", indices_text(&idx), lines.join("\n"), page_line(ctx, &id)))
        }
        "delete" => {
            let i = block_arg(&model)?;
            let line = block_line(i, &model.blocks[i]);
            model.blocks.remove(i);
            store_model(ctx, &id, &model)?;
            Ok(format!("deleted {line}\n{} blocks left\n{}\n", model.blocks.len(), page_line(ctx, &id)))
        }
        "move" => {
            let i = block_arg(&model)?;
            let mut geom = parse_geom(v)?;
            let has_pos = usize_field(v, "index").is_some() || ref_field(v, "after").is_some() || ref_field(v, "before").is_some();
            if !has_pos && geom.x.is_none() && geom.w.is_none() && geom.h.is_none() {
                return Err("pass index / after / before (order) and/or x + y (place on the canvas)".to_string());
            }
            let mut notes = Vec::new();
            if !needs_own_height(&model.blocks[i]) {
                if geom.h.take().is_some() {
                    notes.push(text_height_note(i, &model.blocks[i]));
                }
                drop_text_height(&mut model.blocks[i]);
            }
            // Секция считается по порядку документа — при смене порядка её
            // индексы уже не те, заголовок тогда едет один.
            let members = if has_pos { Vec::new() } else { section(&model, i) };
            let old = free::pos_of(&model.blocks[i].attrs);
            let mut changes = Vec::new();
            let mut at = i;
            if has_pos {
                let pos = parse_pos(&model, v, InsertPos::End)?;
                let mut target = pos_index(&model, pos);
                let block = model.blocks.remove(i);
                if target > i {
                    target -= 1;
                }
                let target = target.min(model.blocks.len());
                model.blocks.insert(target, block);
                at = target;
                changes.push(format!("order #{i} → #{target}"));
            }
            if !geom.is_empty() {
                geom.apply(&mut model.blocks[at].attrs);
                if let BlockKind::Shape { shape } = model.blocks[at].kind {
                    if shape.is_line() {
                        canonicalize_line(&mut model.blocks[at], shape);
                    }
                }
                changes.push("placed on the canvas".to_string());
                notes.extend(carry_section(&mut model, at, old, &members));
            }
            store_model(ctx, &id, &model)?;
            if changes.is_empty() {
                changes.push("nothing changed".to_string());
            }
            Ok(format!(
                "moved: {}\n{}\n{}{}\n",
                changes.join(", "),
                block_line_checked(&model, at),
                notes_text(&notes),
                page_line(ctx, &id)
            ))
        }
        "nest" => {
            let i = block_arg(&model)?;
            let into = match ref_field(v, "into").or_else(|| ref_field(v, "parent")) {
                Some(r) => resolve_block(&model, &r)?,
                // Умолчание — сосед сверху: так пишут toggle и таблицу
                // подряд, забыв «> » перед строками таблицы.
                None => i.checked_sub(1).ok_or(
                    "block #0 has no block above it — pass \"into\" (the toggle, callout, quote or list item that takes it in)",
                )?,
            };
            if into == i {
                return Err("a block cannot be nested into itself".to_string());
            }
            if model.blocks[into].kind.children().is_none() {
                return Err(format!(
                    "{} cannot hold other blocks — only a toggle, callout, quote or list item can",
                    block_line(into, &model.blocks[into])
                ));
            }
            let mut block = model.blocks.remove(i);
            // Координаты холста живут только у блоков верхнего уровня.
            free::clear(&mut block.attrs);
            let at = if into > i { into - 1 } else { into };
            // Иначе вложенный блок исчезает внутри свёрнутого toggle.
            if let BlockKind::Toggle { collapsed, .. } = &mut model.blocks[at].kind {
                *collapsed = false;
            }
            let child = match model.blocks[at].kind.children_mut() {
                Some(children) => {
                    children.push(block);
                    children.len() - 1
                }
                None => return Err("the target block cannot hold other blocks".to_string()),
            };
            store_model(ctx, &id, &model)?;
            Ok(format!(
                "nested #{i} into #{at} as #{at}.{child}\n{}\n{}\n",
                blocks_text(&model).trim_end(),
                page_line(ctx, &id)
            ))
        }
        "unnest" => {
            // Ссылка ребёнка — либо «3.0» из op=list, либо block + child.
            let raw = ref_field(v, "block").ok_or("missing \"block\" (the container, e.g. the toggle: index from blocks op=list)")?;
            // Путь «3.0» разбирается только когда обе половины — числа:
            // у find:<текст> точка бывает частью искомого фрагмента.
            let path = raw.trim().trim_start_matches('#').split_once('.').and_then(|(p, c)| {
                match (p.trim().parse::<usize>(), c.trim().parse::<usize>()) {
                    (Ok(_), Ok(ci)) => Some((p.trim().to_string(), ci)),
                    _ => None,
                }
            });
            let (parent_ref, mut child) = match path {
                Some((p, c)) => (p, Some(c)),
                None => (raw.clone(), None),
            };
            let i = resolve_block(&model, &parent_ref)?;
            if child.is_none() {
                child = match v.get("child") {
                    Some(Json::String(s)) if matches!(s.trim().to_ascii_lowercase().as_str(), "all" | "*") => None,
                    Some(_) => Some(usize_field(v, "child").ok_or("\"child\" takes the index of the nested block (or \"all\")")?),
                    None => None,
                };
            }
            let count = model.blocks[i].kind.children().map(|c| c.len()).unwrap_or(0);
            let Some(children) = model.blocks[i].kind.children_mut() else {
                return Err(format!(
                    "{} holds no blocks — nothing to take out",
                    block_line(i, &model.blocks[i])
                ));
            };
            if count == 0 {
                return Err(format!("block #{i} is empty — nothing to take out"));
            }
            let taken: Vec<DocBlock> = match child {
                Some(k) if k >= count => {
                    return Err(format!("block #{i} holds {count} blocks — #{i}.{k} does not exist"))
                }
                Some(k) => vec![children.remove(k)],
                None => std::mem::take(children),
            };
            let n = taken.len();
            for (k, b) in taken.into_iter().enumerate() {
                model.blocks.insert(i + 1 + k, b);
            }
            store_model(ctx, &id, &model)?;
            Ok(format!(
                "took {n} block(s) out of #{i} — they follow it on the page\n{}\n{}\n",
                blocks_text(&model).trim_end(),
                page_line(ctx, &id)
            ))
        }
        "set_attrs" => {
            let i = block_arg(&model)?;
            let mut pairs = attrs_arg(v)?;
            let mut notes = Vec::new();
            if !needs_own_height(&model.blocks[i]) {
                let before = pairs.len();
                pairs.retain(|(k, _)| !k.trim().eq_ignore_ascii_case("h"));
                if pairs.len() != before {
                    notes.push(text_height_note(i, &model.blocks[i]));
                }
                drop_text_height(&mut model.blocks[i]);
            }
            let members = section(&model, i);
            let old = free::pos_of(&model.blocks[i].attrs);
            let changes = if pairs.is_empty() { Vec::new() } else { apply_attrs(&mut model.blocks[i], &pairs)? };
            notes.extend(carry_section(&mut model, i, old, &members));
            store_model(ctx, &id, &model)?;
            let head = if changes.is_empty() { "nothing set".to_string() } else { format!("set {}", changes.join(", ")) };
            Ok(format!("{head}\n{}\n{}{}\n", block_line_checked(&model, i), notes_text(&notes), page_line(ctx, &id)))
        }
        "pin" => {
            let i = block_arg(&model)?;
            let mut geom = parse_geom(v)?;
            let mut notes = Vec::new();
            if !needs_own_height(&model.blocks[i]) {
                if geom.h.take().is_some() {
                    notes.push(text_height_note(i, &model.blocks[i]));
                }
                drop_text_height(&mut model.blocks[i]);
            }
            let members = section(&model, i);
            let old = free::pos_of(&model.blocks[i].attrs);
            let (x, y) = match (geom.x, geom.y) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    // Под самым нижним закреплённым блоком, у левого края холста.
                    let pinned: Vec<(f32, f32, f32, f32)> = model.blocks.iter().filter_map(block_rect).collect();
                    match pinned.iter().map(|r| r.1 + r.3).fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v)))) {
                        Some(bottom) => (pinned.iter().map(|r| r.0).fold(f32::MAX, f32::min).max(0.0), bottom + 20.0),
                        None => (40.0, 40.0),
                    }
                }
            };
            free::set_pos(&mut model.blocks[i].attrs, x, y);
            if let Some(w) = geom.w {
                free::set_width(&mut model.blocks[i].attrs, w);
            }
            if let Some(h) = geom.h {
                free::set_height(&mut model.blocks[i].attrs, h);
            }
            if let BlockKind::Shape { shape } = model.blocks[i].kind {
                if shape.is_line() {
                    canonicalize_line(&mut model.blocks[i], shape);
                }
            }
            notes.extend(carry_section(&mut model, i, old, &members));
            store_model(ctx, &id, &model)?;
            Ok(format!(
                "pinned at x={} y={}\n{}\n{}{}\n",
                fnum(x),
                fnum(y),
                block_line_checked(&model, i),
                notes_text(&notes),
                page_line(ctx, &id)
            ))
        }
        "arrange" => {
            let geom = parse_geom(v)?;
            let (all, objects) = match str_field(v, "only").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                None | Some("flow") | Some("unpinned") => (false, false),
                Some("all") => (true, false),
                Some("everything") => (true, true),
                Some(other) => return Err(format!("unknown \"only\" \"{other}\" (flow | all | everything)")),
            };
            let gap = f32_field(v, "gap").unwrap_or(ARRANGE_GAP);
            if !(0.0..=400.0).contains(&gap) {
                return Err("\"gap\" must be within 0..400 px".to_string());
            }
            let a = Arrange { x: geom.x, y: geom.y, w: geom.w, gap, all, objects };
            let placed = arrange_column(&mut model, &a);
            if placed.is_empty() {
                return Ok(format!(
                    "nothing to arrange: every block already has coordinates (pass only=all to re-stack them)\n{}\n",
                    page_line(ctx, &id)
                ));
            }
            store_model(ctx, &id, &model)?;
            let first = block_rect(&model.blocks[placed[0]]).map(|r| r.0).unwrap_or(0.0);
            let bottom = placed
                .iter()
                .filter_map(|&i| block_rect(&model.blocks[i]))
                .map(|r| r.1 + r.3)
                .fold(0.0f32, f32::max);
            let lines: Vec<String> = placed.iter().map(|&i| block_line_checked(&model, i)).collect();
            // Поставленные объекты `only=all` не трогает — и говорит об этом
            // прямо: 14.09.2026 модель не заметила, что календарь уехал в
            // хвост колонки, и перебирала y по кругу.
            let kept: Vec<String> = if all && !objects {
                (0..model.blocks.len())
                    .filter(|i| !placed.contains(i) && needs_own_height(&model.blocks[*i]))
                    .map(|i| block_line_checked(&model, i))
                    .collect()
            } else {
                Vec::new()
            };
            let kept_text = if kept.is_empty() {
                String::new()
            } else {
                format!(
                    "kept in place — only=all does not re-stack placed objects (only=everything does):\n{}\n",
                    kept.join("\n")
                )
            };
            Ok(format!(
                "arranged {} blocks in a column at x={} (gap {}); the column ends at y={}\n{}\n{}{}\n",
                placed.len(),
                fnum(first),
                fnum(gap),
                fnum(bottom),
                lines.join("\n"),
                kept_text,
                page_line(ctx, &id)
            ))
        }
        "unpin" => {
            let i = block_arg(&model)?;
            free::clear(&mut model.blocks[i].attrs);
            store_model(ctx, &id, &model)?;
            Ok(format!("unpinned — the block flows in the column again\n{}\n{}\n", block_line(i, &model.blocks[i]), page_line(ctx, &id)))
        }
        other => Err(format!(
            "unknown blocks op \"{other}\" (list | read | insert | set_markdown | delete | move | nest | unnest | set_attrs | pin | unpin | arrange)"
        )),
    }
}
