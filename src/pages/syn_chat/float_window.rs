//! Отрыв чата в плавающее окно.
//!
//! Кнопка-стрелка в шапке чата (`chat_header`) отрывает ленту с вводом от
//! страницы: они переезжают в [`FloatingWindow`] — его можно таскать,
//! растягивать и сворачивать кнопкой «—» в кнопку-аватар слева внизу
//! (`fab`). Иконку можно и потянуть: окно встанет туда, где отпустили
//! (`tear_off_host` — DropArea на всю оболочку). Окно смонтировано в
//! оболочке (`lib.rs::build_app`), а не на странице чата, поэтому живёт
//! поверх заметок, редактора и настроек.
//!
//! Окно одно и показывает активный чат: клик по другой плитке в рейле
//! переключает содержимое, а не открывает второе окно
//! (`rail::open`). Пока чат оторван, центральная колонка страницы —
//! плейсхолдер с «Вернуть» (`chat_pane::view`); лента и ввод существуют в
//! одном экземпляре, так что фокус, скролл и черновик не раздваиваются.
//!
//! Состояние — `SynChatCtx.chat_detached / chat_window_minimized /
//! chat_window_pos / chat_window_size`, persist в
//! `AppConfig.syn_chat_detached / syn_chat_window_*`: после перезапуска
//! чат остаётся там, где его оставили.

use syngui::input::CursorIcon;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::feedback::Tooltip;
use syngui::widgets::overlay::drop_area::DropInfo;
use syngui::widgets::overlay::portal::{Portal, PortalAnchor};
use syngui::widgets::overlay::DropArea;
use syngui::widgets::FloatingWindow;

use crate::components::chat_item::{display_title, initials_from_title, tone_for};
use crate::components::workspace_frame::expand;
use crate::context::AppCtx;
use crate::icons::MI_CHAT;
use crate::rail;
use crate::syn_chat::SynChatCtx;

use super::chat_pane;

/// `drag_type` перетаскивания иконки отрыва: свой, чтобы DropArea рейла и
/// панели ввода (файлы) на него не реагировали, а наш хост — только на
/// него.
pub const TEAR_OFF_DRAG_TYPE: &str = "syn-chat-tear-off";

/// Высота полосы заголовка `FloatingWindow` (константа syngui
/// `TITLE_BAR_HEIGHT`): при дропе окно ставится так, чтобы под курсором
/// оказалась середина заголовка — как будто его и тащили.
const TITLE_BAR_HEIGHT: f32 = 36.0;

// ─────────────────────────── действия ───────────────────────────

/// Оторвать чат: окно открывается там, где было в прошлый раз (или в
/// позиции по умолчанию), развёрнутым.
pub fn detach() {
    let ctx = use_context::<SynChatCtx>();
    ctx.chat_window_minimized.set(false);
    ctx.chat_detached.set(true);
}

/// Оторвать чат дропом иконки: окно встаёт заголовком под курсор, не
/// выходя за пределы хоста (`host` — размер DropArea, т.е. оболочки).
pub fn detach_at(drop: Point, host: Size) {
    let ctx = use_context::<SynChatCtx>();
    let size = ctx.chat_window_size.get_untracked();
    let mut x = drop.x - size.width / 2.0;
    let mut y = drop.y - TITLE_BAR_HEIGHT / 2.0;
    if host.width > 0.0 && host.height > 0.0 {
        x = x.clamp(0.0, (host.width - size.width).max(0.0));
        y = y.clamp(0.0, (host.height - size.height).max(0.0));
    }
    ctx.chat_window_pos.set(Point::new(x, y));
    detach();
}

/// Вернуть чат на страницу (крестик окна, «Вернуть» в плейсхолдере и в
/// шапке). Страница чата открывается сама: иначе чат «пропадал» бы, если
/// окно закрыли поверх заметок.
pub fn dock() {
    let ctx = use_context::<SynChatCtx>();
    ctx.chat_detached.set(false);
    ctx.chat_window_minimized.set(false);
    rail::navigate("syn_chat");
}

/// Развернуть свёрнутое окно (клик по кнопке-аватару, плитке чата в
/// рейле, «Показать окно» в плейсхолдере).
pub fn restore() {
    use_context::<SynChatCtx>().chat_window_minimized.set(false);
}

// ─────────────────────────── окно ───────────────────────────

/// Плавающее окно с лентой и вводом. Заголовок — название активного чата;
/// пересобирается только при смене чата или его имени, а тело живёт во
/// вложенной реактивной ветке по `chat_detached`.
pub fn window() -> impl Widget {
    DecoratedBox::new().child(|| {
        let ctx = use_context::<SynChatCtx>();
        let title = active_title(&ctx)
            .map(|(_, t)| display_title(&t))
            .unwrap_or_else(|| tr!("chat.window.title"));
        FloatingWindow::new(title)
            .icon(MI_CHAT)
            .is_open(ctx.chat_detached)
            .position(ctx.chat_window_pos)
            .size(ctx.chat_window_size.get_untracked())
            .size_signal(ctx.chat_window_size)
            .with_resizable(true)
            .closable(true)
            .minimizable(true)
            .is_minimized(ctx.chat_window_minimized)
            .on_close(dock)
            .child(body())
            .class("chat-float-window")
    })
}

/// Тело окна строится только пока чат оторван: `FloatingWindow` монтирует
/// детей и в закрытом виде, а второй экземпляр ленты и поля ввода за
/// спиной у страницы ни к чему.
fn body() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        if use_context::<SynChatCtx>().chat_detached.get() {
            vec![Box::new(expand(Box::new(chat_pane::pane())))]
        } else {
            vec![Box::new(DecoratedBox::new().class("chat-float-window-empty"))]
        }
    })
}

