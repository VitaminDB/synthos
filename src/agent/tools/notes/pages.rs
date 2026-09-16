//! Страницы: `list`, `search`, `read`, а также `create` / `update` /
//! `move` / `delete` / `duplicate` / `open` / `attach`.

use super::*;

pub(super) fn list_impl(ctx: NotesCtx) -> Result<String, String> {
    let tree = ctx.tree.get_untracked();
    let mut out = String::new();
    out.push_str("--- Project ---\n");
    out.push_str(&format!("file: {}\n", ctx.project_path.get_untracked().display()));
    let active = ctx
        .active
        .get_untracked()
        .filter(|id| tree.find(id).is_some())
        .map(|id| format!("{id} \"{}\"", ctx.title_of(&id)))
        .unwrap_or_else(|| "none".to_string());
    out.push_str(&format!("pages: {} · active: {active}\n", tree.all().len()));
    out.push_str("--- Pages (indent = nesting; size in words; objects embedded in the page after ·) ---\n");
    if tree.is_empty() {
        out.push_str("(no pages yet — notes create makes one)\n");
    } else {
        fn walk(ctx: NotesCtx, nodes: &[crate::pages::notes::project::PageNode], depth: usize, out: &mut String) {
            for n in nodes {
                let icon = n.icon.as_deref().filter(|i| !i.is_empty()).map(|i| format!(" {i}")).unwrap_or_default();
                let md = ctx.page_markdown(&n.id);
                let objects: Vec<String> =
                    object_refs(&md).iter().map(|(k, id)| format!(" · {k}:{id}")).collect();
                out.push_str(&format!(
                    "{}{} \"{}\"{icon} · {} words{}\n",
                    "  ".repeat(depth),
                    n.id,
                    n.title,
                    count_words(&plain_markdown(&md)),
                    objects.concat()
                ));
                walk(ctx, &n.children, depth + 1, out);
            }
        }
        walk(ctx, &tree.roots, 0, &mut out);
        // Дерево на тысячу страниц само по себе больше окна. Клипа поверх
        // ответа `notes` нет (инструмент отвечает за свой размер сам),
        // поэтому список режется здесь — по строке, а не посреди неё.
        let budget = ReadBudget::take();
        if !budget.fits((0, 0), ReadBudget::cost(&out)) {
            out = budget.clip_lines(out, "the tree is longer than the context left");
        }
    }
    out.push_str(
        "---\nread {page} shows the markdown, boards and charts on the page and its links; \
         read {pages: [\"id\", \"id\"]} or {page, depth=all} or {page=\"all\"} brings back \
         several pages, a whole subtree or the whole project in ONE call — use it instead of \
         reading page by page; update {page, content | find+replace | mode=append} edits it; \
         kanban / gantt / chart {op=create, page} add a board, a Gantt chart or a \
         line/bar/pie/radar/gauge chart.\n",
    );
    Ok(out)
}

pub(super) fn search_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let query = str_field(v, "query").ok_or("missing \"query\"")?;
    let q = query.to_lowercase();
    let limit = usize_field(v, "limit").unwrap_or(20).clamp(1, 100);
    let tree = ctx.tree.get_untracked();
    let mut out = String::new();
    let mut hits = 0usize;
    for node in tree.all() {
        let md = plain_markdown(&ctx.page_markdown(&node.id));
        let title_hit = node.title.to_lowercase().contains(&q);
        let lines: Vec<(usize, &str)> = md
            .lines()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .take(4)
            .collect();
        if !title_hit && lines.is_empty() {
            continue;
        }
        hits += 1;
        if hits > limit {
            break;
        }
        out.push_str(&format!("{} \"{}\" · path: {}\n", node.id, node.title, path_of(ctx, &node.id)));
        for (n, l) in lines {
            let mut t = l.trim().to_string();
            if t.chars().count() > 160 {
                t = t.chars().take(160).collect::<String>() + "…";
            }
            out.push_str(&format!("  L{}: {t}\n", n + 1));
        }
    }
    if hits == 0 {
        out.push_str(&format!("no pages match \"{query}\"\n"));
    } else if hits > limit {
        out.push_str(&format!("… more than {limit} pages match — narrow the query\n"));
    }
    Ok(out)
}

/// Потолок ответа, когда бюджет не измерен: вызов идёт вне agent-loop
/// (юнит-тесты, чужой код) и остатка окна взять неоткуда. В самом ходе
/// потолка нет — границу ставит живое окно модели.
pub(super) const READ_FALLBACK_TOKENS: usize = 8_000;

