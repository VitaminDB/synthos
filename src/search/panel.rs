//! Панель поиска: поле ввода, чипы групп, выдача и подвал с клавишами.
//!
//! Панель — `PopupPanel` поверх всего окна, поэтому она видна с любой
//! страницы. Содержимое пересобирается реактивно: `compute_view` считает
//! группы по текущему запросу, `build_results` разворачивает их в строки и
//! кладёт снимок в `SearchCtx.state` — оттуда его читает обработчик клавиш.

use syngui::core::Color;
use syngui::input::{CursorIcon, Key};
use syngui::prelude::*;
use syngui::widgets::containers::{GestureDetector, Reactive};
use syngui::widgets::overlay::{PopupAnchor, PopupPanel};

use crate::components::event_hook::{EventHook, KeyReply};
use crate::context::AppCtx;
use crate::icons::*;

use super::matching::{self, ItemMatch};
use super::model::{SearchItem, SearchKind};
use super::{
    actions, handle_panel_key, RowAction, SearchCtx, FILTERED_LIMIT, GROUP_LIMIT, PANEL_INSET,
    PANEL_MAX_HEIGHT, PANEL_WIDTH, RECENT_LIMIT, VISIBLE_LINES,
};

const TITLE_FONT_SIZE: f32 = 13.0;
const TITLE_LINE_HEIGHT: f32 = 1.25;

pub fn view() -> impl Widget {
    let search = use_context::<SearchCtx>();
    let content = search.clone();
    PopupPanel::new()
        .is_open(search.open)
        .anchor_rect(search.anchor)
        .anchor(PopupAnchor::Position)
        .min_width(PANEL_WIDTH)
        .max_width(PANEL_WIDTH)
        .max_height(PANEL_MAX_HEIGHT)
        .child(Reactive::new(move || {
            let open = content.open.get();
            let _nonce = content.nonce.get();
            let widget: Box<dyn Widget> = if open {
                Box::new(card(content.clone()))
            } else {
                Box::new(DecoratedBox::new())
            };
            vec![widget]
        }))
        .class("search-panel")
}

fn card(search: SearchCtx) -> impl Widget {
    let query = search.query;
    let selected = search.selected;
    let field = TextField::new()
        .text(query.get_untracked())
        .autofocus(true)
        .prefix_icon(MI_SEARCH)
        .placeholder(tr!("search.panel.placeholder"))
        .on_change(move |value: &str| {
            query.set(value.to_string());
            selected.set(0);
        })
        .class("search-input");
    let results = {
        let s = search.clone();
        move || build_results(&s)
    };
    let outer_keys = {
        let s = search.clone();
        move |key, mods| handle_panel_key(&s, key, mods)
    };
    let inner_keys = {
        let s = search.clone();
        move |key, mods| handle_panel_key(&s, key, mods)
    };
    EventHook::new().on_key_down(outer_keys).child(
        DecoratedBox::new().class("search-card").child(
            Padding::all(PANEL_INSET).child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(field)
                    .child(results)
                    .child(Divider::horizontal().class("search-divider"))
                    .child(footer())
                    // Невидимый хвост: при открытом оверлее дерево обходит
                    // детей с конца, поэтому стрелки и Enter достаются
                    // панели раньше, чем полю ввода.
                    .child(EventHook::new().on_key_down(inner_keys)),
            ),
        ),
    )
}

struct Hit {
    item: SearchItem,
    ranges: Vec<(usize, usize)>,
    matched: Option<String>,
}

impl Hit {
    fn plain(item: &SearchItem) -> Self {
        Self {
            item: item.clone(),
            ranges: Vec::new(),
            matched: None,
        }
    }

    fn scored(item: &SearchItem, m: ItemMatch) -> Self {
        Self {
            item: item.clone(),
            ranges: m.title_ranges,
            matched: m.matched,
        }
    }
}

struct Group {
    kind: SearchKind,
    hits: Vec<Hit>,
    total: usize,
}

struct View {
    groups: Vec<Group>,
    counts: Vec<(SearchKind, usize)>,
    recent: Vec<Hit>,
    switched: Option<String>,
    has_query: bool,
}