/// `(id, название)` активного чата. Подписки на `chats`, `active_chat_id`
/// и `last_saved_fp` — чтобы переименование дошло до заголовка.
fn active_title(ctx: &SynChatCtx) -> Option<(String, String)> {
    let active = ctx.active_chat_id.get();
    let chats = ctx.chats.get();
    let _ = ctx.last_saved_fp.get();
    active.and_then(|id| chats.into_iter().find(|m| m.id == id).map(|m| (m.id, m.title)))
}

// ─────────────────────────── кнопка-аватар ───────────────────────────

/// Ширина нав-рейла (`.nav-rail { width: 72px }`), высота статусбара
/// (`.window-statusbar { height: 4px }`) и воздух вокруг оболочки в
/// restored-режиме (`.window-backdrop { padding: 30px }`): Portal считает
/// отступы от края вьюпорта, а кнопка должна стоять правее рейла — над
/// кнопкой настроек она её перекрывала.
const RAIL_WIDTH: f32 = 72.0;
const STATUSBAR_HEIGHT: f32 = 4.0;
const SHELL_AIR_RESTORED: f32 = 30.0;
const FAB_GAP: f32 = 12.0;

/// Свёрнутое окно: аватар активного чата в левом нижнем углу контента,
/// правее рейла, по образцу голосового FAB справа
/// (`components::voice_fab::fab_button`). Portal позиционирует кнопку сам,
/// минуя раскладку оболочки; отступы пересчитываются при maximize —
/// воздух вокруг оболочки исчезает, и кнопка подтягивается к углу.
pub fn fab() -> impl Widget {
    let open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let want = ctx.chat_detached.get() && ctx.chat_window_minimized.get();
        if open.get_untracked() != want {
            open.set(want);
        }
    });
    DecoratedBox::new().child(move || {
        let maximized = use_context::<AppCtx>().appearance.window_state.get().maximized;
        let air = if maximized { 0.0 } else { SHELL_AIR_RESTORED };
        Portal::new()
            .is_open(open)
            .anchor(PortalAnchor::BottomStart {
                margin_bottom: air + STATUSBAR_HEIGHT + FAB_GAP,
                margin_left: air + RAIL_WIDTH + FAB_GAP,
            })
            .modal(false)
            .backdrop(false)
            .child(fab_button())
    })
}

fn fab_button() -> impl Widget {
    DecoratedBox::new().child(|| {
        let ctx = use_context::<SynChatCtx>();
        let (id, title) = active_title(&ctx).unwrap_or_default();
        let shown = if title.trim().is_empty() {
            tr!("chat.window.title")
        } else {
            display_title(&title)
        };
        // Пока идёт генерация, кнопка пульсирует: ответ приходит в
        // свёрнутое окно, и без подсказки этого не видно.
        let class = if ctx.pending.get() {
            "fab-chat-corner generating"
        } else {
            "fab-chat-corner"
        };
        let avatar = Avatar::new()
            .text(initials_from_title(&title))
            .size(44.0)
            .class(tone_for(&id));
        let body = DecoratedBox::new()
            .class(class)
            .child(Center::new().child(avatar));
        GestureDetector::new()
            .cursor(CursorIcon::Pointer)
            .on_click(restore)
            .child(Tooltip::new(body, tr!("chat.window.fab.tooltip", name = shown)))
    })
}

// ─────────────────────────── хост дропа ───────────────────────────

/// Обёртка оболочки, принимающая дроп иконки отрыва: окно встаёт туда, где
/// отпустили. Прозрачна для раскладки и для чужих перетаскиваний (плитки
/// рейла, файлы в панель ввода).
pub fn tear_off_host(child: impl Widget + 'static) -> impl Widget {
    DropArea::new()
        .accept_types(vec![TEAR_OFF_DRAG_TYPE.to_string()])
        .on_drop_positioned(|info: DropInfo| detach_at(info.position, info.size))
        .child(child)
}

// ─────────────────────────── плейсхолдер ───────────────────────────

/// Центральная колонка страницы, пока чат оторван: подсказка и кнопки
/// «Показать окно» / «Вернуть на страницу».
pub fn placeholder() -> impl Widget {
    let buttons = DecoratedBox::new().child(|| {
        let minimized = use_context::<SynChatCtx>().chat_window_minimized.get();
        let mut row = Row::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        if minimized {
            row = row.child(
                Button::new(tr!("chat.detached.show"))
                    .leading_icon(MI_CHAT)
                    .on_click(restore)
                    .class("code-editor-dialog-btn-secondary"),
            );
        }
        row.child(
            Button::new(tr!("chat.detached.dock"))
                .leading_icon(crate::icons::MI_CLOSE_FULLSCREEN)
                .on_click(dock)
                .class("code-editor-dialog-btn-primary"),
        )
    });
    mgui! {
        DecoratedBox::new().class("chat-pane-wrap chat-detached-placeholder") => [
            Center::new() => [
                Column::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("chat-detached-card") => [
                        Icon::new(crate::icons::MI_OPEN_IN_NEW).class("chat-detached-icon"),
                        Text::new(tr!("chat.detached.title")).class("chat-detached-title"),
                        Text::new(tr!("chat.detached.hint"))
                            .class("chat-detached-hint"),
                        buttons,
                    ]
            ]
        ]
    }
}
