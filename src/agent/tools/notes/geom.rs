//! Блоки на холсте: ссылка на блок, позиция вставки, геометрия,
//! оценка высоты и раскладка колонкой (`arrange`).

use super::*;

/// Ширина блока в потоке / без своей ширины (как `DocLayout::block_width`).
const DEFAULT_BLOCK_W: f32 = 520.0;

/// Куда вставлять фрагмент относительно верхнеуровневых блоков.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum InsertPos {
    End,
    Start,
    Index(usize),
}

/// Позиция из `index` / `after` / `before` (ссылки на блоки); без них —
/// `fallback`.
pub(super) fn parse_pos(model: &DocModel, v: &Json, fallback: InsertPos) -> Result<InsertPos, String> {
    if let Some(i) = usize_field(v, "index") {
        return Ok(InsertPos::Index(i.min(model.blocks.len())));
    }
    if let Some(a) = ref_field(v, "after") {
        return Ok(InsertPos::Index(resolve_block(model, &a)? + 1));
    }
    if let Some(b) = ref_field(v, "before") {
        return Ok(InsertPos::Index(resolve_block(model, &b)?));
    }
    Ok(fallback)
}

pub(super) fn pos_index(model: &DocModel, pos: InsertPos) -> usize {
    match pos {
        InsertPos::End => model.blocks.len(),
        InsertPos::Start => 0,
        InsertPos::Index(i) => i.min(model.blocks.len()),
    }
}

pub(super) fn pos_text(pos: InsertPos) -> String {
    match pos {
        InsertPos::End => "at the end of the page".to_string(),
        InsertPos::Start => "at the start of the page".to_string(),
        InsertPos::Index(i) => format!("at block #{i}"),
    }
}

/// Геометрия из аргументов `x y w h`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Geom {
    pub(super) x: Option<f32>,
    pub(super) y: Option<f32>,
    pub(super) w: Option<f32>,
    pub(super) h: Option<f32>,
}

pub(super) fn parse_geom(v: &Json) -> Result<Geom, String> {
    let num = |k: &str| -> Result<Option<f32>, String> {
        match v.get(k) {
            None | Some(Json::Null) => Ok(None),
            Some(_) => f32_field(v, k).filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("bad \"{k}\" — a number in px")),
        }
    };
    let g = Geom { x: num("x")?, y: num("y")?, w: num("w")?, h: num("h")? };
    if let Some(w) = g.w {
        if w < 40.0 {
            return Err("\"w\" must be at least 40 px".to_string());
        }
    }
    if let Some(h) = g.h {
        if h < 20.0 {
            return Err("\"h\" must be at least 20 px".to_string());
        }
    }
    if g.x.is_some() != g.y.is_some() {
        return Err("pass both \"x\" and \"y\" to place a block on the canvas".to_string());
    }
    Ok(g)
}

impl Geom {
    pub(super) fn is_empty(&self) -> bool {
        self.x.is_none() && self.w.is_none() && self.h.is_none()
    }

    pub(super) fn apply(&self, attrs: &mut Attrs) {
        if let (Some(x), Some(y)) = (self.x, self.y) {
            free::set_pos(attrs, x, y);
        }
        if let Some(w) = self.w {
            free::set_width(attrs, w);
        }
        if let Some(h) = self.h {
            free::set_height(attrs, h);
        }
    }
}

/// Число в атрибут: до десятых, целые — без хвоста.
pub(super) fn fnum(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < f32::EPSILON {
        format!("{}", r.round() as i64)
    } else {
        format!("{r}")
    }
}