/// Вторая мера — байты, предохранитель исполнителя
/// ([`MAX_NOTES_OUTPUT_BYTES`]): он режет вывод молча и посреди строки, а
/// пачка обязана обрываться на границе страницы. Вычет — на шапку ответа и
/// список недочитанного.
pub(super) const READ_MAX_BYTES: usize = MAX_NOTES_OUTPUT_BYTES - 8 * 1024;

/// Сколько инструмент может отдать этим ответом.
pub(super) struct ReadBudget {
    /// Грант живого окна в токенах — половина того, что осталось.
    tokens: usize,
    pub(super) bytes: usize,
    /// Сколько токенов окна осталось до конца контекста (0 — не измерено).
    pub(super) left: usize,
    measured: bool,
}

impl ReadBudget {
    pub(super) fn take() -> Self {
        // Потолка сверху нет: сколько окна модель даёт, столько и читаем —
        // проект на 14k токенов при свободном контексте уходит одним
        // ответом, а не тремя ходами с полным префиллом каждый.
        let g = budget::grant(usize::MAX);
        Self {
            tokens: if g.measured { g.tokens } else { READ_FALLBACK_TOKENS },
            bytes: READ_MAX_BYTES,
            left: g.left,
            measured: g.measured,
        }
    }

    /// Влезает ли ещё один кусок: считаем обеими мерами сразу.
    pub(super) fn fits(&self, spent: (usize, usize), piece: (usize, usize)) -> bool {
        spent.0 + piece.0 <= self.tokens && spent.1 + piece.1 <= self.bytes
    }

    /// Обрыв случился из-за тесноты в окне, а не из-за предохранителя.
    pub(super) fn by_window(&self, spent: (usize, usize), piece: (usize, usize)) -> bool {
        self.measured && spent.0 + piece.0 > self.tokens
    }

    /// Чего стоит кусок: токены (точно либо оценкой) и байты.
    pub(super) fn cost(text: &str) -> (usize, usize) {
        (budget::count(text), text.len())
    }

    /// Хвост шапки: сколько занял ответ и сколько окна осталось. Модель
    /// должна видеть причину обрыва — «контекст кончается», а не «инструмент
    /// такой».
    pub(super) fn note(&self, spent: usize) -> String {
        if !self.measured {
            return String::new();
        }
        format!(" · ~{spent} tokens of the ~{} left in context", self.left)
    }

    /// Текст длиннее бюджета: отрезаем по строкам (в markdown это границы
    /// блоков) и дописываем пометку — `tail(keep, total, spent)`. Доля
    /// берётся от цены целого и ужимается, пока ответ вместе с пометкой не
    /// влезет: считать токены по каждой строке слишком дорого.
    fn clip_by_lines(&self, text: String, tail: impl Fn(usize, usize, usize) -> String) -> String {
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let total = lines.len();
        let cost = Self::cost(&text);
        let mut share = (self.tokens as f32 / cost.0.max(1) as f32)
            .min(self.bytes as f32 / cost.1.max(1) as f32)
            .min(1.0);
        let mut last = String::new();
        for _ in 0..4 {
            share *= 0.8;
            let keep = ((total as f32 * share) as usize).clamp(1, total);
            let head: String = lines[..keep].concat();
            last = format!("{head}{}", tail(keep, total, Self::cost(&head).0));
            if self.fits((0, 0), Self::cost(&last)) {
                return last;
            }
        }
        last
    }

    /// Список страниц длиннее бюджета: дерево на тысячу страниц само по
    /// себе больше окна.
    fn clip_lines(&self, text: String, why: &str) -> String {
        self.clip_by_lines(text, |keep, total, spent| {
            format!("--- Cut after {keep} of {total} lines: {why}{} ---\n", self.note(spent))
        })
    }

    /// Одна страница больше всего бюджета: обрываем по строке и говорим,
    /// чем дочитать — `blocks op=read` умеет отдать остаток кусками.
    fn clip_page(&self, text: String, id: &str) -> String {
        self.clip_by_lines(text, |keep, total, spent| {
            format!(
                "--- Page truncated: {keep} of {total} lines{} ---\n\
                 Read the rest in pieces: blocks {{\"op\": \"read\", \"page\": \"{id}\", \
                 \"block\": \"...\"}}.\n{}",
                self.note(spent),
                self.advice(self.measured)
            )
        })
    }

    /// Подсказка, когда режет именно теснота окна.
    pub(super) fn advice(&self, by_window: bool) -> &'static str {
        if by_window {
            "\nContext is filling up: read only what you need next, or ask the user to \
             compact the chat before reading the rest.\n"
        } else {
            ""
        }
    }
}

/// «Весь проект» в `page`/`pages` — но только если так не называется
/// настоящая страница.
fn is_all_pages(ctx: NotesCtx, s: &str) -> bool {
    let t = s.trim();
    matches!(t.to_ascii_lowercase().as_str(), "all" | "*" | "project" | "everything")
        && ctx.tree.get_untracked().find(t).is_none()
        && ctx.find_by_title(t).is_empty()
}