fn compute_view(
    items: &[SearchItem],
    query: &str,
    filter: Option<SearchKind>,
    recent_keys: &[String],
) -> View {
    let Some(plan) = matching::plan(query) else {
        return browse_view(items, filter, recent_keys);
    };
    let mut matched: Vec<(usize, ItemMatch)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| matching::score_item(item, &plan).map(|m| (i, m)))
        .collect();
    matched.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| items[a.0].kind.rank().cmp(&items[b.0].kind.rank()))
            .then_with(|| items[a.0].title.cmp(&items[b.0].title))
    });
    let switched = matched
        .first()
        .filter(|(_, m)| m.via_layout)
        .and_then(|_| plan.switched.as_ref().map(|(text, _)| text.clone()));
    let limit = if filter.is_some() {
        FILTERED_LIMIT
    } else {
        GROUP_LIMIT
    };
    let mut groups: Vec<Group> = Vec::new();
    for (i, m) in matched {
        let item = &items[i];
        let group = match groups.iter().position(|g| g.kind == item.kind) {
            Some(pos) => &mut groups[pos],
            None => {
                groups.push(Group {
                    kind: item.kind,
                    hits: Vec::new(),
                    total: 0,
                });
                groups.last_mut().expect("группа только что добавлена")
            }
        };
        group.total += 1;
        if group.hits.len() < limit {
            group.hits.push(Hit::scored(item, m));
        }
    }
    let counts = groups.iter().map(|g| (g.kind, g.total)).collect();
    if let Some(kind) = filter {
        groups.retain(|g| g.kind == kind);
    }
    View {
        groups,
        counts,
        recent: Vec::new(),
        switched,
        has_query: true,
    }
}

/// Пустой запрос: недавнее сверху, затем страницы и команды — то, ради чего
/// панель чаще всего и открывают вслепую.
fn browse_view(items: &[SearchItem], filter: Option<SearchKind>, recent_keys: &[String]) -> View {
    let mut counts: Vec<(SearchKind, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|c| c.0 == item.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((item.kind, 1)),
        }
    }
    counts.sort_by_key(|c| c.0.rank());
    let total_of = |kind: SearchKind| counts.iter().find(|c| c.0 == kind).map(|c| c.1).unwrap_or(0);
    let take = |kind: SearchKind, limit: usize| -> Vec<Hit> {
        items
            .iter()
            .filter(|i| i.kind == kind)
            .take(limit)
            .map(Hit::plain)
            .collect()
    };
    let mut groups = Vec::new();
    match filter {
        Some(kind) => groups.push(Group {
            kind,
            hits: take(kind, FILTERED_LIMIT),
            total: total_of(kind),
        }),
        None => {
            for kind in [SearchKind::Chat, SearchKind::Page, SearchKind::Command] {
                let total = total_of(kind);
                if total > 0 {
                    groups.push(Group {
                        kind,
                        hits: take(kind, GROUP_LIMIT),
                        total,
                    });
                }
            }
        }
    }
    let recent = if filter.is_none() {
        recent_keys
            .iter()
            .filter_map(|key| items.iter().find(|i| i.key == *key))
            .take(RECENT_LIMIT)
            .map(Hit::plain)
            .collect()
    } else {
        Vec::new()
    };
    View {
        groups,
        counts,
        recent,
        switched: None,
        has_query: false,
    }
}

enum Line<'a> {
    Header(String, usize),
    Hit(&'a Hit, usize),
    More(SearchKind, usize, usize),
}

fn build_results(search: &SearchCtx) -> Column {
    let app = use_context::<AppCtx>();
    let query = search.query.get();
    let filter = search.kind_filter.get();
    let requested = search.selected.get();
    let items = search.index.get();
    let recent_keys = search.recent.get();
    let scanning = search.scanning.get();
    let palette = Palette::current(&app);
    let view = compute_view(&items, &query, filter, &recent_keys);

    let mut rows: Vec<RowAction> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    if !view.recent.is_empty() {
        lines.push(Line::Header(tr!("search.recent"), view.recent.len()));
        for hit in &view.recent {
            lines.push(Line::Hit(hit, rows.len()));
            rows.push(RowAction::Item(hit.item.clone()));
        }
    }
    for group in &view.groups {
        lines.push(Line::Header(tr!(group.kind.label_key()), group.total));
        for hit in &group.hits {
            lines.push(Line::Hit(hit, rows.len()));
            rows.push(RowAction::Item(hit.item.clone()));
        }
        if group.total > group.hits.len() {
            lines.push(Line::More(group.kind, group.total, rows.len()));
            rows.push(RowAction::More(group.kind));
        }
    }
    let selected = requested.min(rows.len().saturating_sub(1));
    let kinds: Vec<SearchKind> = view.counts.iter().map(|c| c.0).collect();
    let total: usize = view.counts.iter().map(|c| c.1).sum();
    if let Ok(mut state) = search.state.lock() {
        state.rows = rows;
        state.kinds = kinds;
    }

    let mut column = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !view.counts.is_empty() {
        column = column.child(chips_row(search, &view.counts, total, filter));
    }
    if let Some(alt) = &view.switched {
        column = column.child(note(MI_TRANSLATE, format!("{}: «{alt}»", tr!("search.layout_note"))));
    }
    if scanning {
        column = column.child(note(MI_AUTORENEW, tr!("search.scanning")));
    }
    if lines.is_empty() {
        return column.child(empty_state(&query, view.has_query));
    }

    let lines_total = lines.len();
    let mut list = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch);
    for line in lines {
        list = match line {
            Line::Header(title, count) => list.child(group_header(&title, count)),
            Line::Hit(hit, row) => list.child(hit_row(search, hit, row == selected, &palette)),
            Line::More(kind, count, row) => list.child(more_row(search, kind, count, row == selected)),
        };
    }
    if lines_total > VISIBLE_LINES {
        column.child(
            DecoratedBox::new()
                .class("search-results")
                .child(ScrollView::new().vertical().child(list)),
        )
    } else {
        column.child(list)
    }
}

