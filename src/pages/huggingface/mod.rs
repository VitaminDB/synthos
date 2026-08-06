//! Страница HuggingFace — поиск, просмотр и скачивание моделей через
//! HuggingFace Hub API (https://huggingface.co/api/...).
//!
//! Маршрут `huggingface` (footer nav-rail, между `syn_explorer` и `settings`).
//! Контекст [`state::HuggingFaceCtx`] провайдится единожды в
//! [`crate::run_desktop`] и достаётся компонентами через
//! `use_context::<HuggingFaceCtx>()`.
//!
//! ```text
//! Stack [
//!   Column(.hf-page) [
//!     header::view()                       // title + search + chips
//!     SplitView Horizontal (split_ratio) [
//!       list_panel::view()                  // карточки моделей
//!       detail_panel::view()                // README + Files
//!     ]
//!   ]
//!   dialogs::view()                         // cache-dir prompt (Portal)
//! ]
//! ```

pub mod actions;
pub mod api;
pub mod card;
pub mod control;
pub mod convert;
pub mod detail_panel;
pub mod dialogs;
pub mod download;
pub mod filter;
pub mod header;
pub mod list_panel;
pub mod persist;
pub mod rate;
pub mod state;
pub mod verify;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

pub use state::HuggingFaceCtx;

pub fn view() -> impl Widget {
    let main = DecoratedBox::new()
        .class("hf-page")
        .child(mgui! {
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    header::view(),
                    DecoratedBox::new()
                        .class("hf-body grow")
                        .child(body_split()),
                ]
        });

    mgui! {
        Stack::new() => [
            main,
            dialogs::view(),
        ]
    }
}

fn body_split() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let ratio = ctx.split_ratio;
        let split = SplitView::new(list_panel::view(), detail_panel::view())
            .class("hf-split")
            .direction(SplitDirection::Horizontal)
            .ratio_signal(ratio)
            .min_size(260.0)
            .divider_width(6.0);
        vec![Box::new(split)]
    })
}