/// Одна ссылка на страницу либо несколько через запятую/перенос строки:
/// целая строка важнее — в названии страницы запятая законна.
fn resolve_many(ctx: NotesCtx, raw: &str) -> Result<Vec<String>, String> {
    let whole = resolve_page_ref(ctx, raw);
    if let Ok(id) = whole {
        return Ok(vec![id]);
    }
    let parts: Vec<&str> = raw.split([',', '\n']).map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return whole.map(|id| vec![id]);
    }
    parts.into_iter().map(|p| resolve_page_ref(ctx, p)).collect()
}

/// `depth` — сколько уровней подстраниц добрать к каждой запрошенной
/// странице: число, `true` либо «all» (всё поддерево).
fn depth_arg(v: &Json) -> Result<usize, String> {
    let level = |i: i64| if i < 0 { usize::MAX } else { i as usize };
    match v.get("depth") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(b)) => Ok(if *b { usize::MAX } else { 0 }),
        Some(Json::Number(n)) => n.as_i64().map(level).ok_or_else(|| "\"depth\" must be a whole number".to_string()),
        Some(Json::String(s)) => {
            let t = s.trim().to_ascii_lowercase();
            if t.is_empty() {
                return Ok(0);
            }
            if matches!(t.as_str(), "all" | "*" | "full" | "tree" | "subtree" | "max" | "true") {
                return Ok(usize::MAX);
            }
            t.parse::<i64>()
                .map(level)
                .map_err(|_| format!("unknown \"depth\" \"{s}\" (a number of levels or \"all\")"))
        }
        Some(other) => Err(format!("\"depth\" must be a number or \"all\", got {other}")),
    }
}

/// Страница и её потомки до глубины `depth` (DFS, как в дереве).
fn subtree_within(tree: &crate::pages::notes::project::ProjectTree, id: &str, depth: usize, out: &mut Vec<String>) {
    fn walk(n: &crate::pages::notes::project::PageNode, left: usize, out: &mut Vec<String>) {
        out.push(n.id.clone());
        if left == 0 {
            return;
        }
        for c in &n.children {
            walk(c, left - 1, out);
        }
    }
    if let Some(n) = tree.find(id) {
        walk(n, depth, out);
    }
}

