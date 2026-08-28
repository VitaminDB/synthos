//! Страница SynExplorer — UI для работы с `.syn` bundle-пакетами:
//! просмотр содержимого, статистика, редактирование метаданных и файлов
//! внутри пакета, создание новых пакетов.
//!
//! Маршрут `syn_explorer`. Контекст [`state::SynExplorerCtx`] провайдится
//! единожды в [`crate::run_desktop`] и достаётся компонентами через
//! `use_context::<SynExplorerCtx>()`.
//!
//! ```text
//! Stack [
//!   workspace_frame(.syn-explorer-page) [
//!     [▤ Закладки +] [пакет · путь/статус  🔍 поиск  кнопки toolbar'а] [Файлы пакета ▥]
//!     SplitView Horizontal (left_split_ratio) [
//!       left_panel::view()                // закладки + файлы папки
//!       SplitView Horizontal (right_split_ratio) [
//!         center (Reactive: tabs+content or placeholder)
//!         right_panel::view()             // дерево пакета
//!       ]
//!     ]
//!   ]
//!   dialogs::view()                       // Portal с модалами
//! ]
//! ```

pub mod actions;
pub mod bookmarks;
pub mod bundle_io;
pub mod dialogs;
pub mod left_panel;
pub mod right_panel;
pub mod pack_dialogs;
pub mod quant_pack;
pub mod state;
pub mod tab_files;
pub mod tab_layers;
pub mod tab_metadata;
pub mod tab_overview;
pub mod tab_preview;
pub mod tabs;
pub mod toolbar;
pub mod tree_build;

use syngui::mgui;
use syngui::prelude::*;

pub use state::SynExplorerCtx;

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::{MI_FOLDER_OPEN, MI_INVENTORY_2};

pub fn view() -> impl Widget {
    let app = use_context::<AppCtx>();
    let ctx = use_context::<SynExplorerCtx>();
    let (left_visible, right_visible) = app.panels.syn_explorer;

    let spec = FrameSpec::new(
        "syn-explorer-h-split",
        || {
            let identity: Box<dyn Widget> =
                Box::new(DecoratedBox::new().child(identity_reactive));
            Box::new(panel_header::center(
                CenterSpec::new(identity).actions(toolbar::header_actions()),
            ))
        },
        || {
            Box::new(
                DecoratedBox::new()
                    .class("syn-explorer-center")
                    .child(center_or_placeholder()),
            )
        },
    )
    .left(Pane::new(
        left_visible,
        ctx.left_split_ratio,
        220.0,
        || Box::new(left_panel::header()),
        || Box::new(left_panel::view()),
    ))
    .right(Pane::new(
        right_visible,
        ctx.right_split_ratio,
        180.0,
        || Box::new(right_panel::header()),
        || Box::new(right_panel::view()),
    ));

    let main = DecoratedBox::new()
        .class("syn-explorer-page")
        .child(workspace_frame::view(spec));

    mgui! {
        Stack::new() => [
            main,
            dialogs::view(),
        ]
    }
}

/// Идентичность: имя открытого пакета и его путь (или статус операции);
/// без пакета — название раздела.
fn identity_reactive() -> Stack {
    let ctx = use_context::<SynExplorerCtx>();
    let active = ctx.active_bundle.get();
    let load_state = ctx.load_state.get();
    let (title, subtitle) = match active {
        Some(b) => {
            let path = b.path.get();
            let dirty = b.dirty.get();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            let sub = toolbar::status_text(load_state, dirty)
                .unwrap_or_else(|| path.display().to_string());
            (name, sub)
        }
        None => (
            tr!("nav.syn_explorer"),
            toolbar::status_text(load_state, false)
                .unwrap_or_else(|| tr!("explorer.header.no_bundle")),
        ),
    };
    expand(panel_header::identity_text(MI_INVENTORY_2, title, subtitle))
}

/// Центр страницы — Reactive, который рисует табы/контент когда пакет открыт,
/// и placeholder иначе.
fn center_or_placeholder() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let Some(active) = ctx.active_bundle.get() else {
            return vec![Box::new(empty_center_placeholder())];
        };
        let _ = active.reload_gen.get();
        vec![Box::new(center_with_tabs(active))]
    })
}

/// Центр с табами и контентом активного таба.
fn center_with_tabs(active: state::OpenBundle) -> impl Widget {
    let _ = active;
    mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                tabs::tab_bar(),
                DecoratedBox::new()
                    .class("syn-tab-content grow")
                    .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
                        let ctx = use_context::<SynExplorerCtx>();
                        let Some(active) = ctx.active_bundle.get() else {
                            return vec![Box::new(DecoratedBox::new())];
                        };
                        let _ = active.reload_gen.get();
                        let widget: Box<dyn Widget> = match ctx.current_tab.get() {
                            state::TabKind::Overview => Box::new(tab_overview::view(active)),
                            state::TabKind::Layers => Box::new(tab_layers::view(active)),
                            state::TabKind::Files => Box::new(tab_files::view(active)),
                            state::TabKind::Metadata => Box::new(tab_metadata::view(active)),
                            state::TabKind::Preview => Box::new(tab_preview::view(active)),
                        };
                        vec![widget]
                    })),
            ]
    }
}

/// Заглушка центра, когда пакет не открыт.
fn empty_center_placeholder() -> impl Widget {
    let inner = mgui! {
        Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Center)
            .class("syn-placeholder") => [
                Text::new(MI_INVENTORY_2).class("syn-placeholder-icon"),
                Text::new(tr!("explorer.placeholder.title")).class("syn-placeholder-title"),
                Text::new(tr!("explorer.placeholder.hint"))
                    .class("syn-placeholder-hint"),
                Button::new(tr!("explorer.placeholder.open_button"))
                    .leading_icon(MI_FOLDER_OPEN)
                    .on_click(|| {
                        let ctx = use_context::<SynExplorerCtx>();
                        actions::pick_and_open_bundle(ctx);
                    })
                    .class("syn-placeholder-cta"),
            ]
    };
    Center::new().child(inner)
}
