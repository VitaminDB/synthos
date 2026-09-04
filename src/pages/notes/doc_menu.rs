//! Контекстное меню документа (правый клик) и локализованный slash-каталог.
//!
//! Редактор на правый клик ставит каретку в точку клика и отдаёт позицию
//! ([`DocumentEditor::on_context_menu`]); здесь по ней открывается
//! `PopupMenu`. Пункты — вставка блока в место каретки, «превратить в»,
//! действия над блоком (дублировать/сдвинуть/удалить) — идут в редактор
//! очередью [`DocOp`] через ручку страницы (`NotesCtx::doc_op`): правка
//! проходит через историю undo и ставит каретку в новый блок.
//!
//! Подменю «Вставить» и «Превратить в» разбиты на категории-подменю —
//! плоским списком из двадцати с лишним пунктов меню не влезало на экран:
//! «Текст» (абзац, заголовки), «Списки» (маркированный, нумерованный,
//! чек-лист, раскрывающийся), «Блоки» (цитата, выноска, код, таблица,
//! разделитель), «Объекты» (канбан-доска и диаграмма Ганта — объекты
//! проекта `![[kanban:<id>]]` / `![[gantt:<id>]]`, см. [`super::kanban`],
//! [`super::gantt`]), «Примитивы» (прямоугольник … двойная стрелка — блоки
//! `![[shape:<вид>]]`, настраиваются в панели свойств) и «Медиа»
//! («Картинка…», «SVG-файл…», «SVG из буфера», «Файл…» — вложения бандла,
//! см. [`super::media`]). «Превратить в» — те же категории без объектов и
//! медиа (в них блок не превращается).

use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocOp, ShapeKind, SlashAction, SlashItem};
use syngui::widgets::{MenuItem, PopupAnchor, PopupMenu};

use crate::context::AppCtx;
use crate::icons::*;

use super::calendar::model::CalView;
use super::media::{self, PickKind};
use super::state::NotesCtx;

/// Примитивы в порядке меню: id, вид, иконка, ключ подписи.
const SHAPES: [(&str, ShapeKind, &str, &str); 10] = [
    ("rect", ShapeKind::Rect, MI_CROP_SQUARE, "notes.shape.rect"),
    ("ellipse", ShapeKind::Ellipse, MI_CIRCLE, "notes.shape.ellipse"),
    ("triangle", ShapeKind::Triangle, MI_CHANGE_HISTORY, "notes.shape.triangle"),
    ("diamond", ShapeKind::Diamond, MI_DIAMOND, "notes.shape.diamond"),
    ("line", ShapeKind::Line, MI_HORIZONTAL_RULE, "notes.shape.line"),
    ("arrow", ShapeKind::Arrow, MI_ARROW_RIGHT_ALT, "notes.shape.arrow"),
    ("arrow2", ShapeKind::DoubleArrow, MI_COMPARE_ARROWS, "notes.shape.arrow2"),
    ("curve", ShapeKind::Curve, MI_GESTURE, "notes.shape.curve"),
    ("curve-arrow", ShapeKind::CurveArrow, MI_REDO, "notes.shape.curve_arrow"),
    ("curve-arrow2", ShapeKind::CurveDoubleArrow, MI_SWAP_CALLS, "notes.shape.curve_arrow2"),
];

/// Подпись вида примитива на языке интерфейса.
pub fn shape_label(kind: ShapeKind) -> String {
    SHAPES
        .iter()
        .find(|(_, k, _, _)| *k == kind)
        .map(|(_, _, _, key)| syngui::i18n::tr(key))
        .unwrap_or_default()
}

pub fn shape_icon(kind: ShapeKind) -> &'static str {
    SHAPES
        .iter()
        .find(|(_, k, _, _)| *k == kind)
        .map(|(_, _, icon, _)| *icon)
        .unwrap_or(MI_CROP_SQUARE)
}

/// Объекты-примитивы: id, иконка, ключ подписи.
const OBJECTS: [(&str, &str, &str); 4] = [
    ("kanban", MI_VIEW_KANBAN, "notes.block.kanban"),
    ("gantt", MI_VIEW_TIMELINE, "notes.block.gantt"),
    ("mindmap", MI_SCHEMA, "notes.block.mindmap"),
    ("calendar", MI_CALENDAR_MONTH, "notes.block.calendar"),
];