/// Что читать: `page` и/или `pages` (массив либо строка через запятую),
/// плюс подстраницы по `depth`; `page="all"` — весь проект. Порядок — как
/// в дереве, повторы убираются, `limit` режет хвост.
fn read_targets(ctx: NotesCtx, v: &Json) -> Result<Vec<String>, String> {
    let mut raws: Vec<String> = Vec::new();
    match v.get("pages") {
        None | Some(Json::Null) => {}
        Some(Json::Array(a)) => {
            for e in a {
                match e {
                    Json::String(s) if !s.trim().is_empty() => raws.push(s.trim().to_string()),
                    Json::Number(n) => raws.push(n.to_string()),
                    other => return Err(format!("\"pages\" takes page ids or titles, got {other}")),
                }
            }
        }
        Some(Json::String(s)) if !s.trim().is_empty() => raws.push(s.trim().to_string()),
        Some(Json::String(_)) => {}
        Some(other) => {
            return Err(format!("\"pages\" must be an array of pages or a comma-separated string, got {other}"))
        }
    }
    if let Some(s) = str_field(v, "page") {
        raws.push(s.to_string());
    }
    if raws.is_empty() {
        return Err("missing \"page\" (page id or title; \"pages\" takes several at once, \
                    page=\"all\" the whole project)"
            .to_string());
    }
    let tree = ctx.tree.get_untracked();
    let mut roots: Vec<String> = Vec::new();
    let mut all = false;
    for raw in &raws {
        if is_all_pages(ctx, raw) {
            all = true;
            continue;
        }
        for id in resolve_many(ctx, raw)? {
            if !roots.contains(&id) {
                roots.push(id);
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    if all {
        out = tree.all_ids();
    } else {
        let depth = depth_arg(v)?;
        for id in &roots {
            let mut sub = Vec::new();
            subtree_within(&tree, id, depth, &mut sub);
            for s in sub {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    if out.is_empty() {
        return Err("no pages to read — the project has none yet (notes create makes one)".to_string());
    }
    if out.len() > 1 {
        if let Some(limit) = usize_field(v, "limit") {
            out.truncate(limit.max(1));
        }
    }
    Ok(out)
}

/// Одна страница целиком: markdown, её блоки (по запросу), доски,
/// диаграммы, карты и календари на ней, ссылки.
fn page_text(ctx: NotesCtx, id: &str, with_blocks: bool) -> String {
    let tree = ctx.tree.get_untracked();
    let Some(node) = tree.find(id) else {
        return format!("page: {id} · (gone from the tree — see notes list)\n");
    };
    let raw = ctx.page_markdown(id);
    let md = plain_markdown(&raw);
    let mut out = String::new();
    out.push_str(&page_line(ctx, id));
    out.push('\n');
    out.push_str(&format!(
        "icon: {} · {} · {} words · children: {}\n",
        node.icon.as_deref().unwrap_or("none"),
        layout_text(&node.layout),
        count_words(&md),
        node.children.len()
    ));
    out.push_str("--- Markdown ---\n");
    if md.trim().is_empty() {
        out.push_str("(empty page)\n");
    } else {
        out.push_str(&md);
        if !md.ends_with('\n') {
            out.push('\n');
        }
    }
    if with_blocks {
        out.push_str("--- Blocks (index · kind · canvas x y w h, ~ = estimated · attributes) ---\n");
        out.push_str(&blocks_text(&parse_document(&raw)));
    }
    let objects = object_refs(&md);
    if !objects.is_empty() {
        out.push_str("--- Objects on the page ---\n");
        for (kind, oid) in &objects {
            match ctx.object(kind, oid) {
                Some(LiveObject::Kanban { handle, .. }) => out.push_str(&board_text(oid, &handle, false)),
                Some(LiveObject::Gantt { handle, .. }) => out.push_str(&gantt_text(oid, &handle)),
                Some(LiveObject::Mindmap { handle, .. }) => out.push_str(&map_text(oid, &handle)),
                Some(LiveObject::Calendar { handle, .. }) => out.push_str(&calendar_widget_text(oid, &handle)),
                Some(LiveObject::Chart { handle, .. }) => out.push_str(&chart_text(oid, &handle)),
                None => out.push_str(&format!("{kind}:{oid} · (file missing)\n")),
            }
        }
    }
    let index = ctx.index.get_untracked();
    let outgoing = index.outgoing_of(id);
    let backlinks = index.backlinks_of(id);
    if !outgoing.is_empty() || !backlinks.is_empty() {
        out.push_str("--- Links ---\n");
        let fmt = |ids: &[String]| {
            ids.iter().map(|i| format!("\"{}\" ({i})", ctx.title_of(i))).collect::<Vec<_>>().join(", ")
        };
        if !outgoing.is_empty() {
            out.push_str(&format!("outgoing: {}\n", fmt(&outgoing)));
        }
        if !backlinks.is_empty() {
            out.push_str(&format!("backlinks: {}\n", fmt(&backlinks)));
        }
    }
    out
}

/// `read` — одна страница или сразу пачка (`pages`, `depth`, `page="all"`).
/// Чтение по одной странице за вызов у локальной модели стоит целого хода
/// с полным префиллом, поэтому дерево целиком отдаётся одним ответом,
/// сколько влезает в [`ReadBudget`].
pub(super) fn read_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let ids = read_targets(ctx, v)?;
    let with_blocks = bool_field(v, "blocks").unwrap_or(false);
    let budget = ReadBudget::take();
    if ids.len() == 1 {
        let mut out = page_text(ctx, &ids[0], with_blocks);
        // Подстраницы называем сразу: иначе модель обходит дерево по
        // странице за ход, а это полный префилл на каждую.
        let kids = ctx.tree.get_untracked().find(&ids[0]).map(|n| n.children.clone()).unwrap_or_default();
        if !kids.is_empty() {
            out.push_str(&format!(
                "--- Sub-pages ({}) — read the whole subtree in ONE call with depth=all ---\n",
                kids.len()
            ));
            for k in &kids {
                out.push_str(&format!(
                    "{} \"{}\" · {} words · children: {}\n",
                    k.id,
                    k.title,
                    count_words(&plain_markdown(&ctx.page_markdown(&k.id))),
                    k.children.len()
                ));
            }
        }
        if !budget.fits((0, 0), ReadBudget::cost(&out)) {
            out = budget.clip_page(out, &ids[0]);
        }
        return Ok(out);
    }
    let total = ids.len();
    let mut body = String::new();
    let mut done = 0usize;
    let mut words = 0usize;
    let mut spent = (0usize, 0usize);
    let mut by_window = false;
    for (n, id) in ids.iter().enumerate() {
        let text = page_text(ctx, id, with_blocks);
        let cost = ReadBudget::cost(&text);
        if !body.is_empty() && !budget.fits(spent, cost) {
            by_window = budget.by_window(spent, cost);
            break;
        }
        spent = (spent.0 + cost.0, spent.1 + cost.1);
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&format!("=== page {}/{} ===\n", n + 1, total));
        body.push_str(&text);
        if !text.ends_with('\n') {
            body.push('\n');
        }
        words += count_words(&plain_markdown(&ctx.page_markdown(id)));
        done += 1;
    }
    let mut out = String::new();
    if done == total {
        out.push_str(&format!(
            "--- {total} pages · {words} words{} · all of them in this reply ---\n",
            budget.note(spent.0)
        ));
    } else {
        out.push_str(&format!(
            "--- {done} of {total} pages · {words} words{} · the rest did not fit in one reply ---\n",
            budget.note(spent.0)
        ));
    }
    out.push_str(&body);
    if done < total {
        let rest: Vec<String> =
            ids[done..].iter().map(|id| format!("{id} \"{}\"", ctx.title_of(id))).collect();
        out.push_str(&format!(
            "--- Not read ({}) ---\n{}\nRead them with one more call: pages=[\"{}\", …].\n{}",
            total - done,
            rest.join("\n"),
            ids[done],
            budget.advice(by_window)
        ));
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
// Страницы: create / update / move / delete / duplicate / open / attach
// ─────────────────────────────────────────────────────────────────────────────

fn parse_parent(ctx: NotesCtx, v: &Json) -> Result<Option<String>, String> {
    match str_field(v, "parent") {
        None => Ok(None),
        Some(p) if matches!(p.to_ascii_lowercase().as_str(), "root" | "/" | "none" | "null") => Ok(None),
        Some(p) => resolve_page_ref(ctx, p).map(Some),
    }
}

/// id страницы-соседа с точно таким же названием — то самое совпадение, из-за
/// которого `insert_page` переименует новую страницу в «Название 2».
fn sibling_with_title(ctx: NotesCtx, parent: Option<&str>, title: &str) -> Option<String> {
    let tree = ctx.tree.get_untracked();
    let siblings = match parent {
        None => tree.roots.clone(),
        Some(pid) => tree.find(pid).map(|n| n.children.clone()).unwrap_or_default(),
    };
    siblings.into_iter().find(|n| n.title == title).map(|n| n.id)
}

pub(super) fn create_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let title = str_field(v, "title").map(str::to_string).unwrap_or_else(|| tr!("notes.untitled"));
    let parent = parse_parent(ctx, v)?;
    let index = usize_field(v, "index");
    // Дубликат названия среди соседей ловим ДО вставки: `insert_page` тихо
    // переименует новую страницу («Туду — канбан 2»), и «created» в ответе
    // выглядит как полный успех. Агент, который не увидел свою же прошлую
    // страницу, на этом зацикливается — 04.09.2026 так родился 21 дубликат
    // подряд. Страницу всё равно создаём (одноимённые страницы законны:
    // «Журнал» в каждой сфере), но о совпадении говорим прямо.
    let clash = sibling_with_title(ctx, parent.as_deref(), &title);
    let id = ctx.insert_page(parent.as_deref(), index, &title, false);
    let mut out = String::from("created\n");
    if let Some(existing) = clash {
        out.push_str(&format!(
            "note: a sibling page \"{title}\" already exists (page: {existing}), so this new one \
             was named \"{}\". If you meant that existing page, do not create it again — use \
             update/open on {existing}.\n",
            ctx.title_of(&id)
        ));
    }
    if let Some(icon) = str_field(v, "icon") {
        ctx.set_icon(&id, Some(icon.to_string()));
    }
    apply_layout_args(ctx, &id, v)?;
    if let Some(content) = raw_string(v, "content") {
        write_page(ctx, &id, &content)?;
        out.push_str(&format!("content: {} words\n", count_words(&content)));
        // `create layout=free` + markdown: агент явно просил разложить
        // контент по холсту — кладём его колонкой, иначе первая же фигура
        // ляжет поверх текста. Без явного `layout` страницу не трогаем:
        // холст без закреплённых блоков рисуется как обычный документ.
        let asked_free = str_field(v, "layout")
            .is_some_and(|l| matches!(l.trim().to_ascii_lowercase().as_str(), "free" | "canvas"));
        if asked_free {
            let pinned = pin_flow_blocks(ctx, &id)?;
            if pinned > 0 {
                out.push_str(&format!("layout: {pinned} blocks pinned in a column\n"));
            }
        }
    }
    if bool_field(v, "open").unwrap_or(false) {
        show_page(ctx, Some(&id));
    }
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
}

/// Раскладка страницы одной строкой (для `read`).
fn layout_text(l: &PageLayout) -> String {
    let grid = match l.grid {
        PageGrid::None => "none".to_string(),
        g => format!("{} step {}", grid_name(g), fnum(l.grid_step)),
    };
    format!(
        "layout: canvas · grid: {grid} · snap: {} · bg: {} · props panel: {}",
        if l.snap { format!("on step {}", fnum(l.snap_step)) } else { "off".to_string() },
        if l.bg.is_empty() { "theme" } else { l.bg.as_str() },
        props_panel_name(l.props_hidden)
    )
}

fn props_panel_name(hidden: bool) -> &'static str {
    if hidden { "hidden" } else { "shown" }
}

/// `props_panel`: правая панель свойств страницы — `shown`/`true` или
/// `hidden`/`false`; возвращает «скрыта ли».
fn props_panel_arg(v: &Json) -> Result<Option<bool>, String> {
    match v.get("props_panel") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(shown)) => Ok(Some(!shown)),
        Some(Json::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "shown" | "show" | "visible" | "open" | "on" | "true" | "yes" => Ok(Some(false)),
            "hidden" | "hide" | "collapsed" | "closed" | "close" | "off" | "false" | "no" => Ok(Some(true)),
            other => Err(format!("unknown props_panel \"{other}\" (shown | hidden)")),
        },
        Some(other) => Err(format!("\"props_panel\" must be \"shown\" or \"hidden\", got {other}")),
    }
}

fn grid_name(g: PageGrid) -> &'static str {
    match g {
        PageGrid::None => "none",
        PageGrid::Dots => "dots",
        PageGrid::Lines => "lines",
        PageGrid::Cross => "cross",
    }
}

/// Раскладка страницы из аргументов `layout grid grid_step snap snap_step
/// bg props_panel`; возвращает список изменений.
fn apply_layout_args(ctx: NotesCtx, id: &str, v: &Json) -> Result<Vec<String>, String> {
    let mut l: PageLayout = ctx.page_layout(id);
    let mut arrange_after = false;
    let mut changes = Vec::new();
    let props_hidden = props_panel_arg(v)?;
    // Раскладка у страницы одна — холст. `layout=free` оставлен как
    // «разложи, что не закреплено, колонкой»; режима «поток» больше нет,
    // и просьбу переключиться на него честнее отклонить, чем принять
    // молча и оставить страницу как была.
    if let Some(layout) = str_field(v, "layout") {
        match layout.to_ascii_lowercase().as_str() {
            "free" | "canvas" => {
                arrange_after = true;
                changes.push("layout: canvas".to_string());
            }
            "flow" | "document" | "column" => {
                return Err("pages are canvas-only: blocks live at x/y, there is no flow mode. \
                            Use blocks op=arrange to lay them out in a column"
                    .to_string())
            }
            other => return Err(format!("unknown layout \"{other}\" (canvas)")),
        }
    }
    if let Some(grid) = str_field(v, "grid") {
        l.grid = match grid.to_ascii_lowercase().as_str() {
            "none" | "off" => PageGrid::None,
            "dots" | "dot" => PageGrid::Dots,
            "lines" | "line" => PageGrid::Lines,
            "cross" | "crosses" => PageGrid::Cross,
            other => return Err(format!("unknown grid \"{other}\" (none | dots | lines | cross)")),
        };
        changes.push(format!("grid: {}", grid_name(l.grid)));
    }
    if let Some(step) = f32_field(v, "grid_step") {
        if !(2.0..=200.0).contains(&step) {
            return Err("\"grid_step\" must be within 2..200 px".to_string());
        }
        l.grid_step = step.round();
        changes.push(format!("grid_step: {}", fnum(l.grid_step)));
    }
    if let Some(snap) = bool_field(v, "snap") {
        l.snap = snap;
        changes.push(format!("snap: {}", if snap { "on" } else { "off" }));
    }
    if let Some(step) = f32_field(v, "snap_step") {
        if !(1.0..=100.0).contains(&step) {
            return Err("\"snap_step\" must be within 1..100 px".to_string());
        }
        l.snap_step = step.round();
        changes.push(format!("snap_step: {}", fnum(l.snap_step)));
    }
    if let Some(bg) = raw_string(v, "bg") {
        l.bg = if bg.trim().is_empty() { String::new() } else { parse_hex_color(&bg, true)? };
        changes.push(if l.bg.is_empty() { "bg: theme".to_string() } else { format!("bg: {}", l.bg) });
    }
    if !changes.is_empty() {
        ctx.set_page_layout(id, l);
    }
    // После `set_page_layout`: тот пишет узел целиком, из снимка `l`.
    if let Some(hidden) = props_hidden {
        ctx.set_props_hidden(id, hidden);
        changes.push(format!("props panel: {}", props_panel_name(hidden)));
    }
    if arrange_after {
        // Блоки без координат встают колонкой в порядке документа.
        // Редактор на экране сделал бы то же по настоящим
        // прямоугольникам; агенту они недоступны, поэтому колонка — по
        // оценке высот.
        let pinned = pin_flow_blocks(ctx, id)?;
        if pinned > 0 {
            changes.push(format!("{pinned} blocks pinned in a column"));
        }
    }
    Ok(changes)
}

/// Разложить неприкреплённые блоки страницы колонкой (см.
/// [`arrange_column`]); возвращает, сколько блоков получили координаты.
fn pin_flow_blocks(ctx: NotesCtx, id: &str) -> Result<usize, String> {
    let mut model = load_model(ctx, id);
    let placed = arrange_column(&mut model, &Arrange::default());
    if !placed.is_empty() {
        store_model(ctx, id, &model)?;
    }
    Ok(placed.len())
}

pub(super) fn update_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let many = v.get("pages").is_some_and(|p| !p.is_null())
        || str_field(v, "page").is_some_and(|p| is_all_pages(ctx, p));
    if !many {
        let id = page_arg(ctx, v, "page")?;
        return update_one(ctx, &id, v);
    }
    let ids = read_targets(ctx, v)?;
    if let [id] = ids.as_slice() {
        return update_one(ctx, id, v);
    }
    update_many(ctx, &ids, v)
}

