//! Нав-рейл — узкая колонка слева от контента.
//!
//! ```text
//! ┌────┐
//! │ S  │  логотип
//! │────│
//! │ ▤  │  плитки рабочих пространств (`crate::rail::entries`):
//! │ ⬡  │  code-сессия / граф нод / чат / разделитель — в порядке создания,
//! │ AB │  скроллятся, если не влезают
//! │ ─  │
//! │ +  │  меню: Редактор кода · Нодовый редактор · Чат · Разделитель
//! │    │
//! │ ▣  │  Syn-пакеты
//! │ ☁  │  HuggingFace (+ бейдж активных загрузок)
//! │────│
//! │ 文 │  язык интерфейса
//! │────│
//! │ ⚙  │  Настройки
//! └────┘
//! ```
//!
//! Верхних иконок-разделов «Чат» и «Ноды» больше нет: к чату и графу
//! ведут их плитки, а новые создаются через «+». Клик по плитке —
//! `rail::open`, «Закрыть» в контекстном меню — `rail::request_close`.
//! Плитки (и разделители) перетаскиваются: `Draggable` с ключом плитки в
//! payload, `DropArea` на каждой плитке (`rail::move_before`) и на «+»
//! (`rail::move_to_end`) — так плитки группируются разделителями.
//!
//! Группа — одна плитка со своей иконкой. Клик выдвигает справа от неё
//! панель с плитками группы (`group_flyout`): плитка становится вкладкой,
//! которая перетекает в панель (`flow-edge` вкладки + `PopupPanel::reveal`
//! с якорем `EndCenter`). Сброс плитки на группу кладёт её в группу.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::{Positioned, Stack};
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
use syngui::widgets::overlay::{Draggable, DropArea};
use syngui::widgets::visual::{Badge, Image, ImageFit};

use syngui::widgets::containers::AnimationAxis;
use syngui::widgets::overlay::menu::PopupAnchor;
use syngui::widgets::overlay::{Portal, PortalAnchor, PopupPanel};

use crate::components::chat_item::{
    display_title, initials_from_title, is_generating, tone_for, typing_dot,
};
use crate::config::RailGroupConfig;
use crate::context::{AppCtx, RailGroupEdit};
use crate::icons::*;
use crate::pages::code_editor::state::CodeSession;
use crate::pages::huggingface::state::DlStatus;
use crate::pages::huggingface::HuggingFaceCtx;
use crate::pages::node_editor::tabs::OpenTab;
use crate::rail::{self, RailEntry};

#[derive(Clone, Copy)]
struct Item {
    icon: &'static str,
    route: &'static str,
    tooltip_key: &'static str,
}

const FOOTER_UTILITY: &[Item] = &[
    Item { icon: MI_INVENTORY_2,    route: "syn_explorer",  tooltip_key: "nav.syn_explorer" },
    Item { icon: MI_CLOUD_DOWNLOAD, route: "huggingface",   tooltip_key: "nav.huggingface" },
];

const FOOTER_SETTINGS: &[Item] = &[
    Item { icon: MI_SETTINGS,       route: "settings",      tooltip_key: "nav.settings" },
];

/// `drag_type` перетаскивания плиток — свой, чтобы DropArea рейла не
/// реагировала на drop'ы из других мест (вкладки открытых файлов).
const DRAG_TYPE_TILE: &str = "nav-rail-tile";

pub fn view() -> impl Widget {
    DecoratedBox::new().class("nav-rail").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                top_cluster(),
                workspaces_segment(),
                bottom_cluster(),
            ]
    })
}

fn top_cluster() -> impl Widget {
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                logo(),
                DecoratedBox::new().class("nav-rail-divider"),
            ]
    }
}

/// Футер снизу вверх: настройки — разделитель — язык — разделитель —
/// утилиты (Syn-пакеты, HuggingFace). Аватара пользователя нет.
fn bottom_cluster() -> impl Widget {
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("nav-rail-footer") => [
                DecoratedBox::new().class("nav-rail-divider"),
                rail_column(FOOTER_UTILITY),
                DecoratedBox::new().class("nav-rail-divider"),
                language_button(),
                DecoratedBox::new().class("nav-rail-divider"),
                rail_column(FOOTER_SETTINGS),
            ]
    }
}

