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
//!   Column(.syn-explorer-page) [
//!     toolbar::view()
//!     SplitView Horizontal (left_split_ratio) [
//!       left_panel::view()
//!       SplitView Horizontal (right_split_ratio) [
//!         center (Reactive: tabs+content or placeholder)
//!         right_panel::view()
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
pub mod state;
pub mod tab_files;
pub mod tab_metadata;
pub mod tab_overview;
pub mod tab_preview;
pub mod tabs;
pub mod toolbar;
pub mod tree_build;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

pub use state::SynExplorerCtx;

use crate::icons::{MI_FOLDER_OPEN, MI_INVENTORY_2};

pub fn view() -> impl Widget {
    let main = DecoratedBox::new()
        .class("syn-explorer-page")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            // Подписки на ratio'ы — выясняются в SplitView::ratio_signal.
            let left_ratio = ctx.left_split_ratio;
            let right_ratio = ctx.right_split_ratio;

            let center = DecoratedBox::new()
                .class("syn-explorer-center")
                .child(center_or_placeholder());

            let center_with_right = SplitView::new(center, right_panel::view())
                .class("syn-explorer-h-split")
                .direction(SplitDirection::Horizontal)
                .ratio_signal(right_ratio)
                .min_size(180.0)
                .divider_width(6.0);

            let split = SplitView::new(left_panel::view(), center_with_right)
                .class("syn-explorer-h-split")
                .direction(SplitDirection::Horizontal)
                .ratio_signal(left_ratio)
                .min_size(220.0)
                .divider_width(6.0);

            vec![Box::new(mgui! {
                Column::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        toolbar::view(),
                        DecoratedBox::new()
                            .class("syn-explorer-body grow")
                            .child(split),
                    ]
            })]
        }));

    mgui! {
        Stack::new() => [
            main,
            dialogs::view(),
        ]
    }
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