/// `update` на нескольких страницах сразу (`pages`, `page="all"`,
/// `depth`): только настройки страницы, одинаковые для каждой. «Скрой
/// панель свойств везде» по вызову на страницу стоило бы локальной модели
/// хода с полным префиллом на каждую.
fn update_many(ctx: NotesCtx, ids: &[String], v: &Json) -> Result<String, String> {
    const SETTINGS: &str = "props_panel, grid, grid_step, snap, snap_step, bg";
    if let Some(k) = ["title", "icon", "content", "find", "layout"].into_iter().find(|k| v.get(k).is_some_and(|x| !x.is_null())) {
        return Err(format!(
            "\"{k}\" is set one page at a time; update on several pages (pages / page=\"all\") takes only \
             page settings: {SETTINGS}"
        ));
    }
    let mut changes = Vec::new();
    for id in ids {
        changes = apply_layout_args(ctx, id, v)?;
        if changes.is_empty() {
            return Err(format!("nothing to update: pass page settings ({SETTINGS})"));
        }
    }
    Ok(format!("{}\napplied to {} pages\n", changes.join("\n"), ids.len()))
}

fn update_one(ctx: NotesCtx, id: &str, v: &Json) -> Result<String, String> {
    let id = id.to_string();
    let mut changes: Vec<String> = Vec::new();

    if let Some(title) = str_field(v, "title") {
        let old = ctx.title_of(&id);
        ctx.rename_page(&id, title);
        changes.push(format!("renamed \"{old}\" → \"{}\"", ctx.title_of(&id)));
    }
    if let Some(icon) = raw_string(v, "icon") {
        let icon = icon.trim().to_string();
        let cleared = icon.is_empty() || matches!(icon.to_ascii_lowercase().as_str(), "none" | "null");
        ctx.set_icon(&id, (!cleared).then_some(icon.clone()));
        changes.push(if cleared { "icon cleared".to_string() } else { format!("icon set to {icon}") });
    }
    changes.extend(apply_layout_args(ctx, &id, v)?);

    let mode = str_field(v, "mode").map(|m| m.to_ascii_lowercase()).unwrap_or_else(|| "replace".to_string());
    if let Some(content) = raw_string(v, "content") {
        match mode.as_str() {
            "replace" => {
                write_page(ctx, &id, &content)?;
                changes.push(format!("content replaced: {} words now", count_words(&content)));
            }
            "append" | "prepend" | "insert" => {
                let mut model = load_model(ctx, &id);
                let fallback = if mode == "prepend" { InsertPos::Start } else { InsertPos::End };
                let pos = parse_pos(&model, v, fallback)?;
                let blocks = fragment_blocks(&content, &parse_geom(v)?, false)?;
                let idx = insert_blocks(&mut model, blocks, pos);
                store_model(ctx, &id, &model)?;
                changes.push(format!("inserted {} block{} {} ({})", idx.len(), if idx.len() == 1 { "" } else { "s" }, pos_text(pos), indices_text(&idx)));
            }
            other => return Err(format!("unknown mode \"{other}\" (replace | append | prepend | insert)")),
        }
    }

    if let Some(find) = raw_string(v, "find") {
        if find.is_empty() {
            return Err("\"find\" is empty".to_string());
        }
        let replace = raw_string(v, "replace").unwrap_or_default();
        let all = bool_field(v, "all").unwrap_or(false);
        let mut model = load_model(ctx, &id);
        let r = replace_in_blocks(&mut model, &find, &replace, all)?;
        store_model(ctx, &id, &model)?;
        changes.push(format!("replaced {} occurrence{}", r.n, if r.n == 1 { "" } else { "s" }));
        if let Some(t) = r.approx {
            changes.push(format!(
                "find did not match word for word — replaced the closest text instead:\n{t}\n(read the page if \
                 that was not the fragment you meant)"
            ));
        }
    }

    if changes.is_empty() {
        return Err(
            "nothing to update: pass title, icon, layout/grid/snap/props_panel, content (+mode) or find/replace"
                .to_string()
        );
    }
    let mut out = changes.join("\n");
    out.push('\n');
    out.push_str(&page_line(ctx, &id));
    out.push('\n');
    Ok(out)
}

