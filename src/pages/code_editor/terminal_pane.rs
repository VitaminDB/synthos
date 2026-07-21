//! Нижняя панель центрального split'а — встроенный VTE-терминал с табами
//! в стиле Zed editor / VSCode.
//!
//! Архитектура:
//! - **TerminalsState** живёт в каждой [`CodeSession`] и держит
//!   `Vec<TerminalTab>` с `TerminalSession` в каждом (Arc inside) —
//!   переживает route switch.
//! - **tabs_bar** сверху: `Reactive` подписан на `tabs` + `active_id`
//!   активной сессии, рисует горизонтальный ряд табов + `+` для создания
//!   + gear-кнопка настроек шрифта справа.
//! - **active_terminal_view** ниже: `Reactive` подписан на `active_id`,
//!   создаёт `Terminal::new().attach(session)` для активного таба либо
//!   empty placeholder если ни одного таба нет.

use syngui::input::DragData;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::visual::terminal::TerminalCommand;
use syngui::widgets::{DropArea, GestureDetector, Terminal};

use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::settings::terminal::panel::terminal_font_panel;

use super::state::{
    add_terminal, close_terminal, paste_to_active_terminal, set_active_terminal, shell_quote_posix,
    CodeEditorCtx, CodeSession, TerminalTab,
};

pub fn view() -> impl Widget {
    // RwSignal видимости gear-popover'а. Локальный — popover не должен
    // переживать переключение страниц «Чат» ↔ «Редактор кода».
    let gear_open = use_signal(false);

    DecoratedBox::new()
        .child(mgui! {
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    tabs_bar(gear_open),
                    active_terminal_view(),
                    gear_popover(gear_open),
                ]
        })
        .class("code-editor-terminal-pane")
}

/// Верхний бар с табами + кнопка `+` + gear-кнопка справа.
/// Реактивен по активной сессии и её `tabs`/`active_id`.
fn tabs_bar(gear_open: RwSignal<bool>) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let code = use_context::<CodeEditorCtx>();
        let app = use_context::<AppCtx>();
        let Some(session) = code.active_session() else {
            // Без активной сессии не должны попасть сюда (mod.rs::view()
            // рендерит SplitView только при наличии сессии). Защитный
            // empty-bar — чтобы Reactive не паниковал.
            return vec![Box::new(DecoratedBox::new().class("term-tabs-bar"))];
        };
        let tabs = session.terminals.tabs.get();
        let active_id = session.terminals.active_id.get();

        let mut tabs_row = Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Start);
        for tab in tabs.iter() {
            tabs_row = tabs_row.child(tab_chip(session, tab.clone(), active_id == Some(tab.id)));
        }
        let app_for_add = app.clone();
        let add_btn = ToolButton::new(MI_ADD)
            .on_click(move || {
                let _ = add_terminal(session, app_for_add.clone());
            })
            .class("term-tab-add");
        tabs_row = tabs_row.child(add_btn);

        let gear_btn = ToolButton::new(MI_TUNE)
            .on_click(move || {
                gear_open.set(!gear_open.get_untracked());
            })
            .class("term-gear-btn");

        let bar = DecoratedBox::new()
            .child(
                Row::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .child(tabs_row)
                    .child(DecoratedBox::new().class("term-tabs-spacer"))
                    .child(gear_btn),
            )
            .class("term-tabs-bar");
        vec![Box::new(bar)]
    })
}

/// Один таб: GestureDetector(on_click=switch) > DecoratedBox > Row { icon, title, × }.
fn tab_chip(session: CodeSession, tab: TerminalTab, is_active: bool) -> impl Widget {
    let id = tab.id;
    let title_signal = tab.title;
    let class = if is_active {
        "term-tab term-tab--active"
    } else {
        "term-tab"
    };
    let icon = Text::new(MI_TERMINAL).class("term-tab-icon");
    let title_box = DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            vec![Box::new(
                Text::new(title_signal.get()).class("term-tab-title-text"),
            )]
        }))
        .class("term-tab-title");
    let close_btn = ToolButton::new(MI_CLOSE)
        .on_click(move || {
            close_terminal(session, id);
        })
        .class("term-tab-close");
    let inner = DecoratedBox::new()
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Start)
                .child(icon)
                .child(title_box)
                .child(close_btn),
        )
        .class(class);
    GestureDetector::new()
        .on_click(move || {
            set_active_terminal(session, id);
        })
        .child(inner)
}