/// Виды календаря для подменю вставки: id пункта, вид, иконка, ключ.
const CALENDAR_VIEWS: [(&str, CalView, &str, &str); 4] = [
    ("ins_calendar_year", CalView::Year, MI_DATE_RANGE, "notes.calendar.view.year"),
    ("ins_calendar_month", CalView::Month, MI_CALENDAR_MONTH, "notes.calendar.view.month"),
    ("ins_calendar_week", CalView::Week, MI_CALENDAR_VIEW_WEEK, "notes.calendar.view.week"),
    ("ins_calendar_day", CalView::Day, MI_CALENDAR_VIEW_DAY, "notes.calendar.view.day"),
];

/// Пункты подменю «Примитивы» с общим префиксом (вставка / превратить в).
fn shape_items(prefix: &str) -> Vec<MenuItem> {
    SHAPES
        .iter()
        .map(|(id, _, icon, key)| {
            MenuItem::new(format!("{prefix}{id}"), syngui::i18n::tr(key)).icon(*icon)
        })
        .collect()
}

/// Пункты вставки доски, диаграммы и интеллект-карты (у карты — подменю:
/// пустая либо из текущего блока-списка).
fn object_items() -> Vec<MenuItem> {
    OBJECTS
        .iter()
        .map(|(id, icon, key)| {
            let item = MenuItem::new(format!("ins_object_{id}"), syngui::i18n::tr(key)).icon(*icon);
            if *id == "mindmap" {
                item.children(vec![
                    MenuItem::new("ins_object_mindmap", tr!("notes.mindmap.empty_map")).icon(MI_SCHEMA),
                    MenuItem::new("ins_mindmap_from_block", tr!("notes.mindmap.from_block")).icon(MI_FORMAT_LIST_BULLETED),
                ])
            } else if *id == "calendar" {
                item.children(
                    CALENDAR_VIEWS
                        .iter()
                        .map(|(mid, _, icon, key)| MenuItem::new(*mid, syngui::i18n::tr(key)).icon(*icon))
                        .collect(),
                )
            } else {
                item
            }
        })
        .collect()
}

/// Виджет календаря заданного вида — врезка в место каретки.
pub fn insert_calendar(ctx: NotesCtx, view: CalView) {
    let id = ctx.create_calendar(view);
    ctx.doc_op(DocOp::InsertMarkdown(format!(
        "![[calendar:{id}]]{{h={}}}",
        super::embeds::default_object_h("calendar") as i64
    )));
}

/// Интеллект-карта из текущего блока: заголовок/список → узлы; блок
/// заменяется врезкой карты.
pub fn insert_mindmap_from_block(ctx: NotesCtx) {
    let Some(page) = ctx.active.get_untracked().and_then(|id| ctx.page(&id)) else { return };
    let Some(block) = page.handle.selected().get_untracked() else {
        insert_object(ctx, "mindmap");
        return;
    };
    let md = page.handle.block_markdown(block).unwrap_or_default();
    let doc = super::mindmap::model::MindmapDoc::from_outline(&md, &tr!("notes.mindmap.root"));
    let id = ctx.create_mindmap(doc);
    ctx.doc_op(DocOp::InsertMarkdown(format!(
        "![[mindmap:{id}]]{{h={}}}",
        super::embeds::default_object_h("mindmap") as i64
    )));
    ctx.doc_op(DocOp::DeleteBlock(block));
}

fn shape_of(id: &str) -> Option<ShapeKind> {
    SHAPES.iter().find(|(name, _, _, _)| *name == id).map(|(_, k, _, _)| *k)
}