pub(super) fn move_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let parent = parse_parent(ctx, v)?;
    let index = usize_field(v, "index");
    if !ctx.move_page(&id, parent.as_deref(), index) {
        return Err("move failed: a page can't be moved into its own subtree".to_string());
    }
    Ok(format!("moved\n{}\n", page_line(ctx, &id)))
}

pub(super) fn delete_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let tree = ctx.tree.get_untracked();
    let doomed = tree.subtree_ids(&id);
    let objects: usize = doomed.iter().map(|p| object_refs(&ctx.page_markdown(p)).len()).sum();
    let line = page_line(ctx, &id);
    drop(tree);
    ctx.delete_page(&id);
    Ok(format!(
        "deleted {} page{} and {objects} embedded object{}\n{line}\n",
        doomed.len(),
        if doomed.len() == 1 { "" } else { "s" },
        if objects == 1 { "" } else { "s" }
    ))
}

pub(super) fn duplicate_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let copy = ctx.duplicate_page_with(&id, false).ok_or("page vanished")?;
    Ok(format!("duplicated\n{}\n", page_line(ctx, &copy)))
}

/// Показать страницу пользователю: активная страница, маршрут «Заметки».
fn show_page(ctx: NotesCtx, id: Option<&str>) {
    if let Some(id) = id {
        ctx.activate(id);
    }
    crate::rail::navigate("notes");
}