fn language_button() -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let current = use_context::<AppCtx>().general.language.get_untracked();
    let btn = ToolButton::new(MI_TRANSLATE)
        .tooltip(tr!("nav.language"))
        .on_click_with_bounds(move |_, bounds| {
            pos.set(Point::new(bounds.origin.x + bounds.size.width + 8.0, bounds.origin.y));
            open.set(true);
        })
        .class("nav-rail-item");
    let menu = PopupMenu::new()
        .items(crate::i18n::language_menu_items(&current))
        .is_open(open)
        .position(pos)
        .on_select(move |id| {
            let ctx = use_context::<AppCtx>();
            ctx.general.language.set(id.to_string());
        });
    Stack::new().clip(false).child(btn).child(menu)
}

fn logo() -> impl Widget {
    const LOGO_SVG: &[u8] = include_bytes!("../../packaging/synthos.svg");
    DecoratedBox::new()
        .child(
            Image::from_bytes("nav-rail-logo-synthos", LOGO_SVG.to_vec())
                .fit(ImageFit::Contain),
        )
        .class("nav-rail-logo")
}

fn rail_column(items: &'static [Item]) -> impl Widget {
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center);
    for it in items {
        let item = *it;
        if item.route == "huggingface" {
            col = col.child(move || hf_rail_item(item));
        } else {
            col = col.child(move || rail_item(item));
        }
    }
    col
}

/// Пункт нав-рейла HuggingFace со счётчиком-бейджем активных + ожидающих
/// загрузок. Бейдж показывается только при count > 0; реактивен на `downloads`
/// (через `.get()`), поэтому счётчик обновляется на лету.
fn hf_rail_item(it: Item) -> impl Widget {
    let app = use_context::<AppCtx>();
    let is_selected = app.current_route.get() == it.route;
    let class = if is_selected { "nav-rail-item selected" } else { "nav-rail-item" };

    let route = it.route;
    let btn = ToolButton::new(it.icon)
        .tooltip(tr!(it.tooltip_key))
        .on_click(move || rail::navigate(route))
        .class(class);

    let hf = use_context::<HuggingFaceCtx>();
    let count = hf
        .downloads
        .get()
        .values()
        .filter(|d| matches!(d.status, DlStatus::Active | DlStatus::Pending))
        .count();

    let mut stack = Stack::new().child(btn);
    if count > 0 {
        stack = stack.child(Positioned::new(Badge::new(count.to_string()).small()).at(22.0, -2.0));
    }
    stack
}

fn rail_item(it: Item) -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let is_selected = ctx.current_route.get() == it.route;
    let class = if is_selected { "nav-rail-item selected" } else { "nav-rail-item" };

    let route = it.route;
    ToolButton::new(it.icon)
        .tooltip(tr!(it.tooltip_key))
        .on_click(move || rail::navigate(route))
        .class(class)
}

// ─────────────────────── Плитки рабочих пространств ───────────────────────

/// Сегмент между логотипом и футером: плитки + «+». Занимает всю свободную
/// высоту и скроллится, когда плиток больше, чем влезает.
fn workspaces_segment() -> impl Widget {
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let entries = rail::entries();
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("nav-rail-sessions");
        let mut code_idx = 0usize;
        for entry in entries.iter() {
            let tile: Box<dyn Widget> = match entry {
                RailEntry::Separator(ts) => Box::new(separator_tile(*ts, entry.clone())),
                RailEntry::Group { group, members } => Box::new(group_tile(group, members, entry.clone())),
                _ => entry_tile(entry, &mut code_idx, false),
            };
            col = col.child(Stack::new().children(vec![tile]));
        }
        col = col.child(add_button());
        vec![Box::new(col)]
    });
    DecoratedBox::new()
        .class("grow nav-rail-scroll-host")
        .child(ScrollView::new().vertical().class("nav-rail-scroll").child(list))
}