/// Slash-каталог «/» на языке интерфейса + объекты проекта.
pub fn slash_items() -> Vec<SlashItem> {
    vec![
        SlashItem::new(SlashAction::Paragraph, tr!("notes.block.text"), "text paragraph текст абзац"),
        SlashItem::new(SlashAction::Heading(1), tr!("notes.block.h1"), "h1 heading заголовок"),
        SlashItem::new(SlashAction::Heading(2), tr!("notes.block.h2"), "h2 heading заголовок"),
        SlashItem::new(SlashAction::Heading(3), tr!("notes.block.h3"), "h3 heading заголовок"),
        SlashItem::new(SlashAction::Bullet, tr!("notes.block.bullet"), "bullet list список"),
        SlashItem::new(SlashAction::Numbered, tr!("notes.block.numbered"), "numbered ordered список"),
        SlashItem::new(SlashAction::Todo, tr!("notes.block.todo"), "todo checkbox задача чеклист"),
        SlashItem::new(SlashAction::Toggle, tr!("notes.block.toggle"), "toggle collapse свернуть"),
        SlashItem::new(SlashAction::Quote, tr!("notes.block.quote"), "quote цитата"),
        SlashItem::new(SlashAction::Callout, tr!("notes.block.callout"), "callout note заметка"),
        SlashItem::new(SlashAction::CodeBlock, tr!("notes.block.code"), "code код"),
        SlashItem::new(SlashAction::Table, tr!("notes.block.table"), "table таблица"),
        SlashItem::new(SlashAction::Divider, tr!("notes.block.divider"), "divider hr разделитель"),
        SlashItem::new(SlashAction::Custom("kanban".into()), tr!("notes.block.kanban"), "kanban board канбан доска задачи"),
        SlashItem::new(SlashAction::Custom("gantt".into()), tr!("notes.block.gantt"), "gantt timeline гант диаграмма план сроки"),
        SlashItem::new(SlashAction::Custom("mindmap".into()), tr!("notes.block.mindmap"), "mindmap mind map карта идей интеллект-карта"),
        SlashItem::new(SlashAction::Custom("calendar-month".into()), tr!("notes.calendar.insert.month"), "calendar month календарь месяц события"),
        SlashItem::new(SlashAction::Custom("calendar-week".into()), tr!("notes.calendar.insert.week"), "calendar week календарь неделя расписание"),
        SlashItem::new(SlashAction::Custom("calendar-day".into()), tr!("notes.calendar.insert.day"), "calendar day календарь день расписание"),
        SlashItem::new(SlashAction::Custom("calendar-year".into()), tr!("notes.calendar.insert.year"), "calendar year календарь год"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Rect), tr!("notes.shape.rect"), "rect shape прямоугольник фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Ellipse), tr!("notes.shape.ellipse"), "ellipse circle овал круг фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Triangle), tr!("notes.shape.triangle"), "triangle треугольник фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Diamond), tr!("notes.shape.diamond"), "diamond ромб фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Line), tr!("notes.shape.line"), "line линия фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Arrow), tr!("notes.shape.arrow"), "arrow стрелка фигура"),
        SlashItem::new(SlashAction::Shape(ShapeKind::DoubleArrow), tr!("notes.shape.arrow2"), "arrow double двойная стрелка"),
        SlashItem::new(SlashAction::Shape(ShapeKind::Curve), tr!("notes.shape.curve"), "curve bezier кривая безье дуга"),
        SlashItem::new(SlashAction::Shape(ShapeKind::CurveArrow), tr!("notes.shape.curve_arrow"), "curve arrow bezier кривая стрелка"),
        SlashItem::new(SlashAction::Shape(ShapeKind::CurveDoubleArrow), tr!("notes.shape.curve_arrow2"), "curve arrow double кривая двойная"),
        SlashItem::new(SlashAction::Custom("image".into()), tr!("notes.menu.image"), "image picture картинка изображение"),
        SlashItem::new(SlashAction::Custom("svg".into()), tr!("notes.menu.svg"), "svg вектор картинка"),
        SlashItem::new(SlashAction::Custom("file".into()), tr!("notes.menu.file"), "file файл вложение"),
    ]
}

/// Обработчик кастомных пунктов slash-меню: объекты проекта и медиа.
pub fn slash_custom(ctx: NotesCtx) -> impl Fn(&str) + Send + Sync + 'static {
    move |id| match id {
        "image" => media::pick_and_insert(ctx, PickKind::Image),
        "svg" => media::pick_and_insert(ctx, PickKind::Svg),
        "file" => media::pick_and_insert(ctx, PickKind::File),
        "calendar-year" => insert_calendar(ctx, CalView::Year),
        "calendar-month" | "calendar" => insert_calendar(ctx, CalView::Month),
        "calendar-week" => insert_calendar(ctx, CalView::Week),
        "calendar-day" => insert_calendar(ctx, CalView::Day),
        _ => insert_object(ctx, id),
    }
}

/// SVG из буфера обмена; если там не разметка — подсказка тостом.
fn paste_svg(ctx: NotesCtx) {
    if !media::insert_svg_from_clipboard(ctx) {
        use_context::<AppCtx>().notifications.info(tr!("notes.menu.svg_clipboard.empty"));
    }
}

/// Создать доску/диаграмму и вставить живую врезку в место каретки: сразу
/// с высотой по умолчанию, чтобы в свободной раскладке блок был нужного
/// размера и его можно было тянуть за нижнюю кромку.
pub fn insert_object(ctx: NotesCtx, kind: &str) {
    if let Some(id) = ctx.create_object(kind) {
        ctx.doc_op(DocOp::InsertMarkdown(format!(
            "![[{kind}:{id}]]{{h={}}}",
            super::embeds::default_object_h(kind) as i64
        )));
    }
}