pub(super) fn open_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = match str_field(v, "page") {
        Some(p) => Some(resolve_page_ref(ctx, p)?),
        None => None,
    };
    show_page(ctx, id.as_deref());
    Ok(match id {
        Some(id) => format!("opened\n{}\n", page_line(ctx, &id)),
        None => "opened Notes\n".to_string(),
    })
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// Байты вложения из аргументов: `path` (файл на диске) либо `attachment`
/// (вложение чата). Возвращает `(bytes, ext, stem, имя файла)`. Модель не
/// передаёт байты сама — только ссылку на файл.
pub(super) fn attachment_bytes(v: &Json) -> Result<(Vec<u8>, String, String, String), String> {
    if let Some(p) = str_field(v, "path") {
        let path = expand_home(p);
        let bytes = std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Ok((bytes, ext, stem, name))
    } else if let Some(r) = str_field(v, "attachment") {
        let r = r.strip_prefix("attachment:").unwrap_or(r);
        let a = crate::agent::tools::pipelines::find_attachment(r)
            .ok_or_else(|| format!("chat attachment \"{r}\" not found (name, sha prefix or last)"))?;
        let path = blobs::blob_path(&a.sha256, &a.ext);
        let bytes = std::fs::read(&path).map_err(|e| format!("can't read attachment blob: {e}"))?;
        let ext = if a.ext.is_empty() {
            Path::new(&a.original_name).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            a.ext.clone()
        };
        let stem = Path::new(&a.original_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok((bytes, ext, stem, a.original_name.clone()))
    } else {
        Err("pass \"path\" (file on disk) or \"attachment\" (chat attachment: name | sha | last)".to_string())
    }
}

pub(super) fn attach_impl(ctx: NotesCtx, v: &Json) -> Result<String, String> {
    let id = page_arg(ctx, v, "page")?;
    let (bytes, ext, default_caption, _) = attachment_bytes(v)?;
    let size = bytes.len();
    let url = media::ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
    let caption = str_field(v, "caption").map(str::to_string).unwrap_or(default_caption);
    let caption = caption.replace(['[', ']', '\n'], " ");
    let block = format!("![{caption}]({url})");
    let mut model = load_model(ctx, &id);
    let pos = parse_pos(&model, v, InsertPos::End)?;
    let blocks = fragment_blocks(&block, &parse_geom(v)?, false)?;
    let idx = insert_blocks(&mut model, blocks, pos);
    store_model(ctx, &id, &model)?;
    Ok(format!(
        "attached {url} ({size} bytes) as a media block {} ({})\n{}\n",
        pos_text(pos),
        indices_text(&idx),
        page_line(ctx, &id)
    ))
}