/// Цвета подсветки берутся из активной темы: MSS-переменные виджету
/// `RichText` недоступны, он красит спаны явными `Color`.
struct Palette {
    text: Color,
    primary: Color,
}

impl Palette {
    fn current(app: &AppCtx) -> Self {
        let theme = crate::active_theme(app.appearance, app.theme_key);
        Self {
            text: Color::from_hex(theme.text),
            primary: Color::from_hex(theme.primary),
        }
    }
}

fn chips_row(
    search: &SearchCtx,
    counts: &[(SearchKind, usize)],
    total: usize,
    filter: Option<SearchKind>,
) -> impl Widget {
    let mut flex = Flex::row()
        .wrap()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center);
    let all = search.clone();
    flex = flex.child(chip(&tr!("search.all"), total, filter.is_none(), move || {
        all.set_filter(None)
    }));
    for (kind, count) in counts {
        let s = search.clone();
        let k = *kind;
        flex = flex.child(chip(
            &tr!(kind.label_key()),
            *count,
            filter == Some(k),
            move || s.set_filter(Some(k)),
        ));
    }
    Padding::only(2.0, 2.0, 2.0, 0.0).child(flex)
}

fn chip(
    label: &str,
    count: usize,
    active: bool,
    on_click: impl FnMut() + Send + 'static,
) -> impl Widget {
    let (chip_class, label_class) = if active {
        (
            "search-chip search-chip-active",
            "search-chip-label search-chip-label-active",
        )
    } else {
        ("search-chip", "search-chip-label")
    };
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(on_click)
        .child(
            DecoratedBox::new().class(chip_class).child(
                Padding::symmetric(10.0, 3.0).child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(label).class(label_class))
                        .child(Text::new(count.to_string()).class("search-chip-count")),
                ),
            ),
        )
}

fn note(icon: &str, text: String) -> impl Widget {
    DecoratedBox::new().class("search-note").child(
        Padding::symmetric(10.0, 5.0).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class("search-note-icon"))
                .child(Text::new(text).max_lines(1).class("search-note-text")),
        ),
    )
}

fn group_header(title: &str, count: usize) -> impl Widget {
    Padding::only(10.0, 8.0, 10.0, 2.0).child(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(title.to_uppercase()).class("search-group-title"))
            .child(Text::new(count.to_string()).class("search-group-count")),
    )
}

fn empty_state(query: &str, has_query: bool) -> impl Widget {
    let title = if has_query {
        format!("{} «{}»", tr!("search.nothing"), query.trim())
    } else {
        tr!("search.empty_index")
    };
    Padding::all(28.0).child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_SEARCH_OFF).class("search-empty-icon"))
            .child(Text::new(title).class("search-empty-title"))
            .child(Text::new(tr!("search.nothing_hint")).class("search-empty-hint")),
    )
}

fn is_nav_key(key: Key) -> bool {
    matches!(
        key,
        Key::Up
            | Key::Down
            | Key::PageUp
            | Key::PageDown
            | Key::Home
            | Key::End
            | Key::Left
            | Key::Right
    )
}

fn icon_box(item: &SearchItem) -> impl Widget {
    let icon = if item.icon.is_empty() {
        MI_CIRCLE
    } else {
        item.icon
    };
    DecoratedBox::new()
        .class(item.kind.icon_box_class())
        .child(Center::new().child(Icon::new(icon).class(item.kind.icon_class())))
}

fn kbd_enter(visible: bool) -> impl Widget {
    let class = if visible {
        "search-kbd search-kbd-iconbox"
    } else {
        "search-kbd search-kbd-iconbox search-kbd-hidden"
    };
    DecoratedBox::new()
        .class(class)
        .child(Icon::new(MI_KEYBOARD_RETURN).class("search-kbd-icon"))
}

fn title_widget(title: &str, ranges: &[(usize, usize)], palette: &Palette) -> RichText {
    let mut rich = RichText::new()
        .wrap(false)
        .default_color(palette.text)
        .default_font_size(TITLE_FONT_SIZE)
        .line_height(TITLE_LINE_HEIGHT);
    for (segment, highlighted) in matching::split_highlight(title, ranges) {
        if highlighted {
            let primary = palette.primary;
            rich = rich.span(segment, move |span| span.color(primary).bold());
        } else {
            rich = rich.text(segment);
        }
    }
    rich
}