/// Категория — пункт с подменю.
fn category(id: &str, label: String, icon: &str, children: Vec<MenuItem>) -> MenuItem {
    MenuItem::new(id, label).icon(icon).children(children)
}

/// «Текст»: абзац и заголовки.
fn text_items(prefix: &str) -> Vec<MenuItem> {
    vec![
        MenuItem::new(format!("{prefix}text"), tr!("notes.block.text")).icon(MI_ARTICLE),
        MenuItem::new(format!("{prefix}h1"), tr!("notes.block.h1")).icon(MI_FORMAT_BOLD),
        MenuItem::new(format!("{prefix}h2"), tr!("notes.block.h2")).icon(MI_FORMAT_BOLD),
        MenuItem::new(format!("{prefix}h3"), tr!("notes.block.h3")).icon(MI_FORMAT_BOLD),
    ]
}

/// «Списки»: маркированный, нумерованный, чек-лист, раскрывающийся блок.
fn list_items(prefix: &str) -> Vec<MenuItem> {
    vec![
        MenuItem::new(format!("{prefix}bullet"), tr!("notes.block.bullet")).icon(MI_FORMAT_LIST_BULLETED),
        MenuItem::new(format!("{prefix}numbered"), tr!("notes.block.numbered")).icon(MI_FORMAT_LIST_NUMBERED),
        MenuItem::new(format!("{prefix}todo"), tr!("notes.block.todo")).icon(MI_CHECK),
        MenuItem::new(format!("{prefix}toggle"), tr!("notes.block.toggle")).icon(MI_EXPAND_MORE),
    ]
}

/// «Блоки»: цитата, выноска, код; при вставке — ещё таблица и разделитель
/// (в них текстовый блок не превращается).
fn block_items(prefix: &str, with_layout: bool) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::new(format!("{prefix}quote"), tr!("notes.block.quote")).icon(MI_WRAP_TEXT),
        MenuItem::new(format!("{prefix}callout"), tr!("notes.block.callout")).icon(MI_CAMPAIGN),
        MenuItem::new(format!("{prefix}code"), tr!("notes.block.code")).icon(MI_CODE),
    ];
    if with_layout {
        items.push(MenuItem::new(format!("{prefix}table"), tr!("notes.block.table")).icon(MI_GRID_ON));
        items.push(MenuItem::new(format!("{prefix}divider"), tr!("notes.block.divider")).icon(MI_HORIZONTAL_RULE));
    }
    items
}

/// «Медиа»: вложения бандла.
fn media_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("ins_image", tr!("notes.menu.image")).icon(MI_IMAGE_ICON),
        MenuItem::new("ins_svg", tr!("notes.menu.svg")).icon(MI_BRUSH),
        MenuItem::new("ins_svg_clip", tr!("notes.menu.svg_clipboard")).icon(MI_CODE),
        MenuItem::new("ins_file", tr!("notes.menu.file")).icon(MI_ATTACH_FILE),
    ]
}

fn items() -> Vec<MenuItem> {
    let insert = vec![
        category("ins_cat_text", tr!("notes.menu.cat.text"), MI_ARTICLE, text_items("ins_")),
        category("ins_cat_lists", tr!("notes.menu.cat.lists"), MI_FORMAT_LIST_BULLETED, list_items("ins_")),
        category("ins_cat_blocks", tr!("notes.menu.cat.blocks"), MI_DASHBOARD_CUSTOMIZE, block_items("ins_", true)),
        category("ins_cat_objects", tr!("notes.menu.cat.objects"), MI_VIEW_KANBAN, object_items()),
        category("ins_shapes", tr!("notes.menu.shapes"), MI_CATEGORY, shape_items("ins_shape_")),
        category("ins_cat_media", tr!("notes.menu.cat.media"), MI_IMAGE_ICON, media_items()),
    ];
    let turn = vec![
        category("turn_cat_text", tr!("notes.menu.cat.text"), MI_ARTICLE, text_items("turn_")),
        category("turn_cat_lists", tr!("notes.menu.cat.lists"), MI_FORMAT_LIST_BULLETED, list_items("turn_")),
        category("turn_cat_blocks", tr!("notes.menu.cat.blocks"), MI_DASHBOARD_CUSTOMIZE, block_items("turn_", false)),
        category("turn_shapes", tr!("notes.menu.shapes"), MI_CATEGORY, shape_items("turn_shape_")),
    ];
    vec![
        MenuItem::new("insert", tr!("notes.menu.insert")).icon(MI_ADD).children(insert),
        MenuItem::new("turn", tr!("notes.menu.turn_into")).icon(MI_AUTORENEW).children(turn),
        MenuItem::separator(),
    ]
    .into_iter()
    .chain(block_action_items())
    .collect()
}