/// Активный терминал — `Terminal::attach(session)` для active_id, либо
/// empty placeholder. Реактивен по `active_id` активной сессии и font-сигналам.
///
/// Обёрнут в [`DropArea`] (accept_types=["file"]): при перетаскивании файла из
/// проводника в область терминала путь shell-quoted'ится и вставляется в
/// активную PTY-session **без** auto-newline — пользователь сам решит, нажать
/// Enter или дописать команду перед путём (`cd `, `cat ` и т.п.). Multi-file
/// drop вставит несколько путей подряд через пробел (winit шлёт на каждый
/// файл отдельный `DroppedFile`).
///
/// Внешний `DecoratedBox` с классом `term-active-area` (flex-grow: 1) нужен
/// именно здесь, на child Column'а: DropArea сам не имеет flex-grow и
/// получил бы от Column `max_height = INFINITY` → Terminal::layout попал бы
/// в fallback-ветку 320 px и не растягивался на доступное пространство
/// pane'а (regression коммита, добавившего DropArea — см. `term-active-area`).
fn active_terminal_view() -> impl Widget {
    DecoratedBox::new()
        .child(
            DropArea::new()
                .accept_types(vec![DragData::TYPE_FILE.to_string()])
                .on_drop(|data| {
                    // OS DnD path drop — `payload` уже UTF-8 (через `to_string_lossy`),
                    // у пути может быть пробел/спецсимвол → POSIX-quote обязателен.
                    // Trailing space нужен на случай multi-file drop, чтобы пути
                    // разделились в shell-prompt'е как отдельные аргументы.
                    let mut quoted = shell_quote_posix(&data.payload);
                    quoted.push(' ');
                    let _ = paste_to_active_terminal(&quoted);
                })
                .child(active_terminal_inner()),
        )
        .class("term-active-area")
}

fn active_terminal_inner() -> impl Widget {
    DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let code = use_context::<CodeEditorCtx>();
            let app = use_context::<AppCtx>();
            let Some(session) = code.active_session() else {
                return vec![Box::new(empty_placeholder())];
            };
            let active_id = session.terminals.active_id.get();
            let family = app.terminal_font_family.get();
            let size = app.terminal_font_size.get();

            // session берём untracked — реактивность нам нужна только на
            // active_id и font; перерисовка всех табов при добавлении
            // нового спровоцировала бы лишний rebuild Terminal-Element'а.
            let pty_session = active_id.and_then(|id| {
                session
                    .terminals
                    .tabs
                    .get_untracked()
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.session.clone())
            });

            match pty_session {
                None => vec![Box::new(empty_placeholder())],
                Some(pty) => {
                    // Канал команд от ContextMenu (Copy/Paste/Clear) → Terminal.
                    let cmd: RwSignal<Option<TerminalCommand>> = use_signal(None);
                    let mut term = Terminal::new()
                        .attach(pty)
                        .autofocus(true)
                        .font_size(size)
                        .command_signal(cmd);
                    if !family.is_empty() {
                        term = term.font_family(family);
                    }
                    let menu = ContextMenu::new()
                        .child(term.class("code-editor-terminal"))
                        .items(vec![
                            MenuItem::new("copy", "Копировать")
                                .icon(MI_CONTENT_COPY)
                                .shortcut("Ctrl+Shift+C"),
                            MenuItem::new("paste", "Вставить")
                                .icon(MI_CONTENT_PASTE)
                                .shortcut("Ctrl+Shift+V"),
                            MenuItem::separator(),
                            MenuItem::new("clear", "Очистить").icon(MI_CLEAR_ALL),
                        ])
                        .on_select(move |id| {
                            let action = match id {
                                "copy" => Some(TerminalCommand::Copy),
                                "paste" => Some(TerminalCommand::Paste),
                                "clear" => Some(TerminalCommand::Clear),
                                _ => None,
                            };
                            if let Some(a) = action {
                                cmd.set(Some(a));
                            }
                        });
                    vec![Box::new(menu)]
                }
            }
        }))
}

/// Empty state: подсказка «нажмите +, чтобы открыть терминал».
fn empty_placeholder() -> impl Widget {
    Column::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(8.0)
        .child(Text::new(MI_TERMINAL).class("term-empty-icon"))
        .child(Text::new("Нет открытых терминалов").class("term-empty-title"))
        .child(Text::new("Нажмите + чтобы открыть новый").class("term-empty-hint"))
}

/// Portal-popover с панелью настроек шрифта. Открывается gear-кнопкой,
/// закрывается кликом мимо backdrop'а. Якорь — top-end (правый верхний)
/// со смещением, чтобы попадать примерно под gear-иконку в tab-bar'е.
fn gear_popover(gear_open: RwSignal<bool>) -> impl Widget {
    Portal::new()
        .is_open(gear_open)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::TopEnd {
            margin_top: 56.0,
            margin_right: 16.0,
        })
        .child(
            DecoratedBox::new()
                .child(terminal_font_panel())
                .class("term-gear-popover"),
        )
}