/// Блок по ссылке агента: индекс верхнего уровня (`3`, `#3`) либо
/// `find:<фрагмент>` / текст — единственное вхождение в markdown блока.
pub(super) fn resolve_block(model: &DocModel, s: &str) -> Result<usize, String> {
    let t = s.trim();
    let t = t.strip_prefix("block:").unwrap_or(t).trim();
    let n = model.blocks.len();
    if let Ok(i) = t.trim_start_matches('#').parse::<usize>() {
        return (i < n).then_some(i).ok_or_else(|| format!("block #{i} does not exist — the page has {n} blocks (blocks op=list)"));
    }
    // «#3.0» — вложенный блок из op=list: операции работают с верхним
    // уровнем, ребёнка сначала вынимают наружу.
    if let Some((parent, child)) = t.trim_start_matches('#').split_once('.') {
        if let (Ok(p), Ok(c)) = (parent.trim().parse::<usize>(), child.trim().parse::<usize>()) {
            return Err(format!(
                "#{p}.{c} is nested inside block #{p} — take it out with blocks op=unnest block={p} child={c}, \
                 or rewrite the whole container with blocks op=set_markdown block={p}"
            ));
        }
    }
    let needle = t.strip_prefix("find:").unwrap_or(t).trim().to_lowercase();
    if needle.is_empty() {
        return Err("empty block reference — pass an index from blocks op=list or find:<text>".to_string());
    }
    let hits: Vec<usize> =
        (0..n).filter(|&i| block_markdown(&model.blocks[i]).to_lowercase().contains(&needle)).collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!("no block contains \"{t}\" — see blocks op=list")),
        _ => Err(format!(
            "{} blocks contain \"{t}\" — use the index: {}",
            hits.len(),
            hits.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Оценка высоты блока в px (модель не знает раскладки текста — числа
/// приблизительные, в выводе помечаются тильдой).
fn est_height(b: &DocBlock, w: f32) -> f32 {
    fn text_lines(chars: usize, w: f32, glyph: f32) -> f32 {
        let per_line = ((w - 16.0) / glyph).max(8.0);
        (chars as f32 / per_line).ceil().max(1.0)
    }
    match &b.kind {
        BlockKind::Shape { shape } if shape.is_line() => shape::line_box(&b.attrs, *shape).height,
        BlockKind::Shape { .. } => shape::height_of(&b.attrs),
        BlockKind::Media { .. } => free::height_of(&b.attrs).unwrap_or(220.0),
        BlockKind::Divider => 17.0,
        BlockKind::Embed { .. } => free::height_of(&b.attrs).unwrap_or(200.0),
        BlockKind::Table { rows, .. } => (rows.len() as f32 + 1.0) * 30.0,
        BlockKind::CodeBlock { code, .. } => code.lines().count().max(1) as f32 * 20.0 + 16.0,
        BlockKind::Heading { level, text } => {
            let size = match level {
                1 => 32.0,
                2 => 26.0,
                3 => 22.0,
                _ => 18.0,
            };
            text_lines(text.text().chars().count(), w, size * 0.55) * size * 1.4 + 8.0
        }
        _ => {
            let chars = b.kind.text().map(|t| t.text().chars().count()).unwrap_or(0);
            let own = text_lines(chars, w, 8.5) * 24.0 + 8.0;
            let children: f32 = b.kind.children().map(|c| c.iter().map(|x| est_height(x, w - 24.0)).sum()).unwrap_or(0.0);
            own + children
        }
    }
}

fn block_width(b: &DocBlock) -> f32 {
    free::width_of(&b.attrs).unwrap_or(DEFAULT_BLOCK_W)
}

/// Прямоугольник закреплённого блока на холсте (высота — из `h` либо оценка).
pub(super) fn block_rect(b: &DocBlock) -> Option<(f32, f32, f32, f32)> {
    let (x, y) = free::pos_of(&b.attrs)?;
    let w = block_width(b);
    let h = free::height_of(&b.attrs).unwrap_or_else(|| est_height(b, w));
    Some((x, y, w, h))
}

/// Параметры [`arrange_column`]. `x`/`y` — начало колонки (иначе — под
/// нижним закреплённым блоком у его левого края, на пустом холсте 40×40),
/// `w` — ширина всем разложенным (иначе своя у блока, иначе
/// [`DEFAULT_BLOCK_W`]), `all` — перекладывать и уже закреплённые.
pub(super) struct Arrange {
    pub(super) x: Option<f32>,
    pub(super) y: Option<f32>,
    pub(super) w: Option<f32>,
    pub(super) gap: f32,
    pub(super) all: bool,
}

impl Default for Arrange {
    fn default() -> Self {
        Self { x: None, y: None, w: None, gap: ARRANGE_GAP, all: false }
    }
}

/// Зазор между блоками колонки по умолчанию, px.
pub(super) const ARRANGE_GAP: f32 = 24.0;
/// Левый край и верх колонки на пустом холсте, px.
const ARRANGE_ORIGIN: f32 = 40.0;

/// Разложить блоки колонкой в порядке документа: каждый следующий — под
/// предыдущим с зазором `gap`. Берёт неприкреплённые блоки (или все при
/// `all`). У объектов без своей высоты (фигуры, медиа, врезки) высота
/// фиксируется оценкой — чтобы редактор и агент считали одно и то же.
/// Возвращает индексы разложенных блоков.
pub(super) fn arrange_column(model: &mut DocModel, a: &Arrange) -> Vec<usize> {
    let targets: Vec<usize> = (0..model.blocks.len())
        .filter(|&i| a.all || free::pos_of(&model.blocks[i].attrs).is_none())
        .collect();
    if targets.is_empty() {
        return targets;
    }
    let others: Vec<(f32, f32, f32, f32)> = model
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| !targets.contains(i))
        .filter_map(|(_, b)| block_rect(b))
        .collect();
    let x = a.x.unwrap_or_else(|| {
        others
            .iter()
            .map(|r| r.0)
            .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.min(v))))
            .unwrap_or(ARRANGE_ORIGIN)
            .max(0.0)
    });
    let mut y = a.y.unwrap_or_else(|| {
        others
            .iter()
            .map(|r| r.1 + r.3)
            .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
            .map(|bottom| bottom + a.gap)
            .unwrap_or(ARRANGE_ORIGIN)
    });
    for &i in &targets {
        let b = &mut model.blocks[i];
        let w = a.w.or_else(|| free::width_of(&b.attrs)).unwrap_or(DEFAULT_BLOCK_W);
        free::set_pos(&mut b.attrs, x.round(), y.round());
        free::set_width(&mut b.attrs, w);
        if needs_own_height(b) && free::height_of(&b.attrs).is_none() {
            let h = est_height(b, w).round();
            free::set_height(&mut b.attrs, h);
        }
        if let BlockKind::Shape { shape } = b.kind {
            if shape.is_line() {
                canonicalize_line(b, shape);
            }
        }
        let h = free::height_of(&b.attrs).unwrap_or_else(|| est_height(b, w));
        y = (y + h + a.gap).round();
    }
    targets
}