/// Плитка обычной записи (сессия, граф, чат, заметки) — в рейле или в
/// панели группы (`in_flyout`).
fn entry_tile(entry: &RailEntry, code_idx: &mut usize, in_flyout: bool) -> Box<dyn Widget> {
    let active = rail::is_active(entry);
    let t = TileOpts { selected: active, in_flyout };
    match entry {
        RailEntry::Code(s) => {
            *code_idx += 1;
            Box::new(code_tile(*s, *code_idx, t, entry.clone()))
        }
        RailEntry::Graph(tab) => Box::new(graph_tile(*tab, t, entry.clone())),
        RailEntry::Chat(m) => Box::new(chat_tile(m.id.clone(), m.title.clone(), t, entry.clone())),
        RailEntry::Notes { path, .. } => Box::new(note_tile(path, t, entry.clone())),
        RailEntry::Separator(_) | RailEntry::Group { .. } => Box::new(DecoratedBox::new()),
    }
}

/// Вид плитки: выбрана ли и лежит ли в панели группы. В панели свои
/// классы `in-flyout`: подложка панели того же тона, что наведение в
/// рейле, и наведение там другое.
#[derive(Clone, Copy)]
struct TileOpts {
    selected: bool,
    in_flyout: bool,
}

impl TileOpts {
    /// `base` + `selected` / `in-flyout` по состоянию.
    fn class(self, base: &str) -> String {
        let mut c = base.to_string();
        if self.selected {
            c.push_str(" selected");
        }
        if self.in_flyout {
            c.push_str(" in-flyout");
        }
        c
    }
}

/// Общая обёртка плитки: визуал + подпись, перетаскивание (клик —
/// `rail::open`, drop сверху — `rail::move_before`) и контекстное меню
/// «Закрыть». События идут от самого глубокого элемента к корню, поэтому
/// кнопка внутри плитки обязана быть `press_passthrough` — иначе она
/// заберёт MouseDown себе, и `Draggable` не увидит ни клика, ни старта
/// перетаскивания (так рейл и «не кликался»). Правая кнопка проходит к
/// `ContextMenu`.
fn tile_with_label(
    body: impl Widget + 'static,
    label: String,
    t: TileOpts,
    entry: RailEntry,
) -> impl Widget {
    let label_class = t.class("nav-rail-session-label");
    let tile = mgui! {
        Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                body,
                Text::new(label.clone()).max_lines(1).class(label_class),
            ]
    };
    draggable_tile(tile, label, entry)
}

/// Пункты «В группу ▸» для плитки верхнего уровня, «Убрать из группы» —
/// для плитки группы.
fn group_menu_items(entry: &RailEntry) -> Vec<MenuItem> {
    if !entry.groupable() {
        return Vec::new();
    }
    if rail::group_of(&entry.key()).is_some() {
        return vec![MenuItem::new("leave_group", tr!("nav.group.leave")).icon(MI_UNARCHIVE)];
    }
    let mut sub: Vec<MenuItem> = use_context::<AppCtx>()
        .rail_groups
        .get_untracked()
        .iter()
        .map(|g| MenuItem::new(format!("to_group:{}", g.id), g.name.clone()).icon(rail::group_icon(&g.icon)))
        .collect();
    if !sub.is_empty() {
        sub.push(MenuItem::separator());
    }
    sub.push(MenuItem::new("new_group", tr!("nav.group.new")).icon(MI_ADD));
    vec![MenuItem::new("to_group", tr!("nav.group.move_to")).icon(GROUP_DEFAULT_ICON).children(sub)]
}

/// Значок группы по умолчанию (Material «workspaces»).
const GROUP_DEFAULT_ICON: &str = "\u{EA0F}";