/// Действия над блоком — общие для меню документа и строки панели «Блоки».
/// Буфер обмена работает над выделенными блоками (рамка, Ctrl+клик), без
/// выделения — над текущим; «Вставить» ставит блоки из буфера следом.
pub fn block_action_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("copy", tr!("app.copy")).icon(MI_CONTENT_COPY),
        MenuItem::new("cut", tr!("app.cut")).icon(MI_CONTENT_CUT),
        MenuItem::new("paste", tr!("app.paste")).icon(MI_CONTENT_PASTE),
        MenuItem::separator(),
        MenuItem::new("dup", tr!("notes.menu.duplicate")).icon(MI_CONTENT_COPY),
        MenuItem::new("up", tr!("notes.menu.move_up")).icon(MI_ARROW_UPWARD),
        MenuItem::new("down", tr!("notes.menu.move_down")).icon(MI_ARROW_DOWNWARD),
        MenuItem::separator(),
        MenuItem::new("del", tr!("notes.menu.delete_block")).icon(MI_DELETE),
    ]
}

fn action_of(kind: &str) -> Option<SlashAction> {
    Some(match kind {
        "text" => SlashAction::Paragraph,
        "h1" => SlashAction::Heading(1),
        "h2" => SlashAction::Heading(2),
        "h3" => SlashAction::Heading(3),
        "bullet" => SlashAction::Bullet,
        "numbered" => SlashAction::Numbered,
        "todo" => SlashAction::Todo,
        "toggle" => SlashAction::Toggle,
        "quote" => SlashAction::Quote,
        "callout" => SlashAction::Callout,
        "code" => SlashAction::CodeBlock,
        "table" => SlashAction::Table,
        "divider" => SlashAction::Divider,
        _ => return None,
    })
}

pub fn handle(ctx: NotesCtx, id: &str) {
    if let Some(name) = id.strip_prefix("ins_shape_").and_then(shape_of) {
        ctx.doc_op(DocOp::InsertBlock(SlashAction::Shape(name)));
        return;
    }
    if let Some(name) = id.strip_prefix("turn_shape_").and_then(shape_of) {
        ctx.doc_op(DocOp::TurnInto(SlashAction::Shape(name)));
        return;
    }
    if id == "ins_mindmap_from_block" {
        insert_mindmap_from_block(ctx);
        return;
    }
    if let Some((_, view, _, _)) = CALENDAR_VIEWS.iter().find(|(mid, _, _, _)| *mid == id) {
        insert_calendar(ctx, *view);
        return;
    }
    if let Some(kind) = id.strip_prefix("ins_object_") {
        if OBJECTS.iter().any(|(k, _, _)| *k == kind) {
            insert_object(ctx, kind);
        }
        return;
    }
    if let Some(kind) = id.strip_prefix("ins_") {
        match kind {
            "image" => media::pick_and_insert(ctx, PickKind::Image),
            "svg" => media::pick_and_insert(ctx, PickKind::Svg),
            "svg_clip" => paste_svg(ctx),
            "file" => media::pick_and_insert(ctx, PickKind::File),
            _ => {
                if let Some(a) = action_of(kind) {
                    ctx.doc_op(DocOp::InsertBlock(a));
                }
            }
        }
        return;
    }
    if let Some(kind) = id.strip_prefix("turn_") {
        if let Some(a) = action_of(kind) {
            ctx.doc_op(DocOp::TurnInto(a));
        }
        return;
    }
    match id {
        "copy" => ctx.doc_op(DocOp::Copy),
        "cut" => ctx.doc_op(DocOp::Cut),
        "paste" => ctx.doc_op(DocOp::Paste),
        "dup" => ctx.doc_op(DocOp::Duplicate),
        "up" => ctx.doc_op(DocOp::Move { down: false }),
        "down" => ctx.doc_op(DocOp::Move { down: true }),
        "del" => ctx.doc_op(DocOp::Delete),
        _ => {}
    }
}

/// Само меню; живёт рядом с редактором в Stack без клипа.
pub fn popup(ctx: NotesCtx) -> impl Widget {
    PopupMenu::new()
        .items(items())
        .is_open(ctx.doc_menu_open)
        .position(ctx.doc_menu_pos)
        .anchor(PopupAnchor::Position)
        .min_width(210.0)
        .on_select(move |id| handle(ctx, id))
}