/// Блок, у которого нет собственной высоты по содержимому: без `h` редактор
/// возьмёт свою (200 px), и оценка агента с ней разойдётся.
fn needs_own_height(b: &DocBlock) -> bool {
    matches!(b.kind, BlockKind::Shape { .. } | BlockKind::Media { .. } | BlockKind::Embed { .. })
}

pub(super) fn is_line(b: &DocBlock) -> bool {
    matches!(b.kind, BlockKind::Shape { shape } if shape.is_line())
}

/// Закреплённые блоки, чьи рамки пересекают рамку блока `i`, — строкой для
/// ответа (`!! overlaps #3 (x=… y=… w=… h=…)`), либо `None`. Не запрет, а
/// подсказка: высоты текста оценочные (в списке помечены `~`). Линии не в
/// счёт — стрелка между двумя блоками пересекает оба по замыслу.
fn overlap_note(model: &DocModel, i: usize) -> Option<String> {
    let me = model.blocks.get(i)?;
    if is_line(me) {
        return None;
    }
    let (x, y, w, h) = block_rect(me)?;
    let hits: Vec<String> = model
        .blocks
        .iter()
        .enumerate()
        .filter(|(j, b)| *j != i && !is_line(b))
        .filter_map(|(j, b)| {
            let (ox, oy, ow, oh) = block_rect(b)?;
            let overlaps = x < ox + ow - 1.0 && ox < x + w - 1.0 && y < oy + oh - 1.0 && oy < y + h - 1.0;
            overlaps.then(|| format!("#{j} (x={} y={} w={} h={})", fnum(ox), fnum(oy), fnum(ow), fnum(oh)))
        })
        .collect();
    (!hits.is_empty()).then(|| format!("!! overlaps {}", hits.join(", ")))
}