/// Draggable → DropArea → контент; снаружи — контекстное меню. `label` —
/// подпись призрака на случай, если фреймворк не сможет нарисовать живой
/// снимок плитки.
fn draggable_tile(tile: impl Widget + 'static, label: String, entry: RailEntry) -> impl Widget {
    let key = entry.key();
    let drop_key = key.clone();
    let entry_click = entry.clone();
    let entry_close = entry.clone();
    let dnd = Draggable::new(DRAG_TYPE_TILE, key)
        .label(label)
        .on_click(move || {
            rail::close_flyout();
            rail::open(&entry_click);
        })
        .child(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_TILE.to_string()])
                .on_drop(move |data| rail::move_before(&data.payload, &drop_key))
                .child(tile),
        );
    // Проект заметок — это файл: плитку можно переименовать.
    let mut items = Vec::new();
    if matches!(entry, RailEntry::Notes { .. }) {
        items.push(MenuItem::new("rename", tr!("notes.project.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE));
    }
    items.extend(group_menu_items(&entry));
    items.push(MenuItem::new("close", tr!("app.close")).icon(MI_CLOSE));
    ContextMenu::new()
        .items(items)
        .on_select(move |action| match (action, &entry_close) {
            ("close", _) => rail::request_close(&entry_close),
            ("rename", RailEntry::Notes { path, .. }) => crate::pages::notes::project_ui::request_rename(path),
            ("leave_group", e) => rail::remove_from_group(&e.key()),
            ("new_group", e) => rail::edit_group(RailGroupEdit { id: None, seed: Some(e.key()) }),
            (a, e) => {
                if let Some(id) = a.strip_prefix("to_group:").and_then(|s| s.parse().ok()) {
                    rail::add_to_group(&e.key(), id);
                }
            }
        })
        .child(dnd)
}

fn code_tile(session: CodeSession, idx: usize, t: TileOpts, entry: RailEntry) -> impl Widget {
    let folder = session.root_folder.get_untracked();
    let icon = if folder.is_some() { MI_FOLDER } else { MI_DESCRIPTION };
    let label = folder
        .as_ref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| tr!("nav.session.unnamed", n = idx));
    let btn = ToolButton::new(icon)
        .tooltip(label.clone())
        .press_passthrough()
        .class(t.class("nav-rail-item"));

    // Бейдж количества открытых терминалов сессии — по образцу hf_rail_item.
    // Цвет по занятости (busy_count пишет семплер terminal_activity;
    // занят = вывод обновлялся в последние секунды): все заняты — зелёный,
    // все простаивают — красный, смешанно — оранжевый.
    // Реактивность — через .get() в scope Reactive'а workspaces_segment.
    let term_count = session.terminals.tabs.get().len();
    let busy_count = session.terminals.busy_count.get();
    let mut btn_stack = Stack::new().child(btn);
    if term_count > 0 {
        let tone = if busy_count == 0 {
            "idle"
        } else if busy_count >= term_count {
            "busy"
        } else {
            "mixed"
        };
        btn_stack = btn_stack.child(
            Positioned::new(
                Badge::new(term_count.to_string())
                    .small()
                    .class(format!("nav-rail-term-badge {tone}")),
            )
            .at(24.0, -2.0),
        );
    }
    tile_with_label(btn_stack, label, t, entry)
}

fn graph_tile(tab: OpenTab, t: TileOpts, entry: RailEntry) -> impl Widget {
    // Иконка по происхождению графа: агентский (раскрыт из чата),
    // из шаблона или Untitled.
    let icon = if tab.agent_chat.get_untracked().is_some() {
        MI_PSYCHOLOGY
    } else if tab.source.get_untracked().is_some() {
        MI_HUB
    } else {
        MI_TUNE
    };
    let title = tab.title.get_untracked();
    let label = if tab.dirty.get_untracked() {
        format!("• {title}")
    } else {
        title.clone()
    };
    let btn = ToolButton::new(icon)
        .tooltip(title)
        .press_passthrough()
        .class(t.class("nav-rail-item"));
    tile_with_label(btn, label, t, entry)
}

fn chat_tile(id: String, title: String, t: TileOpts, entry: RailEntry) -> impl Widget {
    let shown = display_title(&title);
    let avatar = Avatar::new()
        .text(initials_from_title(&title))
        .size(32.0)
        .class(tone_for(&id));
    let body = DecoratedBox::new()
        .class(t.class("nav-rail-chat-tile"))
        .child(Center::new().child(avatar));
    // Модель печатает ответ в этом чате — пульсирующая точка в углу: ход
    // виден, даже когда чат в фоне или свёрнут в окно. `.get()` внутри
    // `is_generating` подписывает Reactive рейла на старт и конец хода.
    let generating = is_generating(&id);
    let mut stack = Stack::new().child(body);
    if generating {
        stack = stack.child(Positioned::new(typing_dot()).at(28.0, 0.0));
    }
    let tooltip = if generating {
        tr!("chat.typing.tooltip", name = shown.clone())
    } else {
        shown.clone()
    };
    tile_with_label(Tooltip::new(stack, tooltip), shown, t, entry)
}

/// Плитка проекта заметок: иконка режима в скруглённой рамке, подпись —
/// имя файла, подсказка — полный путь (два проекта с одним именем в разных
/// папках различимы).
fn note_tile(path: &std::path::Path, t: TileOpts, entry: RailEntry) -> impl Widget {
    let title = crate::pages::notes::project::project_title(path);
    let hint = path.display().to_string();
    let icon = MI_EDIT_NOTE;
    let body = DecoratedBox::new()
        .class(t.class("nav-rail-item nav-rail-note-tile"))
        .child(Center::new().child(Icon::new(icon).class("nav-rail-note-icon")));
    tile_with_label(Tooltip::new(body, hint), title, t, entry)
}

/// Разделитель: тонкая линия в широкой невидимой зоне — чтобы по ней можно
/// было попасть правой кнопкой (удалить) и ухватить для перетаскивания.
fn separator_tile(ts: u64, entry: RailEntry) -> impl Widget {
    let line = DecoratedBox::new()
        .class("nav-rail-separator-hit")
        .child(Center::new().child(DecoratedBox::new().class("nav-rail-separator")));
    let key = entry.key();
    let drop_key = key.clone();
    let dnd = Draggable::new(DRAG_TYPE_TILE, key)
        .label(tr!("nav.add.separator"))
        .child(
        DropArea::new()
            .accept_types(vec![DRAG_TYPE_TILE.to_string()])
            .on_drop(move |data| rail::move_before(&data.payload, &drop_key))
            .child(line),
    );
    ContextMenu::new()
        .items(vec![MenuItem::new("remove", tr!("app.delete")).icon(MI_CLOSE)])
        .on_select(move |action| {
            if action == "remove" {
                rail::remove_separator(ts);
            }
        })
        .child(dnd)
}

/// «+» — меню выбора, что создать. Сброс плитки на «+» ставит её в конец.
fn add_button() -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let btn = ToolButton::new(MI_ADD)
        .tooltip(tr!("nav.add.tooltip"))
        .on_click_with_bounds(move |_, bounds| {
            pos.set(Point::new(bounds.origin.x + bounds.size.width + 8.0, bounds.origin.y));
            open.set(true);
        })
        .class("nav-rail-item nav-rail-item-add");
    let drop = DropArea::new()
        .accept_types(vec![DRAG_TYPE_TILE.to_string()])
        .on_drop(|data| rail::move_to_end(&data.payload))
        .child(btn);
    let menu = PopupMenu::new()
        .items(vec![
            MenuItem::new("code", tr!("nav.add.code")).icon(MI_CODE),
            MenuItem::new("nodes", tr!("nav.add.nodes")).icon(MI_HUB),
            MenuItem::new("chat", tr!("nav.add.chat")).icon(MI_CHAT),
            MenuItem::new("note", tr!("nav.add.note"))
                .icon(MI_EDIT_NOTE)
                .children(crate::pages::notes::project_ui::add_menu_items()),
            MenuItem::separator(),
            MenuItem::new("group", tr!("nav.add.group")).icon(GROUP_DEFAULT_ICON),
            MenuItem::new("separator", tr!("nav.add.separator")).icon(MI_HORIZONTAL_RULE),
        ])
        .is_open(open)
        .position(pos)
        .on_select(|id| match id {
            "code" => rail::new_code_session(),
            "nodes" => rail::new_graph(),
            "chat" => rail::new_chat(),
            id if crate::pages::notes::project_ui::handle_menu(id) => {}
            "separator" => rail::add_separator(),
            "group" => rail::edit_group(RailGroupEdit { id: None, seed: None }),
            _ => {}
        });
    Stack::new().clip(false).child(drop).child(menu)
}

