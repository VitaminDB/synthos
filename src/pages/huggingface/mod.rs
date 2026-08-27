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
//!   workspace_frame [
//!     page_header                        // «HuggingFace», пилюля + поле поиска по Hub
//!     SplitView (hf_left_split_ratio) [
//!       list_panel                       // чипы сортировки + карточки моделей
//!       SplitView (hf_right_split_ratio) [
//!         detail_panel::view             // шапка репозитория + README
//!         detail_panel::files_view       // файлы + загрузки
//!       ]
//!     ]
//!   ]
//!   dialogs::view()                      // cache-dir prompt (Portal)
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

use crate::components::page_header::{self, HeaderSpec};
use crate::components::workspace_frame::{self, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::MI_CLOUD_DOWNLOAD;

pub use state::HuggingFaceCtx;

pub fn view() -> impl Widget {
    let app = use_context::<AppCtx>();
    let (left_visible, right_visible) = app.panels.huggingface;

    let identity = page_header::identity_text(
        MI_CLOUD_DOWNLOAD,
        "HuggingFace".to_string(),
        tr!("hf.header.subtitle"),
    );
    let header = page_header::view(
        HeaderSpec::new(identity)
            .center_extra(DecoratedBox::new().class("hf-header-search").child(header::search_bar()))
            .toggles(Some(left_visible), Some(right_visible)),
    );

    let spec = FrameSpec::new("hf-split", || Box::new(detail_panel::view()))
        .left(Pane::new(left_visible, app.hf_left_split_ratio, 260.0, || {
            Box::new(list_panel::view())
        }))
        .right(Pane::new(right_visible, app.hf_right_split_ratio, 260.0, || {
            Box::new(detail_panel::files_view())
        }));

    let main = DecoratedBox::new()
        .class("hf-page")
        .child(workspace_frame::view(header, spec));

    mgui! {
        Stack::new() => [
            main,
            dialogs::view(),
        ]
    }
}
