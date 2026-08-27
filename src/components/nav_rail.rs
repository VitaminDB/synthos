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
//! │ ⚙  │  Настройки
//! │ 文 │  язык интерфейса
//! │ AB │  аватар пользователя
//! └────┘
//! ```
//!
//! Верхних иконок-разделов «Чат» и «Ноды» больше нет: к чату и графу
//! ведут их плитки, а новые создаются через «+». Клик по плитке —
//! `rail::open`, «Закрыть» в контекстном меню — `rail::request_close`.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::{GestureDetector, Positioned, Stack};
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
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
    Item { icon: MI_SETTINGS,       route: "settings",      tooltip_key: "nav.settings" },
];

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

fn bottom_cluster() -> impl Widget {
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("nav-rail-footer") => [
                DecoratedBox::new().class("nav-rail-divider"),
                rail_column(FOOTER_UTILITY),
                language_button(),
                avatar(),
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

fn avatar() -> impl Widget {
    Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let name = ctx.general.display_name.get();
        vec![Box::new(
            Avatar::new()
                .text(initials(&name))
                .size(40.0)
                .class("avatar-slate"),
        ) as Box<dyn Widget>]
    })
}

fn initials(name: &str) -> String {
    let mut out = String::new();
    for word in name.split_whitespace().take(2) {
        if let Some(c) = word.chars().next() {
            for up in c.to_uppercase() {
                out.push(up);
            }
        }
    }
    out
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
        for (idx, entry) in entries.iter().enumerate() {
            let active = rail::is_active(entry);
            let tile: Box<dyn Widget> = match entry {
                RailEntry::Code(s) => Box::new(code_tile(*s, idx, active)),
                RailEntry::Graph(t) => Box::new(graph_tile(*t, active)),
                RailEntry::Chat(m) => Box::new(chat_tile(m.id.clone(), m.title.clone(), active)),
                RailEntry::Separator(ts) => Box::new(separator_tile(*ts)),
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

/// Общая обёртка плитки: кнопка/аватар + подпись под ней + контекстное
/// меню «Закрыть».
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
                Text::new(label).max_lines(1).class(label_class),
            ]
    };
    ContextMenu::new()
        .items(vec![MenuItem::new("close", tr!("app.close")).icon(MI_CLOSE)])
        .on_select(move |action| {
            if action == "close" {
                rail::request_close(&entry);
            }
        })
        .child(tile)
}

fn code_tile(session: CodeSession, idx: usize, is_selected: bool) -> impl Widget {
    let folder = session.root_folder.get_untracked();
    let icon = if folder.is_some() { MI_FOLDER } else { MI_DESCRIPTION };
    let label = folder
        .as_ref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| tr!("nav.session.unnamed", n = idx + 1));
    let btn_class = if is_selected {
        "nav-rail-item selected"
    } else {
        "nav-rail-item"
    };

    let entry = RailEntry::Code(session);
    let entry_click = entry.clone();
    let btn = ToolButton::new(icon)
        .tooltip(label.clone())
        .on_click(move || rail::open(&entry_click))
        .class(btn_class);

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

fn graph_tile(tab: OpenTab, is_selected: bool) -> impl Widget {
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
    let entry = RailEntry::Graph(tab);
    let entry_click = entry.clone();
    let btn = ToolButton::new(icon)
        .tooltip(title)
        .on_click(move || rail::open(&entry_click))
        .class(btn_class);
    tile_with_label(btn, label, is_selected, entry)
}

fn chat_tile(id: String, title: String, is_selected: bool) -> impl Widget {
    let shown = display_title(&title);
    let avatar = Avatar::new()
        .text(initials_from_title(&title))
        .size(34.0)
        .class(tone_for(&id));
    let ring_class = if is_selected {
        "nav-rail-chat-tile selected"
    } else {
        "nav-rail-chat-tile"
    };
    let meta = crate::agent::state::ChatMeta {
        id: id.clone(),
        title: title.clone(),
        preview: String::new(),
        created_at: 0,
        updated_at: 0,
        model_name: None,
        archived: false,
    };
    let entry = RailEntry::Chat(meta);
    let entry_click = entry.clone();
    let body = GestureDetector::new()
        .on_click(move || rail::open(&entry_click))
        .child(
            DecoratedBox::new()
                .class(ring_class)
                .child(Center::new().child(avatar)),
        );
    tile_with_label(Tooltip::new(body, shown.clone()), shown, is_selected, entry)
}

/// Разделитель: тонкая линия в широкой невидимой зоне — чтобы по ней можно
/// было попасть правой кнопкой и удалить.
fn separator_tile(ts: u64) -> impl Widget {
    let line = DecoratedBox::new()
        .class("nav-rail-separator-hit")
        .child(Center::new().child(DecoratedBox::new().class("nav-rail-separator")));
    ContextMenu::new()
        .items(vec![MenuItem::new("remove", tr!("app.delete")).icon(MI_CLOSE)])
        .on_select(move |action| {
            if action == "remove" {
                rail::remove_separator(ts);
            }
        })
        .child(line)
}

/// «+» — меню выбора, что создать.
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
    Stack::new().clip(false).child(btn).child(menu)
}