// ─────────────────────────────── Группы ───────────────────────────────

/// Плитка группы. Во всю ширину рейла лежит «вкладка»: пока панель группы
/// закрыта, она прозрачна; открыта — окрашена в цвет панели и перетекает
/// в неё (`flow-edge: right` дорисовывает вогнутые ушки у края рейла).
/// Клик выдвигает панель, сброс плитки — кладёт её в группу.
fn group_tile(group: &RailGroupConfig, members: &[RailEntry], entry: RailEntry) -> impl Widget {
    let fly = use_context::<AppCtx>().rail_flyout;
    let id = group.id;
    let open = fly.open.get() && fly.group.get() == Some(id);
    let active = members.iter().any(rail::is_active);
    let t = TileOpts { selected: active && !open, in_flyout: false };
    let btn = ToolButton::new(rail::group_icon(&group.icon))
        .tooltip(group.name.clone())
        .press_passthrough()
        .class(t.class("nav-rail-item nav-rail-group-btn"));
    // Число плиток — маленькая метка в углу: группу видно среди плиток.
    let mut body = Stack::new().child(btn);
    if !members.is_empty() {
        body = body.child(
            Positioned::new(Badge::new(members.len().to_string()).small().class("nav-rail-group-count")).at(24.0, -2.0),
        );
    }
    // Чат группы печатает — точка и на плитке группы.
    if members.iter().any(|m| matches!(m, RailEntry::Chat(c) if is_generating(&c.id))) {
        body = body.child(Positioned::new(typing_dot()).at(28.0, 28.0));
    }
    let label_class = if open { "nav-rail-session-label open".to_string() } else { t.class("nav-rail-session-label") };
    let tile = mgui! {
        Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                body,
                Text::new(group.name.clone()).max_lines(1).class(label_class),
            ]
    };
    let tab_class = if open { "nav-rail-group-tab open" } else { "nav-rail-group-tab" };
    let slot = DecoratedBox::new()
        .class("nav-rail-group-slot")
        .child(DecoratedBox::new().class(tab_class).child(Center::new().child(tile)));

    let key = entry.key();
    let dnd = Draggable::new(DRAG_TYPE_TILE, key)
        .label(group.name.clone())
        .on_click_with_bounds(move |r| rail::toggle_flyout(id, r))
        .child(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_TILE.to_string()])
                .on_drop(move |data| rail::add_to_group(&data.payload, id))
                .child(slot),
        );
    ContextMenu::new()
        .items(vec![
            MenuItem::new("edit", tr!("nav.group.edit")).icon(MI_EDIT),
            MenuItem::new("ungroup", tr!("nav.group.ungroup")).icon(MI_UNARCHIVE),
        ])
        .on_select(move |action| match action {
            "edit" => rail::edit_group(RailGroupEdit { id: Some(id), seed: None }),
            "ungroup" => rail::ungroup(id),
            _ => {}
        })
        .child(dnd)
}

