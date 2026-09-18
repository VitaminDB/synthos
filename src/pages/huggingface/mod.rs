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
//!   Column [ workspace_frame (grow), dock::view() ]   // нижняя панель загрузок
//!   workspace_frame [
//!     [▤ Модели] [HuggingFace  🔍 поиск  поле поиска по Hub] [Файлы ▥]
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
pub mod dock;
pub mod download;
pub mod filter;
pub mod header;
pub mod list_panel;
pub mod persist;
pub mod progress;
pub mod rate;
pub mod state;
pub mod verify;

use syngui::mgui;
use syngui::prelude::*;

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::{MI_CLOUD_DOWNLOAD, MI_DESCRIPTION, MI_SEARCH};

pub use state::HuggingFaceCtx;

pub fn view() -> impl Widget {
    let app = use_context::<AppCtx>();
    let (left_visible, right_visible) = app.panels.huggingface;

    let spec = FrameSpec::new(
        "hf-split",
        || {
            let identity = panel_header::identity_text(
                MI_CLOUD_DOWNLOAD,
                "HuggingFace".to_string(),
                tr!("hf.header.subtitle"),
            );
            Box::new(panel_header::center(CenterSpec::new(identity).center_extra(
                DecoratedBox::new().class("hf-header-search").child(header::search_bar()),
            )))
        },
        || Box::new(detail_panel::view()),
    )
    .left(Pane::new(
        left_visible,
        app.hf_left_split_ratio,
        260.0,
        || Box::new(panel_header::side_title(MI_SEARCH, tr!("hf.list.models_section"))),
        || Box::new(list_panel::view()),
    ))
    .right(Pane::new(
        right_visible,
        app.hf_right_split_ratio,
        260.0,
        || Box::new(panel_header::side_title(MI_DESCRIPTION, tr!("hf.detail.tab.files"))),
        || Box::new(detail_panel::files_view()),
    ));

    // Панель загрузок — под каркасом и на всю ширину: сводку видно при любой
    // раскладке боковых панелей.
    let main = DecoratedBox::new().class("hf-page").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow").child(workspace_frame::view(spec)))
            .child(dock::view()),
    );

    mgui! {
        Stack::new() => [
            main,
            dialogs::view(),
        ]
    }
}
