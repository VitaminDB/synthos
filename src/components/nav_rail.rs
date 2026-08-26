use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::{Positioned, Stack};
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
use syngui::widgets::visual::{Badge, Image, ImageFit};

use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::code_editor::state::{CodeEditorCtx, CodeSession, SessionId};
use crate::pages::huggingface::state::DlStatus;
use crate::pages::huggingface::HuggingFaceCtx;

#[derive(Clone, Copy)]
struct Item {
    icon: &'static str,
    route: &'static str,
    tooltip_key: &'static str,
}

const PRIMARY: &[Item] = &[
    Item { icon: MI_CHAT, route: "syn_chat", tooltip_key: "nav.syn_chat" },
    Item { icon: MI_HUB,  route: "nodes",    tooltip_key: "nav.nodes" },
];

const FOOTER_UTILITY: &[Item] = &[
    Item { icon: MI_HISTORY,        route: "voice_history", tooltip_key: "nav.voice_history" },
    Item { icon: MI_INVENTORY_2,    route: "syn_explorer",  tooltip_key: "nav.syn_explorer" },
    Item { icon: MI_CLOUD_DOWNLOAD, route: "huggingface",   tooltip_key: "nav.huggingface" },
    Item { icon: MI_SETTINGS,       route: "settings",      tooltip_key: "nav.settings" },
];

pub fn view() -> impl Widget {
    DecoratedBox::new().class("nav-rail").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                top_cluster(),
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
                rail_column(PRIMARY),
                DecoratedBox::new().class("nav-rail-divider"),
                sessions_segment(),
            ]
    }
}

fn bottom_cluster() -> impl Widget {
    mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("nav-rail-footer") => [
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
        .on_click(move || {
            let app = use_context::<AppCtx>();
            if app.current_route.get_untracked() == route {
                return;
            }
            app.router.lock().unwrap().navigate(route);
            app.current_route.set(route.to_string());
        })
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
        .on_click(move || {
            let ctx = use_context::<AppCtx>();
            if ctx.current_route.get_untracked() == route {
                return;
            }
            ctx.router.lock().unwrap().navigate(route);
            ctx.current_route.set(route.to_string());
        })
        .class(class)
}

fn sessions_segment() -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let code = use_context::<CodeEditorCtx>();
        let app = use_context::<AppCtx>();
        let _ = code.session_gen.get();
        let sessions = code.sessions.get();
        let active_id = code.active_id.get();
        let on_code_route = app.current_route.get() == "code";

        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("nav-rail-sessions");
        for (idx, s) in sessions.iter().enumerate() {
            let session = *s;
            let is_active = on_code_route && active_id == Some(session.id);
            col = col.child(session_rail_item(session, idx, is_active));
        }
        col = col.child(add_session_btn());
        vec![Box::new(col)]
    })
}

fn session_rail_item(session: CodeSession, idx: usize, is_selected: bool) -> impl Widget {
    let id: SessionId = session.id;
    let folder = session.root_folder.get();
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
    let label_class = if is_selected {
        "nav-rail-session-label selected"
    } else {
        "nav-rail-session-label"
    };

    let btn = ToolButton::new(icon)
        .tooltip(label.clone())
        .on_click(move || {
            let app = use_context::<AppCtx>();
            let code = use_context::<CodeEditorCtx>();
            code.switch_to(id);
            if app.current_route.get_untracked() != "code" {
                app.router.lock().unwrap().navigate("code");
                app.current_route.set("code".to_string());
            }
        })
        .class(btn_class);

    // Бейдж количества открытых терминалов сессии — по образцу hf_rail_item.
    // Цвет по занятости (busy_count пишет семплер terminal_activity;
    // занят = вывод обновлялся в последние секунды): все заняты — зелёный,
    // все простаивают — красный, смешанно — оранжевый.
    // Реактивность — через .get() в scope Reactive'а sessions_segment.
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

    let tile = mgui! {
        Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                btn_stack,
                Text::new(label).max_lines(1).class(label_class),
            ]
    };

    ContextMenu::new()
        .items(vec![MenuItem::new("close", tr!("app.close")).icon(MI_CLOSE)])
        .on_select(move |action| {
            if action == "close" {
                let code = use_context::<CodeEditorCtx>();
                code.close(id);
            }
        })
        .child(tile)
}

fn add_session_btn() -> impl Widget {
    ToolButton::new(MI_ADD)
        .tooltip(tr!("nav.session.new"))
        .on_click(move || {
            let app = use_context::<AppCtx>();
            let code = use_context::<CodeEditorCtx>();
            let _ = code.create_empty();
            if app.current_route.get_untracked() != "code" {
                app.router.lock().unwrap().navigate("code");
                app.current_route.set("code".to_string());
            }
        })
        .class("nav-rail-item nav-rail-item-add")
}