/// Ширина панели группы: рейл (72) + 8.
const FLYOUT_WIDTH: f32 = 80.0;

/// Выезжающая панель открытой группы: справа от её плитки, по центру её
/// высоты; уезжает обратно по клику мимо, Escape или выбору плитки.
/// Смонтирована в корне приложения — поверх страниц.
pub fn group_flyout() -> impl Widget {
    let fly = use_context::<AppCtx>().rail_flyout;
    let content = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(id) = fly.group.get() else {
            return vec![Box::new(DecoratedBox::new())];
        };
        let Some(RailEntry::Group { group, members }) =
            rail::entries().into_iter().find(|e| matches!(e, RailEntry::Group { group, .. } if group.id == id))
        else {
            return vec![Box::new(DecoratedBox::new())];
        };
        let mut code_idx = 0usize;
        // Столбиком, как сам рейл: плитки с подписями друг под другом.
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for m in &members {
            col = col.child(entry_tile(m, &mut code_idx, true));
        }
        let body: Box<dyn Widget> = if members.is_empty() {
            Box::new(Text::new(tr!("nav.group.empty")).class("nav-rail-flyout-empty"))
        } else {
            Box::new(col)
        };
        // Сброс в свободное место панели — в конец группы.
        let drop = DropArea::new()
            .accept_types(vec![DRAG_TYPE_TILE.to_string()])
            .on_drop(move |data| rail::add_to_group(&data.payload, id))
            .child(body);
        vec![Box::new(mgui! {
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new(group.name.clone()).max_lines(1).class("nav-rail-flyout-title"),
                    drop,
                ]
        })]
    });
    PopupPanel::new()
        .is_open(fly.open)
        .anchor_rect(fly.anchor)
        .anchor(PopupAnchor::EndCenter)
        // Колонка шириной с рейл и чуть шире (72 + 8): подписи плиток
        // обрезаются многоточием, как в самом рейле. Без жёсткой ширины
        // Text подписи брал всю ширину попапа и уезжал за его обрезку.
        .min_width(FLYOUT_WIDTH)
        .max_width(FLYOUT_WIDTH)
        .max_height(640.0)
        .reveal(AnimationAxis::Width)
        .class("nav-rail-flyout")
        .child(content)
}