/// Строка блока для ответа плюс предупреждение о наложении, если есть.
pub(super) fn block_line_checked(model: &DocModel, i: usize) -> String {
    let mut s = block_line(i, &model.blocks[i]);
    if let Some(note) = overlap_note(model, i) {
        s.push('\n');
        s.push_str(&note);
    }
    s
}

fn kind_label(b: &DocBlock) -> String {
    match &b.kind {
        BlockKind::Heading { level, .. } => format!("heading{level}"),
        BlockKind::Shape { shape } => format!("shape:{}", shape.name()),
        BlockKind::Embed { target } => format!("embed:{}", target.trim()),
        other => props::kind_name(other).to_string(),
    }
}

/// Абсолютные точки линейной фигуры (концы и, у кривой, направляющие) —
/// только у закреплённого блока; иначе локальные.
fn line_points_abs(b: &DocBlock, kind: ShapeKind) -> Vec<(f32, f32)> {
    let (ox, oy) = free::pos_of(&b.attrs).map(|(x, y)| (x + shape::LINE_PAD, y + shape::LINE_PAD)).unwrap_or((0.0, 0.0));
    shape::line_handles(&b.attrs, kind).into_iter().map(|(x, y)| (x + ox, y + oy)).collect()
}

/// Строка блока в `blocks op=list`.
pub(super) fn block_line(i: usize, b: &DocBlock) -> String {
    let mut s = format!("#{i} {}", kind_label(b));
    let label = props::label_of(b).replace(['\n', '"'], " ");
    if !label.trim().is_empty() && !matches!(b.kind, BlockKind::Shape { .. } | BlockKind::Embed { .. }) {
        s.push_str(&format!(" \"{}\"", label.trim()));
    }
    let w = block_width(b);
    match free::pos_of(&b.attrs) {
        Some((x, y)) => s.push_str(&format!(" · x={} y={} w={}", fnum(x), fnum(y), fnum(w))),
        None => s.push_str(" · flow"),
    }
    match free::height_of(&b.attrs) {
        Some(h) => s.push_str(&format!(" h={}", fnum(h))),
        None => s.push_str(&format!(" h=~{}", fnum(est_height(b, w)))),
    }
    let mut rest = Attrs::default();
    for (k, v) in b.attrs.0.iter() {
        let is_point = matches!(b.kind, BlockKind::Shape { shape } if shape.is_line())
            && matches!(k.as_str(), "x1" | "y1" | "x2" | "y2" | "cx1" | "cy1" | "cx2" | "cy2");
        if !free::is_geom_key(k) && !is_point {
            rest.set(k.clone(), v.clone());
        }
    }
    if let BlockKind::Shape { shape } = &b.kind {
        if shape.is_line() {
            let pts = line_points_abs(b, *shape);
            let p = |i: usize| format!("({},{})", fnum(pts[i].0), fnum(pts[i].1));
            s.push_str(&format!(" · from {} to {}", p(0), p(1)));
            if shape.is_curve() && pts.len() == 4 {
                s.push_str(&format!(" via {} {}", p(2), p(3)));
            }
            if free::pos_of(&b.attrs).is_none() {
                s.push_str(" (relative — not pinned)");
            }
        }
    }
    if !rest.is_empty() {
        s.push_str(&format!(" · {}", serialize_attrs(&rest)));
    }
    s
}

pub(super) fn blocks_text(model: &DocModel) -> String {
    if model.blocks.is_empty() {
        return "(no blocks)\n".to_string();
    }
    let mut out = String::new();
    for (i, b) in model.blocks.iter().enumerate() {
        out.push_str(&block_line(i, b));
        out.push('\n');
        push_children(&mut out, &i.to_string(), b, 1);
    }
    out
}

