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

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::{Positioned, Stack};
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
use syngui::widgets::overlay::{Draggable, DropArea};
use syngui::widgets::visual::{Badge, Image, ImageFit};

use crate::components::chat_item::{display_title, initials_from_title, tone_for};
use crate::context::AppCtx;
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
            let active = rail::is_active(entry);
            let tile: Box<dyn Widget> = match entry {
                RailEntry::Code(s) => {
                    code_idx += 1;
                    Box::new(code_tile(*s, code_idx, active, entry.clone()))
                }
                RailEntry::Graph(t) => Box::new(graph_tile(*t, active, entry.clone())),
                RailEntry::Chat(m) => {
                    Box::new(chat_tile(m.id.clone(), m.title.clone(), active, entry.clone()))
                }
                RailEntry::Separator(ts) => Box::new(separator_tile(*ts, entry.clone())),
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

/// Общая обёртка плитки: визуал + подпись, перетаскивание (клик —
/// `rail::open`, drop сверху — `rail::move_before`) и контекстное меню
/// «Закрыть». `Draggable` глотает MouseDown левой кнопки, поэтому у
/// внутренних кнопок своих on_click нет; правая кнопка проходит к
/// `ContextMenu`.
fn tile_with_label(
    body: impl Widget + 'static,
    label: String,
    is_selected: bool,
    entry: RailEntry,
) -> impl Widget {
    let label_class = if is_selected {
        "nav-rail-session-label selected"
    } else {
        "nav-rail-session-label"
    };
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
        .on_click(move || rail::open(&entry_click))
        .child(
            DropArea::new()
                .accept_types(vec![DRAG_TYPE_TILE.to_string()])
                .on_drop(move |data| rail::move_before(&data.payload, &drop_key))
                .child(tile),
        );
    ContextMenu::new()
        .items(vec![MenuItem::new("close", tr!("app.close")).icon(MI_CLOSE)])
        .on_select(move |action| {
            if action == "close" {
                rail::request_close(&entry_close);
            }
        })
        .child(dnd)
}

fn code_tile(session: CodeSession, idx: usize, is_selected: bool, entry: RailEntry) -> impl Widget {
    let folder = session.root_folder.get_untracked();
    let icon = if folder.is_some() { MI_FOLDER } else { MI_DESCRIPTION };
    let label = folder
        .as_ref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| tr!("nav.session.unnamed", n = idx));
    let btn_class = if is_selected {
        "nav-rail-item selected"
    } else {
        "nav-rail-item"
    };
    let btn = ToolButton::new(icon).tooltip(label.clone()).class(btn_class);

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
    tile_with_label(btn_stack, label, is_selected, entry)
}

fn graph_tile(tab: OpenTab, is_selected: bool, entry: RailEntry) -> impl Widget {
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
    let btn_class = if is_selected {
        "nav-rail-item selected"
    } else {
        "nav-rail-item"
    };
    let btn = ToolButton::new(icon).tooltip(title).class(btn_class);
    tile_with_label(btn, label, is_selected, entry)
}

fn chat_tile(id: String, title: String, is_selected: bool, entry: RailEntry) -> impl Widget {
    let shown = display_title(&title);
    let avatar = Avatar::new()
        .text(initials_from_title(&title))
        .size(32.0)
        .class(tone_for(&id));
    let ring_class = if is_selected {
        "nav-rail-chat-tile selected"
    } else {
        "nav-rail-chat-tile"
    };
    let body = DecoratedBox::new()
        .class(ring_class)
        .child(Center::new().child(avatar));
    tile_with_label(Tooltip::new(body, shown.clone()), shown, is_selected, entry)
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
            MenuItem::separator(),
            MenuItem::new("separator", tr!("nav.add.separator")).icon(MI_HORIZONTAL_RULE),
        ])
        .is_open(open)
        .position(pos)
        .on_select(|id| match id {
            "code" => rail::new_code_session(),
            "nodes" => rail::new_graph(),
            "chat" => rail::new_chat(),
            "separator" => rail::add_separator(),
            _ => {}
        });
    Stack::new().clip(false).child(drop).child(menu)
}