/// Диалог группы: название и иконка. Новая группа (с плиткой, из меню
/// которой её создали) или правка существующей.
pub fn group_dialog() -> impl Widget {
    let edit = use_context::<AppCtx>().rail_flyout.edit;
    let is_open = use_signal(false);
    create_effect(move || {
        let has = edit.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });
    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(move || edit.set(None))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            match edit.get() {
                Some(e) => vec![Box::new(group_card(e))],
                None => vec![Box::new(DecoratedBox::new().class("code-editor-dialog-empty"))],
            }
        }))
}

fn group_card(e: RailGroupEdit) -> impl Widget {
    let app = use_context::<AppCtx>();
    let existing = e.id.and_then(|id| app.rail_groups.get_untracked().into_iter().find(|g| g.id == id));
    let initial_name = existing.as_ref().map(|g| g.name.clone()).unwrap_or_default();
    let initial_icon = existing
        .as_ref()
        .map(|g| g.icon.clone())
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| rail::GROUP_ICONS[0].0.to_string());
    let name = use_signal(initial_name.clone());
    let icon = use_signal(initial_icon);
    let title = if e.id.is_some() { tr!("nav.group.edit_title") } else { tr!("nav.group.new_title") };
    let submit = {
        let e = e.clone();
        move || {
            let n = name.get_untracked().trim().to_string();
            let n = if n.is_empty() { tr!("nav.group.default_name") } else { n };
            let i = icon.get_untracked();
            match e.id {
                Some(id) => rail::update_group(id, &n, &i),
                None => {
                    rail::create_group(&n, &i, e.seed.as_deref());
                }
            }
            use_context::<AppCtx>().rail_flyout.edit.set(None);
        }
    };
    let submit_enter = submit.clone();
    let icons = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = icon.get();
        let mut grid = Flex::row().wrap().gap(6.0).class("nav-group-icon-grid");
        for (n, glyph) in rail::GROUP_ICONS {
            let class = if *n == cur { "nav-group-icon selected" } else { "nav-group-icon" };
            grid = grid.child(ToolButton::new(*glyph).on_click(move || icon.set(n.to_string())).class(class));
        }
        vec![Box::new(grid)]
    });
    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(title).class("code-editor-dialog-title"),
                    TextField::new()
                        .text(initial_name)
                        .placeholder(tr!("nav.group.name_placeholder"))
                        .autofocus(true)
                        .on_change(move |s| name.set(s.to_string()))
                        .on_submit(move |_| submit_enter())
                        .class("code-editor-dialog-input"),
                    Text::new(tr!("nav.group.icon")).class("code-editor-dialog-hint"),
                    icons,
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(move || use_context::<AppCtx>().rail_flyout.edit.set(None))
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(move || submit())
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}