/// Дети контейнера — строками `#3.0` с отступом. Без них не видно, что
/// таблица уже лежит внутри toggle: у вложенного блока нет ни своего
/// индекса верхнего уровня, ни геометрии (она бывает только у корня).
fn push_children(out: &mut String, path: &str, b: &DocBlock, depth: usize) {
    let Some(children) = b.kind.children() else { return };
    for (j, c) in children.iter().enumerate() {
        let p = format!("{path}.{j}");
        let label = props::label_of(c).replace(['\n', '"'], " ");
        out.push_str(&"  ".repeat(depth));
        out.push_str(&format!("#{p} {}", kind_label(c)));
        if !label.trim().is_empty() && !matches!(c.kind, BlockKind::Shape { .. } | BlockKind::Embed { .. }) {
            out.push_str(&format!(" \"{}\"", label.trim()));
        }
        out.push('\n');
        push_children(out, &p, c, depth + 1);
    }
}

/// Разобрать фрагмент markdown в блоки; геометрия — первому (`last=false`)
/// или последнему блоку фрагмента.
pub(super) fn fragment_blocks(md: &str, geom: &Geom, last: bool) -> Result<Vec<DocBlock>, String> {
    let mut blocks = parse_document(md).blocks;
    if blocks.is_empty() {
        return Err("the markdown fragment is empty".to_string());
    }
    if !geom.is_empty() {
        let idx = if last { blocks.len() - 1 } else { 0 };
        geom.apply(&mut blocks[idx].attrs);
    }
    Ok(blocks)
}

/// Вставить блоки в позицию; возвращает индексы вставленных.
pub(super) fn insert_blocks(model: &mut DocModel, blocks: Vec<DocBlock>, pos: InsertPos) -> Vec<usize> {
    let at = pos_index(model, pos);
    let n = blocks.len();
    let tail = model.blocks.split_off(at);
    model.blocks.extend(blocks);
    model.blocks.extend(tail);
    (at..at + n).collect()
}

pub(super) fn indices_text(idx: &[usize]) -> String {
    idx.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ")
}

/// Заменить блок `i` результатом разбора `md`; первый новый блок наследует
/// атрибуты старого (те, которых у него нет).
pub(super) fn replace_block(model: &mut DocModel, i: usize, md: &str) -> Result<Vec<usize>, String> {
    let old = model.blocks.remove(i);
    let mut fresh = parse_document(md).blocks;
    if fresh.is_empty() {
        return Ok(Vec::new());
    }
    for (k, v) in old.attrs.0.iter() {
        if fresh[0].attrs.get(k).is_none() {
            fresh[0].attrs.set(k.clone(), v.clone());
        }
    }
    Ok(insert_blocks(model, fresh, InsertPos::Index(i)))
}

/// `find`/`replace` по блокам: совпадение ищется внутри markdown одного
/// верхнеуровневого блока, блок перепарсивается с сохранением атрибутов.
pub(super) fn replace_in_blocks(model: &mut DocModel, find: &str, replace: &str, all: bool) -> Result<usize, String> {
    let per_block: Vec<usize> = model.blocks.iter().map(|b| block_markdown(b).matches(find).count()).collect();
    let n: usize = per_block.iter().sum();
    if n == 0 {
        let joined: String = model.blocks.iter().map(block_markdown).collect::<Vec<_>>().join("\n");
        return Err(if joined.contains(find) {
            "find text spans several blocks — replace it with blocks op=set_markdown, or update with \
             content"
                .to_string()
        } else {
            "find text not found on the page — read the page and copy the fragment exactly \
             (the page is compared as markdown, not as rendered text)"
                .to_string()
        });
    }
    if n > 1 && !all {
        return Err(format!(
            "find text occurs {n} times — pass all=true to replace every occurrence, or a longer \
             unique fragment"
        ));
    }
    // С конца — индексы впереди не съезжают.
    for i in (0..model.blocks.len()).rev() {
        if per_block[i] == 0 {
            continue;
        }
        let md = block_markdown(&model.blocks[i]);
        let new_md = if all { md.replace(find, replace) } else { md.replacen(find, replace, 1) };
        replace_block(model, i, &new_md)?;
    }
    Ok(n)
}