fn hit_row(search: &SearchCtx, hit: &Hit, selected: bool, palette: &Palette) -> impl Widget {
    let item = &hit.item;
    let row_class = if selected {
        "search-row search-row-selected"
    } else {
        "search-row"
    };
    // Если совпало по скрытому полю (ключевое слово, id, название чата) —
    // показываем его в подписи: иначе непонятно, почему строка нашлась.
    let subtitle = match &hit.matched {
        Some(m) if !item.subtitle.contains(m.as_str()) && !item.hint.contains(m.as_str()) => {
            if item.subtitle.is_empty() {
                m.clone()
            } else {
                format!("{m} · {}", item.subtitle)
            }
        }
        _ => item.subtitle.clone(),
    };
    let mut text_column = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(title_widget(&item.title, &hit.ranges, palette));
    if !subtitle.is_empty() {
        text_column = text_column.child(Text::new(subtitle).max_lines(1).class("search-row-sub"));
    }
    let mut row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_box(item))
        .child(
            DecoratedBox::new()
                .class("grow")
                .clip(true)
                .child(text_column),
        );
    if !item.hint.is_empty() {
        row = row.child(Text::new(&item.hint).max_lines(1).class("search-row-hint"));
    }
    row = row.child(kbd_enter(selected));

    let s = search.clone();
    let it = item.clone();
    EventHook::new()
        .on_key_up(move |key, _| {
            if selected && is_nav_key(key) {
                KeyReply::ScrollIntoView
            } else {
                KeyReply::Ignore
            }
        })
        .child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || actions::run(&s, &it, false))
                .child(
                    DecoratedBox::new()
                        .class(row_class)
                        .child(Padding::only(8.0, 6.0, 8.0, 6.0).child(row)),
                ),
        )
}

fn more_row(search: &SearchCtx, kind: SearchKind, total: usize, selected: bool) -> impl Widget {
    let row_class = if selected {
        "search-row search-row-more search-row-selected"
    } else {
        "search-row search-row-more"
    };
    let label = format!(
        "{} · {}",
        tr!("search.show_all"),
        tr!(kind.label_key()).to_lowercase()
    );
    let s = search.clone();
    EventHook::new()
        .on_key_up(move |key, _| {
            if selected && is_nav_key(key) {
                KeyReply::ScrollIntoView
            } else {
                KeyReply::Ignore
            }
        })
        .child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || s.set_filter(Some(kind)))
                .child(
                    DecoratedBox::new().class(row_class).child(
                        Padding::only(8.0, 5.0, 8.0, 5.0).child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(
                                    DecoratedBox::new()
                                        .class("search-icon-box search-icon-box-more")
                                        .child(
                                            Center::new().child(
                                                Icon::new(MI_EXPAND_MORE)
                                                    .class("search-row-icon search-row-icon-more"),
                                            ),
                                        ),
                                )
                                .child(Text::new(label).class("search-row-more-text"))
                                .child(Text::new(total.to_string()).class("search-row-more-count"))
                                .child(DecoratedBox::new().class("grow"))
                                .child(kbd_enter(selected)),
                        ),
                    ),
                ),
        )
}

fn footer() -> impl Widget {
    DecoratedBox::new().class("search-footer").child(
        Padding::only(6.0, 4.0, 6.0, 0.0).child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(hint(
                    vec![kbd_icon(MI_ARROW_UPWARD), kbd_icon(MI_ARROW_DOWNWARD)],
                    tr!("search.kbd.navigate"),
                ))
                .child(hint(vec![kbd_icon(MI_KEYBOARD_RETURN)], tr!("search.kbd.open")))
                .child(hint(
                    vec![kbd_text("Shift"), kbd_icon(MI_KEYBOARD_RETURN)],
                    tr!("search.kbd.secondary"),
                ))
                .child(hint(
                    vec![
                        kbd_text("Ctrl"),
                        kbd_icon(MI_CHEVRON_LEFT),
                        kbd_icon(MI_CHEVRON_RIGHT),
                    ],
                    tr!("search.kbd.filter"),
                ))
                .child(DecoratedBox::new().class("grow"))
                .child(hint(vec![kbd_text("Esc")], tr!("search.kbd.close"))),
        ),
    )
}

fn hint(keys: Vec<Box<dyn Widget>>, label: String) -> impl Widget {
    Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(keys)
        .child(Text::new(label).class("search-footer-text"))
}

fn kbd_icon(icon: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class("search-kbd search-kbd-iconbox")
            .child(Icon::new(icon).class("search-kbd-icon")),
    )
}

fn kbd_text(text: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class("search-kbd")
            .child(Text::new(text).class("search-kbd-text")),
    )
}
